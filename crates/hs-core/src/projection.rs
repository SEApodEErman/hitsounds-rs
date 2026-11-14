//! Projection: base layer + diff overrides, intersected with one difficulty's objects.
//!
//! The base layer covers the whole song; a difficulty can only play hitsounds where
//! it has something to play them on. Projection decides, per difficulty:
//! * which events land on which clickable edge (circle, slider head/repeat/tail, spinner end),
//! * which edges got nothing: muted slider ends, or the fallback,
//! * which events have no object (orphaned) or land on a slider tick.

use crate::diff::TickPolicy;
use crate::{ChannelId, DiffProfile, Ms, Project};
use osu_format::timing::active_point;
use osu_format::{HitObjectKind, OsuDocument};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Circle,
    SliderHead,
    SliderRepeat,
    SliderTail,
    SpinnerEnd,
}

/// A moment where the player triggers a hitsound.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Edge {
    pub time: Ms,
    pub kind: EdgeKind,
    /// Index into the difficulty's hit objects.
    pub object: usize,
    /// Edge index within a slider (0 = head); 0 otherwise.
    pub edge: usize,
    /// Number of slides of the owning slider (1 = no repeats); 0 for non-sliders.
    pub slides: u32,
    /// Duration of the owning object in ms (0 for circles).
    pub object_duration_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Tick {
    pub time: Ms,
    pub object: usize,
}

/// A redline, kept for beat-snap queries (slider end muting).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Redline {
    pub time: f64,
    pub beat_length: f64,
    /// Beats per measure, for the timeline's bar lines.
    pub meter: u32,
}

/// The hitsound-relevant shape of one difficulty.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffTimeline {
    /// Sorted by time.
    pub edges: Vec<Edge>,
    /// Sorted by time.
    pub ticks: Vec<Tick>,
    /// Sorted by time.
    pub redlines: Vec<Redline>,
}

impl DiffTimeline {
    /// Builds the timeline from a parsed `.osu`.
    ///
    /// Slider timing follows osu!stable (findings 3):
    /// * `px_per_beat = 100 * SliderMultiplier * sv`, tick spacing `px_per_beat / SliderTickRate`;
    /// * a tick is skipped if it is within 10 ms of travel from the span's end;
    /// * on later spans ticks mirror the previous span, unless the last tick of a span
    ///   was skipped - then each span restarts its ticks from its own start.
    pub fn from_document(doc: &OsuDocument) -> osu_format::Result<Self> {
        let points = doc.timing_points()?;
        let objects = doc.hit_objects()?;
        let multiplier: f64 = doc.value("Difficulty", "SliderMultiplier").and_then(|v| v.parse().ok()).unwrap_or(1.4);
        let tick_rate: f64 = doc.value("Difficulty", "SliderTickRate").and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let mut reds: Vec<_> = points.iter().filter(|p| p.uninherited).cloned().collect();
        reds.retain(|p| p.time.is_finite() && p.beat_length.is_finite() && p.beat_length > 0.0);
        reds.sort_by(|a, b| a.time.total_cmp(&b.time));

        let mut tl = DiffTimeline {
            redlines: reds
                .iter()
                .map(|p| Redline { time: p.time, beat_length: p.beat_length, meter: p.meter.max(1) })
                .collect(),
            ..Default::default()
        };
        for (i, o) in objects.iter().enumerate() {
            match o.kind {
                HitObjectKind::Circle | HitObjectKind::Hold => tl.edges.push(Edge {
                    time: o.time,
                    kind: EdgeKind::Circle,
                    object: i,
                    edge: 0,
                    slides: 0,
                    object_duration_ms: 0.0,
                }),
                HitObjectKind::Spinner => {
                    let end = o.explicit_end_time().unwrap_or(o.time);
                    tl.edges.push(Edge {
                        time: end,
                        kind: EdgeKind::SpinnerEnd,
                        object: i,
                        edge: 0,
                        slides: 0,
                        object_duration_ms: f64::from(end) - f64::from(o.time),
                    });
                }
                HitObjectKind::Slider => {
                    let t = f64::from(o.time);
                    let beat = active_point(&reds, t).map_or(500.0, |p| p.beat_length);
                    let sv = active_point(&points, t).map_or(1.0, |p| p.sv_multiplier());
                    let px_per_beat = 100.0 * multiplier * sv;
                    let span = o.length / px_per_beat * beat;
                    let slides = o.slides.max(1) as usize;
                    let tick_px = px_per_beat / tick_rate;
                    let tick_count = if tick_rate > 0.0 { o.length / tick_px * slides as f64 } else { 0.0 };
                    if !span.is_finite() || span < 0.0 || !tick_count.is_finite() || tick_count > 100_000.0 {
                        return Err(osu_format::ParseError::Invalid { line: i, message: "invalid slider timing".into() });
                    }
                    for e in 0..=slides {
                        let kind = match e {
                            0 => EdgeKind::SliderHead,
                            e if e == slides => EdgeKind::SliderTail,
                            _ => EdgeKind::SliderRepeat,
                        };
                        tl.edges.push(Edge {
                            time: (t + span * e as f64).round() as Ms,
                            kind,
                            object: i,
                            edge: e,
                            slides: slides as u32,
                            object_duration_ms: span * slides as f64,
                        });
                    }
                    if tick_rate > 0.0 && px_per_beat.is_finite() && px_per_beat > 0.0 {
                        let min_from_end_px = 10.0 / beat * px_per_beat;
                        let mut offsets = Vec::new();
                        let mut k = 1.0;
                        let mut skipped = false;
                        while k * tick_px <= o.length {
                            if o.length - k * tick_px <= min_from_end_px {
                                skipped = true;
                                break;
                            }
                            offsets.push(k * tick_px / px_per_beat * beat);
                            k += 1.0;
                        }
                        for s in 0..slides {
                            let start = t + span * s as f64;
                            let mirror = !skipped && s % 2 == 1;
                            for &off in &offsets {
                                let off = if mirror { span - off } else { off };
                                tl.ticks.push(Tick { time: (start + off).round() as Ms, object: i });
                            }
                        }
                    }
                }
            }
        }
        tl.edges.sort_by_key(|e| e.time);
        tl.ticks.sort_by_key(|t| t.time);
        Ok(tl)
    }

    /// The redline governing `time`.
    pub fn redline_at(&self, time: f64) -> Option<Redline> {
        self.redlines.iter().rev().find(|r| r.time <= time).or(self.redlines.first()).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", content = "index", rename_all = "snake_case")]
pub enum Target {
    /// Index into [`DiffTimeline::edges`].
    Edge(usize),
    /// Index into [`DiffTimeline::ticks`].
    Tick(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HitKind {
    /// One or more events landed here.
    Matched,
    /// Nothing landed; the fallback channel plays.
    Fallback,
    /// Nothing landed and the slider-end mute filter selected it.
    Muted,
    /// Nothing landed, nothing muted it and no fallback is configured. Export resets
    /// the edge to a plain default hit (no additions, Auto sets, the skin's sound) and
    /// reports it (D-36). It keeps the active custom index when that plays the skin's
    /// sound too (D-81).
    Empty,
}

/// The channels that sound at one target in one difficulty.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectedHit {
    pub time: Ms,
    pub target: Target,
    /// Empty for `Muted` and `Empty`.
    pub channels: Vec<ChannelId>,
    pub kind: HitKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// An edge with no event and no fallback configured.
    Unhitsounded { time: Ms },
    /// Events that have nothing to play on in this difficulty.
    Orphaned { time: Ms, channels: Vec<ChannelId> },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Projection {
    pub hits: Vec<ProjectedHit>,
    pub warnings: Vec<Warning>,
}

/// Index and distance of the entry of `times` (sorted ascending) nearest to `t`.
/// Ties go to the earlier entry.
fn nearest(times: &[Ms], t: Ms) -> Option<(usize, u32)> {
    let pos = times.partition_point(|&et| et < t);
    [pos.checked_sub(1), (pos < times.len()).then_some(pos)]
        .into_iter()
        .flatten()
        .map(|i| (i, times[i].abs_diff(t)))
        .min_by_key(|&(i, d)| (d, i))
}

/// Groups `(channel, time)` events into moments the way Mapping Tools `ZipLayers`
/// does (`HitsoundStudioEngine.cs`): events are visited in the given order (channel
/// priority, then time) and each joins the first-created moment within `leniency` ms,
/// else starts a new one. A moment keeps the time of its first event.
pub fn zip_events(events: impl IntoIterator<Item = (ChannelId, Ms)>, leniency: Ms) -> Vec<(Ms, Vec<ChannelId>)> {
    let leniency = leniency.max(0);
    let mut moments: Vec<(Ms, Vec<ChannelId>)> = Vec::new();
    let mut by_time: std::collections::BTreeMap<Ms, usize> = std::collections::BTreeMap::new();
    for (channel, t) in events {
        let found = by_time.range(t.saturating_sub(leniency)..=t.saturating_add(leniency)).map(|(_, &i)| i).min();
        match found {
            Some(i) => {
                if !moments[i].1.contains(&channel) {
                    moments[i].1.push(channel);
                }
            }
            None => {
                by_time.insert(t, moments.len());
                moments.push((t, vec![channel]));
            }
        }
    }
    moments.sort_by_key(|m| m.0);
    moments
}

/// The base layer as if it were a difficulty with one circle per moment. Preview of
/// the base uses it so the full hitsound score gets exactly the packing a real
/// difficulty gets (D-26).
pub fn base_view(project: &Project) -> (DiffTimeline, Projection) {
    let leniency = project.export.grouping_leniency_ms.round() as Ms;
    let moments = zip_events(project.base_events(), leniency);
    let mut timeline = DiffTimeline::default();
    let mut projection = Projection::default();
    for (i, (time, mut channels)) in moments.into_iter().enumerate() {
        timeline.edges.push(Edge { time, kind: EdgeKind::Circle, object: i, edge: 0, slides: 0, object_duration_ms: 0.0 });
        channels.sort_unstable();
        projection.hits.push(ProjectedHit { time, target: Target::Edge(i), channels, kind: HitKind::Matched });
    }
    (timeline, projection)
}

/// Projects the base layer onto one difficulty.
pub fn project(project: &Project, diff: &DiffProfile, timeline: &DiffTimeline) -> Projection {
    let events = diff.apply_overrides(project.base_events());
    let leniency = diff.match_leniency_ms.round().max(0.0) as u32;

    let edge_times: Vec<Ms> = timeline.edges.iter().map(|e| e.time).collect();
    let tick_times: Vec<Ms> = timeline.ticks.iter().map(|k| k.time).collect();
    let mut on_edge: Vec<Vec<ChannelId>> = vec![Vec::new(); timeline.edges.len()];
    let mut on_tick: Vec<Vec<ChannelId>> = vec![Vec::new(); timeline.ticks.len()];
    let mut orphans: Vec<(Ms, ChannelId)> = Vec::new();

    for (channel, t) in events {
        if project.channel(channel).is_none() {
            // An override that still names a deleted channel (ids are never reused).
            continue;
        }
        if let Some((i, dist)) = nearest(&edge_times, t)
            && dist <= leniency
        {
            on_edge[i].push(channel);
            continue;
        }
        if diff.tick_policy == TickPolicy::Render
            && let Some((i, dist)) = nearest(&tick_times, t)
            && dist <= leniency
        {
            on_tick[i].push(channel);
            continue;
        }
        orphans.push((t, channel));
    }

    let fallback = diff.fallback.filter(|&id| project.channel(id).is_some())
        .or(project.fallback.filter(|&id| project.channel(id).is_some()));
    let mut out = Projection::default();
    for (i, mut channels) in on_edge.into_iter().enumerate() {
        let edge = &timeline.edges[i];
        let time = edge.time;
        channels.sort_unstable();
        channels.dedup();
        let target = Target::Edge(i);
        if !channels.is_empty() {
            out.hits.push(ProjectedHit { time, target, channels, kind: HitKind::Matched });
        } else if diff.slider_end_mute.as_ref().is_some_and(|m| m.should_mute(edge, timeline)) {
            // Events win over muting: only otherwise-empty ends are muted (Hitsound Copier rule).
            out.hits.push(ProjectedHit { time, target, channels, kind: HitKind::Muted });
        } else if let Some(fb) = fallback {
            out.hits.push(ProjectedHit { time, target, channels: vec![fb], kind: HitKind::Fallback });
        } else {
            out.hits.push(ProjectedHit { time, target, channels, kind: HitKind::Empty });
            out.warnings.push(Warning::Unhitsounded { time });
        }
    }
    for (i, mut channels) in on_tick.into_iter().enumerate() {
        if channels.is_empty() {
            continue;
        }
        channels.sort_unstable();
        channels.dedup();
        out.hits.push(ProjectedHit { time: timeline.ticks[i].time, target: Target::Tick(i), channels, kind: HitKind::Matched });
    }
    out.hits.sort_by_key(|h| h.time);

    orphans.sort_unstable();
    for (t, c) in orphans {
        match out.warnings.last_mut() {
            Some(Warning::Orphaned { time, channels }) if *time == t => channels.push(c),
            _ => out.warnings.push(Warning::Orphaned { time: t, channels: vec![c] }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Channel;
    use crate::mute::SliderEndMute;
    use osu_format::{Hitsound, SampleSet};

    fn circle(i: usize, time: Ms) -> Edge {
        Edge { time, kind: EdgeKind::Circle, object: i, edge: 0, slides: 0, object_duration_ms: 0.0 }
    }

    fn timeline(times: &[Ms]) -> DiffTimeline {
        DiffTimeline {
            edges: times.iter().enumerate().map(|(i, &t)| circle(i, t)).collect(),
            ticks: vec![Tick { time: 1250, object: 0 }],
            redlines: vec![Redline { time: 0.0, beat_length: 500.0, meter: 4 }],
        }
    }

    #[test]
    fn edge_without_event_or_fallback_is_an_empty_hit_with_a_warning() {
        let (mut p, _, _) = project_with_kick_and_fallback();
        p.fallback = None;
        let out = project(&p, &DiffProfile::new("hard.osu", "Hard"), &timeline(&[1000, 2000]));
        assert_eq!(out.hits[1].kind, HitKind::Empty);
        assert!(out.warnings.contains(&Warning::Unhitsounded { time: 2000 }));
    }

    #[test]
    fn nearest_prefers_closest_then_earliest() {
        assert_eq!(nearest(&[100, 200, 300], 240), Some((1, 40)));
        assert_eq!(nearest(&[100, 200, 300], 250), Some((1, 50)));
        assert_eq!(nearest(&[100], 500), Some((0, 400)));
        assert_eq!(nearest(&[], 5), None);
    }

    #[test]
    fn zip_events_matches_mapping_tools_grouping() {
        // Channel 1 creates moments at 1000 and 1020. Channel 2's 1010 is within 15 ms
        // of both and joins the first created (1000), as MT's FirstOrDefault does.
        let zipped = zip_events([(1, 1000), (1, 1020), (2, 1010), (2, 1036)], 15);
        assert_eq!(zipped, vec![(1000, vec![1, 2]), (1020, vec![1]), (1036, vec![2])]);
    }

    #[test]
    fn base_view_has_one_circle_per_moment() {
        let (p, kick, _) = project_with_kick_and_fallback();
        let (tl, proj) = base_view(&p);
        assert_eq!(tl.edges.iter().map(|e| e.time).collect::<Vec<_>>(), vec![1000, 1250, 1500]);
        assert!(proj.hits.iter().all(|h| h.channels == vec![kick] && h.kind == HitKind::Matched));
    }

    fn project_with_kick_and_fallback() -> (Project, ChannelId, ChannelId) {
        let mut p = Project::new("t", ".");
        let mut kick = Channel::new(0, "Kick", SampleSet::Drum, Hitsound::Normal);
        for t in [1000, 1250, 1500] {
            kick.add_event(t);
        }
        let kick = p.add_channel(kick);
        let soft = p.add_channel(Channel::new(0, "Soft normal", SampleSet::Soft, Hitsound::Normal));
        p.fallback = Some(soft);
        (p, kick, soft)
    }

    #[test]
    fn base_events_follow_diff_rhythm() {
        let (p, kick, soft) = project_with_kick_and_fallback();
        let diff = DiffProfile::new("hard.osu", "Hard");
        // Diff has objects at 1000 and 2000 only: 1000 matches, 2000 needs fallback,
        // 1250/1500 are orphaned.
        let out = project(&p, &diff, &timeline(&[1002, 2000]));
        assert_eq!(out.hits.len(), 2);
        assert_eq!(out.hits[0].channels, vec![kick]);
        assert_eq!((out.hits[1].kind, out.hits[1].channels.clone()), (HitKind::Fallback, vec![soft]));
        assert_eq!(
            out.warnings,
            vec![
                Warning::Orphaned { time: 1250, channels: vec![kick] },
                Warning::Orphaned { time: 1500, channels: vec![kick] },
            ]
        );
    }

    #[test]
    fn tick_policy_render_claims_tick_events() {
        let (p, kick, _) = project_with_kick_and_fallback();
        let mut diff = DiffProfile::new("hard.osu", "Hard");
        diff.tick_policy = TickPolicy::Render;
        let out = project(&p, &diff, &timeline(&[1000, 1500]));
        let tick = out.hits.iter().find(|h| matches!(h.target, Target::Tick(_))).unwrap();
        assert_eq!((tick.time, tick.channels.clone()), (1250, vec![kick]));
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn empty_slider_end_on_muted_snap_is_muted_not_fallback() {
        let (p, _, _) = project_with_kick_and_fallback();
        let mut diff = DiffProfile::new("hard.osu", "Hard");
        diff.slider_end_mute = Some(SliderEndMute::default());
        let mut tl = timeline(&[1000]);
        // 1-slide slider 1000 -> 1250 (half a beat at 120 bpm): tail on 1/2 -> muted by default.
        tl.edges = vec![
            Edge { time: 1000, kind: EdgeKind::SliderHead, object: 0, edge: 0, slides: 1, object_duration_ms: 250.0 },
            Edge { time: 1250, kind: EdgeKind::SliderTail, object: 0, edge: 1, slides: 1, object_duration_ms: 250.0 },
        ];
        let out = project(&p, &diff, &tl);
        // 1250 has a kick event in the base, so it is Matched, not muted.
        assert_eq!(out.hits[1].kind, HitKind::Matched);
        let mut diff2 = diff.clone();
        diff2.overrides.push(crate::Override::MuteChannel { channel: 1, range: None });
        let out = project(&p, &diff2, &tl);
        assert_eq!(out.hits[1].kind, HitKind::Muted);
    }

    fn doc(slider: &str, tick_rate: u32) -> OsuDocument {
        OsuDocument::parse(&format!(
            "osu file format v14\n\n[Difficulty]\nSliderMultiplier:1\nSliderTickRate:{tick_rate}\n\n[TimingPoints]\n0,500,4,2,0,70,1,0\n\n[HitObjects]\n{slider}\n"
        ))
    }

    #[test]
    fn slider_ticks_match_stable_spacing() {
        // 100 px/beat, 500 ms/beat; 300 px slider = 1500 ms, ticks at 500 and 1000.
        let tl = DiffTimeline::from_document(&doc("0,0,0,2,0,L|300:0,1,300", 1)).unwrap();
        assert_eq!(tl.ticks.iter().map(|t| t.time).collect::<Vec<_>>(), vec![500, 1000]);
        assert_eq!(tl.edges.last().unwrap().time, 1500);
    }

    #[test]
    fn tick_within_10ms_of_end_is_skipped_and_spans_restart() {
        // 202 px = 1010 ms; tick at 1000 ms is 10 ms from the end -> skipped.
        // Reverse span restarts ticks from the repeat point: 1010 + 500.
        let tl = DiffTimeline::from_document(&doc("0,0,0,2,0,L|202:0,2,202", 1)).unwrap();
        assert_eq!(tl.ticks.iter().map(|t| t.time).collect::<Vec<_>>(), vec![500, 1510]);
    }

    #[test]
    fn ticks_mirror_on_reverse_when_not_skipped() {
        // 250 px = 1250 ms, tick at 500 and 1000. Reverse span mirrors: 1250 + 250, + 750.
        let tl = DiffTimeline::from_document(&doc("0,0,0,2,0,L|250:0,2,250", 1)).unwrap();
        assert_eq!(tl.ticks.iter().map(|t| t.time).collect::<Vec<_>>(), vec![500, 1000, 1500, 2000]);
    }
}
