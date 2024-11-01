#![doc = include_str!("../README.md")]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
mod error;
use std::{
    io::{self, BufRead, Read, Seek, Write},
    ops::Range,
    str::FromStr,
};

use error::Error;
#[cfg(feature = "async")]
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, BufReader,
};

pub trait FromIniStr: Sized {
    type Err: std::error::Error + 'static;
    fn from_ini_str(ini_str: &str) -> Result<Self, Self::Err>;
}

macro_rules! impl_from_ini_str {
    ($type:ty) => {
        impl FromIniStr for $type {
            type Err = <$type as FromStr>::Err;
            fn from_ini_str(ini_str: &str) -> Result<Self, Self::Err> {
                FromStr::from_str(ini_str)
            }
        }
    };
}

impl FromIniStr for bool {
    type Err = <bool as FromStr>::Err;
    fn from_ini_str(ini_str: &str) -> Result<Self, <bool as FromStr>::Err> {
        let ini_str = ini_str.trim().to_ascii_lowercase();
        match &ini_str.as_str() {
            x if ["1", "yes", "on"].contains(x) => return Ok(true),
            x if ["0", "no", "off"].contains(x) => return Ok(false),
            _ => {}
        }
        <bool as FromStr>::from_str(&ini_str)
    }
}

impl FromIniStr for String {
    type Err = <String as FromStr>::Err;
    fn from_ini_str(ini_str: &str) -> Result<Self, Self::Err> {
        FromStr::from_str(trim_whitespace_and_quotes(ini_str))
    }
}

impl_from_ini_str!(i8);
impl_from_ini_str!(i16);
impl_from_ini_str!(i32);
impl_from_ini_str!(i64);
impl_from_ini_str!(i128);
impl_from_ini_str!(u8);
impl_from_ini_str!(u16);
impl_from_ini_str!(u32);
impl_from_ini_str!(u64);
impl_from_ini_str!(u128);
impl_from_ini_str!(f32);
impl_from_ini_str!(f64);
impl_from_ini_str!(char);
impl_from_ini_str!(std::path::PathBuf);

#[derive(Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum DuplicateKeyStrategy {
    /// Seems to be the most widely used.
    #[default]
    UseLast,
    /// Fastest because as soon as it finds a match it can stop.
    UseFirst,
    Error,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct IniParser {
    /// These characters indicate the start of a comment.
    pub comment_delimiters: &'static [char],
    pub trailing_comments: bool,
    pub value_start_delimiters: &'static [char],
    pub line_continuation: bool,
    /// How should we handle duplicate keys in the ini file?
    pub duplicate_keys: DuplicateKeyStrategy,
    /// Prevents attacks where an untrusted source could continuously send data to forever
    /// lock your system.
    ///
    /// The default size limit is 20MiB.
    /// If you are accepting untrusted input, you may want to reduce this to match what you would
    /// expect so it fails sooner in the case of an attack.
    ///
    /// To remove the limit, just set it to [`u64::MAX`].
    pub size_limit: u64,
}

impl Default for IniParser {
    /// The defaults are chosen to be compatible with the widest range of ini formats.
    fn default() -> Self {
        Self {
            comment_delimiters: &['#', ';'],
            trailing_comments: true,
            value_start_delimiters: &['='],
            // If true, any lines that end with `\` will consider the next line part of the
            // current line. This allows multiline values.
            line_continuation: true,
            duplicate_keys: DuplicateKeyStrategy::default(),
            size_limit: 1024 * 1024 * 20,
        }
    }
}

struct ValueByteRangeResult {
    file_size_bytes: usize,
    last_byte_in_section: Option<usize>,
    value_range: Option<Range<usize>>,
}

impl IniParser {
    /// Read a value from a INI file source.
    /// If section is none, it will look in the global space.
    pub fn read_value<T>(
        &self,
        source: impl Read,
        section: Option<&str>,
        key: &str,
    ) -> Result<Option<T>, Error>
    where
        T: FromIniStr,
    {
        let value = self.value_unaltered(source, section, key)?;
        let Some(value) = value else {
            return Ok(None);
        };
        let value = FromIniStr::from_ini_str(&value).map_err(Error::new_parse)?;
        Ok(Some(value))
    }

    /// Read a value from an async INI file source.
    /// If section is none, it will look in the global space.
    #[cfg(feature = "async")]
    pub async fn read_value_async<T>(
        &self,
        source: impl AsyncRead,
        section: Option<&str>,
        key: &str,
    ) -> Result<Option<T>, Error>
    where
        T: FromIniStr,
    {
        let value = self.value_unaltered_async(source, section, key).await?;
        let Some(value) = value else {
            return Ok(None);
        };
        let value = FromIniStr::from_ini_str(&value).map_err(Error::new_parse)?;
        Ok(Some(value))
    }

    /// Get the current byte range where the value is stored in the source ini file, if it exists.
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

    pub fn write_value<const BUFFER_SIZE: usize>(
        &self,
        source: &mut (impl BufRead + Seek),
        mut destination: impl Write,
        section: Option<&str>,
        key: &str,
        value: &str,
    ) -> Result<(), Error> {
        let mut value = value.to_owned();
        if self.size_limit != u64::MAX {
            source.seek(io::SeekFrom::End(0))?;
            let position = source.stream_position()?;
            if position > self.size_limit {
                Err(Error::TooLarge {
                    limit: self.size_limit,
                    found: position,
                })?
            }
        }
        source.rewind()?;
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

    /// Returns the value for the given section and name without any parsing. Notably this may
    /// still have quotation marks around strings. Leading and trailing whitespace will still be
    /// stripped though.
    ///
    /// Usually only use this if you are manually parsing something.
    fn value_unaltered(
        &self,
        source: impl Read,
        section: Option<&str>,
        key: &str,
    ) -> Result<Option<String>, Error> {
        // TODO: Ideally this would return Error::TooLarge instead of silently truncating
        let buffer = std::io::BufReader::new(source.take(self.size_limit));

        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut value = None;
        let mut lines = io::BufRead::lines(buffer);
        loop {
            let Some(line) = lines.next() else {
                break;
            };
            let mut line = line?;
            if let Some(line2) = line.strip_suffix('\\') {
                line = line2.to_string();
                for next_line in lines.by_ref() {
                    let next_line = next_line?;
                    let next_line = next_line.trim_start();
                    line.push_str(next_line);
                    if let Some(line2) = line.strip_suffix('\\') {
                        line = line2.to_string();
                    } else {
                        break;
                    }
                }
            }
            if self.process_line(line, section, key, &mut in_section, &mut value)? {
                return Ok(value);
            }
        }
        Ok(value)
    }
    /// Returns the value for the given section and name without any parsing. Notably this may
    /// still have quotation marks around strings. Leading and trailing whitespace will still be
    /// stripped though.
    ///
    /// Usually only use this if you are manually parsing something.
    #[cfg(feature = "async")]
    async fn value_unaltered_async(
        &self,
        source: impl AsyncRead,
        section: Option<&str>,
        key: &str,
    ) -> Result<Option<String>, Error> {
        // TODO: Ideally this would return Error::TooLarge instead of silently truncating
        let buffer = Box::pin(BufReader::new(source).take(self.size_limit));

        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut value = None;
        let mut lines = buffer.lines();
        loop {
            let Some(line) = lines.next_line().await? else {
                break;
            };
            let mut line = line;
            // Handle line continuation
            if let Some(line2) = line.strip_suffix('\\') {
                line = line2.to_string();
                while let Some(next_line) = lines.next_line().await? {
                    let next_line = next_line.trim_start();
                    line.push_str(next_line);
                    if let Some(line2) = line.strip_suffix('\\') {
                        line = line2.to_string();
                    } else {
                        break;
                    }
                }
            }
            if self.process_line(line, section, key, &mut in_section, &mut value)? {
                return Ok(value);
            }
        }
        Ok(value)
    }

    /// Mainly used to extract common functionality between async and sync implementations.
    /// Returns true if we found the final value. (Note that depending on duplicate handling, this
    /// may not be the first time we see the value)
    fn process_line(
        &self,
        line: String,
        section: Option<&str>,
        key: &str,
        in_section: &mut bool,
        value: &mut Option<String>,
    ) -> Result<bool, Error> {
        if let Some(this_section) = try_section_from_line(&line) {
            if let Some(section) = &section {
                *in_section = *section == this_section;
            } else {
                // If section is None, we are looking for a global variable.
                // Since this_section is some here, we know we aren't in the global section
                *in_section = false;
            }
        } else if *in_section {
            if let Some(range) = self.try_value(&line, key) {
                let had_previous = value.is_some();
                *value = Some(line[range].to_string());
                match self.duplicate_keys {
                    DuplicateKeyStrategy::Error => {
                        if had_previous {
                            return Err(Error::DuplicateKey {
                                key: key.to_string(),
                                section: section.map(|s| s.to_owned()),
                            });
                        }
                    }
                    DuplicateKeyStrategy::UseFirst => {
                        return Ok(true);
                    }
                    _ => {}
                }
            }
        }

        Ok(false)
    }

    /// Given a string, check try to parse as a key value and return the range of the string that
    /// contains the value.
    fn try_value(&self, line: &str, key: &str) -> Option<Range<usize>> {
        let name = key.trim();
        // Since comments are always at the end of the line, it won't change the positions to
        // remove them.
        let line = line
            .split_once(self.comment_delimiters)
            .map(|x| x.0)
            .unwrap_or(line);

        if let Some(delimiter_index) = line
            .chars()
            .position(|c| self.value_start_delimiters.contains(&c))
        {
            let this_name = line
                .split_at(line.char_indices().nth(delimiter_index)?.0)
                .0
                .trim();
            if this_name != name {
                return None;
            }
            let mut value_start = delimiter_index + 1;

            // Find the first non-whitespace character after the '='
            while value_start < line.len()
                && line
                    .chars()
                    .nth(value_start)
                    .is_some_and(|c| c.is_whitespace())
            {
                value_start += 1;
            }

            // Determine the end index of the value
            let mut value_end = line.chars().count() - 1;

            // Find the last non-whitespace character
            while value_end > value_start
                && line
                    .chars()
                    .nth(value_end)
                    .is_some_and(|c| c.is_whitespace())
            {
                value_end -= 1;
            }
            Some(line.char_indices().nth(value_start)?.0..line.char_indices().nth(value_end)?.0 + 1)
        } else {
            // If there isn't a value delimiter, there's no value.
            None
        }
    }
}

fn try_section_from_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with('[') {
        let end = trimmed.find(']')?;
        let section_name = &trimmed[1..end];
        Some(section_name.trim())
    } else {
        None
    }
}
fn trim_whitespace_and_quotes(text: &str) -> &str {
    let text = text.trim();
    let text = text.strip_prefix('"').unwrap_or(text);
    let text = text.strip_suffix('"').unwrap_or(text);
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[cfg(feature = "async")]
    use ::paste::paste;
    use indoc::indoc;
    use io::{read_to_string, Seek};
    use std::io::Write;

    /// Generate async and sync versions of tests that get values from a given ini
    #[macro_export]
    macro_rules! read_value_eq {
        {
            $test_name:ident,
            $parser:expr,
            $ini_file_string:expr,
            $section:expr,
            $key:expr,
            $expected:expr $(,)?
        } => {
            #[test]
            fn $test_name() {
                let parser = $parser;
                let reader = std::io::Cursor::new($ini_file_string);
                let value = parser.read_value(reader, $section, $key).unwrap();
                assert_eq!(value, $expected);
            }

            #[cfg(feature = "async")]
            paste! {
                #[tokio::test]
                async fn [<$test_name _async>]() {
                    let parser = $parser;
                    let reader = std::io::Cursor::new($ini_file_string);
                    let value = parser.read_value_async(reader, $section, $key).await.unwrap();
                    assert_eq!(value, $expected);
                }
            }
        };
    }

    /// Generate async and sync versions of tests that get values from a given ini and assert that
    /// the result matches a pattern. Useful for partially matching errors.
    #[macro_export]
    macro_rules! read_value_matches {
        {
            $test_name:ident,
            $parser:expr,
            $ini_file_string:expr,
            $section:expr,
            $key:expr,
            $expected:pat $(,)?
        } => {
            #[test]
            fn $test_name() {
                let parser = $parser;
                let reader = std::io::Cursor::new($ini_file_string);
                let value = parser.read_value(reader, $section, $key);
                ::assert_matches::assert_matches!(value, $expected);
            }

            #[cfg(feature = "async")]
            paste! {
                #[tokio::test]
                async fn [<$test_name _async>]() {
                    let parser = $parser;
                    let reader = std::io::Cursor::new($ini_file_string);
                    let value = parser.read_value_async(reader, $section, $key).await;
                    ::assert_matches::assert_matches!(value, $expected);
                }
            }
        };
    }

    #[test]
    fn try_section_not() {
        assert_eq!(try_section_from_line("This is a line"), None);
    }

    #[test]
    fn try_section_no_comment() {
        assert_eq!(try_section_from_line("[SECTION]"), Some("SECTION"));
    }

    #[test]
    fn try_section_comment() {
        assert_eq!(
            try_section_from_line("[SECTION] # This is a comment"),
            Some("SECTION")
        );
    }

    #[test]
    fn try_section_whitespace() {
        assert_eq!(try_section_from_line("[ SECTION ]"), Some("SECTION"));
    }

    #[test]
    fn try_value() {
        let name_line = "  Name=John Doe  ".to_string();
        let parser = IniParser::default();

        // make sure the variable's name check works and is case sensitive
        assert!(parser.try_value(&name_line, "name").is_none());

        let value_range = parser.try_value(&name_line, "Name").unwrap();
        let mut new_name = String::new();
        new_name.push_str(&name_line[..value_range.start]);
        new_name.push_str("Ender Wiggins");
        new_name.push_str(&name_line[value_range.end..]);
        assert_eq!(new_name, "  Name=Ender Wiggins  ");
    }

    read_value_eq! {
        read_value,
        IniParser::default(),
        r#"
            first_name = "tom"
        "#,
        None,
        "first_name",
        Some("tom".to_string()),
    }

    read_value_eq! {
        read_value_section,
        IniParser::default(),
        r#"
            [user]
            first_name = "tom"
        "#,
        Some("user"),
        "first_name",
        Some("tom".to_string()),
    }

    read_value_eq! {
        read_value_no_section,
        IniParser::default(),
        r#"
            date = "10/29/2024"

            [user]
            first_name = "tom"
            date = "shouldn't get this"
        "#,
        None,
        "date",
        Some("10/29/2024".to_string()),
    }

    read_value_eq! {
        read_unquoted_string,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
        "#,
        Some("user"),
        "first_name",
        Some("tom".to_string()),
    }

    read_value_eq! {
        read_bool_true,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = true
        "#,
        Some("user"),
        "is_admin",
        Some(true),
    }

    read_value_matches! {
        read_bool_quotes,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = "true"
        "#,
        Some("user"),
        "is_admin",
        Err::<Option<bool>, _>(Error::Parse(_)),
    }

    read_value_matches! {
        read_bool_uppercase,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = TRUE
        "#,
        Some("user"),
        "is_admin",
        Ok(Some(true)),
    }
    read_value_matches! {
        read_bool_num_true,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = 1
        "#,
        Some("user"),
        "is_admin",
        Ok(Some(true)),
    }
    read_value_matches! {
        read_bool_num_false,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = 0
        "#,
        Some("user"),
        "is_admin",
        Ok(Some(false)),
    }

    read_value_eq! {
        read_bool_false,
        IniParser::default(),
        r#"
            [user]
            first_name = bill
            is_admin = false
        "#,
        Some("user"),
        "is_admin",
        Some(false),
    }

    read_value_eq! {
        read_value_multiline,
        IniParser::default(),
        r#"
            description = "a longer \
            value \
            spanning multiple \
            lines"
        "#,
        None,
        "description",
        Some("a longer value spanning multiple lines".to_string()),
    }

    /// A test ini file that has duplicate entries including a duplicate section with the same key
    const DUPLICATE_INI: &str = r#"
        [contact]
        email = test@example.com
        email = test2@example.com

        [other]
        another_key= something

        [contact]
        email = test3@example.com
    "#;

    read_value_eq! {
        read_duplicate_value_first,
        IniParser{
            duplicate_keys: DuplicateKeyStrategy::UseFirst,
            ..IniParser::default()
        },
        DUPLICATE_INI,
        Some("contact"),
        "email",
        Some("test@example.com".to_string()),
    }

    read_value_eq! {
        read_duplicate_value_last,
        IniParser{
            duplicate_keys: DuplicateKeyStrategy::UseLast,
            ..IniParser::default()
        },
        DUPLICATE_INI,
        Some("contact"),
        "email",
        Some("test3@example.com".to_string()),
    }

    read_value_matches! {
        read_duplicate_value_error,
        IniParser{
            duplicate_keys: DuplicateKeyStrategy::Error,
            ..IniParser::default()
        },
        DUPLICATE_INI,
        Some("contact"),
        "email",
        Err::<Option<String>, _>(Error::DuplicateKey{..}),
    }

    const ROUNDTRIP_INI_START: &str = r#"
        version=10
        [section_one]
    "#;
    const ROUNDTRIP_INI_END: &str = r#"
        version=11
        [section_one]
    "#;

    #[test]
    fn read_write_value_file_roundtrip() {
        let mut file = tempfile::tempfile().unwrap();
        write!(file, "{}", ROUNDTRIP_INI_START).unwrap();

        file.rewind().unwrap();
        let parser = IniParser::default();

        let version: u32 = parser.read_value(&file, None, "version").unwrap().unwrap();
        let new_version = version + 1;
        let mut destination = tempfile::tempfile().unwrap();

        file.rewind().unwrap();
        let mut buffer = std::io::BufReader::new(file);
        parser
            .write_value::<1024>(
                &mut buffer,
                &mut destination,
                None,
                "version",
                &new_version.to_string(),
            )
            .unwrap();

        destination.rewind().unwrap();
        let new = read_to_string(destination).unwrap();
        assert_eq!(new, ROUNDTRIP_INI_END);
    }

    #[test]
    fn try_value_newline() {
        let parser = IniParser::default();
        let test = "        version=10\n";
        let version = parser.try_value(test, "version").unwrap();
        assert_eq!(&test[version], "10");
    }
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
