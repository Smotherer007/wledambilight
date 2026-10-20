#!/usr/bin/env bash
# Regenerates cargo-sources.json (offline crate list for flatpak-builder)
# from Cargo.lock. Run it whenever Cargo.lock changes.
# Needs: python3 with aiohttp and tomlkit (pip install aiohttp tomlkit).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
gen="$here/.flatpak-cargo-generator.py"
if [ ! -f "$gen" ]; then
  curl -fsSL -o "$gen" \
    https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi
python3 "$gen" "$root/Cargo.lock" -o "$here/cargo-sources.json"
echo "wrote $here/cargo-sources.json"
