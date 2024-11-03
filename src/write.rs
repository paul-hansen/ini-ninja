use crate::try_section_from_line;
use crate::DuplicateKeyStrategy;
use crate::{error::Error, IniParser, ValueByteRangeResult};
use std::io::{BufRead, Seek, Write};

#[cfg(feature = "async")]
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

impl IniParser {
    /// Changes the value in the source ini and writes the resulting changed ini file to the
    /// destination.
    ///
    /// BUFFER_SIZE is the size of a buffer used to write the lines
    pub fn write_value<const BUFFER_SIZE: usize>(
        &self,
        source: &mut (impl std::io::Read + Seek),
        mut destination: impl Write,
        section: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), Error> {
        // Because we might not know if there are other copies of the until we reach the end of
        // the file, we have to scan the file once to find the correct location of the value.
        // Once we know that we rewind and write the contents
        // Technically with DuplicateKeyStrategy::UseFirst, we could just use the first location
        // encountered and not have to rewind, it would need to be implemented as another method
        // though to remove the trait bound.
        let mut value = value.to_owned();
        let ValueByteRangeResult {
            file_size_bytes,
            last_byte_in_section,
            value_range,
        } = {
            let mut buffer = std::io::BufReader::new(&mut *source);
            self.value_byte_range(&mut buffer, section, key)?
        };
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
        let mut buffer_window_start = 0;
        let mut buffer_window_end = 0;
        let mut in_value = false;
        loop {
            let bytes_read = source.read(&mut buffer)?.min(BUFFER_SIZE);

            debug_assert!(bytes_read <= BUFFER_SIZE, "{bytes_read}");
            if bytes_read == 0 {
                break;
            }
            buffer_window_end += bytes_read;
            // is the start of the value inside of the buffer's current window?
            let start_in_window =
                (buffer_window_start..buffer_window_end).contains(&value_range.start);
            // is the end of the value inside of the buffer's current window?
            let end_in_window = (buffer_window_start..buffer_window_end).contains(&value_range.end);
            if start_in_window {
                in_value = true;
            }
            match (start_in_window, end_in_window, in_value) {
                // We are not in a value and no value is starting or ending, write all the bytes we
                // read exactly the same as the source.
                (false, false, false) => destination.write_all(&buffer[..bytes_read])?,
                // if the whole buffer window is inside the value we are replacing, we don't need to
                // write the old value so do nothing
                (false, false, true) => {}
                // value is starting in this buffer window
                (true, end_in_window, _) => {
                    in_value = true;
                    let write_until = value_range.start - buffer_window_start;
                    debug_assert!(
                        write_until < BUFFER_SIZE,
                        "buffer_window: [{}..{}], write_until: {}",
                        buffer_window_start,
                        buffer_window_end,
                        write_until
                    );
                    destination.write_all(&buffer[0..write_until])?;
                    destination.write_all(value.as_bytes())?;
                    if end_in_window {
                        destination.write_all(
                            &buffer[value_range.end - buffer_window_start
                                ..buffer_window_end - buffer_window_start],
                        )?;
                    }
                }
                // value is ending but did not start in this buffer window
                (false, true, _) => {
                    destination
                        .write_all(&buffer[value_range.end - buffer_window_start..bytes_read])?;
                }
            }
            if end_in_window {
                in_value = false;
            }
            buffer_window_start = buffer_window_end
        }
        Ok(())
    }

    #[cfg(feature = "async")]
    pub async fn write_value_async<const BUFFER_SIZE: usize>(
        &self,
        source: &mut (impl AsyncRead + AsyncSeek + Unpin),
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
        } = {
            let mut buffer = tokio::io::BufReader::new(&mut *source);
            self.value_byte_range_async(&mut buffer, section, key)
                .await?
        };
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
        let mut buffer_window_start = 0;
        let mut buffer_window_end = 0;
        let mut in_value = false;
        loop {
            let bytes_read = source.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            buffer_window_end += bytes_read;
            // is the start of the value inside of the buffer's current window?
            let start_in_window =
                value_range.start >= buffer_window_start && value_range.start < buffer_window_end;
            // is the end of the value inside of the buffer's current window?
            let end_in_window =
                value_range.end >= buffer_window_start && value_range.end < buffer_window_end;
            if start_in_window {
                in_value = true;
            }
            match (start_in_window, end_in_window, in_value) {
                // We are not in a value and no value is starting or ending, write all the bytes we
                // read exactly the same as the source.
                (false, false, false) => destination.write_all(&buffer[..bytes_read])?,
                // if the whole buffer window is inside the value we are replacing, we don't need to
                // write the old value so do nothing
                (false, false, true) => {}
                // value is starting in this buffer window
                (true, end_in_window, _) => {
                    in_value = true;
                    let write_until = value_range.start - buffer_window_start;
                    debug_assert!(
                        write_until < BUFFER_SIZE,
                        "buffer_window: [{}..{}], write_until: {}",
                        buffer_window_start,
                        buffer_window_end,
                        write_until
                    );
                    destination.write_all(&buffer[0..write_until])?;
                    destination.write_all(value.as_bytes())?;
                    if end_in_window {
                        destination.write_all(
                            &buffer[value_range.end - buffer_window_start
                                ..buffer_window_end - buffer_window_start],
                        )?;
                    }
                }
                // value is ending but did not start in this buffer window
                (false, true, _) => {
                    destination
                        .write_all(&buffer[value_range.end - buffer_window_start..bytes_read])?;
                }
            }
            if end_in_window {
                in_value = false;
            }
            buffer_window_start = buffer_window_end
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
            paste! {
                #[test]
                fn [<$test_name _small_buf>]() {
                    let parser = $parser;
                    let mut reader = std::io::Cursor::new($ini_file_string);
                    let mut dest = Vec::new();
                    parser.write_value::<10>(&mut reader, &mut dest, $section, $key, $value).unwrap();
                    let value = String::from_utf8(dest).unwrap();
                    assert_eq!(value, $expected, $($description),*);
                }
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

            #[cfg(feature = "async")]
            paste! {
                #[tokio::test]
                async fn [<$test_name _async_small_buf>]() {
                    let parser = $parser;
                    let mut reader = std::io::Cursor::new($ini_file_string);
                    let mut dest = Vec::new();
                    parser.write_value_async::<10>(&mut reader, &mut dest, $section, $key, $value).await.unwrap();
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
            description=first line \
            second line \
            third line
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
        "expected all of the lines for the value to be changed to `hello world`",
    }
}
