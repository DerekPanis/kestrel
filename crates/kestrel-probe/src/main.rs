//! kestrel-probe — Stage 1 hardware validation tool.
//!
//! Proves the full Remote Play path against a real PS5 before any UI exists:
//! discover → pair → wake → stream (video recorded to a playable file, audio
//! to the speakers, controller input to the console, feedback back to the pad).

mod audio;
mod input;

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use kestrel_core::account::{self, AccountId};
use kestrel_core::discovery::{self, DiscoveredHost, Discovery, HostState};
use kestrel_core::nal::Codec;
use kestrel_core::regist;
use kestrel_core::session::{
    Resolution, Session, SessionConfig, SessionEvent, SessionSink, VideoFrame,
};
use kestrel_core::store::{ConsoleRecord, ConsoleStore};
use parking_lot::Mutex;

#[derive(Parser)]
#[command(
    name = "kestrel-probe",
    version,
    about = "Kestrel Stage-1 probe: discover, pair and stream a PS5"
)]
struct Cli {
    /// Show libchiaki's verbose protocol log.
    #[arg(long, global = true)]
    verbose: bool,
    /// Use a different console store file (default: %APPDATA%\Kestrel\Kestrel\config\consoles.json).
    #[arg(long, global = true)]
    store: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Find consoles on the local network.
    Discover {
        /// Probe one address instead of broadcasting (other subnet / broadcast blocked).
        #[arg(long)]
        host: Option<String>,
        #[arg(long, default_value_t = 5)]
        seconds: u64,
    },
    /// Pair with a console showing Settings → System → Remote Play → Link Device.
    Pair {
        /// Console IP address (from `discover`).
        #[arg(long)]
        host: String,
        /// PSN Account ID (base64, or the decimal user id).
        #[arg(long)]
        account_id: String,
        /// The 8-digit code shown on the TV. Prompted for if omitted.
        #[arg(long)]
        pin: Option<String>,
        /// PS5 login passcode, if the user has one.
        #[arg(long, default_value = "")]
        console_pin: String,
        /// Pair a PS4 instead of a PS5.
        #[arg(long)]
        ps4: bool,
    },
    /// List paired consoles.
    List,
    /// Remove a paired console from this PC.
    Forget { console: String },
    /// Wake a paired console from rest mode.
    Wake {
        console: String,
        /// Wait until the console reports ready.
        #[arg(long)]
        wait: bool,
    },
    /// List audio output devices.
    AudioDevices,
    /// List connected game controllers.
    Controllers,
    /// Stream from a paired console.
    Stream(StreamArgs),
}

#[derive(clap::Args)]
struct StreamArgs {
    /// Console nickname, id or address (see `list`).
    console: String,
    #[arg(long, default_value = "1080p")]
    resolution: String,
    #[arg(long, default_value_t = 60)]
    fps: u32,
    /// Video bitrate in kbps (default: the console preset for the resolution).
    #[arg(long)]
    bitrate: Option<u32>,
    #[arg(long, value_enum, default_value_t = CodecArg::H265)]
    codec: CodecArg,
    /// Write the received video stream to this file (play it with ffplay or VLC).
    #[arg(long)]
    record: Option<PathBuf>,
    /// Stop after this many seconds (default: until Ctrl+C or the console ends the stream).
    #[arg(long)]
    duration: Option<u64>,
    /// Output device name (substring match, see `audio-devices`).
    #[arg(long)]
    audio_device: Option<String>,
    #[arg(long)]
    no_audio: bool,
    /// Playback volume 0-100.
    #[arg(long, default_value_t = 100)]
    volume: u8,
    /// Don't request DualSense trigger effects.
    #[arg(long)]
    no_dualsense: bool,
    /// Override the stored console address.
    #[arg(long)]
    host: Option<String>,
    /// Don't try to wake the console first.
    #[arg(long)]
    no_wake: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum CodecArg {
    H264,
    H265,
}

fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;
    kestrel_core::log::set_handler(move |level, msg| {
        use kestrel_core::log::Level::*;
        let show = match level {
            Error | Warning => true,
            Info => true,
            Verbose | Debug => verbose,
        };
        if show {
            eprintln!("  [chiaki {level:?}] {msg}");
        }
    });
    if let Err(e) = run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn open_store(path: &Option<PathBuf>) -> Result<ConsoleStore> {
    Ok(match path {
        Some(p) => ConsoleStore::open(p.clone())?,
        None => ConsoleStore::open_default()?,
    })
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Discover { host, seconds } => {
            discover(&open_store(&cli.store)?, host.as_deref(), seconds)
        }
        Cmd::Pair {
            host,
            account_id,
            pin,
            console_pin,
            ps4,
        } => pair(
            &mut open_store(&cli.store)?,
            &host,
            &account_id,
            pin,
            &console_pin,
            !ps4,
        ),
        Cmd::List => list(&open_store(&cli.store)?),
        Cmd::Forget { console } => {
            let mut store = open_store(&cli.store)?;
            let id = store
                .find(&console)
                .map(|c| c.id.clone())
                .ok_or_else(|| anyhow!("no paired console '{console}'"))?;
            store.remove(&id)?;
            println!("Removed {console}. To revoke it on the PS5 too: Settings → System → Remote Play → Linked devices.");
            Ok(())
        }
        Cmd::Wake { console, wait } => {
            let store = open_store(&cli.store)?;
            let rec = store
                .find(&console)
                .ok_or_else(|| anyhow!("no paired console '{console}'"))?;
            wake_console(rec, &rec.host, wait)
        }
        Cmd::AudioDevices => {
            for d in audio::list_devices()? {
                println!("{d}");
            }
            Ok(())
        }
        Cmd::Controllers => {
            let mut input = input::Input::new()?;
            input.poll();
            if input.pads.is_empty() {
                println!("No game controllers detected.");
            }
            for p in input.pads.values() {
                println!("{}  [{:?}]", p.name, p.kind);
            }
            Ok(())
        }
        Cmd::Stream(args) => stream(&mut open_store(&cli.store)?, args),
    }
}

fn state_str(s: HostState) -> &'static str {
    match s {
        HostState::Ready => "ready",
        HostState::Standby => "rest mode",
        HostState::Unknown => "unknown",
    }
}

fn discover(store: &ConsoleStore, host: Option<&str>, seconds: u64) -> Result<()> {
    println!(
        "Searching for consoles for {seconds}s{}...",
        host.map(|h| format!(" at {h}")).unwrap_or_default()
    );
    let (tx, rx) = mpsc::channel();
    let _d = Discovery::start(host, 500, move |hosts| {
        let _ = tx.send(hosts);
    })?;
    let mut seen: HashMap<String, DiscoveredHost> = HashMap::new();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Ok(hosts) = rx.recv_timeout(left) {
            for h in hosts {
                if seen.get(&h.host_id) != Some(&h) {
                    let paired = store
                        .consoles()
                        .iter()
                        .any(|c| c.id.eq_ignore_ascii_case(&h.host_id));
                    println!(
                        "  {:<18} {:<20} {}  {:<9} fw {}  {}{}",
                        h.address,
                        h.name,
                        if h.is_ps5 { "PS5" } else { "PS4" },
                        state_str(h.state),
                        h.system_version,
                        if paired { "paired" } else { "not paired" },
                        if h.running_app_name.is_empty() {
                            String::new()
                        } else {
                            format!("  playing: {}", h.running_app_name)
                        },
                    );
                    seen.insert(h.host_id.clone(), h);
                }
            }
        }
    }
    if seen.is_empty() {
        println!(
            "No consoles answered. Check: the PS5 is on or in rest mode, Remote Play is enabled,"
        );
        println!(
            "the PC is on the same network, and Windows Firewall allows kestrel-probe (UDP 9302)."
        );
        println!("If they're on different subnets, retry with --host <PS5 IP>.");
    }
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn pair(
    store: &mut ConsoleStore,
    host: &str,
    account_id: &str,
    pin: Option<String>,
    console_pin: &str,
    ps5: bool,
) -> Result<()> {
    let account = AccountId::parse(account_id)?;
    let console_pin = account::parse_console_pin(console_pin)?;
    println!("On the PS5: Settings → System → Remote Play → Link Device. Keep that screen open.");
    let pin = match pin {
        Some(p) => p,
        None => prompt("Enter the 8-digit code shown on the TV: ")?,
    };
    let pin = account::parse_link_pin(&pin)?;
    println!("Pairing with {host}...");
    let pairing = regist::pair(host, ps5, account, pin, console_pin, Duration::from_secs(30)).map_err(|e| {
        anyhow!(
            "{e}\nHints: the code expires after a few minutes — get a fresh one; the Account ID must belong to the \
             PSN user signed in on the console; the PC must reach the console on port 9295 (UDP and TCP)."
        )
    })?;
    let rec = ConsoleRecord::new(host, &pairing, account)?;
    let nickname = rec.nickname.clone();
    let scheme = rec.protection.clone();
    store.upsert(rec)?;
    println!(
        "Paired with \"{nickname}\". Credentials saved to {} (protection: {scheme}).",
        store.path().display()
    );
    Ok(())
}

fn list(store: &ConsoleStore) -> Result<()> {
    if store.consoles().is_empty() {
        println!("No paired consoles. Use `kestrel-probe discover`, then `kestrel-probe pair`.");
    }
    for c in store.consoles() {
        println!(
            "{:<20} {:<16} {}  id {}  last connected: {}",
            c.nickname,
            c.host,
            if c.ps5 { "PS5" } else { "PS4" },
            c.id,
            c.last_connected
                .map(|t| format!("unix {t}"))
                .unwrap_or_else(|| "never".into()),
        );
    }
    Ok(())
}

/// Returns the console's state as seen by a unicast probe, or None if it didn't answer.
fn probe_state(host: &str, timeout: Duration) -> Result<Option<DiscoveredHost>> {
    let (tx, rx) = mpsc::channel();
    let _d = Discovery::start(Some(host), 250, move |hosts| {
        let _ = tx.send(hosts);
    })?;
    let deadline = Instant::now() + timeout;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Ok(hosts) = rx.recv_timeout(left) {
            if let Some(h) = hosts.into_iter().next() {
                return Ok(Some(h));
            }
        }
    }
    Ok(None)
}

fn wake_console(rec: &ConsoleRecord, host: &str, wait: bool) -> Result<()> {
    let creds = rec.credentials()?;
    discovery::wake(host, creds.regist_key.expose(), rec.ps5)?;
    println!("Wake packet sent to {host}.");
    if wait {
        let deadline = Instant::now() + Duration::from_secs(40);
        while Instant::now() < deadline {
            if let Some(h) = probe_state(host, Duration::from_secs(2))? {
                if h.state == HostState::Ready {
                    println!("Console is ready.");
                    return Ok(());
                }
            }
        }
        bail!("console did not report ready within 40 s");
    }
    Ok(())
}

// ------------------------------------------------------------------ streaming

enum Msg {
    Event(SessionEvent),
    AudioFormat(u32, u32),
}

struct Sink {
    tx: mpsc::Sender<Msg>,
    record: Option<mpsc::SyncSender<Vec<u8>>>,
    record_dropped: Arc<AtomicU64>,
    keyframes: Arc<AtomicU64>,
    audio: Option<Arc<Mutex<audio::Shared>>>,
}

impl SessionSink for Sink {
    fn on_event(&self, event: SessionEvent) {
        let _ = self.tx.send(Msg::Event(event));
    }
    fn on_video(&self, frame: VideoFrame<'_>) -> bool {
        if frame.keyframe {
            self.keyframes.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(rec) = &self.record {
            // Never block libchiaki's receive thread on disk I/O.
            if rec.try_send(frame.data.to_vec()).is_err() {
                self.record_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        true
    }
    fn on_audio_format(&self, channels: u32, rate: u32) {
        if let Some(a) = &self.audio {
            a.lock().set_source(rate, channels);
        }
        let _ = self.tx.send(Msg::AudioFormat(channels, rate));
    }
    fn on_audio(&self, pcm: &[i16]) {
        if let Some(a) = &self.audio {
            a.lock().push(pcm);
        }
    }
}

fn stream(store: &mut ConsoleStore, args: StreamArgs) -> Result<()> {
    let rec = store
        .find(&args.console)
        .cloned()
        .ok_or_else(|| anyhow!("no paired console '{}'", args.console))?;
    let creds = rec
        .credentials()
        .context("decrypting pairing credentials")?;
    let host = args.host.clone().unwrap_or_else(|| rec.host.clone());
    let resolution = Resolution::parse(&args.resolution)
        .ok_or_else(|| anyhow!("resolution must be 360p, 540p, 720p or 1080p"))?;
    if args.fps != 30 && args.fps != 60 {
        bail!("fps must be 30 or 60");
    }

    // 1. Make sure the console is awake.
    if !args.no_wake {
        match probe_state(&host, Duration::from_secs(2))? {
            Some(h) if h.state == HostState::Standby => {
                println!("Console is in rest mode — waking it...");
                wake_console(&rec, &host, true)?;
            }
            Some(h) => {
                if h.address != host {
                    store.update_host(&rec.id, &h.address)?;
                }
                println!("Console \"{}\" is {}.", h.name, state_str(h.state));
            }
            None => {
                println!("Console did not answer discovery at {host}; trying to connect anyway.")
            }
        }
    }

    // 2. Local devices.
    let audio_out = if args.no_audio {
        None
    } else {
        match audio::AudioOut::open(args.audio_device.as_deref(), 48_000, 2) {
            Ok(a) => {
                a.shared.lock().volume = args.volume.min(100) as f32 / 100.0;
                println!("Audio → {}", a.device_name);
                Some(a)
            }
            Err(e) => {
                eprintln!("warning: audio disabled: {e:#}");
                None
            }
        }
    };
    let mut input = input::Input::new()?;
    input.poll();
    if input.pads.is_empty() {
        println!("No controller connected yet — plug one in or pair it over Bluetooth any time.");
    }
    for p in input.pads.values() {
        println!("Controller: {} [{:?}]", p.name, p.kind);
    }

    // 3. Optional recorder thread.
    let record_dropped = Arc::new(AtomicU64::new(0));
    let (record_tx, record_thread) = match &args.record {
        Some(path) => {
            let file =
                File::create(path).with_context(|| format!("creating {}", path.display()))?;
            let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(240);
            let handle = std::thread::spawn(move || -> io::Result<u64> {
                let mut w = BufWriter::new(file);
                let mut bytes = 0u64;
                for frame in rx {
                    w.write_all(&frame)?;
                    bytes += frame.len() as u64;
                }
                w.flush()?;
                Ok(bytes)
            });
            (Some(tx), Some(handle))
        }
        None => (None, None),
    };

    // 4. Connect.
    let codec = match args.codec {
        CodecArg::H264 => Codec::H264,
        CodecArg::H265 => Codec::H265,
    };
    let (tx, rx) = mpsc::channel();
    let keyframes = Arc::new(AtomicU64::new(0));
    let sink = Sink {
        tx,
        record: record_tx,
        record_dropped: record_dropped.clone(),
        keyframes: keyframes.clone(),
        audio: audio_out.as_ref().map(|a| a.shared.clone()),
    };
    println!(
        "Connecting to {host} ({}, {} fps, {}, {})...",
        args.resolution,
        args.fps,
        args.bitrate
            .map(|b| format!("{b} kbps"))
            .unwrap_or_else(|| "preset bitrate".into()),
        match codec {
            Codec::H264 => "H.264",
            Codec::H265 => "H.265",
        }
    );
    let session = Session::start(
        SessionConfig {
            host: host.clone(),
            ps5: rec.ps5,
            resolution,
            fps: args.fps,
            bitrate_kbps: args.bitrate,
            codec,
            dualsense: !args.no_dualsense,
            packet_loss_max: 0.05,
            regist_key: creds.regist_key.clone(),
            morning: creds.morning.clone(),
            account: creds.account,
        },
        sink,
    )?;

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
            .context("installing Ctrl+C handler")?;
    }

    // 5. Main loop: input at ~250 Hz, events, stats once per second.
    let started = Instant::now();
    let mut connected_at: Option<Instant> = None;
    let mut last_stats = Instant::now();
    let mut last_state = None;
    let mut stop_requested_at: Option<Instant> = None;
    let mut inputs_sent = 0u64;
    let mut quit_info = None;
    loop {
        for change in input.poll() {
            match change {
                input::InputChange::Connected(n) => println!("Controller connected: {n}"),
                input::InputChange::Disconnected(n) => println!("Controller disconnected: {n}"),
            }
        }
        if connected_at.is_some() {
            let state = input.state();
            if last_state != Some(state) {
                session.set_controller(&state)?;
                inputs_sent += 1;
                last_state = Some(state);
            }
        }

        while let Ok(msg) = rx.try_recv() {
            let ev = match msg {
                Msg::AudioFormat(ch, rate) => {
                    println!("Audio stream: {ch} ch, {rate} Hz");
                    continue;
                }
                Msg::Event(ev) => ev,
            };
            match ev {
                SessionEvent::Connected => {
                    connected_at = Some(Instant::now());
                    store.mark_connected(&rec.id)?;
                    println!(
                        "Connected after {:.1}s. Ctrl+C to disconnect.",
                        started.elapsed().as_secs_f64()
                    );
                }
                SessionEvent::LoginPinRequested { incorrect } => {
                    if incorrect {
                        println!("The console passcode was wrong.");
                    }
                    let pin = if creds.console_pin != 0 && !incorrect {
                        format!("{:04}", creds.console_pin)
                    } else {
                        prompt("PS5 login passcode: ")?
                    };
                    session.set_login_pin(&pin)?;
                }
                SessionEvent::Quit {
                    message,
                    is_error,
                    reason,
                } => {
                    quit_info = Some((reason, message, is_error));
                }
                SessionEvent::Rumble { left, right } => input.rumble(left, right),
                SessionEvent::LedColor { rgb } => input.set_led(rgb),
                SessionEvent::TriggerEffects {
                    type_left,
                    type_right,
                    left,
                    right,
                } => input.trigger_effects(type_left, &left, type_right, &right),
                SessionEvent::Nickname { name } => println!("Console nickname: {name}"),
                SessionEvent::PlayerIndex { .. } | SessionEvent::VideoFecFailure => {}
            }
        }
        if quit_info.is_some() {
            break;
        }

        if last_stats.elapsed() >= Duration::from_secs(1) && connected_at.is_some() {
            last_stats = Instant::now();
            let s = session.stats();
            let audio = audio_out
                .as_ref()
                .map(|a| {
                    let sh = a.shared.lock();
                    format!(
                        "  audio {:.0}ms buf, {} underruns",
                        sh.buffered_ms(),
                        sh.underruns
                    )
                })
                .unwrap_or_default();
            println!(
                "{}x{} {} | {:5.1} fps (target {}) | {:5.1} Mbps rx, {:4.1} Mbps measured | rtt {:4.1} ms | loss {:4.1}% | jitter {:4.1} ms | lost {} | {:?} | pads {}{}",
                s.width, s.height, s.codec, s.fps, s.target_fps, s.received_mbps, s.measured_mbps, s.rtt_ms,
                s.packet_loss * 100.0, s.jitter_ms, s.frames_lost_total, s.quality, input.pads.len(), audio
            );
        }

        let timed_out = args
            .duration
            .is_some_and(|d| connected_at.is_some_and(|c| c.elapsed() >= Duration::from_secs(d)));
        if (stop.load(Ordering::SeqCst) || timed_out) && stop_requested_at.is_none() {
            println!("Disconnecting...");
            session.stop()?;
            stop_requested_at = Some(Instant::now());
        }
        if stop_requested_at.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
            println!("Console did not acknowledge the disconnect in 5 s; closing.");
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }

    input.reset_feedback();
    let final_stats = session.stats();
    drop(session); // joins libchiaki threads; closes the recorder channel
    let recorded = match record_thread {
        Some(h) => Some(
            h.join()
                .map_err(|_| anyhow!("recorder thread panicked"))??,
        ),
        None => None,
    };

    println!("\nSession summary");
    if let Some((reason, message, is_error)) = &quit_info {
        println!(
            "  ended:            {message} (reason {reason}{})",
            if *is_error { ", error" } else { "" }
        );
    }
    if let Some(c) = connected_at {
        println!("  connected for:    {:.1}s", c.elapsed().as_secs_f64());
    }
    println!(
        "  video frames:     {} ({} keyframes, {} lost, {} FEC-recovered)",
        final_stats.frames_total,
        keyframes.load(Ordering::Relaxed),
        final_stats.frames_lost_total,
        final_stats.frames_recovered_total
    );
    println!("  input updates:    {inputs_sent}");
    if let Some(a) = &audio_out {
        let sh = a.shared.lock();
        println!("  audio underruns:  {}", sh.underruns);
    }
    if let (Some(path), Some(bytes)) = (&args.record, recorded) {
        println!(
            "  recorded:         {} ({:.1} MB, {} frames dropped by the recorder)",
            path.display(),
            bytes as f64 / 1e6,
            record_dropped.load(Ordering::Relaxed)
        );
        println!(
            "  play it with:     ffplay -f {} \"{}\"",
            match codec {
                Codec::H264 => "h264",
                Codec::H265 => "hevc",
            },
            path.display()
        );
    }
    if let Some((_, _, true)) = quit_info {
        if connected_at.is_none() {
            bail!("the session ended before connecting");
        }
    }
    Ok(())
}
