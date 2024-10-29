#![allow(dead_code)]
use std::{
    io::{self, Read},
    ops::RangeInclusive,
    str::FromStr,
};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

#[derive(Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum DuplicateKeyStrategy {
    #[default]
    UseLast,
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
    pub duplicate_keys: DuplicateKeyStrategy,
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
        }
    }
}

#[derive(Debug)]
pub enum ParseError<F> {
    Io(io::Error),
    Parse(F),
}

impl<T> From<io::Error> for ParseError<T> {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl IniParser {
    /// Read a value from a INI file source.
    /// If section is none, it will look in the global space.
    pub fn read<T>(
        &self,
        source: impl Read,
        section: Option<&str>,
        name: &str,
    ) -> Result<Option<T>, ParseError<<T as FromStr>::Err>>
    where
        T: FromStr,
    {
        let value = self.value_unaltered(source, section, name)?;
        let Some(value) = value else {
            return Ok(None);
        };
        let value = trim_whitespace_and_quotes(&value);
        let value = value.parse::<T>().map_err(ParseError::Parse)?;
        Ok(Some(value))
    }

    /// Read a value from an async INI file source.
    /// If section is none, it will look in the global space.
    pub async fn read_async<T>(
        &self,
        source: impl AsyncRead,
        section: Option<&str>,
        name: &str,
    ) -> Result<Option<T>, ParseError<<T as FromStr>::Err>>
    where
        T: FromStr,
    {
        let value = self.value_unaltered_async(source, section, name).await?;
        let Some(value) = value else {
            return Ok(None);
        };
        let value = trim_whitespace_and_quotes(&value);
        let value = value.parse::<T>().map_err(ParseError::Parse)?;
        Ok(Some(value))
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
        name: &str,
    ) -> io::Result<Option<String>> {
        let buffer = std::io::BufReader::new(source);

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
            if let Some(this_section) = try_section(&line) {
                if let Some(section) = &section {
                    in_section = *section == this_section;
                } else {
                    // If section is None, we are looking for a global variable.
                    // Since this_section is some here, we know we aren't in the global section
                    in_section = false;
                }
                continue;
            } else if in_section {
                if let Some(range) = self.try_value(name, &line) {
                    match self.duplicate_keys {
                        DuplicateKeyStrategy::Error => {
                            if value.is_some() {
                                return Err(io::Error::new(
                                    io::ErrorKind::AlreadyExists,
                                    format!(
                                        "{}{name} is defined twice",
                                        section.map(|s| format!("[{s}].")).unwrap_or_default()
                                    ),
                                ));
                            }
                            value = Some(line[range].to_string());
                        }
                        DuplicateKeyStrategy::UseLast => {
                            value = Some(line[range].to_string());
                        }
                        DuplicateKeyStrategy::UseFirst => {
                            return Ok(Some(line[range].to_string()));
                        }
                    }
                }
            }
        }
        Ok(value)
    }
    /// Returns the value for the given section and name without any parsing. Notably this may
    /// still have quotation marks around strings. Leading and trailing whitespace will still be
    /// stripped though.
    ///
    /// Usually only use this if you are manually parsing something.
    async fn value_unaltered_async(
        &self,
        source: impl AsyncRead,
        section: Option<&str>,
        name: &str,
    ) -> io::Result<Option<String>> {
        let buffer = Box::pin(BufReader::new(source));

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
            if let Some(this_section) = try_section(&line) {
                if let Some(section) = &section {
                    in_section = *section == this_section;
                } else {
                    // If section is None, we are looking for a global variable.
                    // Since this_section is some here, we know we aren't in the global section
                    in_section = false;
                }
                continue;
            } else if in_section {
                if let Some(range) = self.try_value(name, &line) {
                    match self.duplicate_keys {
                        DuplicateKeyStrategy::Error => {
                            if value.is_some() {
                                return Err(io::Error::new(
                                    io::ErrorKind::AlreadyExists,
                                    format!(
                                        "{}{name} is defined twice",
                                        section.map(|s| format!("[{s}].")).unwrap_or_default()
                                    ),
                                ));
                            }
                            value = Some(line[range].to_string());
                        }
                        DuplicateKeyStrategy::UseLast => {
                            value = Some(line[range].to_string());
                        }
                        DuplicateKeyStrategy::UseFirst => {
                            return Ok(Some(line[range].to_string()));
                        }
                    }
                }
            }
        }
        Ok(value)
    }

    /// Given a string, check if it could be a
    fn try_value(&self, name: &str, line: &str) -> Option<RangeInclusive<usize>> {
        let name = name.trim();
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
                .split_at(line.char_indices().nth(delimiter_index).unwrap().0)
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
                    .nth(value_end - 1)
                    .is_some_and(|c| c.is_whitespace())
            {
                value_end -= 1;
            }
            Some(
                line.char_indices().nth(value_start).unwrap().0
                    ..=line.char_indices().nth(value_end).unwrap().0,
            )
        } else {
            // If there isn't a value delimiter, there's no value.
            None
        }
    }
}

fn try_section(line: &str) -> Option<&str> {
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
    use super::*;

    #[test]
    fn test_try_section_not() {
        assert_eq!(try_section("This is a line"), None);
    }

    #[test]
    fn test_try_section_no_comment() {
        assert_eq!(try_section("[SECTION]"), Some("SECTION"));
    }

    #[test]
    fn test_try_section_comment() {
        assert_eq!(
            try_section("[SECTION] # This is a comment"),
            Some("SECTION")
        );
    }

    #[test]
    fn test_try_section_whitespace() {
        assert_eq!(try_section("[ SECTION ]"), Some("SECTION"));
    }

    #[test]
    fn test_try_value() {
        let name = "  Name=John Doe  ".to_string();
        let parser = IniParser::default();

        // make sure the variable's name check works and is case sensitive
        assert!(parser.try_value("name", &name).is_none());

        let value_range = parser.try_value("Name", &name).unwrap();
        let mut new_name = String::new();
        new_name.push_str(&name[..*value_range.start()]);
        new_name.push_str("Ender Wiggins");
        new_name.push_str(&name[*value_range.end()..]);
        assert_eq!(new_name, "  Name=Ender Wiggins  ");
    }

    const TEST_INI: &str = r#"
        user="tom"
        [contact]
        # quoted string
        email = "test@example.com"
        # duplicate entry
        email = "test2@example.com"
        description = some longer description that \
        takes multiple lines \
        sometimes more than two

        [ database auth ] # whitespace around section names will be removed
        password=password ; an unquoted string
        "#;

    #[test]
    fn test_get_value_no_section() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read(reader, None, "user").unwrap();
        assert_eq!(value, Some("tom".to_string()));
    }

    #[tokio::test]
    async fn test_get_value_no_section_async() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read_async(reader, None, "user").await.unwrap();
        assert_eq!(value, Some("tom".to_string()));
    }
    #[test]
    fn test_get_value_multiline() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read(reader, Some("contact"), "description").unwrap();
        assert_eq!(
            value,
            Some(
                "some longer description that takes multiple lines sometimes more than two"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_get_value_section() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser
            .read(reader, Some("database auth"), "password")
            .unwrap();
        assert_eq!(value, Some("password".to_string()));
    }

    #[tokio::test]
    async fn test_get_value_section_async() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser
            .read_async(reader, Some("database auth"), "password")
            .await
            .unwrap();
        assert_eq!(value, Some("password".to_string()));
    }

    #[test]
    fn test_get_quoted_value() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read(reader, Some("contact"), "email").unwrap();
        assert_eq!(value, Some("test2@example.com".to_string()));
    }

    #[test]
    fn test_get_duplicate_value() {
        let parser = IniParser {
            duplicate_keys: DuplicateKeyStrategy::UseFirst,
            ..Default::default()
        };
        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read(reader, Some("contact"), "email").unwrap();
        assert_eq!(value, Some("test@example.com".to_string()));

        let parser = IniParser {
            duplicate_keys: DuplicateKeyStrategy::Error,
            ..Default::default()
        };
        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read::<String>(reader, Some("contact"), "email");
        assert!(value.is_err());
    }

    #[tokio::test]
    async fn test_get_value_async() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read_async(reader, None, "user").await.unwrap();
        assert_eq!(value, Some("tom".to_string()));

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser
            .read_async(reader, Some("database auth"), "password")
            .await
            .unwrap();
        assert_eq!(value, Some("password".to_string()));

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser
            .read_async(reader, Some("contact"), "email")
            .await
            .unwrap();
        assert_eq!(value, Some("test2@example.com".to_string()));

        let parser = IniParser {
            duplicate_keys: DuplicateKeyStrategy::UseFirst,
            ..Default::default()
        };
        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser
            .read_async(reader, Some("contact"), "email")
            .await
            .unwrap();
        assert_eq!(value, Some("test@example.com".to_string()));

        let parser = IniParser {
            duplicate_keys: DuplicateKeyStrategy::Error,
            ..Default::default()
        };
        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser
            .read_async::<String>(reader, Some("contact"), "email")
            .await;
        assert!(value.is_err());
    }
}
