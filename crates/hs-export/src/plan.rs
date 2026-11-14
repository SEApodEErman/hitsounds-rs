//! From a project and one difficulty to the hits that get written, and to what they
//! sound like. Export and preview both go through here.

use crate::apply::{WrittenHit, apply_hits};
use crate::library::find_sample;
use crate::simulate::edge_sounds;
use crate::slider::{self, Bodies, SliderPlaying};
use hs_core::packing::{
    self, CustomIndex, PackingError, ResolvedPackage, SliderSound, Source, SourceMix,
};
use hs_core::project::ExportSettings;
use hs_core::projection::{self, DiffTimeline, HitKind, Projection, Target};
use hs_core::resolve::Playing;
use hs_core::{DiffProfile, Ms, Project};
use osu_format::sample::sample_file_stem;
use osu_format::{HitObject, SampleSet, TimingPoint};
use std::collections::BTreeSet;
use std::path::Path;

/// One difficulty projected and packed, before indices are assigned.
#[derive(Debug, Clone)]
pub struct DiffPlan {
    pub timeline: DiffTimeline,
    pub projection: Projection,
    /// One per sounding edge or custom tick, in time order.
    pub resolved: Vec<ResolvedPackage>,
    pub(crate) bodies: Bodies,
}

impl DiffPlan {
    /// Projects the base layer onto `timeline` with `profile`'s overrides.
    pub fn for_diff(project: &Project, profile: &DiffProfile, timeline: DiffTimeline) -> Self {
        let projection = projection::project(project, profile, &timeline);
        let bodies = slider::bodies(profile, &timeline);
        Self::from_projection(project, Some(profile), timeline, projection, bodies)
    }

    /// The base layer as a virtual difficulty (one circle per moment), with the sample
    /// sets of `sets` when given (D-91).
    pub fn for_base(project: &Project, sets: Option<&DiffTimeline>) -> Self {
        let (timeline, projection) = projection::base_view(project, sets);
        Self::from_projection(project, None, timeline, projection, Bodies::new())
    }

    fn from_projection(
        project: &Project,
        profile: Option<&DiffProfile>,
        timeline: DiffTimeline,
        projection: Projection,
        bodies: Bodies,
    ) -> Self {
        // The volume envelope scales each hit's greenline, never the mix (D-44).
        let envelope = project.volume_for(profile);
        let resolved = packing::packages_for(project, &projection)
            .iter()
            .map(|p| match p.target {
                Target::Edge(i) => packing::resolve(
                    p,
                    project.export.addition_only_normal,
                    timeline.edges[i].point_set,
                ),
                Target::Tick(i) => {
                    let set = bodies
                        .get(&timeline.ticks[i].object)
                        .map_or(SampleSet::Auto, |b| b.0);
                    packing::resolve_tick(p, set)
                }
            })
            .map(|mut r| {
                r.volume = envelope.scale(r.volume, r.time);
                r
            })
            .collect();
        Self {
            timeline,
            projection,
            resolved,
            bodies,
        }
    }

    /// The hits to write once `indices` (one per resolved package, from
    /// [`packing::assign`]) are known.
    pub fn written_hits(&self, profile: Option<&DiffProfile>, indices: &[u32]) -> Vec<WrittenHit> {
        let mut out: Vec<WrittenHit> = self
            .resolved
            .iter()
            .zip(indices)
            .filter_map(|(r, &index)| match r.target {
                Target::Edge(edge) => Some(WrittenHit {
                    edge,
                    normal_set: r.normal_set,
                    addition_set: r.addition_set,
                    whistle: r.whistle,
                    finish: r.finish,
                    clap: r.clap,
                    index: Some(index),
                    volume: Some(r.volume),
                    reset: false,
                }),
                Target::Tick(_) => None,
            })
            .collect();
        let mute = profile.and_then(|p| p.slider_end_mute.as_ref());
        for hit in &self.projection.hits {
            let Target::Edge(edge) = hit.target else {
                continue;
            };
            match (hit.kind, mute) {
                (HitKind::Muted, Some(m)) => out.push(WrittenHit::muted(edge, m)),
                (HitKind::Empty, _) => out.push(WrittenHit::reset(edge)),
                _ => {}
            }
        }
        out.sort_by_key(|h| self.timeline.edges[h.edge].time);
        out
    }
}

/// Use one mapset-wide pool for edges and a separate range for generated ticks.
pub(crate) fn build_schema<'a>(
    plans: impl IntoIterator<Item = &'a DiffPlan>,
    previous: &[CustomIndex],
    settings: &ExportSettings,
    reserved: &BTreeSet<u32>,
) -> Vec<CustomIndex> {
    let (ticks, edges): (Vec<_>, Vec<_>) = plans
        .into_iter()
        .flat_map(|p| &p.resolved)
        .partition(|r| matches!(r.target, Target::Tick(_)));
    let (prev_ticks, prev_edges): (Vec<_>, Vec<_>) = previous
        .iter()
        .cloned()
        .partition(|c| c.slots.keys().any(|slot| slot.ends_with("-slidertick")));
    let old = |entries: &[CustomIndex]| {
        entries
            .iter()
            .filter(|c| settings.allow_growth || c.index.is_none_or(|i| !reserved.contains(&i)))
            .cloned()
            .collect::<Vec<_>>()
    };
    let prev_edges = old(&prev_edges);
    let prev_ticks = old(&prev_ticks);
    let mut reserved_edges = reserved.clone();
    reserved_edges.extend(prev_ticks.iter().filter_map(|c| c.index));
    let mut schema = packing::build_schema(
        &edges
            .into_iter()
            .map(|r| r.requirement.clone())
            .collect::<Vec<_>>(),
        (!previous.is_empty()).then_some(&prev_edges),
        settings.allow_growth,
        settings.first_custom_index,
        &reserved_edges,
    );
    let mut reserved_ticks = reserved.clone();
    reserved_ticks.extend(schema.iter().filter_map(|c| c.index));
    schema.extend(packing::build_schema(
        &ticks
            .into_iter()
            .map(|r| r.requirement.clone())
            .collect::<Vec<_>>(),
        (!previous.is_empty()).then_some(&prev_ticks),
        settings.allow_growth,
        settings.body_sample_first_index,
        &reserved_ticks,
    ));
    schema
}

/// The content of the sample osu! would load for `playing`: the schema's mix if the
/// export writes a file for that slot, else the skin's own sound.
pub fn sound_content(playing: &Playing, schema: &[CustomIndex]) -> BTreeSet<SourceMix> {
    let skin = || {
        BTreeSet::from([SourceMix {
            source: Source::Skin {
                set: playing.set,
                sound: playing.sound,
            },
            gain_pct: 100,
        }])
    };
    let Some(slot) = sample_file_stem(playing.set, playing.sound, 1) else {
        return BTreeSet::new();
    };
    if playing.index == 0 {
        return skin();
    }
    schema
        .iter()
        .find(|c| c.index == Some(playing.index))
        .and_then(|c| c.slots.get(&slot))
        .filter(|mixes| !mixes.is_empty() && CustomIndex::needs_file(&slot, mixes))
        .cloned()
        .unwrap_or_else(skin)
}

/// One sound of a previewed hit: what to render, and the gain stable plays it at.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewSound {
    pub content: BTreeSet<SourceMix>,
    pub gain: f32,
    /// A generated file receives the same optional RC fixes in preview and export.
    pub generated: bool,
    /// Body sounds loop until this song time; edges and ticks play once.
    pub end_ms: Option<Ms>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewHit {
    pub time: Ms,
    pub sounds: Vec<PreviewSound>,
}

/// A difficulty to preview: its profile and its parsed objects and timing.
pub struct PreviewDiff<'a> {
    pub profile: &'a DiffProfile,
    pub timeline: &'a DiffTimeline,
    pub objects: &'a [HitObject],
    pub points: &'a [TimingPoint],
    pub general_set: SampleSet,
}

/// What the monitored project sounds like: the base layer (`diff: None`) or one
/// difficulty. The hits are written into a scratch copy of the difficulty and read
/// back with stable's rules, exactly as export does, so the preview is the export
/// (D-26). Channels muted or not soloed are left out, which export never does (D-14a).
pub fn preview(
    project: &Project,
    diff: Option<&PreviewDiff<'_>>,
) -> Result<Vec<PreviewHit>, PackingError> {
    preview_with_mapset(project, diff, None, None)
}

/// [`preview`] with the mapset's own samples. `base_sets` is the difficulty whose
/// greenline sample sets the base layer borrows (D-91); it is ignored for a difficulty.
pub fn preview_in_project(
    project: &Project,
    diff: Option<&PreviewDiff<'_>>,
    base_sets: Option<&DiffTimeline>,
    project_dir: &Path,
) -> Result<Vec<PreviewHit>, PackingError> {
    preview_with_mapset(project, diff, base_sets, Some(project_dir))
}

fn preview_with_mapset(
    project: &Project,
    diff: Option<&PreviewDiff<'_>>,
    base_sets: Option<&DiffTimeline>,
    project_dir: Option<&Path>,
) -> Result<Vec<PreviewHit>, PackingError> {
    let mut plan = match diff {
        Some(d) => DiffPlan::for_diff(project, d.profile, d.timeline.clone()),
        None => DiffPlan::for_base(project, base_sets),
    };

    // Monitoring: drop inaudible channels, and whole hits that become empty.
    let mut silenced = BTreeSet::new();
    for hit in &mut plan.projection.hits {
        if matches!(hit.kind, HitKind::Matched | HitKind::Fallback) {
            hit.channels.retain(|&c| project.audible(c));
            if hit.channels.is_empty() {
                silenced.insert(match hit.target {
                    Target::Edge(i) => (false, i),
                    Target::Tick(i) => (true, i),
                });
            }
        }
    }
    plan.projection.hits.retain(|h| {
        !(matches!(h.kind, HitKind::Matched | HitKind::Fallback) && h.channels.is_empty())
    });
    let plan = DiffPlan::from_projection(
        project,
        diff.map(|d| d.profile),
        plan.timeline,
        plan.projection,
        plan.bodies,
    );

    let existing = project_dir
        .filter(|_| diff.is_some())
        .map(|dir| crate::export::existing_samples(project, dir));
    let reserved = existing
        .as_ref()
        .map(|files| files.values().map(|(_, name)| name.index).collect())
        .unwrap_or_default();
    let settings = ExportSettings {
        allow_growth: true,
        ..project.export.clone()
    };
    let schema = build_schema([&plan], &[], &settings, &reserved);
    let indices = packing::assign(&plan.resolved, &schema)?;
    let hits = plan.written_hits(diff.map(|d| d.profile), &indices);

    let (mut objects, mut points, general_set) = match diff {
        Some(d) => (d.objects.to_vec(), d.points.to_vec(), d.general_set),
        None => base_scratch(&plan.timeline),
    };
    if let Some((dir, diff)) = project_dir.zip(diff) {
        let manifest = crate::Manifest::load(dir);
        if manifest.applies_to(&project.mapset_dir) {
            manifest
                .sliders
                .restore(&diff.profile.file, &mut objects, &mut points);
        }
    }
    apply_hits(&plan.timeline, &mut objects, &mut points, &hits, &schema, general_set);
    slider::apply(
        &plan.timeline,
        &mut objects,
        &mut points,
        &plan.bodies,
        &plan.resolved,
        &indices,
    );
    let sounds = edge_sounds(&plan.timeline, &objects, &points, general_set);

    let file = |stem: String| {
        existing
            .as_ref()
            .and_then(|files| files.get(&stem))
            .and_then(|(name, _)| {
                find_sample(&project.mapset_dir, &stem)
                    .or_else(|| Some(project.mapset_dir.join(name)))
            })
    };
    let mut preview: Vec<_> = plan
        .timeline
        .edges
        .iter()
        .zip(sounds)
        .enumerate()
        .filter(|(i, _)| !silenced.contains(&(false, *i)))
        .map(|(_, (edge, playing))| PreviewHit {
            time: edge.time,
            sounds: playing
                .iter()
                .map(|p| {
                    let mut content = sound_content(p, &schema);
                    let generated = sample_file_stem(p.set, p.sound, 1)
                        .is_some_and(|slot| CustomIndex::needs_file(&slot, &content));
                    if p.index != 0
                        && !generated
                        && let Some(path) =
                            sample_file_stem(p.set, p.sound, p.index).and_then(&file)
                    {
                        content = single(Source::File { path });
                    }
                    PreviewSound {
                        content,
                        gain: p.gain(),
                        generated,
                        end_ms: None,
                    }
                })
                .collect(),
        })
        .collect();
    let slider_sound = |p: SliderPlaying, end_ms| {
        let slot = p.sound.stem(p.set, 1);
        let mixes = schema
            .iter()
            .find(|c| c.index == Some(p.index))
            .and_then(|c| c.slots.get(slot.as_deref()?));
        let blank = match p.sound {
            SliderSound::Tick => project.export.mute_slider_ticks,
            _ => project.export.mute_slider_slides,
        };
        let content = if let Some(mixes) = mixes {
            mixes.clone()
        } else if let Some(path) = (p.index != 0).then(|| p.stem().and_then(&file)).flatten() {
            single(Source::File { path })
        } else if blank && p.index != 0 {
            single(Source::Silent)
        } else {
            single(Source::SkinSlider {
                set: p.set,
                sound: p.sound,
            })
        };
        PreviewSound {
            content,
            gain: p.gain(),
            generated: mixes.is_some(),
            end_ms,
        }
    };
    for (i, &tick) in plan.timeline.ticks.iter().enumerate() {
        if !silenced.contains(&(true, i)) {
            let playing = slider::tick_at(tick, &objects, &points, general_set);
            preview.push(PreviewHit {
                time: tick.time,
                sounds: vec![slider_sound(playing, None)],
            });
        }
    }
    for (time, end, playing) in slider::slides(&plan.timeline, &objects, &points, general_set) {
        preview.push(PreviewHit {
            time,
            sounds: vec![slider_sound(playing, Some(end))],
        });
    }
    preview.sort_by_key(|h| h.time);
    Ok(preview)
}

fn single(source: Source) -> BTreeSet<SourceMix> {
    BTreeSet::from([SourceMix {
        source,
        gain_pct: 100,
    }])
}
/// Objects and timing for the virtual base difficulty: a circle per edge and one redline.
fn base_scratch(timeline: &DiffTimeline) -> (Vec<HitObject>, Vec<TimingPoint>, SampleSet) {
    let objects = timeline
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            HitObject::parse(&format!("256,192,{},1,0,0:0:0:0:", e.time), i)
                .expect("synthetic circle parses")
        })
        .collect();
    let redline = TimingPoint {
        time: 0.0,
        beat_length: 500.0,
        meter: 4,
        sample_set: SampleSet::Normal,
        sample_index: 0,
        volume: 100,
        uninherited: true,
        effects: 0,
    };
    (objects, vec![redline], SampleSet::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hs_core::{Channel, SampleSource};
    use osu_format::{Hitsound, OsuDocument};

    fn file(path: &str) -> SourceMix {
        SourceMix {
            source: Source::File { path: path.into() },
            gain_pct: 100,
        }
    }

    fn skin(set: SampleSet, sound: Hitsound) -> SourceMix {
        SourceMix {
            source: Source::Skin { set, sound },
            gain_pct: 100,
        }
    }

    fn project() -> Project {
        let mut p = Project::new("t", ".");
        let mut kick = Channel::new(0, "Kick", SampleSet::Drum, Hitsound::Normal);
        kick.sample = SampleSource::File {
            path: "samples/kick.wav".into(),
        };
        kick.events = vec![1000, 2000];
        p.add_channel(kick);
        let mut snare = Channel::new(0, "Snare", SampleSet::Drum, Hitsound::Clap);
        snare.sample = SampleSource::File {
            path: "samples/snare.wav".into(),
        };
        snare.volume = 0.5;
        snare.events = vec![2000, 3000];
        p.add_channel(snare);
        p
    }

    #[test]
    fn volume_envelope_scales_greenlines_without_new_samples() {
        use hs_core::{Curve, VolumePoint};
        let flat = project();
        let mut ramped = project();
        // 100 % at 1000 ms, ramping to 50 % at 3000 ms (D-44).
        ramped.volume = hs_core::VolumeEnvelope::new(vec![
            VolumePoint { time: 1000, volume: 100, curve: Curve::Linear },
            VolumePoint { time: 3000, volume: 50, curve: Curve::Linear },
        ]);
        let (a, b) = (DiffPlan::for_base(&flat, None), DiffPlan::for_base(&ramped, None));
        let volumes = |p: &DiffPlan| p.resolved.iter().map(|r| r.volume).collect::<Vec<_>>();
        assert_eq!(volumes(&a), vec![100, 100, 50]);
        assert_eq!(volumes(&b), vec![100, 75, 25], "greenlines follow the ramp");
        let mixes = |p: &DiffPlan| p.resolved.iter().map(|r| r.requirement.clone()).collect::<Vec<_>>();
        assert_eq!(mixes(&a), mixes(&b), "the sample content is unchanged");

        // Preview hears the same scaling: the 2000 ms kick plays at 75 % x 0.8.
        let hits = preview(&ramped, None).unwrap();
        assert!((hits[1].sounds[0].gain - 0.75 * 0.8).abs() < 1e-6);
    }

    #[test]
    fn base_preview_plays_merged_content_at_stable_gains() {
        let hits = preview(&project(), None).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.time).collect::<Vec<_>>(),
            vec![1000, 2000, 3000]
        );

        // Kick alone: one hitnormal at 100 % x 0.8.
        assert_eq!(
            hits[0].sounds,
            vec![PreviewSound {
                content: BTreeSet::from([file("samples/kick.wav")]),
                gain: 0.8,
                generated: true,
                end_ms: None
            }]
        );

        // Kick + half-volume snare: greenline at 100 %, snare baked in at 50 %.
        let snare_half = SourceMix {
            source: Source::File {
                path: "samples/snare.wav".into(),
            },
            gain_pct: 50,
        };
        assert_eq!(
            hits[1].sounds[1],
            PreviewSound {
                content: BTreeSet::from([snare_half]),
                gain: 0.85,
                generated: true,
                end_ms: None
            }
        );

        // Snare alone (addition only): the skin's drum-hitnormal plays under it (D-31),
        // and the greenline carries the snare's 50 %.
        assert_eq!(
            hits[2].sounds[0].content,
            BTreeSet::from([skin(SampleSet::Drum, Hitsound::Normal)])
        );
        assert!((hits[2].sounds[0].gain - 0.5 * 0.8).abs() < 1e-6);
        assert_eq!(
            hits[2].sounds[1].content,
            BTreeSet::from([file("samples/snare.wav")])
        );
    }

    #[test]
    fn a_fixed_hitnormal_set_plays_under_lone_additions_in_preview() {
        // D-78: the snare alone at 3000 ms gets the project's soft-hitnormal, not the
        // drum-hitnormal of its own set, and the snare keeps its drum slot.
        let mut p = project();
        p.export.addition_only_normal = hs_core::project::AdditionOnlyNormal::SkinSoft;
        let hits = preview(&p, None).unwrap();
        assert_eq!(
            hits[2].sounds[0].content,
            BTreeSet::from([skin(SampleSet::Soft, Hitsound::Normal)])
        );
        let plan = DiffPlan::for_base(&p, None);
        assert_eq!(
            (plan.resolved[2].normal_set, plan.resolved[2].addition_set),
            (SampleSet::Soft, SampleSet::Drum)
        );
        // The kick's own drum hitnormal is untouched where it plays.
        assert_eq!(plan.resolved[1].normal_set, SampleSet::Drum);
    }

    #[test]
    fn a_lone_addition_plays_over_the_difficultys_hitnormal() {
        // D-91: the snare alone at 3000 ms sits on a soft greenline, so it plays over
        // the skin's soft-hitnormal, not the drum-hitnormal of its own set.
        let p = project();
        let doc = OsuDocument::parse(
            "osu file format v14

[General]
SampleSet: Normal

[TimingPoints]
0,500,4,2,0,60,1,0

[HitObjects]
0,0,1000,1,0,0:0:0:0:
0,0,2000,1,0,0:0:0:0:
0,0,3000,1,0,0:0:0:0:
",
        );
        let timeline = DiffTimeline::from_document(&doc).unwrap();
        let objects = doc.hit_objects().unwrap();
        let points = doc.timing_points().unwrap();
        let profile = DiffProfile::new("d.osu", "D");
        let hits = preview(
            &p,
            Some(&PreviewDiff {
                profile: &profile,
                timeline: &timeline,
                objects: &objects,
                points: &points,
                general_set: SampleSet::Normal,
            }),
        )
        .unwrap();
        assert_eq!(
            hits[2].sounds[0].content,
            BTreeSet::from([skin(SampleSet::Soft, Hitsound::Normal)])
        );
        assert_eq!(
            hits[2].sounds[1].content,
            BTreeSet::from([file("samples/snare.wav")])
        );
        let plan = DiffPlan::for_diff(&p, &profile, timeline);
        assert_eq!(
            (plan.resolved[2].normal_set, plan.resolved[2].addition_set),
            (SampleSet::Soft, SampleSet::Drum)
        );
        // The kick's own drum hitnormal still plays where it is.
        assert_eq!(plan.resolved[1].normal_set, SampleSet::Drum);
    }

    #[test]
    fn muted_channels_are_left_out_of_the_preview_only() {
        let mut p = project();
        p.channels[0].muted = true;
        let hits = preview(&p, None).unwrap();
        // 1000 had only the kick: nothing plays there at all.
        assert_eq!(
            hits.iter().map(|h| h.time).collect::<Vec<_>>(),
            vec![2000, 3000]
        );
        // 2000 is now the snare alone.
        assert_eq!(
            hits[0].sounds[1].content,
            BTreeSet::from([file("samples/snare.wav")])
        );
    }

    #[test]
    fn solo_silences_everything_else() {
        let mut p = project();
        p.channels[0].solo = true;
        let hits = preview(&p, None).unwrap();
        assert_eq!(
            hits.iter().map(|h| h.time).collect::<Vec<_>>(),
            vec![1000, 2000]
        );
        assert_eq!(hits[1].sounds.len(), 1);
    }

    #[test]
    fn slider_slides_keep_the_difficultys_set_after_a_hit() {
        // D-79: the slider (sets 0:0) follows the soft greenline, even though its head
        // gets the project's drum kick.
        let p = project();
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[General]\nSampleSet: None\n\n[Difficulty]\nSliderMultiplier:1\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,80,1,0\n\n[HitObjects]\n0,0,1000,2,0,L|100:0,1,100,0|0,0:0|0:0,0:0:0:0:\n",
        );
        let timeline = DiffTimeline::from_document(&doc).unwrap();
        let objects = doc.hit_objects().unwrap();
        let points = doc.timing_points().unwrap();
        let profile = DiffProfile::new("d.osu", "D");
        let hits = preview(
            &p,
            Some(&PreviewDiff {
                profile: &profile,
                timeline: &timeline,
                objects: &objects,
                points: &points,
                general_set: SampleSet::Normal,
            }),
        )
        .unwrap();
        let slides: Vec<_> = hits
            .iter()
            .flat_map(|h| &h.sounds)
            .filter(|s| s.end_ms.is_some())
            .collect();
        assert!(!slides.is_empty());
        for slide in slides {
            assert_eq!(
                slide.content,
                single(Source::SkinSlider {
                    set: SampleSet::Soft,
                    sound: SliderSound::Slide,
                })
            );
        }
    }

    #[test]
    fn an_empty_object_after_a_custom_snare_stays_on_its_index() {
        // D-81, the case from the first hands-on pass: a lone custom snare gets an
        // index, and the empty object after it plays the skin's soft-hitnormal at that
        // index, since the index has no soft-hitnormal of its own. So no greenline
        // goes back to index 0.
        let mut p = Project::new("t", ".");
        let mut snare = Channel::new(0, "Snare", SampleSet::Drum, Hitsound::Clap);
        snare.sample = SampleSource::File {
            path: "samples/snare.wav".into(),
        };
        snare.events = vec![1000];
        p.add_channel(snare);
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[General]\nSampleSet: Soft\n\n[TimingPoints]\n0,500,4,2,0,60,1,0\n\n[HitObjects]\n0,0,1000,1,0,0:0:0:0:\n0,0,1500,1,0,0:0:0:0:\n",
        );
        let timeline = DiffTimeline::from_document(&doc).unwrap();
        let profile = DiffProfile::new("d.osu", "D");
        let plan = DiffPlan::for_diff(&p, &profile, timeline.clone());
        let schema = build_schema([&plan], &[], &p.export, &BTreeSet::new());
        let indices = packing::assign(&plan.resolved, &schema).unwrap();
        let hits = plan.written_hits(Some(&profile), &indices);
        let mut objects = doc.hit_objects().unwrap();
        let mut points = doc.timing_points().unwrap();
        apply_hits(&timeline, &mut objects, &mut points, &hits, &schema, SampleSet::Soft);
        assert_eq!(points.len(), 2, "{points:?}");
        assert_ne!(points[1].sample_index, 0);

        let objects = doc.hit_objects().unwrap();
        let points = doc.timing_points().unwrap();
        let hits = preview(
            &p,
            Some(&PreviewDiff {
                profile: &profile,
                timeline: &timeline,
                objects: &objects,
                points: &points,
                general_set: SampleSet::Soft,
            }),
        )
        .unwrap();
        assert_eq!(
            hits[1].sounds.iter().map(|s| s.content.clone()).collect::<Vec<_>>(),
            vec![BTreeSet::from([skin(SampleSet::Soft, Hitsound::Normal)])]
        );
    }

    #[test]
    fn diff_preview_plays_fallback_and_reset_edges() {
        let mut p = project();
        let fb = p.add_channel(Channel::new(
            0,
            "Fallback",
            SampleSet::Soft,
            Hitsound::Normal,
        ));
        let doc = OsuDocument::parse(
            "osu file format v14\n\n[General]\nSampleSet: Normal\n\n[TimingPoints]\n0,500,4,1,3,60,1,0\n\n[HitObjects]\n0,0,1000,1,0,0:0:0:0:\n0,0,1500,1,8,3:3:0:0:\n",
        );
        let timeline = DiffTimeline::from_document(&doc).unwrap();
        let objects = doc.hit_objects().unwrap();
        let points = doc.timing_points().unwrap();
        let mut profile = DiffProfile::new("d.osu", "D");
        let view = |profile: &DiffProfile| {
            preview(
                &p,
                Some(&PreviewDiff {
                    profile,
                    timeline: &timeline,
                    objects: &objects,
                    points: &points,
                    general_set: SampleSet::Normal,
                }),
            )
            .unwrap()
        };

        // No fallback: the object at 1500 is reset to a plain hit on the skin's samples
        // (its old clap is gone), at the volume of the hit before it. Its set is the
        // difficulty's own (the redline's normal), not the drum kick's before it (D-79).
        let hits = view(&profile);
        assert_eq!(
            hits[1].sounds,
            vec![PreviewSound {
                content: BTreeSet::from([skin(SampleSet::Normal, Hitsound::Normal)]),
                gain: 0.8,
                generated: false,
                end_ms: None
            }]
        );

        // With a fallback channel it plays the fallback's soft-hitnormal instead.
        profile.fallback = Some(fb);
        let hits = view(&profile);
        assert_eq!(
            hits[1].sounds,
            vec![PreviewSound {
                content: BTreeSet::from([skin(SampleSet::Soft, Hitsound::Normal)]),
                gain: 0.8,
                generated: false,
                end_ms: None
            }]
        );
    }
}
