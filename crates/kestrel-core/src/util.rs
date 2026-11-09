use std::ffi::{c_char, CStr, CString};

use crate::{Error, Result};

/// Reads a fixed-size, NUL-terminated C char array into a String.
pub(crate) fn fixed_str(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn cstring(s: &str, what: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::InvalidInput(format!("{what} contains a NUL byte")))
}

#[allow(dead_code)]
pub(crate) unsafe fn cstr_lossy(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
