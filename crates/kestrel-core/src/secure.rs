//! Secret handling: in-memory wrapper and at-rest protection.
//!
//! At rest on Windows, pairing secrets are encrypted with DPAPI
//! (CryptProtectData, user scope): only the same Windows user on the same
//! machine can decrypt them, and nothing secret is ever written in clear.
//! On other platforms (development builds only) secrets are stored unencrypted
//! in a file readable only by the owner, and the store marks them as such.

use std::fmt;

#[cfg(windows)]
use crate::Error;
use crate::Result;

/// A fixed-size secret that is zeroed on drop and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret<const N: usize>([u8; N]);

impl<const N: usize> Secret<N> {
    pub fn new(bytes: [u8; N]) -> Self {
        Secret(bytes)
    }
    pub fn expose(&self) -> &[u8; N] {
        &self.0
    }
}

impl<const N: usize> Drop for Secret<N> {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // Volatile so the compiler cannot elide the wipe.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

impl<const N: usize> fmt::Debug for Secret<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret<{N}>(<redacted>)")
    }
}

/// Name of the at-rest protection scheme used on this platform.
pub fn scheme() -> &'static str {
    if cfg!(windows) {
        "dpapi"
    } else {
        "plain-0600"
    }
}

#[cfg(windows)]
pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 || output.pbData.is_null() {
        return Err(Error::Store("CryptProtectData failed".into()));
    }
    let out = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData as _) };
    Ok(out)
}

#[cfg(windows)]
pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: blob.len() as u32,
        pbData: blob.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 || output.pbData.is_null() {
        return Err(Error::Store(
            "CryptUnprotectData failed (pairing was made by another Windows user or PC)".into(),
        ));
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
    let out = slice.to_vec();
    slice.fill(0);
    unsafe { LocalFree(output.pbData as _) };
    Ok(out)
}

#[cfg(not(windows))]
pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>> {
    Ok(blob.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let data = b"regist-key-and-morning-material";
        let blob = protect(data).unwrap();
        if cfg!(windows) {
            assert_ne!(&blob[..], &data[..], "DPAPI output must not be plaintext");
        }
        assert_eq!(unprotect(&blob).unwrap(), data);
    }

    #[test]
    fn secret_debug_redacted() {
        let s = Secret::new([0x41u8; 16]);
        assert!(!format!("{s:?}").contains("41"));
        assert!(!format!("{s:?}").contains('A'));
    }
}
