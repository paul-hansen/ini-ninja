use std::io;

#[derive(Debug)]
pub enum Error {
    ReadIo(io::Error),
    DuplicateKey {
        key: String,
        section: Option<String>,
    },
    Parse(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    pub(crate) fn new_parse<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        Self::Parse(Box::new(err))
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ReadIo(source) => Option::Some(source),
            Error::DuplicateKey { .. } => Option::None,
            Error::Parse(err) => Some(err.as_ref()),
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
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Error::ReadIo(source)
    }
}
