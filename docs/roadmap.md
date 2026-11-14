# Roadmap

## M0: Scaffold ✅ (2026-11-10)
- Cargo workspace (`osu-format`, `hs-core`, `hs-audio`, `hs-export`) and an egui app shell.
- Lossless `.osu` document; timing points; hit objects; `TimingPointChange` port.
- Project model + JSON; projection; hierarchy packing; index schema; osu! resolve.
- WAV encoder, 44-byte blank, RC sample checks, backups, `apply_hits`.
- Documentation: findings, methodology, architecture, decisions, references.

## M1: Channels, timeline, preview ✅
- Open a mapset: list diffs, read red lines, build the beat-snap grid.
- Channel CRUD and reordering (priority); sample browser + import into `samples/`.
- Custom-drawn timeline: zoom/scroll, snap divisor, place/remove/drag events, select.
- Audio engine: cpal + symphonia + rubato; play/pause/seek/rate; music + hitsound voices.
- Preview through `hs_core::resolve` + packing (exact osu! simulation, D-26).
- Skin folder setting (D-30), with a warning if a needed default sample is missing.
- Project save/load, undo/redo.

## M2: Export, merging and indices ✅
- Render mixes (`hs-audio::mix`) per `(index, slot)`; WAV output.
- Saved schema with growth; export manifest; stale-file cleanup.
- RC check UI with auto-fixes (trim leading silence, pad to 25 ms).
- Hitsound-only `.osu` export.
- Parity tests against Mapping Tools Hitsound Studio output.

## M3: Stem toggle ✅
- One stem file per project; Original ↔ Stem toggle (keyboard shortcut); seamless switch.

## M4: Per-difficulty projection and overrides ✅
- Diff tabs; diff objects lane; matched/fallback/orphaned/tick states on the timeline.
- Override editing (add/remove/mute/replace/slider body), per-diff fallback, leniency.
- "Mute slider ends" dialog (divisors, muted divisors, min length, set/index/volume), with a preview of the affected ends.
- Tick rendering writer: set the slider's object set to Auto, add the tick greenline, revert at +5 ms.

## M5: Direct write + live sync ✅
- Backups → `apply_hits` → verify → write, for every enabled diff (D-19).
- Diff/preview of what will change before writing.
- Watch mapset `.osu` files (`notify`), re-project on change (D-20).

## MVP feedback round (2026-11-10)
- Recent projects on the home screen and in the app menu (D-41).
- App-wide skin folder in Settings, migrated from older projects (D-40).
- osu!-style wheel: seek by snap, Alt zoom, Ctrl divisor; stable snap colours (D-42).
- Compact editor layout with live-apply panels (D-43).
- Master volume envelope with ramps, shared or per difficulty (D-44).
- Sample library browser with audition and drag-and-drop (D-45).
- Alt+Space plays instead of opening the window menu (D-46).
- Box select by dragging, cross-channel moves, double-click rename (D-47).

## Verification

- Synthetic standard and catch difficulties exercise the desktop workflow and
  export. Rust tests check backup bytes, stale previews, rollback, repeated
  writes, and `.osu` round-tripping.
- The hierarchy parity fixture compares generated sample names and resolved
  sets, indices, and volumes against Mapping Tools 1.12.20 Hitsound Studio
  output from the same synthetic layers.
- Playback timing and sample selection are tested in the mixer and simulator.
  Listening to the exported map in osu!stable is **unverified in-game**.

## Editor workspace (D-48 to D-65)
Windows-only egui app, FL Studio-style workspace (D-61) on the quiet graphite palette
(D-60, D-63), flagship polish.
See [design.md](design.md) §8 for the steps.
- [x] `hs-session`, the GUI-free session layer (D-48).
- [x] Flagship shell (D-55) with theme tokens, fonts, icons, reduced motion, custom title bar
  (D-57), guided first run, create/open, settings, a project popover, worker, and toasts.
- [x] Timeline with ghost hits, diff outlines and
  projection marks. Gesture tests cover §8 step 3; the 144 Hz budget test (`cargo test
  --release -- --ignored fit_to_song_frame_budget`) measured p95 2.8 ms CPU with 30 channels
  and 43,200 hits.
- [x] Rail, inspector, library, difficulty panel with issue list (N / Shift+N) and timeline overrides.
- [x] Export view with an automatic preview and a changed-lines `.osu` diff.
- [x] Ctrl+K command palette (D-59).
- [x] FL Studio-style workspace (D-61 to D-65): menu bar and hint bar, toolbar with Draw /
  Paint / Erase / Select, channel rack with step pads, mute lights and volume knobs, docked
  Browser, Channel settings, Difficulty review and Mixer (F8, F6, F7, F9), labelled panels.
  The frame-budget test now covers both zooms: p95 1.7 ms fitted, 1.6 ms with step pads.
- [x] New project from a mania hitsound layer diff (D-66), change the mapset (D-67),
  interface scale (D-68).
- [ ] Hands-on pass over a real mapset.

## Built-in drum separation ✅ (D-70 to D-74)
Separate the drum stem inside the app instead of importing one from a website.
- [x] **Phase 0, spike (go/no-go): go.** See `tools/demucs-export/README.md` for results.
  - [x] Export script (`tools/demucs-export/export_drums_onnx.py`): the drums model of
    `htdemucs_ft` to ONNX (opset 17, 185 MB fp32) with the STFT outside the graph.
  - [x] NumPy reference of the outside parts (`reference.py verify`): 92.5 dB SNR
    against PyTorch Demucs on a synthetic signal, exact length.
  - [x] Timing (6-core desktop CPU, entry-level GPU): CPU 2.7 s per 7.8 s segment, about 2 minutes
    for a 4-minute song. DirectML was slower and wrong, so v1 is CPU only (D-72).
  - [x] Listening test on one of the user's songs: "decent, I can work with this".
    Not a 1:1 match for vocalremover.org, which stays available through import.
- [x] **Phase 1, `hs-separate`.** Resampling, normalisation, torch-compatible STFT/iSTFT,
  Demucs padding and complex-as-channels, chunked overlap-add, `ort` 2.0.0-rc.13 CPU
  session (runtime loaded at run time, model inputs and outputs checked), progress and
  cancel. 16 tests with stand-in models (no weights in CI): STFT round trip and torch
  scaling, overlap-add reproduces the input at any length, a click keeps its sample
  (also through 48 kHz), the spectral branch lines up, cancel and progress, wrong-shape
  model. Ignored `tests/parity.rs` with the real model: **92.5 dB SNR against PyTorch
  Demucs**, 12 s for 20 s of audio on a 6-core desktop CPU.
- [x] **Phase 2, session (`hs_session::separation`).** Separation on its own thread with
  stages, per-segment progress and cancel; peak limiting, OGG write and decode check
  (D-73); `audio.stem_origin`; reuse for the same song and model, including stems in the
  undo history; out-of-date warning from the mapset watcher; one undo step. Tests: the
  whole flow with a synthetic song (apply, song changes, warning, undo, reuse), plus an
  ignored end-to-end run with the real model (`tests/separation.rs`: 12 s of 48 kHz
  audio in 10.6 s, exact length, reuse, cancel writes nothing).
- [x] **Phase 3, UI.** Drum stem section in Project settings (Separate from song or
  again, Import file…, Remove); progress card with segment count, time left and Cancel;
  drums model in App settings (checked when chosen); optional step in Get set up and an
  offer after creating a project; Tools menu and palette entries; the disabled Stem
  toggle says how to get one. Checked on screen with the QA hooks on a synthetic 30 s
  song: separate, listen to the stem (T), reuse, out-of-date warning after replacing the
  song, cancel, undo.
- [x] **Phase 4, docs and packaging.** ONNX Runtime 1.31.1 CPU bundled next to the exe
  (D-74, `tools/fetch-onnxruntime.ps1`); `THIRD_PARTY_NOTICES.md`, architecture,
  methodology, project format, README, CLAUDE.md.
- **Not checked yet:** a separation of a full song in the release build through the UI
  (the 4-minute song was separated through the Python reference in phase 0, and 20 s
  through Rust), the model picker dialog itself (the QA run set the model in the
  settings file), and listening to the stem in game.

## Hands-on pass follow-ups (2026-11-14)
- [x] Empty objects keep the custom index before them when it has no file for their
  hitnormal, so the map stops switching back to the skin's index around them (D-81).
- [x] Mapset picker over the osu! Songs folder, cached list seeded from `osu!.db`,
  projects folder, moved-mapset recovery (D-82). First scan of 8,261 mapsets 0.5 s.
- [x] Starting maximized fills the screen; Restore returns to the earlier size (D-83).
- [x] Tiny viewport frames no longer panic when the editor's 30% panel-width cap drops
  below its 220 px minimum. A synthetic UI regression reproduces the reported bounds
  and checks zero-size frames and restoration while refresh disables editing, with
  animations on and off.
- [ ] Listen in game to an export where empty objects stayed on a custom index (D-81).

## Tester feedback (2026-11-14)
- [x] Tools reordered to Select, Draw, Paint, Delete on keys 1–4 (D-86).
- [x] Listening levels take a typed value (D-87).
- [x] Copy, cut and paste of hits at the playhead, replacing the pasted span (D-88).
- [x] Hit volume lane: Ctrl+click adds a point, a click on a point switches ramp and step (D-89).
- [ ] Check all four on screen in a real mapset.

## Alpha release (D-84, D-85)
- [x] Background update check, download and checksum, applied on exit (installer or
  portable swap), *Restart now* toast and App settings section (D-84). Unit tests for
  release choice, checksum lists and the flat-archive rule.
- [x] Inno Setup per-user installer, portable zip, `tools/package.ps1` (D-85).
- [x] CI workflow; release workflow that drafts a GitHub release from a version tag.
- [ ] Push the repository to GitHub (public) and tag `v0.1.0-alpha.1`.
- [ ] End-to-end update test: install `alpha.1`, publish `alpha.2`, check that the
  installed and the portable copy both update and restart. Not yet run.
- [ ] Code signing before a wider release.

## Later (post-MVP)
- Loop range, tap-to-record (D-53). Copy/paste of hits is done (D-88).
- Opt-in storyboard-sample fallback (D-06).
- OGG export for gameplay sounds; audibility lint.
- osu!lazer support.
- Onset/spectrogram assists (D-23 says not needed now).
- Optional silent slider ends with a 44-byte blank index (OQ-7).
