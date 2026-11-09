use std::ffi::CStr;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("libchiaki error {code}: {message}")]
    Chiaki { code: i32, message: String },
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("pairing failed: the console refused or did not answer (see the log above)")]
    RegistFailed,
    #[error("pairing was canceled")]
    RegistCanceled,
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("storage error: {0}")]
    Store(String),
}

impl Error {
    pub fn from_code(code: i32) -> Self {
        let message = unsafe {
            let p = crate::ffi::kb_error_string(code);
            if p.is_null() {
                "unknown".to_string()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        Error::Chiaki { code, message }
    }

    pub(crate) fn check(code: i32) -> Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(Error::from_code(code))
        }
    }
}
