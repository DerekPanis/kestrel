# Kestrel — Stage 1 hardware test

This build is a command-line probe, not the app yet. Its only job is to prove
that discovery, pairing, video, audio and controller input really work between
your PC and your PS5 before the UI is built on top.

## Before you start

On the PS5:
1. Settings → System → Remote Play → **Enable Remote Play**: on.
2. Settings → System → Power Saving → Features Available in Rest Mode →
   **Stay Connected to the Internet** and **Enable Turning On PS5 from Network**: on
   (needed for waking it from rest mode).

On the PC:
- Same network as the PS5 (Ethernet on at least one side is best).
- When Windows Firewall asks about `kestrel-probe.exe`, allow it on **private** networks.
- Your **PSN Account ID** in base64 (looks like `AbCdEfGhIjI=`). This is not your
  Online ID. If you already use chiaki-ng it's in its settings; otherwise any PSN
  account-ID lookup gives it. You never type your PSN password into Kestrel.

Open PowerShell in the unzipped `kestrel-probe-win64` folder.

## 1. Discover

```powershell
.\kestrel-probe.exe discover
```
Expected: a line with your console's IP, name, `PS5`, `ready` or `rest mode`.
Nothing found? Try `.\kestrel-probe.exe discover --host 192.168.x.x` with the
PS5's IP (Settings → Network → Connection Status).

## 2. Pair

On the PS5 open Settings → System → Remote Play → **Link Device** and leave the
code on screen. Then:

```powershell
.\kestrel-probe.exe pair --host <PS5 IP> --account-id <your base64 ID>
```
Type the 8-digit code when asked. If your PS5 user has a login passcode, add
`--console-pin 1234`. Success prints `Paired with "<console name>"`.
Credentials are stored encrypted with Windows DPAPI in
`%APPDATA%\Kestrel\Kestrel\config\consoles.json`.

## 3. Stream

Connect your DualSense (USB or Bluetooth) first, then:

```powershell
.\kestrel-probe.exe stream "<console name>" --record test.h265 --duration 60
```
While it runs:
- **Audio**: game audio should play on your default output.
- **Controller**: press buttons/sticks — the PS5 should respond (try the PS button, navigating the home screen).
- **Rumble / triggers**: open a game that uses them (Astro's Playroom is ideal).
- A stats line prints every second with real FPS, Mbps, RTT, packet loss and jitter.

After it ends, play the recorded video to confirm the picture:
```powershell
ffplay -f hevc test.h265      # or open it in VLC
```
Use `--codec h264 --record test.h264` if your GPU/VLC can't play HEVC.

Other options: `--resolution 720p`, `--fps 30`, `--bitrate 15000`,
`--audio-device "Headphones"`, `--volume 60`, `--no-audio`.

## 4. Other commands

```powershell
.\kestrel-probe.exe list                 # paired consoles
.\kestrel-probe.exe wake "<name>" --wait # wake from rest mode
.\kestrel-probe.exe controllers          # what SDL sees
.\kestrel-probe.exe audio-devices
.\kestrel-probe.exe forget "<name>"
```

## What to send back

Copy the full console output of steps 1–3 (it contains no secrets) and answer:
1. Did the recorded video play and look right?
2. Did you hear game audio? Any crackling?
3. Did the PS5 respond to the controller? Rumble? Adaptive triggers? Light bar?
4. The last few stats lines (FPS / RTT / loss) and whether you were on Wi-Fi or Ethernet.

If something fails, rerun that step with `--verbose` and send that output too.
