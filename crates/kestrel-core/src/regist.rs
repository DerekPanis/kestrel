//! Pairing ("registration") with a console via the Link Device PIN.

use std::ffi::{c_int, c_void};
use std::sync::mpsc;
use std::time::Duration;

use crate::account::AccountId;
use crate::ffi;
use crate::secure::Secret;
use crate::util::{cstring, fixed_str};
use crate::{Error, Result};

/// What a successful pairing yields. `regist_key` and `morning` are the
/// long-lived credentials for this PC↔console pair and must be stored securely.
#[derive(Clone)]
pub struct Pairing {
    pub target: i32,
    pub nickname: String,
    pub mac: [u8; 6],
    pub regist_key: Secret<16>,
    pub morning: Secret<16>,
    pub rp_key_type: u32,
    pub console_pin: u32,
}

impl Pairing {
    pub fn is_ps5(&self) -> bool {
        self.target >= 1_000_000
    }
    pub fn mac_hex(&self) -> String {
        crate::util::hex(&self.mac)
    }
}

type Outcome = std::result::Result<Pairing, Error>;

unsafe extern "C" fn regist_trampoline(
    status: c_int,
    res: *const ffi::KbRegistResult,
    user: *mut c_void,
) {
    let tx = &*(user as *const mpsc::SyncSender<Outcome>);
    let outcome = match status {
        ffi::KB_REGIST_SUCCESS if !res.is_null() => {
            let r = &*res;
            let mut regist_key = [0u8; 16];
            for (d, s) in regist_key.iter_mut().zip(r.rp_regist_key.iter()) {
                *d = *s as u8;
            }
            Ok(Pairing {
                target: r.target,
                nickname: fixed_str(&r.server_nickname),
                mac: r.server_mac,
                regist_key: Secret::new(regist_key),
                morning: Secret::new(r.rp_key),
                rp_key_type: r.rp_key_type,
                console_pin: r.console_pin,
            })
        }
        ffi::KB_REGIST_CANCELED => Err(Error::RegistCanceled),
        _ => Err(Error::RegistFailed),
    };
    let _ = tx.try_send(outcome);
}

/// Pairs with the console at `host`. The PS5 must be on
/// Settings → System → Remote Play → Link Device, showing `pin`.
/// Blocks until the console answers or `timeout` elapses.
pub fn pair(
    host: &str,
    ps5: bool,
    account: AccountId,
    pin: u32,
    console_pin: u32,
    timeout: Duration,
) -> Result<Pairing> {
    crate::init()?;
    let host_c = cstring(host, "host")?;
    let broadcast = host == "255.255.255.255";
    let (tx, rx) = mpsc::sync_channel::<Outcome>(1);
    let tx = Box::new(tx);
    let user = &*tx as *const mpsc::SyncSender<Outcome> as *mut c_void;
    let mut err = 0;
    let raw = unsafe {
        ffi::kb_regist_start(
            host_c.as_ptr(),
            broadcast,
            ps5,
            account.0.as_ptr(),
            pin,
            console_pin,
            Some(regist_trampoline),
            user,
            crate::log::CB,
            std::ptr::null_mut(),
            &mut err,
        )
    };
    if raw.is_null() {
        return Err(Error::from_code(err));
    }
    let result = match rx.recv_timeout(timeout) {
        Ok(outcome) => outcome,
        Err(_) => Err(Error::Timeout(
            "the console did not answer the pairing request".into(),
        )),
    };
    // Stops the worker if still running and joins it; the callback cannot fire afterwards,
    // so dropping `tx` below is safe.
    unsafe { ffi::kb_regist_free(raw) };
    drop(tx);
    result
}
