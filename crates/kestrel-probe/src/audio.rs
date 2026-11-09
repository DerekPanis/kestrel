//! Low-latency PCM playback through cpal (WASAPI on Windows).
//!
//! The PS5 sends Opus that libchiaki decodes to s16 PCM (normally 48 kHz
//! stereo). Samples go into a small jitter buffer; the device callback pulls
//! from it. The buffer is kept between TARGET and MAX so latency cannot creep
//! up over a long session, and it refills after an underrun instead of
//! crackling.

use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use parking_lot::Mutex;

const TARGET_MS: usize = 40;
const MAX_MS: usize = 120;

pub struct Shared {
    queue: VecDeque<f32>,
    out_rate: u32,
    out_channels: usize,
    src_rate: u32,
    src_channels: usize,
    resample_pos: f64,
    prev_frame: Vec<f32>,
    pub volume: f32,
    pub muted: bool,
    playing: bool,
    pub underruns: u64,
    pub trimmed_samples: u64,
}

impl Shared {
    /// Called when the console announces its audio format.
    pub fn set_source(&mut self, rate: u32, channels: u32) {
        if rate > 0
            && channels > 0
            && (rate != self.src_rate || channels as usize != self.src_channels)
        {
            self.src_rate = rate;
            self.src_channels = channels as usize;
            self.resample_pos = 0.0;
            self.prev_frame.clear();
        }
    }

    fn frames_for_ms(&self, ms: usize) -> usize {
        self.out_rate as usize * ms / 1000
    }

    /// Accepts interleaved s16 at the source format, converts channel layout
    /// and sample rate to the device format, and enqueues.
    pub fn push(&mut self, pcm: &[i16]) {
        let src_ch = self.src_channels.max(1);
        let frames = pcm.len() / src_ch;
        if frames == 0 {
            return;
        }
        if self.prev_frame.len() != src_ch {
            self.prev_frame = vec![0.0; src_ch];
        }
        let step = self.src_rate as f64 / self.out_rate as f64;
        let get = |i: isize, c: usize, prev: &[f32]| -> f32 {
            if i < 0 {
                prev[c]
            } else {
                pcm[i as usize * src_ch + c] as f32 / 32768.0
            }
        };
        let mut src = vec![0.0f32; src_ch];
        // Linear interpolation between frame floor(pos) and floor(pos)+1, where
        // index -1 is the last frame of the previous chunk. With equal rates
        // (the normal case) step == 1.0 and samples pass through unchanged.
        let mut pos = self.resample_pos;
        while pos < (frames - 1) as f64 + 1e-9 {
            let i = pos.floor() as isize;
            let frac = (pos - i as f64) as f32;
            for (c, v) in src.iter_mut().enumerate() {
                let a = get(i, c, &self.prev_frame);
                let b = if i + 1 < frames as isize {
                    get(i + 1, c, &self.prev_frame)
                } else {
                    a
                };
                *v = a + (b - a) * frac;
            }
            match (self.out_channels, src_ch) {
                (1, _) => self
                    .queue
                    .push_back(src.iter().sum::<f32>() / src_ch as f32),
                (n, 1) => {
                    for c in 0..n {
                        self.queue.push_back(if c < 2 { src[0] } else { 0.0 });
                    }
                }
                (n, _) => {
                    for c in 0..n {
                        self.queue.push_back(if c < src_ch { src[c] } else { 0.0 });
                    }
                }
            }
            pos += step;
        }
        self.resample_pos = pos - frames as f64;
        for c in 0..src_ch {
            self.prev_frame[c] = pcm[(frames - 1) * src_ch + c] as f32 / 32768.0;
        }
        // Keep latency bounded: drop the oldest audio beyond MAX.
        let max = self.frames_for_ms(MAX_MS) * self.out_channels;
        if self.queue.len() > max {
            let target = self.frames_for_ms(TARGET_MS) * self.out_channels;
            let drop = self.queue.len() - target;
            let drop = drop - drop % self.out_channels;
            self.queue.drain(..drop);
            self.trimmed_samples += drop as u64;
        }
    }

    fn pull(&mut self, out: &mut [f32]) {
        let target = self.frames_for_ms(TARGET_MS) * self.out_channels;
        if !self.playing && self.queue.len() >= target {
            self.playing = true;
        }
        if !self.playing {
            out.fill(0.0);
            return;
        }
        let gain = if self.muted { 0.0 } else { self.volume };
        for (n, o) in out.iter_mut().enumerate() {
            match self.queue.pop_front() {
                Some(s) => *o = s * gain,
                None => {
                    out[n..].fill(0.0);
                    self.playing = false;
                    self.underruns += 1;
                    return;
                }
            }
        }
    }

    pub fn buffered_ms(&self) -> f64 {
        if self.out_rate == 0 || self.out_channels == 0 {
            return 0.0;
        }
        self.queue.len() as f64 / self.out_channels as f64 / self.out_rate as f64 * 1000.0
    }
}

pub struct AudioOut {
    _stream: cpal::Stream,
    pub shared: Arc<Mutex<Shared>>,
    pub device_name: String,
}

pub fn list_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    Ok(host
        .output_devices()
        .context("enumerating audio outputs")?
        .map(|d| d.to_string())
        .collect())
}

impl AudioOut {
    /// Opens `device` (by name substring) or the system default output.
    pub fn open(device: Option<&str>, src_rate: u32, src_channels: u32) -> Result<Self> {
        let host = cpal::default_host();
        let dev = match device {
            Some(needle) => host
                .output_devices()?
                .find(|d| {
                    d.to_string()
                        .to_lowercase()
                        .contains(&needle.to_lowercase())
                })
                .ok_or_else(|| {
                    anyhow!(
                        "no audio output matching '{needle}' (see `kestrel-probe audio-devices`)"
                    )
                })?,
            None => host
                .default_output_device()
                .ok_or_else(|| anyhow!("no default audio output device"))?,
        };
        let device_name = dev.to_string();

        // Prefer the console's own rate so no resampling is needed.
        let supported = dev.supported_output_configs()?.collect::<Vec<_>>();
        let preferred = supported
            .iter()
            .filter(|c| c.min_sample_rate() <= src_rate && c.max_sample_rate() >= src_rate)
            .find(|c| c.channels() as u32 == src_channels && c.sample_format() == SampleFormat::F32)
            .or_else(|| {
                supported.iter().find(|c| {
                    c.min_sample_rate() <= src_rate
                        && c.max_sample_rate() >= src_rate
                        && c.channels() as u32 >= src_channels
                })
            })
            .map(|c| c.with_sample_rate(src_rate));
        let chosen = match preferred {
            Some(c) => c,
            None => dev
                .default_output_config()
                .context("querying default output format")?,
        };
        let config = chosen.config();
        let shared = Arc::new(Mutex::new(Shared {
            queue: VecDeque::with_capacity(48_000),
            out_rate: config.sample_rate,
            out_channels: config.channels as usize,
            src_rate,
            src_channels: src_channels as usize,
            resample_pos: 0.0,
            prev_frame: Vec::new(),
            volume: 1.0,
            muted: false,
            playing: false,
            underruns: 0,
            trimmed_samples: 0,
        }));

        let stream = match chosen.sample_format() {
            SampleFormat::F32 => build::<f32>(&dev, config, shared.clone())?,
            SampleFormat::I16 => build::<i16>(&dev, config, shared.clone())?,
            SampleFormat::I32 => build::<i32>(&dev, config, shared.clone())?,
            SampleFormat::U16 => build::<u16>(&dev, config, shared.clone())?,
            other => return Err(anyhow!("unsupported output sample format {other:?}")),
        };
        stream.play().context("starting audio stream")?;
        Ok(AudioOut {
            _stream: stream,
            shared,
            device_name,
        })
    }
}

fn build<T>(
    dev: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    let stream = dev.build_output_stream::<T, _, _>(
        config,
        move |out: &mut [T], _| {
            scratch.resize(out.len(), 0.0);
            shared.lock().pull(&mut scratch);
            for (o, s) in out.iter_mut().zip(scratch.iter()) {
                *o = T::from_sample(*s);
            }
        },
        |e| eprintln!("audio stream error: {e}"),
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(src_rate: u32, src_ch: usize, out_rate: u32, out_ch: usize) -> Shared {
        Shared {
            queue: VecDeque::new(),
            out_rate,
            out_channels: out_ch,
            src_rate,
            src_channels: src_ch,
            resample_pos: 0.0,
            prev_frame: Vec::new(),
            volume: 1.0,
            muted: false,
            playing: false,
            underruns: 0,
            trimmed_samples: 0,
        }
    }

    #[test]
    fn passthrough_when_formats_match() {
        let mut s = shared(48_000, 2, 48_000, 2);
        let pcm: Vec<i16> = (0..480 * 2).map(|i| (i as i16) * 10).collect();
        s.push(&pcm);
        assert_eq!(s.queue.len(), pcm.len());
        assert!((s.queue[3] - 30.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn resample_48k_to_44k1_keeps_duration() {
        let mut s = shared(48_000, 2, 44_100, 2);
        for _ in 0..100 {
            s.push(&vec![0i16; 480 * 2]); // 10 ms chunks
        }
        // 1 s of input → ~44100 frames out (latency trimming keeps <= MAX)
        let frames = s.queue.len() as u64 / 2 + s.trimmed_samples / 2;
        assert!((frames as i64 - 44_100).abs() <= 2, "got {frames}");
    }

    #[test]
    fn waits_for_target_then_reports_underrun() {
        let mut s = shared(48_000, 2, 48_000, 2);
        let mut out = vec![1.0f32; 96];
        s.push(&vec![1000i16; 48 * 2 * 10]); // 10 ms < 40 ms target
        s.pull(&mut out);
        assert!(
            out.iter().all(|&x| x == 0.0),
            "must not start before target is buffered"
        );
        s.push(&vec![1000i16; 48 * 2 * 40]);
        let mut big = vec![0.0f32; 48 * 2 * 60];
        s.pull(&mut big);
        assert_eq!(s.underruns, 1);
    }

    #[test]
    fn bounded_latency() {
        let mut s = shared(48_000, 2, 48_000, 2);
        for _ in 0..50 {
            s.push(&vec![0i16; 480 * 2]);
        }
        assert!(s.buffered_ms() <= MAX_MS as f64 + 0.01);
    }

    #[test]
    fn mute_and_volume() {
        let mut s = shared(48_000, 1, 48_000, 1);
        s.push(&vec![16384i16; 48 * 50]);
        s.volume = 0.5;
        let mut out = vec![0.0f32; 10];
        s.pull(&mut out);
        assert!((out[0] - 0.25).abs() < 1e-6);
        s.muted = true;
        s.pull(&mut out);
        assert_eq!(out[0], 0.0);
    }
}
