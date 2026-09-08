use thiserror::Error;

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct Error {
    code: &'static str,
    message: String,
}

impl Error {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

pub type Result<T> = std::result::Result<T, Error>;
