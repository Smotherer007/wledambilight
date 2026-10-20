# Flatpak

`io.github.smotherer007.WledAmbilight.yml` builds the app against the
freedesktop 25.08 runtime. Screen capture uses the ScreenCast portal and
GStreamer's `pipewiresrc` from the runtime, so the sandbox needs no special
permissions.

## Build locally

```sh
flatpak install --user flathub org.freedesktop.Sdk//25.08 org.freedesktop.Sdk.Extension.rust-stable//25.08
pip install --user aiohttp tomlkit
bash packaging/flatpak/update-cargo-sources.sh
flatpak-builder --user --install --force-clean build-flatpak \
    packaging/flatpak/io.github.smotherer007.WledAmbilight.yml
flatpak run io.github.smotherer007.WledAmbilight
```

The release workflow builds the same manifest and attaches a
`.flatpak` bundle to every GitHub release
(`flatpak install --user wled-ambilight-*.flatpak`).

## Publishing on Flathub

1. Make sure a tagged release exists and `docs/screenshot.png` is on `main`
   (the metainfo links to it).
2. Fork <https://github.com/flathub/flathub>, branch off `new-pr`.
3. Copy the manifest and `cargo-sources.json` into the root of the fork and
   replace the `dir` source with the release tag:

   ```yaml
       sources:
         - type: git
           url: https://github.com/Smotherer007/wledambilight.git
           tag: v0.2.0
           commit: <full commit hash of the tag>
         - cargo-sources.json
   ```

4. Open the pull request against `new-pr`. Flathub's bot builds it; the
   reviewers check the metainfo (`flatpak run --command=flatpak-builder-lint
   org.flatpak.Builder appstream …` runs the same checks locally).
5. After the merge you get a `flathub/io.github.smotherer007.WledAmbilight`
   repository. Each new release is a pull request there with the new tag,
   commit and a regenerated `cargo-sources.json`.

The app id `io.github.smotherer007.*` is verified automatically through the
GitHub account `Smotherer007`.
