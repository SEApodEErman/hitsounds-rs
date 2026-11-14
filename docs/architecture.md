# Architecture

## Overview

```text
┌──────────────────────── hitsounds-rs (egui / eframe) ───────────────────────┐
│  ┌──────────────┐  direct calls   ┌──────────────────────────────────┐   │
│  │ timeline     │ ──────────────► │ hs-session                       │   │
│  │ channel rack │ ◄────────────── │ Session: project, undo, watcher, │   │
│  │ panels       │  reads / clock  │          playback, settings      │   │
│  └──────────────┘                 └───────┬───────────┬──────────────┘   │
└───────────────────────────────────────────┼───────────┼──────────────────┘
                                            │           │
        ┌───────────────────┬───────────────┘           │
        ▼                   ▼                           ▼
  ┌───────────┐      ┌──────────────┐               ┌─────────────┐
  │ hs-core   │◄─────│ hs-export    │──────────────►│ hs-audio    │
  │ model,    │      │ apply, wav,  │  render mixes │ buffers,    │
  │ projection│      │ backup,      │               │ mix,        │
  │ packing,  │      │ RC checks    │               │ transport,  │
  │ resolve   │      └──────┬───────┘               │ (cpal) M1   │
  └─────┬─────┘             │                       └─────────────┘
        └──────────┬────────┘
                   ▼
            ┌─────────────┐
            │ osu-format  │  lossless .osu read/write, TimingPointChange
            └─────────────┘
```

**Rules**
- `osu-format` and `hs-core` are pure: no I/O beyond explicit load/save helpers, no
  audio, no GUI. All the hard logic lives here and is unit-tested.
- `hs-audio` has no osu! knowledge. `hs-export` is the only crate that writes into a
  mapset.
- `hs-session` holds the editor session (open project, history, mapset watching, playback,
  app settings) with no GUI dependency (D-48). `hitsounds-rs` holds UI state only and calls
  it directly.
- The UI never computes osu! semantics. It renders what the core returns
  (projections, warnings, resolved sounds).

## Crates

| Crate | Responsibility | Key types |
|---|---|---|
| `osu-format` | Line-preserving `.osu` document; typed timing points, hit objects, hit samples; greenline merge | `OsuDocument`, `TimingPoint`, `TimingPointChange`, `HitObject`, `HitSample` |
| `hs-core` | Project file, channels, diff profiles and overrides, projection, packing and index allocation, osu! playback resolution | `Project`, `Channel`, `DiffProfile`, `Override`, `DiffTimeline`, `Projection`, `CustomIndex`, `ResolvedPackage` |
| `hs-audio` | PCM buffers, offline mix, transport state; real-time engine (M1) | `AudioBuffer`, `mix`, `Transport` |
| `hs-export` | Apply hits to a diff, WAV and Ogg Vorbis encoding, backups, ranking checks | `apply_hits`, `WrittenHit`, `blank_wav`, `encode`, `check_sample` |
| `hs-session` | Editor session: project + undo history, mapset reading and watching, playback, sample library scan, mania hitsound diff import, drum separation runs (stem write, reuse, out-of-date check), app settings, the Songs folder list (D-82) | `Session`, `Playback`, `Settings`, `import::HitsoundDiff`, `separation::Separation`, `songs::{SongIndex, Scan}` |
| `hs-separate` | Drum stem separation (D-70 to D-73): HT-Demucs FT drums through ONNX Runtime on the CPU, torch-compatible STFT and chunked overlap-add. Pure: audio in, drum audio out; loads only ONNX Runtime and the model from given paths | `separate`, `DrumsModel`, `SegmentModel`, `Progress` |
| `hitsounds-rs` | The Windows app: egui views, worker thread, and Win32 chrome | `Studio`, `Worker` |

## Data flow

### Editing
1. The UI edits the `Project` (channels, events, overrides) by sending `Edit`s through
   `hs-session`. The core is the source of truth; the UI reads it every frame.
2. When the base layer or a diff profile changes, or a watched `.osu` changes, the
   affected diffs are re-projected and the UI draws the new projection and warnings.
   Projection is linear in events + edges and cheap enough to rerun on every edit.

### Preview
1. Resolve each hit to the files osu! would play (`hs_core::resolve`), with the same
   packing as export. The ear hears the real merge result.
2. The audio engine schedules sample voices against the music clock, using stable's
   playback gain (8 % floor, ×0.8 normal, ×0.85 whistle/clap). Music is the original
   or the stem (toggle).

### Export pipeline
1. **Project** every enabled diff → packages → `resolve` → `ResolvedPackage`s.
2. **Schema**: `build_schema(all requirements, saved schema, allow_growth, first_index)`,
   one schema for the mapset.
3. **Assign** indices per diff (`assign`, longest-run greedy).
4. **Apply and verify in memory** with `apply_hits` and the slider writer. Re-parse
   the proposed files and check their edge and custom tick sounds.
5. **Render** required sample slots in the chosen format (D-69), copying an unchanged
   single source when it already matches. Run ranking checks (on the decoded file for
   OGG) and optional auto-fixes. Show
   the planned `.osu` changes, sample files, and cleanup in the export preview.
6. **Direct export** rechecks the reviewed files, stages replacements, and backs
   up the files it replaces or removes under `<project>/backups/`. It restores
   earlier writes if a later replacement fails.
7. **Clean up** stale files recorded in the export manifest. Existing mapset
   samples are kept by default. Taking them over requires opting in and backs
   up every affected file.
8. **Hitsound-only export** writes a new folder with circles for edge and custom
   tick hits, the generated samples, and the original song when available.

## Audio engine

The real-time stack uses the following components.

- **cpal** output stream. The audio callback owns the clock, and the UI reads the
  playback position every frame.
- **symphonia** decodes the song (mp3/ogg) and samples (wav/ogg). **rubato** resamples
  everything to the device rate once, at load time.
- **Voices**. The app sends transport commands and the compiled hitsound schedule
  through a channel. The mixer starts voices at their frame within the output
  block and loops sustained slider sounds through their scheduled end.
- **Monitor toggle**: the original and the single stem are decoded and kept time-aligned. Switching
  changes which bus is audible, so it's instant and keeps the position.
- **Listening levels** (D-77): `Command::SetVolumes` carries the music, hitsound and
  audition gains, applied while mixing so a change reaches voices already sounding.
  `hs_session::Levels` turns master, music, effects and the two mutes into those gains.
  Each session has its own engine, so the worker applies the saved levels when a project
  opens; a dragged fader goes straight to the engine and is saved on release.
- **Offline render** (export, bounce) reuses `hs_audio::mix`, so preview and export
  produce the same output.

Preview and export share one engine, so what you hear is what gets written.

## App shell

The app calls `hs-session` directly (D-48). A worker handles file dialogs,
settings I/O, mapset refresh, and audio loading. Drum separation runs on a thread of its
own (`hs_session::separation::Separation`, D-72): the worker builds the request, the
shell polls its stage for the progress card, and the finished stem goes back through the
worker as one undo step. `onnxruntime.dll` ships next to the exe (D-74). egui renders the latest workspace
view and reads the playback clock with `try_lock`. A watcher requests repaint and
the shell debounces refreshes.

- The timeline is custom-drawn with egui's painter; hit-testing is done by hand.
- File watching (M5) uses the `notify` crate on the mapset folder, debounced. osu!
  writes a temp file and then renames it.
- App-wide settings (`hs-session/src/settings.rs`) live in `settings.json` in the app's
  config folder: the skin folder (D-40), recent projects (D-41), sample library folders
  (D-45), listening levels (D-77), the Songs and projects folders (D-82) and UI
  preferences.
- `hs-session/src/songs.rs` lists the osu! Songs folder for the mapset picker (D-82,
  `hitsounds-rs/src/mapsets.rs`). It runs on its own thread (`songs::Scan`), caches the
  list in `songs.json` next to `settings.json`, and seeds changed folders from osu!'s
  `osu!.db` (`hs-session/src/osudb.rs`) before falling back to `.osu` headers.
- `hs-session/src/library.rs` lists library folders. `hitsounds-rs/src/chrome.rs` draws the
  custom title bar (D-57) and turns Alt+Space into play/pause (D-46).
- The session lock is always taken before the settings lock.

## Storage

```text
<anywhere>/MyMapset.hsproj/
├── project.json          # hs_core::Project (see project-format.md)
├── samples/              # imported custom samples (copied in for portability)
├── stems/                # optional stem: an imported copy, or drums.ogg separated in-app (D-73)
├── backups/<timestamp>/ # replaced and removed files before direct export
└── exports/
    ├── manifest.json    # owned samples and previous slider writes
    └── hitsound-only/   # separate folder per hitsound-only export
```

Nothing hitsounds-rs needs internally goes into the mapset folder (RC: no unused files).

## Testing strategy

- Unit tests next to the code (`cargo test`). The core rules have tests that mirror
  the examples in methodology.md.
- **Golden round-trip**: every `.osu` fixture must serialise byte-identically when
  untouched. Fixtures must be synthetic or licensed; never commit real mapsets.
- **Export verification**: after applying, `resolve` must reproduce the intended
  sounds. This is the same check export runs at runtime.
- **Parity tests vs Mapping Tools** (M2): run the same layers through MT's Hitsound
  Studio and through hitsounds-rs, then compare the hitsound diff and sample sets.
