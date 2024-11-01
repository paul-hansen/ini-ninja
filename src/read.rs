use crate::try_section_from_line;
use crate::DuplicateKeyStrategy;
use std::io::{BufRead, Read};

use crate::{error::Error, FromIniStr, IniParser};
#[cfg(feature = "async")]
use tokio::io::{AsyncBufReadExt, AsyncRead};

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
        let buffer = std::io::BufReader::new(source);

        // Are we in the section we are looking for?
        // Starts in the global namespace, so if section is none it starts as true, changing as we
        // parse different sections.
        let mut in_section = section.is_none();
        let mut value = None;
        let mut lines = BufRead::lines(buffer);
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

        let buffer = Box::pin(tokio::io::BufReader::new(source));

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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use crate::{try_section_from_line, DuplicateKeyStrategy};

    use super::*;
    #[cfg(feature = "async")]
    use ::paste::paste;

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
}
