# Decisions

These decisions come from the planning Q&A (2026-11-10). IDs match the original
question numbers so they can be referenced from code (`D-09`).

## Platform & ecosystem

| ID | Decision |
|---|---|
| D-01 | **osu!stable only** for now. Lazer is out of scope. |
| D-02 | **Rust throughout**, with a native **egui/eframe** UI. |
| D-03 | **Standalone replacement** for the Mapping Tools hitsound workflow, with a **one-way importer** from Hitsound Preview Helper mania layer diffs (post-MVP). |

## osu! semantics

| ID | Decision |
|---|---|
| D-04 | Target **osu!standard + catch**. Catch uses standard hitsounding. |
| D-05 | Slider heads/repeats/tails are treated like circles. A diff's hit objects are shown on the timeline and can be selected and given hitsounds (an `AddEvent` override). Events on a slider tick can be rendered as custom `slidertick` samples (`TickPolicy::Render`). `sliderslide` is supported as a per-slider override (`SliderBody`), usually constant or unused. |
| D-06 | Storyboarded-sample fallback is **ignored at first** and comes later as opt-in. RC forbids it as a replacement for active hitsounds. |
| D-07 | Follow **Mapping Tools conventions**, verified against the source in findings §4–6: one greenline per hit, merged with `TimingPointChange` so SV, kiai and existing greenlines are preserved. Greenlines keep the difficulty's own sample set (D-79). |
| D-08 | **Never write the `hitSample` filename field.** |

## Merging & indices

| ID | Decision |
|---|---|
| D-09 | **Hierarchy merge, as in Mapping Tools.** Channel order is priority. A lower channel's sound is mixed into the sample set of the higher one for the same sound type (normal, or additions). |
| D-10 | A reverse index lookup ("which file plays here") is **not a core feature**. Missing or wrong hitsounds are fixed in the editor view. |
| D-11 | No index budget. Indices may be overridden, set back to default (skin) on lines, or a new index may hold only a custom sample with the other slots left to the skin. |
| D-12 | Ranking checks run as **warnings with auto-fix**: WAV/OGG only, ≥ 25 ms, peak ≤ 5 ms, 44-byte blank for silence, no unused files. |

## Channels

| ID | Decision |
|---|---|
| D-13 | **Each channel has exactly one sample assignment**, DAW-style. No time regions. Two kicks for two song sections are two channels. |
| D-14 | Channel controls: **volume, mute/solo, colour**. |
| D-14a | *(Not discussed; Claude's call, open to change.)* Mute/solo are **monitoring only** and never change export. Exporting with a channel accidentally muted would silently drop hitsounds. |
| D-15 | Sample browser + import is enough. SoundFonts and VSTs are out of scope. |

## Multi-difficulty

| ID | Decision |
|---|---|
| D-16 | **Projection model**: base events ∩ diff edges (± leniency, default 5 ms as in Hitsound Copier), plus per-diff overrides. |
| D-17 | **Each diff overrides only the base.** No diff-to-diff inheritance. |
| D-18 | Edges with no event use a **configurable fallback** channel. It's set per project and can be overridden per diff. |
| D-19 | **Write directly into the diffs with automatic backups.** Hitsound-only `.osu` export is an option. |
| D-20 | **Live sync**: watch the diffs and re-project when they change. |

## Audio

| ID | Decision |
|---|---|
| D-21 | **Revised by D-70.** Stems come from an external website and already match the original's length and offset. No offset nudging needed. |
| D-22 | A simple **Original ↔ Stem toggle** (no mixer). The project holds **one stem file**, normally the drum stem (D-32). |
| D-23 | No onset or spectrogram assists. |
| D-24 | Stems are project assets. The mapset's `AudioFilename` is never touched. |

## Editing & storage

| ID | Decision |
|---|---|
| D-25 | **No piano roll**: osu!-editor-style placement on the beat-snap divisor. |
| D-26 | Preview **simulates osu! playback**. |
| D-27 | The project (JSON) lives **outside the mapset folder**, because unused files must be removed before ranking. |

## Scope & licence

| ID | Decision |
|---|---|
| D-28 | MVP = channels/timeline/preview, export with merging and indices, stem toggle, per-diff projection and overrides, direct write. **Not in MVP**: the mania importer and the onset/spectrogram assists. |
| D-29 | Name **hitsounds-rs**, open source. Mapping Tools turned out to be **MIT**, so hitsounds-rs is **MIT** as well. Ported code keeps the MIT notice and attribution (see `THIRD_PARTY_NOTICES.md`). |
| D-30 | **Default skin samples are provided by the user** (a skin folder). osu!'s default resources aren't freely distributable, and Mapping Tools doesn't bundle them either. |

## Follow-up decisions (2026-11-10, second round)

| ID | Decision |
|---|---|
| D-31 | At **addition-only moments** the always-playing hitnormal is the **skin hitnormal of the addition's set**, and it is added **once** however many additions coincide (test `addition_only_with_several_additions_adds_one_skin_hitnormal`). `export.addition_only_normal = "silent"` remains as an option. **Extended by D-78**: the project can fix the hitnormal's set instead. **Default revised by D-91**: the difficulty's greenline set. |
| D-32 | Events are stored as **integer ms**. Re-timing red lines doesn't move them, the same as osu! objects. |
| D-33 | **Mute slider ends, like Hitsound Copier.** This is a per-diff `slider_end_mute` filter. The user picks which ends are muted: the divisors to consider (priority order), which divisors mute, minimum length in beats, whether spinner ends are included, and the muted sample set/index/volume. Only ends **without an event** qualify, so a deliberate hitsound always wins. Repeating sliders are never muted. A muted end is no longer a fallback candidate. Edges that are neither matched nor muted get the fallback. |
| D-34 | **One stem file** per project (`audio.stem`). The user only works with drum stems. |
| D-35 | *Retired.* |

## Follow-up decisions (2026-11-10, MVP feedback)

IDs D-36 to D-39 are cited in code but were never recorded here, so this round starts at D-40.

| ID | Decision |
|---|---|
| D-40 | **The skin sample folder is an app-wide setting**, not a project field (revises where D-30's folder is stored). It lives in the app's `settings.json` with the recent list. Format-1 projects that still carry `skin_dir` keep loading: when no app-wide folder is set yet, opening such a project adopts its folder. The field is never written back. Exports now depend on the machine's skin setting, which is the trade-off the user chose to remove per-project friction. |
| D-41 | **Recent projects** (up to 12, most recent first) are kept in app settings and shown on the home screen and in the app menu. Missing projects stay listed, marked as missing, until removed. |
| D-42 | **The timeline follows osu!stable editor conventions** (findings §9): the wheel seeks by the snap divisor (Shift = 4 ticks, one beat while playing), Alt+wheel zooms, and Ctrl+wheel halves or doubles the divisor. Over the channel rail the plain wheel scrolls channels. Grid and ruler ticks use stable's beat-snap colours. |
| D-43 | **Compact editor layout.** One top bar (app menu, difficulty picker, transport, snap, monitor, undo/redo, Export). The timeline is the main area; channels are added, muted and soloed from its rail. The right panel shows only the selection (channel, or difficulty in a difficulty view), and every field applies on change with no Apply buttons. Mapset, stem, default fallback and the skin folder live in Settings. Zoom, Follow, the legend and warnings sit in the status bar. |

## Follow-up decisions (2026-11-10, MVP feedback, second round)

| ID | Decision |
|---|---|
| D-44 | **Master volume envelope.** One curve of points (time, 5–100 %, and a segment that is a linear ramp or a step) scales the **greenline volume** of every hit: `round(loudest channel × envelope(t))`, clamped to 5–100. Mixes and custom indices are untouched, so volume automation never adds sample files. Rendered slider ticks follow it too. Mute-slider-end volumes (D-33) and reset edges don't. All difficulties share the base curve; editing the Volume lane in a difficulty view gives that difficulty its own curve, which replaces the base one (as with D-17). "Use shared" drops it. Per-channel and per-event volume automation were considered and rejected for now: both bake gain into the mixes and multiply indices. |
| D-45 | **Sample library browser.** Library folders are an app setting. The Samples tab lists their audio files (folder tree, or flat search results); clicking auditions, double-click/Use/drag loads a sample into a channel. Only a used sample is copied into the project's `samples/`; re-importing an identical file reuses the existing copy. |
| D-46 | **Alt+Space toggles playback** instead of opening the Windows window menu. The main window is subclassed to swallow that one `SC_KEYMENU` and forward it to the UI. Other system keys are untouched. |
| D-47 | **Timeline editing.** A drag on empty lane space draws a selection box (a click still places an event). Dragging events moves them in time and, vertically, to other channels (`MoveEvents.rows`); in a difficulty view the move becomes remove/add overrides. Double-clicking a channel name renames it. |

## Editor design (2026-11-10)

| ID | Decision |
|---|---|
| D-48 | **The editor session is a GUI-free crate** (`hs-session`): open project, undo history, mapset watching, playback and app settings. The egui UI calls it directly and reads the project and the audio clock every frame, so there is no second copy of the model to keep in sync. The design is in [design.md](design.md). |
| D-49 | **Layout superseded by D-61**; the layer tabs, app-wide Settings, dark theme and apply-on-change rules still hold. **Layout** (replaces D-43; design.md §4). Top bar: app menu, project name (opens project settings: mapset, stem, default fallback, refresh), transport with bar:beat, undo/redo, app Settings, gesture sheet, Export. A **layer bar** of tabs (Base + one per difficulty, each with its open issue count) replaces the difficulty dropdown and carries the timeline tools (monitor, snap, zoom, follow). Sample library in a left panel, inspector (Channel, Difficulty) on the right; both closable and resizable. Settings is app-wide only. Dark theme only for now. Fields still apply on change. |
| D-50 | **Difficulty layers are a visible mode** (§5.3–5.4). The layer is tinted and labelled ("changes apply to this difficulty only"), diff-only and removed hits are drawn differently from base hits, and the Difficulty inspector opens with an **issue list** (empty, fallback, orphaned, tick) that jumps to each one. N / Shift+N step through issues. |
| D-51 | **Overrides are made on the timeline** (§5.5); the override form is retired and the list stays for review. **Right-click on a hit deletes it** (osu! editor convention); right-click on the channel rail, an object or empty lane space opens a context menu. |
| D-52 | **Export is a full view** (§5.7) with grouped options on the left and a preview that **builds automatically** on open and after any option change, with a progress state. The `.osu` comparison is a line diff of changed lines. |
| D-53 | **Not in this design**: loop range, copy/paste of hits, and tap-to-record (§5.9). They stay on the roadmap for later. Copy/paste was built after tester feedback (D-88). |
| D-54 | **Superseded by D-60.** Visual direction: rhythm-game vivid (design.md §10.2). Deep indigo base (`#120F24`, surfaces `#1C1836`), neon cyan (`#5CF2E0`) for interaction and the base layer, hot magenta (`#FF5CB8`) for difficulty layers. Hits flash and glow as the playhead passes them. Projection-state colours get re-derived so they don't collide with the accents, and the magenta must not read as osu!'s brand pink. A *Reduce motion* setting turns the animation off. |
| D-55 | **The bar is flagship polish**, not a basic shell (§10.1): one token system (colour, type, spacing, radii, elevation, motion), bundled fonts, one icon set, every state designed (empty, loading, error, success), purposeful motion, keyboard-first, a guided first run, and a smooth timeline at high refresh rates. |
| D-56 | **Humour lives in Easter eggs only** (§10.4): a launch-parody About dialog ("engineered for an audience of one"), release codenames, a startup line, a harmless key combo. Working UI copy (errors, warnings, empty states, export review) stays plain. |
| D-57 | **Custom title bar** (§10.3). The top bar is the window's title bar, with its own minimize/maximize/close. Windows 11 snap layouts, resize edges, double-click maximize and the system menu keep working natively. |
| D-58 | **The app targets Windows only** (10 and 11), so Windows-specific polish (custom title bar, Mica backdrop, D-46) needs no cross-platform compromises. |
| D-60 | **Sizes and radii superseded by D-63**; the palette and type choices still hold. **Quiet graphite chrome, one accent** (supersedes D-54, design.md §10.2). Every surface comes from one neutral graphite ramp (`#0F0F10` to `#232325`, hairlines `#303032`), text in three roles (`#EDEDED`, `#A1A1A6`, `#85858B`). Teal `#4DD6C8` is the only accent: primary action, selection, focus, the base layer and the timecode. Orchid `#DE78C8` appears only inside a difficulty layer (D-50). Static halos and washes are gone. The hit flash as the playhead crosses a hit stays, because it carries meaning (D-54's "hits play visibly" still holds). Headings are Inter Display Medium, so Space Grotesk Bold is dropped. Labels are sentence case, never tracked uppercase. Sizes sit on the 4 px grid, controls are 32 px and radii nest (control 8, card 12, panel 16). Shortcuts show as keycaps. |
| D-59 | **A Ctrl+K command palette** (answers the open question from design.md §11). It lists every shell and editor action by name, filtered by context, and only builds the same `Intent`s the buttons and shortcuts emit, so it adds no behaviour of its own. Per D-56 its copy stays plain. |

## FL Studio-style workspace (2026-11-12)

The first hands-on pass over the app found it unpolished and unclear for someone
opening it for the first time: icon-only tools, a cramped channel rail, an inspector with
unlabelled fields. The user asked for a rework in the manner of FL Studio and answered
three questions: FL layout and workflow **without** patterns, FL density on the current
palette, and docked panels with FL-style toggles.

| ID | Decision |
|---|---|
| D-61 | **The workspace is laid out like FL Studio** (supersedes the layout in D-49; design.md §4). The title bar carries a real menu bar (**File, Edit, View, Tools, Help**) that lists every action by name with its shortcut, a **hint bar** that says what is under the pointer and what click, drag, right-click and wheel do there, and the project name. Below it sits one **toolbar**: play, stop, a time display with bar:beat, speed, snap, the pointer tools (D-62), undo/redo, the panel switches and Export. The **layer strip** keeps D-49's tabs and D-50's visible mode, and adds a sentence saying where edits go. The timeline is a **channel rack**: each channel is one compact row (reorder handle, mute light, solo, volume knob, coloured name button, osu! slot) and its lane is a row of **step pads**, one per snap step, shaded by beat, with hits as lit pads. Pads are centred on their tick, so placement still rounds to the nearest snap as in the osu! editor (D-25); zoomed out, pads give way to thin markers. Panels are **docked**, never floating: Browser left (F8), Channel settings and Difficulty review as two tabs on the right (F6, F7), Mixer along the bottom (F9). Song/Stem, zoom, fit and follow sit beside the lanes they act on. Project settings is a dialog under File, not a popover. The command palette (D-59) and the gesture sheet stay. |
| D-62 | **Order, names and keys revised by D-86.** **Four pointer tools**, with FL's keys: **Draw** (P) is the old behaviour (click places, drag on empty space selects, drag a hit moves it, D-47); **Paint** (B) drags a hit onto every snap step of the channel the stroke began on; **Erase** (D) deletes the hits it is clicked or dragged across; **Select** (E) never places. Right-click deletes a hit with every tool (D-51). Each stroke is one edit and one undo step. |
| D-63 | **FL density and hardware-style controls on the D-60 palette** (supersedes D-60's sizes). Controls are 26 px, channel rows 30 px, body text 13 px, meta text 11 px, radii nest at 5 / 8 / 10. Mute is a light that flashes as the channel's hits play (right-click solos, as in FL), channel volume is a knob in the rack and a fader in the mixer, and side-panel fields are rows of a fixed-width label and a control under plain section titles, each with a one-line explanation where the term is osu!- or app-specific. Colours, typefaces, sentence case and the single teal accent are unchanged. The minimum window is 940 × 560. |
| D-64 | **What stays osu!, and what was left out.** The wheel keeps the osu! editor conventions of D-42 (seek by snap, Alt zoom, Ctrl divisor), not FL's scroll and zoom. Mute and solo stay monitoring-only (D-14a), and the mixer has no master fader because hit volume is the envelope of D-44 (listening levels came later, D-77; they are not an export fader). **Patterns are not part of this**: the user chose layout and workflow only, so hits stay individual events and D-53 (loop range, copy/paste, tap-to-record) still stands. |
| D-65 | **"An audience of one" is the joke, not the design brief** (clarifies D-55, D-56). The working UI must be clear to a hitsounder who has never seen the app: every icon has a named menu entry and a hint-bar line, and no control depends on a hidden gesture alone. |

## Import, mapset and scale (2026-11-13)

| ID | Decision |
|---|---|
| D-66 | **New project from a mania hitsound diff** (the importer D-03 and D-28 left for later). The user picks the `.osu`; its folder is the mapset when it has standard or catch difficulties, otherwise the one sibling folder with the same set number that does (a copy such as `123 Artist - Title - hs`), otherwise the user picks the mapset. Each distinct sound becomes a channel in column order, with the sound's slot, the sample copied into `samples/`, and the note volume as channel volume. Greenline volume becomes the base volume envelope (D-44). Notes resolve the way stable plays mania (findings §10). The whole import is one undo step. Importing into an already open project was not asked for and isn't offered. |
| D-67 | **The project's mapset can be changed** (File, Project settings, palette). Channels, hits, overrides and the envelope stay. Difficulty profiles carry over by file name; a profile for a file the new folder lacks is dropped if it holds only defaults and kept, with a missing-file warning, if it holds settings or overrides. The new folder must have a standard or catch difficulty. The change is one undo step, and undo or redo re-reads and re-watches the folder. |
| D-68 | **Interface scale is an app setting** (90 % to 200 %, default 100 %), on top of the Windows display scale, through egui's zoom factor. Ctrl+= / Ctrl+- / Ctrl+0 and View > Interface scale change it. Sizes whose 940 × 560 minimum window (D-63) wouldn't fit the monitor are disabled. The minimum window is enforced in scaled points by the title bar's `WM_GETMINMAXINFO`, since winit's own minimum adds a frame this window doesn't have. |
| D-69 | **Export file type for samples, as in Mapping Tools' Hitsound Studio.** Two project export settings: one for samples with a single source and one for mixes. Choices: 16-bit WAV (the default, and what earlier versions always wrote), 32-bit float WAV, Ogg Vorbis, and, for single samples only, *Same as source*. A single source at full gain that no auto-fix changed is copied byte for byte when it already is the asked-for format (MT's `IsCopyCompatible`); *Same as source* copies any WAV or OGG and re-encodes OGG as OGG and everything else as 16-bit WAV. MP3 and FLAC are never copied. Silent samples stay the 44-byte WAV. OGG is encoded with libvorbis (aoTuV) at quality 0.6 and a fixed stream serial, so re-exports are byte-identical. Each encoded OGG is decoded again: it must keep its length and onset (within 1 ms), and the ranking checks run on the decoded audio. MT's 1024-sample lead-in is not copied: it compensates for MT's C# encoder, and libvorbis decodes aligned without it. Switching format removes the previous file of the same stem, since osu! prefers `.wav`. MT's soft limiter before 16-bit and Vorbis output is not adopted; mixes still clip at full scale as before. MIDI export is out of scope. |

## Built-in drum separation (2026-11-13)

| ID | Decision |
|---|---|
| D-70 | **hitsounds-rs can separate the drum stem itself** (revises D-21). Project settings offers *Separate from song* next to *Import file…*. It runs the drums model of **HT-Demucs FT** (`htdemucs_ft`, from the maintained fork `adefossez/demucs`, v4.1.0) on the song as hitsounds-rs decodes it, so the stem lines up with what the app plays by construction. The result fills the project's one stem slot: drums only, D-34 stands. Importing a file stays. The quality bar is the user's current site (vocalremover.org, proprietary models); if Demucs falls short of it in the phase 0 A/B, import stays the main path and separation is the quick option. Splitting drums further (kick, snare, hats) was considered and left out. UVR5 was looked at and dropped: it is a front end for other models, and for drums it runs this same Demucs model. |
| D-71 | **The user provides the model file; hitsounds-rs never bundles, hosts or downloads weights.** The Demucs code is MIT, but its README says the licence of the weights "is not clear". An MIT export script in `tools/` converts the official `htdemucs_ft` drums model to ONNX once; the user points app Settings at the file, as with the skin folder (D-30). The app checks the file's inputs and outputs before using it and records its SHA-256 with the stem (D-73). Without a model, *Separate from song* explains how to get one and import still works. |
| D-72 | **Separation runs natively** in a new crate, `hs-separate`, kept pure like `hs-core` (audio in, drum audio out; the only I/O is loading the model from a given path). Inference uses ONNX Runtime through `ort` **on the CPU**, and the runtime DLL loads dynamically so a missing runtime only disables separation (hitsounds-rs ships it, D-74). DirectML was tried in phase 0: on an entry-level GPU it returned wrong output (−90 dB against the CPU, with graph optimisation on or off) and was slower than the CPU anyway (4.8 s against 2.7 s per segment on a 6-core desktop CPU). A GPU path is left for later and may only be used after it matches the CPU on a test segment. STFT, iSTFT, normalisation and the chunked overlap-add (about 7.8 s segments, 25 % overlap, triangular crossfade) are written in Rust after `demucs/apply.py`, credited in `THIRD_PARTY_NOTICES.md`. Songs not at 44.1 kHz are resampled for the model and back. It runs on **its own thread**, never the session worker, so editing and playback continue; it reports progress per chunk, can be cancelled between chunks, and applying the result is one undo step. |
| D-73 | **The separated stem is stored as Ogg Vorbis**, `stems/drums.ogg`, through the D-69 encoder (libvorbis aoTuV, quality 0.6, fixed serial), at the song's sample rate and decoded length. Separation uses `hs_audio::decode_file`, as Song monitoring does. Tagged MP3 delay and padding are trimmed to match the BASS decode (D-26, findings §11). An older generated stem for the same song hash, at the song's rate and with the exact untrimmed frame count, receives the same trim in playback. Demucs output that peaks above full scale is scaled down as a whole. The Ogg is decoded again to check its format and reject a confidently measured shift of more than 1 ms around its first strong attack. The project records the model and song SHA-256 in `audio.stem_origin` for separated stems. Running separation again for the same song and model reuses the file. When the mapset watcher sees the song change, the app warns that the stem is out of date and offers to separate again. |
| D-74 | **hitsounds-rs ships ONNX Runtime**: Microsoft's plain CPU build (`onnxruntime-win-x64`, 1.31.1, MIT), as `onnxruntime.dll` next to the exe with its `LICENSE` and `ThirdPartyNotices.txt`. `tools/fetch-onnxruntime.ps1` downloads the pinned release and checks its SHA-256; the build of `hitsounds-rs` copies it next to the exe. The app loads it by full path, never from `PATH` or System32, where Windows keeps an older copy of its own. It matched PyTorch Demucs at 92.5 dB, the same as the Python wheel. 1.32.0 was skipped because it came out the day this was decided. The weights stay the user's own (D-71). |

**How phases 2 and 3 implemented D-70 to D-74** (2026-11-13): the session hands a request to its own thread (`hs_session::separation`), which hashes the model and song, decodes the song with `decode_file`, runs `hs-separate`, scales a stem over full scale to a 0.98 peak, encodes and decodes the OGG to check it, and writes it without replacing any file (`stems/drums.ogg`, then `drums (2).ogg` and so on, so undo can always go back). The result applies as one `SetStem` edit carrying `stem_origin`. Reuse looks at the current, saved and undo/redo states of the project, so a stem that was undone is still found. The model's SHA-256 is remembered for the session, since the file is 185 MB. The mapset watcher now reacts to audio files as well as `.osu` files, and playback reloads a track whose file size or modified time changed. The model is checked by loading it once when it is chosen in App settings. In the UI: a *Drum stem* section in Project settings (*Separate from song* / *Separate again*, *Import file…*, *Remove*), a progress card with segment count, time estimate and *Cancel*, a *Drum separation model* section in App settings, an optional step in Get set up, an offer to separate after creating a project when a model is set, Tools menu and palette entries, and the disabled *Stem* toggle says how to get one.

## Crash reports (2026-11-14)

| ID | Decision |
|---|---|
| D-75 | **A panic on any thread leaves a crash report.** Release builds abort on panic and have no console, so a panic used to close the app without a word (the first case: the post-create *Separate drum stem* toast outlived its timer and `Duration::from_secs_f64` got a negative value). The app's panic hook appends the time, version, thread, message, location and a backtrace to `crash.log` in the native config folder (the temp folder if that fails; past 256 KB it becomes `crash.old.log`), then shows an error dialog that names the file and offers to show it in File Explorer. Only the first panic shows a dialog. The release profile keeps line tables in a separate `.pdb` (`debug = "line-tables-only"`, `strip = "none"`) so backtraces name functions and lines; the exe stays the same size. No reports are sent anywhere. |

## Rename (2026-11-14)

| ID | Decision |
|---|---|
| D-76 | **The project is renamed from HS Studio to hitsounds-rs** (revises the name in D-29), written in lowercase everywhere, including the UI. The app crate and exe become `hitsounds-rs`. The library crates keep their `hs-` prefix, and projects keep the `.hsproj` extension, since "hs" still means hitsounds. The config folder becomes `dev.hitsounds-rs.app`, the debug QA variables become `HITSOUNDS_RS_*`, and the model export script writes `hitsounds_rs.*` metadata keys. The app does not read those keys, so models exported earlier still work. Settings are not migrated: before 0.1 the only install is the developer's own, and that folder was moved by hand. |

## Listening levels and lone additions (2026-11-14)

From the first hands-on pass in a real mapset.

| ID | Decision |
|---|---|
| D-77 | **Listening levels, as in osu!'s options.** The mixer's first strip, *Listening*, has **Master**, **Music** and **Effects** faders and two mutes: **Mute music** and **Mute all channels**. Master scales everything. Music scales the song or drum stem. Effects scales hitsounds and sample auditions. They are monitoring only, like D-14a: they never touch the project or export, and they make no undo step. *Mute all channels* silences every scheduled hit without changing any channel's own mute or solo, so turning it off restores exactly what was set; the rack's mute lights go dark while it is on, and sample auditions still play. The three volumes are an app setting (`levels` in `settings.json`, default 100 / 60 / 100 %, the old fixed mix). The mutes last for the session, so the app never opens silent. Both mutes are also in Tools > Listen to and the command palette (D-65). The engine applies the levels to voices that are already sounding, and a dragged fader is heard live and saved on release. |
| D-78 | **The hitnormal under lone additions can have a fixed set** (extends D-31). This is a project setting, *Hitnormal under lone additions* in Project settings. The choices are the skin hitnormal of the additions' set (D-31, Mapping Tools; the default until D-91), the skin's **soft**, **normal** or **drum** hitnormal, or silent. With a fixed set the object's normal set is that set and the addition set stays the top addition's, so a normal-set crash plays over `soft-hitnormal`. A real hitnormal channel at the moment always wins. The plain skin sound needs no file (findings §2). The setting applies to preview and export alike, so it moved out of the Export page, which now only shows it; the stored field is still `export.addition_only_normal`, with new values `skin_soft`, `skin_normal` and `skin_drum`. A *channel* as the default hitnormal (for a custom sample) was considered and left for later. |
| D-79 | **Hit greenlines keep the difficulty's sample set** (refines D-07). Export and preview still write one greenline per hit for its index and volume, but no longer its sample set. Every hit carries concrete sets on its object or slider edge, so it never needed the greenline's. Writing it changed the set of everything after the hit that is left on Auto until the next greenline: objects with no hit (D-36 resets), slider slides, and slider ticks that aren't rendered. A normal-set kick made them play `normal-*` where the mapset uses soft. Rendered ticks (D-05) still write their own set, with the revert after them. A difficulty that an earlier version already exported directly keeps the sets that export wrote; restore it from its backup to get the mapper's sets back. |
| D-80 | **Export removes greenlines that change nothing** (an export option, on by default: *Remove greenlines that change nothing*, `export.remove_redundant_greenlines`). After hits, ticks and slider bodies are applied, a greenline is dropped when its SV, sample set, index, volume and kiai all equal the point before it (`osu_format::timing::remove_redundant`). The merge (D-07) already skips a new greenline that changes nothing, but it never removed existing lines, and hit greenlines often make the mapper's own index or volume changes redundant. Those are removed too, which is why it is an option. Redlines are never removed. The result is read back and every edge checked as before, and the export review shows how many were removed. On the first real mapset this took a difficulty from 632 written greenlines to 570. |

## Sample index, mapset picker and window (2026-11-14)

From the same hands-on pass: the custom index switched back to the skin's too often,
picking mapsets and project folders meant browsing for folders every time, and the app
didn't fill the screen when it started maximized.

| ID | Decision |
|---|---|
| D-81 | **An empty object keeps the custom index before it when that changes nothing** (refines D-36). An edge with no hit and no fallback still becomes a plain default hit with Auto sets and no additions, but it no longer writes a greenline back to index 0 whenever the index of the hit before it would play the same sound. That is when the active index (after the other hits' greenlines) is 0, or an index the export writes that has no file for the edge's `<set>-hitnormal`; a slider head or repeat also needs no file for its set's `sliderslide`, `sliderwhistle` or `slidertick`. A missing indexed file falls back to the skin (findings §2), so the edge plays the skin's hitnormal either way. An index the export doesn't write (the mapper's own files) still goes back to 0, since any file may exist for it. Resets are decided in time order: one that goes back to 0 makes 0 active for the empty objects after it, up to the next hit. Preview and export share the rule (`apply_hits`), so preview still plays what export writes. Example: a lone custom snare (a drum clap) on index 7 followed by empty objects on soft: they stay on index 7 and play the skin's `soft-hitnormal`, with no greenline until the next hit. **Fix (2026-11-14):** a hit whose greenline the merge left out (the point before it already gave its index and volume) lost that index when a reset wrote its index-0 greenline in between. Export then failed verification with *the written hit does not resolve to the intended sounds* and *neighboring timing changes conflict*, which the user could not fix. In the first real mapset this blocked all five difficulties. A reset that writes a greenline now gives the next hit its own greenline back. |
| D-82 | **Mapsets are picked from the osu! Songs folder by name** (new project, Change mapset, a moved mapset). The picker is a song-select-like list: search across artist, title, mapper, difficulty names, tags and set ID, sort by most recently changed (default), artist or title, Up/Down and Enter, double-click. Mapsets without a standard or catch difficulty are listed but can't be chosen. *Browse for a folder…* keeps the old file dialog. **The Songs folder** is an app setting; without one, the Songs folder of the osu!stable install is used: `%LOCALAPPDATA%\osu!`, else the folder of the `osu!.exe` registered for `.osz` files, with the user config's `BeatmapDirectory` when it moved the folder. **The list is cached** in `songs.json` in the app's config folder (the "small database"), JSON rather than SQLite since it is read whole and is about 7 MB for 8,000 mapsets. Each time the picker opens it lists the Songs folder again on its own thread and re-reads only folders whose `.osu` files changed (name, size, modified time). A changed folder takes its metadata from osu!'s own `osu!.db` next to the Songs folder when that lists every `.osu` file in it, else from the files' headers, read in parallel. On the developer's install (8,261 mapsets, 32,841 difficulties) the first scan took 411 s reading headers one at a time and 0.5 s with `osu!.db` and threads; a refresh takes 0.4 s, and a search 5-7 ms. **Projects folder**: an optional app setting; with it, *New project* creates `<folder>\<mapset folder>.hsproj` (with ` (2)` and so on if taken) without asking. Without it the save dialog starts in the folder of the most recent project. **A moved mapset**: opening a project whose mapset folder is gone first looks it up in the cached list (same folder name, else the folder holding most of the project's difficulty files); found, the project opens there with the new folder as an unsaved change; not found, the picker opens with the new location preselected once its refresh finds it. Reading `osu!.db` was considered as the only source and rejected: its layout changes between osu! versions and it misses mapsets osu! hasn't imported yet. |
| D-83 | **A maximized window starts maximized for real.** eframe restores a maximized window hidden with `WS_MAXIMIZE` set, so the custom frame (D-57) must give it the maximized rectangle itself instead of resizing it to its client size; it also trims the restore rectangle by the standard frame winit added. On save, the shell stores the restore rectangle rather than eframe's maximized size, so *Restore down* returns to the size the window had before, not one taller than the work area. Checked with Win32 window rectangles at 1920 × 1080 (work area 1032 px high): maximized `(-8,-8)-(1928,1040)` after restart, restored 1440 × 860. |

## Alpha releases and updates (2026-11-14)

Getting builds to alpha testers and keeping them current without a reinstall.

| ID | Decision |
|---|---|
| D-84 | **hitsounds-rs updates itself from GitHub Releases, without blocking anything.** About 5 s after start, if the app setting *Check for updates when hitsounds-rs starts* is on (the default, `update_checks` in `settings.json`), a background thread asks the GitHub API for the repository's releases. It picks the newest published release above the running version. A prerelease counts only while the running version is one, so alpha testers move on to the stable release and a stable install never gets an alpha. It then downloads the asset for this copy: the setup for an installed copy (its `unins000.exe` is next to the exe) or the portable zip. The file goes to `updates/<version>/` in the config folder and is checked against GitHub's SHA-256 digest of the asset, else the release's `SHA256SUMS.txt`. Downloads only come from `https://github.com/<repo>/releases/download/`. A file that doesn't match is discarded. When the download is ready, a toast offers *Restart now* (the usual save prompt first), *What's new* (the release page) and *Later*. **The update is applied after the window closes**, never during editing. An installed copy runs the new setup with `/VERYSILENT /SUPPRESSMSGBOXES /CLOSEAPPLICATIONS`. A portable copy unpacks the flat zip next to the exe and renames each current file to `.old` before moving the new one in, putting them all back if any step fails; the `.old` files are removed at the next start. *Restart now* starts the app again afterwards. If another hitsounds-rs window is open, the update waits, since the installer would close it. Turning the setting off also drops a downloaded update. App settings shows the status and has *Check now*. The repository is set at build time (`HITSOUNDS_RS_UPDATE_REPO`, from the release workflow), so builds from source never update themselves. The request carries only a `hitsounds-rs/<version>` user agent; D-75 still stands, and no crash report or usage data is sent. Velopack was considered and not chosen, to keep a plain Inno Setup installer and an updater the project fully owns. |
| D-85 | **Releases are built by GitHub Actions and published by hand.** The source and the releases are in one public GitHub repository, so the updater needs no token. `.github/workflows/ci.yml` runs clippy (warnings are errors) and the tests on Windows for each push to `main` and each pull request. Pushing a tag `v<version>` that matches the workspace version runs `.github/workflows/release.yml`. It fetches ONNX Runtime (D-74), runs the tests, builds the release, and runs `tools/package.ps1`. That makes `hitsounds-rs-<version>-setup.exe` (Inno Setup 6, `installer/hitsounds-rs.iss`), `hitsounds-rs-<version>-portable.zip` (flat: exe, `.pdb` for D-75, ONNX Runtime and its licences, `LICENSE.txt`, `THIRD_PARTY_NOTICES.md`) and `SHA256SUMS.txt`. The workflow then creates a **draft** release with notes generated from the merged pull requests (`.github/release.yml`), marked prerelease when the version has a `-` suffix. A person reviews the notes and presses Publish. Only then do updaters see it. **The installer is per user** (`%LOCALAPPDATA%\Programs\hitsounds-rs`, `PrivilegesRequired=lowest`), so updates never show a UAC prompt. Its `AppId` must never change. Settings in the config folder survive uninstalling. **Alpha builds are not code signed**: testers see SmartScreen's *Windows protected your PC* once and choose *More info* > *Run anyway*. Files the updater downloads carry no mark of the web, so updates don't show it. Signing (SignPath or Azure Trusted Signing) can be added to the workflow later without changing the app. The first alpha is `0.1.0-alpha.1`. |

## Tester feedback (2026-11-14)

| ID | Decision |
|---|---|
| D-86 | **The tools are Select, Draw, Paint and Delete, on 1 to 4** (revises D-62's order, names and keys). The toolbar, the Tools menu, the palette and the gesture sheet list them in that order. *Erase* is renamed *Delete*; what each tool does is unchanged, and Draw stays the default tool. The letter keys P, B, D and E no longer pick tools. Testers found number keys easier to reach than FL's letters. Tap-to-record (§5.9) planned keys 1–9 for channels, so it will need another scheme if it is built. |
| D-87 | **A listening level can be typed** (refines D-77). Clicking the percentage under the Master, Music or Effects fader turns it into a text field with the old value selected. Enter or clicking away sets the typed value (0–100, a trailing `%` is allowed, out-of-range values are clamped), Esc keeps the old one. It is the same app setting the fader writes, so it makes no undo step. |
| D-88 | **Copy and paste of hits** (takes copy/paste out of D-53). Ctrl+C copies the selected hits, keeping each hit's channel and its distance from the earliest one; Ctrl+X copies and deletes them. Ctrl+V pastes with the earliest hit on the playhead, snapped as Enter places (D-47). On each channel the clipboard holds, the pasted span (first to last pasted time) **replaces** what was there: hits in it that are not part of the paste are removed. Other channels are not touched. The paste is one `Batch` edit, so one undo restores every replaced hit. The pasted hits become the selection, so a drag moves the whole paste. The clipboard lives in the app, outlives layer switches (so hits copy between the base layer and a difficulty, where the paste becomes overrides, D-17), and is cleared with a new project; channels deleted since the copy are skipped. Copy also puts a line of text on the Windows clipboard, because egui-winit sends no paste event for Ctrl+V when the system clipboard has no text. Edit menu and palette have Cut, Copy and Paste. |
| D-89 | **The Hit volume lane needs Ctrl to add a point** (revises the lane gestures of D-44). Testers changed the curve by accident with stray clicks. Now **Ctrl+click** adds a point (and Ctrl+drag places it), a **click on a point** switches the segment after it between ramp and step, **dragging a point** moves it, right-click deletes it, and a plain click on empty lane space does nothing. A click on the last point changes nothing, because its segment leads nowhere. |
| D-91 | **A lone addition plays over the difficulty's own hitnormal by default** (revises the default of D-31 and D-78). Under a whistle, finish or clap with no hitnormal channel, the always-playing hitnormal is now the skin hitnormal of the set the **difficulty's greenline** gives the object (5 ms ahead, as stable looks it up), or of `[General] SampleSet` when that greenline says 0. That is what the object played before any hitsounds were added, so a lone drum or normal clap on a soft map plays over `soft-hitnormal` and not the much louder `drum-hitnormal` or `normal-hitnormal`. In the first hands-on pass that jarred, and the setting that avoided it (D-78) was hidden in Project settings. The addition set stays the top addition's, and a real hitnormal channel still wins. Each difficulty uses its own greenlines, so two difficulties can resolve one moment differently, and the custom index pool takes both. The sets come from the difficulty as it is on disk, which keeps the mapper's sets since D-79. A difficulty that an earlier version exported before D-79 has that export's sets. The base layer has no greenlines of its own, so its preview borrows the sets of the project's first difficulty (the first enabled one, else the first loaded one). That keeps it in step with what export writes. With no difficulty loaded it falls back to the additions' set. The old default is kept as *Skin hitnormal of the additions' set* (`additions_set`). The new default is stored as `difficulty`. **The old stored value `skin_default` is read as the new default**, because projects saved it without anyone choosing it, so existing projects change their lone-addition sound and their packing on the next export. Change it back in Project settings to keep the Mapping Tools sound. |

## Resolved questions

| ID | Question | Answer |
|---|---|---|
| OQ-1 | Hitnormal at addition-only moments | D-31 |
| OQ-2 | Integer ms vs beat positions | D-32 |
| OQ-3 | Fallback on slider ends | D-33 |
| OQ-4 | One stem file or several | D-34 |
| OQ-5 | Slider tick sample set | **Verified in stable:** the tick uses the slider's object `normalSet` unless it is Auto, in which case it uses the greenline at `tick + 2 ms`. Index and volume always come from that greenline. So forcing the slider's set to Auto (as MT does) is required for custom ticks. |
| OQ-6 | Does the `filename` field work in standard? | **Verified in stable: no.** Only catch and mania gameplay load it (the editor plays it too, which is why it seems to work). So you were right for standard. D-08 stands. |

## Open questions

| ID | Question | Current default |
|---|---|---|
| OQ-7 | 5 % greenline volume is floored to **8 %** by stable (×0.8 for hitnormal), so "muted" ends are very quiet, not silent. Is that acceptable, or should muted ends point at an index with a 44-byte blank `hitnormal`? | 5 %, as in Hitsound Copier. A blank-index option is possible later |
