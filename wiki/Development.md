## Build and run

Windows 10 or 11 with Rust 1.88 or newer.

```sh
powershell -ExecutionPolicy Bypass -File tools/fetch-onnxruntime.ps1   # once per checkout
cargo run -p hitsounds-rs              # debug build
cargo run -p hitsounds-rs --release    # smooth timeline and audio
```

The fetch script downloads ONNX Runtime (Microsoft's CPU build, pinned and checked
against its SHA-256) for drum separation; the build copies it next to the exe. Without
it, everything except separation works.

## Test and lint

```sh
cargo test --workspace                  # all crates, including egui_kittest UI tests
cargo clippy --workspace --all-targets  # CI treats warnings as errors
```

Tests that need the real Demucs model are ignored by default; see the top of
`crates/hs-separate/tests/parity.rs` and `crates/hs-session/tests/separation.rs`.

CI runs clippy and the tests on Windows for every push to `main` and every pull
request.

## Layout

```text
crates/
  osu-format/    lossless .osu reader and writer, greenline merge
  hs-core/       project model, projection, hierarchy packing, index schema, osu! resolve
  hs-audio/      decoding, resampling, offline mix, real-time playback
  hs-export/     apply hitsounds to difficulties, WAV, backups, ranking checks
  hs-separate/   drum stem separation (HT-Demucs FT drums through ONNX Runtime)
  hs-session/    editor session: project, undo, mapset watch, playback, separation, settings
  hitsounds-rs/  the Windows app (egui and eframe)
docs/            design, decisions, osu! findings, methodology, file formats
installer/       Inno Setup script
tools/           ONNX Runtime fetch, Demucs export, packaging, demo mapset, wiki publish
wiki/            the source of these wiki pages
```

`osu-format`, `hs-core` and `hs-separate` stay pure: no audio device, no GUI, no I/O
beyond explicit loading and saving. `hs-session` holds the editor session without a GUI;
the app crate holds UI state only.

## Ground rules

- **Read [`docs/decisions.md`](https://github.com/SEApodEErman/hitsounds-rs/blob/main/docs/decisions.md)
  before changing behaviour**, and cite decision IDs (`D-09`) in code comments where a
  rule comes from one.
- **osu! behaviour lives in [`docs/findings.md`](https://github.com/SEApodEErman/hitsounds-rs/blob/main/docs/findings.md).**
  If you change a rule, update it and mark anything not checked in game as
  **unverified**.
- **Untouched `.osu` lines must round-trip byte for byte.** Add a round-trip test for
  any parser change.
- **No copyrighted test material**: no mapsets, songs, skins or sample packs. Use
  synthetic fixtures. `sandbox/` is git-ignored for local material.
- **Never commit model weights or `.onnx` files.**
- Code ported from Mapping Tools (MIT) gets a header comment naming the source file and
  an entry in `THIRD_PARTY_NOTICES.md`.

## A demo mapset

`tools/demo-mapset/make_demo.py` generates a synthetic mapset (a made-up beat and two
difficulties), a skin, a sample library, a project and an app config folder. Nothing in
it is copyrighted.

```powershell
python tools/demo-mapset/make_demo.py            # writes sandbox/demo
$env:HITSOUNDS_RS_CONFIG_DIR = "sandbox\demo\config"
cargo run -p hitsounds-rs
```

`HITSOUNDS_RS_CONFIG_DIR` points the app at a separate config folder, so your own
settings are never touched.

## QA hooks (debug builds only)

| Variable | Effect |
|---|---|
| `HITSOUNDS_RS_QA_SIZE=1440x860` | Starts with this window size. |
| `HITSOUNDS_RS_QA_CAPTURE=<file>` | When `<file>` contains a PNG path, the app saves a screenshot of its window there and deletes `<file>`. |
| `HITSOUNDS_RS_QA_INPUT=<file>` | Reads one input event per write: `move x y`, `down x y`, `up x y`, `rightdown x y`, `key F8`, `key E ctrl`, `wheel x y lines [ctrl\|shift\|alt]`. Coordinates are in UI points (pixels divided by the interface scale). |

These are compiled out of release builds. The README screenshots were taken with them.

## Releasing

1. Set `version` in the workspace `Cargo.toml` (for example `0.1.0-alpha.2`) and commit.
2. Tag and push: `git tag v0.1.0-alpha.2` then `git push origin v0.1.0-alpha.2`.
3. The **Release** workflow tests, builds and packages, then opens a **draft** release
   with generated notes, the setup, the portable zip and `SHA256SUMS.txt`.
4. Edit the notes and press **Publish**. Only published releases reach the updater.
   A version with a `-` suffix is marked as a prerelease.

To package locally after a release build, run `tools/package.ps1`. It needs Inno
Setup 6, or pass `-SkipInstaller` for the zip only. Local builds never update
themselves: only the release workflow sets the repository the updater checks.

## Editing this wiki

The pages are Markdown files in `wiki/` in the main repository, so they're reviewed
like code. After a change is merged, publish them with:

```powershell
powershell -ExecutionPolicy Bypass -File tools/publish-wiki.ps1
```
