# Changelog

All notable changes are listed here. New entries are added above by
[release-please](https://github.com/googleapis/release-please) from the
commit messages (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Before the first release (on top of 0.1.0)

### Features

- Windows (Windows Graphics Capture) and macOS (ScreenCaptureKit) support next to Linux; on both the app's own window is left out of the capture
- Native tray icon on Windows and macOS, monitor selection on both
- Redesigned interface: light spill around the preview, inspector with tabs, switches and segmented controls, empty states, embedded Inter font, new app icon
- German and English interface, following the system language
- Device discovery via mDNS on every OS (no `avahi-browse` needed); add devices by IP or host name
- `--minimized`, `--version`, `--help`; `--config` is honoured now
- `AMBILIGHT_DEMO=1` test picture
- Release pipeline with packages for all platforms, Flatpak manifest, GPL-3.0 license

### Bug fixes

- Zone edits now apply immediately (the sampling plan was only rebuilt when the capture size changed)
- A disabled device no longer shifts the colours of the devices after it
- Saturation scales the existing saturation instead of replacing it (grey areas no longer turn red)
- DDP: only the last packet of a frame sets the PUSH flag, so strips with more than 480 LEDs no longer tear; sequence numbers stay in 1–15
- LED rulers in the preview show the correct colours for reversed and BGR devices
- The incoming frame rate is measured (was always 0)
- The portal restore token is no longer overwritten by the UI, so the share dialog does not come back
- Arrow keys no longer move the zone while typing in a text field
- Pause respects the per-device frame rate limit
- The configuration is written atomically
- Tests compile again

### Performance

- Frames are shared instead of copied between capture, pipeline and preview
- New frames are sampled once; ticks in between only advance the smoothing
