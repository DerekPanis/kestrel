# Validation matrix

"Tested here" means run in the development environment (Linux) or CI against the
**real libchiaki code**, using loopback test peers — never a PS5. Nothing in this
project has been tested against real PS5 hardware yet.

| Area | Status | Evidence |
|---|---|---|
| libchiaki builds as part of kbridge (Linux) | ✅ tested here | `cmake --build` |
| libchiaki + kbridge build on Windows x64 (clang-cl, vcpkg) | ✅ tested (CI) | Windows job, run 42674968167 |
| All 25 Rust tests pass on Windows | ✅ tested (CI) | incl. loopback discovery + session |
| Packaged exe starts and finds all DLLs | ✅ tested (CI) | `kestrel-probe --version` in Package step |
| FFI struct layouts match C | ✅ tested | `ffi::tests::layouts_match_c` |
| Discovery: probe format + PS5 reply parsing | ✅ tested (loopback, Linux + Windows) | `tests/loopback.rs`; caught and fixed the Windows WSAECONNRESET bug |
| Discovery of a real PS5 (broadcast + unicast) | 🔶 needs PS5 | Stage 1 test step 1 |
| Pairing request is sent; failure reported cleanly | ✅ tested (refused on loopback) | probe smoke run |
| Pairing with a real PS5 (Account ID + PIN) | 🔶 needs PS5 | step 2 |
| Wake from rest mode | 🔶 needs PS5 | `wake --wait` |
| Session lifecycle: start → Quit → clean join | ✅ tested (unreachable host) | `tests/loopback.rs` |
| Real video received (H.264 / HEVC) | 🔶 needs PS5 | `stream --record`, then play file |
| Keyframe detection / codec string | ✅ tested (synthetic NAL units) | `nal::tests` |
| Real audio played, latency bounded | 🔶 needs PS5 | step 3 |
| Jitter buffer, resampler, mute/volume | ✅ tested | `audio::tests` |
| Controller input reaches PS5 | 🔶 needs PS5 + controller | step 3 |
| Rumble / light bar / adaptive triggers | 🔶 needs PS5 + DualSense | step 3 |
| DualSense trigger report layout | ✅ tested (byte layout) | `input::tests` |
| DualSense haptics playback | ❌ not implemented (Stage 4) | |
| Stats math (FPS/Mbps rates, quality class) | ✅ tested | `stats::tests` |
| Secure storage round-trip, owner-only file | ✅ tested (Linux) | `store::tests` |
| DPAPI encryption | ✅ tested (Windows CI) | `secure::tests::round_trip` |
| Desktop UI, WebCodecs rendering | ❌ Stage 2–3 | |
| Internet (remote) play | ❌ later stage | |
| Installer | ❌ Stage 5 | |
