//! Packing: turn "these channels sound at this moment" into osu! object fields plus
//! a mapset-wide set of custom indices whose sample files reproduce the mix.
//!
//! Ported from Mapping Tools (`SamplePackage.cs`, `CustomIndex.cs`,
//! `HitsoundStudioEngine.cs`); see THIRD_PARTY_NOTICES.md. Rules:
//!
//! * **Hierarchy (D-09).** Channel order is priority. The normal set at a moment is
//!   the set of the highest-priority hitnormal channel; every hitnormal channel at
//!   that moment is mixed into `<normalSet>-hitnormal<idx>`. The addition set is the
//!   set of the highest-priority addition channel; all whistles are mixed into
//!   `<additionSet>-hitwhistle<idx>`, and likewise for finish/clap.
//! * **Custom index.** 12 slots (3 sets x 4 sounds), each holding the set of sources
//!   that must be mixed into that file. Indices are merged greedily when compatible.
//! * **Volume.** The loudest sample at a moment sets the greenline volume; quieter
//!   samples are baked into the mix at their relative gain.

use crate::project::AdditionOnlyNormal;
use crate::projection::{HitKind, Projection, Target};
use crate::{Ms, Project, SampleSource};
use osu_format::sample::{sample_file_stem, slider_file_stem};
use osu_format::{Hitsound, SampleSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Audio identity of something that can be mixed into a sample file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// osu! default sound from the user's skin folder.
    Skin { set: SampleSet, sound: Hitsound },
    SkinSlider { set: SampleSet, sound: SliderSound },
    File { path: PathBuf },
    /// 44-byte silent file.
    Silent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliderSound {
    Slide,
    Whistle,
    Tick,
}

impl SliderSound {
    pub fn stem(self, set: SampleSet, index: u32) -> Option<String> {
        slider_file_stem(set, match self {
            Self::Slide => "sliderslide",
            Self::Whistle => "sliderwhistle",
            Self::Tick => "slidertick",
        }, index)
    }
}

/// A source at a gain relative to the loudest sample of its moment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceMix {
    pub source: Source,
    /// Percent, quantized to [`GAIN_STEP_PCT`].
    pub gain_pct: u8,
}

/// Gain quantization; coarser steps mean fewer distinct mixes (Mapping Tools "roughness").
pub const GAIN_STEP_PCT: u8 = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct PackedSample {
    /// 0 = highest. Synthetic fills use `usize::MAX`.
    pub priority: usize,
    pub set: SampleSet,
    pub sound: Hitsound,
    pub source: Source,
    /// Linear 0..=1.
    pub volume: f32,
}

/// Everything sounding at one target of one difficulty.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub time: Ms,
    pub target: Target,
    pub samples: Vec<PackedSample>,
}

/// The 12-slot requirement or assignment for one custom index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomIndex {
    pub index: Option<u32>,
    /// Keyed by slot stem, e.g. `soft-hitnormal`. Empty/absent = don't care.
    pub slots: BTreeMap<String, BTreeSet<SourceMix>>,
}

impl CustomIndex {
    fn get(&self, key: &str) -> Option<&BTreeSet<SourceMix>> {
        self.slots.get(key).filter(|s| !s.is_empty())
    }

    /// Every non-empty slot of `other` is exactly ours.
    pub fn fits(&self, other: &CustomIndex) -> bool {
        other.slots.iter().filter(|(_, v)| !v.is_empty()).all(|(k, v)| self.get(k) == Some(v))
    }

    /// No slot where both are non-empty and differ.
    pub fn can_merge(&self, other: &CustomIndex) -> bool {
        other
            .slots
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .all(|(k, v)| self.get(k).is_none_or(|mine| mine == v))
    }

    pub fn merge_with(&mut self, other: &CustomIndex) {
        for (k, v) in &other.slots {
            self.slots.entry(k.clone()).or_default().extend(v.iter().cloned());
        }
        if self.index.is_none() {
            self.index = other.index;
        }
    }

    /// Whether `slot` needs a file in the beatmap folder. A slot whose only content
    /// is the matching skin sound at full gain needs none: osu! falls back to the skin
    /// when the indexed file is missing.
    pub fn needs_file(slot: &str, mixes: &BTreeSet<SourceMix>) -> bool {
        match mixes.iter().collect::<Vec<_>>().as_slice() {
            [SourceMix { source: Source::Skin { set, sound }, gain_pct: 100 }] => {
                sample_file_stem(*set, *sound, 1).as_deref() != Some(slot)
            }
            _ => true,
        }
    }
}

/// The object-level result for one package.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPackage {
    pub time: Ms,
    pub target: Target,
    pub normal_set: SampleSet,
    pub addition_set: SampleSet,
    pub whistle: bool,
    pub finish: bool,
    pub clap: bool,
    /// osu! volume 5..=100 for the greenline.
    pub volume: u32,
    pub requirement: CustomIndex,
}

/// Builds packages for sounding edges and custom ticks. Muted ends are written
/// directly by export.
pub fn packages_for(project: &Project, projection: &Projection) -> Vec<Package> {
    projection
        .hits
        .iter()
        .filter(|h| matches!(h.kind, HitKind::Matched | HitKind::Fallback))
        .map(|hit| Package {
            time: hit.time,
            target: hit.target,
            samples: hit
                .channels
                .iter()
                .filter_map(|&id| {
                    let ch = project.channel(id)?;
                    Some(PackedSample {
                        priority: project.priority(id)?,
                        set: ch.sample_set,
                        sound: ch.hitsound,
                        source: match &ch.sample {
                            SampleSource::SkinDefault => Source::Skin { set: ch.sample_set, sound: ch.hitsound },
                            SampleSource::File { path } => Source::File { path: path.clone() },
                        },
                        volume: ch.volume,
                    })
                })
                .collect(),
        })
        .collect()
}

fn top_set<'a>(samples: impl Iterator<Item = &'a PackedSample>) -> Option<SampleSet> {
    samples
        .filter(|s| s.set != SampleSet::Auto)
        .min_by_key(|s| s.priority)
        .map(|s| s.set)
}

/// Applies the hierarchy rule and volume balancing to one package.
pub fn resolve(package: &Package, addition_only_normal: AdditionOnlyNormal) -> ResolvedPackage {
    let normals = || package.samples.iter().filter(|s| s.sound == Hitsound::Normal);
    let additions = || package.samples.iter().filter(|s| s.sound.is_addition());

    // With no hitnormal channel, the hitnormal takes the project's fixed set if it has
    // one (D-78), otherwise the top addition's set as in Mapping Tools (D-31).
    let normal_set = top_set(normals())
        .or(addition_only_normal.fixed_set())
        .or_else(|| top_set(additions()))
        .unwrap_or(SampleSet::Soft);
    let addition_set = top_set(additions()).unwrap_or(normal_set);

    let max_vol = max_volume(package);

    let mut requirement = CustomIndex::default();
    let mut put = |set: SampleSet, sound: Hitsound, mix: SourceMix| {
        if let Some(key) = sample_file_stem(set, sound, 1) {
            requirement.slots.entry(key).or_default().insert(mix);
        }
    };
    for s in package.samples.iter() {
        let slot_set = if s.sound == Hitsound::Normal { normal_set } else { addition_set };
        put(slot_set, s.sound, SourceMix { source: s.source.clone(), gain_pct: relative_gain(s.volume, max_vol) });
    }
    if normals().next().is_none() {
        // Hitnormal always plays in standard/catch; decide what it is.
        let source = match addition_only_normal {
            AdditionOnlyNormal::Silent => Source::Silent,
            _ => Source::Skin { set: normal_set, sound: Hitsound::Normal },
        };
        put(normal_set, Hitsound::Normal, SourceMix { source, gain_pct: 100 });
    }

    let has = |h: Hitsound| package.samples.iter().any(|s| s.sound == h);
    ResolvedPackage {
        time: package.time,
        target: package.target,
        normal_set,
        addition_set,
        whistle: has(Hitsound::Whistle),
        finish: has(Hitsound::Finish),
        clap: has(Hitsound::Clap),
        volume: point_volume(max_vol),
        requirement,
    }
}

fn max_volume(package: &Package) -> f32 {
    package.samples.iter().map(|s| s.volume).fold(0.0_f32, f32::max).max(1e-6)
}

fn relative_gain(volume: f32, max_volume: f32) -> u8 {
    let pct = (volume / max_volume * 100.0).clamp(0.0, 100.0);
    ((pct / f32::from(GAIN_STEP_PCT)).round() as u8) * GAIN_STEP_PCT
}

fn point_volume(volume: f32) -> u32 {
    ((volume * 100.0).round() as u32).clamp(5, 100)
}

/// Stable plays a single slidertick at this moment (D-05). Bake its contributing
/// channel samples into that one slot. There is no additional hitnormal at a tick.
pub fn resolve_tick(package: &Package, body_set: SampleSet) -> ResolvedPackage {
    let normal_set = if body_set == SampleSet::Auto {
        top_set(package.samples.iter()).unwrap_or(SampleSet::Soft)
    } else {
        body_set
    };
    let max = max_volume(package);
    let slot = SliderSound::Tick.stem(normal_set, 1).expect("tick set is concrete");
    let mixes = package.samples.iter().map(|s| SourceMix {
        source: s.source.clone(),
        gain_pct: relative_gain(s.volume, max),
    }).collect();
    ResolvedPackage {
        time: package.time,
        target: package.target,
        normal_set,
        addition_set: normal_set,
        whistle: false,
        finish: false,
        clap: false,
        volume: point_volume(max),
        requirement: CustomIndex { index: None, slots: BTreeMap::from([(slot, mixes)]) },
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PackingError {
    #[error("the saved index schema cannot express a mix at {time} ms and growth is disabled")]
    SchemaTooSmall { time: Ms },
}

/// Builds the mapset-wide index schema from every difficulty's requirements.
///
/// With a `previous` schema, existing indices keep their numbers so re-exports don't
/// churn sample files; `allow_growth` permits adding new indices.
///
/// `reserved` lists index numbers that must not be handed out because the mapset
/// already holds sample files with that number which hitsounds-rs did not write (D-38).
/// A previous index that has become reserved is renumbered.
pub fn build_schema(
    requirements: &[CustomIndex],
    previous: Option<&[CustomIndex]>,
    allow_growth: bool,
    first_index: u32,
    reserved: &BTreeSet<u32>,
) -> Vec<CustomIndex> {
    let unreserve = |mut c: CustomIndex| {
        if c.index.is_some_and(|i| reserved.contains(&i)) {
            c.index = None;
        }
        c
    };
    match previous {
        None => give_indices(optimize(requirements), first_index, reserved),
        Some(prev) if allow_growth => {
            let all: Vec<CustomIndex> = prev.iter().cloned().map(unreserve).chain(requirements.iter().cloned()).collect();
            give_indices(optimize(&all), first_index, reserved)
        }
        Some(prev) => prev.to_vec(),
    }
}

/// Drops schema entries no package was assigned to, so their numbers can be reused
/// and no file is kept alive for them (RC: no unused files).
pub fn prune_schema(schema: Vec<CustomIndex>, used: &BTreeSet<u32>) -> Vec<CustomIndex> {
    schema.into_iter().filter(|c| c.index.is_some_and(|i| used.contains(&i))).collect()
}

fn optimize(indices: &[CustomIndex]) -> Vec<CustomIndex> {
    let mut out: Vec<CustomIndex> = Vec::new();
    for ci in indices {
        match out.iter_mut().find(|c| c.can_merge(ci)) {
            Some(target) => target.merge_with(ci),
            None => out.push(ci.clone()),
        }
    }
    // Drop indices that no requirement exclusively needs.
    let keep: Vec<bool> = (0..out.len())
        .map(|i| {
            indices
                .iter()
                .any(|req| out[i].fits(req) && out.iter().enumerate().all(|(j, o)| j == i || !o.fits(req)))
        })
        .collect();
    out.into_iter().zip(keep).filter_map(|(c, k)| k.then_some(c)).collect()
}

fn give_indices(mut indices: Vec<CustomIndex>, first: u32, reserved: &BTreeSet<u32>) -> Vec<CustomIndex> {
    let mut used: BTreeSet<u32> = indices.iter().filter_map(|c| c.index).chain(reserved.iter().copied()).collect();
    let mut next = first.max(1);
    for ci in indices.iter_mut().filter(|c| c.index.is_none()) {
        while used.contains(&next) {
            next = next.checked_add(1).unwrap_or(1);
        }
        ci.index = Some(next);
        used.insert(next);
    }
    indices
}

/// Picks an index for each package of one difficulty, preferring long runs of the
/// same index (fewer greenline changes).
pub fn assign(packages: &[ResolvedPackage], schema: &[CustomIndex]) -> Result<Vec<u32>, PackingError> {
    let mut out = Vec::with_capacity(packages.len());
    let mut i = 0;
    while i < packages.len() {
        // The first schema entry with the longest run wins, as in MT `MatchPackages`
        // (`count > bestCount`), so ties resolve to the lower schema position.
        let mut best: Option<(&CustomIndex, usize)> = None;
        for c in schema {
            let run = packages[i..].iter().take_while(|p| c.fits(&p.requirement)).count();
            if run > best.map_or(0, |(_, r)| r) {
                best = Some((c, run));
            }
        }
        let Some((ci, run)) = best else {
            return Err(PackingError::SchemaTooSmall { time: packages[i].time });
        };
        let index = ci.index.expect("schema indices are assigned");
        out.extend(std::iter::repeat_n(index, run));
        i += run;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(priority: usize, set: SampleSet, sound: Hitsound, file: &str) -> PackedSample {
        PackedSample {
            priority,
            set,
            sound,
            source: Source::File { path: file.into() },
            volume: 1.0,
        }
    }

    fn pkg(time: Ms, samples: Vec<PackedSample>) -> Package {
        Package { time, target: Target::Edge(0), samples }
    }

    #[test]
    fn lower_hitnormal_mixes_into_upper_set() {
        // normal-hitnormal is above soft-hitnormal: both end up in normal-hitnormal.
        let p = pkg(0, vec![
            sample(0, SampleSet::Normal, Hitsound::Normal, "kick.wav"),
            sample(1, SampleSet::Soft, Hitsound::Normal, "pad.wav"),
        ]);
        let r = resolve(&p, AdditionOnlyNormal::SkinDefault);
        assert_eq!(r.normal_set, SampleSet::Normal);
        let slot = &r.requirement.slots["normal-hitnormal"];
        assert_eq!(slot.len(), 2);
        assert!(!r.requirement.slots.contains_key("soft-hitnormal"));
    }

    #[test]
    fn addition_clash_uses_top_addition_set() {
        // drum-hitclap above soft-hitwhistle: additions become drum, whistle goes to drum-hitwhistle.
        let p = pkg(0, vec![
            sample(0, SampleSet::Soft, Hitsound::Normal, "n.wav"),
            sample(1, SampleSet::Drum, Hitsound::Clap, "snare.wav"),
            sample(2, SampleSet::Soft, Hitsound::Whistle, "hat.wav"),
        ]);
        let r = resolve(&p, AdditionOnlyNormal::SkinDefault);
        assert_eq!((r.normal_set, r.addition_set), (SampleSet::Soft, SampleSet::Drum));
        assert!(r.clap && r.whistle && !r.finish);
        assert!(r.requirement.slots.contains_key("drum-hitwhistle"));
        assert!(r.requirement.slots.contains_key("drum-hitclap"));
    }

    #[test]
    fn addition_only_gets_skin_normal_of_addition_set() {
        let p = pkg(0, vec![sample(0, SampleSet::Drum, Hitsound::Clap, "snare.wav")]);
        let r = resolve(&p, AdditionOnlyNormal::SkinDefault);
        assert_eq!(r.normal_set, SampleSet::Drum);
        let normal = &r.requirement.slots["drum-hitnormal"];
        assert!(!CustomIndex::needs_file("drum-hitnormal", normal));
    }

    #[test]
    fn addition_only_with_several_additions_adds_one_skin_hitnormal() {
        // OQ-1: several additions at once must not stack hitnormals.
        let p = pkg(0, vec![
            sample(0, SampleSet::Drum, Hitsound::Clap, "snare.wav"),
            sample(1, SampleSet::Soft, Hitsound::Whistle, "hat.wav"),
            sample(2, SampleSet::Normal, Hitsound::Finish, "crash.wav"),
        ]);
        let r = resolve(&p, AdditionOnlyNormal::SkinDefault);
        let normal_slots: Vec<_> = r.requirement.slots.iter().filter(|(k, _)| k.ends_with("-hitnormal")).collect();
        assert_eq!(normal_slots.len(), 1);
        let (key, mixes) = normal_slots[0];
        assert_eq!(key, "drum-hitnormal");
        assert_eq!(mixes.len(), 1);
        assert!(!CustomIndex::needs_file(key, mixes), "plain skin hitnormal needs no file");
    }

    #[test]
    fn addition_only_takes_the_fixed_hitnormal_set() {
        // D-78: a normal-set crash over a soft-hitnormal project default.
        let p = pkg(0, vec![
            sample(0, SampleSet::Normal, Hitsound::Finish, "crash.wav"),
            sample(1, SampleSet::Soft, Hitsound::Clap, "snare.wav"),
        ]);
        let r = resolve(&p, AdditionOnlyNormal::SkinSoft);
        assert_eq!((r.normal_set, r.addition_set), (SampleSet::Soft, SampleSet::Normal));
        let normal = &r.requirement.slots["soft-hitnormal"];
        assert!(!CustomIndex::needs_file("soft-hitnormal", normal), "plain skin hitnormal needs no file");
        assert!(!r.requirement.slots.contains_key("normal-hitnormal"));
        assert!(r.requirement.slots.contains_key("normal-hitfinish"));
        assert!(r.requirement.slots.contains_key("normal-hitclap"), "additions still share the top addition set");
    }

    #[test]
    fn a_fixed_hitnormal_set_leaves_real_hitnormals_alone() {
        let p = pkg(0, vec![
            sample(0, SampleSet::Normal, Hitsound::Finish, "crash.wav"),
            sample(1, SampleSet::Drum, Hitsound::Normal, "kick.wav"),
        ]);
        let r = resolve(&p, AdditionOnlyNormal::SkinSoft);
        assert_eq!((r.normal_set, r.addition_set), (SampleSet::Drum, SampleSet::Normal));
        let normal = &r.requirement.slots["drum-hitnormal"];
        assert_eq!(normal.len(), 1, "no skin hitnormal is added under a real one");
    }

    #[test]
    fn compatible_requirements_share_an_index() {
        let a = resolve(&pkg(0, vec![sample(0, SampleSet::Soft, Hitsound::Normal, "kick.wav")]), AdditionOnlyNormal::SkinDefault);
        let b = resolve(&pkg(10, vec![
            sample(0, SampleSet::Soft, Hitsound::Normal, "kick.wav"),
            sample(1, SampleSet::Soft, Hitsound::Clap, "snare.wav"),
        ]), AdditionOnlyNormal::SkinDefault);
        let c = resolve(&pkg(20, vec![sample(0, SampleSet::Soft, Hitsound::Normal, "other.wav")]), AdditionOnlyNormal::SkinDefault);
        let reqs: Vec<_> = [&a, &b, &c].iter().map(|r| r.requirement.clone()).collect();
        let schema = build_schema(&reqs, None, false, 1, &BTreeSet::new());
        assert_eq!(schema.len(), 2);
        let idx = assign(&[a, b, c], &schema).unwrap();
        assert_eq!(idx[0], idx[1]);
        assert_ne!(idx[1], idx[2]);
    }

    #[test]
    fn quieter_samples_are_baked_relative_to_loudest() {
        let mut quiet = sample(1, SampleSet::Soft, Hitsound::Clap, "snare.wav");
        quiet.volume = 0.4;
        let mut loud = sample(0, SampleSet::Soft, Hitsound::Normal, "kick.wav");
        loud.volume = 0.8;
        let r = resolve(&pkg(0, vec![loud, quiet]), AdditionOnlyNormal::SkinDefault);
        assert_eq!(r.volume, 80);
        let clap = r.requirement.slots["soft-hitclap"].iter().next().unwrap();
        assert_eq!(clap.gain_pct, 50);
    }
}
