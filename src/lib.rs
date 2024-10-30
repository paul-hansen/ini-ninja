#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
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
    pub fn read_value<T>(
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
    pub async fn read_value_async<T>(
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
            self.process_line(line, section, name, &mut in_section, &mut value)?;
            if self.duplicate_keys == DuplicateKeyStrategy::UseFirst && value.is_some() {
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
            self.process_line(line, section, name, &mut in_section, &mut value)?;
            if self.duplicate_keys == DuplicateKeyStrategy::UseFirst && value.is_some() {
                return Ok(value);
            }
        }
        Ok(value)
    }

    /// Mainly used to extract common functionality between async and sync implementations.
    fn process_line(
        &self,
        line: String,
        section: Option<&str>,
        name: &str,
        in_section: &mut bool,
        value: &mut Option<String>,
    ) -> io::Result<bool> {
        if let Some(this_section) = try_section(&line) {
            if let Some(section) = &section {
                *in_section = *section == this_section;
            } else {
                // If section is None, we are looking for a global variable.
                // Since this_section is some here, we know we aren't in the global section
                *in_section = false;
            }
            return Ok(true);
        } else if *in_section {
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
                        *value = Some(line[range].to_string());
                    }
                    _ => {
                        *value = Some(line[range].to_string());
                    }
                }
            }
        }

        Ok(false)
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
                    .nth(value_end - 1)
                    .is_some_and(|c| c.is_whitespace())
            {
                value_end -= 1;
            }
            Some(line.char_indices().nth(value_start)?.0..=line.char_indices().nth(value_end)?.0)
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
    #![allow(clippy::unwrap_used)]
    use ::paste::paste;

    /// Generate async and sync versions of tests that get values from a given ini
    #[macro_export]
    macro_rules! get_value_eq {
        {
            $test_name:ident,
            $parser:expr,
            $ini_file_string:expr,
            $section:expr,
            $key:expr,
            $expected:expr
        } => {
            #[test]
            fn $test_name() {
                let parser = $parser;
                let reader = std::io::Cursor::new($ini_file_string);
                let value = parser.read_value(reader, $section, $key).unwrap();
                assert_eq!(value, $expected);
            }

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
    macro_rules! get_value_matches {
        {
            $test_name:ident,
            $parser:expr,
            $ini_file_string:expr,
            $section:expr,
            $key:expr,
            $expected:pat
        } => {
            #[test]
            fn $test_name() {
                let parser = $parser;
                let reader = std::io::Cursor::new($ini_file_string);
                let value = parser.read_value(reader, $section, $key);
                ::assert_matches::assert_matches!(value, $expected);
            }

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

    use super::*;

    #[test]
    fn try_section_not() {
        assert_eq!(try_section("This is a line"), None);
    }

    #[test]
    fn try_section_no_comment() {
        assert_eq!(try_section("[SECTION]"), Some("SECTION"));
    }

    #[test]
    fn try_section_comment() {
        assert_eq!(
            try_section("[SECTION] # This is a comment"),
            Some("SECTION")
        );
    }

    #[test]
    fn try_section_whitespace() {
        assert_eq!(try_section("[ SECTION ]"), Some("SECTION"));
    }

    #[test]
    fn try_value() {
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

    get_value_eq! {
        get_value,
        IniParser::default(),
        r#"
            first_name = "tom"
        "#,
        None,
        "first_name",
        Some("tom".to_string())
    }

    get_value_eq! {
        get_value_section,
        IniParser::default(),
        r#"
            [user]
            first_name = "tom"
        "#,
        Some("user"),
        "first_name",
        Some("tom".to_string())
    }

    get_value_eq! {
        get_value_no_section,
        IniParser::default(),
        r#"
            date = "10/29/2024"

            [user]
            first_name = "tom"
            date = "shouldn't get this"
        "#,
        None,
        "date",
        Some("10/29/2024".to_string())
    }

    get_value_eq! {
        get_unquoted_string,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
        "#,
        Some("user"),
        "first_name",
        Some("tom".to_string())
    }

    get_value_eq! {
        get_bool_true,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = true
        "#,
        Some("user"),
        "is_admin",
        Some(true)
    }

    get_value_eq! {
        get_bool_quotes,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = "true"
        "#,
        Some("user"),
        "is_admin",
        Some(true)
    }

    get_value_matches! {
        get_bool_uppercase,
        IniParser::default(),
        r#"
            [user]
            first_name = tom
            is_admin = "TRUE"
        "#,
        Some("user"),
        "is_admin",
        Ok(Some(true))
    }

    get_value_eq! {
        get_bool_false,
        IniParser::default(),
        r#"
            [user]
            first_name = bill
            is_admin = false
        "#,
        Some("user"),
        "is_admin",
        Some(false)
    }
    
    get_value_eq! {
        get_value_multiline,
        IniParser::default(),
        r#"
            description = "a longer \
            value \
            spanning multiple \
            lines"
        "#,
        None,
        "description",
        Some("a longer value spanning multiple lines".to_string())
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

    get_value_eq! {
        get_duplicate_value_first,
        IniParser{
            duplicate_keys: DuplicateKeyStrategy::UseFirst,
            ..IniParser::default()
        },
        DUPLICATE_INI,
        Some("contact"),
        "email",
        Some("test@example.com".to_string())
    }

    get_value_eq! {
        get_duplicate_value_last,
        IniParser{
            duplicate_keys: DuplicateKeyStrategy::UseLast,
            ..IniParser::default()
        },
        DUPLICATE_INI,
        Some("contact"),
        "email",
        Some("test3@example.com".to_string())
    }

    get_value_matches! {
        get_duplicate_value_error,
        IniParser{
            duplicate_keys: DuplicateKeyStrategy::Error,
            ..IniParser::default()
        },
        DUPLICATE_INI,
        Some("contact"),
        "email",
        Err::<Option<String>, _>(ParseError::Io(_))
    }
}
