//! The channel rack: one compact row per channel with its mute light, solo, volume
//! knob, name button and osu! slot, plus the labels of the lanes above it (D-61).

use super::{
    Act, Panel,
    canvas::{Canvas, edit},
    grid::timecode,
};
use crate::{
    app::Intent,
    keys::{self, Action},
    theme as t,
    views::Emit,
    widgets::{self, Hint, Segment},
    worker::{Job, ProjectView},
};
use eframe::egui::{
    self, Align, Align2, Button, Color32, CursorIcon, FontId, Id, Key, Layout, Popup, Rect,
    Response, Sense, StrokeKind, TextEdit, TextFormat, Ui, UiBuilder, WidgetInfo, WidgetType, pos2,
    text::{LayoutJob, TextWrapping},
    vec2,
};
use egui_phosphor::regular as icons;
use hs_core::{
    Channel, ChannelId, ChannelProps, EdgeKind, Edit, Override, SampleSource, Target, TimeRange,
    project::Monitor,
};
use osu_format::{Hitsound, SampleSet};
use std::path::PathBuf;

pub const SETS: [SampleSet; 3] = [SampleSet::Normal, SampleSet::Soft, SampleSet::Drum];

#[derive(Clone)]
pub enum DragItem {
    Channel(ChannelId),
    Sample(PathBuf),
}

#[derive(Default)]
pub struct Rail {
    rename: Option<Rename>,
}

struct Rename {
    id: ChannelId,
    draft: String,
    focus: bool,
}

/// What the rack needs to know about the timeline it sits beside.
pub(super) struct RailState {
    pub scroll: f32,
    pub selected: Option<ChannelId>,
    pub range: Option<TimeRange>,
    pub follow: bool,
    pub wave: bool,
}

impl Rail {
    pub fn start_rename(&mut self, channel: &Channel) {
        self.rename = Some(Rename {
            id: channel.id,
            draft: channel.name.clone(),
            focus: true,
        });
    }

    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        canvas: &Canvas<'_>,
        state: &RailState,
        emit: Emit<'_>,
    ) {
        let g = canvas.geometry;
        let view = canvas.view;
        ui.painter().rect_filled(g.rail, 0, t::CHROME);
        self.header(ui, canvas, state, emit);
        let bounds = Rect::from_min_max(pos2(g.rail.left(), g.lanes.top()), g.rail.max);
        let any_solo = view.project.channels.iter().any(|channel| channel.solo);
        for row in g.rows(view.project.channels.len() + 1, state.scroll) {
            let rect = g.row(row, state.scroll);
            let rect = Rect::from_min_max(
                pos2(g.rail.left(), rect.top()),
                pos2(g.rail.right() - t::HAIRLINE, rect.bottom()),
            );
            if let Some(channel) = view.project.channels.get(row) {
                let response = ui.interact(
                    rect.intersect(bounds),
                    Id::new(("rail-row", channel.id)),
                    Sense::click(),
                );
                let fill = if response.dnd_hover_payload::<DragItem>().is_some() {
                    t::ACCENT_SURFACE
                } else if state.selected == Some(channel.id) {
                    t::RAISED
                } else {
                    t::SURFACE
                };
                let p = ui.painter().with_clip_rect(bounds);
                p.rect_filled(rect, 0, fill);
                p.hline(
                    rect.x_range(),
                    rect.bottom() - t::HAIRLINE,
                    t::stroke(t::GRID),
                );
                if canvas.ready && response.clicked() {
                    emit(Intent::SelectChannel(channel.id));
                }
                if canvas.ready {
                    response.context_menu(|ui| channel_menu(ui, view, row, state.range, emit));
                    if let Some(item) = response.dnd_release_payload::<DragItem>() {
                        match &*item {
                            DragItem::Channel(id) if *id != channel.id => {
                                edit(emit, Edit::MoveChannel { id: *id, to: row })
                            }
                            DragItem::Sample(path) => emit(Intent::Job(Job::UseSample {
                                channel: Some(channel.id),
                                path: Some(path.clone()),
                            })),
                            _ => {}
                        }
                    }
                }
                ui.push_id(channel.id, |ui| {
                    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                        ui.set_clip_rect(bounds.intersect(rect));
                        ui.add_enabled_ui(canvas.ready, |ui| {
                            self.row(ui, canvas, row, state, any_solo, rect, emit)
                        });
                    });
                });
            } else {
                ui.scope_builder(
                    UiBuilder::new()
                        .id_salt("add-channel")
                        .max_rect(rect.shrink2(vec2(t::GAP, 2.0))),
                    |ui| {
                        ui.set_clip_rect(bounds);
                        let response = ui.add_enabled(
                            canvas.ready,
                            Button::new((
                                t::icon(icons::PLUS).color(t::MUTED),
                                t::muted("Add channel"),
                            ))
                            .frame_when_inactive(false)
                            .min_size(vec2(ui.available_width(), t::CONTROL_HEIGHT)),
                        );
                        if response.clicked() {
                            emit(Intent::Job(Job::AddChannel));
                        }
                        if canvas.ready
                            && let Some(item) = response.dnd_release_payload::<DragItem>()
                            && let DragItem::Sample(path) = &*item
                        {
                            emit(Intent::Job(Job::UseSample {
                                channel: None,
                                path: Some(path.clone()),
                            }));
                        }
                        widgets::describe(
                            response,
                            "Add a channel. Drop a sample here to create one from it",
                            "",
                        );
                    },
                );
            }
        }
        ui.painter().vline(
            g.rail.right() - t::HAIRLINE,
            g.rail.y_range(),
            t::stroke(t::EDGE),
        );
    }

    /// The labels and switches beside the ruler, waveform, volume and objects lanes.
    fn header(&self, ui: &mut Ui, canvas: &Canvas<'_>, state: &RailState, emit: Emit<'_>) {
        let g = canvas.geometry;
        let view = canvas.view;
        let ready = canvas.ready;
        let left = g.rail.left() + t::SPACE;
        let right = g.rail.right() - t::GAP;
        let p = ui.painter().with_clip_rect(g.rail);
        let title = |rect: Rect, label: &str, colour: Color32| {
            p.text(
                pos2(left, rect.top() + 6.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(12.0),
                colour,
            );
        };
        let note = |rect: Rect, label: &str, colour: Color32| {
            p.text(
                pos2(left, rect.top() + 24.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(t::META_SIZE),
                colour,
            );
        };
        let cell = |rect: Rect, width: f32| {
            Rect::from_min_max(
                pos2(right - width, rect.top()),
                pos2(right, rect.bottom() - t::HAIRLINE),
            )
        };

        // Ruler row: what the rows below are, and the view controls of the timeline.
        let count = view.project.channels.len();
        p.text(
            pos2(left, g.ruler.center().y),
            Align2::LEFT_CENTER,
            "Channels",
            FontId::proportional(12.0),
            t::TEXT,
        );
        p.text(
            pos2(left + 60.0, g.ruler.center().y),
            Align2::LEFT_CENTER,
            count.to_string(),
            FontId::monospace(t::META_SIZE),
            t::DIM,
        );
        let info = ui.interact(
            Rect::from_min_max(
                pos2(g.rail.left(), g.ruler.top()),
                pos2(left + 84.0, g.ruler.bottom()),
            ),
            Id::new("rail-priority-note"),
            Sense::hover(),
        );
        widgets::describe(
            info,
            "Row order is priority: when channels share a hit, the higher row decides the sample set. Drag a row to reorder",
            "",
        );
        let mut x = right;
        for (glyph, label, shortcut, on, act) in [
            (
                icons::ARROWS_OUT_LINE_HORIZONTAL,
                "Fit the whole song",
                "",
                false,
                Act::Fit,
            ),
            (
                icons::MAGNIFYING_GLASS_PLUS,
                "Zoom in",
                "Alt+Wheel",
                false,
                Act::ZoomIn,
            ),
            (
                icons::MAGNIFYING_GLASS_MINUS,
                "Zoom out",
                "Alt+Wheel",
                false,
                Act::ZoomOut,
            ),
            (
                icons::CROSSHAIR_SIMPLE,
                "Follow the playhead while it moves",
                "",
                state.follow,
                Act::Follow,
            ),
            (
                icons::WAVEFORM,
                "Show the song waveform",
                keys::label(Action::ToggleWaveform),
                state.wave,
                Act::Waveform,
            ),
        ] {
            let rect = Rect::from_center_size(pos2(x - 11.0, g.ruler.center().y), vec2(22.0, 20.0));
            x -= 24.0;
            if mini(
                ui,
                rect,
                Id::new(("rail-view", label)),
                glyph,
                on,
                label,
                shortcut,
            )
            .clicked()
            {
                emit(Intent::Editor(act));
            }
        }

        // Waveform row: which audio you hear behind the hitsounds.
        let stems = view.project.audio.monitor == Monitor::Stems && view.stem_ready;
        title(g.wave, "Listen to", t::TEXT);
        if state.wave {
            note(
                g.wave,
                if stems {
                    "Drum stem waveform"
                } else {
                    "Song waveform"
                },
                t::DIM,
            );
        }
        ui.scope_builder(
            UiBuilder::new()
                .id_salt("monitor-switch")
                .max_rect(cell(g.wave, 102.0))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.add_enabled_ui(ready, |ui| {
                    let current = if stems {
                        Monitor::Stems
                    } else {
                        Monitor::Original
                    };
                    let picked = widgets::segmented(
                        ui,
                        current,
                        &[
                            Segment {
                                value: Monitor::Original,
                                icon: "",
                                label: "Song",
                                hint: "Play the full song behind the hitsounds",
                                shortcut: keys::label(Action::ToggleMonitor),
                                enabled: true,
                            },
                            Segment {
                                value: Monitor::Stems,
                                icon: "",
                                label: "Stem",
                                hint: if view.stem_ready {
                                    "Play only the drum stem behind the hitsounds"
                                } else {
                                    "No drum stem yet. Separate or import one in File, Project settings"
                                },
                                shortcut: keys::label(Action::ToggleMonitor),
                                enabled: view.stem_ready,
                            },
                        ],
                        true,
                    );
                    if let Some(source) = picked {
                        emit(Intent::Job(Job::Monitor(source)));
                    }
                });
            },
        );

        // Volume row: the envelope every hit's volume follows (D-44).
        let own = view
            .layer
            .file
            .as_deref()
            .and_then(|file| view.project.diff(file))
            .filter(|diff| diff.volume.is_some());
        title(g.volume, "Hit volume", t::TEXT);
        note(
            g.volume,
            if own.is_some() {
                "This difficulty's own curve"
            } else {
                "One curve for all difficulties"
            },
            if own.is_some() { t::DIFFICULTY } else { t::DIM },
        );
        if own.is_some() {
            ui.scope_builder(
                UiBuilder::new()
                    .id_salt("volume-shared")
                    .max_rect(cell(g.volume, 96.0))
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| {
                    let response = ui.add_enabled(ready, Button::new("Use shared"));
                    if widgets::describe(
                        response,
                        "Drop this difficulty's own volume curve and follow the shared one again",
                        "",
                    )
                    .clicked()
                    {
                        edit(
                            emit,
                            Edit::SetVolume {
                                diff: view.layer.file.clone(),
                                points: None,
                            },
                        );
                    }
                },
            );
        }

        // Objects row: only in a difficulty layer (D-50).
        if g.objects.height() > 0.0 {
            p.text(
                pos2(left, g.objects.center().y),
                Align2::LEFT_CENTER,
                "Objects",
                FontId::proportional(12.0),
                t::DIFFICULTY,
            );
            let mut x = right;
            for (mark, label, colour) in [
                (icons::DOT, "Slider tick with a custom sound", t::TICK_HIT),
                (icons::X, "Empty: no hitsound lands here", t::EMPTY_HIT),
                (icons::MINUS, "Muted slider end", t::MUTED_HIT),
                (
                    icons::CARET_RIGHT,
                    "Fallback: filled by the fallback channel",
                    t::FALLBACK,
                ),
                (
                    icons::CHECK,
                    "Matched: a hit lands on this object",
                    t::MATCHED,
                ),
            ] {
                let rect =
                    Rect::from_center_size(pos2(x - 9.0, g.objects.center().y), vec2(18.0, 18.0));
                x -= 19.0;
                let response = ui.interact(rect, Id::new(("legend", label)), Sense::hover());
                p.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    mark,
                    FontId::new(12.0, t::icon_family()),
                    colour,
                );
                widgets::describe(response, label, "");
            }
        }
        for y in [
            g.ruler.bottom(),
            g.wave.bottom(),
            g.volume.bottom(),
            g.objects.bottom(),
        ] {
            p.hline(g.rail.x_range(), y - t::HAIRLINE, t::stroke(t::EDGE));
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a rack row shares the row geometry and the edit context"
    )]
    fn row(
        &mut self,
        ui: &mut Ui,
        canvas: &Canvas<'_>,
        row: usize,
        state: &RailState,
        any_solo: bool,
        rect: Rect,
        emit: Emit<'_>,
    ) {
        let view = canvas.view;
        let channel = &view.project.channels[row];
        let color = t::channel_color(&channel.color);
        let y = rect.center().y;
        let mut x = rect.left() + t::SMALL;
        let mut take = |width: f32, height: f32| {
            let cell = Rect::from_center_size(pos2(x + width / 2.0, y), vec2(width, height));
            x += width + 2.0;
            cell
        };

        let grip = ui.interact(take(14.0, 22.0), ui.id().with("grip"), Sense::drag());
        grip.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Other,
                ui.is_enabled(),
                format!("Reorder {}", channel.name),
            )
        });
        ui.painter().text(
            grip.rect.center(),
            Align2::CENTER_CENTER,
            icons::DOTS_SIX_VERTICAL,
            FontId::new(14.0, t::icon_family()),
            if grip.hovered() {
                t::MUTED
            } else {
                t::EDGE_STRONG
            },
        );
        grip.dnd_set_drag_payload(DragItem::Channel(channel.id));
        widgets::describe(
            grip.on_hover_cursor(CursorIcon::Grab),
            "Drag to change priority. Higher rows win when channels share a hit",
            "",
        );

        // Mute light: lit while the channel is audible, flashing as its hits play.
        let audible = !canvas.all_muted && !channel.muted && (!any_solo || channel.solo);
        let glow = if audible && canvas.clock.playing && !canvas.reduced {
            let hits = &view.layer.lanes[row].hits;
            let played = hits.partition_point(|&time| f64::from(time) <= canvas.clock.position_ms);
            played.checked_sub(1).map_or(0.0, |i| {
                let age = canvas.clock.position_ms - f64::from(hits[i]);
                (1.0 - age / (t::HIT_PULSE_MS * canvas.clock.rate.max(0.25))).clamp(0.0, 1.0) as f32
            })
        } else {
            0.0
        };
        let led = widgets::led(
            ui,
            take(16.0, 22.0),
            ui.id().with("mute"),
            audible,
            glow,
            t::ACCENT,
        );
        led.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::Button,
                ui.is_enabled(),
                channel.muted,
                format!("Mute {}", channel.name),
            )
        });
        if led.clicked() || led.secondary_clicked() {
            let solo = led.secondary_clicked();
            edit(
                emit,
                Edit::SetMonitor {
                    id: channel.id,
                    muted: if solo { channel.muted } else { !channel.muted },
                    solo: if solo { !channel.solo } else { channel.solo },
                },
            );
        }
        if led.contains_pointer() {
            widgets::hint(
                ui.ctx(),
                Hint::new(if channel.muted {
                    "Muted while listening. Export is not affected"
                } else {
                    "Audible"
                })
                .key("Click", "Mute")
                .key("Right-click", "Solo"),
            );
        }

        let solo = ui.interact(take(18.0, 18.0), ui.id().with("solo"), Sense::click());
        solo.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::Button,
                ui.is_enabled(),
                channel.solo,
                format!("Solo {}", channel.name),
            )
        });
        let (fill, text) = if channel.solo {
            (t::WARNING.gamma_multiply(0.22), t::WARNING)
        } else if solo.hovered() {
            (t::HOVER, t::TEXT)
        } else {
            (Color32::TRANSPARENT, t::DIM)
        };
        ui.painter().rect_filled(solo.rect, t::SMALL, fill);
        ui.painter().text(
            solo.rect.center(),
            Align2::CENTER_CENTER,
            "S",
            FontId::monospace(t::META_SIZE),
            text,
        );
        if widgets::describe(solo, "Solo: listen to this channel alone", "").clicked() {
            edit(
                emit,
                Edit::SetMonitor {
                    id: channel.id,
                    muted: channel.muted,
                    solo: !channel.solo,
                },
            );
        }

        x += 2.0;
        let (knob, shown, commit) = widgets::knob(
            ui,
            Rect::from_center_size(pos2(x + 11.0, y), vec2(22.0, 22.0)),
            ui.id().with("volume"),
            channel.volume.clamp(0.0, 1.0),
            1.0,
            color,
        );
        x += 22.0 + t::GAP;
        knob.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Slider,
                ui.is_enabled(),
                format!("Volume of {}", channel.name),
            )
        });
        if knob.contains_pointer() || knob.dragged() {
            widgets::hint(
                ui.ctx(),
                Hint::new(format!("Channel volume {:.0}%", shown * 100.0))
                    .key("Drag", "Change")
                    .key("Shift+Drag", "Fine")
                    .key("Double-click", "Reset"),
            );
        }
        if let Some(volume) = commit {
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

        let wide = rect.width() >= 280.0;
        let chip_width = if wide { 96.0 } else { 0.0 };
        let name = Rect::from_min_max(
            pos2(x, rect.top() + 3.0),
            pos2(
                rect.right() - t::GAP - chip_width - if wide { t::SMALL } else { 0.0 },
                rect.bottom() - 4.0,
            ),
        );
        self.name(ui, view, channel, row, state, name, emit);
        if wide {
            let chip = Rect::from_min_max(
                pos2(rect.right() - t::GAP - chip_width, rect.top() + 5.0),
                pos2(rect.right() - t::GAP, rect.bottom() - 6.0),
            );
            let response = ui.interact(chip, ui.id().with("slot"), Sense::click());
            response.widget_info(|| {
                WidgetInfo::labeled(
                    WidgetType::Button,
                    ui.is_enabled(),
                    format!("Hitsound slot of {}", channel.name),
                )
            });
            ui.painter().rect(
                chip,
                t::SMALL,
                t::INSET,
                t::stroke(if response.hovered() {
                    t::EDGE_STRONG
                } else {
                    t::EDGE
                }),
                StrokeKind::Inside,
            );
            let mut job = LayoutJob::default();
            let font = FontId::proportional(t::META_SIZE);
            job.append(
                set_name(channel.sample_set),
                0.0,
                TextFormat::simple(font.clone(), t::TEXT),
            );
            job.append(
                sound_name(channel.hitsound),
                4.0,
                TextFormat::simple(font, t::MUTED),
            );
            job.wrap = TextWrapping::truncate_at_width(chip.width() - 12.0);
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(
                pos2(chip.left() + 6.0, chip.center().y - galley.size().y / 2.0),
                galley,
                t::TEXT,
            );
            if response.contains_pointer() {
                widgets::hint(
                    ui.ctx(),
                    Hint::new(format!(
                        "Plays as {} in osu!",
                        slot_file(channel.sample_set, channel.hitsound)
                    ))
                    .key("Click", "Choose sample set and sound"),
                );
            }
            slot_popup(&response, channel, emit);
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "inline row control shares the row geometry and edit context"
    )]
    fn name(
        &mut self,
        ui: &mut Ui,
        view: &ProjectView,
        channel: &Channel,
        row: usize,
        state: &RailState,
        rect: Rect,
        emit: Emit<'_>,
    ) {
        if let Some(rename) = self
            .rename
            .as_mut()
            .filter(|rename| rename.id == channel.id)
        {
            let response = ui.put(
                rect,
                TextEdit::singleline(&mut rename.draft)
                    .id(ui.id().with("rename"))
                    .desired_width(rect.width()),
            );
            if rename.focus {
                rename.focus = false;
                response.request_focus();
            }
            let escape = ui.input(|i| i.key_pressed(Key::Escape));
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if escape || response.lost_focus() || enter {
                let new = rename.draft.trim();
                if !escape && !new.is_empty() && new != channel.name {
                    let mut props = ChannelProps::from(channel);
                    props.name = new.into();
                    edit(
                        emit,
                        Edit::SetChannel {
                            id: channel.id,
                            props,
                        },
                    );
                }
                self.rename = None;
            }
            return;
        }
        // The channel button: FL's coloured name plate. Click selects, double-click renames.
        let response = ui.interact(rect, ui.id().with("name"), Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &channel.name)
        });
        let color = t::channel_color(&channel.color);
        let selected = state.selected == Some(channel.id);
        let tint = if selected {
            0.34
        } else if response.hovered() {
            0.26
        } else {
            0.16
        };
        let p = ui.painter();
        p.rect(
            rect,
            t::SMALL,
            t::RAISED.lerp_to_gamma(color, tint),
            t::stroke(if selected {
                color
            } else {
                t::EDGE.lerp_to_gamma(color, 0.25)
            }),
            StrokeKind::Inside,
        );
        p.rect_filled(
            Rect::from_min_max(
                rect.left_top() + vec2(2.0, 2.0),
                rect.left_bottom() + vec2(5.0, -2.0),
            ),
            1,
            if channel.muted {
                color.gamma_multiply(0.4)
            } else {
                color
            },
        );
        let mut job = LayoutJob::simple_singleline(
            channel.name.clone(),
            FontId::proportional(t::BODY_SIZE),
            if channel.muted { t::MUTED } else { t::TEXT },
        );
        job.wrap = TextWrapping::truncate_at_width(rect.width() - 16.0);
        let galley = p.layout_job(job);
        p.galley(
            pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
            galley,
            t::TEXT,
        );
        if response.clicked() {
            emit(Intent::SelectChannel(channel.id));
        }
        if response.double_clicked() {
            self.start_rename(channel);
            ui.ctx().request_repaint();
        }
        response.context_menu(|ui| channel_menu(ui, view, row, state.range, emit));
        if response.contains_pointer() {
            widgets::hint(
                ui.ctx(),
                Hint::new(format!(
                    "{}  ·  {}  ·  {}",
                    channel.name,
                    slot_label(channel),
                    sample_name(&channel.sample)
                ))
                .key("Click", "Select")
                .key("Double-click", "Rename")
                .key("Right-click", "Menu"),
            );
        }
    }
}

fn mini(
    ui: &Ui,
    rect: Rect,
    id: Id,
    glyph: &str,
    on: bool,
    label: &str,
    shortcut: &str,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, on, label));
    let fill = if on {
        t::ACCENT_SURFACE
    } else if response.hovered() {
        t::HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, t::SMALL, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::new(15.0, t::icon_family()),
        if on {
            t::ACCENT
        } else if response.hovered() {
            t::TEXT
        } else {
            t::MUTED
        },
    );
    widgets::describe(response, label, shortcut)
}

pub fn sample_name(sample: &SampleSource) -> &str {
    match sample {
        SampleSource::SkinDefault => "Skin sample",
        SampleSource::File { path } => path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Sample"),
    }
}

pub fn set_name(set: SampleSet) -> &'static str {
    match set {
        SampleSet::Auto => "Auto",
        SampleSet::Normal => "Normal",
        SampleSet::Soft => "Soft",
        SampleSet::Drum => "Drum",
    }
}

pub fn sound_name(sound: Hitsound) -> &'static str {
    match sound {
        Hitsound::Normal => "Normal",
        Hitsound::Whistle => "Whistle",
        Hitsound::Finish => "Finish",
        Hitsound::Clap => "Clap",
    }
}

pub fn slot_label(channel: &Channel) -> String {
    format!(
        "{} · {}",
        set_name(channel.sample_set),
        sound_name(channel.hitsound)
    )
}

/// The osu! sample file name a slot stands for, such as `soft-hitwhistle`.
pub fn slot_file(set: SampleSet, sound: Hitsound) -> String {
    format!(
        "{}-hit{}",
        set_name(set).to_lowercase(),
        sound_name(sound).to_lowercase()
    )
}

/// osu!'s slot matrix: sample sets down, sounds across. One click picks a slot.
pub fn slot_matrix(ui: &mut Ui, channel: &Channel, emit: Emit<'_>) -> bool {
    let mut picked = false;
    egui::Grid::new(("osu-slot-matrix", channel.id))
        .spacing(vec2(3.0, 3.0))
        .min_col_width(0.0)
        .show(ui, |ui| {
            ui.label("");
            for sound in Hitsound::ALL {
                ui.add_sized(
                    vec2(50.0, 14.0),
                    egui::Label::new(t::meta(sound_name(sound), t::MUTED)),
                );
            }
            ui.end_row();
            for set in SETS {
                ui.add_sized(
                    vec2(46.0, 22.0),
                    egui::Label::new(t::meta(set_name(set), t::MUTED)),
                );
                for sound in Hitsound::ALL {
                    let selected = channel.sample_set == set && channel.hitsound == sound;
                    let (cell, response) = ui.allocate_exact_size(vec2(50.0, 22.0), Sense::click());
                    let (fill, edge) = if selected {
                        (t::ACCENT_SURFACE, t::ACCENT)
                    } else if response.hovered() {
                        (t::HOVER, t::EDGE_STRONG)
                    } else {
                        (t::RAISED, t::EDGE)
                    };
                    ui.painter()
                        .rect(cell, t::SMALL, fill, t::stroke(edge), StrokeKind::Inside);
                    if selected {
                        ui.painter().circle_filled(cell.center(), 4.0, t::ACCENT);
                    }
                    response.widget_info(|| {
                        WidgetInfo::selected(
                            WidgetType::RadioButton,
                            ui.is_enabled(),
                            selected,
                            slot_file(set, sound),
                        )
                    });
                    if widgets::describe(response, &slot_file(set, sound), "").clicked()
                        && !selected
                    {
                        let mut props = ChannelProps::from(channel);
                        props.sample_set = set;
                        props.hitsound = sound;
                        edit(
                            emit,
                            Edit::SetChannel {
                                id: channel.id,
                                props,
                            },
                        );
                        picked = true;
                    }
                }
                ui.end_row();
            }
        });
    picked
}

pub fn slot_popup(response: &Response, channel: &Channel, emit: Emit<'_>) {
    Popup::menu(response).show(|ui| {
        ui.label(t::display("Hitsound in osu!", t::TITLE_SIZE));
        if slot_matrix(ui, channel, emit) {
            ui.close();
        }
        widgets::help(
            ui,
            "Rows are sample sets, columns are sounds. The channel exports to this slot.",
        );
    });
}

fn channel_menu(
    ui: &mut Ui,
    view: &ProjectView,
    row: usize,
    range: Option<TimeRange>,
    emit: Emit<'_>,
) {
    let channel = &view.project.channels[row];
    ui.set_min_width(t::MENU_WIDTH - 40.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    if ui
        .add(Button::new("Channel settings").shortcut_text(Panel::Channel.shortcut()))
        .clicked()
    {
        emit(Intent::Editor(Act::Show(Panel::Channel)));
        emit(Intent::SelectChannel(channel.id));
        ui.close();
    }
    if ui.button("Rename").clicked() {
        emit(Intent::Editor(Act::Rename(channel.id)));
        ui.close();
    }
    ui.separator();
    if ui.button("Preview sample").clicked() {
        emit(Intent::Job(Job::AuditionChannel(channel.id)));
        ui.close();
    }
    if ui.button("Choose sample file…").clicked() {
        emit(Intent::Job(Job::UseSample {
            channel: Some(channel.id),
            path: None,
        }));
        ui.close();
    }
    if channel.sample != SampleSource::SkinDefault && ui.button("Use the skin's sample").clicked() {
        let mut props = ChannelProps::from(channel);
        props.sample = SampleSource::SkinDefault;
        edit(
            emit,
            Edit::SetChannel {
                id: channel.id,
                props,
            },
        );
        ui.close();
    }
    ui.separator();
    for (target, label) in [
        (row.checked_sub(1), "Move up (higher priority)"),
        (
            (row + 1 < view.project.channels.len()).then_some(row + 1),
            "Move down (lower priority)",
        ),
    ] {
        if ui
            .add_enabled(target.is_some(), Button::new(label))
            .clicked()
            && let Some(to) = target
        {
            edit(emit, Edit::MoveChannel { id: channel.id, to });
            ui.close();
        }
    }
    if let Some(file) = view.layer.file.as_deref() {
        let label = view
            .project
            .diff(file)
            .map_or("difficulty", |diff| diff.label.as_str());
        ui.separator();
        ui.menu_button(format!("Mute in {label}"), |ui| {
            ranges(ui, range, |range| {
                add_override(
                    emit,
                    file,
                    Override::MuteChannel {
                        channel: channel.id,
                        range,
                    },
                )
            });
        });
        ui.menu_button(format!("Replace in {label} with…"), |ui| {
            for target in view
                .project
                .channels
                .iter()
                .filter(|target| target.id != channel.id)
            {
                ui.menu_button(&target.name, |ui| {
                    ranges(ui, range, |range| {
                        add_override(
                            emit,
                            file,
                            Override::ReplaceChannel {
                                from: channel.id,
                                to: target.id,
                                range,
                            },
                        )
                    });
                });
            }
        });
    }
    ui.separator();
    if ui
        .button(egui::RichText::new("Delete channel").color(t::ERROR))
        .clicked()
    {
        edit(emit, Edit::RemoveChannel { id: channel.id });
        ui.close();
    }
}

fn ranges(ui: &mut Ui, selected: Option<TimeRange>, mut apply: impl FnMut(Option<TimeRange>)) {
    if ui.button("Whole difficulty").clicked() {
        apply(None);
        ui.close();
    }
    if let Some(range) = selected {
        let label = format!(
            "Selected time range  {} – {}",
            timecode(f64::from(range.start)),
            timecode(f64::from(range.end))
        );
        if ui.button(label).clicked() {
            apply(Some(range));
            ui.close();
        }
    }
}

pub fn object_menu(
    ui: &mut Ui,
    view: &ProjectView,
    target: Target,
    channel: Option<ChannelId>,
    emit: Emit<'_>,
) {
    let Some(file) = view.layer.file.as_deref() else {
        return;
    };
    let Some(objects) = &view.layer.objects else {
        return;
    };
    let Some(time) = objects.time(target) else {
        return;
    };
    ui.label(t::meta(timecode(f64::from(time)), t::DIFFICULTY));
    if let Some(channel) = channel.and_then(|id| view.project.channel(id))
        && ui.button(format!("Add {} here", channel.name)).clicked()
    {
        add_hit(emit, file, channel.id, time);
        ui.close();
    }
    ui.menu_button("Add another channel here", |ui| {
        for channel in &view.project.channels {
            if ui.button(&channel.name).clicked() {
                add_hit(emit, file, channel.id, time);
                ui.close();
            }
        }
    });
    if let Target::Edge(index) = target
        && objects.timeline.edges[index].kind == EdgeKind::SliderHead
    {
        ui.menu_button("Slider body sound", |ui| {
            for set in SETS {
                ui.menu_button(set_name(set), |ui| {
                    for (label, whistle) in [("Slide", false), ("Slide and whistle", true)] {
                        if ui.button(label).clicked() {
                            let value = Override::SliderBody {
                                slider_time: time,
                                sample_set: set,
                                whistle,
                            };
                            let previous = view.project.diff(file).and_then(|d| {
                                d.overrides.iter().rposition(|o| {
                                    matches!(o, Override::SliderBody { slider_time, .. } if *slider_time == time)
                                })
                            });
                            if let Some(index) = previous {
                                edit(
                                    emit,
                                    Edit::Batch {
                                        edits: vec![
                                            Edit::RemoveOverride {
                                                file: file.into(),
                                                index,
                                            },
                                            Edit::AddOverride {
                                                file: file.into(),
                                                value,
                                            },
                                        ],
                                    },
                                );
                            } else {
                                add_override(emit, file, value);
                            }
                            ui.close();
                        }
                    }
                });
            }
        });
    }
}

fn add_hit(emit: Emit<'_>, file: &str, channel: ChannelId, time: hs_core::Ms) {
    edit(
        emit,
        Edit::AddEvents {
            diff: Some(file.into()),
            events: vec![(channel, time)],
        },
    );
}

fn add_override(emit: Emit<'_>, file: &str, value: Override) {
    edit(
        emit,
        Edit::AddOverride {
            file: file.into(),
            value,
        },
    );
}
