//! Clipped timeline drawing. Expensive projection and waveform preparation are
//! performed before the frame, and only visible rows, buckets, and hits are drawn.

use super::{
    canvas::{Canvas, Geometry, Timeline, Tool, Viewport, envelope},
    grid,
    model::{IssueKind, Objects},
};
use crate::{theme as t, worker::ProjectView};
use eframe::egui::{
    Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};
use hs_core::{Curve, EdgeKind, HitKind, Ms, Target, VolumeEnvelope};

/// One visible snap step, gathered once per frame for the ruler, the grid and the pads.
struct Step {
    x: f32,
    time: f64,
    alternate: bool,
}

pub fn timeline(ui: &Ui, state: &Timeline, canvas: &Canvas<'_>, hover: Option<Pos2>) {
    let g = canvas.geometry;
    let view = canvas.view;
    let painter = ui.painter().with_clip_rect(g.plot);
    painter.rect_filled(g.plot, 0, t::BASE);
    painter.rect_filled(g.ruler, 0, t::CHROME);
    painter.rect_filled(g.wave, 0, t::STAGE);
    painter.rect_filled(g.volume, 0, t::STAGE);
    painter.rect_filled(g.objects, 0, t::CHROME);

    let from = state.viewport.time(g.plot, g.plot.left());
    let until = state.viewport.time(g.plot, g.plot.right());
    let rows = g.rows(view.project.channels.len(), state.scroll);
    let lanes = painter.with_clip_rect(g.lanes);
    for row in rows.clone() {
        let channel = &view.project.channels[row];
        let area = g.row(row, state.scroll);
        if Some(channel.id) == state.channel {
            lanes.rect_filled(area, 0, t::SURFACE);
        }
        lanes.hline(
            area.x_range(),
            area.bottom() - t::HAIRLINE,
            t::stroke(t::GRID),
        );
    }
    let steps = ruler(&painter, g, state, view, from, until);
    pads(&lanes, g, state, view, &steps, rows.clone());
    waveform(&painter, g, state.viewport, view, from, until);
    let accent = if view.layer.file.is_some() {
        t::DIFFICULTY
    } else {
        t::ACCENT
    };
    volume(
        &painter,
        g,
        state.viewport,
        state.draft().unwrap_or_else(|| envelope(view)),
        (from, until),
        accent,
    );
    if let Some(objects) = &view.layer.objects {
        object_lane(&painter, g, state, objects, from, until);
    }
    for strip in [g.ruler, g.wave, g.volume, g.objects] {
        if strip.height() > 0.0 {
            painter.hline(
                strip.x_range(),
                strip.bottom() - t::HAIRLINE,
                t::stroke(t::EDGE),
            );
        }
    }
    let hovered = hover.and_then(|point| state.hovered_hit(canvas, point));
    for row in rows {
        hit_row(&lanes, g, state, canvas, row, (from, until), hovered);
    }
    let width = |time: Ms| state.pad_width(view, f64::from(time)).unwrap_or(MARK_WIDTH);
    if let Some(point) = hover
        && let Some((row, time)) = state.ghost(view, g, point)
    {
        let pos = pos2(
            state.viewport.x(g.plot, f64::from(time)),
            g.row(row, state.scroll).center().y,
        );
        let color = t::channel_color(&view.project.channels[row].color);
        hit(
            &lanes,
            pos,
            width(time),
            color.gamma_multiply(0.55),
            Look::Outline,
            0.0,
        );
    }
    if let Some((row, times)) = state.painting() {
        let color = t::channel_color(&view.project.channels[row].color);
        let y = g.row(row, state.scroll).center().y;
        for &time in times {
            let x = state.viewport.x(g.plot, f64::from(time));
            if g.plot.x_range().contains(x) {
                hit(
                    &lanes,
                    pos2(x, y),
                    width(time),
                    color.gamma_multiply(0.7),
                    Look::Solid,
                    0.0,
                );
            }
        }
    }
    if let Some((delta, rows)) = state.move_delta() {
        for (id, time) in state.moved(view, delta, rows) {
            let x = state.viewport.x(g.plot, f64::from(time));
            if !g.plot.x_range().contains(x) {
                continue;
            }
            let row = view.layer.rows[&id];
            let rect = g.row(row, state.scroll);
            if !rect.intersects(g.lanes) {
                continue;
            }
            hit(
                &lanes,
                pos2(x, rect.center().y),
                width(time),
                t::ACCENT,
                Look::Selected,
                0.0,
            );
        }
    }
    if let Some(rect) = state.marquee() {
        lanes.rect(
            rect,
            2,
            t::ACCENT.gamma_multiply(0.13),
            t::stroke(t::ACCENT),
            StrokeKind::Inside,
        );
    }
    if view.project.channels.is_empty() {
        notice(
            &lanes,
            g,
            "Add a channel on the left to start placing hits.",
        );
    } else if view.layer.lanes.iter().all(|lane| lane.hits.is_empty()) {
        notice(
            &lanes,
            g,
            "No hits yet. Click a step to place one, or press Space to listen first.",
        );
    }
    let x = state.viewport.x(g.plot, canvas.clock.position_ms);
    if g.plot.x_range().contains(x) {
        painter.vline(x, g.plot.top()..=g.lanes.bottom(), t::stroke(accent));
        painter.add(Shape::convex_polygon(
            vec![
                pos2(x - 5.0, g.plot.top()),
                pos2(x + 5.0, g.plot.top()),
                pos2(x, g.plot.top() + t::GAP),
            ],
            accent,
            Stroke::NONE,
        ));
    }
}

fn notice(painter: &Painter, g: Geometry, text: &str) {
    let galley = painter.layout_no_wrap(text.into(), FontId::proportional(t::BODY_SIZE), t::MUTED);
    let center = pos2(g.lanes.center().x, g.lanes.top() + 64.0);
    let rect = Rect::from_center_size(center, galley.size() + vec2(24.0, 14.0));
    painter.rect(
        rect,
        t::CARD_RADIUS,
        t::SURFACE,
        t::stroke(t::EDGE),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(12.0, 7.0), galley, t::MUTED);
}

fn ruler(
    painter: &Painter,
    g: Geometry,
    state: &Timeline,
    view: &ProjectView,
    from: f64,
    until: f64,
) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut last_label_x = f32::NEG_INFINITY;
    let padded = state.pad_width(view, from).is_some();
    view.layer
        .grid
        .ticks(from, until, state.viewport.scale, state.divisor, |tick| {
            let x = state.viewport.x(g.plot, tick.time);
            let (length, ruler_alpha) = if tick.bar.is_some() {
                (g.ruler.height() - 2.0, 0.65)
            } else if tick.beat {
                (10.0, 0.44)
            } else {
                (5.0, 0.78)
            };
            painter.vline(
                x,
                (g.ruler.bottom() - length)..=g.ruler.bottom(),
                t::stroke(tick.color.gamma_multiply(ruler_alpha)),
            );
            // The pads already show every step, so the lanes keep only bar and beat lines.
            let line = if tick.bar.is_some() {
                Some(t::EDGE_STRONG.gamma_multiply(0.75))
            } else if tick.beat {
                Some(t::EDGE.gamma_multiply(0.7))
            } else if padded {
                None
            } else {
                Some(tick.color.gamma_multiply(0.07))
            };
            if let Some(color) = line {
                let top = if tick.every && padded && tick.bar.is_none() {
                    // Between pads the beat line would cut through a pad: keep it above the lanes.
                    g.ruler.bottom()..=g.lanes.top()
                } else {
                    g.ruler.bottom()..=g.lanes.bottom()
                };
                painter.vline(x, top, t::stroke(color));
            }
            if let Some(bar) = tick.bar
                && x - last_label_x >= 31.0
            {
                painter.text(
                    pos2(x + t::SMALL, g.ruler.top() + 2.0),
                    Align2::LEFT_TOP,
                    bar,
                    FontId::monospace(t::META_SIZE),
                    t::MUTED,
                );
                last_label_x = x;
            }
            if tick.every {
                steps.push(Step {
                    x,
                    time: tick.time,
                    alternate: tick.alternate,
                });
            }
        });
    steps
}

/// The unlit step pads of the channel rack: one per snap step, shaded by beat (D-61).
fn pads(
    painter: &Painter,
    g: Geometry,
    state: &Timeline,
    view: &ProjectView,
    steps: &[Step],
    rows: std::ops::Range<usize>,
) {
    let mut mesh = Mesh::default();
    for step in steps {
        let Some(width) = state.pad_width(view, step.time) else {
            continue;
        };
        let color = if step.alternate { t::PAD_ALT } else { t::PAD };
        for row in rows.clone() {
            let area = g.row(row, state.scroll);
            mesh.add_colored_rect(
                Rect::from_center_size(
                    pos2(step.x, area.center().y),
                    vec2(width, area.height() - t::PAD_INSET * 2.0),
                ),
                color,
            );
        }
    }
    if !mesh.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
}

fn waveform(
    painter: &Painter,
    g: Geometry,
    viewport: Viewport,
    view: &ProjectView,
    from: f64,
    until: f64,
) {
    let wave = painter.with_clip_rect(g.wave);
    let y = g.wave.center().y;
    wave.hline(g.wave.x_range(), y, t::stroke(t::GRID));
    let Some((bucket, pairs)) = view.waveform.at_scale(1.0 / viewport.scale) else {
        wave.text(
            g.wave.center(),
            Align2::CENTER_CENTER,
            "No waveform available",
            FontId::proportional(t::META_SIZE),
            t::DIM,
        );
        return;
    };
    let start = (from / bucket).floor() as usize;
    let end = ((until / bucket).ceil() as usize + 1).min(pairs.len());
    let amplitude = (g.wave.height() * 0.46) / f32::from(i8::MAX);
    let width = (viewport.scale * bucket).clamp(1.0, 2.0) as f32;
    let mut mesh = Mesh::default();
    // Neutral, so the accent stays on the Hit volume curve below it (D-92).
    let color = t::WAVE;
    for (i, &[lo, hi]) in pairs.iter().enumerate().take(end).skip(start) {
        let x = viewport.x(g.plot, i as f64 * bucket);
        let top = y - f32::from(hi) * amplitude;
        let bottom = (y - f32::from(lo) * amplitude).max(top + t::HAIRLINE);
        mesh.add_colored_rect(
            Rect::from_min_max(pos2(x, top), pos2(x + width, bottom)),
            color,
        );
    }
    wave.add(Shape::mesh(mesh));
}

fn volume(
    painter: &Painter,
    g: Geometry,
    viewport: Viewport,
    envelope: &VolumeEnvelope,
    (from, until): (f64, f64),
    color: Color32,
) {
    // D-92: the lane must not read as part of the waveform above it. It gets its own
    // tinted ground and edge, a filled band under a bold curve, and scale labels, so
    // even the flat 100 % curve of a new project is plainly a control.
    let p = painter.with_clip_rect(g.volume);
    p.rect_filled(g.volume, 0, color.gamma_multiply(0.04));
    p.hline(
        g.volume.x_range(),
        g.volume.top() + 0.5,
        Stroke::new(1.0_f32, color.gamma_multiply(0.45)),
    );
    let y = |value: f64| g.volume_y(value);
    let point = |time: f64, value: f64| pos2(viewport.x(g.plot, time), y(value));
    for value in [50.0, 100.0] {
        p.hline(g.volume.x_range(), y(value), t::stroke(t::GRID));
    }
    let floor = g.volume.bottom();
    let fill = color.gamma_multiply(0.16);
    let mut area = Mesh::default();
    let mut segment = |a: Pos2, b: Pos2| {
        let base = area.vertices.len() as u32;
        for corner in [a, b, pos2(b.x, floor), pos2(a.x, floor)] {
            area.colored_vertex(corner, fill);
        }
        area.add_triangle(base, base + 1, base + 2);
        area.add_triangle(base, base + 2, base + 3);
        p.line_segment([a, b], Stroke::new(2.0_f32, color));
    };
    let points = &envelope.points;
    let visible = grid::visible(points, from, until, |point| point.time);
    let mut last = point(from, envelope.at(from));
    let mut nodes = Vec::new();
    for i in visible {
        let node = points[i];
        let at = point(f64::from(node.time), f64::from(node.volume));
        if i > 0 && points[i - 1].curve == Curve::Step {
            let corner = pos2(at.x, last.y);
            segment(last, corner);
            segment(corner, at);
        } else {
            segment(last, at);
        }
        nodes.push(at);
        last = at;
    }
    segment(last, point(until, envelope.at(until)));
    p.add(Shape::mesh(area));
    for at in nodes {
        p.circle(at, 4.0, t::STAGE, Stroke::new(2.0_f32, color));
    }
    let label = |value: f64, text: &str| {
        p.text(
            pos2(g.plot.right() - t::GAP, y(value) + 1.0),
            Align2::RIGHT_TOP,
            text,
            FontId::monospace(t::META_SIZE - 1.0),
            t::DIM,
        );
    };
    label(100.0, "100%");
    label(50.0, "50%");
    if envelope.points.is_empty() {
        // A new project's curve: say what it is and how to change it.
        p.text(
            pos2(g.plot.left() + t::SPACE, (y(100.0) + floor) / 2.0),
            Align2::LEFT_CENTER,
            "Every hit at full volume. Ctrl+click here to add a volume point",
            FontId::proportional(t::META_SIZE),
            t::TEXT.gamma_multiply(0.8),
        );
    }
}

pub fn hit_color(kind: HitKind) -> Color32 {
    match kind {
        HitKind::Matched => t::MATCHED,
        HitKind::Fallback => t::FALLBACK,
        HitKind::Muted => t::MUTED_HIT,
        HitKind::Empty => t::EMPTY_HIT,
    }
}

pub fn issue_color(kind: IssueKind) -> Color32 {
    match kind {
        IssueKind::Empty => t::EMPTY_HIT,
        IssueKind::Fallback => t::FALLBACK,
        IssueKind::Orphaned => t::ORPHAN_HIT,
        IssueKind::Tick => t::TICK_HIT,
    }
}

fn object_lane(
    painter: &Painter,
    g: Geometry,
    state: &Timeline,
    objects: &Objects,
    from: f64,
    until: f64,
) {
    let p = painter.with_clip_rect(g.objects);
    let y = g.objects.center().y;
    let x = |ms| state.viewport.x(g.plot, f64::from(ms));
    let mut last_x = f32::NEG_INFINITY;
    for i in grid::visible(&objects.timeline.ticks, from, until, |tick| tick.time) {
        let x = x(objects.timeline.ticks[i].time);
        let selected = state.object == Some(Target::Tick(i));
        let custom = objects.ticks[i];
        if x - last_x < 2.0 && !custom && !selected {
            continue;
        }
        let color = if custom || selected {
            t::TICK_HIT
        } else {
            t::DIM.gamma_multiply(0.35)
        };
        p.circle_filled(pos2(x, y + 9.0), if selected { 3.5 } else { 1.8 }, color);
        last_x = x;
    }
    last_x = f32::NEG_INFINITY;
    for i in grid::visible(&objects.timeline.edges, from, until, |edge| edge.time) {
        let edge = objects.timeline.edges[i];
        let x = x(edge.time);
        let selected = state.object == Some(Target::Edge(i));
        if x - last_x < 2.5 && !selected {
            continue;
        }
        let pos = pos2(x, y);
        let kind = objects.edges[i];
        let color = hit_color(kind);
        let r = if selected { 6.5 } else { 5.0 };
        let stroke = if selected {
            Stroke::new(1.5_f32, t::TEXT)
        } else {
            t::stroke(color.gamma_multiply(0.8))
        };
        if edge.kind == EdgeKind::Circle {
            p.circle(pos, r, t::STAGE, stroke);
        } else {
            p.add(Shape::convex_polygon(
                vec![
                    pos + vec2(0.0, -r),
                    pos + vec2(r, 0.0),
                    pos + vec2(0.0, r),
                    pos + vec2(-r, 0.0),
                ],
                t::STAGE,
                stroke,
            ));
        }
        let line = |a: (f32, f32), b: (f32, f32)| {
            p.line_segment(
                [pos + vec2(a.0, a.1), pos + vec2(b.0, b.1)],
                t::stroke(color),
            );
        };
        match kind {
            HitKind::Matched => {
                line((-2.5, 0.0), (-0.5, 2.0));
                line((-0.5, 2.0), (2.5, -2.0));
            }
            HitKind::Fallback => {
                line((-1.5, -2.0), (1.5, 0.0));
                line((1.5, 0.0), (-1.5, 2.0));
            }
            HitKind::Muted => line((-2.5, 0.0), (2.5, 0.0)),
            HitKind::Empty => {
                line((-2.0, -2.0), (2.0, 2.0));
                line((-2.0, 2.0), (2.0, -2.0));
            }
        }
        last_x = x;
    }
}

fn hit_row(
    painter: &Painter,
    g: Geometry,
    state: &Timeline,
    canvas: &Canvas<'_>,
    row: usize,
    (from, until): (f64, f64),
    hovered: Option<(hs_core::ChannelId, Ms)>,
) {
    let view = canvas.view;
    let channel = &view.project.channels[row];
    let lane = &view.layer.lanes[row];
    let y = g.row(row, state.scroll).center().y;
    let color = t::channel_color(&channel.color);
    let monitor_color = if channel.muted {
        color.gamma_multiply(0.35)
    } else {
        color
    };
    let x = |ms| state.viewport.x(g.plot, f64::from(ms));
    let width = |time: Ms| state.pad_width(view, f64::from(time)).unwrap_or(MARK_WIDTH);
    // D-55: at song scale the markers occupy only a few pixels. A single mesh per
    // visible row avoids thousands of rounded-rectangle tessellations per frame.
    let dense = state.viewport.scale < 0.045;
    let batch = dense.then(|| painter.add(Shape::Noop));
    let mut mesh = Mesh::default();
    let erasing = state.erasing();
    let moving = state.move_delta().is_some();
    let mut last_x = f32::NEG_INFINITY;
    for &time in &lane.removed[grid::visible(&lane.removed, from, until, |t| *t)] {
        let x = x(time);
        if x - last_x >= 2.0 {
            let ghost_color = color.gamma_multiply(0.3);
            if dense {
                compact_hit(&mut mesh, pos2(x, y), ghost_color, true);
            } else {
                hit(
                    painter,
                    pos2(x, y),
                    width(time),
                    ghost_color,
                    Look::Outline,
                    0.0,
                );
            }
            last_x = x;
        }
    }
    last_x = f32::NEG_INFINITY;
    for &time in &lane.hits[grid::visible(&lane.hits, from, until, |t| *t)] {
        let x = x(time);
        let key = (channel.id, time);
        let selected = state.selection.contains(&key);
        let doomed = erasing.is_some_and(|hits| hits.contains(&key))
            || state.tool == Tool::Delete && hovered == Some(key);
        let age = canvas.clock.position_ms - f64::from(time);
        let pulse = if !canvas.reduced && canvas.clock.playing && age >= 0.0 {
            (1.0 - age / (t::HIT_PULSE_MS * canvas.clock.rate.max(0.25))).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        if x - last_x < 2.0 && !selected && !doomed && pulse == 0.0 {
            continue;
        }
        let outline = view.layer.file.is_some() && channel.events.binary_search(&time).is_err();
        let color = if selected && moving || doomed {
            monitor_color.gamma_multiply(0.3)
        } else {
            monitor_color
        };
        if dense && !selected && !doomed && pulse == 0.0 {
            compact_hit(&mut mesh, pos2(x, y), color, outline);
        } else {
            let look = if doomed {
                Look::Doomed
            } else if selected {
                Look::Selected
            } else if outline {
                Look::Added
            } else {
                Look::Solid
            };
            hit(painter, pos2(x, y), width(time), color, look, pulse);
        }
        if view.layer.orphans.contains(&key) {
            painter.circle_filled(pos2(x, y - 12.0), 1.8, t::ORPHAN_HIT);
        }
        last_x = x;
    }
    if let Some(batch) = batch {
        painter.set(batch, Shape::mesh(mesh));
    }
}

/// The width of a hit marker when the view is too far out for step pads.
const MARK_WIDTH: f32 = 5.0;

fn compact_hit(mesh: &mut Mesh, at: Pos2, color: Color32, outline: bool) {
    let width = if outline { 3.0 } else { 1.4 };
    let rect = Rect::from_center_size(at, vec2(width, 16.0));
    if outline {
        let border = t::HAIRLINE;
        for part in [
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + border)),
            Rect::from_min_max(pos2(rect.left(), rect.bottom() - border), rect.max),
            Rect::from_min_max(rect.min, pos2(rect.left() + border, rect.bottom())),
            Rect::from_min_max(pos2(rect.right() - border, rect.top()), rect.max),
        ] {
            mesh.add_colored_rect(part, color);
        }
    } else {
        mesh.add_colored_rect(rect, color);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    /// A lit pad in the channel colour.
    Solid,
    /// A hit that exists only in this difficulty (D-50): tinted, with an outline.
    Added,
    /// A placement preview, or a base hit removed in this difficulty.
    Outline,
    Selected,
    /// About to be erased.
    Doomed,
}

fn hit(painter: &Painter, position: Pos2, width: f32, color: Color32, look: Look, pulse: f32) {
    let rect = Rect::from_center_size(position, vec2(width, t::ROW_HEIGHT - t::PAD_INSET * 2.0));
    if pulse > 0.0 {
        painter.rect_filled(
            rect.expand(1.0 + pulse * 4.0),
            t::SMALL,
            color.gamma_multiply(pulse * 0.3),
        );
    }
    let (fill, border) = match look {
        Look::Solid => (color, Stroke::NONE),
        Look::Added => (color.gamma_multiply(0.22), t::stroke(color)),
        Look::Outline => (Color32::TRANSPARENT, t::stroke(color)),
        Look::Selected => (color, Stroke::new(1.5_f32, t::TEXT)),
        Look::Doomed => (color, t::stroke(t::ERROR)),
    };
    painter.rect(rect, 2, fill, border, StrokeKind::Inside);
    if look == Look::Solid && width >= 8.0 {
        // A lit top edge, like the cap of a hardware step button.
        painter.hline(
            (rect.left() + 1.5)..=(rect.right() - 1.5),
            rect.top() + 1.5,
            t::stroke(Color32::from_white_alpha(70)),
        );
    }
}
