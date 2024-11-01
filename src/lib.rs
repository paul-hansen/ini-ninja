#![doc = include_str!("../README.md")]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
mod error;
mod read;
mod write;
use std::{ops::Range, str::FromStr};

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
    use std::io::Write;
    use std::io::{read_to_string, Seek};

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
}
