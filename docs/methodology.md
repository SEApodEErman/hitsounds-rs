# Methodology: the hitsounds-rs workflow

hitsounds-rs takes the mania "Hitsound Preview Helper + Hitsound Studio" workflow and
removes the friction points:

| Friction today | hitsounds-rs answer |
|---|---|
| Hard to tell which merged sample/index plays at a timestamp | You don't need to: preview simulates osu! exactly, and missing or wrong hitsounds are fixed in the per-diff view, where the problem shows up |
| Diffs with different rhythms need manual fixes after copying | **Projection** + **per-diff overrides** + **fallback** |
| Juggling `AudioFilename` overrides in a stems `.osu` | **Original ↔ Stem** monitor toggle (one stem file, usually drums). The mapset audio is never touched |
| An 18-lane limit, and sharing a lane between two samples over time | **Unlimited channels**, each with its own sample |
| Manual renaming and index bookkeeping | Automatic packing, index allocation and file naming, following Mapping Tools rules |

## Glossary

- **Channel**: a DAW-style track with a fixed osu! slot (sample set + hitsound, e.g.
  `drum-hitclap`) and a sample (the skin default or a custom file). It also has volume,
  mute/solo (monitoring only) and a colour. Channel order is **priority**.
- **Event**: a moment where a channel sounds, placed on the beat-snap grid like objects
  in the osu! editor.
- **Base layer**: all channel events. It describes the song's full hitsound score,
  independent of any diff.
- **Difficulty profile**: one target `.osu`, plus its overrides, fallback, match
  leniency and tick policy.
- **Edge**: a clickable moment in a diff (circle, slider head/repeat/tail, spinner end).
- **Projection**: base layer + diff overrides, intersected with the diff's edges.
- **Fallback**: the channel used for edges that no event lands on (e.g. "this project
  defaults to soft-hitnormal").
- **Package**: everything that sounds at one edge.
- **Custom index**: a numbered set of up to 12 sample files (3 sets × 4 sounds),
  shared across the mapset.

## The workflow

### 1. Set up the project

1. Create a project. It **lives outside the mapset folder** (D-27) and points at:
   - the mapset folder (`osu!/Songs/<set>`),
   - a skin folder for default samples (D-30),
   - your stem, if you have one: a single file, usually the drum stem. hitsounds-rs can
     separate it from the song itself (Project settings, *Separate from song*, D-70),
     using a Demucs model file you make once and choose in App settings (D-71). Or
     import one from your separation site, with the same length and offset as the
     song.
2. hitsounds-rs reads every `.osu` in the mapset and offers a profile for each. It uses
   the first diff's red lines as the grid. All diffs of a mapset share timing; a
   mismatch is a warning.

### 2. Build channels

- Add as many channels as the song needs, for example: `Kick (acoustic)`,
  `Kick (hardstyle)`, `Snare`, `Hat`, `Crash`, `Fallback soft-hitnormal`.
- Each channel picks a slot (`drum-hitnormal`, `soft-hitclap`, …) and a sample: the skin
  default or an imported file from the sample browser.
- Two kicks used in different song sections are just **two channels**. No regions
  needed (D-13).
- **Order the channels by priority.** When two channels with the same sound type but
  different sets meet at one moment, the lower one is mixed into the set of the upper
  one (see *Hierarchy* below).

### 3. Hitsound the base layer

- Place events on the beat-snap divisor, as in the osu! editor.
- Toggle **Original ↔ Stem** while working to hear one layer clearly. Switching keeps
  the playback position.
- Preview plays exactly what osu! would play (D-26). That includes the always-on
  hitnormal, set and index inheritance, and skin fallback.

### 4. Specialise per difficulty

Select a difficulty. The timeline now shows that diff's hit objects above the
channel lanes, and the projection result on them:

| State | Meaning | Typical action |
|---|---|---|
| Matched | One or more base events land on this edge | none |
| Muted | An empty slider/spinner end selected by the diff's mute filter | Adjust the filter, or add an event to un-mute it |
| Fallback | No event landed (and not muted); the fallback channel plays | Accept, or add a diff-only event |
| Empty | No event landed and there is no fallback or mute. Export resets the edge to a default hit. | Add an event or choose a fallback |
| Orphaned | A base event has no object in this diff | Usually fine (the diff is simpler); or move it with an override |
| Tick | The event lands on a slider tick | Enable tick rendering for this diff, or ignore |

**Overrides** apply only to the selected diff, on top of the base. Each diff
overrides only the base, with no inheritance between diffs (D-17).

- **Add event**: e.g. kicks on an Extra-only triple, or select an object and give it
  a sound.
- **Remove event**: drop a base hit in this diff.
- **Mute channel** (optionally over a range): e.g. no hats in Normal.
- **Replace channel** (optionally over a range): e.g. Hard plays a soft-hitwhistle
  where the base plays the kick.
- **Slider body** changes the `sliderslide` sample set and optional
  `sliderwhistle` on a v14 slider.

**Mute slider ends** (per diff, like Hitsound Copier). A dialog asks which ends to
mute:
- the divisors to consider, in priority order (default 1/1, 1/4, 1/3, 1/8, 1/6, 1/16, 1/12),
- which of them mute (default everything except 1/1),
- the minimum slider length in beats (default ½),
- whether spinner ends are included,
- the muted sample set, index and volume (default Auto, keep index, 5 %).

Each empty end is classified by the first divisor it lies on. Only ends with **no
event** are muted, and repeating sliders never are. Note that stable floors hit
volume at 8 %, so muted ends are very quiet rather than silent (OQ-7).

### 5. Export

- **Direct (default)**: write hitsounds into each enabled diff, after an automatic
  backup into the project folder (D-19).
- **Hitsound-only `.osu`**: one file per diff, for anyone who prefers copying by hand.
- **Base hitsound difficulty** (D-95): one extra difficulty with a circle on every
  base-layer hit, to copy into every diff with Hitsound Copier, as hitsounding was done
  before. It can be written alone or with either mode above.
- Export renders the **mapset-wide** sample files, runs the ranking checks (≥ 25 ms,
  peak ≤ 5 ms, 44-byte blank for silence), writes the files, verifies them, and
  removes stale files that hitsounds-rs itself wrote earlier.
- **Live sync** (D-20): while you keep mapping in osu!, hitsounds-rs watches the diffs.
  When a diff changes it re-projects, so the warnings stay current. It never writes
  unless you export.

## Hierarchy (clash resolution)

This follows Mapping Tools exactly (findings §5). For every moment:

1. **Normal set** = the set of the highest channel playing a hitnormal. Every
   hitnormal channel at that moment is mixed into `<normalSet>-hitnormal<idx>`.
2. **Addition set** = the set of the highest channel playing an addition. Every
   whistle is mixed into `<additionSet>-hitwhistle<idx>`, and likewise finish/clap.
3. **Addition-only moments**: the hitnormal still plays. It is the skin hitnormal of
   the addition's set, added **once** however many additions coincide (D-31).
   "Silent" is the other option.
4. **Volume**: the loudest channel sets the greenline volume. Others are baked into
   the mix at their relative gain, quantised to 5 % steps.

Example: `normal-hitnormal (Kick)` sits above `soft-hitnormal (Pad)` and both hit at
01:23.456. They become one file, `normal-hitnormal<idx>.wav` = Kick + Pad. The object
gets normal set = Normal.

Example: `drum-hitclap (Snare)` sits above `soft-hitwhistle (Hat)`. The addition set
becomes Drum, so the Hat sample is written into `drum-hitwhistle<idx>.wav`.

## Indices

- Custom indices are **one pool for the whole mapset**: every diff's requirements go
  into one schema. Compatible requirements share an index (greedy merge), and each
  diff then picks indices that keep long runs (fewer greenlines).
- The schema is saved in the project. Re-exports keep existing numbers and only add
  new ones (stable files, small diffs for modders).
- Index 0 is never written; it means skin default. Numbering starts at 1 by default.
  Generated slider tick/slide samples start at 100.
- You can still force defaults: a channel whose sample is the skin default produces no
  file, so that slot falls through to the skin.
