//! Timeline gestures and view state. A gesture emits one core Edit on release (D-47).

use super::{
    grid,
    model::{Hit, Issue},
    rail,
};
use crate::{
    app::Intent,
    keys::{self, Action},
    theme as t,
    views::Emit,
    widgets::{self, Hint},
    worker::{Job, ProjectView},
};
use eframe::egui::{
    self, CursorIcon, Id, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, Response,
    Sense, Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use hs_core::{ChannelId, Curve, Edit, Ms, Target, TimeRange, VolumeEnvelope, VolumePoint};
use hs_session::{Clock, Transport};
use std::collections::BTreeSet;

const DEFAULT_SCALE: f64 = 0.17;
const MAX_SCALE: f64 = 1.8;
const END_PAD_MS: f64 = 500.0;
/// Blank lead-in before 0 ms, in points, so a hit at the very start is not cut in half.
const LEAD: f64 = 18.0;
/// While scrubbing the playhead, the view scrolls once the pointer is this close to an edge.
const EDGE_ZONE: f32 = 32.0;
/// Edge scroll speed in points per second for each point the pointer is into the zone.
const EDGE_SPEED: f32 = 40.0;
/// The fastest edge scroll, in points per second.
const EDGE_MAX_SPEED: f32 = 6000.0;
/// A wheel step this soon after the last one continues from where that one went, since
/// the clock shows a seek only once it is heard (D-92). Longer than the output latency.
const WHEEL_CHAIN_SECONDS: f64 = 0.35;
/// How far the clock may be from where the last wheel step went for the next step to
/// still continue from it; further means something else moved the playhead.
const WHEEL_CHAIN_MS: f64 = 350.0;

/// The pointer tools of the timeline, as in FL Studio's playlist (D-62), in the order
/// and on the number keys testers asked for (D-86). Right-click deletes a hit with every
/// tool (D-51).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Never places: drag selects, drag a hit moves it.
    Select,
    /// Click places, drag on empty space selects, drag a hit moves it (D-47).
    #[default]
    Draw,
    /// Drag to place a hit on every snap step of one channel.
    Paint,
    /// Click or drag across hits to delete them.
    Delete,
}

impl Tool {
    pub const ALL: [Self; 4] = [Self::Select, Self::Draw, Self::Paint, Self::Delete];

    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Draw => "Draw",
            Self::Paint => "Paint",
            Self::Delete => "Delete",
        }
    }

    /// The shortcut that picks the tool (D-86 defaults, rebindable by D-92).
    pub fn action(self) -> Action {
        match self {
            Self::Select => Action::ToolSelect,
            Self::Draw => Action::ToolDraw,
            Self::Paint => Action::ToolPaint,
            Self::Delete => Action::ToolDelete,
        }
    }

    pub fn shortcut(self) -> &'static str {
        keys::label(self.action())
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::Select => "Select: drag a box around hits, then move, copy or delete them",
            Self::Draw => "Draw: click to place a hit, drag a hit to move it",
            Self::Paint => "Paint: drag to place a hit on every snap step",
            Self::Delete => "Delete: click or drag across hits to delete them",
        }
    }
}

pub struct Timeline {
    pub channel: Option<ChannelId>,
    pub tool: Tool,
    pub selection: BTreeSet<Hit>,
    pub object: Option<Target>,
    pub issue: Option<Issue>,
    pub divisor: u32,
    pub follow: bool,
    pub hover: String,
    pub rail: rail::Rail,
    pub(crate) scroll: f32,
    pub(crate) viewport: Viewport,
    tween: Option<Zoom>,
    wheel_carry: f32,
    /// The last wheel seek: where it went (ms) and when (UI seconds), D-92.
    wheel_seek: Option<(f64, f64)>,
    width: f32,
    gesture: Option<Gesture>,
    context: Option<ContextTarget>,
    /// The playhead position of the previous frame, to tell a seek from a scroll.
    last_position: f64,
    /// Every channel is muted from the mixer (D-77), so no mute light is lit.
    pub all_muted: bool,
    /// Copied hits as (channel, ms after the earliest copied hit), D-88. It outlives
    /// layer switches, so hits copy between the base layer and difficulties.
    clipboard: Vec<Hit>,
    /// Text for the system clipboard, written on the next frame. Without text there,
    /// egui-winit sends no paste event for Ctrl+V at all.
    clipboard_text: Option<String>,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            channel: None,
            tool: Tool::default(),
            selection: BTreeSet::new(),
            object: None,
            issue: None,
            divisor: 4,
            follow: true,
            all_muted: false,
            hover: String::new(),
            rail: rail::Rail::default(),
            scroll: 0.0,
            viewport: Viewport {
                scale: DEFAULT_SCALE,
                left: 0.0,
            },
            tween: None,
            wheel_carry: 0.0,
            wheel_seek: None,
            width: 400.0,
            gesture: None,
            context: None,
            last_position: 0.0,
            clipboard: Vec::new(),
            clipboard_text: None,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    pub scale: f64,
    pub left: f64,
}

impl Viewport {
    pub fn time(self, plot: Rect, x: f32) -> f64 {
        (f64::from(x - plot.left()) - LEAD + self.left) / self.scale
    }

    pub fn x(self, plot: Rect, time: f64) -> f32 {
        plot.left() + (time * self.scale - self.left + LEAD) as f32
    }

    fn bounded(self, length: f64, width: f32) -> Self {
        let max = ((length + END_PAD_MS) * self.scale + LEAD - f64::from(width)).max(0.0);
        Self {
            left: self.left.clamp(0.0, max),
            ..self
        }
    }
}

struct Zoom {
    start: Viewport,
    end: Viewport,
    at: f64,
}

pub(crate) enum Gesture {
    Box {
        start: Pos2,
        end: Pos2,
        channel: ChannelId,
        time: Ms,
        additive: bool,
        /// A click without a drag places a hit (Draw) or only clears the selection (Select).
        place: bool,
    },
    Paint {
        channel: ChannelId,
        row: usize,
        times: BTreeSet<Ms>,
        last: f64,
    },
    Erase {
        hits: BTreeSet<Hit>,
        last: Pos2,
    },
    Scroll {
        grab: f32,
    },
    Move {
        start: Pos2,
        anchor: Hit,
        delta: Ms,
        rows: i32,
    },
    Volume {
        start: Pos2,
        draft: VolumeEnvelope,
        index: usize,
        changed: bool,
        /// A plain click on an existing point: released without a drag, it switches
        /// the point's segment between ramp and step (D-89).
        toggle: bool,
    },
    /// Scrubbing the playhead. Playback pauses while the button is held, so the audio
    /// does not loop a frame-long slice under a stationary pointer, and resumes on release.
    Seek {
        resume: bool,
        /// The last time sent, so a held pointer seeks only when it moves.
        last: f64,
    },
}

#[derive(Clone, Copy)]
enum Clip {
    Copy,
    Cut,
    Paste,
}

#[derive(Clone, Copy)]
enum ContextTarget {
    Empty(Hit),
    Object(Target),
}

#[derive(Clone, Copy)]
pub(crate) struct Geometry {
    pub body: Rect,
    pub rail: Rect,
    pub plot: Rect,
    pub ruler: Rect,
    pub wave: Rect,
    pub volume: Rect,
    pub objects: Rect,
    pub lanes: Rect,
    pub hscroll: Rect,
}

impl Geometry {
    pub fn new(body: Rect, difficulty: bool) -> Self {
        let rail_width = if body.width() < 500.0 {
            t::COMPACT_RAIL_WIDTH
        } else {
            t::RAIL_WIDTH
        };
        let rail_width = rail_width.min(body.width() * 0.56);
        let rail = Rect::from_min_max(body.min, pos2(body.left() + rail_width, body.bottom()));
        let plot = Rect::from_min_max(
            pos2(rail.right(), body.top()),
            pos2(body.right() - t::TIMELINE_SCROLLBAR, body.bottom()),
        );
        let strip = |top: f32, height: f32| {
            Rect::from_min_max(
                pos2(plot.left(), top.min(plot.bottom())),
                pos2(plot.right(), (top + height).min(plot.bottom())),
            )
        };
        let ruler = strip(plot.top(), t::RULER_HEIGHT);
        let wave = strip(ruler.bottom(), t::WAVE_HEIGHT);
        let volume = strip(wave.bottom(), t::VOLUME_HEIGHT);
        let objects = strip(
            volume.bottom(),
            if difficulty { t::OBJECTS_HEIGHT } else { 0.0 },
        );
        let foot = (plot.bottom() - t::TIMELINE_SCROLLBAR).max(objects.bottom());
        let lanes = Rect::from_min_max(
            pos2(plot.left(), objects.bottom()),
            pos2(plot.right(), foot),
        );
        let hscroll = Rect::from_min_max(pos2(plot.left(), foot), plot.max);
        Self {
            body,
            rail,
            plot,
            ruler,
            wave,
            volume,
            objects,
            lanes,
            hscroll,
        }
    }

    pub fn row(self, row: usize, scroll: f32) -> Rect {
        let top = self.lanes.top() + row as f32 * t::ROW_HEIGHT - scroll;
        Rect::from_min_max(
            pos2(self.plot.left(), top),
            pos2(self.plot.right(), top + t::ROW_HEIGHT),
        )
    }

    pub fn rows(self, count: usize, scroll: f32) -> std::ops::Range<usize> {
        let first = (scroll / t::ROW_HEIGHT).floor() as usize;
        let end = ((scroll + self.lanes.height()) / t::ROW_HEIGHT).ceil() as usize;
        first.min(count)..end.min(count)
    }

    fn row_at(self, y: f32, scroll: f32) -> Option<usize> {
        self.lanes
            .y_range()
            .contains(y)
            .then(|| ((y - self.lanes.top() + scroll) / t::ROW_HEIGHT) as usize)
    }

    fn max_scroll(self, count: usize) -> f32 {
        ((count + 1) as f32 * t::ROW_HEIGHT - self.lanes.height()).max(0.0)
    }

    pub fn volume_y(self, volume: f64) -> f32 {
        let plot = self.volume.shrink(6.0);
        plot.top() + (1.0 - volume as f32 / 100.0) * plot.height()
    }

    fn volume_at(self, y: f32) -> u32 {
        let plot = self.volume.shrink(6.0);
        let percent = (1.0 - (y - plot.top()) / plot.height().max(1.0)) * 100.0;
        ((percent.clamp(5.0, 100.0) / 5.0).round() * 5.0) as u32
    }
}

pub(super) struct Canvas<'a> {
    pub view: &'a ProjectView,
    pub clock: &'a Clock,
    pub geometry: Geometry,
    pub ready: bool,
    pub reduced: bool,
    pub all_muted: bool,
}

impl Timeline {
    pub fn reset_layer(&mut self) {
        self.selection.clear();
        self.object = None;
        self.issue = None;
        self.gesture = None;
        self.context = None;
    }

    pub fn adopt(&mut self, view: &ProjectView) {
        self.selection.retain(|hit| view.layer.contains(hit));
        if self
            .channel
            .is_none_or(|id| !view.layer.rows.contains_key(&id))
        {
            self.channel = view.project.channels.first().map(|channel| channel.id);
        }
        if self
            .issue
            .is_some_and(|issue| !view.layer.issues.contains(&issue))
        {
            self.issue = None;
            self.object = None;
        }
    }

    pub fn range(&self) -> Option<TimeRange> {
        let mut times = self.selection.iter().map(|&(_, time)| time);
        let first = times.next()?;
        let (start, end) = times.fold((first, first), |(lo, hi), t| (lo.min(t), hi.max(t)));
        Some(TimeRange { start, end })
    }

    pub fn jump(&mut self, view: &ProjectView, time: Ms) {
        self.tween = None;
        self.viewport.left =
            (f64::from(time) * self.viewport.scale + LEAD - f64::from(self.width) * 0.35).max(0.0);
        self.viewport = self.viewport.bounded(view.length_ms, self.width);
    }

    pub fn next_issue(
        &mut self,
        view: &ProjectView,
        position: f64,
        backwards: bool,
        emit: Emit<'_>,
    ) {
        let issues = &view.layer.issues;
        if issues.is_empty() {
            return;
        }
        let index = self
            .issue
            .and_then(|current| issues.iter().position(|issue| *issue == current));
        let next = if backwards {
            index
                .unwrap_or_else(|| issues.partition_point(|issue| f64::from(issue.time) < position))
                .checked_sub(1)
                .unwrap_or(issues.len() - 1)
        } else {
            index.map_or_else(
                || issues.partition_point(|issue| f64::from(issue.time) <= position),
                |i| i + 1,
            ) % issues.len()
        };
        self.select_issue(view, issues[next], emit);
    }

    pub fn select_issue(&mut self, view: &ProjectView, issue: Issue, emit: Emit<'_>) {
        self.issue = Some(issue);
        self.object = issue.target;
        if let Some(channel) = issue.channel {
            self.channel = Some(channel);
            if let Some(&row) = view.layer.rows.get(&channel) {
                self.scroll = row as f32 * t::ROW_HEIGHT;
            }
        }
        if issue.target.is_none() {
            self.selection = view
                .project
                .channels
                .iter()
                .map(|c| (c.id, issue.time))
                .filter(|hit| view.layer.orphans.contains(hit))
                .collect();
        }
        self.jump(view, issue.time);
        seek(emit, f64::from(issue.time));
    }

    pub fn zoom(&mut self, view: &ProjectView, factor: f64, anchor: f32, now: f64, reduced: bool) {
        let width = self.width;
        let anchor = f64::from(anchor.clamp(0.0, width)) - LEAD;
        let from = self.tween.as_ref().map_or(self.viewport, |tween| tween.end);
        let min = Self::fit_scale(view, width);
        let scale = (from.scale * factor).clamp(min, MAX_SCALE);
        let anchor_time = (from.left + anchor) / from.scale;
        let end = Viewport {
            scale,
            left: anchor_time * scale - anchor,
        }
        .bounded(view.length_ms, width);
        self.set_view(end, now, reduced);
    }

    pub fn zoom_center(&mut self, view: &ProjectView, factor: f64, now: f64, reduced: bool) {
        self.zoom(view, factor, self.width / 2.0, now, reduced);
    }

    pub fn fit(&mut self, view: &ProjectView, now: f64, reduced: bool) {
        let scale = Self::fit_scale(view, self.width);
        self.set_view(Viewport { scale, left: 0.0 }, now, reduced);
    }

    fn fit_scale(view: &ProjectView, width: f32) -> f64 {
        ((f64::from(width) - LEAD).max(1.0) / (view.length_ms + END_PAD_MS)).min(MAX_SCALE)
    }

    fn set_view(&mut self, end: Viewport, now: f64, reduced: bool) {
        if reduced {
            self.viewport = end;
            self.tween = None;
        } else {
            self.tween = Some(Zoom {
                start: self.viewport,
                end,
                at: now,
            });
        }
    }

    pub fn keys(&mut self, ctx: &egui::Context, view: &ProjectView, clock: &Clock, emit: Emit<'_>) {
        // egui-winit turns Ctrl+C / Ctrl+X / Ctrl+V into these events, not key presses.
        // They work wherever the pointer is, like the other editing shortcuts in app.rs;
        // the caller already leaves them to a focused text field.
        let clip = ctx.input_mut(|input| {
            let mut found = None;
            input.events.retain(|event| {
                let clip = match event {
                    egui::Event::Copy => Clip::Copy,
                    egui::Event::Cut => Clip::Cut,
                    egui::Event::Paste(_) => Clip::Paste,
                    _ => return true,
                };
                found = Some(clip);
                false
            });
            found
        });
        match clip {
            Some(Clip::Copy) => self.copy_selection(),
            Some(Clip::Cut) => self.cut_selection(view, emit),
            Some(Clip::Paste) => self.paste(view, clock, emit),
            None => {}
        }
        let focused = ctx.memory(|memory| memory.has_focus(surface_id()));
        let pressed = ctx.input_mut(|input| {
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                return Some(None);
            }
            if !focused {
                return None;
            }
            [Action::SelectAll, Action::DeleteSelection, Action::PlaceHit]
                .into_iter()
                .find(|&action| keys::pressed(input, action))
                .map(Some)
        });
        match pressed {
            Some(None) => self.reset_layer(),
            Some(Some(Action::SelectAll)) => self.select_all(view),
            Some(Some(Action::DeleteSelection)) => self.delete_selection(view, emit),
            Some(Some(Action::PlaceHit)) => self.place_at_playhead(view, clock, emit),
            _ => {}
        }
    }

    pub fn select_all(&mut self, view: &ProjectView) {
        self.selection = view
            .project
            .channels
            .iter()
            .zip(&view.layer.lanes)
            .flat_map(|(channel, lane)| lane.hits.iter().map(move |&time| (channel.id, time)))
            .collect();
    }

    pub fn delete_selection(&mut self, view: &ProjectView, emit: Emit<'_>) {
        if self.selection.is_empty() {
            return;
        }
        edit(
            emit,
            Edit::RemoveEvents {
                diff: view.layer.file.clone(),
                events: self.selection.iter().copied().collect(),
            },
        );
        self.selection.clear();
    }

    /// Copies the selection, keeping each hit's channel and its distance from the
    /// earliest one (D-88).
    pub fn copy_selection(&mut self) {
        let Some(range) = self.range() else {
            return;
        };
        self.clipboard = self
            .selection
            .iter()
            .map(|&(id, time)| (id, time - range.start))
            .collect();
        let count = self.clipboard.len();
        self.clipboard_text = Some(format!(
            "hitsounds-rs: {count} hit{} copied",
            if count == 1 { "" } else { "s" }
        ));
    }

    pub fn cut_selection(&mut self, view: &ProjectView, emit: Emit<'_>) {
        self.copy_selection();
        self.delete_selection(view, emit);
    }

    /// How many hits a paste would place.
    pub fn clipboard_len(&self) -> usize {
        self.clipboard.len()
    }

    /// Pastes the copied hits with the earliest at the snapped playhead (D-88). On each
    /// channel the clipboard holds, the pasted span replaces what was there: hits
    /// between the first and last pasted time that are not part of the paste go. It is
    /// one edit, so one undo puts every replaced hit back. The pasted hits become the
    /// selection, ready to be dragged into place.
    pub fn paste(&mut self, view: &ProjectView, clock: &Clock, emit: Emit<'_>) {
        let at = self.snap(view, clock.position_ms);
        let pasted: BTreeSet<Hit> = self
            .clipboard
            .iter()
            // A channel deleted since the copy is skipped; ids are never reused.
            .filter(|(id, _)| view.layer.rows.contains_key(id))
            .filter_map(|&(id, offset)| Some((id, at.checked_add(offset)?)))
            .collect();
        let Some(end) = pasted.iter().map(|&(_, time)| time).max() else {
            return;
        };
        let channels: BTreeSet<ChannelId> = pasted.iter().map(|&(id, _)| id).collect();
        let replaced: Vec<Hit> = channels
            .into_iter()
            .filter_map(|id| Some((id, &view.layer.lanes.get(*view.layer.rows.get(&id)?)?.hits)))
            .flat_map(|(id, hits)| {
                hits[grid::visible(hits, f64::from(at), f64::from(end), |time| *time)]
                    .iter()
                    .map(move |&time| (id, time))
            })
            .filter(|hit| !pasted.contains(hit))
            .collect();
        let added: Vec<Hit> = pasted
            .iter()
            .copied()
            .filter(|hit| !view.layer.contains(hit))
            .collect();
        let diff = view.layer.file.clone();
        let mut edits = Vec::new();
        if !replaced.is_empty() {
            edits.push(Edit::RemoveEvents {
                diff: diff.clone(),
                events: replaced,
            });
        }
        if !added.is_empty() {
            edits.push(Edit::AddEvents {
                diff,
                events: added,
            });
        }
        if !edits.is_empty() {
            edit(emit, Edit::Batch { edits });
        }
        self.selection = pasted;
    }

    pub fn place_at_playhead(&mut self, view: &ProjectView, clock: &Clock, emit: Emit<'_>) {
        if let Some(channel) = self.channel {
            let time = self.snap(view, clock.position_ms);
            self.place(view, (channel, time), emit);
        }
    }

    /// The width of a step pad at `time`, or `None` when the view is too far out to
    /// draw pads and hits fall back to thin markers (D-61).
    pub(crate) fn pad_width(&self, view: &ProjectView, time: f64) -> Option<f32> {
        let step = (view.layer.grid.step_ms(time, self.divisor) * self.viewport.scale) as f32;
        (step >= t::PAD_MIN_STEP).then(|| (step - 2.0).min(t::PAD_MAX_WIDTH))
    }

    /// How far from its own time, in ms, a hit can still be grabbed.
    fn reach(&self, view: &ProjectView, time: f64) -> f64 {
        let half = self
            .pad_width(view, time)
            .map_or(t::HIT_RADIUS, |width| width / 2.0 + 1.0);
        f64::from(half) / self.viewport.scale
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        view: &ProjectView,
        clock: &Clock,
        ready: bool,
        reduced: bool,
        emit: Emit<'_>,
    ) {
        if let Some(text) = self.clipboard_text.take() {
            ui.ctx().copy_text(text);
        }
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let geometry = Geometry::new(rect, view.layer.file.is_some());
        let canvas = Canvas {
            view,
            clock,
            geometry,
            ready,
            reduced,
            all_muted: self.all_muted,
        };
        let response = ui.interact(geometry.plot, surface_id(), Sense::click_and_drag());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, ready, "Editing timeline"));
        self.width = geometry.plot.width().max(1.0);
        let max_scroll = geometry.max_scroll(view.project.channels.len());
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        let now = ui.input(|input| input.time);
        if let Some(tween) = &self.tween {
            let progress = if reduced {
                1.0
            } else {
                t::ease_out(((now - tween.at) / t::ZOOM_SECONDS) as f32)
            };
            let interpolate = |from: f64, to: f64| from + (to - from) * f64::from(progress);
            self.viewport = Viewport {
                scale: interpolate(tween.start.scale, tween.end.scale),
                left: interpolate(tween.start.left, tween.end.left),
            };
            if progress >= 1.0 {
                self.tween = None;
            } else {
                ui.ctx().request_repaint();
            }
        }
        let moved = clock.playing || (clock.position_ms - self.last_position).abs() > 0.5;
        self.last_position = clock.position_ms;
        if self.follow && moved && self.tween.is_none() && self.gesture.is_none() {
            let x = self.viewport.x(geometry.plot, clock.position_ms);
            if x < geometry.plot.left() || x > geometry.plot.left() + self.width * 0.88 {
                self.jump(view, clock.position_ms as Ms);
            }
        }
        self.viewport = self.viewport.bounded(view.length_ms, self.width);
        let previous_hover = std::mem::take(&mut self.hover);
        let hover = self.input(ui, &response, &canvas, now, emit);
        if self.hover != previous_hover {
            ui.ctx().request_repaint();
        }
        super::paint::timeline(ui, self, &canvas, hover);
        let rack = rail::RailState {
            scroll: self.scroll,
            selected: self.channel,
            range: self.range(),
            follow: self.follow,
        };
        self.rail.show(ui, &canvas, &rack, emit);
        if max_scroll > 0.0 {
            self.scrollbar(ui, geometry, max_scroll);
        }
        self.overview(ui, &canvas);
    }

    fn overview_span(&self, canvas: &Canvas<'_>) -> (f32, f64) {
        let total = ((canvas.view.length_ms + END_PAD_MS) * self.viewport.scale + LEAD).max(1.0);
        let fraction = (f64::from(self.width) / total).clamp(0.04, 1.0) as f32;
        (
            fraction * canvas.geometry.hscroll.width(),
            (total - f64::from(self.width)).max(0.0),
        )
    }

    /// The horizontal scrollbar under the lanes: where the view sits in the song.
    fn overview(&mut self, ui: &mut Ui, canvas: &Canvas<'_>) {
        let rect = canvas.geometry.hscroll;
        if rect.height() <= 0.0 || rect.width() <= 0.0 {
            return;
        }
        let (width, max_left) = self.overview_span(canvas);
        let span = (rect.width() - width).max(1.0);
        let at = if max_left > 0.0 {
            (self.viewport.left / max_left) as f32
        } else {
            0.0
        };
        let handle = Rect::from_min_size(
            pos2(rect.left() + at * span, rect.top() + 2.0),
            vec2(width, rect.height() - 4.0),
        );
        let grabbed = matches!(self.gesture, Some(Gesture::Scroll { .. }));
        let hovered = ui.rect_contains_pointer(rect);
        let p = ui.painter();
        p.rect_filled(rect, 0, t::CHROME);
        p.hline(rect.x_range(), rect.top(), t::stroke(t::EDGE));
        p.rect_filled(
            handle,
            t::SMALL,
            if grabbed {
                t::DIM
            } else if hovered {
                t::EDGE_STRONG
            } else {
                t::EDGE
            },
        );
        let song = (canvas.view.length_ms + END_PAD_MS).max(1.0);
        let x = rect.left() + (canvas.clock.position_ms / song) as f32 * rect.width();
        p.vline(x, rect.y_range(), t::stroke(t::ACCENT.gamma_multiply(0.8)));
    }

    fn scrollbar(&mut self, ui: &mut Ui, geo: Geometry, max_scroll: f32) {
        let rect = Rect::from_min_max(pos2(geo.plot.right(), geo.lanes.top()), geo.body.max);
        if rect.height() <= 0.0 {
            return;
        }
        let response = ui.interact(
            rect,
            Id::new("timeline-channel-scroll"),
            Sense::click_and_drag(),
        );
        let fraction = (rect.height() / (rect.height() + max_scroll)).clamp(0.08, 1.0);
        let height = fraction * rect.height();
        let span = (rect.height() - height).max(1.0);
        if (response.clicked() || response.dragged())
            && let Some(point) = response.interact_pointer_pos()
        {
            self.scroll =
                ((point.y - rect.top() - height / 2.0) / span).clamp(0.0, 1.0) * max_scroll;
        }
        let top = rect.top() + self.scroll / max_scroll * span;
        let handle = Rect::from_min_size(
            pos2(rect.left() + 2.0, top),
            vec2(rect.width() - 4.0, height),
        );
        ui.painter().rect_filled(rect, 0, t::STAGE);
        ui.painter().rect_filled(
            handle,
            t::SMALL,
            if response.hovered() { t::DIM } else { t::EDGE },
        );
    }

    fn input(
        &mut self,
        ui: &mut Ui,
        response: &Response,
        canvas: &Canvas<'_>,
        now: f64,
        emit: Emit<'_>,
    ) -> Option<Pos2> {
        let Canvas {
            view, geometry: g, ..
        } = *canvas;
        let hovered = response.hovered().then(|| response.hover_pos()).flatten();
        self.hover.clear();
        if !canvas.ready {
            self.gesture = None;
            return None;
        }
        if !egui::Popup::is_any_open(ui.ctx())
            && self.gesture.is_none()
            && ui.rect_contains_pointer(g.body)
        {
            self.wheel(ui, canvas, now, emit);
        }
        let pointer = ui.input(|input| {
            let mouse = &input.pointer;
            (
                mouse.button_pressed(PointerButton::Primary),
                mouse.button_released(PointerButton::Primary),
                mouse.press_origin(),
                mouse.interact_pos(),
                input.modifiers,
            )
        });
        if pointer.0
            && (response.is_pointer_button_down_on() || response.clicked())
            && let Some(origin) = pointer.2.or(pointer.3)
        {
            response.request_focus();
            self.begin(canvas, origin, pointer.4, emit);
        }
        if let Some(position) = pointer.3 {
            if matches!(self.gesture, Some(Gesture::Seek { .. })) {
                let dt = ui.input(|input| input.stable_dt).min(0.1);
                if self.edge_scroll(canvas, position.x, dt) {
                    ui.ctx().request_repaint();
                }
            }
            self.drag(canvas, position, emit);
        }
        if pointer.1 {
            self.finish(canvas, emit);
        }
        if response.secondary_clicked()
            && let Some(point) = pointer.3
        {
            response.request_focus();
            self.context = self.right_click(canvas, point, emit);
        }
        if let Some(target) = self.context {
            response.context_menu(|ui| match target {
                ContextTarget::Empty(hit) => {
                    if ui.button("Place hit here").clicked() {
                        self.place(view, hit, emit);
                        ui.close();
                    }
                }
                ContextTarget::Object(target) => {
                    rail::object_menu(ui, view, target, self.channel, emit)
                }
            });
        }
        if let Some(point) = hovered {
            self.hover = self.readout(canvas, point);
            let on_hit = g.lanes.contains(point) && self.hit_at(canvas, point).is_some();
            let cursor = match (&self.gesture, self.tool) {
                (Some(Gesture::Move { .. } | Gesture::Scroll { .. }), _) => CursorIcon::Grabbing,
                (Some(Gesture::Seek { .. }), _) => CursorIcon::ResizeHorizontal,
                _ if g.hscroll.contains(point) => CursorIcon::Grab,
                _ if !g.lanes.contains(point) => CursorIcon::PointingHand,
                (_, Tool::Paint) => CursorIcon::Crosshair,
                (_, Tool::Delete) => CursorIcon::NotAllowed,
                _ if on_hit => CursorIcon::Grab,
                (_, Tool::Draw) => CursorIcon::Crosshair,
                (_, Tool::Select) => CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(cursor);
            widgets::hint(ui.ctx(), self.hint(canvas, point, on_hit));
        }
        hovered
    }

    fn wheel(&mut self, ui: &Ui, canvas: &Canvas<'_>, now: f64, emit: Emit<'_>) {
        let g = canvas.geometry;
        let (hover, wheels) = ui.input(|input| {
            let events: Vec<_> = input
                .events
                .iter()
                .filter_map(|event| match *event {
                    egui::Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                    } => Some((unit, delta, modifiers)),
                    _ => None,
                })
                .collect();
            (input.pointer.hover_pos(), events)
        });
        let Some(hover) = hover else {
            return;
        };
        let mut seek_to = None;
        for (unit, delta, mods) in wheels {
            let shift_only = mods.shift && !mods.alt && !mods.ctrl && !mods.command;
            let y = if delta.y == 0.0 && shift_only {
                delta.x
            } else {
                delta.y
            };
            let points = y * match unit {
                MouseWheelUnit::Point => 1.0,
                MouseWheelUnit::Line => 40.0,
                MouseWheelUnit::Page => g.lanes.height(),
            };
            if points == 0.0 {
                continue;
            }
            if mods.alt {
                self.zoom(
                    canvas.view,
                    f64::from(points.clamp(-200.0, 200.0) * 0.0015).exp(),
                    hover.x - g.plot.left(),
                    now,
                    canvas.reduced,
                );
            } else if g.rail.contains(hover) && !mods.ctrl && !mods.command {
                self.scroll = (self.scroll - points)
                    .clamp(0.0, g.max_scroll(canvas.view.project.channels.len()));
            } else {
                // Turning the other way drops what was gathered the old way, so a
                // touchpad reverses at once instead of first undoing the carry.
                if self.wheel_carry != 0.0 && self.wheel_carry.signum() != points.signum() {
                    self.wheel_carry = 0.0;
                }
                self.wheel_carry += points;
                if self.wheel_carry.abs() < 40.0 {
                    continue;
                }
                let direction = self.wheel_carry.signum();
                self.wheel_carry = 0.0;
                if mods.ctrl || mods.command {
                    self.divisor = grid::step_divisor(self.divisor, direction > 0.0);
                } else {
                    let steps = -(direction as i32) * if mods.shift { 4 } else { 1 };
                    let divisor = if canvas.clock.playing {
                        1
                    } else {
                        self.divisor
                    };
                    let from =
                        seek_to.unwrap_or_else(|| self.wheel_from(canvas.clock, now, steps > 0));
                    let time = canvas
                        .view
                        .layer
                        .grid
                        .step(from, divisor, steps)
                        .min(canvas.view.length_ms);
                    self.wheel_seek = Some((time, now));
                    seek_to = Some(time);
                }
            }
        }
        if let Some(time) = seek_to {
            seek(emit, time);
            if self.follow {
                self.jump(canvas.view, time as Ms);
            }
        }
    }

    /// Where the next wheel step starts (D-92). A seek reaches the clock only once it is
    /// heard, one output latency later, so the clock alone would send a quick second
    /// step to the same beat. Steps soon after a wheel seek continue from it instead:
    /// forward from where playback has got to since, back from the beat it went to, so
    /// every notch moves a beat even while the song plays.
    fn wheel_from(&self, clock: &Clock, now: f64, forward: bool) -> f64 {
        let Some((target, at)) = self.wheel_seek else {
            return clock.position_ms;
        };
        let elapsed = now - at;
        let heard = if clock.playing {
            target + elapsed.max(0.0) * 1000.0 * clock.rate
        } else {
            target
        };
        if elapsed >= WHEEL_CHAIN_SECONDS || (clock.position_ms - heard).abs() > WHEEL_CHAIN_MS {
            clock.position_ms
        } else if forward {
            heard
        } else {
            target
        }
    }

    pub(crate) fn snap(&self, view: &ProjectView, time: f64) -> Ms {
        self.target_at(
            view,
            time,
            (f64::from(t::HIT_RADIUS) / self.viewport.scale).min(32.0),
        )
        .and_then(|target| view.layer.objects.as_ref()?.time(target))
        .unwrap_or_else(|| view.layer.grid.snap(time, self.divisor))
        .max(0)
    }

    fn target_at(&self, view: &ProjectView, time: f64, limit: f64) -> Option<Target> {
        let objects = view.layer.objects.as_ref()?;
        let edges = &objects.timeline.edges;
        let ticks = &objects.timeline.ticks;
        [
            grid::nearest(edges, time, |edge| edge.time, limit).map(Target::Edge),
            grid::nearest(ticks, time, |tick| tick.time, limit).map(Target::Tick),
        ]
        .into_iter()
        .flatten()
        .min_by(|a, b| {
            let distance =
                |target| (f64::from(objects.time(target).unwrap_or_default()) - time).abs();
            distance(*a).total_cmp(&distance(*b))
        })
    }

    fn hit_at(&self, canvas: &Canvas<'_>, point: Pos2) -> Option<Hit> {
        let g = canvas.geometry;
        let row = g.row_at(point.y, self.scroll)?;
        let channel = canvas.view.project.channels.get(row)?;
        let hits = &canvas.view.layer.lanes[row].hits;
        let time = self.viewport.time(g.plot, point.x);
        let index = grid::nearest(hits, time, |t| *t, self.reach(canvas.view, time))?;
        Some((channel.id, hits[index]))
    }

    fn object_at(&self, canvas: &Canvas<'_>, point: Pos2) -> Option<Target> {
        self.target_at(
            canvas.view,
            self.viewport.time(canvas.geometry.plot, point.x),
            f64::from(t::HIT_RADIUS) / self.viewport.scale,
        )
    }

    fn begin(&mut self, canvas: &Canvas<'_>, point: Pos2, mods: Modifiers, emit: Emit<'_>) {
        let view = canvas.view;
        let g = canvas.geometry;
        let time = self.viewport.time(g.plot, point.x);
        let ctrl = mods.ctrl || mods.command;
        if g.hscroll.contains(point) {
            let (width, max_left) = self.overview_span(canvas);
            let span = (g.hscroll.width() - width).max(1.0);
            let left = g.hscroll.left() + (self.viewport.left / max_left.max(1.0)) as f32 * span;
            // Grab the handle where it was pressed, or centre it under the pointer.
            let grab = if (left..=left + width).contains(&point.x) {
                point.x - left
            } else {
                width / 2.0
            };
            self.tween = None;
            self.gesture = Some(Gesture::Scroll { grab });
        } else if g.volume.contains(point) {
            self.begin_volume(view, g, point, time, ctrl);
        } else if g.lanes.contains(point) {
            let row = g.row_at(point.y, self.scroll).unwrap_or(0);
            let Some(channel) = view.project.channels.get(row).map(|c| c.id) else {
                if !mods.shift && !ctrl {
                    self.selection.clear();
                }
                return;
            };
            self.channel = Some(channel);
            emit(Intent::SelectChannel(channel));
            let hit = self.hit_at(canvas, point);
            match self.tool {
                Tool::Paint if !ctrl && !mods.shift => {
                    self.selection.clear();
                    let first = hit.map_or_else(|| self.snap(view, time), |hit| hit.1);
                    self.gesture = Some(Gesture::Paint {
                        channel,
                        row,
                        times: BTreeSet::from([first]),
                        last: time,
                    });
                    return;
                }
                Tool::Delete if !ctrl && !mods.shift => {
                    self.selection.clear();
                    self.gesture = Some(Gesture::Erase {
                        hits: hit.into_iter().collect(),
                        last: point,
                    });
                    return;
                }
                _ => {}
            }
            if let Some(hit) = hit
                && ctrl
            {
                if !self.selection.remove(&hit) {
                    self.selection.insert(hit);
                }
            } else if let Some(hit) = hit
                && !mods.shift
            {
                if !self.selection.contains(&hit) {
                    self.selection = BTreeSet::from([hit]);
                }
                self.gesture = Some(Gesture::Move {
                    start: point,
                    anchor: hit,
                    delta: 0,
                    rows: 0,
                });
            } else {
                self.gesture = Some(Gesture::Box {
                    start: point,
                    end: point,
                    channel,
                    time: self.snap(view, time),
                    additive: mods.shift || ctrl,
                    place: self.tool == Tool::Draw,
                });
            }
        } else {
            self.object = g
                .objects
                .contains(point)
                .then(|| self.object_at(canvas, point))
                .flatten();
            self.issue = None;
            if let Some(target) = self.object {
                emit(Intent::ShowDifficulty);
                seek(
                    emit,
                    f64::from(
                        view.layer
                            .objects
                            .as_ref()
                            .and_then(|o| o.time(target))
                            .unwrap_or_default(),
                    ),
                );
            } else {
                let resume = canvas.clock.playing;
                if resume {
                    emit(Intent::Transport(Transport::Pause));
                }
                let time = time.clamp(0.0, view.length_ms);
                self.gesture = Some(Gesture::Seek { resume, last: time });
                seek(emit, time);
            }
        }
    }

    fn begin_volume(
        &mut self,
        view: &ProjectView,
        g: Geometry,
        point: Pos2,
        time: f64,
        ctrl: bool,
    ) {
        let mut draft = envelope(view).clone();
        let at = self.volume_point(g, &draft, point);
        // D-89: a plain click must not change the curve by accident. It only grabs an
        // existing point (to drag it, or to switch ramp and step on release); adding a
        // point takes Ctrl+click.
        if !ctrl {
            if let Some(index) = at {
                self.gesture = Some(Gesture::Volume {
                    start: point,
                    draft,
                    index,
                    changed: false,
                    toggle: true,
                });
            }
            return;
        }
        let mut changed = false;
        let index = at.unwrap_or_else(|| {
            let time = view.layer.grid.snap(time, self.divisor);
            match draft.points.binary_search_by_key(&time, |p| p.time) {
                Ok(i) => i,
                Err(i) => {
                    let curve = i
                        .checked_sub(1)
                        .map_or(Curve::Linear, |prev| draft.points[prev].curve);
                    draft.points.insert(
                        i,
                        VolumePoint {
                            time,
                            volume: g.volume_at(point.y),
                            curve,
                        },
                    );
                    changed = true;
                    i
                }
            }
        });
        self.gesture = Some(Gesture::Volume {
            start: point,
            draft,
            index,
            changed,
            toggle: false,
        });
    }

    fn volume_point(&self, g: Geometry, envelope: &VolumeEnvelope, point: Pos2) -> Option<usize> {
        let time = self.viewport.time(g.plot, point.x);
        let i = grid::nearest(
            &envelope.points,
            time,
            |p| p.time,
            f64::from(t::HIT_RADIUS) / self.viewport.scale,
        )?;
        ((g.volume_y(f64::from(envelope.points[i].volume)) - point.y).abs() <= t::GAP).then_some(i)
    }

    fn drag(&mut self, canvas: &Canvas<'_>, point: Pos2, emit: Emit<'_>) {
        let Some(mut gesture) = self.gesture.take() else {
            return;
        };
        let g = canvas.geometry;
        let view = canvas.view;
        match &mut gesture {
            Gesture::Box { end, .. } => *end = point,
            Gesture::Paint { times, last, .. } => {
                let x = point.x.clamp(g.plot.left(), g.plot.right());
                let time = self.viewport.time(g.plot, x).clamp(0.0, view.length_ms);
                // Fill every snap step the pointer crossed, so a fast drag leaves no gaps.
                let until = last.max(time);
                let mut tick = f64::from(view.layer.grid.snap(last.min(time), self.divisor));
                for _ in 0..4096 {
                    if tick > until {
                        break;
                    }
                    times.insert(tick.round() as Ms);
                    tick = view.layer.grid.step(tick, self.divisor, 1);
                }
                times.insert(self.snap(view, time));
                *last = time;
            }
            Gesture::Erase { hits, last } => {
                if let Some(row) = g.row_at(point.y, self.scroll)
                    && let Some(channel) = view.project.channels.get(row)
                {
                    let from = self.viewport.time(g.plot, last.x.min(point.x));
                    let until = self.viewport.time(g.plot, last.x.max(point.x));
                    let reach = self.reach(view, from);
                    let lane = &view.layer.lanes[row].hits;
                    hits.extend(
                        lane[grid::visible(lane, from - reach, until + reach, |time| *time)]
                            .iter()
                            .map(|&time| (channel.id, time)),
                    );
                }
                *last = point;
            }
            Gesture::Scroll { grab } => {
                let (width, max_left) = self.overview_span(canvas);
                let span = (g.hscroll.width() - width).max(1.0);
                let at = ((point.x - *grab - g.hscroll.left()) / span).clamp(0.0, 1.0);
                self.viewport.left = f64::from(at) * max_left;
            }
            Gesture::Move {
                start,
                anchor,
                delta,
                rows,
            } => {
                let by = point - *start;
                let moved = (by.y / t::ROW_HEIGHT).round() as i32;
                let mut earliest = Ms::MAX;
                let mut row_min = i32::MAX;
                let mut row_max = 0;
                for &(id, time) in &self.selection {
                    earliest = earliest.min(time);
                    if let Some(&row) = view.layer.rows.get(&id) {
                        row_min = row_min.min(row as i32);
                        row_max = row_max.max(row as i32);
                    }
                }
                *rows = moved.clamp(-row_min, view.project.channels.len() as i32 - 1 - row_max);
                *delta = if by.x.abs() >= t::DRAG_THRESHOLD {
                    let time = f64::from(anchor.1) + f64::from(by.x) / self.viewport.scale;
                    self.snap(view, time)
                        .saturating_sub(anchor.1)
                        .max(earliest.saturating_neg())
                } else {
                    0
                };
            }
            Gesture::Volume {
                start,
                draft,
                index,
                changed,
                toggle,
            } => {
                if point.distance(*start) >= t::DRAG_THRESHOLD {
                    *toggle = false;
                    let time = self.viewport.time(g.plot, point.x);
                    let before = index
                        .checked_sub(1)
                        .map_or(0, |i| draft.points[i].time.saturating_add(1));
                    let after = draft
                        .points
                        .get(*index + 1)
                        .map_or(Ms::MAX, |p| p.time.saturating_sub(1));
                    let value = &mut draft.points[*index];
                    value.time = view
                        .layer
                        .grid
                        .snap(time, self.divisor)
                        .clamp(before.min(after), after);
                    value.volume = g.volume_at(point.y);
                    *changed = true;
                }
            }
            Gesture::Seek { last, .. } => {
                // Past an edge the playhead stays pinned to it while the view scrolls.
                let x = point.x.clamp(g.plot.left(), g.plot.right());
                let time = self.viewport.time(g.plot, x).clamp(0.0, view.length_ms);
                if (time - *last).abs() >= 1.0 {
                    *last = time;
                    seek(emit, time);
                }
            }
        }
        self.gesture = Some(gesture);
    }

    /// Scrolls the view while the playhead is dragged into an edge of the plot, faster the
    /// further the pointer goes. Returns whether it scrolled, so the caller keeps repainting.
    fn edge_scroll(&mut self, canvas: &Canvas<'_>, x: f32, dt: f32) -> bool {
        let plot = canvas.geometry.plot;
        let zone = EDGE_ZONE.min(plot.width() / 4.0);
        let depth = if x < plot.left() + zone {
            x - (plot.left() + zone)
        } else if x > plot.right() - zone {
            x - (plot.right() - zone)
        } else {
            return false;
        };
        let speed = (depth * EDGE_SPEED).clamp(-EDGE_MAX_SPEED, EDGE_MAX_SPEED);
        let before = self.viewport.left;
        self.tween = None;
        self.viewport.left += f64::from(speed * dt);
        self.viewport = self.viewport.bounded(canvas.view.length_ms, self.width);
        self.viewport.left != before
    }

    fn finish(&mut self, canvas: &Canvas<'_>, emit: Emit<'_>) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        let g = canvas.geometry;
        let view = canvas.view;
        match gesture {
            Gesture::Box {
                start,
                end,
                channel,
                time,
                additive,
                place,
            } => {
                let rect = Rect::from_two_pos(start, end);
                if rect.width().max(rect.height()) < t::DRAG_THRESHOLD {
                    if place && !additive {
                        self.place(view, (channel, time), emit);
                    } else if !additive {
                        self.selection.clear();
                    }
                    return;
                }
                if !additive {
                    self.selection.clear();
                }
                let from = self.viewport.time(g.plot, rect.left());
                let until = self.viewport.time(g.plot, rect.right());
                for row in g.rows(view.layer.lanes.len(), self.scroll) {
                    if !g.row(row, self.scroll).intersects(rect) {
                        continue;
                    }
                    let hits = &view.layer.lanes[row].hits;
                    let id = view.project.channels[row].id;
                    self.selection.extend(
                        hits[grid::visible(hits, from, until, |time| *time)]
                            .iter()
                            .map(|&time| (id, time)),
                    );
                }
            }
            Gesture::Move { delta, rows, .. } if delta != 0 || rows != 0 => {
                let events: Vec<_> = self.selection.iter().copied().collect();
                let destinations = self.moved(view, delta, rows).collect();
                edit(
                    emit,
                    Edit::MoveEvents {
                        diff: view.layer.file.clone(),
                        events,
                        delta,
                        rows,
                    },
                );
                self.selection = destinations;
            }
            Gesture::Volume {
                draft,
                changed: true,
                ..
            } => set_volume(emit, view, draft.points),
            Gesture::Volume {
                mut draft,
                index,
                toggle: true,
                ..
            } if index + 1 < draft.points.len() => {
                // The last point's curve leads nowhere, so switching it would change nothing.
                let curve = &mut draft.points[index].curve;
                *curve = if *curve == Curve::Linear {
                    Curve::Step
                } else {
                    Curve::Linear
                };
                set_volume(emit, view, draft.points);
            }
            Gesture::Paint { channel, times, .. } => {
                let events: Vec<Hit> = times
                    .into_iter()
                    .map(|time| (channel, time))
                    .filter(|hit| !view.layer.contains(hit))
                    .collect();
                if !events.is_empty() {
                    edit(
                        emit,
                        Edit::AddEvents {
                            diff: view.layer.file.clone(),
                            events,
                        },
                    );
                }
            }
            Gesture::Erase { hits, .. } if !hits.is_empty() => {
                self.selection.retain(|hit| !hits.contains(hit));
                edit(
                    emit,
                    Edit::RemoveEvents {
                        diff: view.layer.file.clone(),
                        events: hits.into_iter().collect(),
                    },
                );
            }
            Gesture::Seek { resume: true, .. } => emit(Intent::Transport(Transport::Play)),
            _ => {}
        }
    }

    /// The hits a paint stroke will add: its row and their times.
    pub(crate) fn painting(&self) -> Option<(usize, &BTreeSet<Ms>)> {
        match &self.gesture {
            Some(Gesture::Paint { row, times, .. }) => Some((*row, times)),
            _ => None,
        }
    }

    /// The hits an erase stroke will remove.
    pub(crate) fn erasing(&self) -> Option<&BTreeSet<Hit>> {
        match &self.gesture {
            Some(Gesture::Erase { hits, .. }) => Some(hits),
            _ => None,
        }
    }

    /// The hit under the pointer, for the tools that act on existing hits.
    pub(super) fn hovered_hit(&self, canvas: &Canvas<'_>, point: Pos2) -> Option<Hit> {
        (self.gesture.is_none() && canvas.geometry.lanes.contains(point))
            .then(|| self.hit_at(canvas, point))
            .flatten()
    }

    pub(crate) fn moved<'a>(
        &'a self,
        view: &'a ProjectView,
        delta: Ms,
        rows: i32,
    ) -> impl Iterator<Item = Hit> + 'a {
        self.selection.iter().filter_map(move |&(id, time)| {
            let row = *view.layer.rows.get(&id)? as i32 + rows;
            Some((
                view.project.channels.get(row as usize)?.id,
                time.checked_add(delta)?,
            ))
        })
    }

    fn right_click(
        &mut self,
        canvas: &Canvas<'_>,
        point: Pos2,
        emit: Emit<'_>,
    ) -> Option<ContextTarget> {
        let g = canvas.geometry;
        let view = canvas.view;
        if g.volume.contains(point) {
            let envelope = envelope(view);
            if let Some(i) = self.volume_point(g, envelope, point) {
                let mut points = envelope.points.clone();
                points.remove(i);
                set_volume(emit, view, points);
            }
            return None;
        }
        if g.objects.contains(point) {
            return self.object_at(canvas, point).map(ContextTarget::Object);
        }
        if let Some(hit) = self.hit_at(canvas, point) {
            // D-51: a hit's secondary click deletes it; context menus are for the lane and objects.
            edit(
                emit,
                Edit::RemoveEvents {
                    diff: view.layer.file.clone(),
                    events: vec![hit],
                },
            );
            self.selection.remove(&hit);
            return None;
        }
        let row = g.row_at(point.y, self.scroll)?;
        let channel = view.project.channels.get(row)?;
        Some(ContextTarget::Empty((
            channel.id,
            self.snap(view, self.viewport.time(g.plot, point.x)),
        )))
    }

    fn place(&mut self, view: &ProjectView, hit: Hit, emit: Emit<'_>) {
        edit(
            emit,
            Edit::AddEvents {
                diff: view.layer.file.clone(),
                events: vec![hit],
            },
        );
        self.selection = BTreeSet::from([hit]);
    }

    /// What the mouse does where it is now, for the hint bar (D-61).
    fn hint(&self, canvas: &Canvas<'_>, point: Pos2, on_hit: bool) -> Hint {
        let g = canvas.geometry;
        let line = Hint::new(self.hover.clone());
        if g.hscroll.contains(point) {
            Hint::new("Song overview")
                .key("Drag", "Scroll")
                .key("Alt+Wheel", "Zoom")
        } else if g.volume.contains(point) {
            let on_point = self.volume_point(g, envelope(canvas.view), point).is_some();
            if on_point {
                line.key("Click", "Ramp or step")
                    .key("Drag", "Move point")
                    .key("Right-click", "Delete point")
            } else {
                line.key("Ctrl+Click", "Add point")
            }
        } else if g.objects.contains(point) {
            line.key("Click", "Select and seek")
                .key("Right-click", "Add a hit here")
        } else if !g.lanes.contains(point) {
            line.key("Click", "Seek")
                .key("Wheel", "Step by snap")
                .key("Alt+Wheel", "Zoom")
                .key("Ctrl+Wheel", "Change snap")
        } else if g
            .row_at(point.y, self.scroll)
            .is_none_or(|row| row >= canvas.view.project.channels.len())
        {
            Hint::new("Below the last channel")
                .key("Wheel", "Seek")
                .key("Alt+Wheel", "Zoom")
        } else {
            match (self.tool, on_hit) {
                (Tool::Paint, _) => line
                    .key("Drag", "Paint hits on every snap step")
                    .key("Right-click", "Delete"),
                (Tool::Delete, _) => line.key("Click or drag", "Delete hits"),
                (_, true) => line
                    .key("Drag", "Move")
                    .key("Right-click", "Delete")
                    .key("Ctrl+Click", "Add to selection"),
                (Tool::Draw, false) => line
                    .key("Click", "Place hit")
                    .key("Drag", "Select")
                    .key("Wheel", "Seek"),
                (Tool::Select, false) => line.key("Drag", "Select hits").key("Wheel", "Seek"),
            }
        }
    }

    fn readout(&self, canvas: &Canvas<'_>, point: Pos2) -> String {
        let view = canvas.view;
        let g = canvas.geometry;
        let time = self.viewport.time(g.plot, point.x);
        if let Some(Gesture::Volume { draft, index, .. }) = &self.gesture {
            let node = draft.points[*index];
            return format!(
                "Volume {}% at {}",
                node.volume,
                grid::timecode(f64::from(node.time))
            );
        }
        if g.volume.contains(point) {
            return format!(
                "Volume {:.0}% at {}",
                envelope(view).at(time),
                grid::timecode(time)
            );
        }
        if g.objects.contains(point)
            && let Some(target) = self.object_at(canvas, point)
            && let Some(objects) = &view.layer.objects
        {
            let label = match target {
                Target::Edge(i) => super::edge_name(objects.timeline.edges[i].kind),
                Target::Tick(_) => "Slider tick",
            };
            return format!(
                "{label} at {}",
                grid::timecode(f64::from(objects.time(target).unwrap_or_default()))
            );
        }
        if let Some(row) = g.row_at(point.y, self.scroll)
            && let Some(channel) = view.project.channels.get(row)
        {
            let at = self
                .hit_at(canvas, point)
                .map_or_else(|| self.snap(view, time), |hit| hit.1);
            return format!("{} at {}", channel.name, grid::timecode(f64::from(at)));
        }
        grid::timecode(time)
    }

    pub(crate) fn draft(&self) -> Option<&VolumeEnvelope> {
        match &self.gesture {
            Some(Gesture::Volume { draft, .. }) => Some(draft),
            _ => None,
        }
    }

    pub(crate) fn marquee(&self) -> Option<Rect> {
        match self.gesture {
            Some(Gesture::Box { start, end, .. }) if start.distance(end) >= t::DRAG_THRESHOLD => {
                Some(Rect::from_two_pos(start, end))
            }
            _ => None,
        }
    }

    pub(crate) fn move_delta(&self) -> Option<(Ms, i32)> {
        match self.gesture {
            Some(Gesture::Move { delta, rows, .. }) if delta != 0 || rows != 0 => {
                Some((delta, rows))
            }
            _ => None,
        }
    }

    pub(crate) fn ghost(
        &self,
        view: &ProjectView,
        geo: Geometry,
        point: Pos2,
    ) -> Option<(usize, Ms)> {
        if self.gesture.is_some() || !geo.lanes.contains(point) {
            return None;
        }
        let row = geo.row_at(point.y, self.scroll)?;
        let lane = view.layer.lanes.get(row)?;
        if matches!(self.tool, Tool::Delete | Tool::Select) {
            return None;
        }
        let time = self.viewport.time(geo.plot, point.x);
        grid::nearest(&lane.hits, time, |t| *t, self.reach(view, time))
            .is_none()
            .then(|| (row, self.snap(view, time)))
    }
}

pub fn surface_id() -> Id {
    Id::new("timeline-editing-surface")
}

pub fn envelope(view: &ProjectView) -> &VolumeEnvelope {
    view.layer
        .file
        .as_deref()
        .and_then(|file| view.project.diff(file))
        .and_then(|diff| diff.volume.as_ref())
        .unwrap_or(&view.project.volume)
}

pub fn edit(emit: Emit<'_>, edit: Edit) {
    emit(Intent::Job(Job::Edit(edit)));
}

pub fn seek(emit: Emit<'_>, time: f64) {
    emit(Intent::Transport(Transport::Seek { ms: time.max(0.0) }));
}

fn set_volume(emit: Emit<'_>, view: &ProjectView, points: Vec<VolumePoint>) {
    edit(
        emit,
        Edit::SetVolume {
            diff: view.layer.file.clone(),
            points: Some(points),
        },
    );
}
