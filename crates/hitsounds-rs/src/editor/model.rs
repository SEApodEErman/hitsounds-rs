//! Render data prepared on the worker. The canvas only visits the visible ranges.

use super::grid::BeatGrid;
use hs_core::{
    ChannelId, DiffTimeline, HitKind, Ms, Project, ProjectedHit, Projection, Target, Warning,
};
use hs_session::{Session, Waveform};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type Hit = (ChannelId, Ms);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IssueKind {
    Empty,
    Fallback,
    Orphaned,
    Tick,
}

impl IssueKind {
    pub const ALL: [Self; 4] = [Self::Empty, Self::Fallback, Self::Orphaned, Self::Tick];

    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::Fallback => "Fallback",
            Self::Orphaned => "Orphaned",
            Self::Tick => "Custom tick",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Issue {
    pub kind: IssueKind,
    pub time: Ms,
    pub target: Option<Target>,
    pub channel: Option<ChannelId>,
}

fn hit_issue(hit: &ProjectedHit) -> Option<IssueKind> {
    match (hit.target, hit.kind) {
        (Target::Tick(_), _) => Some(IssueKind::Tick),
        (_, HitKind::Empty) => Some(IssueKind::Empty),
        (_, HitKind::Fallback) => Some(IssueKind::Fallback),
        _ => None,
    }
}

fn issue_count(projection: &Projection) -> usize {
    projection
        .hits
        .iter()
        .filter(|h| hit_issue(h).is_some())
        .count()
        + projection
            .warnings
            .iter()
            .filter(|w| matches!(w, Warning::Orphaned { .. }))
            .count()
}

pub struct Layer {
    pub file: Option<String>,
    pub grid: BeatGrid,
    pub lanes: Vec<Lane>,
    pub rows: HashMap<ChannelId, usize>,
    pub objects: Option<Objects>,
    pub issues: Vec<Issue>,
    pub issue_groups: [Vec<usize>; 4],
    pub badges: BTreeMap<String, usize>,
    pub orphans: BTreeSet<Hit>,
}

#[derive(Default)]
pub struct Lane {
    pub hits: Vec<Ms>,
    pub removed: Vec<Ms>,
}

pub struct Objects {
    pub timeline: DiffTimeline,
    pub edges: Vec<HitKind>,
    pub ticks: Vec<bool>,
}

impl Objects {
    pub fn time(&self, target: Target) -> Option<Ms> {
        match target {
            Target::Edge(i) => self.timeline.edges.get(i).map(|edge| edge.time),
            Target::Tick(i) => self.timeline.ticks.get(i).map(|tick| tick.time),
        }
    }
}

impl Layer {
    pub fn from_session(session: &Session, view: &hs_session::Workspace<'_>) -> Self {
        let mut layer = Self::new(
            &session.project,
            view.active_diff,
            BeatGrid::new(view.redlines.to_vec()),
            view.events.as_deref(),
            view.timeline.zip(view.projection.as_ref()),
        );
        for diff in &session.project.diffs {
            if Some(diff.file.as_str()) == view.active_diff {
                layer.badges.insert(diff.file.clone(), layer.issues.len());
            } else if let Some(projection) = session.projection_for(&diff.file) {
                layer
                    .badges
                    .insert(diff.file.clone(), issue_count(&projection));
            }
        }
        layer
    }

    pub fn new(
        project: &Project,
        file: Option<&str>,
        grid: BeatGrid,
        events: Option<&[Hit]>,
        projection: Option<(&DiffTimeline, &Projection)>,
    ) -> Self {
        let rows: HashMap<_, _> = project
            .channels
            .iter()
            .enumerate()
            .map(|(row, c)| (c.id, row))
            .collect();
        let mut lanes: Vec<Lane> = project.channels.iter().map(|_| Lane::default()).collect();
        if let Some(events) = events {
            for &(channel, time) in events {
                if let Some(&row) = rows.get(&channel) {
                    lanes[row].hits.push(time);
                }
            }
            // D-50: show what disappeared even after range mute or channel replacement.
            for (channel, lane) in project.channels.iter().zip(&mut lanes) {
                lane.removed.extend(
                    channel
                        .events
                        .iter()
                        .copied()
                        .filter(|t| lane.hits.binary_search(t).is_err()),
                );
            }
        } else {
            for (channel, lane) in project.channels.iter().zip(&mut lanes) {
                lane.hits.clone_from(&channel.events);
                lane.hits.sort_unstable();
                lane.hits.dedup();
            }
        }

        let mut issues = Vec::new();
        let mut orphans = BTreeSet::new();
        let objects = projection.map(|(timeline, projection)| {
            let mut edges = vec![HitKind::Empty; timeline.edges.len()];
            let mut ticks = vec![false; timeline.ticks.len()];
            for hit in &projection.hits {
                match hit.target {
                    Target::Edge(i) => edges[i] = hit.kind,
                    Target::Tick(i) => ticks[i] = true,
                }
                if let Some(kind) = hit_issue(hit) {
                    issues.push(Issue {
                        kind,
                        time: hit.time,
                        target: Some(hit.target),
                        channel: hit.channels.first().copied(),
                    });
                }
            }
            for warning in &projection.warnings {
                if let Warning::Orphaned { time, channels } = warning {
                    orphans.extend(channels.iter().map(|&id| (id, *time)));
                    issues.push(Issue {
                        kind: IssueKind::Orphaned,
                        time: *time,
                        target: None,
                        channel: channels.first().copied(),
                    });
                }
            }
            issues.sort_by_key(|issue| (issue.time, issue.kind));
            Objects {
                timeline: timeline.clone(),
                edges,
                ticks,
            }
        });

        Self {
            file: file.map(str::to_owned),
            grid,
            lanes,
            rows,
            objects,
            issue_groups: IssueKind::ALL.map(|kind| {
                issues
                    .iter()
                    .enumerate()
                    .filter_map(|(i, issue)| (issue.kind == kind).then_some(i))
                    .collect()
            }),
            issues,
            badges: BTreeMap::new(),
            orphans,
        }
    }

    pub fn contains(&self, &(id, time): &Hit) -> bool {
        self.rows
            .get(&id)
            .is_some_and(|&row| self.lanes[row].hits.binary_search(&time).is_ok())
    }
}

/// Min/max pairs at successive powers of two, so fit-to-song never scans the full
/// audio or loses narrow transients. All levels are built off the render thread.
#[derive(Default)]
pub struct Wave {
    pub bucket_ms: f64,
    pub levels: Vec<Vec<[i8; 2]>>,
}

impl Wave {
    pub fn new(wave: Waveform<'_>) -> Self {
        let pairs: Vec<_> = wave.peaks.as_chunks::<2>().0.to_vec();
        let mut levels = vec![pairs];
        while let Some(level) = levels.last().filter(|l| l.len() > 1) {
            let next = level
                .chunks(2)
                .map(|chunk| match chunk {
                    [a, b] => [a[0].min(b[0]), a[1].max(b[1])],
                    [a] => *a,
                    _ => unreachable!(),
                })
                .collect();
            levels.push(next);
        }
        Self {
            bucket_ms: wave.bucket_ms,
            levels,
        }
    }

    pub fn at_scale(&self, ms_per_point: f64) -> Option<(f64, &[[i8; 2]])> {
        let level = ((ms_per_point / self.bucket_ms).log2().floor() as usize)
            .min(self.levels.len().checked_sub(1)?);
        let points = self.levels.get(level)?.as_slice();
        (!points.is_empty()).then_some((self.bucket_ms * 2_f64.powi(level as i32), points))
    }
}
