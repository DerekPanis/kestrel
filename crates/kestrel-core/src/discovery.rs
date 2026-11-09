//! LAN discovery of PlayStation consoles (UDP 9302 for PS5).

use std::ffi::c_void;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ffi;
use crate::util::{cstring, fixed_str};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostState {
    Unknown,
    Ready,
    Standby,
}

/// A console that answered a discovery probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredHost {
    pub state: HostState,
    pub is_ps5: bool,
    pub address: String,
    pub name: String,
    /// Console MAC in hex; stable identity used to match paired consoles.
    pub host_id: String,
    pub host_type: String,
    pub system_version: String,
    pub running_app_title_id: String,
    pub running_app_name: String,
}

impl From<&ffi::KbHost> for DiscoveredHost {
    fn from(h: &ffi::KbHost) -> Self {
        DiscoveredHost {
            state: match h.state {
                1 => HostState::Ready,
                2 => HostState::Standby,
                _ => HostState::Unknown,
            },
            is_ps5: h.is_ps5 != 0,
            address: fixed_str(&h.host_addr),
            name: fixed_str(&h.host_name),
            host_id: fixed_str(&h.host_id),
            host_type: fixed_str(&h.host_type),
            system_version: fixed_str(&h.system_version),
            running_app_title_id: fixed_str(&h.running_app_titleid),
            running_app_name: fixed_str(&h.running_app_name),
        }
    }
}

type HostsCallback = dyn Fn(Vec<DiscoveredHost>) + Send + Sync;

/// A running discovery service. Probes until dropped.
pub struct Discovery {
    raw: *mut ffi::KbDiscovery,
    // Kept alive for as long as libchiaki may call back into it.
    _cb: Box<Arc<HostsCallback>>,
}

// The raw handle is only touched by Drop; libchiaki synchronises internally.
unsafe impl Send for Discovery {}

unsafe extern "C" fn hosts_trampoline(hosts: *const ffi::KbHost, count: usize, user: *mut c_void) {
    let cb = &*(user as *const Arc<HostsCallback>);
    let list = if hosts.is_null() || count == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(hosts, count)
            .iter()
            .map(DiscoveredHost::from)
            .collect()
    };
    // Never unwind across the C boundary.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb(list)));
}

impl Discovery {
    /// `manual_host`: None → broadcast on the LAN; Some(ip) → probe only that address
    /// (use for consoles on another subnet, or when broadcast is blocked).
    /// The callback receives the full current host list after every change.
    pub fn start(
        manual_host: Option<&str>,
        ping_ms: u32,
        on_hosts: impl Fn(Vec<DiscoveredHost>) + Send + Sync + 'static,
    ) -> Result<Self> {
        crate::init()?;
        let host_c = manual_host.map(|h| cstring(h, "host")).transpose()?;
        let cb: Box<Arc<HostsCallback>> = Box::new(Arc::new(on_hosts));
        let user = &*cb as *const Arc<HostsCallback> as *mut c_void;
        let mut err = 0;
        let raw = unsafe {
            ffi::kb_discovery_start(
                host_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                ping_ms,
                Some(hosts_trampoline),
                user,
                crate::log::CB,
                std::ptr::null_mut(),
                &mut err,
            )
        };
        if raw.is_null() {
            return Err(if err != 0 {
                Error::from_code(err)
            } else {
                Error::InvalidInput("discovery failed to start".into())
            });
        }
        Ok(Discovery { raw, _cb: cb })
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        // Joins the libchiaki thread; no callbacks run after this returns.
        unsafe { ffi::kb_discovery_stop(self.raw) };
    }
}

/// Sends the Remote Play wake-up packet to a paired console in rest mode.
pub fn wake(host: &str, regist_key: &[u8; ffi::KB_REGIST_KEY_SIZE], ps5: bool) -> Result<()> {
    crate::init()?;
    let host_c = cstring(host, "host")?;
    let code = unsafe {
        ffi::kb_wakeup(
            host_c.as_ptr(),
            regist_key.as_ptr() as *const _,
            ps5,
            crate::log::CB,
            std::ptr::null_mut(),
        )
    };
    Error::check(code)
}
