use crate::DuplicateKeyStrategy;
use crate::try_section_from_line;
use crate::{IniParser, ValueByteRangeResult, error::Error};
use std::io::{BufRead, Seek, Write};

#[cfg(feature = "async")]
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

const WRITE_BUFFER_SIZE: usize = 8192;

impl IniParser<'_> {
    /// Changes the value in the source ini and writes the resulting changed ini file to the
    /// destination.
    pub fn write_value(
        &self,
        source: &mut (impl std::io::Read + Seek),
        mut destination: impl Write,
        section: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), Error> {
        source.rewind()?;
        // Because we might not know if there are other instances until we reach the end of
        // the file, we have to scan the file once to find the correct location of the value.
        // Once we know that we rewind and write the contents
        // Technically with DuplicateKeyStrategy::UseFirst, we could just use the first location
        // encountered and not have to rewind, it would need to be implemented as another method
        // though to remove the Seek trait bound.
        let mut value = value.to_owned();
        let ValueByteRangeResult {
            file_size_bytes,
            last_byte_in_section,
            value_range,
        } = {
            let mut buffer = std::io::BufReader::new(&mut *source);
            self.value_byte_range(&mut buffer, section, key)?
        };
        // If the value wasn't found, we'll be adding it to the end of the section, or the end of
        // the file. We'll also need to add the key and section.
        let value_range = value_range.unwrap_or_else(|| {
            if let Some(position) = last_byte_in_section {
                value = format!("{key}={value}\n");
                position..position
            } else {
                let section = section.map(|s| format!("[{s}]\n")).unwrap_or_default();
                value = format!("{section}{key}={value}\n");
                file_size_bytes..file_size_bytes
            }
        });

        source.rewind()?;
        let mut buffer = [0; WRITE_BUFFER_SIZE];
        let mut buffer_window_start = 0;
        let mut buffer_window_end = 0;
        let mut in_value = false;
        let mut value_written = false;
        loop {
            let bytes_read = source.read(&mut buffer)?.min(WRITE_BUFFER_SIZE);

            debug_assert!(bytes_read <= WRITE_BUFFER_SIZE, "{bytes_read}");
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
                        write_until < WRITE_BUFFER_SIZE,
                        "buffer_window: [{}..{}], write_until: {}",
                        buffer_window_start,
                        buffer_window_end,
                        write_until
                    );
                    destination.write_all(&buffer[0..write_until])?;
                    destination.write_all(value.as_bytes())?;
                    value_written = true;
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
        if !value_written {
            destination.write_all(value.as_bytes())?;
        }
        Ok(())
    }

    #[cfg(feature = "async")]
    pub async fn write_value_async(
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
        // If the value wasn't found, we'll be adding it to the end of the section, or the end of
        // the file. We'll also need to add the key and section.
        let value_range = value_range.unwrap_or_else(|| {
            if let Some(position) = last_byte_in_section {
                value = format!("{key}={value}\n");
                position..position
            } else {
                let section = section.map(|s| format!("[{s}]\n")).unwrap_or_default();
                value = format!("{section}{key}={value}\n");
                file_size_bytes..file_size_bytes
            }
        });

        source.rewind().await?;
        let mut buffer = [0; WRITE_BUFFER_SIZE];
        let mut buffer_window_start = 0;
        let mut buffer_window_end = 0;
        let mut in_value = false;
        let mut value_written = false;
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
                        write_until < WRITE_BUFFER_SIZE,
                        "buffer_window: [{}..{}], write_until: {}",
                        buffer_window_start,
                        buffer_window_end,
                        write_until
                    );
                    destination.write_all(&buffer[0..write_until])?;
                    destination.write_all(value.as_bytes())?;
                    value_written = true;
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
        if !value_written {
            destination.write_all(value.as_bytes())?;
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
        // Whitespace around section names is not significant
        let section = section.map(|s| s.trim());

        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut last_in_section = None;
        let mut line = String::new();
        let mut next_line = String::new();
        let mut last_value_candidate = None;
        let mut bytes_processed = 0;
        if in_section {
            last_in_section = Some(bytes_processed);
        }
        loop {
            line.clear();
            let mut bytes_read = source.read_line(&mut line)?;
            if bytes_read == 0 {
                break;
            }
            if line.trim().ends_with('\\') {
                loop {
                    let bytes_read_continuation = source.read_line(&mut next_line)?;
                    if bytes_read_continuation == 0 {
                        break;
                    }
                    bytes_read += bytes_read_continuation;
                    if next_line.trim_end().ends_with('\\') {
                        line.push_str(&next_line);
                    } else {
                        line.push_str(&next_line);
                        break;
                    }
                    next_line.clear();
                }
            }
            if let Some(this_section) = try_section_from_line(&line) {
                if let Some(section) = section {
                    in_section = section == this_section;
                } else {
                    in_section = false;
                }
            } else if in_section && let Some(line_range) = self.try_value(&line, key) {
                last_value_candidate =
                    Some(bytes_processed + line_range.start..bytes_processed + line_range.end);

                // We can return early if UseFirst is set
                if last_value_candidate.is_some()
                    && self.duplicate_keys == DuplicateKeyStrategy::UseFirst
                {
                    bytes_processed += bytes_read;
                    if in_section && !line.trim().is_empty() {
                        last_in_section = Some(bytes_processed);
                    }
                    return Ok(ValueByteRangeResult {
                        file_size_bytes: bytes_processed,
                        last_byte_in_section: last_in_section,
                        value_range: last_value_candidate,
                    });
                }
            }
            bytes_processed += bytes_read;

            if in_section && !line.trim().is_empty() {
                last_in_section = Some(bytes_processed);
            }
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
        // Whitespace around section names is not significant
        let section = section.map(|s| s.trim());
        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut last_in_section = None;
        let mut line = String::new();
        let mut next_line = String::new();
        let mut last_value_candidate = None;
        let mut bytes_processed = 0;
        if in_section {
            last_in_section = Some(bytes_processed);
        }
        loop {
            line.clear();
            let mut bytes_read = source.read_line(&mut line).await?;
            if bytes_read == 0 {
                break;
            }
            if line.trim().ends_with('\\') {
                loop {
                    let bytes_read_continuation = source.read_line(&mut next_line).await?;
                    if bytes_read_continuation == 0 {
                        break;
                    }
                    bytes_read += bytes_read_continuation;
                    if next_line.trim_end().ends_with('\\') {
                        line.push_str(&next_line);
                    } else {
                        line.push_str(&next_line);
                        break;
                    }
                    next_line.clear();
                }
            }

            if let Some(this_section) = try_section_from_line(&line) {
                if let Some(section) = section {
                    in_section = section == this_section;
                } else {
                    in_section = false;
                }
            } else if in_section && let Some(line_range) = self.try_value(&line, key) {
                last_value_candidate =
                    Some(bytes_processed + line_range.start..bytes_processed + line_range.end);

                // We can return early if UseFirst is set
                if last_value_candidate.is_some()
                    && self.duplicate_keys == DuplicateKeyStrategy::UseFirst
                {
                    bytes_processed += bytes_read;
                    if in_section && !line.trim().is_empty() {
                        last_in_section = Some(bytes_processed);
                    }
                    return Ok(ValueByteRangeResult {
                        file_size_bytes: bytes_processed,
                        last_byte_in_section: last_in_section,
                        value_range: last_value_candidate,
                    });
                }
            }
            bytes_processed += bytes_read;
            if in_section && !line.trim().is_empty() {
                last_in_section = Some(bytes_processed);
            }
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
    use crate::assert_eq_preserve_new_lines;
    #[cfg(feature = "async")]
    use ::paste::paste;
    use indoc::indoc;

    macro_rules! write_value_eq {
        {
            test_name = $test_name:ident,
            input = $input:expr,
            section = $section:expr,
            key = $key:expr,
            value = $value:expr,
            expected = $expected:expr
            $(, description = $description:expr)*
            $(, parser = $parser:expr)* $(,)?
        } => {
            #[test]
            fn $test_name() {
                #[allow(unused_variables)]
                let parser = IniParser::default();
                $(
                    let parser = $parser;
                )*
                let mut reader = std::io::Cursor::new($input);
                let mut dest = Vec::new();
                parser.write_value(&mut reader, &mut dest, $section, $key, $value).unwrap();
                let value = String::from_utf8(dest).unwrap();
                let value = value.replace("\n", "\\n\n").replace(" ", "·");
                let expected = $expected.replace("\n", "\\n\n").replace(" ", "·");
                assert_eq_preserve_new_lines!(value, expected, $($description),*);
            }

            #[cfg(feature = "async")]
            paste! {
                #[tokio::test]
                async fn [<$test_name _async>]() {
                    #[allow(unused_variables)]
                    let parser = IniParser::default();
                    $(
                        let parser = $parser;
                    )*
                    let mut reader = std::io::Cursor::new($input);
                    let mut dest = Vec::new();
                    parser.write_value_async(&mut reader, &mut dest, $section, $key, $value).await.unwrap();
                    let value = String::from_utf8(dest).unwrap();
                    assert_eq_preserve_new_lines!(value, $expected, $($description),*);
                }
            }
        };
    }

    write_value_eq! {
        test_name=write_value_no_section_replace,
        input="name=tom",
        section=None,
        key="name",
        value="bill",
        expected="name=bill",
        description="test",
        parser=IniParser::default(),
    }

    write_value_eq! {
        test_name=write_value_no_section_add_empty,
        input="",
        section=None,
        key="name",
        value="bill",
        expected=indoc!{"
            name=bill
        "},
        description="expected name=bill to be added to an empty file",
    }

    write_value_eq! {
        test_name=write_value_section_add_empty,
        input="",
        section=Some("contact"),
        key="name",
        value="bill",
        expected=indoc!{"
            [contact]
            name=bill
        "},
        description="expected [contact]name=bill to be added to an empty file",
    }

    write_value_eq! {
        test_name=write_value_section_add,
        input=indoc!{"
            [contact]
            name=bill
        "},
        section=Some("stats"),
        key="performance",
        value="100",
        expected=indoc!{"
            [contact]
            name=bill
            [stats]
            performance=100
        "},
        description="expected [stats]performance=100 to be added as a new section, leaving the existing section intact.",
    }

    write_value_eq! {
        test_name=write_value_section_add_multiple_sections,
        input=indoc!{"
            [schedule]

            [contact]
            name=bill
        "},
        section=Some("stats"),
        key="performance",
        value="100",
        expected=indoc!{"
            [schedule]

            [contact]
            name=bill
            [stats]
            performance=100
        "},
        description="expected [stats]performance=100 to be added as a new section, leaving the existing sections intact.",
    }

    write_value_eq! {
        test_name=write_value_no_section_add_multiple_sections,
        input=indoc!{"
            [schedule]

            [contact]
            name=bill
        "},
        section=None,
        key="performance",
        value="100",
        expected=indoc!{"
            performance=100
            [schedule]

            [contact]
            name=bill
        "},
        description="expected performance=100 to be added to the global space, leaving the existing sections intact.",
    }

    write_value_eq! {
        test_name=write_value_no_section_add,
        input=indoc!{"
            [contact]
            name=tom
        "},
        section=None,
        key="name",
        value="bill",
        expected=indoc!{"
            name=bill
            [contact]
            name=tom
        "},
        description="expected this to add name=bill in the global space, leaving the contact section alone",
    }

    write_value_eq! {
        test_name=write_new_value_existing_section,
        input=indoc!{"
            [contact]
            name=bill
        "},
        section=Some("contact"),
        key="email",
        value="bill@example.com",
        expected=indoc!{"
            [contact]
            name=bill
            email=bill@example.com
        "},
        description="",
    }

    write_value_eq! {
        test_name=write_value_section,
        input=indoc!{"
            [contact]
            name=tom
        "},
        section=Some("contact"),
        key="name",
        value="bill",
        expected=indoc!{"
            [contact]
            name=bill
        "},
        description="expected name to change from tom to bill",
    }

    write_value_eq! {
        test_name=write_value_trailing_comment,
        input=indoc!{"
            [contact]
            name=tom # test
        "},
        section=Some("contact"),
        key="name",
        value="bill",
        expected=indoc!{"
            [contact]
            name=bill # test
        "},
        description="expected name to change while keeping the trailing comment",
    }

    write_value_eq! {
        test_name=write_value_line_continuation_comment,
        input=indoc!{"
            [contact]
            # this is a \\
            multiline comment
            test=hello
        "},
        section=Some("contact"),
        key="test",
        value="goodbye",
        expected=indoc!{"
            [contact]
            # this is a \\
            multiline comment
            test=goodbye
        "},
        description="",
        parser=IniParser{line_continuation:true, ..Default::default()}
    }

    write_value_eq! {
        test_name=write_value_line_continuation,
        input=indoc!{"
            [contact]
            description=first line \\
            second line \\
            third line
            another_key=another value
        "},
        section=Some("contact"),
        key="description",
        value="hello world",
        expected=indoc!{r#"
            [contact]
            description=hello world
            another_key=another value
        "#},
        description="expected all of the lines for the value to be changed to `hello world`",
        parser=IniParser{line_continuation:true, ..Default::default()},
    }

    write_value_eq! {
        test_name=write_empty_value_existing_empty,
        input=indoc!{"
            name=
        "},
        section=None,
        key="name",
        value="",
        expected=indoc!{"
            name=
        "},
        description="expected writing an empty value to an empty value to reuse the existing key",
    }

    write_value_eq! {
        test_name=write_value_existing_empty,
        input=indoc!{"
            name=
        "},
        section=None,
        key="name",
        value="bill",
        expected=indoc!{"
            name=bill
        "},
        description="expected writing a value to an empty value to reuse the existing key",
    }

    write_value_eq! {
        test_name=write_value_emoji_characters,
        input=indoc!{"
            [display]
            emoji=🚀🌎🌟 # space emoji
        "},
        section=Some("display"),
        key="emoji",
        value="🎮🎯",
        expected=indoc!{"
            [display]
            emoji=🎮🎯 # space emoji
        "},
        description="multi-byte emoji characters as values should be allowed",
    }

    write_value_eq! {
        test_name=write_value_special_characters_in_section,
        input=indoc!{"
            [special!@$%^&*()]
            key=value
        "},
        section=Some("special!@$%^&*()"),
        key="key",
        value="new value",
        expected=indoc!{"
            [special!@$%^&*()]
            key=new value
        "},
        description="section names should allow special characters",
    }

    write_value_eq! {
        test_name=write_value_comment_delimiter_in_section,
        input=indoc!{"
            [special;#1]
            key=value
        "},
        section=Some("special;#1"),
        key="key",
        value="new value",
        expected=indoc!{"
            [special;#1]
            key=new value
        "},
        description="comment delimiter should work in section names",
    }

    #[test]
    fn test_comment_delimiter_not_in_key() {
        #[allow(unused_variables)]
        let parser = IniParser::default();
        let mut reader = std::io::Cursor::new(indoc! {
            "
                [section]
                special#1=value
            "
        });
        let mut dest = Vec::new();
        parser
            .write_value(
                &mut reader,
                &mut dest,
                Some("section"),
                "special",
                "new value",
            )
            .unwrap();
        let value = String::from_utf8(dest).unwrap();
        let value = value.replace("\n", "\\n\n").replace(" ", "·");
        let should_not_be = (indoc! {
            "
                [section]
                special#1=new value
            "
        })
        .replace("\n", "\\n\n")
        .replace(" ", "·");
        assert_ne!(
            value, should_not_be,
            "comment delimiter should not work in key names"
        );
    }

    write_value_eq! {
        test_name=write_value_special_characters_in_key,
        input=indoc!{"
            [section]
            special!@$%^&*()=value
        "},
        section=Some("section"),
        key="special!@$%^&*()",
        value="new value",
        expected=indoc!{"
            [section]
            special!@$%^&*()=new value
        "},
        description="key names should allow special characters that aren't comment delimiters",
    }

    write_value_eq! {
        test_name=write_value_special_characters_in_value,
        input=indoc!{"
            [section]
            key=value!@$%^&*()
        "},
        section=Some("section"),
        key="key",
        value="new!@$%^&*()",
        expected=indoc!{"
            [section]
            key=new!@$%^&*()
        "},
        description="values should allow special characters that aren't comment delimiters",
    }

    write_value_eq! {
        test_name=write_value_unicode_characters,
        input=indoc!{"
            [unicode]
            key=áéíóúñ
        "},
        section=Some("unicode"),
        key="key",
        value="αβγδεζηθ",
        expected=indoc!{"
            [unicode]
            key=αβγδεζηθ
        "},
        description="values should allow unicode characters",
    }

    write_value_eq! {
        test_name=write_value_very_long_value,
        input=indoc!{"
            [section]
            key=short value
        "},
        section=Some("section"),
        key="key",
        value="This is a very long value that contains many characters and should be properly handled by the parser. It includes multiple sentences and various punctuation marks. The value is intentionally made long to test the parser's ability to handle large values without issues.",
        expected=indoc!{"
            [section]
            key=This is a very long value that contains many characters and should be properly handled by the parser. It includes multiple sentences and various punctuation marks. The value is intentionally made long to test the parser's ability to handle large values without issues.
        "},
        description="values should allow very long values",
    }

    write_value_eq! {
        test_name=write_value_duplicate_keys_first,
        input=indoc!{"
            [section]
            key=first value
            other=other value
            key=second value
        "},
        section=Some("section"),
        key="key",
        value="new value",
        expected=indoc!{"
            [section]
            key=new value
            other=other value
            key=second value
        "},
        description="first key should be updated when using DuplicateKeyStrategy::UseFirst, other keys should be left alone",
        parser=IniParser{duplicate_keys: DuplicateKeyStrategy::UseFirst,..Default::default()},
    }

    write_value_eq! {
        test_name=write_value_duplicate_sections,
        input=indoc!{"
            [section]
            key=first value
            [other]
            key=other value
            [section]
            key=second value
        "},
        section=Some("section"),
        key="key",
        value="new value",
        expected=indoc!{"
            [section]
            key=new value
            [other]
            key=other value
            [section]
            key=second value
        "},
        description="first section should be updated when using DuplicateKeyStrategy::UseFirst, other sections should be left alone",
        parser=IniParser{duplicate_keys: DuplicateKeyStrategy::UseFirst,..Default::default()},
    }

    write_value_eq! {
        test_name=write_value_nested_sections,
        input=indoc!{"
            [parent]
            key=parent value
            [parent.child]
            key=child value
        "},
        section=Some("parent.child"),
        key="key",
        value="new child value",
        expected=indoc!{"
            [parent]
            key=parent value
            [parent.child]
            key=new child value
        "},
        description="nested sections should work the same as other sections and not affect the \"parent\" section",
    }

    write_value_eq! {
        test_name=write_value_whitespace_in_section,
        input=indoc!{"
            [ section with spaces ]
            key=value
        "},
        section=Some(" section with spaces "),
        key="key",
        value="new value",
        expected=indoc!{"
            [ section with spaces ]
            key=new value
        "},
        description="whitespace around section names should not be significant",
    }

    write_value_eq! {
        test_name=write_value_whitespace_in_key_value,
        input=indoc!{"
            [section]
            key with spaces = value
        "},
        section=Some("section"),
        key="key with spaces ",
        value="new value",
        expected=indoc!{"
            [section]
            key with spaces = new value
        "},
        description="whitespace around keys and values should be preserved",
    }

    write_value_eq! {
        test_name=write_value_quoted_values,
        input=indoc!{"
            [section]
            key=\"quoted value\"
        "},
        section=Some("section"),
        key="key",
        value="\"new quoted value\"",
        expected=indoc!{"
            [section]
            key=\"new quoted value\"
        "},
        description="quoted values should be preserved when writing a value",
    }

    write_value_eq! {
        test_name=write_value_multiple_comments,
        input=indoc!{"
            # Global comment
            [section] # Section comment
            key=value # Key comment
        "},
        section=Some("section"),
        key="key",
        value="new value",
        expected=indoc!{"
            # Global comment
            [section] # Section comment
            key=new value # Key comment
        "},
        description="multiple comments should be preserved when writing a value",
    }
    write_value_eq! {
        test_name=add_key_to_section_trailing_empty_lines,
        input=indoc!{"
            [section]
            key=value

            [section2]
            key=value2
        "},
        section=Some("section"),
        key="key2",
        value="new value",
        expected=indoc!{"
            [section]
            key=value
            key2=new value

            [section2]
            key=value2
        "},
        description="adding a key to a section should insert it before any trailing empty lines",
    }

    write_value_eq! {
        test_name=add_key_to_global_trailing_empty_lines,
        input=indoc!{"
            # Global comment


            [section]
            key=value

            [section2]
            key=value2
        "},
        section=None,
        key="key2",
        value="new value",
        expected=indoc!{"
            # Global comment
            key2=new value


            [section]
            key=value

            [section2]
            key=value2
        "},
        description="adding a key to the global section should insert it before any trailing empty lines",
    }

    write_value_eq! {
        test_name=add_key_to_last_section_trailing_empty_lines,
        input=indoc!{"
            [section]
            key=value

            [section2]
            key=value2



        "},
        section=Some("section2"),
        key="key2",
        value="new value",
        expected=indoc!{"
            [section]
            key=value

            [section2]
            key=value2
            key2=new value



        "},
        description="adding a key to the last section should insert it before any trailing empty lines",
    }
}
