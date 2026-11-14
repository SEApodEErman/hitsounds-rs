# Findings: how osu!stable hitsounds actually work

This is the ground truth hitsounds-rs's model is built on. Each fact has a source:
**[wiki]** is the osu! wiki, **[MT]** is the Mapping Tools source (commit
`1fa4171`, 2026-11-07), **[RC]** is the Ranking Criteria, and **[stable]** is osu!stable's
own playback behaviour. See [references.md](references.md). Anything marked **unverified** still needs an
in-game test before code relies on it.

---

## 1. The sample model

| Concept | Values | Notes |
|---|---|---|
| Sample set | Normal (1), Soft (2), Drum (3), Auto (0) | Auto = inherit. **[wiki]** |
| Hitsound | hitnormal + additions whistle / finish / clap | Bits 0/1/2/3 of `hitSound`. **[wiki]** |
| Custom index | integer | Selects `<set>-hit<sound><index>.wav` in the mapset. **[wiki]** |
| Volume | 5–100 % | Per greenline, or per object for circles. **[wiki]** |

- **The hitnormal always plays** in osu!standard and catch. This comes from the skin's
  `LayeredHitSounds`, which is on by default. Bit 0 only matters in mania. **[wiki]**
  MT encodes this as `normal = mode != Mania || Normal || !(additions)`. **[MT `TimelineObject`]**
- Each object has **one normal set and one addition set**. All three additions
  share the addition set and the index. **[wiki]**
- The resolution order is **[wiki][MT `GiveHitsoundTimingPoint`]**:
  - `normalSet` 0 takes the timing point's set, and if that is 0 too, `[General] SampleSet`.
  - `additionSet` 0 takes the **resolved normal set**.
  - `index` 0 takes the timing point's index. `volume` 0 takes the timing point's volume.
- **The hitsound greenline is looked up at `object time + 5 ms`.** A greenline placed
  up to 5 ms after an object still controls it. This holds for circles, slider
  heads, repeats and tails. **[MT `Timeline.GiveTimingPoints`][stable]**
- **Slider ticks look up their greenline at `tick time + 2 ms`.** **[stable]**
- **Playback gain** **[stable]**: the volume is floored at
  **8 %**, then the hitnormal plays at ×0.8, whistle and clap at ×0.85, and finish at
  ×1.0. Preview applies this (`resolve::Playing::gain`). Merging never moves a sound
  between types, so mixes keep the right factor. One consequence: a 5 % "muted"
  greenline plays at about 6.4 %, which is not silent (OQ-7).

## 2. Filenames and lookup

- The file is `<set>-hit<sound><index>.wav|.ogg`. The number is **omitted for index 0 and 1**.
  So `soft-hitclap.wav` is the index-1 file. **[wiki][MT `GetFileName`]**
- The lookup order is **[wiki]**:
  1. The beatmap folder, but only when the index is not 0.
  2. The skin, with the index removed.
  3. The osu! default resources.
- **Extension order for indexed hitsounds is `.wav`, `.mp3`, `.ogg`** **[stable]**. So a leftover
  `soft-hitclap2.wav` shadows a new `soft-hitclap2.ogg`; export removes its own file of
  the same stem when it changes format (D-69). `filename` samples use a different order
  (§10).
- **Files under 1024 bytes load as silence** **[stable]**, for both indexed and `filename` samples. The 44-byte blank relies on this.
  Export warns about any other file that small.
- **A missing indexed file falls back to the skin sound.** hitsounds-rs relies on this.
  A custom index only needs files for the slots it actually customises. A slot whose
  content is "the skin's own sound" needs no file. (`CustomIndex::needs_file`)
- Slider body and tick samples follow the same scheme: `<set>-sliderslide<index>`,
  `-sliderwhistle`, `-slidertick`. **[MT `HitsoundFilename`]**

## 3. What each object type can carry

| Edge | Hitsound bits | Normal/addition set | Per-object index/volume | Filename |
|---|---|---|---|---|
| Circle | ✔ | ✔ (`hitSample`) | ✔ | catch/mania only |
| Slider head / repeat / tail | ✔ (`edgeSounds`) | ✔ (`edgeSets`) | ✘, from the greenline | ✘ |
| Spinner end | ✔ | ✔ | ✔ | ✘ |
| Slider tick | ✘ | slider `normalSet`, or the greenline if Auto | ✘, from the greenline at +2 ms | ✘ |

Sources: **[wiki][MT `CanCustoms => IsCircle || IsHoldnoteHead`][stable]**

**Consequence.** Slider edges can't carry their own index or volume, so the
index/volume of *every* hit must come from a greenline at that time. This is how
Mapping Tools exports: one greenline per hitsound event. **[MT `BuildStandardTimingPoints`]**
hitsounds-rs does the same.

### The `filename` field

The wiki says that when `filename` is set, "no addition sounds play and that file
plays instead". MT applies this to circles and hold heads in every mode. **Verified
in stable:** only **catch and mania** load the file. In osu!standard
gameplay it is ignored. The editor loads it in compose mode, which is why it seems
to work while editing. We never write it (D-08).

### Slider ticks and bodies

- MT's Hitsound Copier has an option, `CopyToSliderTicks`, for hitsounds that land on
  a slider tick in the target diff. It works like this **[MT `HitsoundCopierEngine`]**:
  1. Render the source moment's sounds into one custom `slidertick` sample at a new
     index. Indices start at **100**.
  2. Set the slider's object-level sample set to **Auto**, so greenlines control it.
  3. Place a greenline at the tick with the new index and sample set.
  4. Place a **revert greenline 5 ms later**.
- `CopyToSliderSlides` does the same for `sliderslide`, without the revert.
- **Verified in stable**: the tick sample set is
  the slider's object `normalSet` when it isn't Auto; otherwise it's the greenline
  at `tick + 2 ms`. The index and volume always come from that greenline. hitsounds-rs
  normally sets the slider to Auto for a custom tick. If a body override sets a
  concrete sample set, hitsounds-rs renders the tick file under that set too.
  The revert at `tick + 5 ms` sits after the +2 ms lookup.
- **Tick placement** (stable, verified, implemented in `DiffTimeline::from_document`):
  - Tick spacing is `100 * SliderMultiplier * sv / SliderTickRate` px.
  - A tick is dropped if it lies within 10 ms of travel (`0.01 * velocity`) of the
    span's end.
  - On repeat spans, ticks mirror the previous span. But if the last tick of a span
    was dropped, every span restarts its ticks from its own start.
- **Slider body** (`sliderslide`/`sliderwhistle`): in v14 maps it uses the slider's
  object `normalSet`/`additionSet`. The custom index follows the **live** active
  greenline (looked up at `now + 5 ms` while playing), so a greenline change
  mid-slider changes the slide sample.
- A greenline placed at a slider head also governs that slider's ticks and body,
  unless another greenline changes it.

## 4. Timing point merge semantics (Mapping Tools `TimingPointChange`)

This is the non-destructive greenline merge. Without it, writing hitsounds would wreck SV.

1. A change says *which fields* it sets: sample set, index, volume, or kiai. All other
   fields are left alone.
2. Existing points within **±2 ms** are edited in place. Redline changes use 0.4 ms.
3. Otherwise a new greenline is inserted as a **copy of the previous point**, so SV is
   kept. A copy of a redline becomes a 1.0× greenline (`-100`).
4. The new greenline is **skipped if it would have the same effect** as the previous point.
5. The optional `allAfter` setting propagates the fields to every later point. This is
   Hitsound Copier's "overwrite everything" mode.

The port is `osu_format::TimingPointChange`, and it has tests.

## 5. Mapping Tools Hitsound Studio pipeline

- **Layers.** Each layer has `(name, sampleSet, hitsound, priority, sampleArgs, times[])`.
  Priority is the order in the list.
- **ZipLayers.** Events from all layers within **15 ms** of each other become one
  `SamplePackage`. Packages without a hitnormal get a default normal sample:
  sample set Auto, lowest priority.
- **Hierarchy** (`SamplePackage.GetSampleSet` / `GetAdditions` / `GetCustomIndex`):
  - The normal set is the set of the **highest-priority hitnormal** sample. If there
    are only additions, it's taken from the additions.
  - The addition set is the set of the **highest-priority addition**. If there are no
    additions, it's taken from the normals.
  - **All** hitnormal sources are mixed into `<normalSet>-hitnormal`. All whistles go
    into `<additionSet>-hitwhistle`, and the same for finish and clap.

  This is the rule you described: a lower layer gets mixed into the sample set of the
  layer above it.
- **hitsounds-rs departs from this at addition-only moments** (D-91): by default the
  hitnormal's set is the one the difficulty's greenline gives the object (`[General]
  SampleSet` when the greenline says 0), so a lone drum clap on a soft map plays over
  `soft-hitnormal`. This is the sound an Auto normal set gives in stable (§1), and it
  is what the object played before it got any hitsounds. The Mapping Tools rule above
  is still an option.
- **BalanceVolumes.** The loudest sample sets the greenline volume. Quieter samples
  get baked into the mix at their relative amplitude, with roughness quantisation to
  limit the number of distinct mixes.
- **CustomIndex.** There are 12 slots (3 sets × 4 sounds), and each slot holds a set of
  sources.
  - `Fits`: every non-empty slot of the requirement equals ours.
  - `CanMerge`: no conflicting non-empty slots.
  - `Optimize`: greedy merge, then drop indices nothing needs exclusively.
  - `GiveIndices`: numbers start at `FirstCustomIndex`, default 1.
  - `MatchPackages`: greedy, picking the index that fits the longest run of upcoming
    packages, so there are fewer greenline changes.
- **SampleSchema.** The index→sources assignment is saved so a later export can reuse
  the same numbers. "Allow growth" adds new ones.
- **Export modes.** Standard (custom indices), Coinciding/Storyboard (named files), and MIDI.
- **Hitsound Preview Helper.** This is a mania map where lane = zone = (sample set,
  hitsound, filename/index). The Preview Helper writes `Filename` per object so the
  mania editor previews each lane. Lanes are capped at **18**, the mania key limit
  (`GenerateManiaPositions` clamps to 18). hitsounds-rs channels don't have this limit.

## 6. Mapping Tools Hitsound Copier

- Each source hit is matched to the **nearest** target timeline object within the
  **temporal leniency (default 5 ms)**.
- It copies the sample set, addition set and hitsound bits. Circles also get the
  per-object index and volume. Greenline changes carry the index and volume for
  everything.
- **Hold logic.** Target objects with no source hit keep the sound they currently
  produce. A greenline is added at them so the copied greenlines don't change them.
- Other options: copy slider body sounds, copy storyboard samples (off by default),
  mute slider ends by snap divisor (sample set Auto, 5 % volume), and always preserve
  5 % volume.

hitsounds-rs's projection is "Hitsound Copier from a virtual hitsound diff". The base
layer plays the role of the source map. Overrides change the source per diff before
copying.

## 7. Ranking Criteria rules that constrain export

| Rule | Where it lands |
|---|---|
| No unused or 0-byte files in the mapset folder | Project, backups and stems live **outside** the mapset (D-27). Export removes stale files hitsounds-rs created. |
| Clicked hitsounds: clear impact, **peak ≤ 5 ms** late | `hs_export::checks::LatePeak` |
| Active hitsounds must be **WAV or OGG**, not MP3 | Export writes 16-bit WAV by default, or float WAV or Ogg Vorbis (D-69). Sources are copied unchanged only when they are WAV or OGG. |
| Every hitsound file **≥ 25 ms** | `checks::TooShort` |
| Silent files must be the **44-byte** blank WAV | `wav::blank_wav()` |
| Storyboarded hitsounds can't replace active ones | Storyboard fallback is off by default and opt-in later (D-06). |
| Hitsounds must be audible | Warn on very low effective volume (planned). |
| Avoid replacing soft/normal finish with frequent custom samples (guideline) | Possible future lint. |

## 8. What this means for hitsounds-rs

1. **Greenline per hit** for index and volume, merged with `TimingPointChange`, as
   Mapping Tools does.
2. **One custom-index schema for the whole mapset.** All diffs share the sample files,
   and the saved schema keeps numbering stable between exports, so re-exporting
   doesn't churn the files.
3. **Skin-default slots need no files.** Fewer files, and default sounds stay default.
4. **Slider-tick hitsounds** use MT's tick technique, opt-in per diff (`TickPolicy::Render`).
5. **Default samples come from a skin folder you provide** (D-30). They are needed for
   preview and for mixing a default sound into a custom slot. The folder is an app-wide
   setting (D-40).
6. **Empty objects keep the custom index before them when it has no file for their
   hitnormal** (D-81). This follows from §2's fallback: `soft-hitnormal7` missing means
   index 7 plays the skin's `soft-hitnormal`, the same as index 0. It relies on the same
   fallback the skin-default slots of point 3 do; **unverified in game** for this
   use specifically.

## 9. Editor conventions hitsounds-rs mirrors (UI only)

(See also §1: greenline volume is what the D-44 volume envelope scales.)

These don't affect what gets exported. They keep a stable mapper's muscle memory and
visual memory working (D-42).

- **Beat-snap tick colours**. With `r = tickIndex mod
  divisor`, counted from the redline offset: `r = 0` white (255,255,255); else the first
  of `2r`, `3r`, `4r`, `6r`, `8r` that is a multiple of the divisor gives red (255,0,0),
  violet (200,0,200), blue (50,128,255), magenta (144,64,144) or yellow (255,255,0);
  otherwise grey (160,160,160). So 1/4 reads white, blue, red, blue; 1/3 white, violet,
  violet; 1/12 and 1/16 have grey ticks.
- **Tick heights**: a beat tick is one tick height and a bar line twice that. Ticks between
  beats are 7/8 of a beat tick.
- **Mouse wheel**: wheel up seeks
  back and wheel down seeks forward, one snap tick per notch, or four with Shift. While
  playing, the step is a beat rather than the divisor. Alt+wheel over the timeline zooms
  (wheel up zooms in). In compose mode Ctrl+wheel doubles or halves the divisor.
  **Unverified** in game: the Ctrl+wheel divisor step.
- **Playback speed keeps the pitch** (D-90). Below 1x the editor time-stretches the
  song, so it plays slower at its own pitch; the option *Don't stretch audio when
  changing speed* switches to tape-style varispeed (slower and lower, which the option
  describes as making timing more precise). The stretch is grain-based (WSOLA, about
  30 ms grains with 4 ms crossfades), so slowed transients can sound doubled.
  **Unverified** in game: that keeping the pitch is the default.

## 10. Mania hitsound layer difficulties (importer, D-66)

How osu!stable plays a mania note, which is what `hs_core::import` reproduces when it
turns a Hitsound Preview Helper style difficulty into channels. Verified unless marked.

- **A `filename` that loads plays alone.** The note plays that one sample instead of
  its hitsound bits, at the resolved volume. If the file can't be loaded, the
  note falls back to its bits like any other note.
- **Filename lookup**: a name whose first dot comes
  after a character is tried as written; otherwise, or when that file is missing, the
  name up to its last dot is tried as `.wav`, `.ogg` and `.mp3`. The Preview Helper
  writes names without an extension (`kick`). Windows makes the lookup case-insensitive.
- **No layered hitnormal in mania.** The hitnormal is added only
  when no bit is set, or when the skin's `LayeredHitSounds` is on and the mode isn't
  mania. So in mania a note plays its own bits: the normal bit for the hitnormal, the
  addition bits for the additions.
- **Volume**: the note's own volume when it isn't 0, else the timing point's. Index, sets and the `+5 ms` greenline lookup work
  as in §1.
- Files under 1024 bytes load as an empty (silent) sample.
- **Unverified:** that indexed files above 2 also accept `.mp3` (the importer only looks
  for `.wav` and `.ogg`, as §2 says), and whether hold-note tails play a sound (the
  importer reads holds as their head only).

- **Ogg Vorbis samples.** osu! loads them through BASS like WAV. libvorbis output decodes
  to exactly the input length with the onset in place (tested with symphonia's gapless
  decoder in `hs_export::encode`). **Unverified in game:** that BASS decodes them with
  the same alignment, and that 32-bit float WAVs are converted to 16-bit on load (BASS
  documents this for `BASS_SampleLoad` without `BASS_SAMPLE_FLOAT`, which stable doesn't
  pass).

**What the importer keeps.** Each distinct sound (slot, mapset file or skin, note
volume) becomes one channel, in column order. Greenline volume becomes step points of
the master envelope (D-44), moved back to the first note that already hears them
through the `+5 ms` lookup. A note's own volume becomes its channel's volume, which the
envelope then scales. That matches what the layer map *means*: the Preview Helper sets
every note to 100 and puts the intended hit volume on greenlines. In the mania preview
itself those notes ignore the greenlines.

Exported through a standard slot, a filename sample that played at ×1.0 in mania gets
that slot's playback gain (×0.8 for hitnormal, §1), so it sounds slightly quieter than
it did in the mania preview.

## 11. MP3 song timing

osu!stable sets BASS config option 68 to 1.
The installed BASS 2.4.15.2 decoder, tested with that setting, removed the 1105
leading frames and the trailing padding signalled by the affected song's LAME
tag. A synthetic tagged MP3 matched the same rule. A synthetic MP3 without the
tag kept its full frame count with option 68 enabled. hitsounds-rs uses Symphonia's
packet trim metadata to follow this timing in both Song and Stem monitoring
(D-26, D-73). This was verified against decoded PCM. In-game playback was
unverified.

For the affected song, 829 identifiable drum transients matched the original
sample for sample. The first full-mix attack loses much of its level during
HT-Demucs separation. Its first 5%-of-full-scale peak was about 35 ms later in
the model output, at the same sample as in the encoded Ogg.

## 12. The Songs folder and `osu!.db` (D-82)

- **The Songs folder** is `Songs` in the install folder unless the user's
  `osu!.<name>.cfg` sets `BeatmapDirectory` (relative to the install folder or
  absolute). `osu!.cfg` without a name holds machine settings, not this. **[stable]**
- **The installer registers** `HKEY_CLASSES_ROOT\osustable.File.osz\shell\open\command`
  as `"<install>\osu!.exe" "%1"`, which finds an install outside `%LOCALAPPDATA%`.
  Seen on the developer's machine. **[stable]**
- **`osu!.db`** in the install folder lists every imported difficulty with artist, title
  (both also in Unicode), creator, difficulty name, `.osu` file name, mode, online set
  ID, source, tags and its folder under Songs, in the layout on the osu! wiki ("osu!.db").
  Star ratings are `0x08, Int, 0x0d, Double` pairs; the reader also accepts `0x0c,
  Single`. Verified on a real `osu!.db` (35 MB, 32,841 difficulties): a sample of 207
  mapsets matched their `.osu` headers in every field except the set ID, where the
  database has the online ID and some files have a different or no `BeatmapSetID`.
- **One folder level of nesting**: a folder under Songs with no `.osu` files of its own
  is read one level further down, as packs unpack. **unverified** whether stable goes
  deeper.
