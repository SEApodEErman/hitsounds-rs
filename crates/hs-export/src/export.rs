//! The export: plan everything in memory, verify it, then write.
//!
//! [`plan`] never touches the mapset. It returns the new text of every difficulty, the
//! bytes of every sample file, and a list of problems; the UI shows that as a preview.
//! [`execute`] writes a plan that has no problems, after backing up what it replaces
//! (D-19). See docs/architecture.md, "Export pipeline".

use crate::apply::apply_hits;
use crate::checks::{SampleFix, SampleIssue, auto_fix, check_sample};
use crate::encode::{SourceKind, encode, extension};
use crate::library::{SampleLibrary, describe};
use crate::names::{SampleName, parse_sample_name};
use crate::plan::{DiffPlan, build_schema};
use crate::simulate::edge_sounds;
use crate::slider::{self, SliderPlaying};
use crate::wav::blank_wav;
pub use crate::write::write_atomic;
use crate::write::{Change, commit, snapshot};
use hs_core::import::general_sample_set;
use hs_core::packing::{self, CustomIndex, SliderSound, Source, SourceMix};
use hs_core::project::{SampleFormat, WriteMode};
use hs_core::projection::{DiffTimeline, HitKind, Target, Warning};
use hs_core::resolve::Playing;
use hs_core::{DiffProfile, Project};
use osu_format::sample::sample_file_stem;
use osu_format::{Hitsound, OsuDocument};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("the export has problems that must be fixed first: {0}")]
    Blocked(String),
    #[error("{0} changed on disk since the export was previewed. Build a new preview.")]
    Changed(String),
    #[error(
        "export failed ({error}). Some files could not be restored ({files}). Backups are at {backup}."
    )]
    Rollback {
        error: String,
        files: String,
        backup: String,
    },
}

/// The sample files hitsounds-rs wrote into the mapset last time. Only these are ever
/// overwritten or cleaned up without asking (findings 7, "no unused files").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub files: Vec<String>,
    /// Ownership belongs to this mapset, even if the project is later pointed elsewhere.
    #[serde(default)]
    pub mapset_dir: Option<PathBuf>,
    #[serde(default)]
    pub(crate) sliders: slider::History,
}

impl Manifest {
    pub fn path(project_dir: &Path) -> PathBuf {
        project_dir.join("exports").join("manifest.json")
    }

    pub fn load(project_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(project_dir))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, project_dir: &Path) -> std::io::Result<()> {
        let path = Self::path(project_dir);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(
            &path,
            serde_json::to_string_pretty(self)
                .unwrap_or_default()
                .as_bytes(),
        )
    }

    fn owned(&self, mapset: &Path) -> BTreeSet<String> {
        if !self.applies_to(mapset) {
            return BTreeSet::new();
        }
        self.files
            .iter()
            .filter(|f| parse_sample_name(f).is_some())
            .map(|f| f.to_ascii_lowercase())
            .collect()
    }

    pub(crate) fn applies_to(&self, mapset: &Path) -> bool {
        self.mapset_dir.as_deref().is_none_or(|p| p == mapset)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DiffStats {
    pub edges: usize,
    pub matched: usize,
    pub fallback: usize,
    pub muted: usize,
    /// Edges with no event and no fallback: reset to a plain default hit (D-36).
    pub empty: usize,
    /// Events with no object to play on in this difficulty.
    pub orphaned: usize,
    pub ticks: usize,
    pub slider_bodies: usize,
    pub objects_changed: usize,
    pub greenlines_before: usize,
    pub greenlines_after: usize,
    /// Greenlines removed because they changed nothing (D-80), counted in `greenlines_after`.
    pub greenlines_removed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffExport {
    /// The difficulty's file name in the mapset.
    pub file: String,
    /// The file name written: the same in direct mode, a new one in hitsound-only mode.
    pub out_file: String,
    pub label: String,
    #[serde(skip)]
    pub original: String,
    #[serde(skip)]
    pub updated: String,
    pub changed: bool,
    pub stats: DiffStats,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SampleFile {
    pub name: String,
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub size: usize,
    /// What the file is a mix of, for the preview.
    pub content: String,
    pub issues: Vec<SampleIssue>,
    pub fixes: Vec<SampleFix>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportPlan {
    pub mode: WriteMode,
    pub mapset_dir: PathBuf,
    pub diffs: Vec<DiffExport>,
    pub samples: Vec<SampleFile>,
    /// Files from the last export that nothing uses any more; deleted on export.
    pub stale: Vec<String>,
    /// Sample files hitsounds-rs did not write that this export replaces. They are
    /// moved to the backup folder first. Only with `take_over_samples`.
    pub taken_over: Vec<String>,
    pub schema: Vec<CustomIndex>,
    /// Anything listed here blocks the export.
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    inputs: BTreeMap<String, crate::write::FileStamp>,
    #[serde(skip)]
    retained: Vec<String>,
    #[serde(skip)]
    sliders: slider::History,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    /// Where the files went: the mapset, or the hitsound-only folder.
    pub out_dir: PathBuf,
    pub backup_dir: Option<PathBuf>,
    pub diffs_written: usize,
    pub samples_written: usize,
    pub files_removed: usize,
}

/// A sample file in the mapset that hitsounds-rs did not write.
struct Foreign {
    file: String,
    name: SampleName,
}

struct Prepared<'a> {
    profile: &'a DiffProfile,
    original: String,
    doc: OsuDocument,
    plan: DiffPlan,
}

fn osu_file(file: &str) -> bool {
    let mut parts = Path::new(file).components();
    matches!(parts.next(), Some(Component::Normal(part)) if part == file)
        && parts.next().is_none()
        && Path::new(file)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("osu"))
}

pub(crate) fn existing_samples(
    project: &Project,
    project_dir: &Path,
) -> BTreeMap<String, (String, SampleName)> {
    let ours = Manifest::load(project_dir).owned(&project.mapset_dir);
    let taking_over =
        project.export.write_mode == WriteMode::Direct && project.export.take_over_samples;
    let mut samples = BTreeMap::new();
    if let Ok(listing) = std::fs::read_dir(&project.mapset_dir) {
        for entry in listing.flatten() {
            let Ok(file) = entry.file_name().into_string() else {
                continue;
            };
            if ours.contains(&file.to_ascii_lowercase())
                || !entry.file_type().is_ok_and(|t| t.is_file())
            {
                continue;
            }
            if let Some(name) =
                parse_sample_name(&file).filter(|name| !taking_over || !name.is_hit())
            {
                samples.insert(name.stem.clone(), (file, name));
            }
        }
    }
    samples
}

fn expected_sounds(r: &packing::ResolvedPackage, index: u32) -> Vec<Playing> {
    let mut out = vec![Playing {
        set: r.normal_set,
        sound: Hitsound::Normal,
        index,
        volume: r.volume,
    }];
    for (on, sound) in [
        (r.whistle, Hitsound::Whistle),
        (r.finish, Hitsound::Finish),
        (r.clap, Hitsound::Clap),
    ] {
        if on {
            out.push(Playing {
                set: r.addition_set,
                sound,
                index,
                volume: r.volume,
            });
        }
    }
    out
}

/// Plans an export of every enabled difficulty. Reads the mapset, writes nothing.
pub fn plan(project: &Project, project_dir: &Path, library: &mut SampleLibrary) -> ExportPlan {
    let settings = &project.export;
    let mapset = &project.mapset_dir;
    let mut out = ExportPlan {
        mode: settings.write_mode,
        mapset_dir: mapset.clone(),
        diffs: Vec::new(),
        samples: Vec::new(),
        stale: Vec::new(),
        taken_over: Vec::new(),
        schema: Vec::new(),
        problems: Vec::new(),
        warnings: Vec::new(),
        inputs: BTreeMap::new(),
        retained: Vec::new(),
        sliders: slider::History::default(),
    };

    // What is in the mapset, and which of it is ours.
    let manifest = Manifest::load(project_dir);
    let previous_sliders = if manifest.applies_to(mapset) {
        manifest.sliders.clone()
    } else {
        slider::History::default()
    };
    out.sliders = previous_sliders.clone();
    let ours = manifest.owned(mapset);
    out.inputs = match crate::write::snapshot(mapset, |file| {
        osu_file(file) || parse_sample_name(file).is_some()
    }) {
        Ok(inputs) => inputs,
        Err(e) => {
            out.problems.push(format!(
                "cannot read the mapset folder {}: {e}",
                mapset.display()
            ));
            return out;
        }
    };
    let direct = settings.write_mode == WriteMode::Direct;
    let foreign: Vec<Foreign> = out
        .inputs
        .keys()
        .filter(|f| direct && !ours.contains(&f.to_ascii_lowercase()))
        .filter_map(|f| {
            parse_sample_name(f).map(|name| Foreign {
                file: f.clone(),
                name,
            })
        })
        .collect();
    let take_over = settings.take_over_samples && direct;
    let enabled: BTreeSet<_> = project
        .diffs
        .iter()
        .filter(|d| d.enabled)
        .map(|d| d.file.as_str())
        .collect();
    // Keep generated samples still in use in difficulties outside this write.
    let unchanged: Vec<_> = out
        .inputs
        .keys()
        .filter(|file| osu_file(file) && !enabled.contains(file.as_str()))
        .collect();
    let mut retained_indices = BTreeSet::new();
    let mut protected_audio = BTreeSet::new();
    for file in &unchanged {
        let parsed = std::fs::read_to_string(mapset.join(file))
            .ok()
            .and_then(|text| {
                let doc = OsuDocument::parse(&text);
                protected_audio.extend(audio_filename(&doc));
                Some((doc.timing_points().ok()?, doc.hit_objects().ok()?))
            });
        if direct && !take_over {
            if let Some((points, objects)) = parsed {
                retained_indices.extend(points.iter().map(|t| t.sample_index));
                retained_indices.extend(objects.iter().map(|o| o.hit_sample.index));
            } else {
                retained_indices.extend(
                    out.inputs
                        .keys()
                        .filter_map(|f| parse_sample_name(f).map(|n| n.index)),
                );
                out.warnings.push(format!("Could not read {file} to determine sample usage. Existing indices are preserved."));
            }
        }
    }
    // Index numbers that already mean something in this mapset are never reused (D-38).
    let mut reserved = retained_indices.clone();
    if !take_over {
        reserved.extend(foreign.iter().map(|f| f.name.index));
    }

    // Project and pack every enabled difficulty.
    let mut prepared: Vec<Prepared<'_>> = Vec::new();
    for profile in project.diffs.iter().filter(|d| d.enabled) {
        let file = &profile.file;
        if !osu_file(&profile.file) || !out.inputs.contains_key(&profile.file) {
            out.problems.push(format!(
                "{} is not a readable difficulty in the mapset",
                profile.file
            ));
            continue;
        }
        let original = match std::fs::read(mapset.join(&profile.file)).map(String::from_utf8) {
            Ok(Ok(text)) => text,
            Ok(Err(_)) => {
                out.problems
                    .push(format!("{}: not valid UTF-8", profile.file));
                continue;
            }
            Err(e) => {
                out.problems.push(format!("{}: {e}", profile.file));
                continue;
            }
        };
        let doc = OsuDocument::parse(&original);
        if doc.format_version().is_none() {
            out.problems
                .push(format!("{file}: missing osu! file format header"));
            continue;
        }
        if !matches!(doc.value("General", "Mode"), None | Some("0") | Some("2")) {
            out.problems.push(format!(
                "{}: only osu!standard and catch difficulties can be exported (D-04)",
                profile.file
            ));
            continue;
        }
        match DiffTimeline::from_document(&doc) {
            Ok(timeline) => {
                protected_audio.extend(audio_filename(&doc));
                let plan = DiffPlan::for_diff(project, profile, timeline);
                if direct
                    && doc.format_version().is_some_and(|version| version < 14)
                    && !plan.bodies.is_empty()
                {
                    out.problems.push(format!(
                        "{file}: slider body overrides require the v14 beatmap format"
                    ));
                }
                prepared.push(Prepared {
                    profile,
                    original,
                    doc,
                    plan,
                });
            }
            Err(e) => out.problems.push(format!("{}: {e}", profile.file)),
        }
    }
    if project.diffs.iter().all(|d| !d.enabled) {
        out.problems
            .push("no difficulty is enabled for export".into());
    }
    let protected_audio: BTreeSet<_> = protected_audio
        .iter()
        .filter_map(|path| path.to_str())
        .filter(|name| parse_sample_name(name).is_some())
        .map(str::to_ascii_lowercase)
        .collect();
    reserved.extend(
        protected_audio
            .iter()
            .filter_map(|name| parse_sample_name(name).map(|n| n.index)),
    );

    // One custom-index schema for the whole mapset (findings 8).
    let schema = build_schema(
        prepared.iter().map(|p| &p.plan),
        &project.schema,
        settings,
        &reserved,
    );

    // Files the export needs, by name, with the content of each.
    // Keyed by file stem (`soft-hitclap2`). The extension follows the export format and
    // is chosen when the file is rendered (D-69).
    let mut needed: BTreeMap<String, BTreeSet<SourceMix>> = BTreeMap::new();
    let mut blanks: BTreeSet<String> = BTreeSet::new();
    let mut used_indices: BTreeSet<u32> = BTreeSet::new();

    for p in &prepared {
        let file = &p.profile.file;
        let indices = match packing::assign(&p.plan.resolved, &schema) {
            Ok(i) => i,
            Err(e) => {
                out.problems.push(format!("{file}: {e}"));
                continue;
            }
        };
        used_indices.extend(indices.iter().copied());
        let hits = p.plan.written_hits(Some(p.profile), &indices);

        let (mut objects, mut points) = match (p.doc.hit_objects(), p.doc.timing_points()) {
            (Ok(o), Ok(t)) => (o, t),
            (Err(e), _) | (_, Err(e)) => {
                out.problems.push(format!("{file}: {e}"));
                continue;
            }
        };
        let before: Vec<String> = objects.iter().map(|o| o.serialize()).collect();
        let greenlines_before = points.iter().filter(|t| !t.uninherited).count();
        previous_sliders.restore(file, &mut objects, &mut points);
        let body_fields: Vec<_> = objects.iter().map(slider::BodyFields::read).collect();
        let general_set = general_sample_set(&p.doc);
        apply_hits(&p.plan.timeline, &mut objects, &mut points, &hits, &schema, general_set);
        let expected_edges = edge_sounds(&p.plan.timeline, &objects, &points, general_set);
        if direct {
            slider::apply(
                &p.plan.timeline,
                &mut objects,
                &mut points,
                &p.plan.bodies,
                &p.plan.resolved,
                &indices,
            );
            out.sliders.remember_bodies(file, &body_fields, &objects);
        } else if !p.plan.bodies.is_empty() {
            out.warnings.push(format!("{file}: hitsound-only circles carry edge and tick hitsounds. Slider body settings apply to direct export."));
        }
        // D-80: drop greenlines that change nothing. The verification below reads the
        // cleaned text back, so it checks that every edge still sounds the same.
        let greenlines_removed = if settings.remove_redundant_greenlines {
            osu_format::timing::remove_redundant(&mut points)
        } else {
            0
        };
        let objects_changed = objects
            .iter()
            .zip(&before)
            .filter(|(o, b)| &o.serialize() != *b)
            .count();
        let greenlines_after = points.iter().filter(|t| !t.uninherited).count();

        let mut new_doc = p.doc.clone();
        new_doc.set_timing_points(&points);
        new_doc.set_hit_objects(&objects);
        let direct_text = new_doc.serialize();

        // Verify: read the written text back and resolve every edge with stable's rules.
        let check = OsuDocument::parse(&direct_text);
        let (sounds, written_objects, written_points) = match (
            DiffTimeline::from_document(&check),
            check.hit_objects(),
            check.timing_points(),
        ) {
            (Ok(tl), Ok(o), Ok(t))
                if tl.edges == p.plan.timeline.edges && tl.ticks == p.plan.timeline.ticks =>
            {
                (edge_sounds(&tl, &o, &t, general_set), o, t)
            }
            _ => {
                out.problems.push(format!(
                    "{file}: the written difficulty could not be read back"
                ));
                continue;
            }
        };
        if let Some(edge) = sounds
            .iter()
            .zip(expected_edges)
            .position(|(now, before)| *now != before)
        {
            out.problems.push(format!(
                "{file}: slider changes interfere with the hit at {} ms",
                p.plan.timeline.edges[edge].time
            ));
        }
        let mut mismatches = 0;
        for (r, &index) in p.plan.resolved.iter().zip(&indices) {
            let Target::Edge(edge) = r.target else {
                continue;
            };
            if sounds[edge] != expected_sounds(r, index) {
                mismatches += 1;
                if mismatches <= 3 {
                    out.problems.push(format!(
                        "{file}: verification failed at {} ms: the written hit does not resolve to the intended sounds",
                        r.time
                    ));
                }
            }
        }
        if mismatches > 3 {
            out.problems.push(format!(
                "{file}: ...and {} more verification failures",
                mismatches - 3
            ));
        }
        for hit in &hits {
            let Some(sound) = sounds[hit.edge].first() else {
                continue;
            };
            if hit.index.is_some_and(|index| sound.index != index)
                || hit.volume.is_some_and(|volume| sound.volume != volume)
            {
                out.problems.push(format!(
                    "{file}: neighboring timing changes conflict at {} ms",
                    p.plan.timeline.edges[hit.edge].time
                ));
                break;
            }
        }

        // Every sound that plays decides which files must exist.
        for playing in sounds.iter().flatten() {
            if playing.index == 0 {
                continue;
            }
            let Some(slot) = sample_file_stem(playing.set, playing.sound, 1) else {
                continue;
            };
            let mixes = schema
                .iter()
                .find(|c| c.index == Some(playing.index))
                .and_then(|c| c.slots.get(&slot));
            if let Some(mixes) =
                mixes.filter(|m| !m.is_empty() && CustomIndex::needs_file(&slot, m))
                && let Some(stem) = sample_file_stem(playing.set, playing.sound, playing.index)
            {
                needed.insert(stem, mixes.clone());
            }
        }

        let tl = &p.plan.timeline;
        let mut tick_circles = Vec::new();
        for (tick, hit, index) in slider::assigned_ticks(tl, &p.plan.resolved, &indices) {
            let expected = SliderPlaying {
                set: hit.normal_set,
                sound: SliderSound::Tick,
                index,
                volume: hit.volume,
            };
            let stem = if direct {
                let played = slider::tick_at(tick, &written_objects, &written_points, general_set);
                if played != expected {
                    out.problems.push(format!(
                        "{file}: custom tick at {} ms did not resolve to its assigned sample",
                        tick.time
                    ));
                    continue;
                }
                out.sliders.tick_indices.insert(index);
                expected.stem()
            } else {
                tick_circles.push((tick.time, expected));
                sample_file_stem(hit.normal_set, Hitsound::Normal, index)
            };
            if let Some(stem) = stem
                && let Some(mixes) = hit.requirement.slots.values().next()
            {
                needed.insert(stem, mixes.clone());
            }
        }
        let mut blank = |p: SliderPlaying| {
            if p.index != 0
                && let Some(stem) = p.stem()
            {
                blanks.insert(stem);
            }
        };
        if direct && settings.mute_slider_slides {
            for (_, _, sound) in slider::slides(tl, &written_objects, &written_points, general_set)
            {
                blank(sound);
            }
        }
        if direct && settings.mute_slider_ticks {
            for &tick in &tl.ticks {
                blank(slider::tick_at(
                    tick,
                    &written_objects,
                    &written_points,
                    general_set,
                ));
            }
        }

        let count = |kind: HitKind| {
            p.plan
                .projection
                .hits
                .iter()
                .filter(|h| h.kind == kind && matches!(h.target, Target::Edge(_)))
                .count()
        };
        let stats = DiffStats {
            edges: p.plan.timeline.edges.len(),
            matched: count(HitKind::Matched),
            fallback: count(HitKind::Fallback),
            muted: count(HitKind::Muted),
            empty: count(HitKind::Empty),
            orphaned: p
                .plan
                .projection
                .warnings
                .iter()
                .filter(|w| matches!(w, Warning::Orphaned { .. }))
                .count(),
            ticks: p
                .plan
                .resolved
                .iter()
                .filter(|r| matches!(r.target, Target::Tick(_)))
                .count(),
            slider_bodies: p.plan.bodies.len(),
            objects_changed,
            greenlines_before,
            greenlines_after,
            greenlines_removed,
        };
        let (out_file, updated) = match settings.write_mode {
            WriteMode::Direct => (file.clone(), direct_text),
            WriteMode::HitsoundOnly => {
                hitsound_only(&new_doc, &p.plan.timeline, &hits, &tick_circles, file)
            }
        };
        out.diffs.push(DiffExport {
            file: file.clone(),
            out_file,
            label: p.profile.label.clone(),
            changed: updated != p.original,
            original: p.original.clone(),
            updated,
            stats,
            warnings: p.plan.projection.warnings.clone(),
        });
    }

    out.schema = packing::prune_schema(schema, &used_indices);

    // Render the sample files.
    for (stem, mixes) in &needed {
        let silent = mixes.iter().all(|m| m.source == Source::Silent);
        if silent {
            out.samples.push(blank_sample(stem, "silence".into(), vec![]));
            continue;
        }
        let single = single_source(mixes, library);
        let format = if single.is_some() {
            settings.single_format
        } else {
            settings.mixed_format
        };
        match library.render(mixes) {
            Ok(mut buf) => {
                let fixes = if settings.auto_fix {
                    auto_fix(&mut buf)
                } else {
                    Vec::new()
                };
                if settings.auto_fix && buf.peak() == 0.0 {
                    out.samples.push(blank_sample(stem, describe(mixes), fixes));
                    continue;
                }
                match sample_file(stem, &buf, single, format, &fixes) {
                    Ok((name, bytes, issues, copied)) => out.samples.push(SampleFile {
                        name,
                        size: bytes.len(),
                        bytes,
                        content: if copied {
                            format!("{} (source file copied as is)", describe(mixes))
                        } else {
                            describe(mixes)
                        },
                        issues,
                        fixes,
                    }),
                    Err(e) => out.problems.push(format!("{stem}: {e}")),
                }
            }
            Err(e) => out.problems.push(format!("{stem}: {e}")),
        }
    }

    // Blank slider samples, unless a file of that name is already there.
    for stem in &blanks {
        let name = format!("{stem}.wav");
        if needed.contains_key(stem)
            || protected_audio.contains(&name.to_ascii_lowercase())
            || parse_sample_name(&name).is_some_and(|n| retained_indices.contains(&n.index))
        {
            continue;
        }
        if !take_over && foreign.iter().any(|f| f.name.stem == *stem) {
            continue;
        }
        out.samples.push(blank_sample(stem, "silence".into(), vec![]));
    }

    // Files that are not ours.
    let writing: BTreeSet<String> = out.samples.iter().map(|s| file_stem(&s.name)).collect();
    if take_over {
        // hitsounds-rs now owns the mapset's hit samples: any left behind would either
        // play instead of a skin sound we intend, or sit there unused.
        out.taken_over = foreign
            .iter()
            .filter(|f| !protected_audio.contains(&f.file.to_ascii_lowercase()))
            .filter(|f| f.name.is_hit() || writing.contains(&f.name.stem))
            .map(|f| f.file.clone())
            .collect();
        if !out.taken_over.is_empty() && !unchanged.is_empty() {
            out.warnings.push("taking over the mapset's sample files can change difficulties that are not enabled for export".into());
        }
    } else {
        for f in foreign.iter().filter(|f| writing.contains(&f.name.stem)) {
            out.problems.push(format!(
                "{} already exists in the mapset and was not written by hitsounds-rs",
                f.file
            ));
        }
        let hit_files = foreign.iter().filter(|f| f.name.is_hit()).count();
        if hit_files > 0 && settings.write_mode == WriteMode::Direct {
            let numbers: Vec<String> = reserved.iter().map(u32::to_string).collect();
            out.warnings.push(format!(
                "the mapset has {hit_files} hitsound files hitsounds-rs did not write; their index numbers ({}) are avoided. \
                 Files no difficulty uses any more must be removed before ranking, or turn on \"take over existing sample files\"",
                numbers.join(", ")
            ));
        }
    }
    if settings.write_mode == WriteMode::Direct {
        let new_names: BTreeSet<String> = out
            .samples
            .iter()
            .map(|s| s.name.to_ascii_lowercase())
            .collect();
        for file in out.inputs.keys().filter(|f| {
            let lower = f.to_ascii_lowercase();
            ours.contains(&lower) && !new_names.contains(&lower)
        }) {
            // A sample of ours rewritten in another format must go: osu! looks for
            // .wav before .mp3 and .ogg, so an old WAV would shadow a new OGG (D-69).
            let replaced = writing.contains(&file_stem(file));
            if protected_audio.contains(&file.to_ascii_lowercase())
                || (!replaced
                    && parse_sample_name(file).is_some_and(|n| retained_indices.contains(&n.index)))
            {
                out.retained.push(file.clone());
            } else {
                out.stale.push(file.clone());
            }
        }
        let retained_ticks: BTreeSet<_> = out
            .samples
            .iter()
            .map(|s| s.name.as_str())
            .chain(out.retained.iter().map(String::as_str))
            .filter_map(|file| {
                parse_sample_name(file)
                    .filter(|name| name.kind == "slidertick")
                    .map(|name| name.index)
            })
            .collect();
        out.sliders
            .tick_indices
            .retain(|i| retained_ticks.contains(i));
    }

    let late: Vec<&str> = out
        .samples
        .iter()
        .filter(|s| {
            s.issues
                .iter()
                .any(|i| matches!(i, SampleIssue::LatePeak { .. }))
        })
        .map(|s| s.name.as_str())
        .collect();
    if !late.is_empty() {
        out.warnings.push(format!(
            "{} sample file(s) reach their impact later than 5 ms (RC): {}",
            late.len(),
            late.join(", ")
        ));
    }
    out
}

/// Lower-cased file name without its extension.
fn file_stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_ascii_lowercase()
}

/// The 44-byte silent file, which the ranking criteria require as WAV in every format.
fn blank_sample(stem: &str, content: String, fixes: Vec<SampleFix>) -> SampleFile {
    let bytes = blank_wav();
    SampleFile {
        name: format!("{stem}.wav"),
        size: bytes.len(),
        bytes,
        content,
        issues: vec![],
        fixes,
    }
}

/// The one audible source of a slot, if it has exactly one: Mapping Tools exports such
/// a sample with the single-sample format, and copies it unchanged only at full gain
/// (`SampleGeneratingArgs.CanCopyPaste`).
struct Single {
    path: PathBuf,
    bytes: Vec<u8>,
    kind: SourceKind,
    full_gain: bool,
}

fn single_source(mixes: &BTreeSet<SourceMix>, library: &SampleLibrary) -> Option<Single> {
    let mut audible = mixes.iter().filter(|m| m.source != Source::Silent);
    let mix = audible.next()?;
    if audible.next().is_some() {
        return None;
    }
    let path = match &mix.source {
        Source::File { path } => library.resolve_path(path),
        Source::Skin { set, sound } => library.skin_file(*set, *sound).ok()?,
        Source::SkinSlider { .. } | Source::Silent => return None,
    };
    let bytes = std::fs::read(&path).ok()?;
    let kind = SourceKind::of(&path, &bytes);
    Some(Single { path, bytes, kind, full_gain: mix.gain_pct == 100 })
}

/// Picks the bytes and name of one sample file (D-69): the source copied unchanged
/// when it already is what `format` asks for and no auto-fix touched it, otherwise
/// `buf` encoded. An encoded OGG is decoded again so the ranking checks judge the
/// audio osu! will play, and a file that came back shifted or cut is refused.
fn sample_file(
    stem: &str,
    buf: &hs_audio::AudioBuffer,
    single: Option<Single>,
    format: SampleFormat,
    fixes: &[SampleFix],
) -> Result<(String, Vec<u8>, Vec<SampleIssue>, bool), String> {
    let mut format = format;
    if let Some(Single { path, bytes, kind, full_gain }) = single {
        if full_gain && fixes.is_empty() && kind.copies_as(format) {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("wav")
                .to_ascii_lowercase();
            let mut issues = check_sample(buf, false);
            too_small(&bytes, &mut issues);
            return Ok((format!("{stem}.{ext}"), bytes, issues, true));
        }
        if format == SampleFormat::Source {
            format = kind.source_format();
        }
    }
    let bytes = encode(buf, format)?;
    let ext = extension(format);
    let mut issues = if format == SampleFormat::Ogg {
        let decoded = hs_audio::decode_bytes(bytes.clone(), "ogg")
            .map_err(|e| format!("the encoded OGG does not decode: {e}"))?;
        let shift = onset_shift_ms(buf, &decoded);
        if decoded.frames().abs_diff(buf.frames()) > 1 || shift > 1.0 {
            return Err(format!(
                "the encoded OGG plays {} frames (expected {}) with its onset moved {shift:.1} ms",
                decoded.frames(),
                buf.frames()
            ));
        }
        check_sample(&decoded, false)
    } else {
        check_sample(buf, false)
    };
    too_small(&bytes, &mut issues);
    Ok((format!("{stem}.{ext}"), bytes, issues, false))
}

/// How far the first audible frame moved between two renderings of one sample.
fn onset_shift_ms(a: &hs_audio::AudioBuffer, b: &hs_audio::AudioBuffer) -> f64 {
    let onset = |x: &hs_audio::AudioBuffer| x.first_frame_above(x.peak() * 0.1).unwrap_or(0);
    onset(a).abs_diff(onset(b)) as f64 * 1000.0 / f64::from(a.sample_rate.max(1))
}

/// osu! loads a sample file under 1024 bytes as silence (findings §2).
fn too_small(bytes: &[u8], issues: &mut Vec<SampleIssue>) {
    if bytes.len() < crate::checks::MIN_AUDIBLE_BYTES {
        issues.push(SampleIssue::TooSmall { bytes: bytes.len() });
    }
}

/// A copy of the difficulty holding nothing but its timing and one circle per hit,
/// for mappers who copy hitsounds by hand (D-19).
fn hitsound_only(
    written: &OsuDocument,
    timeline: &DiffTimeline,
    hits: &[crate::WrittenHit],
    ticks: &[(hs_core::Ms, SliderPlaying)],
    file: &str,
) -> (String, String) {
    let mut doc = written.clone();
    let mut circles: Vec<_> = hits
        .iter()
        .map(|h| {
            let flags = u8::from(h.whistle) * 2 + u8::from(h.finish) * 4 + u8::from(h.clap) * 8;
            (
                timeline.edges[h.edge].time,
                flags,
                h.normal_set.index(),
                h.addition_set.index(),
                0,
                0,
            )
        })
        .chain(
            ticks
                .iter()
                .map(|(time, tick)| (*time, 0, tick.set.index(), 0, tick.index, tick.volume)),
        )
        .collect();
    circles.sort_by_key(|circle| circle.0);
    let lines = circles
        .iter()
        .enumerate()
        .map(|(i, &(time, flags, set, addition, index, volume))| {
            let kind = if i == 0 { 5 } else { 1 };
            format!("256,192,{time},{kind},{flags},{set}:{addition}:{index}:{volume}:")
        })
        .collect();
    if let Some(section) = doc.section_mut("HitObjects") {
        section.replace_lines(lines);
    }
    if let Some(section) = doc.section_mut("Events") {
        section.replace_lines(Vec::new());
    }
    if let Some(section) = doc.section_mut("Metadata") {
        for line in &mut section.lines {
            if let Some(version) = line.strip_prefix("Version:") {
                *line = format!("Version:{} (hitsounds)", version.trim_end());
            } else if line.starts_with("BeatmapID:") {
                *line = "BeatmapID:0".into();
            }
        }
    }
    let out_file = match file.rsplit_once('.') {
        Some((stem, ext)) => match stem.rsplit_once(']') {
            Some((head, tail)) => format!("{head} (hitsounds)]{tail}.{ext}"),
            None => format!("{stem} (hitsounds).{ext}"),
        },
        None => format!("{file} (hitsounds)"),
    };
    (out_file, doc.serialize())
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

/// Writes a plan. The caller stores `plan.schema` in the project afterwards.
pub fn execute(plan: &ExportPlan, project_dir: &Path) -> Result<ExportReport, ExportError> {
    if !plan.problems.is_empty() {
        return Err(ExportError::Blocked(plan.problems.join("; ")));
    }
    if project_dir
        .canonicalize()?
        .starts_with(plan.mapset_dir.canonicalize()?)
    {
        return Err(ExportError::Blocked(
            "the project folder must be outside the mapset".into(),
        ));
    }
    check_inputs(plan)?;
    match plan.mode {
        WriteMode::Direct => execute_direct(plan, project_dir),
        WriteMode::HitsoundOnly => execute_hitsound_only(plan, project_dir),
    }
}

fn check_inputs(plan: &ExportPlan) -> Result<(), ExportError> {
    let mapset = &plan.mapset_dir;
    let now = snapshot(mapset, |file| {
        osu_file(file) || parse_sample_name(file).is_some()
    })?;
    if now != plan.inputs {
        let changed = now
            .keys()
            .chain(plan.inputs.keys())
            .find(|file| now.get(*file) != plan.inputs.get(*file));
        return Err(ExportError::Changed(
            changed.cloned().unwrap_or_else(|| "the mapset".into()),
        ));
    }
    for d in &plan.diffs {
        let now = std::fs::read(mapset.join(&d.file))?;
        if now != d.original.as_bytes() {
            return Err(ExportError::Changed(d.file.clone()));
        }
    }
    Ok(())
}

fn execute_direct(plan: &ExportPlan, project_dir: &Path) -> Result<ExportReport, ExportError> {
    let mapset = &plan.mapset_dir;
    let changed: Vec<&DiffExport> = plan.diffs.iter().filter(|d| d.changed).collect();
    let mut changes = Vec::new();
    let actual: BTreeMap<_, _> = plan
        .inputs
        .keys()
        .map(|file| (file.to_ascii_lowercase(), file))
        .collect();
    let replacing: BTreeSet<_> = plan
        .samples
        .iter()
        .map(|sample| sample.name.to_ascii_lowercase())
        .collect();
    let mut removed = 0;
    for file in plan.taken_over.iter().chain(&plan.stale) {
        if !replacing.contains(&file.to_ascii_lowercase()) {
            changes.push(Change::prepare(mapset.join(file), None)?);
            removed += 1;
        }
    }
    for sample in &plan.samples {
        let name = actual
            .get(&sample.name.to_ascii_lowercase())
            .copied()
            .unwrap_or(&sample.name);
        changes.push(Change::prepare(mapset.join(name), Some(&sample.bytes))?);
    }
    for d in &changed {
        changes.push(Change::prepare(
            mapset.join(&d.file),
            Some(d.updated.as_bytes()),
        )?);
    }

    // All replacements are staged before the first write. Check again after staging
    // so changes while preparing backups never enter the write transaction.
    let mut backup_dir = None;
    if changes.iter().any(Change::replaces_file) {
        let root = project_dir.join("backups");
        std::fs::create_dir_all(&root)?;
        let backup = tempfile::Builder::new()
            .prefix(&format!("{}-", unix_ms()))
            .tempdir_in(root)?;
        for change in &changes {
            change.backup(backup.path())?;
        }
        backup_dir = Some(backup.keep());
    }
    let manifest = Manifest {
        files: plan
            .samples
            .iter()
            .map(|s| s.name.clone())
            .chain(plan.retained.iter().cloned())
            .collect(),
        mapset_dir: Some(mapset.clone()),
        sliders: plan.sliders.clone(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(std::io::Error::other)?;
    let path = Manifest::path(project_dir);
    std::fs::create_dir_all(path.parent().expect("manifest has a parent folder"))?;
    changes.push(Change::prepare(path, Some(&manifest_bytes))?);
    check_inputs(plan)?;
    commit(&mut changes, backup_dir.as_deref())?;

    Ok(ExportReport {
        out_dir: mapset.clone(),
        backup_dir,
        diffs_written: changed.len(),
        samples_written: plan.samples.len(),
        files_removed: removed,
    })
}

fn execute_hitsound_only(
    plan: &ExportPlan,
    project_dir: &Path,
) -> Result<ExportReport, ExportError> {
    let root = project_dir.join("exports").join("hitsound-only");
    std::fs::create_dir_all(&root)?;
    let staged = tempfile::Builder::new()
        .prefix(".export-")
        .tempdir_in(&root)?;
    for s in &plan.samples {
        write_atomic(&staged.path().join(&s.name), &s.bytes)?;
    }
    let mut audio = BTreeSet::new();
    for d in &plan.diffs {
        write_atomic(&staged.path().join(&d.out_file), d.updated.as_bytes())?;
        if let Some(relative) = audio_filename(&OsuDocument::parse(&d.original)) {
            audio.insert(relative);
        }
    }
    let mapset = plan.mapset_dir.canonicalize()?;
    for name in audio {
        let Ok(source) = mapset.join(&name).canonicalize() else {
            continue;
        };
        if !source.starts_with(&mapset) || !source.is_file() {
            continue;
        }
        let target = staged.path().join(name);
        std::fs::create_dir_all(target.parent().expect("audio has a parent folder"))?;
        let mut output = std::fs::File::create_new(target)?;
        std::io::copy(&mut std::fs::File::open(source)?, &mut output)?;
    }
    let name = staged
        .path()
        .file_name()
        .expect("staging folder has a name")
        .to_string_lossy();
    let dir = root.join(format!(
        "{}{}",
        unix_ms(),
        name.trim_start_matches(".export")
    ));
    std::fs::rename(staged.path(), &dir)?;
    Ok(ExportReport {
        out_dir: dir,
        backup_dir: None,
        diffs_written: plan.diffs.len(),
        samples_written: plan.samples.len(),
        files_removed: 0,
    })
}

fn audio_filename(doc: &OsuDocument) -> Option<PathBuf> {
    let value = doc.value("General", "AudioFilename")?.trim_matches('"');
    let path = Path::new(value);
    (!value.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_))))
        .then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::encode_pcm16;
    use hs_audio::AudioBuffer;
    use hs_core::{Channel, Override, SampleSource, TickPolicy};
    use osu_format::SampleSet;
    use std::fs;

    const MAP: &str = "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.wav\r\nMode: 0\r\nSampleSet: Soft\r\n\r\n[Metadata]\r\nVersion: Hard\r\n\r\n[Difficulty]\r\nSliderMultiplier:1\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,2,0,75,1,0\r\n// keep this comment\r\n2000,-100,4,2,0,75,0,1\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n256,192,2000,2,0,L|456:192,1,200,0|0,0:0|0:0,0:0:0:0:\r\n256,192,4500,1,0,0:0:0:0:\r\n";

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        project: Project,
        library: SampleLibrary,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            let mapset = temp.path().join("mapset");
            fs::create_dir(&root).unwrap();
            fs::create_dir(&mapset).unwrap();
            fs::write(mapset.join("Hard.osu"), MAP).unwrap();
            let sample = encode_pcm16(&AudioBuffer {
                sample_rate: 44_100,
                channels: 1,
                samples: vec![0.125; 400],
            });
            fs::write(mapset.join("audio.wav"), &sample).unwrap();
            fs::write(mapset.join("drum-hitfinish.ogg"), b"foreign sample").unwrap();
            fs::write(root.join("kick.wav"), &sample).unwrap();
            let mut project = Project::new("Fixture", mapset);
            let mut kick = Channel::new(0, "Kick", SampleSet::Drum, Hitsound::Normal);
            kick.sample = SampleSource::File {
                path: "kick.wav".into(),
            };
            kick.add_event(1000);
            kick.add_event(2500);
            project.add_channel(kick);
            let mut diff = DiffProfile::new("Hard.osu", "Hard");
            diff.tick_policy = TickPolicy::Render;
            diff.overrides.push(Override::SliderBody {
                slider_time: 2000,
                sample_set: SampleSet::Drum,
                whistle: true,
            });
            project.diffs.push(diff);
            let library = SampleLibrary::new(&root, None);
            Self {
                _temp: temp,
                root,
                project,
                library,
            }
        }

        fn plan(&mut self) -> ExportPlan {
            plan(&self.project, &self.root, &mut self.library)
        }

        fn map(&self) -> PathBuf {
            self.project.mapset_dir.join("Hard.osu")
        }
    }

    #[test]
    fn direct_export_writes_and_verifies_ticks_bodies_and_backups() {
        let mut fixture = Fixture::new();
        fixture.project.export.mute_slider_ticks = true;
        let plan = fixture.plan();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert_eq!(fs::read_to_string(fixture.map()).unwrap(), MAP);
        assert_eq!(plan.diffs[0].stats.ticks, 1);
        assert_eq!(plan.diffs[0].stats.slider_bodies, 1);
        assert!(
            plan.samples
                .iter()
                .any(|s| s.name == "drum-hitnormal2.wav" && !s.fixes.is_empty())
        );
        assert!(
            plan.samples
                .iter()
                .any(|s| s.name == "drum-slidertick100.wav" && s.size > 44)
        );
        let report = execute(&plan, &fixture.root).unwrap();
        let written = fs::read_to_string(fixture.map()).unwrap();
        assert_eq!(written, plan.diffs[0].updated);
        assert!(written.contains("// keep this comment\r\n"));
        assert!(written.contains("2500,-100,4,3,100,100,0,1\r\n"));
        assert!(written.contains("2505,-100,4,2,0,75,0,1\r\n"));
        let doc = OsuDocument::parse(&written);
        let objects = doc.hit_objects().unwrap();
        assert!(objects[1].hitsound.has(Hitsound::Whistle));
        assert!(!objects[1].edge_sounds[0].has(Hitsound::Whistle));
        assert_eq!(
            fs::read_to_string(report.backup_dir.unwrap().join("Hard.osu")).unwrap(),
            MAP
        );
        assert_eq!(
            fs::read(fixture.project.mapset_dir.join("drum-hitfinish.ogg")).unwrap(),
            b"foreign sample"
        );

        fixture.project.schema = plan.schema;
        let unchanged = fixture.plan();
        assert!(!unchanged.diffs[0].changed);
        fixture.project.channels[0].remove_event(2500);
        fixture.project.diffs[0].overrides.clear();
        let updated = fixture.plan();
        assert!(updated.stale.contains(&"drum-slidertick100.wav".into()));
        assert!(
            !OsuDocument::parse(&updated.diffs[0].updated)
                .hit_objects()
                .unwrap()[1]
                .hitsound
                .has(Hitsound::Whistle)
        );
        execute(&updated, &fixture.root).unwrap();
        assert!(
            !fixture
                .project
                .mapset_dir
                .join("drum-slidertick100.wav")
                .exists()
        );
    }

    #[test]
    fn redundant_greenlines_are_removed_only_when_asked() {
        // D-80: the mapper's 3000 ms greenline repeats the 2000 ms one exactly.
        let mut fixture = Fixture::new();
        let map = MAP.replace(
            "2000,-100,4,2,0,75,0,1\r\n",
            "2000,-100,4,2,0,75,0,1\r\n3000,-100,4,2,0,75,0,1\r\n",
        );
        fs::write(fixture.map(), &map).unwrap();
        let plan = fixture.plan();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert!(plan.diffs[0].stats.greenlines_removed >= 1);
        assert!(!plan.diffs[0].updated.contains("\r\n3000,-100,"));
        assert!(plan.diffs[0].updated.contains("// keep this comment\r\n"));

        fixture.project.export.remove_redundant_greenlines = false;
        let kept = fixture.plan();
        assert!(kept.problems.is_empty(), "{:?}", kept.problems);
        assert_eq!(kept.diffs[0].stats.greenlines_removed, 0);
        assert!(kept.diffs[0].updated.contains("\r\n3000,-100,4,2,0,75,0,1\r\n"));
    }

    #[test]
    fn stale_preview_rejects_new_mapset_sample_before_mutating_anything() {
        let mut fixture = Fixture::new();
        let plan = fixture.plan();
        let new = fixture.project.mapset_dir.join("normal-hitclap5.wav");
        fs::write(&new, b"new user's sample").unwrap();
        assert!(matches!(
            execute(&plan, &fixture.root),
            Err(ExportError::Changed(_))
        ));
        assert_eq!(fs::read_to_string(fixture.map()).unwrap(), MAP);
        assert_eq!(fs::read(new).unwrap(), b"new user's sample");
    }

    #[test]
    fn hitsound_only_export_isolated_and_includes_song_and_tick_circle() {
        let mut fixture = Fixture::new();
        fixture.project.export.write_mode = WriteMode::HitsoundOnly;
        let plan = fixture.plan();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let report = execute(&plan, &fixture.root).unwrap();
        let output = report.out_dir.join(&plan.diffs[0].out_file);
        let doc = OsuDocument::parse(&fs::read_to_string(output).unwrap());
        let circle = doc
            .hit_objects()
            .unwrap()
            .into_iter()
            .find(|object| object.time == 2500)
            .unwrap();
        assert_eq!(circle.kind, osu_format::HitObjectKind::Circle);
        assert_eq!(circle.hit_sample.index, 100);
        assert!(report.out_dir.join("drum-hitnormal100.wav").is_file());
        assert!(report.out_dir.join("audio.wav").is_file());
        assert_eq!(fs::read_to_string(fixture.map()).unwrap(), MAP);
        let next = execute(&plan, &fixture.root).unwrap();
        assert_ne!(next.out_dir, report.out_dir);
        assert!(report.out_dir.is_dir());
    }

    #[test]
    fn hierarchy_export_matches_mapping_tools_reference() {
        // Mapping Tools 1.12.20 Hitsound Studio, with the same synthetic layers.
        // Compare the files and playback fields. Each writer formats timing differently.
        let mut fixture = Fixture::new();
        fs::remove_file(fixture.project.mapset_dir.join("drum-hitfinish.ogg")).unwrap();
        let (header, _) = MAP.split_once("[HitObjects]").unwrap();
        let times = [1000, 1500, 2000, 3000];
        let circles: String = times
            .iter()
            .map(|t| format!("256,192,{t},1,0,0:0:0:0:\r\n"))
            .collect();
        fs::write(fixture.map(), format!("{header}[HitObjects]\r\n{circles}")).unwrap();
        let kick = &mut fixture.project.channels[0];
        kick.events = times.to_vec();
        kick.volume = 0.75;
        fixture.project.diffs[0].overrides.clear();
        fixture.project.export.auto_fix = false;

        for (path, set, sound, events) in [
            ("pad.wav", SampleSet::Soft, Hitsound::Normal, [1003, 2000]),
            ("clap.wav", SampleSet::Soft, Hitsound::Clap, [1000, 2000]),
            (
                "whistle.wav",
                SampleSet::Normal,
                Hitsound::Whistle,
                [1000, 2000],
            ),
        ] {
            fs::copy(fixture.root.join("kick.wav"), fixture.root.join(path)).unwrap();
            let mut channel = Channel::new(0, path, set, sound);
            channel.sample = SampleSource::File { path: path.into() };
            channel.events = events.to_vec();
            channel.volume = 0.75;
            fixture.project.add_channel(channel);
        }

        let plan = fixture.plan();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let samples: Vec<_> = plan.samples.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(
            samples,
            [
                "drum-hitnormal.wav",
                "drum-hitnormal2.wav",
                "soft-hitclap.wav",
                "soft-hitwhistle.wav"
            ]
        );
        let doc = OsuDocument::parse(&plan.diffs[0].updated);
        let timeline = DiffTimeline::from_document(&doc).unwrap();
        let resolved = edge_sounds(
            &timeline,
            &doc.hit_objects().unwrap(),
            &doc.timing_points().unwrap(),
            SampleSet::Soft,
        );
        for (i, sounds) in resolved.iter().enumerate() {
            let mixed = i % 2 == 0;
            let normal = Playing {
                set: SampleSet::Drum,
                sound: Hitsound::Normal,
                index: if mixed { 1 } else { 2 },
                volume: 75,
            };
            let expected = if mixed {
                vec![
                    normal,
                    Playing {
                        set: SampleSet::Soft,
                        sound: Hitsound::Whistle,
                        index: 1,
                        volume: 75,
                    },
                    Playing {
                        set: SampleSet::Soft,
                        sound: Hitsound::Clap,
                        index: 1,
                        volume: 75,
                    },
                ]
            } else {
                vec![normal]
            };
            assert_eq!(*sounds, expected, "at {} ms", times[i]);
        }
    }

    /// A 100 ms drum-like hit that needs no auto-fix, so it can be copied as is.
    fn tone() -> AudioBuffer {
        let samples = (0..4_410)
            .map(|i| {
                let t = i as f32 / 44_100.0;
                0.8 * (-t * 40.0).exp() * (2.0 * std::f32::consts::PI * 180.0 * t).sin()
            })
            .collect();
        AudioBuffer { sample_rate: 44_100, channels: 1, samples }
    }

    fn by_ext<'a>(plan: &'a ExportPlan, ext: &str) -> Vec<&'a SampleFile> {
        plan.samples.iter().filter(|s| s.name.ends_with(ext)).collect()
    }

    #[test]
    fn ogg_export_writes_vorbis_and_replaces_the_previous_wav() {
        let mut fixture = Fixture::new();
        fs::write(fixture.root.join("kick.wav"), encode_pcm16(&tone())).unwrap();
        let wav = fixture.plan();
        assert!(wav.problems.is_empty(), "{:?}", wav.problems);
        assert!(by_ext(&wav, ".ogg").is_empty(), "16-bit WAV stays the default");
        execute(&wav, &fixture.root).unwrap();
        fixture.project.schema = wav.schema.clone();

        fixture.project.export.single_format = SampleFormat::Ogg;
        fixture.project.export.mixed_format = SampleFormat::Ogg;
        let ogg = fixture.plan();
        assert!(ogg.problems.is_empty(), "{:?}", ogg.problems);
        let encoded = by_ext(&ogg, ".ogg");
        assert!(!encoded.is_empty());
        for sample in &encoded {
            assert_eq!(&sample.bytes[..4], b"OggS");
            assert!(sample.size >= crate::checks::MIN_AUDIBLE_BYTES);
            let decoded = hs_audio::decode_bytes(sample.bytes.clone(), "ogg").unwrap();
            assert!(decoded.peak() > 0.1, "{}", sample.name);
        }
        // Blanks stay the 44-byte WAV in every format.
        assert!(by_ext(&ogg, ".wav").iter().all(|s| s.size == 44));
        // Every WAV the first export wrote under a stem that is now OGG is removed, so it
        // cannot shadow the OGG (osu! looks for .wav first).
        for sample in by_ext(&wav, ".wav").into_iter().filter(|s| s.size > 44) {
            let stem = file_stem(&sample.name);
            assert!(encoded.iter().any(|s| file_stem(&s.name) == stem), "{stem} was rewritten");
            assert!(ogg.stale.contains(&sample.name), "{} is stale", sample.name);
        }
        execute(&ogg, &fixture.root).unwrap();
        for sample in &encoded {
            let stem = file_stem(&sample.name);
            assert!(fixture.project.mapset_dir.join(&sample.name).is_file());
            assert!(!fixture.project.mapset_dir.join(format!("{stem}.wav")).exists());
        }
    }

    #[test]
    fn same_as_source_copies_an_unchanged_ogg_and_re_encodes_otherwise() {
        let mut fixture = Fixture::new();
        fixture.project.diffs[0].overrides.clear();
        fixture.project.diffs[0].tick_policy = TickPolicy::Ignore;
        let source = crate::encode::encode_vorbis(&tone()).unwrap();
        fs::write(fixture.root.join("kick.ogg"), &source).unwrap();
        fixture.project.channels[0].sample = SampleSource::File { path: "kick.ogg".into() };
        fixture.project.export.single_format = SampleFormat::Source;
        let plan = fixture.plan();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let copied = by_ext(&plan, ".ogg");
        assert_eq!(copied.len(), 1);
        assert_eq!(copied[0].bytes, source, "copied byte for byte");
        assert!(copied[0].content.ends_with("(source file copied as is)"));

        // Asking for 16-bit WAV re-encodes the same source.
        fixture.project.export.single_format = SampleFormat::Wav16;
        let plan = fixture.plan();
        let wav = by_ext(&plan, ".wav");
        assert_eq!(wav.len(), 1);
        assert_eq!(crate::wav::wav_encoding(&wav[0].bytes), Some((1, 16)));

        // Channel volume goes to the greenline (D-44), not into the file: still a copy.
        fixture.project.channels[0].volume = 0.5;
        fixture.project.export.single_format = SampleFormat::Source;
        assert_eq!(by_ext(&fixture.plan(), ".ogg")[0].bytes, source);

        // An auto-fix (here: 20 ms of leading silence trimmed) changes the audio, so it
        // is re-encoded, and an OGG source stays OGG.
        let mut late = tone();
        late.samples.splice(0..0, std::iter::repeat_n(0.0, 882));
        fs::write(fixture.root.join("kick.ogg"), crate::encode::encode_vorbis(&late).unwrap()).unwrap();
        fixture.library.clear();
        let plan = fixture.plan();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let encoded = by_ext(&plan, ".ogg");
        assert_eq!(encoded.len(), 1);
        assert!(!encoded[0].fixes.is_empty());
        assert!(!encoded[0].content.contains("copied"));
        let decoded = hs_audio::decode_bytes(encoded[0].bytes.clone(), "ogg").unwrap();
        assert!(crate::checks::impact_delay_ms(&decoded) <= crate::checks::MAX_PEAK_DELAY_MS);
    }

    #[test]
    fn a_single_source_below_full_gain_uses_the_single_format_but_is_not_copied() {
        let mut fixture = Fixture::new();
        fixture.project.diffs[0].overrides.clear();
        fixture.project.diffs[0].tick_policy = TickPolicy::Ignore;
        fs::write(fixture.root.join("kick.wav"), encode_pcm16(&tone())).unwrap();
        fs::write(fixture.root.join("clap.ogg"), crate::encode::encode_vorbis(&tone()).unwrap()).unwrap();
        // A quieter clap on the kick's hit: its slot holds one source at 50 % of the
        // hit's greenline volume (the kick sets that volume).
        let mut clap = Channel::new(0, "Clap", SampleSet::Soft, Hitsound::Clap);
        clap.sample = SampleSource::File { path: "clap.ogg".into() };
        clap.volume = 0.5;
        clap.add_event(1000);
        fixture.project.add_channel(clap);
        fixture.project.export.single_format = SampleFormat::Source;
        fixture.project.export.mixed_format = SampleFormat::Wav16;
        let plan = fixture.plan();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let clap = plan.samples.iter().find(|s| s.name.starts_with("soft-hitclap")).unwrap();
        assert!(clap.content.contains("50%"), "{}", clap.content);
        assert!(clap.name.ends_with(".ogg"), "single source, so the OGG source keeps OGG: {}", clap.name);
        assert!(!clap.content.contains("copied"), "its gain is baked in, so it is re-encoded");
    }
}
