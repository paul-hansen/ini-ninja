//! # INI Ninja 🥷
//!
//! Get and set values from INI files while preserving the file's comments and formatting.
//!
//! ## Features
//!
//! - Custom parsing logic written in pure rust, no slow regex found here.
//! - Can handle large files with low memory use, never needs to have the whole file in ram at once.
//! - Async and sync versions of read and write functions.
//! - Tests, CI, all the good things to make sure the code quality stays consistent in the future.
//! - No dependencies.
//!
//! ## Examples
//!
//! Read a value from a [`File`](std::fs::File)
//!
//! ```no_run
//! # use ini_ninja::IniParser;
//! # use std::fs::File;
//! # fn main () -> Result<(), ini_ninja::Error> {
//! let ini_file = File::open("../examples/ini_files/conan_exiles/DefaultGame.ini")?;
//!
//! // The default parser should work with most ini files
//! let parser = IniParser::default();
//! let max_players: Option<usize> = parser
//!    .read_value(ini_file, Some("/Script/Engine.GameSession"), "MaxPlayers")?;
//!
//! assert_eq!(max_players, Some(40));
//! # Ok(())
//! # }
//! ```
//!
//! Write a value to a [`File`](std::fs::File)
//!
//! ```no_run
//! # use ini_ninja::IniParser;
//! # use std::fs::File;
//! # use std::io::BufReader;
//! # fn main () -> Result<(), ini_ninja::Error> {
//! let ini_file = File::open("file/path")?;
//! let mut read_buffer = BufReader::new(ini_file);
//! // We'll first write the changes to a temporary file.
//! let temp = tempfile::NamedTempFile::new()?;
//!
//! let parser = IniParser::default();
//! parser.write_value(&mut read_buffer, &temp, Some("section"), "key", "Hello World")?;
//!
//! // now we tell the OS to replace the original file with our modified version.
//! std::fs::rename(temp.path(), "file/path");
//! # Ok(())
//! # }
//! ```
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
mod error;
mod read;
#[cfg(test)]
mod test_helpers;
mod write;
pub use error::Error;
use std::{ops::Range, str::FromStr};
#[cfg(doctest)]
mod readme_tests;

pub trait FromIniStr: Sized {
    type Err: std::error::Error + Send + Sync + 'static;
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
impl_from_ini_str!(usize);
impl_from_ini_str!(isize);
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

/// Parses and writes values to INI files with the provided settings.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct IniParser {
    /// Characters that indicate the start of a comment.
    pub comment_delimiters: &'static [char],
    /// Are comments supported after a key=value on the same line?
    pub trailing_comments: bool,
    /// Character that will be used to split the key and value.
    /// It's very uncommon that this isn't `=`.
    pub value_start_delimiters: &'static [char],
    /// If true, lines ending with `\` will consider the next line part of the
    /// current line. This allows multiline values.
    pub line_continuation: bool,
    /// How should we handle duplicate keys in the ini file?
    pub duplicate_keys: DuplicateKeyStrategy,
}

impl Default for IniParser {
    /// The defaults are chosen to be compatible with the widest range of ini formats.
    fn default() -> Self {
        Self {
            comment_delimiters: &['#', ';'],
            trailing_comments: true,
            value_start_delimiters: &['='],
            line_continuation: true,
            duplicate_keys: DuplicateKeyStrategy::default(),
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

            // Start byte position
            let start = line
                .char_indices()
                .nth(value_start)
                .map(|(idx, _)| idx)
                .unwrap_or_else(|| {
                    // If we couldn't get the start position (reached end of string),
                    // use the position right after the delimiter
                    line.char_indices()
                        .nth(delimiter_index + 1)
                        .map(|(idx, _)| idx)
                        .unwrap_or(line.len())
                });

            // Find the last non-whitespace character for the end position
            let end = line[start..]
                .char_indices()
                .rev()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(idx, c)| start + idx + c.len_utf8())
                .unwrap_or(start);

            Some(start..end)
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
    use std::io::{Seek, read_to_string};

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
            .write_value(
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

    #[test]
    fn test_try_value_empty_value() {
        let parser = IniParser::default();
        let line = "name=\n";
        let range = parser.try_value(line, "name").unwrap();
        let mut s = String::from(line);
        s.replace_range(range, "a");
        assert_eq!(s, "name=a\n");
    }

    #[test]
    fn test_try_value_range() {
        let parser = IniParser::default();
        let line = "name=bob\n";
        let range = parser.try_value(line, "name").unwrap();
        let mut s = String::from(line);
        s.replace_range(range, "bill");
        assert_eq!(s, "name=bill\n");
    }

    #[test]
    fn test_try_value_range_no_newline() {
        let parser = IniParser::default();
        let line = "name=bob";
        let range = parser.try_value(line, "name").unwrap();
        let mut s = String::from(line);
        s.replace_range(range, "bill");
        assert_eq!(s, "name=bill");
    }
}
