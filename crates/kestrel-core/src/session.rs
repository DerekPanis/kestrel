//! A live Remote Play session.
//!
//! libchiaki runs the session on its own threads and calls back with events,
//! complete video access units and decoded PCM audio. Those callbacks are
//! forwarded to a [`SessionSink`] supplied by the caller. Sink methods run on
//! libchiaki threads: keep them short and never drop the [`Session`] from
//! inside one (dropping joins those threads).

use std::ffi::{c_void, CString};
use std::sync::atomic::Ordering;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::account::AccountId;
use crate::controller::ControllerState;
use crate::ffi;
use crate::nal::{self, Codec};
use crate::secure::Secret;
use crate::stats::{self, FrameCounters, Quality, RateMeter};
use crate::util::{cstring, fixed_str};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    #[serde(rename = "360p")]
    P360,
    #[serde(rename = "540p")]
    P540,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
}

impl Resolution {
    fn raw(self) -> i32 {
        match self {
            Resolution::P360 => 1,
            Resolution::P540 => 2,
            Resolution::P720 => 3,
            Resolution::P1080 => 4,
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "360p" | "360" => Some(Resolution::P360),
            "540p" | "540" => Some(Resolution::P540),
            "720p" | "720" => Some(Resolution::P720),
            "1080p" | "1080" => Some(Resolution::P1080),
            _ => None,
        }
    }
}

pub struct SessionConfig {
    pub host: String,
    pub ps5: bool,
    pub resolution: Resolution,
    /// 30 or 60.
    pub fps: u32,
    /// None → the preset's default bitrate.
    pub bitrate_kbps: Option<u32>,
    pub codec: Codec,
    /// Receive adaptive-trigger events and the haptics audio stream.
    pub dualsense: bool,
    /// Packet loss fraction above which libchiaki reports congestion.
    pub packet_loss_max: f64,
    pub regist_key: Secret<16>,
    pub morning: Secret<16>,
    pub account: AccountId,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionEvent {
    Connected,
    /// The PS5 user has a login passcode; answer with `Session::set_login_pin`.
    LoginPinRequested {
        incorrect: bool,
    },
    Quit {
        reason: i32,
        message: String,
        is_error: bool,
    },
    Rumble {
        left: u8,
        right: u8,
    },
    TriggerEffects {
        type_left: u8,
        type_right: u8,
        left: [u8; 10],
        right: [u8; 10],
    },
    LedColor {
        rgb: [u8; 3],
    },
    PlayerIndex {
        index: u8,
    },
    Nickname {
        name: String,
    },
    VideoFecFailure,
}

impl SessionEvent {
    fn from_raw(e: &ffi::KbEvent) -> Option<Self> {
        Some(match e.type_ {
            ffi::KB_EVENT_CONNECTED => SessionEvent::Connected,
            ffi::KB_EVENT_LOGIN_PIN_REQUEST => SessionEvent::LoginPinRequested {
                incorrect: e.pin_incorrect != 0,
            },
            ffi::KB_EVENT_QUIT => SessionEvent::Quit {
                reason: e.quit_reason,
                message: fixed_str(&e.quit_reason_str),
                is_error: e.quit_is_error != 0,
            },
            ffi::KB_EVENT_RUMBLE => SessionEvent::Rumble {
                left: e.rumble_left,
                right: e.rumble_right,
            },
            ffi::KB_EVENT_TRIGGER_EFFECTS => SessionEvent::TriggerEffects {
                type_left: e.trigger_type_left,
                type_right: e.trigger_type_right,
                left: e.trigger_left,
                right: e.trigger_right,
            },
            ffi::KB_EVENT_LED_COLOR => SessionEvent::LedColor { rgb: e.led },
            ffi::KB_EVENT_PLAYER_INDEX => SessionEvent::PlayerIndex {
                index: e.player_index,
            },
            ffi::KB_EVENT_NICKNAME => SessionEvent::Nickname {
                name: fixed_str(&e.nickname),
            },
            ffi::KB_EVENT_VIDEO_FEC_FAILURE => SessionEvent::VideoFecFailure,
            _ => return None,
        })
    }
}

/// One compressed access unit (Annex-B) exactly as the console sent it.
pub struct VideoFrame<'a> {
    pub data: &'a [u8],
    pub codec: Codec,
    pub keyframe: bool,
    /// Frames lost immediately before this one.
    pub frames_lost: i32,
    /// True if FEC had to reconstruct part of this frame.
    pub recovered: bool,
}

/// Receives everything a session produces. Called on libchiaki threads.
pub trait SessionSink: Send + Sync + 'static {
    fn on_event(&self, event: SessionEvent);
    /// Return false if the frame could not be decoded; libchiaki then
    /// requests a fresh keyframe from the console.
    fn on_video(&self, frame: VideoFrame<'_>) -> bool;
    fn on_audio_format(&self, _channels: u32, _sample_rate: u32) {}
    /// Interleaved signed 16-bit PCM.
    fn on_audio(&self, _pcm: &[i16]) {}
    /// DualSense haptics stream (3 kHz stereo s16), only when enabled.
    fn on_haptics(&self, _data: &[u8]) {}
}

struct Inner {
    sink: Box<dyn SessionSink>,
    codec: Codec,
    counters: FrameCounters,
    audio_channels: std::sync::atomic::AtomicU32,
}

fn guard<R>(default: R, f: impl FnOnce() -> R) -> R {
    // A panic must never unwind into C.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(default)
}

unsafe extern "C" fn event_cb(event: *const ffi::KbEvent, user: *mut c_void) {
    let inner = &*(user as *const Inner);
    if let Some(e) = event.as_ref().and_then(SessionEvent::from_raw) {
        guard((), || inner.sink.on_event(e));
    }
}

unsafe extern "C" fn video_cb(
    buf: *const u8,
    size: usize,
    frames_lost: i32,
    recovered: bool,
    user: *mut c_void,
) -> bool {
    let inner = &*(user as *const Inner);
    if buf.is_null() || size == 0 {
        return false;
    }
    let data = std::slice::from_raw_parts(buf, size);
    let keyframe = nal::is_keyframe(inner.codec, data);
    inner
        .counters
        .record(size, frames_lost, recovered, keyframe);
    guard(false, || {
        inner.sink.on_video(VideoFrame {
            data,
            codec: inner.codec,
            keyframe,
            frames_lost,
            recovered,
        })
    })
}

unsafe extern "C" fn audio_settings_cb(channels: u32, rate: u32, user: *mut c_void) {
    let inner = &*(user as *const Inner);
    inner
        .audio_channels
        .store(channels.max(1), Ordering::Relaxed);
    guard((), || inner.sink.on_audio_format(channels, rate));
}

unsafe extern "C" fn audio_frame_cb(pcm: *const i16, samples: usize, user: *mut c_void) {
    let inner = &*(user as *const Inner);
    if pcm.is_null() {
        return;
    }
    let ch = inner.audio_channels.load(Ordering::Relaxed).max(1) as usize;
    let data = std::slice::from_raw_parts(pcm, samples * ch);
    guard((), || inner.sink.on_audio(data));
}

unsafe extern "C" fn haptics_cb(buf: *const u8, size: usize, user: *mut c_void) {
    let inner = &*(user as *const Inner);
    if buf.is_null() {
        return;
    }
    guard((), || {
        inner.sink.on_haptics(std::slice::from_raw_parts(buf, size))
    });
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub rtt_ms: f64,
    /// Bitrate libchiaki measured from the video stream.
    pub measured_mbps: f64,
    /// 0..1 over libchiaki's congestion-control window.
    pub packet_loss: f64,
    /// Frames delivered per second over the last sample interval.
    pub fps: f64,
    /// Received video data rate over the last sample interval.
    pub received_mbps: f64,
    pub frames_total: u64,
    pub frames_lost_total: u64,
    pub frames_recovered_total: u64,
    pub jitter_ms: f64,
    pub width: i32,
    pub height: i32,
    pub target_fps: i32,
    pub bitrate_kbps: i32,
    pub codec: String,
    pub quality: Quality,
}

pub struct Session {
    raw: *mut ffi::KbSession,
    inner: Box<Inner>,
    meter: RateMeter,
    _host: CString,
}

// libchiaki guards its session state with its own mutexes; all methods are
// safe to call from any thread.
unsafe impl Send for Session {}
unsafe impl Sync for Session {}

impl Session {
    /// Creates and starts the session. Connection progress arrives as events:
    /// `Connected` on success, `Quit` on failure or disconnect.
    pub fn start(cfg: SessionConfig, sink: impl SessionSink) -> Result<Session> {
        crate::init()?;
        if cfg.fps != 30 && cfg.fps != 60 {
            return Err(Error::InvalidInput("fps must be 30 or 60".into()));
        }
        if let Some(b) = cfg.bitrate_kbps {
            if !(2_000..=50_000).contains(&b) {
                return Err(Error::InvalidInput(
                    "bitrate must be between 2000 and 50000 kbps".into(),
                ));
            }
        }
        let host = cstring(&cfg.host, "host")?;
        // H.265 is PS5-only; kbridge enforces the same rule, mirror it here so
        // keyframe detection uses the codec actually negotiated.
        let codec = if cfg.ps5 { cfg.codec } else { Codec::H264 };
        let inner = Box::new(Inner {
            sink: Box::new(sink),
            codec,
            counters: FrameCounters::default(),
            audio_channels: std::sync::atomic::AtomicU32::new(2),
        });

        let mut regist_key = [0 as std::ffi::c_char; 16];
        for (d, s) in regist_key.iter_mut().zip(cfg.regist_key.expose()) {
            *d = *s as std::ffi::c_char;
        }
        let raw_cfg = ffi::KbSessionConfig {
            host: host.as_ptr(),
            ps5: cfg.ps5 as i32,
            resolution: cfg.resolution.raw(),
            fps: cfg.fps as i32,
            bitrate_kbps: cfg.bitrate_kbps.unwrap_or(0),
            codec: match codec {
                Codec::H264 => 0,
                Codec::H265 => 1,
            },
            enable_dualsense: cfg.dualsense as i32,
            packet_loss_max: cfg.packet_loss_max,
            regist_key,
            morning: *cfg.morning.expose(),
            psn_account_id: cfg.account.0,
        };
        let cbs = ffi::KbSessionCallbacks {
            user: &*inner as *const Inner as *mut c_void,
            event: Some(event_cb),
            video: Some(video_cb),
            audio_settings: Some(audio_settings_cb),
            audio_frame: Some(audio_frame_cb),
            haptics: if cfg.dualsense {
                Some(haptics_cb)
            } else {
                None
            },
            log: crate::log::CB,
        };
        let mut err = 0;
        let raw = unsafe { ffi::kb_session_create(&raw_cfg, &cbs, &mut err) };
        // kbridge copied what it needs; wipe our stack copy of the key.
        let mut raw_cfg = raw_cfg;
        raw_cfg.regist_key.fill(0);
        raw_cfg.morning.fill(0);
        if raw.is_null() {
            return Err(Error::from_code(err));
        }
        let session = Session {
            raw,
            inner,
            meter: RateMeter::default(),
            _host: host,
        };
        Error::check(unsafe { ffi::kb_session_start(session.raw) })?;
        Ok(session)
    }

    pub fn set_controller(&self, state: &ControllerState) -> Result<()> {
        Error::check(unsafe { ffi::kb_session_set_controller(self.raw, state.raw()) })
    }

    pub fn set_login_pin(&self, pin: &str) -> Result<()> {
        let c = cstring(pin, "pin")?;
        Error::check(unsafe { ffi::kb_session_set_login_pin(self.raw, c.as_ptr()) })
    }

    pub fn request_keyframe(&self) -> Result<()> {
        Error::check(unsafe { ffi::kb_session_request_idr(self.raw) })
    }

    /// Puts the console into rest mode.
    pub fn sleep_console(&self) -> Result<()> {
        Error::check(unsafe { ffi::kb_session_goto_bed(self.raw) })
    }

    pub fn go_home(&self) -> Result<()> {
        Error::check(unsafe { ffi::kb_session_go_home(self.raw) })
    }

    /// Asks the session to end; a `Quit` event follows.
    pub fn stop(&self) -> Result<()> {
        Error::check(unsafe { ffi::kb_session_stop(self.raw) })
    }

    /// Takes a stats sample. Rates are measured since the previous call.
    pub fn stats(&self) -> SessionStats {
        let mut raw = ffi::KbStats::default();
        unsafe { ffi::kb_session_get_stats(self.raw, &mut raw) };
        let c = &self.inner.counters;
        let frames = c.frames.load(Ordering::Relaxed);
        let bytes = c.bytes.load(Ordering::Relaxed);
        let lost = c.frames_lost.load(Ordering::Relaxed);
        let rates = self.meter.sample(Instant::now(), frames, bytes, lost);
        let rtt_ms = raw.rtt_us as f64 / 1000.0;
        let jitter_ms = c.jitter().as_secs_f64() * 1000.0;
        SessionStats {
            rtt_ms,
            measured_mbps: raw.measured_bitrate_mbps,
            packet_loss: raw.packet_loss,
            fps: rates.fps,
            received_mbps: rates.mbps,
            frames_total: frames,
            frames_lost_total: lost,
            frames_recovered_total: c.frames_recovered.load(Ordering::Relaxed),
            jitter_ms,
            width: raw.width,
            height: raw.height,
            target_fps: raw.fps,
            bitrate_kbps: raw.bitrate_kbps,
            codec: match raw.codec {
                0 => "H.264",
                1 => "H.265",
                2 => "H.265 HDR",
                _ => "unknown",
            }
            .to_string(),
            quality: stats::classify(
                rtt_ms,
                raw.packet_loss,
                rates.fps,
                raw.fps as f64,
                jitter_ms,
            ),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Stops (if running), joins libchiaki's threads, then frees. After this
        // no callback can touch `inner`, so it is dropped safely afterwards.
        unsafe { ffi::kb_session_free(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_parse() {
        assert_eq!(Resolution::parse("1080p"), Some(Resolution::P1080));
        assert_eq!(Resolution::parse("720"), Some(Resolution::P720));
        assert_eq!(Resolution::parse("4k"), None);
    }

    #[test]
    fn event_mapping() {
        let mut e: ffi::KbEvent = unsafe { std::mem::zeroed() };
        e.type_ = ffi::KB_EVENT_RUMBLE;
        e.rumble_left = 10;
        e.rumble_right = 200;
        assert_eq!(
            SessionEvent::from_raw(&e),
            Some(SessionEvent::Rumble {
                left: 10,
                right: 200
            })
        );
        e.type_ = 99;
        assert_eq!(SessionEvent::from_raw(&e), None);
    }

    struct NullSink;
    impl SessionSink for NullSink {
        fn on_event(&self, _: SessionEvent) {}
        fn on_video(&self, _: VideoFrame<'_>) -> bool {
            true
        }
    }

    #[test]
    fn rejects_bad_config_before_touching_network() {
        let cfg = SessionConfig {
            host: "127.0.0.1".into(),
            ps5: true,
            resolution: Resolution::P1080,
            fps: 45,
            bitrate_kbps: None,
            codec: Codec::H264,
            dualsense: false,
            packet_loss_max: 0.05,
            regist_key: Secret::new([0; 16]),
            morning: Secret::new([0; 16]),
            account: AccountId([0; 8]),
        };
        assert!(matches!(
            Session::start(cfg, NullSink),
            Err(Error::InvalidInput(_))
        ));
    }
}
