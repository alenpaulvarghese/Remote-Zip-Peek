use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Network error: {}", network_cause(.0))]
    Http(reqwest::Error),

    #[error("The server returned HTTP {status}")]
    HttpStatus { status: u16 },

    #[error(
        "The server doesn't support partial downloads (HTTP Range requests), so this archive can't be browsed remotely"
    )]
    RangeNotSupported,

    #[error("The archive changed on the server. Scan it again")]
    RemoteChanged,

    #[error("The server sent an invalid response: {0}")]
    InvalidResponse(&'static str),

    #[error("The connection ended before all the data arrived")]
    TruncatedResponse,

    #[error("This file isn't a ZIP archive, or the end of it is damaged")]
    NotAZip,

    #[error("This archive is too large to open safely: {0}")]
    LimitExceeded(&'static str),

    #[error("ZIP error: {0}")]
    Zip(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("{path} is corrupted: its checksum doesn't match")]
    CrcMismatch { path: String },

    #[error("{path} is corrupted: expected {expected} bytes but got {actual}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },

    #[error("{path} looks malicious: {reason}")]
    SuspiciousEntry { path: String, reason: &'static str },

    #[error("{path} can't be extracted: its compression method isn't supported ({method})")]
    UnsupportedCompression { path: String, method: &'static str },

    #[error("IO error: {0}")]
    Io(std::io::Error),

    #[error("Unsafe entry \"{entry}\": {reason}")]
    UnsafeEntry { entry: String, reason: &'static str },

    #[error("Cancelled")]
    Cancelled,
}

/// The most specific reason available, such as "Connection refused", since a request error's own
/// message only says that the request failed.
fn network_cause(error: &reqwest::Error) -> String {
    let mut cause: &dyn std::error::Error = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}

impl From<reqwest::Error> for Error {
    /// Drops the URL from the error, since it can contain a signed token.
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e.without_url())
    }
}

/// Recovers an error that passed through an I/O interface intact.
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.get_ref().is_some_and(|inner| inner.is::<Error>()) {
            if let Some(inner) = e.into_inner() {
                if let Ok(error) = inner.downcast::<Error>() {
                    return *error;
                }
            }
            unreachable!("the inner error was checked above");
        }
        Error::Io(e)
    }
}

impl From<Error> for std::io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(e) => e,
            other => std::io::Error::other(other),
        }
    }
}

impl From<async_zip::error::ZipError> for Error {
    fn from(e: async_zip::error::ZipError) -> Self {
        use async_zip::error::ZipError;
        match e {
            ZipError::UpstreamReadError(e) => e.into(),
            ZipError::UnableToLocateEOCDR => Error::NotAZip,
            other => Error::Zip(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::HttpStatus { status: 404 };
        assert_eq!(err.to_string(), "The server returned HTTP 404");

        let err = Error::FileNotFound("foo.txt".to_string());
        assert_eq!(err.to_string(), "File not found: foo.txt");
    }

    #[test]
    fn typed_errors_survive_an_io_error_round_trip() {
        let io: std::io::Error = Error::RangeNotSupported.into();
        assert!(matches!(Error::from(io), Error::RangeNotSupported));

        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let round_trip: std::io::Error = Error::from(io).into();
        assert_eq!(round_trip.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn zip_errors_recover_the_upstream_error() {
        let upstream = async_zip::error::ZipError::UpstreamReadError(Error::RemoteChanged.into());
        assert!(matches!(Error::from(upstream), Error::RemoteChanged));
        assert!(matches!(
            Error::from(async_zip::error::ZipError::UnableToLocateEOCDR),
            Error::NotAZip
        ));
    }
}
