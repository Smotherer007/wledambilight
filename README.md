# WLED Ambilight

[![CI](https://github.com/Smotherer007/wledambilight/actions/workflows/ci.yml/badge.svg)](https://github.com/Smotherer007/wledambilight/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Smotherer007/wledambilight)](https://github.com/Smotherer007/wledambilight/releases/latest)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)

**Deutsch:** [README.de.md](README.de.md)

A lightweight ambilight for [WLED](https://kno.wled.ge/) and more. It
captures your screen, averages freely placed zones and streams the colours in
real time to WLED strips, PC peripherals (via OpenRGB and Razer Chroma),
Govee lights and Nanoleaf panels – the light behind your monitor or TV and
on your desk follows the picture. Runs on **Linux, Windows and macOS**,
entirely in your local network: no account, no cloud.

![WLED Ambilight with four edge zones and a free zone on the preview](docs/screenshot.png)

## Features

- **Five device types** – WLED, OpenRGB, Razer Chroma, Govee and Nanoleaf,
  mixed freely in one setup (see [Supported devices](#supported-devices)).
- **Finds your devices by itself** – one search covers all types (mDNS,
  subnet sweep, Govee LAN scan, local OpenRGB and Synapse). Devices on other
  subnets can be added by IP or host name.
- **Zones instead of fixed layouts** – a zone is any rectangle on the screen
  that drives a section of a strip. Drag and resize with the mouse, snap to
  edges and other zones, nudge with the arrow keys. Coordinates are
  normalised, so resolution, scaling and mode switches don't matter.
- **One strip around the screen or one device per edge** – “Split to 4
  edges” shares a strip over top/right/bottom/left by edge length; “Device per
  edge” puts up to four separate WLED boards on their own edge.
- **Any number of LEDs** via DDP (UDP 4048, 480 LEDs per packet, the frame is
  shown at once); DRGB (UDP 21324) for old firmware up to 490 LEDs.
- **Calm, flicker-free light** – median filter over three frames, exponential
  temporal smoothing, spatial smoothing along the strip and a limit on how
  fast an LED may change.
- **Colour controls** – brightness (global and per device), gamma, saturation,
  per-zone gain, reversed strips and BGR clones.
- **Frame rate** 24–144 fps globally, plus a cap per device.
- **Live view** – the preview shows the light each zone throws onto the wall,
  a ruler with the colour of every LED, and the exact output per device.
- **Background operation** – tray / menu bar icon, close-to-tray,
  `--minimized` for autostart, `--headless` for setups without a desktop.
- **German and English** interface, following the system language.

## Supported devices

| Device | How it is reached | Resolution | System | Setup once |
| --- | --- | --- | --- | --- |
| **WLED** | DDP (UDP 4048) or DRGB (UDP 21324) | every LED | all | none |
| **OpenRGB** – keyboards, mice, RAM, fans, mainboards, GPUs, light strips of almost every vendor | OpenRGB SDK server (TCP 6742) | every LED | all | start [OpenRGB](https://openrgb.org) and switch on its *SDK Server* |
| **Razer Chroma** – Razer devices plus *Chroma Connect* partners | REST API of Razer Synapse on `localhost` | columns left → right | Windows | Synapse running, *Chroma Apps* on, logged in |
| **Govee** – LED strips, TV backlights, light bars with LAN control | LAN API (UDP 4001–4003), realtime mode | segments (usually 10–15) | all | *LAN Control* on for the device in the Govee Home app |
| **Nanoleaf** – Shapes, Canvas, Elements, Lines, Light Panels | OpenAPI (port 16021) + External Control v2 (UDP 60222) | every panel | all | pair once (button on the controller) |

All of it stays in your local network. No device needs your account password;
Nanoleaf hands out a token when you pair, which is stored in the config file.

**Notes per device**

- **OpenRGB** talks to the hardware directly. If the vendor's own software
  (Corsair iCUE, Razer Synapse, Logitech G Hub, …) runs at the same time,
  both fight over the LEDs – quit it for the devices you drive through
  OpenRGB. Each OpenRGB controller is its own device in the app. OpenRGB on
  another computer: *Add by address* → OpenRGB → `host:6742`.
- **Razer Chroma** shows only the most recently started app, so all Razer
  hardware is one device in the app: its “LEDs” are columns from left to
  right, spread over keyboard (22 columns), mouse, mouse pad, headset, keypad
  and *Chroma Link* (which also reaches *Chroma Connect* devices of other
  vendors). Put its zone along the bottom edge of the screen for a gradient
  across the desk.
- **Govee** has no official per-segment API. The app uses the realtime mode
  that Govee's own PC app uses for screen sync (community-documented); the
  official LAN API only knows one colour per device. Set the segment count of
  your model on the device card (manual / Govee app). Windows asks once to let
  the app receive on the network – that is the Govee scan reply (UDP 4002).
- **Nanoleaf** panels are ordered left to right (top to bottom for tall
  layouts). Pairing: hold the power button on the controller for 5–7 s until
  the LEDs flash, then click *Pair* on the device card within 30 s.

**Not supported, and why**

- **Corsair iCUE SDK** – technically fine, but the SDK is published under a
  proprietary Corsair licence that forbids combining it with other software
  and does not allow redistributing it. That cannot be reconciled with this
  app's GPL-3.0 licence. Corsair hardware works through **OpenRGB** instead
  (with iCUE closed).
- **Logitech G Hub, SteelSeries GG, ASUS Aura, MSI, Gigabyte** – use OpenRGB,
  which covers this hardware directly.
- **Philips Hue, LIFX, Twinkly** – planned; they need a paired bridge or
  vendor-specific streaming and real hardware for testing.
- **Tuya / Smart Life, WiZ, Yeelight, Magic Home** – single-colour, slow
  protocols (or cloud-only keys) that cannot follow a picture.

## Install

Download the package for your system from the
[latest release](https://github.com/Smotherer007/wledambilight/releases/latest).

| System | File | Notes |
| --- | --- | --- |
| Linux (any distro) | `wled-ambilight-*.flatpak` | `flatpak install --user wled-ambilight-*.flatpak` – recommended for Fedora Atomic / Bazzite / SteamOS |
| Debian, Ubuntu, Mint | `wled-ambilight_*_amd64.deb` | `sudo apt install ./wled-ambilight_*.deb` |
| Fedora, openSUSE | `wled-ambilight-*.x86_64.rpm` | `sudo dnf install ./wled-ambilight-*.rpm` |
| Linux, portable | `wled-ambilight-*-linux-x86_64.tar.gz` | binary plus desktop file and icons |
| Windows 10 1903+ / 11 | `…-windows-x86_64-setup.exe` | installer, optional autostart; or the portable `.zip` |
| macOS 13+ (Apple silicon and Intel) | `…-macos-universal.dmg` | drag to *Applications* |

### First start per system

**Linux** – the desktop asks once which monitor to share (xdg-desktop-portal).
The permission is remembered, later starts need no dialog. The non-Flatpak
packages need GStreamer with the PipeWire plugin (`gstreamer1.0-pipewire` on
Debian/Ubuntu, `pipewire-gstreamer` on Fedora – the `.deb`/`.rpm` pull them in).

**Windows** – nothing to confirm. The app hides its own window from the
capture (Windows 10 2004 and newer), so the preview never ends up in your
LEDs. Windows 10 draws a yellow frame around a captured screen; Windows 11
lets the app switch it off.

**macOS** – macOS asks for *Screen & System Audio Recording* permission. Allow
it in *System Settings → Privacy & Security* and restart the app. If the build
is not notarised, macOS says the app “cannot be checked”: right-click the app
→ *Open* once, or run `xattr -dr com.apple.quarantine "/Applications/WLED Ambilight.app"`.

## Quick start

1. **Prepare WLED.** Any recent WLED works out of the box: realtime UDP is
   on by default (*Config → Sync Interfaces → Realtime*). Set the LED count and
   the power limit (ABL) there as usual. Other devices: see *Setup once* in
   [Supported devices](#supported-devices).
2. **Start WLED Ambilight.** The network is searched automatically; click
   **+** next to a device. It gets one zone with all its LEDs on a free
   screen edge.
3. **Place the zones.** For a strip running around the screen select its
   zone and click **Split to 4 edges**. Check the direction: the small chevron
   in each zone marks LED 0. If the light runs the wrong way, switch on
   **Reverse order** for the device.
4. **Tune the picture** on the *Picture* tab. Settings are saved automatically.

## Concepts

- **Device** – one WLED controller with *n* LEDs.
- **Zone** – a rectangle on the screen plus a number of LEDs. Each zone
  takes the next free LEDs of its device; **the order of the zone list is
  the order on the strip** (`LED 0–36`, `LED 37–58`, …). The LED count of a
  zone is independent of its size: the rectangle is cut into equal cells, one
  per LED, and each cell is averaged.
- **Horizontal / vertical** – horizontal zones run left → right, vertical
  zones top → bottom. A strip that is wired the other way round is fixed per
  device with **Reverse order** (this mirrors the whole strip, including
  the order of split zones).
- **Capture resolution** – the screen is scaled down to a small buffer
  (default 320 px wide). That is the precision of the zones; higher values
  allow finer zones and cost a little more CPU.

## Controls

| Action | How |
| --- | --- |
| Move / resize a zone | drag the zone or its edges and corners (snaps to screen edges, centre and other zones; turn off on the *App* tab) |
| Nudge the selected zone | arrow keys, with <kbd>Shift</kbd> in big steps |
| Delete the selected zone | <kbd>Delete</kbd> |
| Deselect | <kbd>Esc</kbd> |
| Pause / resume | button in the title bar or the tray menu (the LEDs keep their colour) |
| Hide to tray | close the window; quit from the tray menu |

### Picture settings

| Setting | Effect | Default |
| --- | --- | --- |
| Brightness | global scale, multiplied with the per-device brightness | 100 % |
| Saturation | 0 % = grey, 200 % = vivid | 100 % |
| Gamma | > 1 darkens dark tones (more contrast), < 1 lifts them | 1.00 |
| Temporal smoothing | time constant of the fade; 0 = off (also disables the median filter) | 180 ms |
| Spatial smoothing | blends every LED with its neighbours | 60 % |
| Max. change per second | caps how fast an LED may change (0–255 scale) | 900 |
| Frame rate | how often the LEDs are updated; per-device caps on the device card | 50 fps |

### Command line

```text
wled-ambilight [options]
  --minimized        start with the window minimised (autostart with tray)
  --headless         run without a window (servers, no tray)
  --config <file>    use this config file instead of the default
  -V, --version      print the version
  -h, --help         print this help
```

Environment variables: `AMBILIGHT_STATS=1` prints timing and flicker
statistics every two seconds; `AMBILIGHT_DEMO=1` replaces the screen with an
animated test picture (no capture permission needed – useful to try a strip).

### Autostart

- **Windows** – tick *Start automatically* in the installer, or put a
  shortcut to `wled-ambilight.exe --minimized` into `shell:startup`.
- **macOS** – *System Settings → General → Login Items → +* and pick the app.
- **Linux** – copy the desktop file and add the flag:

  ```sh
  mkdir -p ~/.config/autostart
  sed 's/^Exec=wled-ambilight$/Exec=wled-ambilight --minimized/' \
    /usr/share/applications/io.github.smotherer007.WledAmbilight.desktop \
    > ~/.config/autostart/wled-ambilight.desktop
  ```

  (Flatpak: `Exec=flatpak run io.github.smotherer007.WledAmbilight --minimized`.)

## Configuration

Everything is saved automatically as JSON:

| System | Path |
| --- | --- |
| Linux | `~/.config/wled-ambilight/config.json` (`$XDG_CONFIG_HOME`) |
| Flatpak | `~/.var/app/io.github.smotherer007.WledAmbilight/config/wled-ambilight/config.json` |
| Windows | `%APPDATA%\wled-ambilight\config.json` |
| macOS | `~/Library/Application Support/wled-ambilight/config.json` |

All zone coordinates are normalised (0…1) relative to the captured screen, so
the file can be copied between machines. The file is written atomically.
It contains the Nanoleaf pairing token (which only grants access to that one
device in your network), so don't post it publicly.

## How it works

```text
 screen ──► capture backend ──► small RGB frame (e.g. 320×180)
             Linux:   portal + PipeWire (GStreamer)
             Windows: Windows Graphics Capture
             macOS:   ScreenCaptureKit
                                 │
                                 ▼
                  zones → one cell per LED → mean colour
                                 │
             median of 3 frames → exponential smoothing → rate limit
                                 │
              gamma, saturation, brightness, BGR, reverse
                                 │
                                 ▼
      one output per device, each in its own thread (except WLED)
                                 │
   WLED (DDP/DRGB) · OpenRGB (TCP) · Razer (HTTP) · Govee (UDP) · Nanoleaf (UDP)
```

The pipeline runs in its own thread on a fixed tick that is a multiple of the
target rate, so per-device caps land on exact values and the UI can never
slow the LEDs down. Devices that need TCP/HTTP or a handshake run in their
own thread, receive only the newest frame and reconnect by themselves, so an
unreachable device never holds up the others. On exit every session is closed
(Razer session ended, Govee realtime mode switched off). New frames are sampled once; ticks in between only
advance the smoothing, which keeps fades fluid.

On Linux the window freezes its preview while it is not focused, because the
portal cannot leave the app's own window out of the capture; on Windows and
macOS the window is excluded from the capture instead.

## Troubleshooting

| Problem | Fix |
| --- | --- |
| No device found | Same network/VLAN? Firewalls must allow UDP and mDNS. Add the device by address. |
| LEDs flicker | Raise *Temporal smoothing* or lower *Max. change per second*; check the power supply and ABL limit in WLED. |
| Colours run the wrong way | *Reverse order* on the device card. |
| Red and blue swapped | *Swap red and blue (BGR)*, or set the colour order in WLED's LED settings. |
| WLED shows its own effect again | The app sends continuously, also while paused; check that *Realtime* is enabled in WLED and that the device is active here. |
| OpenRGB: “not reachable” | Start OpenRGB and switch on *SDK Server* (port 6742). Quit the vendor software of the device. |
| OpenRGB: device flickers between two colours | The vendor software (iCUE, Synapse, G Hub …) is still running for that device. |
| Razer: “Synapse not reachable” | Synapse must run and you must be logged in; turn on *Chroma Apps* in Synapse. |
| Govee not found / stays dark | Turn on *LAN Control* in the Govee Home app; allow the app in the firewall (UDP 4002); check the segment count. |
| Nanoleaf: “not in pairing mode” | Hold the power button on the controller for 5–7 s until the LEDs flash, then click *Pair* within 30 s. |
| Linux: “gst-launch-1.0 not found” | Install GStreamer and its PipeWire plugin (see *Install*), or use the Flatpak. |
| Linux: wrong monitor | *App → Choose another display* asks the portal again. |
| macOS: black preview / error | Grant *Screen Recording* permission and restart the app (macOS rule). After an update an unsigned build may need the permission again. |

## Building from source

Requirements: [Rust](https://rustup.rs/) 1.90 or newer.

- **Linux:** at runtime `gst-launch-1.0` with `pipewiresrc` and a running
  xdg-desktop-portal. No extra build dependencies.
- **Windows:** the MSVC toolchain (Visual Studio Build Tools).
- **macOS:** Xcode or the Command Line Tools (13.0 SDK or newer).

```sh
git clone https://github.com/Smotherer007/wledambilight.git
cd wledambilight
cargo run --release
cargo test
```

Packaging (Flatpak, `.deb`, `.rpm`, installer, `.dmg`) runs in GitHub
Actions; see [CONTRIBUTING.md](CONTRIBUTING.md) for the release process and
[packaging/flatpak/README.md](packaging/flatpak/README.md) for Flatpak and
Flathub.

## License

Copyright © 2026 Patrick Weppelmann.

WLED Ambilight is free software: you can redistribute it and/or modify it
under the terms of the **GNU General Public License, version 3** as published
by the Free Software Foundation. It is distributed in the hope that it will be
useful, but WITHOUT ANY WARRANTY. See [LICENSE](LICENSE) for the full text.
Distributing a modified version requires publishing its source code under the
same license.

The bundled [Inter](https://rsms.me/inter/) font is licensed under the SIL
Open Font License 1.1 ([assets/fonts/OFL.txt](assets/fonts/OFL.txt)).
WLED is a project of Aircoookie and contributors and is not affiliated with
this app.
