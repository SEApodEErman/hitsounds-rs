//! The workspace: toolbar, layer strip, timeline with its channel rack, and the docked
//! browser, channel settings, difficulty review and mixer (D-61).

mod canvas;
pub(crate) mod grid;
mod inspector;
mod library;
mod mixer;
pub(crate) mod model;
mod paint;
mod rail;

#[cfg(test)]
mod tests;

use crate::{
    app::Intent,
    keys::{self, Action},
    theme as t,
    views::Emit,
    widgets::{self, Hint, Segment},
    worker::{Job, ProjectView, Snapshot},
};
pub use canvas::Tool;
use canvas::{Timeline, seek};
use eframe::egui::{
    self, Align, Button, Context, Frame, Label, Layout, Margin, RichText, ScrollArea, StrokeKind,
    Ui, vec2,
};
use egui_phosphor::regular as icons;
use hs_core::{ChannelId, EdgeKind};
use hs_session::{Clock, Transport};
pub use inspector::Tab;

/// The dockable parts of the workspace, each with an FL-style function key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Browser,
    Channel,
    Review,
    Mixer,
}

impl Panel {
    pub const ALL: [Self; 4] = [Self::Browser, Self::Channel, Self::Review, Self::Mixer];

    pub fn label(self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::Channel => "Channel settings",
            Self::Review => "Difficulty review",
            Self::Mixer => "Mixer",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::Channel => "Channel",
            Self::Review => "Review",
            Self::Mixer => "Mixer",
        }
    }

    /// The shortcut that shows or hides it (D-61 defaults, rebindable by D-92).
    pub fn action(self) -> Action {
        match self {
            Self::Channel => Action::PanelChannel,
            Self::Review => Action::PanelReview,
            Self::Browser => Action::PanelBrowser,
            Self::Mixer => Action::PanelMixer,
        }
    }

    pub fn shortcut(self) -> &'static str {
        keys::label(self.action())
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Browser => icons::FOLDERS,
            Self::Channel => icons::SLIDERS_HORIZONTAL,
            Self::Review => icons::LIST_CHECKS,
            Self::Mixer => icons::FADERS,
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Browser => "Browser: find and preview samples, then drag them onto channels",
            Self::Channel => "Channel settings: sample, osu! hitsound, volume and colour",
            Self::Review => "Difficulty review: issues, rules and changes for one difficulty",
            Self::Mixer => "Mixer: a volume fader for every channel",
        }
    }
}

/// Things menus, shortcuts and the command palette can ask of the workspace. They only
/// change view state or build the same edits the pointer gestures do (D-48).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Tool(Tool),
    Snap(u32),
    ZoomIn,
    ZoomOut,
    Fit,
    Follow,
    /// Show or hide the song waveform (D-92).
    Waveform,
    SelectAll,
    Deselect,
    DeleteSelection,
    Copy,
    Cut,
    Paste,
    PlaceAtPlayhead,
    Rename(ChannelId),
    Toggle(Panel),
    Show(Panel),
}

/// A read-only summary of the workspace for the menus and the status bar.
#[derive(Clone, Copy)]
pub struct Status {
    pub tool: Tool,
    pub divisor: u32,
    pub follow: bool,
    pub waveform: bool,
    pub browser: bool,
    pub inspector: Option<Tab>,
    pub mixer: bool,
    pub selected: usize,
    /// Hits waiting to be pasted (D-88).
    pub copied: usize,
    pub channel: Option<ChannelId>,
}

impl Status {
    pub fn open(&self, panel: Panel) -> bool {
        match panel {
            Panel::Browser => self.browser,
            Panel::Channel => self.inspector == Some(Tab::Channel),
            Panel::Review => self.inspector == Some(Tab::Review),
            Panel::Mixer => self.mixer,
        }
    }
}

pub struct Editor {
    pub timeline: Timeline,
    inspector: inspector::Inspector,
    library: library::Library,
    pub browser_open: bool,
    pub inspector_open: bool,
    pub mixer_open: bool,
    difficulty: bool,
}

impl Editor {
    pub fn new(storage: Option<&dyn eframe::Storage>) -> Self {
        let flag = |key, default| {
            storage
                .and_then(|s| s.get_string(key))
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        };
        Self {
            timeline: {
                let mut timeline = Timeline::default();
                timeline.show_wave = flag("waveform-shown", true);
                timeline
            },
            inspector: inspector::Inspector::default(),
            library: library::Library::default(),
            browser_open: flag("library-open", true),
            inspector_open: flag("inspector-open", true),
            mixer_open: flag("mixer-open", false),
            difficulty: false,
        }
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string("library-open", self.browser_open.to_string());
        storage.set_string("inspector-open", self.inspector_open.to_string());
        storage.set_string("mixer-open", self.mixer_open.to_string());
        storage.set_string("waveform-shown", self.timeline.show_wave.to_string());
    }

    pub fn new_project(&mut self) {
        let (tool, show_wave) = (self.timeline.tool, self.timeline.show_wave);
        self.timeline = Timeline::default();
        self.timeline.tool = tool;
        self.timeline.show_wave = show_wave;
        self.inspector = inspector::Inspector::default();
        self.difficulty = false;
    }

    pub fn layer_changed(&mut self, difficulty: bool) {
        self.timeline.reset_layer();
        self.difficulty = difficulty;
        self.inspector.tab = if difficulty {
            Tab::Review
        } else {
            Tab::Channel
        };
    }

    pub fn select_channel(&mut self, id: ChannelId) {
        self.timeline.channel = Some(id);
        // While reviewing a difficulty, placing hits must not flip the panel away.
        if !self.difficulty {
            self.inspector.tab = Tab::Channel;
        }
    }

    pub fn show_difficulty(&mut self) {
        self.inspector.tab = Tab::Review;
    }

    pub fn status(&self) -> Status {
        Status {
            tool: self.timeline.tool,
            divisor: self.timeline.divisor,
            follow: self.timeline.follow,
            waveform: self.timeline.show_wave,
            browser: self.browser_open,
            inspector: self.inspector_open.then_some(self.inspector.tab),
            mixer: self.mixer_open,
            selected: self.timeline.selection.len(),
            copied: self.timeline.clipboard_len(),
            channel: self.timeline.channel,
        }
    }

    pub fn act(
        &mut self,
        act: Act,
        view: &ProjectView,
        clock: &Clock,
        now: f64,
        reduced: bool,
        emit: Emit<'_>,
    ) {
        match act {
            Act::Tool(tool) => self.timeline.tool = tool,
            Act::Snap(divisor) => self.timeline.divisor = divisor,
            Act::ZoomIn => self.timeline.zoom_center(view, 1.34, now, reduced),
            Act::ZoomOut => self.timeline.zoom_center(view, 0.75, now, reduced),
            Act::Fit => self.timeline.fit(view, now, reduced),
            Act::Follow => self.timeline.follow = !self.timeline.follow,
            Act::Waveform => self.timeline.show_wave = !self.timeline.show_wave,
            Act::SelectAll => self.timeline.select_all(view),
            Act::Deselect => self.timeline.reset_layer(),
            Act::DeleteSelection => self.timeline.delete_selection(view, emit),
            Act::Copy => self.timeline.copy_selection(),
            Act::Cut => self.timeline.cut_selection(view, emit),
            Act::Paste => self.timeline.paste(view, clock, emit),
            Act::PlaceAtPlayhead => self.timeline.place_at_playhead(view, clock, emit),
            Act::Rename(id) => {
                if let Some(channel) = view.project.channel(id) {
                    self.timeline.channel = Some(id);
                    self.timeline.rail.start_rename(channel);
                }
            }
            Act::Toggle(panel) => {
                let open = self.status().open(panel);
                self.set_panel(panel, !open);
            }
            Act::Show(panel) => self.set_panel(panel, true),
        }
    }

    fn set_panel(&mut self, panel: Panel, open: bool) {
        match panel {
            Panel::Browser => self.browser_open = open,
            Panel::Mixer => self.mixer_open = open,
            Panel::Channel | Panel::Review => {
                self.inspector_open = open;
                if open {
                    self.inspector.tab = if panel == Panel::Channel {
                        Tab::Channel
                    } else {
                        Tab::Review
                    };
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // Frame inputs are passed flat, as in the other views.
    pub fn show(
        &mut self,
        ctx: &Context,
        state: &Snapshot,
        view: &ProjectView,
        clock: &Clock,
        ready: bool,
        reduced: bool,
        emit: Emit<'_>,
    ) {
        let width = ctx.content_rect().width();
        self.timeline.all_muted = state.settings.levels.hits_muted;
        self.toolbar(ctx, view, clock, ready, emit);
        layer_strip(ctx, view, ready, emit);

        let frame = || Frame::new().fill(t::SURFACE).inner_margin(t::SPACE);
        let mut close_mixer = false;
        let mixer = egui::TopBottomPanel::bottom("mixer-pane")
            .resizable(false)
            .exact_height(t::MIXER_HEIGHT)
            .frame(Frame::new().fill(t::CHROME).inner_margin(t::GAP));
        let mut draw_mixer = |ui: &mut Ui| {
            close_mixer = dock_heading(ui, "Mixer", |_| {});
            mixer::show(
                ui,
                view,
                &state.settings.levels,
                self.timeline.channel,
                clock,
                ready,
                emit,
            );
        };
        if reduced {
            if self.mixer_open {
                mixer.show(ctx, &mut draw_mixer);
            }
        } else {
            mixer.show_animated(ctx, self.mixer_open, &mut draw_mixer);
        }
        if close_mixer {
            self.mixer_open = false;
        }

        // Leave the rack and a strip of timeline visible however narrow the window is.
        // A transient tiny viewport can put the 30% cap below EDITOR_PANEL_MIN.
        // Let the minimum shrink too, so neither calculation nor panel range can panic.
        let max = ((width - t::RAIL_WIDTH - 280.0) / 2.0)
            .max(t::EDITOR_PANEL_MIN)
            .min(width * 0.3);
        let min = t::EDITOR_PANEL_MIN.min(max);
        let show_panel = |panel: egui::SidePanel, open, show: &mut dyn FnMut(&mut Ui)| {
            if reduced {
                if open {
                    panel.show(ctx, show);
                }
            } else {
                panel.show_animated(ctx, open, show);
            }
        };
        let mut close_browser = false;
        show_panel(
            egui::SidePanel::left("library-pane")
                .default_width(t::LIBRARY_WIDTH.min(max))
                .width_range(min..=max)
                .frame(frame()),
            self.browser_open,
            &mut |ui| {
                close_browser = dock_heading(ui, "Browser", |ui| {
                    ui.add_enabled_ui(ready, |ui| {
                        if t::icon_button(ui, icons::ARROWS_CLOCKWISE, "Rescan the sample folders")
                            .clicked()
                        {
                            emit(Intent::Job(Job::ScanLibrary));
                        }
                        if t::icon_button(ui, icons::FOLDER_PLUS, "Add a sample folder").clicked() {
                            emit(Intent::Job(Job::AddLibrary));
                        }
                    });
                });
                let target = self
                    .timeline
                    .channel
                    .and_then(|id| view.project.channel(id));
                self.library.show(ui, state, target, ready, emit);
            },
        );
        if close_browser {
            self.browser_open = false;
        }
        let mut close_inspector = false;
        show_panel(
            egui::SidePanel::right("inspector-pane")
                .default_width(t::INSPECTOR_WIDTH.min(max))
                .width_range(t::INSPECTOR_WIDTH.min(max)..=max.max(t::INSPECTOR_WIDTH))
                .frame(frame()),
            self.inspector_open,
            &mut |ui| {
                close_inspector = self
                    .inspector
                    .show(ui, view, &mut self.timeline, ready, emit);
            },
        );
        if close_inspector {
            self.inspector_open = false;
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(t::BASE))
            .show(ctx, |ui| {
                self.timeline.show(ui, view, clock, ready, reduced, emit);
            });
    }

    /// Transport, time, speed, snap, tools, history, panel switches and Export.
    fn toolbar(
        &mut self,
        ctx: &Context,
        view: &ProjectView,
        clock: &Clock,
        ready: bool,
        emit: Emit<'_>,
    ) {
        let width = ctx.content_rect().width();
        let (captions, tool_labels, panel_labels) =
            (width >= 1040.0, width >= 1200.0, width >= 1400.0);
        let status = self.status();
        egui::TopBottomPanel::top("toolbar")
            .exact_height(t::TOOLBAR_HEIGHT)
            .frame(
                Frame::new()
                    .fill(t::SURFACE)
                    .inner_margin(Margin::symmetric(t::SPACE as i8, 0)),
            )
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                ui.painter().hline(
                    bar.expand2(vec2(t::SPACE, 0.0)).x_range(),
                    bar.bottom() - t::HAIRLINE,
                    t::stroke(t::EDGE),
                );
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    transport(ui, view, clock, ready, emit);
                    widgets::divider(ui);

                    if captions {
                        ui.label(t::muted("Speed"));
                    }
                    let rate = if clock.rate > 0.0 { clock.rate } else { 1.0 };
                    let speed = egui::ComboBox::from_id_salt("playback-speed")
                        .selected_text(t::meta(format!("{rate:.2}×"), t::TEXT))
                        .width(64.0)
                        .show_ui(ui, |ui| {
                            for value in [0.25_f64, 0.5, 0.75, 1.0] {
                                if ui
                                    .selectable_label(
                                        (rate - value).abs() < 0.001,
                                        format!("{value:.2}×"),
                                    )
                                    .clicked()
                                    && ready
                                {
                                    emit(Intent::Transport(Transport::Rate { rate: value }));
                                }
                            }
                        })
                        .response;
                    widgets::describe(speed, "Playback speed. Slow down to place hits by ear", "");

                    if captions {
                        ui.label(t::muted("Snap"));
                    }
                    let snap = egui::ComboBox::from_id_salt("snap-divisor")
                        .selected_text(t::meta(format!("1/{}", self.timeline.divisor), t::TEXT))
                        .width(56.0)
                        .show_ui(ui, |ui| {
                            for choice in grid::DIVISORS {
                                ui.selectable_value(
                                    &mut self.timeline.divisor,
                                    choice,
                                    format!("1/{choice}"),
                                );
                            }
                        })
                        .response;
                    widgets::describe(
                        snap,
                        "Beat snap divisor: how many steps each beat is split into",
                        "Ctrl+Wheel",
                    );
                    widgets::divider(ui);

                    let tools: Vec<_> = Tool::ALL
                        .into_iter()
                        .map(|tool| Segment {
                            value: tool,
                            icon: tool_icon(tool),
                            label: tool.label(),
                            hint: tool.summary(),
                            shortcut: tool.shortcut(),
                            enabled: true,
                        })
                        .collect();
                    if let Some(tool) =
                        widgets::segmented(ui, self.timeline.tool, &tools, tool_labels)
                    {
                        self.timeline.tool = tool;
                    }
                    widgets::divider(ui);

                    for (redo, enabled, glyph, label, shortcut) in [
                        (
                            false,
                            view.can_undo,
                            icons::ARROW_U_UP_LEFT,
                            "Undo",
                            keys::label(Action::Undo),
                        ),
                        (
                            true,
                            view.can_redo,
                            icons::ARROW_U_UP_RIGHT,
                            "Redo",
                            keys::label(Action::Redo),
                        ),
                    ] {
                        let response = ui
                            .add_enabled_ui(ready && enabled, |ui| {
                                widgets::tool_button(ui, glyph, label, false, None)
                            })
                            .inner;
                        if widgets::describe(response, label, shortcut).clicked() {
                            emit(Intent::Job(Job::History { redo }));
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let export = ui
                            .add_enabled_ui(ready, |ui| {
                                t::primary(ui, (t::icon(icons::EXPORT), "Export"))
                            })
                            .inner;
                        if widgets::describe(
                            export,
                            "Review what will be written to the mapset, then export",
                            keys::label(Action::Export),
                        )
                        .clicked()
                        {
                            emit(Intent::OpenExport);
                        }
                        widgets::divider(ui);
                        for panel in Panel::ALL.into_iter().rev() {
                            let response = widgets::tool_button(
                                ui,
                                panel.icon(),
                                panel.short(),
                                panel_labels,
                                Some(status.open(panel)),
                            );
                            if widgets::describe(response, panel.summary(), panel.shortcut())
                                .clicked()
                            {
                                emit(Intent::Editor(Act::Toggle(panel)));
                            }
                        }
                    });
                });
            });
    }
}

fn tool_icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => icons::SELECTION,
        Tool::Draw => icons::PENCIL_SIMPLE,
        Tool::Paint => icons::PAINT_BRUSH,
        Tool::Delete => icons::ERASER,
    }
}

/// The title row of a dock: its name, any extra buttons, and the close button.
fn dock_heading(ui: &mut Ui, title: &str, extra: impl FnOnce(&mut Ui)) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(t::display(title, t::TITLE_SIZE));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close = t::icon_button(ui, icons::X, "Hide this panel").clicked();
            extra(ui);
        });
    });
    ui.add_space(2.0);
    close
}

fn transport(ui: &mut Ui, view: &ProjectView, clock: &Clock, ready: bool, emit: Emit<'_>) {
    ui.add_enabled_ui(ready && clock.length_ms > 0.0, |ui| {
        let (icon, label) = if clock.playing {
            (icons::PAUSE, "Pause")
        } else {
            (icons::PLAY, "Play")
        };
        let play = ui
            .scope(|ui| {
                ui.spacing_mut().button_padding = vec2(9.0, 6.0);
                t::primary(ui, t::icon(icon).size(18.0))
            })
            .inner;
        play.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
        });
        if widgets::describe(play, label, keys::label(Action::PlayPause)).clicked() {
            emit(Intent::TogglePlayback);
        }
        let stop = widgets::tool_button(ui, icons::STOP, "Stop", false, None);
        if widgets::describe(
            stop,
            "Stop and return to the start",
            keys::label(Action::GoToStart),
        )
        .clicked()
        {
            if clock.playing {
                emit(Intent::TogglePlayback);
            }
            seek(emit, 0.0);
        }
    });
    let (bar, beat) = view.layer.grid.bar_beat(clock.position_ms);
    let display = Frame::new()
        .fill(t::INSET)
        .stroke(t::stroke(t::EDGE))
        .corner_radius(t::CONTROL_RADIUS)
        .inner_margin(Margin::symmetric(10, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(
                    Label::new(t::mono_time(grid::timecode(clock.position_ms))).selectable(false),
                );
                ui.add(Label::new(t::meta(format!("{bar:03}:{beat}"), t::MUTED)).selectable(false));
            });
        })
        .response;
    if display.contains_pointer() {
        widgets::hint(
            ui.ctx(),
            Hint::new("Song position in minutes:seconds, then bar:beat")
                .key("Wheel", "Step by snap on the timeline"),
        );
    }
}

/// Which layer edits go to: the base (all difficulties) or one difficulty (D-50).
fn layer_strip(ctx: &Context, view: &ProjectView, ready: bool, emit: Emit<'_>) {
    let diff = view
        .layer
        .file
        .as_deref()
        .and_then(|file| view.project.diff(file));
    let fill = if diff.is_some() {
        t::CHROME.lerp_to_gamma(t::DIFFICULTY, 0.07)
    } else {
        t::CHROME
    };
    egui::TopBottomPanel::top("layer-bar")
        .exact_height(t::LAYER_BAR_HEIGHT)
        .frame(
            Frame::new()
                .fill(fill)
                .inner_margin(Margin::symmetric(t::SPACE as i8, 0)),
        )
        .show(ctx, |ui| {
            let bar = ui.max_rect();
            ui.painter().hline(
                bar.expand2(vec2(t::SPACE, 0.0)).x_range(),
                bar.bottom() - t::HAIRLINE,
                t::stroke(t::EDGE),
            );
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = t::SMALL;
                ui.label(t::muted("Editing"));
                ui.add_space(t::SMALL);
                let tabs = (ui.available_width() * 0.6).max(200.0);
                ui.allocate_ui_with_layout(
                    vec2(tabs, t::LAYER_BAR_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| layer_tabs(ui, view, ready, emit),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(diff) = diff {
                        let issues = view.layer.issues.len();
                        if issues > 0 {
                            for (glyph, label, shortcut, backwards) in [
                                (icons::CARET_DOWN, "Jump to the next issue", "N", false),
                                (
                                    icons::CARET_UP,
                                    "Jump to the previous issue",
                                    keys::label(Action::PreviousIssue),
                                    true,
                                ),
                            ] {
                                let response = ui
                                    .add_enabled_ui(ready, |ui| {
                                        widgets::tool_button(ui, glyph, label, false, None)
                                    })
                                    .inner;
                                if widgets::describe(response, label, shortcut).clicked() {
                                    emit(Intent::StepIssue { backwards });
                                }
                            }
                            widgets::tag(ui, format!("{issues} to review"), t::EMPTY_HIT);
                        } else {
                            widgets::tag(ui, "Nothing to review", t::MATCHED);
                        }
                        ui.add(
                            Label::new(
                                RichText::new(format!("Changes here apply to {} only", diff.label))
                                    .color(t::DIFFICULTY),
                            )
                            .truncate(),
                        );
                    } else {
                        ui.add(
                            Label::new(t::muted("Hits on the base layer play in every difficulty"))
                                .truncate(),
                        );
                    }
                });
            });
        });
}

fn layer_tabs(ui: &mut Ui, view: &ProjectView, ready: bool, emit: Emit<'_>) {
    ScrollArea::horizontal()
        .id_salt("layer-tabs")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let base = view.layer.file.is_none();
                let response = layer_tab(ui, "Base", None, base, t::ACCENT, ready);
                if widgets::describe(
                    response,
                    "Base layer: the hitsounds shared by every difficulty",
                    "",
                )
                .clicked()
                    && !base
                {
                    emit(Intent::Job(Job::SelectLayer(None)));
                }
                for diff in &view.project.diffs {
                    let exists = view.available_diffs.contains(&diff.file);
                    let active = view.layer.file.as_ref() == Some(&diff.file);
                    let (badge, note) = if !exists {
                        (
                            Some((icons::WARNING.to_owned(), t::WARNING, true)),
                            "Its .osu file is missing from the mapset folder",
                        )
                    } else if !diff.enabled {
                        (None, "Left out of export")
                    } else {
                        match view.layer.badges.get(&diff.file).copied() {
                            Some(0) => (
                                Some((icons::CHECK.to_owned(), t::MATCHED, true)),
                                "Nothing left to review",
                            ),
                            Some(count) => (
                                Some((count.to_string(), t::EMPTY_HIT, false)),
                                "Objects or hits still to review",
                            ),
                            None => (None, "Not checked yet"),
                        }
                    };
                    let colour = if !diff.enabled || !exists {
                        t::DIM
                    } else {
                        t::DIFFICULTY
                    };
                    let response = layer_tab(ui, &diff.label, badge, active, colour, ready);
                    if response.contains_pointer() {
                        widgets::hint(
                            ui.ctx(),
                            Hint::new(format!("{}: {note}", diff.label))
                                .key("Click", "Review and edit this difficulty only"),
                        );
                    }
                    if response.on_hover_text(&diff.file).clicked() && !active {
                        emit(Intent::Job(Job::SelectLayer(Some(diff.file.clone()))));
                    }
                }
            });
        });
}

/// One layer tab: its name and, for a difficulty, a badge with its open issues.
fn layer_tab(
    ui: &mut Ui,
    title: &str,
    badge: Option<(String, egui::Color32, bool)>,
    active: bool,
    colour: egui::Color32,
    ready: bool,
) -> egui::Response {
    let text = RichText::new(title).color(if active { colour } else { t::MUTED });
    let button = match &badge {
        Some((label, tint, true)) => Button::selectable(
            active,
            (text, t::icon(label.as_str()).size(12.0).color(*tint)),
        ),
        Some((label, tint, false)) => {
            Button::selectable(active, (text, t::meta(label.as_str(), *tint)))
        }
        None => Button::selectable(active, text),
    };
    let fill = if active {
        colour.gamma_multiply(0.16)
    } else {
        egui::Color32::TRANSPARENT
    };
    let response = ui.add_enabled(
        ready,
        button
            .fill(fill)
            .frame_when_inactive(active)
            .min_size(vec2(0.0, t::CONTROL_HEIGHT)),
    );
    if active {
        let rect = response.rect;
        ui.painter().rect_stroke(
            rect,
            t::CONTROL_RADIUS,
            t::stroke(colour.gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
    }
    response
}

pub fn edge_name(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Circle => "Circle",
        EdgeKind::SliderHead => "Slider head",
        EdgeKind::SliderRepeat => "Slider repeat",
        EdgeKind::SliderTail => "Slider end",
        EdgeKind::SpinnerEnd => "Spinner end",
    }
}
