//! Measured streaming statistics. Every number here comes from either
//! libchiaki (RTT, bitrate, packet loss) or from counting the frames that
//! actually arrived — nothing is estimated or synthesised.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

/// Lock-free counters updated from libchiaki's video thread.
#[derive(Default)]
pub struct FrameCounters {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub frames_lost: AtomicU64,
    pub frames_recovered: AtomicU64,
    pub keyframes: AtomicU64,
    /// Exponentially weighted mean absolute deviation of frame inter-arrival
    /// time from its running mean, in microseconds.
    jitter_us: AtomicU64,
    last_arrival: Mutex<Option<(Instant, f64)>>,
}

impl FrameCounters {
    pub fn record(&self, bytes: usize, lost: i32, recovered: bool, keyframe: bool) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        if lost > 0 {
            self.frames_lost.fetch_add(lost as u64, Ordering::Relaxed);
        }
        if recovered {
            self.frames_recovered.fetch_add(1, Ordering::Relaxed);
        }
        if keyframe {
            self.keyframes.fetch_add(1, Ordering::Relaxed);
        }
        self.record_arrival(Instant::now());
    }

    fn record_arrival(&self, now: Instant) {
        let mut last = self.last_arrival.lock();
        if let Some((prev, mean_us)) = *last {
            let dt = now.duration_since(prev).as_secs_f64() * 1e6;
            // Ignore stalls (>1s) so a pause doesn't poison the averages.
            if dt < 1e6 {
                let mean = if mean_us == 0.0 {
                    dt
                } else {
                    mean_us * 0.95 + dt * 0.05
                };
                let dev = (dt - mean).abs();
                let j = self.jitter_us.load(Ordering::Relaxed) as f64;
                let j = if j == 0.0 { dev } else { j * 0.95 + dev * 0.05 };
                self.jitter_us.store(j as u64, Ordering::Relaxed);
                *last = Some((now, mean));
                return;
            }
        }
        *last = Some((now, 0.0));
    }

    pub fn jitter(&self) -> Duration {
        Duration::from_micros(self.jitter_us.load(Ordering::Relaxed))
    }
}

/// Converts monotonically increasing counters into per-second rates.
pub struct RateMeter {
    last: Mutex<Option<(Instant, u64, u64, u64)>>,
}

impl Default for RateMeter {
    fn default() -> Self {
        RateMeter {
            last: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rates {
    pub fps: f64,
    pub mbps: f64,
    pub lost_per_sec: f64,
}

impl RateMeter {
    pub fn sample(&self, now: Instant, frames: u64, bytes: u64, lost: u64) -> Rates {
        let mut last = self.last.lock();
        let rates = match *last {
            Some((t, f, b, l)) => {
                let dt = now.duration_since(t).as_secs_f64();
                if dt <= 0.0 {
                    Rates::default()
                } else {
                    Rates {
                        fps: (frames.saturating_sub(f)) as f64 / dt,
                        mbps: (bytes.saturating_sub(b)) as f64 * 8.0 / dt / 1e6,
                        lost_per_sec: (lost.saturating_sub(l)) as f64 / dt,
                    }
                }
            }
            None => Rates::default(),
        };
        *last = Some((now, frames, bytes, lost));
        rates
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Excellent,
    Good,
    Fair,
    Poor,
}

/// Classifies a connection from measured values only. Thresholds are
/// documented in docs/ARCHITECTURE.md.
pub fn classify(
    rtt_ms: f64,
    packet_loss: f64,
    fps: f64,
    target_fps: f64,
    jitter_ms: f64,
) -> Quality {
    let fps_ratio = if target_fps > 0.0 {
        fps / target_fps
    } else {
        1.0
    };
    if packet_loss > 0.05 || rtt_ms > 80.0 || fps_ratio < 0.6 {
        Quality::Poor
    } else if packet_loss > 0.02 || rtt_ms > 40.0 || fps_ratio < 0.85 || jitter_ms > 8.0 {
        Quality::Fair
    } else if packet_loss > 0.005 || rtt_ms > 15.0 || fps_ratio < 0.95 || jitter_ms > 4.0 {
        Quality::Good
    } else {
        Quality::Excellent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_meter_computes_deltas() {
        let m = RateMeter::default();
        let t0 = Instant::now();
        assert_eq!(m.sample(t0, 0, 0, 0).fps, 0.0);
        let r = m.sample(t0 + Duration::from_secs(2), 120, 2_500_000, 4);
        assert!((r.fps - 60.0).abs() < 1e-9);
        assert!((r.mbps - 10.0).abs() < 1e-9);
        assert!((r.lost_per_sec - 2.0).abs() < 1e-9);
    }

    #[test]
    fn quality_thresholds() {
        assert_eq!(classify(3.0, 0.0, 60.0, 60.0, 1.0), Quality::Excellent);
        assert_eq!(classify(20.0, 0.0, 60.0, 60.0, 1.0), Quality::Good);
        assert_eq!(classify(3.0, 0.03, 60.0, 60.0, 1.0), Quality::Fair);
        assert_eq!(classify(3.0, 0.0, 30.0, 60.0, 1.0), Quality::Poor);
    }

    #[test]
    fn counters_track_frames() {
        let c = FrameCounters::default();
        c.record(1000, 0, false, true);
        c.record(500, 2, true, false);
        assert_eq!(c.frames.load(Ordering::Relaxed), 2);
        assert_eq!(c.bytes.load(Ordering::Relaxed), 1500);
        assert_eq!(c.frames_lost.load(Ordering::Relaxed), 2);
        assert_eq!(c.keyframes.load(Ordering::Relaxed), 1);
    }
}
