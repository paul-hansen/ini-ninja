use crate::try_section_from_line;
use crate::DuplicateKeyStrategy;
use crate::{error::Error, IniParser, ValueByteRangeResult};
use std::io::{BufRead, Seek, Write};

#[cfg(feature = "async")]
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncSeek, AsyncSeekExt};

impl IniParser {
    pub fn write_value<const BUFFER_SIZE: usize>(
        &self,
        source: &mut (impl BufRead + Seek),
        mut destination: impl Write,
        section: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), Error> {
        let mut value = value.to_owned();
        let ValueByteRangeResult {
            file_size_bytes,
            last_byte_in_section,
            value_range,
        } = self.value_byte_range(source, section, key)?;
        let value_range = value_range.unwrap_or_else(|| {
            if let Some(position) = last_byte_in_section {
                value = format!("{key}={value}\n");
                position..position
            } else {
                value = format!("[section]\n{key}={value}\n");
                file_size_bytes..file_size_bytes
            }
        });

        source.rewind()?;
        let mut buffer = [0; BUFFER_SIZE];
        let mut source_bytes_index = 0;
        loop {
            let bytes_read = source.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            if source_bytes_index + bytes_read > value_range.start {
                debug_assert!(value_range.start >= source_bytes_index);
                let write_until = value_range.start - source_bytes_index;
                destination.write_all(&buffer[0..write_until])?;
                destination.write_all(value.as_bytes())?;
                if value_range.end < source_bytes_index + bytes_read {
                    destination.write_all(&buffer[value_range.end..bytes_read])?;
                }
            } else {
                destination.write_all(&buffer[0..bytes_read])?;
            };
            source_bytes_index += bytes_read;
        }
        Ok(())
    }

    #[cfg(feature = "async")]
    pub async fn write_value_async<const BUFFER_SIZE: usize>(
        &self,
        source: &mut (impl AsyncBufRead + AsyncSeek + Unpin),
        mut destination: impl Write,
        section: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), Error> {
        let mut value = value.to_owned();
        let ValueByteRangeResult {
            file_size_bytes,
            last_byte_in_section,
            value_range,
        } = self.value_byte_range_async(source, section, key).await?;
        let value_range = value_range.unwrap_or_else(|| {
            if let Some(position) = last_byte_in_section {
                value = format!("{key}={value}\n");
                position..position
            } else {
                value = format!("[section]\n{key}={value}\n");
                file_size_bytes..file_size_bytes
            }
        });

        source.rewind().await?;
        let mut buffer = [0; BUFFER_SIZE];
        let mut source_bytes_index = 0;
        loop {
            let bytes_read = source.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            if source_bytes_index + bytes_read > value_range.start {
                debug_assert!(value_range.start >= source_bytes_index);
                let write_until = value_range.start - source_bytes_index;
                destination.write_all(&buffer[0..write_until])?;
                destination.write_all(value.as_bytes())?;
                if value_range.end < source_bytes_index + bytes_read {
                    destination.write_all(&buffer[value_range.end..bytes_read])?;
                }
            } else {
                destination.write_all(&buffer[0..bytes_read])?;
            };
            source_bytes_index += bytes_read;
        }
        Ok(())
    }

    /// Get the current byte range where the value is stored in the source ini file, if it exists.
    ///
    /// This function is blocking and should be used carefully: it is possible for
    /// an attacker to continuously send bytes without ever sending a newline
    /// or EOF. You can use [`take`] to limit the maximum number of bytes read.
    fn value_byte_range(
        &self,
        source: &mut impl BufRead,
        section: Option<&str>,
        key: &str,
    ) -> Result<ValueByteRangeResult, Error> {
        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut last_in_section = None;
        let mut line = String::new();
        let mut next_line = String::new();
        let mut last_value_candidate = None;
        let mut bytes_processed = 0;
        loop {
            line.clear();
            let bytes_read = source.read_line(&mut line)?;
            if bytes_read == 0 {
                break;
            }
            if line.trim().ends_with('\\') {
                while source.read_line(&mut next_line)? != 0 {
                    let next_line = next_line.trim_start();
                    line.push_str(next_line);
                    if let Some(line2) = line.strip_suffix('\\') {
                        line = line2.to_string();
                    } else {
                        break;
                    }
                }
            }
            if in_section {
                last_in_section = Some(bytes_processed);
            }
            if let Some(this_section) = try_section_from_line(&line) {
                if let Some(section) = section {
                    in_section = section == this_section;
                } else {
                    in_section = false;
                }
            } else if in_section {
                if let Some(line_range) = self.try_value(&line, key) {
                    last_value_candidate =
                        Some(bytes_processed + line_range.start..bytes_processed + line_range.end);

                    // We can return early if UseFirst is set
                    if last_value_candidate.is_some()
                        && self.duplicate_keys == DuplicateKeyStrategy::UseFirst
                    {
                        bytes_processed += bytes_read;
                        return Ok(ValueByteRangeResult {
                            file_size_bytes: bytes_processed,
                            last_byte_in_section: last_in_section,
                            value_range: last_value_candidate,
                        });
                    }
                }
            }
            bytes_processed += bytes_read;
        }
        Ok(ValueByteRangeResult {
            file_size_bytes: bytes_processed,
            last_byte_in_section: last_in_section,
            value_range: last_value_candidate,
        })
    }

    /// Get the current byte range where the value is stored in the source ini file, if it exists.
    #[cfg(feature = "async")]
    async fn value_byte_range_async(
        &self,
        source: &mut (impl AsyncBufRead + Unpin),
        section: Option<&str>,
        key: &str,
    ) -> Result<ValueByteRangeResult, Error> {
        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut last_in_section = None;
        let mut line = String::new();
        let mut next_line = String::new();
        let mut last_value_candidate = None;
        let mut bytes_processed = 0;
        loop {
            line.clear();
            let bytes_read = source.read_line(&mut line).await?;
            if bytes_read == 0 {
                break;
            }
            if line.trim().ends_with('\\') {
                while source.read_line(&mut next_line).await? != 0 {
                    let next_line = next_line.trim_start();
                    line.push_str(next_line);
                    if let Some(line2) = line.strip_suffix('\\') {
                        line = line2.to_string();
                    } else {
                        break;
                    }
                }
            }
            if in_section {
                last_in_section = Some(bytes_processed);
            }
            if let Some(this_section) = try_section_from_line(&line) {
                if let Some(section) = section {
                    in_section = section == this_section;
                } else {
                    in_section = false;
                }
            } else if in_section {
                if let Some(line_range) = self.try_value(&line, key) {
                    last_value_candidate =
                        Some(bytes_processed + line_range.start..bytes_processed + line_range.end);

                    // We can return early if UseFirst is set
                    if last_value_candidate.is_some()
                        && self.duplicate_keys == DuplicateKeyStrategy::UseFirst
                    {
                        bytes_processed += bytes_read;
                        return Ok(ValueByteRangeResult {
                            file_size_bytes: bytes_processed,
                            last_byte_in_section: last_in_section,
                            value_range: last_value_candidate,
                        });
                    }
                }
            }
            bytes_processed += bytes_read;
        }
        Ok(ValueByteRangeResult {
            file_size_bytes: bytes_processed,
            last_byte_in_section: last_in_section,
            value_range: last_value_candidate,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[cfg(feature = "async")]
    use ::paste::paste;
    use indoc::indoc;

    #[macro_export]
    macro_rules! write_value_eq {
        {
            $test_name:ident,
            $parser:expr,
            $ini_file_string:expr,
            $section:expr,
            $key:expr,
            $value:expr,
            $expected:expr
            $(, $description:expr)* $(,)?
        } => {
            #[test]
            fn $test_name() {
                let parser = $parser;
                let mut reader = std::io::Cursor::new($ini_file_string);
                let mut dest = Vec::new();
                parser.write_value::<1024>(&mut reader, &mut dest, $section, $key, $value).unwrap();
                let value = String::from_utf8(dest).unwrap();
                assert_eq!(value, $expected, $($description),*);
            }

            #[cfg(feature = "async")]
            paste! {
                #[tokio::test]
                async fn [<$test_name _async>]() {
                    let parser = $parser;
                    let mut reader = std::io::Cursor::new($ini_file_string);
                    let mut dest = Vec::new();
                    parser.write_value_async::<1024>(&mut reader, &mut dest, $section, $key, $value).await.unwrap();
                    let value = String::from_utf8(dest).unwrap();
                    assert_eq!(value, $expected, $($description),*);
                }
            }
        };
    }

    write_value_eq! {
        write_value_no_section_replace,
        IniParser::default(),
        "name=tom",
        None,
        "name",
        "bill",
        "name=bill"
    }

    write_value_eq! {
        write_value_no_section_add,
        IniParser::default(),
        indoc!{"
            [contact]
            name=tom
        "},
        None,
        "name",
        "bill",
        indoc!{"
            name=bill
            [contact]
            name=tom
        "},
        "expected this to add name=bill in the global space",
    }

    write_value_eq! {
        write_value_section,
        IniParser::default(),
        indoc!{"
            [contact]
            name=tom
        "},
        Some("contact"),
        "name",
        "bill",
        indoc!{"
            [contact]
            name=bill
        "},
    }

    write_value_eq! {
        write_value_trailing_comment,
        IniParser::default(),
        indoc!{"
            [contact]
            name=tom # test
        "},
        Some("contact"),
        "name",
        "bill",
        indoc!{"
            [contact]
            name=bill # test
        "},
        "expected it to keep the trailing comment on the value",
    }
    write_value_eq! {
        write_value_line_continuation,
        IniParser::default(),
        indoc!{"
            [contact]
            description=some long\
            text describing the thing
            another_key=another value
        "},
        Some("contact"),
        "description",
        "hello world",
        indoc!{"
            [contact]
            description=hello world
            another_key=another value
        "},
    }
}
