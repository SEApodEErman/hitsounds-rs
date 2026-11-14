//! The mixer dock: one strip per channel with a fader, mute and solo (D-61).
//!
//! A strip edits the same channel volume as the rack knob (D-14). The first strip holds
//! the listening levels (D-77): master, music and effects as in osu!'s options, with
//! mutes for the music and for every channel at once. They change only what you hear.
//! The hit volume itself is the envelope on the timeline (D-44), shown here as a meter.

use super::{
    canvas::{edit, envelope},
    rail::{set_name, sound_name},
};
use crate::{
    app::Intent,
    theme as t,
    views::Emit,
    widgets::{self, Hint},
    worker::Job,
    worker::ProjectView,
};
use eframe::egui::{
    Align2, Color32, FontId, Id, Painter, Rect, Response, ScrollArea, Sense, StrokeKind, Ui,
    WidgetInfo, WidgetType, pos2, text::LayoutJob, text::TextWrapping, vec2,
};
use hs_core::{ChannelId, ChannelProps, Edit};
use hs_session::{Clock, Levels};

/// Width of one listening-level column.
const LEVEL_COLUMN: f32 = 44.0;

#[allow(clippy::too_many_arguments)] // Frame inputs are passed flat, as in the other views.
pub fn show(
    ui: &mut Ui,
    view: &ProjectView,
    levels: &Levels,
    selected: Option<ChannelId>,
    clock: &Clock,
    ready: bool,
    emit: Emit<'_>,
) {
    let height = ui.available_height();
    ScrollArea::horizontal()
        .id_salt("mixer-strips")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t::SMALL;
                listening(ui, levels, height, ready, emit);
                master(ui, view, clock, height);
                for (row, channel) in view.project.channels.iter().enumerate() {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(t::STRIP_WIDTH, height), Sense::click());
                    let color = t::channel_color(&channel.color);
                    let active = selected == Some(channel.id);
                    let p = ui.painter().with_clip_rect(rect);
                    p.rect(
                        rect,
                        t::CONTROL_RADIUS,
                        if active { t::RAISED } else { t::SURFACE },
                        t::stroke(if active { color } else { t::EDGE }),
                        StrokeKind::Inside,
                    );
                    p.rect_filled(
                        Rect::from_min_max(
                            rect.left_top() + vec2(4.0, 4.0),
                            pos2(rect.right() - 4.0, rect.top() + 7.0),
                        ),
                        1,
                        color,
                    );
                    p.text(
                        pos2(rect.left() + 7.0, rect.top() + 12.0),
                        Align2::LEFT_TOP,
                        (row + 1).to_string(),
                        FontId::monospace(t::META_SIZE),
                        t::DIM,
                    );
                    let mut job = LayoutJob::simple_singleline(
                        channel.name.clone(),
                        FontId::proportional(t::META_SIZE + 1.0),
                        t::TEXT,
                    );
                    job.wrap = TextWrapping::truncate_at_width(rect.width() - 10.0);
                    let galley = p.layout_job(job);
                    p.galley(pos2(rect.left() + 6.0, rect.top() + 26.0), galley, t::TEXT);
                    if ready && response.clicked() {
                        emit(Intent::SelectChannel(channel.id));
                    }

                    let foot = rect.bottom() - 54.0;
                    let lane = Rect::from_min_max(
                        pos2(rect.left() + 6.0, rect.top() + 44.0),
                        pos2(rect.right() - 6.0, foot),
                    );
                    let (fader, shown, commit) = widgets::fader(
                        ui,
                        lane,
                        Id::new(("mixer-fader", channel.id)),
                        channel.volume.clamp(0.0, 1.0),
                        1.0,
                        color,
                    );
                    fader.widget_info(|| {
                        WidgetInfo::labeled(
                            WidgetType::Slider,
                            ready,
                            format!("Fader of {}", channel.name),
                        )
                    });
                    if fader.contains_pointer() || fader.dragged() {
                        widgets::hint(
                            ui.ctx(),
                            Hint::new(format!("{} volume {:.0}%", channel.name, shown * 100.0))
                                .key("Drag", "Change")
                                .key("Double-click", "Reset"),
                        );
                    }
                    if ready && let Some(volume) = commit {
                        let mut props = ChannelProps::from(channel);
                        props.volume = volume;
                        edit(
                            emit,
                            Edit::SetChannel {
                                id: channel.id,
                                props,
                            },
                        );
                    }
                    p.text(
                        pos2(rect.center().x, foot + 3.0),
                        Align2::CENTER_TOP,
                        format!("{:.0}%", shown * 100.0),
                        FontId::monospace(t::META_SIZE),
                        t::MUTED,
                    );
                    for (i, (label, on, colour, hint)) in [
                        (
                            "M",
                            channel.muted,
                            t::ERROR,
                            "Mute while listening. Export is not affected",
                        ),
                        (
                            "S",
                            channel.solo,
                            t::WARNING,
                            "Solo: listen to this channel alone",
                        ),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let cell = Rect::from_min_size(
                            pos2(rect.left() + 7.0 + i as f32 * 28.0, foot + 18.0),
                            vec2(26.0, 16.0),
                        );
                        let button = toggle(
                            ui,
                            &p,
                            cell,
                            Id::new(("mixer-toggle", channel.id, label)),
                            label,
                            on,
                            colour,
                        );
                        if ready && widgets::describe(button, hint, "").clicked() {
                            edit(
                                emit,
                                Edit::SetMonitor {
                                    id: channel.id,
                                    muted: channel.muted != (i == 0),
                                    solo: channel.solo != (i == 1),
                                },
                            );
                        }
                    }
                    p.text(
                        pos2(rect.center().x, rect.bottom() - 6.0),
                        Align2::CENTER_BOTTOM,
                        format!(
                            "{} {}",
                            &set_name(channel.sample_set)[..1],
                            sound_name(channel.hitsound)
                        ),
                        FontId::proportional(t::META_SIZE),
                        t::DIM,
                    );
                }
                if view.project.channels.is_empty() {
                    ui.add_space(t::SPACE);
                    ui.label(t::muted("Add a channel to see its mixer strip."));
                }
            });
        });
}

/// A small M or S switch drawn on a strip.
fn toggle(
    ui: &Ui,
    p: &Painter,
    cell: Rect,
    id: Id,
    label: &str,
    on: bool,
    colour: Color32,
) -> Response {
    let button = ui.interact(cell, id, Sense::click());
    let (fill, text) = if on {
        (colour.gamma_multiply(0.24), colour)
    } else if button.hovered() {
        (t::HOVER, t::TEXT)
    } else {
        (t::INSET, t::DIM)
    };
    p.rect_filled(cell, 3, fill);
    p.text(
        cell.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::monospace(t::META_SIZE),
        text,
    );
    button
}

/// Listening levels (D-77): what you hear in the app, never what is exported.
fn listening(ui: &mut Ui, levels: &Levels, height: f32, ready: bool, emit: Emit<'_>) {
    let width = LEVEL_COLUMN * 3.0 + 8.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let p = ui.painter().with_clip_rect(rect);
    p.rect(
        rect,
        t::CONTROL_RADIUS,
        t::STAGE,
        t::stroke(t::EDGE),
        StrokeKind::Inside,
    );
    p.text(
        pos2(rect.left() + 7.0, rect.top() + 12.0),
        Align2::LEFT_TOP,
        "Listening",
        FontId::proportional(t::META_SIZE + 1.0),
        t::TEXT,
    );
    let all_muted = levels.hits_muted;
    p.text(
        pos2(rect.left() + 7.0, rect.top() + 27.0),
        Align2::LEFT_TOP,
        if all_muted {
            "All channels muted"
        } else {
            "Not exported"
        },
        FontId::proportional(t::META_SIZE),
        if all_muted { t::ERROR } else { t::DIM },
    );
    widgets::describe(
        response,
        "Listening levels, like osu!'s volume options. They only change what you hear here",
        "",
    );

    let defaults = Levels::default();
    let foot = rect.bottom() - 54.0;
    let columns = [
        ("Master", levels.master, defaults.master, None),
        (
            "Music",
            levels.music,
            defaults.music,
            Some(levels.music_muted),
        ),
        (
            "Effects",
            levels.effects,
            defaults.effects,
            Some(levels.hits_muted),
        ),
    ];
    for (i, (name, value, reset, muted)) in columns.into_iter().enumerate() {
        let left = rect.left() + 4.0 + i as f32 * LEVEL_COLUMN;
        let centre = left + LEVEL_COLUMN / 2.0;
        let set = |levels: &mut Levels, v: f32| match i {
            0 => levels.master = v,
            1 => levels.music = v,
            _ => levels.effects = v,
        };
        p.text(
            pos2(centre, rect.top() + 46.0),
            Align2::CENTER_TOP,
            name,
            FontId::proportional(t::META_SIZE),
            t::MUTED,
        );
        let lane = Rect::from_min_max(
            pos2(left + 4.0, rect.top() + 60.0),
            pos2(left + LEVEL_COLUMN - 4.0, foot),
        );
        let silent = muted == Some(true);
        let (fader, shown, commit) = widgets::fader(
            ui,
            lane,
            Id::new(("listening-fader", i)),
            value.clamp(0.0, 1.0),
            reset,
            if silent { t::DIM } else { t::MUTED },
        );
        fader.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Slider, ready, format!("{name} volume"))
        });
        if fader.contains_pointer() || fader.dragged() {
            let what = match i {
                0 => "everything you hear",
                1 => "the song or drum stem",
                _ => "hitsounds and sample auditions",
            };
            widgets::hint(
                ui.ctx(),
                Hint::new(format!("{name} volume {:.0}%: {what}", shown * 100.0))
                    .key("Drag", "Change")
                    .key("Double-click", "Reset"),
            );
        }
        // D-87: the readout takes a typed value as well as the fader.
        let (readout, typed) = widgets::percent_entry(
            ui,
            Rect::from_center_size(pos2(centre, foot + 9.0), vec2(LEVEL_COLUMN - 6.0, 16.0)),
            Id::new(("listening-value", i)),
            shown,
            if silent { t::DIM } else { t::MUTED },
        );
        readout.widget_info(|| {
            WidgetInfo::labeled(WidgetType::TextEdit, ready, format!("{name} volume value"))
        });
        if readout.contains_pointer() {
            widgets::hint(
                ui.ctx(),
                Hint::new(format!("{name} volume {:.0}%", shown * 100.0))
                    .key("Click", "Type a value"),
            );
        }
        if ready {
            let mut next = *levels;
            if let Some(v) = commit.or(typed.filter(|v| (v - value).abs() > 0.0005)) {
                set(&mut next, v);
                emit(Intent::Job(Job::Levels(next)));
            } else if fader.dragged() && (shown - value).abs() > 0.0005 {
                set(&mut next, shown);
                emit(Intent::PreviewLevels(next));
            }
        }
        let Some(on) = muted else { continue };
        let cell = Rect::from_center_size(pos2(centre, foot + 26.0), vec2(26.0, 16.0));
        let button = toggle(
            ui,
            &p,
            cell,
            Id::new(("listening-mute", i)),
            "M",
            on,
            t::ERROR,
        );
        let (label, hint) = if i == 1 {
            ("Mute music", "Mute the music while listening")
        } else {
            (
                "Mute all channels",
                "Mute all channels while listening. Their own mute and solo stay as they are",
            )
        };
        button.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ready, on, label));
        if ready && widgets::describe(button, hint, "").clicked() {
            let mut next = *levels;
            if i == 1 {
                next.music_muted = !on;
            } else {
                next.hits_muted = !on;
            }
            emit(Intent::Job(Job::Levels(next)));
        }
    }
}

/// The hit volume at the playhead: read-only, because it is drawn on the timeline.
fn master(ui: &mut Ui, view: &ProjectView, clock: &Clock, height: f32) {
    let (rect, response) = ui.allocate_exact_size(vec2(t::STRIP_WIDTH, height), Sense::hover());
    let value = envelope(view).at(clock.position_ms).clamp(0.0, 100.0);
    let colour = if view.layer.file.is_some() {
        t::DIFFICULTY
    } else {
        t::ACCENT
    };
    let p = ui.painter().with_clip_rect(rect);
    p.rect(
        rect,
        t::CONTROL_RADIUS,
        t::STAGE,
        t::stroke(t::EDGE),
        StrokeKind::Inside,
    );
    p.text(
        pos2(rect.left() + 7.0, rect.top() + 12.0),
        Align2::LEFT_TOP,
        "Hit volume",
        FontId::proportional(t::META_SIZE + 1.0),
        t::TEXT,
    );
    p.text(
        pos2(rect.left() + 7.0, rect.top() + 27.0),
        Align2::LEFT_TOP,
        "at playhead",
        FontId::proportional(t::META_SIZE),
        t::DIM,
    );
    let meter = Rect::from_min_max(
        pos2(rect.center().x - 5.0, rect.top() + 52.0),
        pos2(rect.center().x + 5.0, rect.bottom() - 36.0),
    );
    p.rect_filled(meter, 2, t::INSET);
    let top = meter.bottom() - meter.height() * (value as f32 / 100.0);
    p.rect_filled(
        Rect::from_min_max(pos2(meter.left(), top), meter.max),
        2,
        colour.gamma_multiply(0.8),
    );
    p.text(
        pos2(rect.center().x, rect.bottom() - 28.0),
        Align2::CENTER_TOP,
        format!("{value:.0}%"),
        FontId::monospace(t::META_SIZE),
        colour,
    );
    widgets::describe(
        response,
        "The hit volume curve at the playhead. Draw it on the Hit volume lane of the timeline",
        "",
    );
}
