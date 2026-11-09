//! kestrel-core — the Remote Play engine behind Kestrel.
//!
//! Protocol work (discovery, pairing, the encrypted Takion session, FEC,
//! audio/video reassembly, controller feedback) is done by chiaki-ng's
//! libchiaki, reached through the small C bridge in `native/kbridge.c`.
//! This crate turns that into safe Rust: owned handles, callbacks routed to
//! trait objects, typed errors, and secure storage of pairing secrets.

pub mod account;
pub mod controller;
pub mod discovery;
mod error;
pub mod ffi;
pub mod log;
pub mod nal;
pub mod regist;
pub mod secure;
pub mod session;
pub mod stats;
pub mod store;
mod util;

pub use error::{Error, Result};

use std::sync::OnceLock;

static INIT: OnceLock<std::result::Result<(), i32>> = OnceLock::new();

/// Initialises libchiaki's global state (sockets on Windows, crypto, FEC
/// tables). Idempotent; every public entry point calls it.
pub fn init() -> Result<()> {
    let r = INIT.get_or_init(|| {
        let code = unsafe { ffi::kb_init() };
        if code == 0 {
            Ok(())
        } else {
            Err(code)
        }
    });
    match r {
        Ok(()) => Ok(()),
        Err(code) => Err(Error::from_code(*code)),
    }
}
