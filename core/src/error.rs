use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Failed to access URL: {0}")]
    UrlAccess(String),

    #[error("Failed to get Content-Length header")]
    NoContentLength,

    #[error("ZIP error: {0}")]
    Zip(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

impl From<async_zip::error::ZipError> for Error {
    fn from(e: async_zip::error::ZipError) -> Self {
        Error::Zip(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::InvalidUrl("test".to_string());
        assert_eq!(err.to_string(), "Invalid URL: test");

        let err = Error::FileNotFound("foo.txt".to_string());
        assert_eq!(err.to_string(), "File not found: foo.txt");
    }
}
