# Architecture

## Why chiaki-ng's libchiaki

Remote Play is an undocumented, encrypted protocol (Takion over UDP, with FEC and
congestion control) that Sony changes with firmware updates. chiaki-ng is the
actively maintained open implementation and tracks those changes, so Kestrel
embeds its protocol library rather than re-implementing it. Updating to a newer
chiaki-ng is a submodule bump plus a rebuild.

## Layers

```
React UI (Stage 2+, WebView2)          ── renders video, overlay, settings
        │  Tauri IPC (commands, events, binary channel for video)
Rust app shell (Stage 2+)              ── orchestration, settings, WebCodecs feed
        │
kestrel-core (Rust)                    ── safe API: Discovery, pair(), Session,
        │                                 ControllerState, stats, ConsoleStore
        │  FFI (ffi.rs ↔ kbridge.h, layout-checked by a test)
kbridge.dll (C)                        ── flat, opaque API; owns libchiaki structs
        │
libchiaki (chiaki-ng, static)          ── discovery, regist, session, Takion, FEC,
                                          Opus decode, feedback, holepunch (later)
```

Why a C bridge instead of binding libchiaki directly: `ChiakiSession` and friends
are large structs containing platform socket types and change between releases.
kbridge exposes only flat structs and opaque handles, and is built as a single DLL
that statically contains libchiaki, so Rust links one library and never sees
libchiaki's dependency graph.

## Threading

libchiaki runs each session on its own threads. Its callbacks (video, audio,
events) are forwarded to a `SessionSink` and must return quickly; the sink never
does I/O on those threads (the probe's recorder uses a bounded channel and drops
frames for the recorder rather than stall the network thread). Dropping a
`Session` stops and joins libchiaki's threads before freeing callback state.

## Video path (decided in Stage 1, built in Stage 2)

Compressed Annex-B access units (H.264 or HEVC, 10–40 Mbps) are forwarded from
the libchiaki callback to the UI over a Tauri binary channel and decoded with the
WebView2 **WebCodecs** `VideoDecoder` (hardware-accelerated via D3D11 on Windows),
then drawn to a canvas. Raw decoded frames are never sent across IPC (1080p60
RGBA is ~500 MB/s). `nal.rs` already marks keyframes and derives the `avc1.*`
codec string WebCodecs needs. If measured latency is not acceptable, the fallback
is an FFmpeg D3D11VA decode + native swap-chain child window owned by Rust, with
the React overlay above it.

## Audio path

libchiaki decodes Opus to s16 PCM (normally 48 kHz stereo). The player keeps a
40 ms target / 120 ms max jitter buffer, trims backlog so latency can't creep,
refills after an underrun instead of crackling, and resamples only if the output
device can't run at the source rate.

## Controller path

SDL3 (HIDAPI enabled) reads DualSense over USB and Bluetooth, Xbox/XInput pads and
other SDL-mapped gamepads. All pads are merged into one `ControllerState` and sent
on change. Feedback from the console: rumble → `SDL_RumbleGamepad`, light bar →
`SDL_SetGamepadLED`, adaptive triggers → DualSense effect report (same layout as
chiaki-ng). DualSense haptics (the audio-driven actuator stream) are exposed by
the engine but not yet played back (Stage 4).

## Statistics: where each number comes from

| Shown            | Source                                                          |
|------------------|-----------------------------------------------------------------|
| RTT              | libchiaki's measurement during session setup (`session.rtt_us`) |
| Measured bitrate | libchiaki's stream statistics                                    |
| Packet loss      | libchiaki congestion-control window                              |
| FPS              | access units actually delivered per second (counted)             |
| Received Mbps    | bytes actually delivered per second (counted)                    |
| Jitter           | EWMA deviation of frame inter-arrival time (measured)            |
| Frames lost      | libchiaki's per-frame loss count                                 |

Quality (`stats::classify`): **Poor** if loss > 5 %, RTT > 80 ms or FPS < 60 % of
target; **Fair** if loss > 2 %, RTT > 40 ms, FPS < 85 % or jitter > 8 ms; **Good**
if loss > 0.5 %, RTT > 15 ms, FPS < 95 % or jitter > 4 ms; otherwise **Excellent**.

## Security

- Pairing yields a *regist key* and *morning* key per PC↔console pair. They are
  stored only as a DPAPI-encrypted blob (user scope), together with the PSN
  Account ID and optional console passcode. Metadata (name, IP, MAC) is plain.
- Secrets live in `Secret<N>` (zeroed on drop, `Debug` prints `<redacted>`).
- libchiaki's debug/verbose levels (which include packet hexdumps) are filtered
  out in kbridge unless explicitly enabled.
- Kestrel never asks for a PSN password. The Account ID can come from Sony's own
  OAuth page (Stage 2) or be pasted.
