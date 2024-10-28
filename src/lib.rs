#![allow(dead_code)]
use std::{
    io::{self, BufRead, BufReader, Read},
    ops::RangeInclusive,
    str::FromStr,
};

#[derive(Default, Copy, Clone, PartialEq, Eq, Hash)]
pub enum DuplicateStrategy {
    #[default]
    Last,
    First,
    Error,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct IniParser {
    /// These characters indicate the start of a comment.
    pub comment_delimiters: Vec<char>,
    pub trailing_comments: bool,
    pub value_start_delimiters: Vec<char>,
    pub multiline: bool,
    pub duplicate_strategy: DuplicateStrategy,
}

impl Default for IniParser {
    fn default() -> Self {
        Self {
            comment_delimiters: vec!['#', ';'],
            trailing_comments: false,
            value_start_delimiters: vec!['='],
            multiline: true,
            duplicate_strategy: DuplicateStrategy::default(),
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
    pub fn read_parsed<T>(
        &self,
        source: impl Read,
        section: Option<&str>,
        name: &str,
    ) -> Result<Option<T>, ParseError<<T as FromStr>::Err>>
    where
        T: FromStr,
    {
        let value = self.value_quotes_removed(source, section, name)?;
        let Some(value) = value else {
            return Ok(None);
        };
        let value = value.parse::<T>().map_err(ParseError::Parse)?;
        Ok(Some(value))
    }

    pub fn read_string(
        &self,
        source: impl Read,
        section: Option<&str>,
        name: &str,
    ) -> io::Result<Option<String>> {
        self.value_quotes_removed(source, section, name)
    }

    fn value_quotes_removed(
        &self,
        source: impl Read,
        section: Option<&str>,
        name: &str,
    ) -> io::Result<Option<String>> {
        let value = self.value_unaltered(source, section, name)?.map(|s| {
            let s = s.trim();
            let s = s.strip_prefix('"').unwrap_or(s);
            let s = s.strip_suffix('"').unwrap_or(s);
            s.to_string()
        });
        Ok(value)
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
        let buffer = BufReader::new(source);

        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut value = None;

        for line in buffer.lines() {
            let line = line?;
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
                    match self.duplicate_strategy {
                        DuplicateStrategy::Error => {
                            if value.is_some() {
                                return Err(io::Error::new(
                                    io::ErrorKind::AlreadyExists,
                                    format!("{}{name} is defined twice", section.map(|s|format!("[{s}].")).unwrap_or_default()),
                                ));
                            }
                            value = Some(line[range].to_string());
                        }
                        DuplicateStrategy::Last => {
                            value = Some(line[range].to_string());
                        }
                        DuplicateStrategy::First => {
                            return Ok(Some(line[range].to_string()));
                        }
                    }
                }
            }
        }
        Ok(value)
    }

    fn try_value(&self, name: &str, line: &str) -> Option<RangeInclusive<usize>> {
        let name = name.trim();
        // Since comments are always at the end of the line, it won't change the positions to
        // remove them.
        let line = line
            .split_once(&*self.comment_delimiters)
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
        Some(&trimmed[1..end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_section() {
        assert_eq!(try_section("This is a line"), None);
        assert_eq!(try_section("[SECTION] This is a line"), Some("SECTION"));
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
        [info]
        email = "test@example.com"
        email = "test2@example.com"
        password=password
        "#;
    #[test]
    fn test_get_value() {
        let parser = IniParser::default();

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read_parsed(reader, None, "user").unwrap();
        assert_eq!(value, Some("tom".to_string()));

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser
            .read_parsed(reader, Some("info"), "password")
            .unwrap();
        assert_eq!(value, Some("password".to_string()));

        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read_parsed(reader, Some("info"), "email").unwrap();
        assert_eq!(value, Some("test2@example.com".to_string()));

        let parser = IniParser{
            duplicate_strategy: DuplicateStrategy::First,
            ..Default::default()};
        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read_parsed(reader, Some("info"), "email").unwrap();
        assert_eq!(value, Some("test@example.com".to_string()));

        let parser = IniParser{
            duplicate_strategy: DuplicateStrategy::Error,
            ..Default::default()};
        let reader = std::io::Cursor::new(TEST_INI);
        let value = parser.read_parsed::<String>(reader, Some("info"), "email");
        assert!(value.is_err());
    }
}
