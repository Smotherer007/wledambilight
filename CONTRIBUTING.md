# Contributing and releasing

## Development

```sh
cargo run                       # GUI
AMBILIGHT_DEMO=1 cargo run      # animated test picture instead of the screen
AMBILIGHT_STATS=1 cargo run     # timing / flicker statistics on stderr
cargo test
cargo clippy --all-targets
cargo fmt
```

### Project layout

| Path | Purpose |
| --- | --- |
| `src/main.rs` | command line, window setup, headless mode |
| `src/app.rs` | the window: device list, preview with zones, inspector, output bar |
| `src/theme.rs`, `src/icons.rs` | design tokens, fonts, custom widgets, painted icons |
| `src/i18n.rs` | German/English switch (`tr("Deutsch", "English")`) |
| `src/worker.rs` | pipeline thread: capture → engine on a fixed tick |
| `src/engine.rs` | median, smoothing, grading, per-device rate limits, sending |
| `src/sampler.rs` | zones → pixel cells → mean colours |
| `src/capture/` | one backend per OS (`linux.rs`, `windows.rs`, `macos.rs`), `scale.rs` downscaler, `demo.rs` |
| `src/wled.rs` | WLED: `/json/info`, DDP and DRGB senders, mDNS and subnet sweep helpers |
| `src/output/` | one output per device: `mod.rs` (threaded driver with reconnect), `openrgb.rs`, `razer.rs`, `govee.rs`, `nanoleaf.rs` |
| `src/discovery.rs` | “Search the network” for every device kind |
| `src/tray/` | tray icon: `ksni` on Linux, `tray-icon` on Windows/macOS |
| `src/config.rs` | config file, migration of old fields, atomic save |
| `assets/` | `icon.svg` (source of all icons), rendered icons, Inter font |
| `packaging/` | desktop file + AppStream, Flatpak, macOS `Info.plist`, Windows installer |
| `scripts/` | icon rendering |

Windows and macOS code is only compiled on those systems; CI builds and tests
all three on every push and pull request.

### Adding a device type

1. Add a variant to `DeviceKind` (`src/config.rs`).
2. Write a `Driver` in `src/output/<name>.rs`: `connect`, `write` (one frame),
   optionally `keepalive` (called about once per second) and `close`. Run it
   through `Threaded::spawn` in `output::open` – reconnecting, frame dropping
   and status reporting come for free.
3. Find the devices in `src/discovery.rs` and map them to a `Found`.
4. Device-specific settings go into the device card in `src/app.rs`.
5. Unit-test the packet format; a fake server in the test (see `openrgb.rs`)
   covers the whole driver.

Only add protocols whose licence fits GPL-3.0 (open protocols, documented
network APIs). Vendor SDKs that must be linked under a proprietary licence
(e.g. Corsair iCUE) cannot be included.

### Icons

`assets/icon.svg` is the single source. After changing it:

```sh
npm i -g playwright && node scripts/render-icons.mjs   # PNGs
python3 scripts/pack-icons.py                          # .ico + raw RGBA (needs Pillow)
```

## Commit messages

Releases are generated from the commit history, so commits on `main` follow
[Conventional Commits](https://www.conventionalcommits.org/):

| Prefix | Meaning | Version bump (before 1.0) |
| --- | --- | --- |
| `feat:` | new feature | minor (0.**x**.0) |
| `fix:` | bug fix | patch |
| `perf:`, `refactor:`, `docs:`, `build:` | listed in the changelog | patch |
| `ci:`, `test:`, `chore:` | not listed | – |
| `feat!:` or `BREAKING CHANGE:` in the body | breaking change | minor before 1.0, major after |

Examples: `feat: per-zone gamma`, `fix(windows): tray icon after explorer restart`.

## Release process

Everything after the merge is automatic
([.github/workflows/release.yml](.github/workflows/release.yml)):

1. Push commits to `main`.
2. [release-please](https://github.com/googleapis/release-please) opens or
   updates a pull request **“chore: release x.y.z”**. It bumps the version in
   `Cargo.toml`, `Cargo.lock` and the AppStream metainfo and writes
   `CHANGELOG.md`.
3. Merge that pull request when you want to ship. release-please creates the
   tag `vx.y.z` and the GitHub release with the changelog.
4. The same workflow builds and attaches:
   - Linux: `.tar.gz`, `.deb`, `.rpm` (x86_64; ARM64 can be enabled), `.flatpak`
   - Windows: installer (`-setup.exe`) and portable `.zip`
   - macOS: universal `.dmg` (Apple silicon + Intel)
   - `SHA256SUMS.txt`

To rebuild an existing release, run the workflow manually
(*Actions → Release → Run workflow*) with the tag, or push a tag `v*` by hand.

### One-time repository settings

- *Settings → Actions → General → Workflow permissions*: **Read and write
  permissions** and **Allow GitHub Actions to create and approve pull
  requests** (release-please needs both).
- Optional, for signed and notarised macOS builds – repository secrets:

  | Secret | Content |
  | --- | --- |
  | `MACOS_CERTIFICATE` | Developer ID Application certificate, `.p12`, base64 encoded |
  | `MACOS_CERTIFICATE_PASSWORD` | password of the `.p12` |
  | `MACOS_SIGNING_IDENTITY` | e.g. `Developer ID Application: Patrick Weppelmann (TEAMID)` |
  | `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | for `notarytool` (app-specific password) |

  Without them the app is ad-hoc signed and users confirm it once.

### Flathub

See [packaging/flatpak/README.md](packaging/flatpak/README.md).

## License of contributions

The project is licensed under GPL-3.0-only. By submitting a contribution you
agree that it is published under this license.
