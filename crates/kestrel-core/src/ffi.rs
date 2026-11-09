//! Raw bindings to `native/kbridge.h`. Every struct mirrors the C layout 1:1;
//! `tests::layouts_match_c` checks the sizes against the compiled library.

#![allow(non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

pub const KB_ACCOUNT_ID_SIZE: usize = 8;
pub const KB_REGIST_KEY_SIZE: usize = 16;
pub const KB_MORNING_SIZE: usize = 16;

pub type KbLogCb =
    Option<unsafe extern "C" fn(level: c_int, msg: *const c_char, user: *mut c_void)>;

#[repr(C)]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct KbStructSizes {
    pub host: u32,
    pub regist_result: u32,
    pub session_config: u32,
    pub event: u32,
    pub controller: u32,
    pub stats: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KbHost {
    pub state: i32,
    pub is_ps5: i32,
    pub request_port: u16,
    pub host_addr: [c_char; 64],
    pub host_name: [c_char; 64],
    pub host_id: [c_char; 32],
    pub host_type: [c_char; 16],
    pub system_version: [c_char; 16],
    pub running_app_titleid: [c_char; 32],
    pub running_app_name: [c_char; 128],
}

pub type KbHostsCb =
    Option<unsafe extern "C" fn(hosts: *const KbHost, count: usize, user: *mut c_void)>;

#[repr(C)]
pub struct KbDiscovery {
    _private: [u8; 0],
}

pub const KB_REGIST_CANCELED: c_int = 0;
pub const KB_REGIST_FAILED: c_int = 1;
pub const KB_REGIST_SUCCESS: c_int = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KbRegistResult {
    pub target: i32,
    pub rp_key_type: u32,
    pub console_pin: u32,
    pub server_mac: [u8; 6],
    pub server_nickname: [c_char; 32],
    pub rp_regist_key: [c_char; KB_REGIST_KEY_SIZE],
    pub rp_key: [u8; KB_MORNING_SIZE],
}

pub type KbRegistCb =
    Option<unsafe extern "C" fn(status: c_int, result: *const KbRegistResult, user: *mut c_void)>;

#[repr(C)]
pub struct KbRegist {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KbSessionConfig {
    pub host: *const c_char,
    pub ps5: i32,
    pub resolution: i32,
    pub fps: i32,
    pub bitrate_kbps: u32,
    pub codec: i32,
    pub enable_dualsense: i32,
    pub packet_loss_max: f64,
    pub regist_key: [c_char; KB_REGIST_KEY_SIZE],
    pub morning: [u8; KB_MORNING_SIZE],
    pub psn_account_id: [u8; KB_ACCOUNT_ID_SIZE],
}

pub const KB_EVENT_CONNECTED: i32 = 0;
pub const KB_EVENT_LOGIN_PIN_REQUEST: i32 = 1;
pub const KB_EVENT_QUIT: i32 = 2;
pub const KB_EVENT_RUMBLE: i32 = 3;
pub const KB_EVENT_TRIGGER_EFFECTS: i32 = 4;
pub const KB_EVENT_LED_COLOR: i32 = 5;
pub const KB_EVENT_PLAYER_INDEX: i32 = 6;
pub const KB_EVENT_NICKNAME: i32 = 7;
pub const KB_EVENT_VIDEO_FEC_FAILURE: i32 = 8;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KbEvent {
    pub type_: i32,
    pub quit_reason: i32,
    pub quit_is_error: i32,
    pub quit_reason_str: [c_char; 256],
    pub pin_incorrect: i32,
    pub rumble_left: u8,
    pub rumble_right: u8,
    pub trigger_type_left: u8,
    pub trigger_type_right: u8,
    pub trigger_left: [u8; 10],
    pub trigger_right: [u8; 10],
    pub led: [u8; 3],
    pub player_index: u8,
    pub nickname: [c_char; 32],
}

pub type KbVideoCb = Option<
    unsafe extern "C" fn(
        buf: *const u8,
        size: usize,
        frames_lost: i32,
        recovered: bool,
        user: *mut c_void,
    ) -> bool,
>;
pub type KbEventCb = Option<unsafe extern "C" fn(event: *const KbEvent, user: *mut c_void)>;
pub type KbAudioSettingsCb =
    Option<unsafe extern "C" fn(channels: u32, rate: u32, user: *mut c_void)>;
pub type KbAudioFrameCb =
    Option<unsafe extern "C" fn(pcm: *const i16, samples: usize, user: *mut c_void)>;
pub type KbHapticsCb = Option<unsafe extern "C" fn(buf: *const u8, size: usize, user: *mut c_void)>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KbSessionCallbacks {
    pub user: *mut c_void,
    pub event: KbEventCb,
    pub video: KbVideoCb,
    pub audio_settings: KbAudioSettingsCb,
    pub audio_frame: KbAudioFrameCb,
    pub haptics: KbHapticsCb,
    pub log: KbLogCb,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct KbControllerState {
    pub buttons: u32,
    pub l2: u8,
    pub r2: u8,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
    pub touch_id: [i8; 2],
    pub touch_x: [u16; 2],
    pub touch_y: [u16; 2],
    pub gyro_x: f32,
    pub gyro_y: f32,
    pub gyro_z: f32,
    pub accel_x: f32,
    pub accel_y: f32,
    pub accel_z: f32,
    pub orient_x: f32,
    pub orient_y: f32,
    pub orient_z: f32,
    pub orient_w: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct KbStats {
    pub rtt_us: u64,
    pub measured_bitrate_mbps: f64,
    pub packet_loss: f64,
    pub width: i32,
    pub height: i32,
    pub fps: i32,
    pub bitrate_kbps: i32,
    pub codec: i32,
}

#[repr(C)]
pub struct KbSession {
    _private: [u8; 0],
}

extern "C" {
    pub fn kb_error_string(code: c_int) -> *const c_char;
    pub fn kb_init() -> c_int;
    pub fn kb_struct_sizes(out: *mut KbStructSizes);

    pub fn kb_discovery_start(
        manual_host: *const c_char,
        ping_ms: u32,
        cb: KbHostsCb,
        cb_user: *mut c_void,
        log_cb: KbLogCb,
        log_user: *mut c_void,
        err_out: *mut c_int,
    ) -> *mut KbDiscovery;
    pub fn kb_discovery_stop(d: *mut KbDiscovery);

    pub fn kb_wakeup(
        host: *const c_char,
        regist_key: *const c_char,
        ps5: bool,
        log_cb: KbLogCb,
        log_user: *mut c_void,
    ) -> c_int;

    pub fn kb_regist_start(
        host: *const c_char,
        broadcast: bool,
        ps5: bool,
        account_id: *const u8,
        pin: u32,
        console_pin: u32,
        cb: KbRegistCb,
        cb_user: *mut c_void,
        log_cb: KbLogCb,
        log_user: *mut c_void,
        err_out: *mut c_int,
    ) -> *mut KbRegist;
    pub fn kb_regist_free(r: *mut KbRegist);

    pub fn kb_session_create(
        cfg: *const KbSessionConfig,
        cbs: *const KbSessionCallbacks,
        err_out: *mut c_int,
    ) -> *mut KbSession;
    pub fn kb_session_start(s: *mut KbSession) -> c_int;
    pub fn kb_session_stop(s: *mut KbSession) -> c_int;
    pub fn kb_session_join(s: *mut KbSession) -> c_int;
    pub fn kb_session_free(s: *mut KbSession);
    pub fn kb_session_set_controller(s: *mut KbSession, state: *const KbControllerState) -> c_int;
    pub fn kb_session_set_login_pin(s: *mut KbSession, pin: *const c_char) -> c_int;
    pub fn kb_session_request_idr(s: *mut KbSession) -> c_int;
    pub fn kb_session_goto_bed(s: *mut KbSession) -> c_int;
    pub fn kb_session_go_home(s: *mut KbSession) -> c_int;
    pub fn kb_session_get_stats(s: *mut KbSession, out: *mut KbStats);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn layouts_match_c() {
        let mut c = KbStructSizes::default();
        unsafe { kb_struct_sizes(&mut c) };
        let rust = KbStructSizes {
            host: size_of::<KbHost>() as u32,
            regist_result: size_of::<KbRegistResult>() as u32,
            session_config: size_of::<KbSessionConfig>() as u32,
            event: size_of::<KbEvent>() as u32,
            controller: size_of::<KbControllerState>() as u32,
            stats: size_of::<KbStats>() as u32,
        };
        assert_eq!(c, rust, "ffi.rs struct layouts drifted from kbridge.h");
    }
}
