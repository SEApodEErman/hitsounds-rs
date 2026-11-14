# Design: the hitsounds-rs app

Status: **agreed** (D-48 to D-58, 2026-11-10), then **reworked as an FL Studio-style
workspace** (D-61 to D-65, 2026-11-12). Sections 4, 5.1, 5.2, 5.6, 5.8 and 10.2 describe
the workspace as built; where an older paragraph disagrees with §4, §4 wins. Section 5.9
is deferred (D-53). **Picking this up later? Start at §11.**

## 1. What the app is for

hitsounds-rs exists for five jobs. They aren't equally frequent, and the layout should
reflect that.

| Job | How often | Where it should live |
|---|---|---|
| 1. Set up (mapset, skin folder, stem, sample library) | Once per project or machine | Home screen, Settings, project popover |
| 2. Build channels (pick and audition samples, choose the osu! slot) | Bursts, early on | Channel rack, Channel settings, Browser |
| 3. **Hitsound the base layer** (listen, place, adjust, toggle the stem) | Most of the time | The timeline. This is the app |
| 4. **Review each difficulty** (find empty, fallback, orphaned and tick hits, fix them) | Once per diff, repeated after mapping changes | The timeline in a difficulty layer, plus an issue list |
| 5. Export (choose output, review changes, write) | A few times | A dedicated export view |

So the screen is built around job 3, job 4 is a *review loop* through problems, and
jobs 1, 2 and 5 are panels and views that get out of the way.

## 2. What made the first layout unintuitive

1. **Gestures are invisible.** Click to place, drag to box-select, right-click to delete,
   wheel to seek, Alt+wheel to zoom: all of it is only in tooltips.
2. **The difficulty view is a hidden mode.** Picking a difficulty from a dropdown
   silently turns every edit into a per-diff override. Nothing on the timeline says so.
3. **Problems can't be found, only counted.** The panel says "7 empty", but finding
   them means scrolling the whole song.
4. **Overrides are a form with raw millisecond fields**, although the timeline already
   knows the time and the channel you're pointing at.
5. **One panel, three unrelated jobs.** Channel, Samples and Difficulty share one tab
   strip, and the app switches tabs for you.
6. **The main menu hides behind the logo**, and Settings mixes app-wide things (skin,
   library) with project things (stem, default fallback).
7. **Export opens with nine technical options** and a "Build preview" button before you
   see anything.

## 3. Principles

- **The timeline is the app.** Everything else is a panel that can close.
- **Always show where an edit goes.** Base layer or a named difficulty, on screen, all
  the time.
- **Teach in place.** Show what a click will do before you click: a ghost hit under
  the cursor, hints in empty lanes, a gesture sheet one key away.
- **Keep osu! editor muscle memory** (D-42): wheel seeks by snap, Alt+wheel zooms,
  Ctrl+wheel changes the divisor, stable's snap colours.
- **Act on the thing.** Edits happen where you're looking (timeline, rail, context
  menus), not in forms.
- **Fields apply on change**, each change one undo step (kept from D-43).

## 4. Layout

FL Studio's arrangement, for hitsounding (D-61): menus and a hint bar on top, one
toolbar, a channel rack whose lanes are step pads, and docked panels.

```text
┌──────────────────────────────────────────────────────────────────────────────────────┐
│ ▮ File Edit View Tools Help │ Kick at 01:24.012  [Click] Place hit  [Drag] Select     My Song •  – □ × │ title bar
├──────────────────────────────────────────────────────────────────────────────────────┤
│ ▶ ■  01:23.456 034:2 │ Speed 1.00× Snap 1/4 │ Select Draw Paint Delete │ ↶ ↷     Browser Channel Review Mixer │ Export │ toolbar
├──────────────────────────────────────────────────────────────────────────────────────┤
│ Editing  [Base]  Easy ✓  Normal 2  Hard 7           Hits on the base layer play in every difficulty │ layer strip
├───────────┬───────────────────────────┬──────────────────────────────┬───────────────┤
│ Browser   │ Channels 8        ⌖ − + ⇔ │ 1 · · · 2 · · · 3 · · ·      │ Channel│Difficulty │
│ [search ] │ Listen to     [Song|Stem] │ waveform                      │  Sample        │
│ ▾ Drums   │ Hit volume                │ volume envelope               │  Hitsound in   │
│   kick    │ Objects (difficulty only) │ ✓ › – ✕ •                     │  osu! (3 × 4)  │
│   snare   ├───────────────────────────┼──────────────────────────────┤  Level         │
│ ▸ Claps   │ ⠿ ● S ◔ [Kick ] Drum Normal│ ▮▯▯▯ ▮▯▯▯ ▮▯▯▯ ▮▯▯▯        │  Appearance    │
│           │ ⠿ ● S ◔ [Snare] Drum Clap  │ ▯▯▯▯ ▮▯▯▯ ▯▯▯▯ ▮▯▯▯        │                │
│ [Load into│ ⠿ ○ S ◔ [Hat  ] Soft Whistle│ ▮▯▮▯ ▮▯▮▯ ▮▯▮▯ ▮▯▮▯        │                │
│  Kick]    │ + Add channel             │ ═══ song overview ═══         │                │
├───────────┴───────────────────────────┴──────────────────────────────┴───────────────┤
│ Mixer (F9):  Listening │ Hit volume │ 1 Kick ▮ │ 2 Snare ▮ │ 3 Hat ▮ │ …              │
├──────────────────────────────────────────────────────────────────────────────────────┤
│ Saved · ⚠ 2 warnings                         Kick at 01:24.012 · 3 selected · Draw · Snap 1/4 │ status bar
└──────────────────────────────────────────────────────────────────────────────────────┘
```

- **Title bar** (D-57): the app mark, the **menu bar**, the **hint bar** and the project
  name (with a dot while unsaved). The menus list every action by name with its shortcut:
  File (projects, export, Project settings, App settings), Edit (undo, selection, add or
  rename a channel), View (the four panels, zoom, follow, layer), Tools (the pointer
  tools, snap, Song or Stem, issue stepping, refresh, stem import), Help (gesture sheet,
  palette, warnings, About). The hint bar names the thing under the pointer and shows the
  gestures that act on it as keycaps; with nothing hovered it says what the current tool
  does. Everything between the menus and the window buttons drags the window.
- **Toolbar**: play and stop, the time display (time, then bar:beat), speed, snap, the
  tools (D-62), undo/redo, the panel switches and Export. Text labels drop away as the
  window narrows; the hint bar and the menus still name everything.
- **Layer strip**: `Base` plus one tab per difficulty with its open issue count (a check
  when clear, a warning when the file is missing, dimmed when left out of export). The
  right side says where edits go. In a difficulty the strip is tinted orchid, says
  *Changes here apply to Hard only*, and has next/previous issue buttons (D-50).
- **Channel rack and lanes**: see §5.1 and §5.2. The rack cells beside the top lanes
  label them and hold their controls: zoom, fit and follow beside the ruler, Song/Stem
  beside the waveform, *Use shared* beside the volume lane, the legend beside Objects. A
  horizontal scrollbar under the lanes shows where the view sits in the song.
- **Browser** (left, F8), **Channel settings** and **Difficulty review** (right, two tabs,
  F6 and F7), **Mixer** (bottom, F9). All docked, closable, and remembered between runs.
- **Status bar**: save state, warnings, hover readout, selection count, tool and snap.

## 5. Interactions

### 5.1 Placing and editing hits

- Each lane is a row of **step pads**, one per snap step, shaded alternately by beat.
  A hit is a lit pad in the channel's colour. A pad is centred on its tick, so the ruler
  tick above it, the waveform transient and the playhead crossing all line up, and a
  click anywhere on a pad means that tick (D-25). Hits on a finer snap than the current
  one are drawn at their own time. Once a step is narrower than 10 points the pads go
  away and hits are thin markers.
- Under the pointer, a **ghost pad** shows where a click will land. The hint bar says
  what click, drag and right-click do with the current tool.
- **Tools** (D-62, D-86): Select (1) never places; Draw (2) places on click, selects on
  a drag over empty space and moves hits on a drag; Paint (3) fills every snap step the
  stroke crosses on the row it began on; Delete (4) deletes what it is clicked or dragged
  across. Right-click deletes a hit with every tool (D-51).
- **Copy and paste** (D-88): Ctrl+C / Ctrl+X copy or cut the selection; Ctrl+V pastes it
  at the snapped playhead, replacing the pasted span on the channels it holds, as one
  undo step, and selects the pasted hits.
- Unchanged: Ctrl-click toggles selection, drag hits up/down to change channel, Enter
  places at the playhead, Delete removes the selection, Ctrl+A selects all (D-47), and
  the wheel follows the osu! editor (D-42, D-64).

### 5.2 Channel rack

One 30 px row per channel, left to right: reorder handle, **mute light**, solo, volume
**knob**, the coloured **name button**, and the osu! slot (`Drum Clap`).

- The mute light is lit while the channel is audible and flashes as its hits play. Click
  it to mute, right-click it to solo (FL's shortcut); the `S` button also solos. Both are
  monitoring only (D-14a).
- Drag the knob to set the channel volume, Shift for fine steps, double-click to reset.
  One drag is one undo step. The Mixer (F9) shows the same volume as a fader.
- Click the name button to select the channel, double-click to rename, right-click for
  its menu (settings, rename, preview, choose sample, move, per-difficulty mute and
  replace, delete).
- Click the slot for the 3 × 4 grid: sample sets down, sounds across. The same grid is
  always visible in Channel settings, with the file name it exports as.
- **Drag the handle to change priority**. The hint bar explains that higher rows win
  clashes (D-09).
- Drop a browser sample on a row to load it, or on *Add channel* to make a new one (D-45).

### 5.3 Difficulty layers: a visible mode

- Selecting a difficulty tab tints the layer strip, which then reads:
  **Editing Hard: changes apply to this difficulty only.**
- The **Objects** lane appears only in difficulty layers, coloured by projection state
  (matched, fallback, muted, empty, tick), with a legend in the lane's rail label.
- Diff-only hits are drawn with an outline, and base hits removed in this diff as
  hollow ghosts, so the difference from the base is visible on the timeline. The UI
  works this out from `overrides`; no core change is needed.

### 5.4 Review loop: the issue list (new)

The Difficulty review tab opens with a count of each projection state (matched, empty,
fallback, orphaned, muted ends, custom ticks), which doubles as the legend, then an
**issue list**: empty, fallback, orphaned and tick hits, filterable by kind, each with
its time and bar:beat. Click a row to jump there and
select the object. **N / Shift+N** step to the next or previous issue from anywhere,
so reviewing a diff becomes: press N, listen, fix, press N.

### 5.5 Overrides from the timeline (new)

The override form goes away; overrides are made where they apply:

| Override | Gesture |
|---|---|
| Add event | Place a hit in a difficulty layer (as now), or right-click an object → *Add Kick here* |
| Remove event | Delete a hit in a difficulty layer (as now) |
| Mute channel | Channel rail menu → *Mute in Hard* (whole diff, or the selected time range) |
| Replace channel | Channel rail menu → *Replace in Hard with…* (whole diff or range) |
| Slider body | Right-click a slider head → *Slider body…* |

The Difficulty review tab keeps the override list (*Changes from the base layer*) for review: each row can jump to its
time or be removed.

### 5.6 Library

The Browser (F8): search, folder tree, click to audition, Up/Down to audition the next
sample, then double-click or drag to load it (D-45). An action bar pinned to the bottom
names the selected file and offers *Load into Kick* (the selected channel) and *New
channel*.

### 5.7 Export view

Export becomes a full view instead of a dialog over the editor.

- **Left: options, grouped and explained.** *Where to write* as two choices with one
  line each (into the mapset, with backups; or hitsound-only copies). Then *Indices*
  (first custom index, first tick index), *Mixing* (grouping, silent hitnormal), and
  *Checks and cleanup* (auto-fix, schema growth, blank slides and ticks, take over
  existing samples).
- **Right: the preview, built automatically** when the view opens and rebuilt when an
  option changes. There's no Build button. Each difficulty row shows its counts and
  opens a **real line diff** of the `.osu` (changed lines only), not two full text
  dumps.
- Blocking problems sit above the preview. The write button names what it will do:
  *Back up and write 5 difficulties*.

### 5.8 Home, setup and help

- Home: *New project from mapset*, *Open project*, and recent projects (D-41). If the
  skin folder isn't set, the home screen asks for it first, since playback needs it.
- **App settings** are app-wide only (skin folder, sample folders, motion). **Project
  settings** (mapset, drum stem, fallback channel) is a dialog under File (D-61).
- **Gesture sheet** (`?` or F1): every mouse gesture and shortcut in one overlay,
  grouped by area.

### 5.9 Further features (deferred, D-53)

| Feature | Why |
|---|---|
| **Loop range**: drag on the ruler, play loops it | Hitsounders replay one section at slow speed all the time |
| ~~**Copy / paste hits** at the playhead~~ | Built (D-88) |
| **Tap to record**: while playing, keys 1–9 place a hit on channel 1–9, snapped | Lets you hitsound by playing along, like finger drumming |

Loop range needs a loop point in the audio engine to be sample-accurate. The other
two need only the UI.

## 6. Building it in egui

### What fits well

- The timeline is custom-drawn with egui's painter, and hit-testing is done by hand.
- The UI reads `Project` directly every frame. There's no mirror to keep in sync, and
  the playhead comes straight from the audio clock each frame.
- egui provides resizable side panels, context menus, modals, tooltips, drag-and-drop
  with typed payloads (library → channel), and drag-to-change number fields (good for
  ms, volume and leniency).
- `egui_kittest` (same release as egui) can drive the UI in tests (click, drag, type)
  and take snapshot images, so timeline gestures get regression tests.

### What needs care

- **Visual polish is on us.** A custom `Style`/`Visuals`, embedded fonts and an icon
  font (`egui-phosphor`, MIT). egui text has no OpenType features, so the timecode
  needs a font whose default digits are tabular.
- **Nothing may block a frame.** Opening a project, decoding the stem, building the
  waveform, export preview and write, and library scans run on a worker thread with a
  busy state.
- **Keyboard focus.** One input pass handles global shortcuts and skips them while a
  text field has focus.
- **Alt+Space** (D-46) is handled in the same Win32 window procedure as the custom
  title bar (§10.3).
- **Iteration speed**: no hot reload. Keep the UI crate small so rebuilds stay quick.

## 7. Architecture

```text
crates/
  osu-format/  hs-core/  hs-audio/  hs-export/  hs-separate/
  hs-session/   session, settings, library, mapset watch, playback
                (no GUI dependency, unit-testable)
  hitsounds-rs/    the eframe binary: UI state, views, timeline widget, theme
```

- The mapset watcher in `hs-session` reports changes through a callback, which
  `hitsounds-rs` wires to `request_repaint`.
- The session sits behind a mutex. The UI uses `try_lock`, so a long job shows the busy
  state instead of freezing the window. Lock order stays session, then settings.
- Rules: `hitsounds-rs` holds UI state only; behaviour lives in `hs-session` and the core
  crates.

## 8. Build order

1. **`hs-session`**: the editor session with no GUI dependency, tested on its own.
2. **Flagship shell** (D-55): theme tokens and motion, bundled fonts, icons, the custom
   title bar (D-57), home screen with a guided first run, create/open, settings, project
   popover, worker thread, toasts.
3. **Timeline**, then the visuals (ghost hit, diff-mode strip, override
   marks). Checklist, each with a kittest:
   - wheel seeks by snap, Shift ×4, one beat while playing; Alt zooms at the cursor;
     Ctrl changes the divisor; plain wheel over the rail scrolls (D-42)
   - snap grid and ruler in stable's colours, bar labels when there's room
   - click places, drag selects, Ctrl-click toggles, drag moves in time and across
     channels, right-click deletes, Enter, Delete, Ctrl+A, Escape (D-47)
   - volume lane: add, drag, right-click delete, Ctrl+click ramp/step, readout (D-44);
     since D-89, Ctrl+click adds and a click on a point switches ramp/step
   - objects lane: hit states, ticks, click to select and seek
   - follow playhead, zoom/fit, double-click rename, mute/solo, sample drop (D-45)
4. **Rail, inspector, library, difficulty panel** with the issue list and timeline
   overrides.
5. **Export view.**

## 9. Answered questions

| # | Question | Answer |
|---|---|---|
| R-1 | Which features from 5.9, and when? | None for now (D-53). Issue navigation and timeline overrides are part of the first version |
| R-2 | Right-click: osu!'s *delete*, or a context menu? | Right-click on a hit deletes it; on the rail, objects or empty lane space it opens a menu (D-51) |
| R-3 | Retire the override form in favour of timeline actions? | Yes; the list stays for review (D-51) |
| R-4 | Auto-build the export preview? | Yes, with a progress state (D-52) |
| R-5 | Dark theme only? | Dark only for now (D-49) |
| R-6 | Library on the left, inspector on the right? | Yes (D-49) |

## 10. Look, feel and polish

### 10.1 The bar (D-55)

hitsounds-rs should feel like a flagship product from a major tech company, built with
complete seriousness for an audience of exactly one hitsounder. "Basic" isn't a
milestone. Concretely:

- **One token system** in a single `theme` module: colour roles, type scale, spacing
  scale, radii, elevation, and motion (durations and easing). No colour or size is
  written ad hoc in a view.
- **Bundled typefaces**, chosen on screen (§11), with tabular digits for the timecode.
- **One icon set** (`egui-phosphor`), used the same way everywhere.
- **Every state is designed**: empty (an invitation to act), loading (progress, not a
  frozen window), error (what happened and how to fix it), success (a short toast).
- **Motion with a purpose**: panels slide, popovers and tabs animate, zoom eases, hits
  pulse as they play. *Reduce motion* turns it off, and it starts off if Windows
  animations are disabled.
- **Keyboard-first**: visible focus, every tooltip shows its shortcut, the gesture
  sheet (§5.8) covers everything.
- **A guided first run**: skin folder, sample library, first project, as three clear
  steps instead of an empty screen.
- **Performance is part of polish**: the timeline stays smooth at 144 Hz with a full
  song zoomed out and 30 channels. Measure frame time, don't guess.

### 10.2 Visual direction: quiet graphite, one accent (D-60, supersedes D-54)

D-54 set a neon-indigo "rhythm-game vivid" look. D-60 replaced it after an unslop
pass: the chrome is quiet so the timeline carries the colour.

| Role | Value | Use |
|---|---|---|
| Chrome / base / surface / raised | `#0F0F10` / `#141415` / `#19191B` / `#232325` | Title bar, window and timeline lanes, panels, controls |
| Hairline | `#303032` (`#46464A` on hover) | Borders and dividers |
| Text | `#EDEDED` / `#A1A1A6` / `#85858B` | Primary, secondary, muted |
| Accent | `#4DD6C8` teal | Primary action, selection, focus, the base layer, the timecode |
| Difficulty | `#DE78C8` orchid | Inside a difficulty layer only (D-50) |

No static glow or colour wash. The only glow is the hit flash as the playhead crosses
a hit, repeated on the channel's mute light. Labels are sentence case. Since D-63 the
workspace uses FL's density: controls are 26 px, channel rows 30 px, body text 13 px,
and radii nest at 5 (control), 8 (card), 10 (panel). Wells that hold a value (the time
display, segmented switches, the slot chip) are recessed in `#0A0A0B`; step pads are
`#222225` and `#2C292E` by beat. Headings are Inter Display Medium.

- **Colour carries meaning.** Teal means "you're working on the base layer or
  interacting", orchid means "you're inside a difficulty". The mode from §5.3 is visible
  from across the room.
- **Hits play visibly**: as the playhead crosses a hit it flashes in its channel colour
  and fades, so you can watch the pattern as well as hear it.
- **Projection-state colours** stay distinguishable from the accents and from each
  other, including for colour-blind users, because each is paired with a mark (see §11).
- **The snap grid keeps stable's colours** (D-42), drawn subdued.
- **Not osu!'s branding.** No osu! logo or circle motif, and the orchid stays a
  secondary accent that must not read as osu!'s brand pink. Check them side by side
  when building.

### 10.3 Window and platform (D-57, D-58)

- **Windows 10 and 11 only.**
- **The top bar is the title bar.** Empty areas drag the window, double-click maximises,
  and the app draws its own minimize/maximize/close. The maximize button answers
  `HTMAXBUTTON` hit-tests so Windows 11 snap layouts appear on hover. Resize edges, Aero
  snap and the system menu keep working. Alt+Space (D-46) is handled in the same window
  procedure. A window restored maximized fills the work area, and Restore returns to
  the size it had before (D-83).
- **Backdrop**: try Mica behind the chrome (for example with the `window-vibrancy`
  crate), falling back to the solid base colour on Windows 10 or if it costs frames.

### 10.4 Easter eggs (D-56)

The working UI stays plain and clear. The joke that this is a flagship product for one
person lives only in a few hidden places:

- **About** reads like a product launch: *Introducing hitsounds-rs. Engineered for an
  audience of one.*
- **Release codenames**, for example 0.2 *Market of One*.
- **A startup line**, one of a few, shown under the logo while the app loads.
- **A key combo** that does something harmless and silly (to be decided). It never
  touches the project.

Never in errors, warnings, empty states, the gesture sheet or the export review.

## 11. Where we left off (2026-11-13)

Current state:

- **Steps 1–5 are implemented, and the workspace was then reworked in FL Studio's
  manner** (D-61 to D-65): menu bar and hint bar, toolbar with the four pointer tools,
  channel rack with step pads, docked Browser / Channel settings / Difficulty review /
  Mixer on F8 / F6 / F7 / F9. The app lives in `crates/hitsounds-rs`. Run it with
  `cargo run -p hitsounds-rs` on Windows. `cargo test --workspace` and clippy pass.
  The frame-budget test (release, 180 s song, 30 channels, 43,200 hits, playing)
  measured p95 1.7 ms CPU fitted to the song and 1.6 ms at the step-pad zoom.
- **Where things are**: `widgets.rs` (hint bar, light, knob, fader, segmented switch,
  form rows), `views.rs` (title bar, menus, status bar, dialogs, home), `editor/mod.rs`
  (toolbar, layer strip, docks, `Act`), `editor/canvas.rs` (tools and gestures),
  `editor/paint.rs` (pads and hits), `editor/rail.rs` (rack rows), `editor/inspector.rs`
  (the two right-hand tabs), `editor/mixer.rs`, `editor/library.rs` (Browser).
- **Checked on screen** with the synthetic smoke project at 1440 × 860 and 960 × 600:
  home, base layer, a difficulty layer, every menu, the slot popup, the channel menu,
  Project and App settings, the gesture sheet, the mixer, Export, and a paint stroke.
  **Not checked**: playback visuals (hit flash, mute-light flash), since the QA pass ran
  without audio, a real mapset, Windows 10, and the visible Snap Layouts flyout.
- **Next:** a hands-on pass over the editor in a real mapset.
- **2026-11-14, after the first hands-on pass:** empty objects keep the custom index
  before them when it changes nothing (D-81); mapsets are picked by name from the osu!
  Songs folder, new projects can go to a projects folder without asking, and a moved
  mapset is found again (D-82, `mapsets.rs`); a maximized window starts maximized (D-83).
  The picker was checked on screen with a synthetic Songs folder (create, keyboard,
  moved mapset) and against the developer's real install (8,261 mapsets, found through
  the registry).
- **2026-11-14, tester feedback:** tools reordered to Select, Draw, Paint, Delete on
  1–4 (D-86); listening levels take a typed value (D-87); copy, cut and paste of hits at
  the playhead (D-88, `Timeline::paste` in `editor/canvas.rs`); the Hit volume lane adds
  points only with Ctrl+click, and a click on a point switches ramp and step (D-89).
  Covered by editor tests; **not yet checked on screen**.
- **Debug-only QA hooks** in `app.rs` and `main.rs`: `HITSOUNDS_RS_QA_INPUT` (a file with
  one pointer, key or wheel event per write), `HITSOUNDS_RS_QA_CAPTURE` (a file naming the
  PNG to write) and `HITSOUNDS_RS_QA_SIZE` (`960x600`). They are compiled out of release
  builds.

The shell uses Inter Display Medium for headings (D-60, replacing Space Grotesk
Bold). Inter Regular handles controls and body text. IBM Plex Mono Medium gives the
timecode fixed-width digits. The app mark is a teal pulse pattern on a graphite tile,
and orchid marks difficulty context. The shell uses a solid graphite backdrop. The maximize button returns `HTMAXBUTTON` for the Windows snap menu.
[Microsoft's snap layout guidance](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-snap-layout-menu)

The projection colours chosen for step 3 pair each colour with a mark. Matched
uses lime `#B9E776` with a check, fallback amber `#FFC66D` with a chevron, muted
slate `#A4A1B8` with a dash, empty coral `#FF806D` with a cross, and ticks
periwinkle `#9DAEFF` with a dot. Check the marks at timeline scale when implementing
the Objects lane.

F1 and the menu bar cover every action. Ctrl+K opens a command palette
(D-59, `crates/hitsounds-rs/src/palette.rs`): type to filter, Up/Down to choose, Enter to
run. It offers play and seek, layer and channel switching, issue stepping, the tools,
zoom, export, save, undo and redo, panels, stem and mapset actions, and the dialogs.
The hidden encore is Ctrl+Alt+Shift+H, which opens a variant of About.

Windows 11 smoke checks covered first-run setup, project creation, format-1
opening, Alt+Space, stem import, unsaved-change handling, live mapset refresh,
and the title bar at default and compact sizes. Windows 10 and the visible Snap
Layouts flyout still need hands-on verification.
