//! Routes libchiaki's log output into one process-wide handler.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Info,
    Verbose,
    Debug,
}

impl Level {
    fn from_raw(l: c_int) -> Self {
        match l {
            1 => Level::Error,
            2 => Level::Warning,
            4 => Level::Info,
            8 => Level::Verbose,
            _ => Level::Debug,
        }
    }
}

type Handler = Box<dyn Fn(Level, &str) + Send + Sync>;

static HANDLER: RwLock<Option<Handler>> = RwLock::new(None);

/// Installs the handler that receives every libchiaki log line.
pub fn set_handler(h: impl Fn(Level, &str) + Send + Sync + 'static) {
    *HANDLER.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(h));
}

pub(crate) fn emit(level: Level, msg: &str) {
    if let Some(h) = HANDLER.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        h(level, msg);
    }
}

pub(crate) unsafe extern "C" fn trampoline(level: c_int, msg: *const c_char, _user: *mut c_void) {
    if msg.is_null() {
        return;
    }
    let text = CStr::from_ptr(msg).to_string_lossy();
    emit(Level::from_raw(level), &text);
}

pub(crate) const CB: crate::ffi::KbLogCb = Some(trampoline);
