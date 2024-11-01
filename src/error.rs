use std::io;

#[derive(Debug)]
pub enum Error {
    ReadIo(io::Error),
    DuplicateKey {
        key: String,
        section: Option<String>,
    },
    Parse(Box<dyn std::error::Error>),
    TooLarge{
        limit: u64,
        found: u64,
    }
}

impl Error {
    pub(crate) fn new_parse<E: std::error::Error + 'static>(err: E) -> Self {
        Self::Parse(Box::new(err))
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ReadIo(source) => Option::Some(source),
            Error::DuplicateKey { .. } => Option::None,
            Error::Parse { .. } => Option::None,
            Error::TooLarge { .. } => Option::None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::core::fmt::Result {
        match self {
            Error::ReadIo(_) => f.write_str("IO error while reading file"),
            Error::DuplicateKey { key: name, section } => {
                write!(
                    f,
                    "duplicate key {}{} found in ini file",
                    section
                        .clone()
                        .map(|s| format!("[{s}]."))
                        .unwrap_or_default(),
                    name
                )
            }
            Error::Parse(_) => f.write_str("error while parsing value"),
            Error::TooLarge{limit, found} => write!(f, "received source with {found} bytes which exceeds the size limit of {limit}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Error::ReadIo(source)
    }
}
