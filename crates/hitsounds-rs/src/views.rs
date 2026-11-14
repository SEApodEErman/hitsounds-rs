//! The shell around the workspace: title bar with the menus and the hint bar, status
//! bar, home page, dialogs and toasts (D-57, D-61).

use crate::{
    app::{Decision, Dialog, Intent, SeparationCard, Toast, ToastAction},
    chrome,
    editor::{Act, Panel, Status, Tool, grid::DIVISORS},
    export_view::addition_only_label,
    theme as t,
    widgets::{self, Hint},
    worker::{Job, ProjectView, Snapshot},
};
use eframe::egui::{
    self, Align, Align2, Button, Color32, Context, Frame, Id, Layout, Margin, Rect, Response,
    RichText, Sense, StrokeKind, TextureHandle, Ui, UiBuilder, ViewportCommand, WidgetInfo,
    WidgetType, containers::menu::MenuButton, pos2, vec2,
};
use egui_phosphor::regular as icons;
use hs_core::{
    Edit,
    project::{AdditionOnlyNormal, ExportSettings, Monitor},
};
use hs_session::Transport;
use std::path::Path;

pub type Emit<'a> = &'a mut dyn FnMut(Intent);

/// What the title bar needs to know to build the menus and the hint bar.
pub struct Shell<'a> {
    pub state: Option<&'a Snapshot>,
    pub available: bool,
    pub custom_frame: bool,
    /// The workspace summary while the editor is on screen (not on Home or in Export).
    pub editor: Option<Status>,
    pub exporting: bool,
    /// A drum separation is running (D-72).
    pub separating: bool,
    pub hint: &'a Hint,
}

impl Shell<'_> {
    fn project(&self) -> Option<&ProjectView> {
        self.state?.active.as_deref()
    }
}

/// The title bar: app mark, menus, the hint bar, the project name and the window buttons.
pub fn title_bar(ctx: &Context, logo: &TextureHandle, shell: &Shell<'_>, emit: Emit<'_>) {
    egui::TopBottomPanel::top("header")
        .exact_height(t::TITLE_HEIGHT)
        .frame(Frame::new().fill(t::CHROME))
        .show(ctx, |ui| {
            let (bar, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), t::TITLE_HEIGHT), Sense::hover());
            ui.painter().hline(
                bar.x_range(),
                bar.bottom() - t::HAIRLINE,
                t::stroke(t::EDGE),
            );
            let left = ui
                .scope_builder(
                    UiBuilder::new()
                        .max_rect(bar)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.add_space(t::GAP + 2.0);
                        ui.image((logo.id(), vec2(18.0, 18.0)));
                        ui.add_space(t::SMALL + 2.0);
                        menus(ui, shell, emit);
                        ui.cursor().left()
                    },
                )
                .inner;

            let (right, max) = ui
                .scope_builder(
                    UiBuilder::new()
                        .max_rect(bar)
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        let max = if shell.custom_frame {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            if caption(ui, icons::X, "Close", false, Some(t::CLOSE_HOVER)).clicked()
                            {
                                emit(Intent::Exit);
                            }
                            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            let (glyph, title) = if maximized {
                                (icons::COPY, "Restore")
                            } else {
                                (icons::SQUARE, "Maximize")
                            };
                            let max = caption(ui, glyph, title, chrome::maximize_hovered(), None);
                            if max.clicked() {
                                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                            }
                            if caption(ui, icons::MINUS, "Minimize", false, None).clicked() {
                                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                            }
                            max.rect
                        } else {
                            Rect::NOTHING
                        };
                        (ui.cursor().right(), max)
                    },
                )
                .inner;

            // Everything between the menus and the window buttons drags the window, so
            // the hint bar and the project name are painted, never interactive (D-57).
            let drag = Rect::from_min_max(pos2(left, bar.top()), pos2(right, bar.bottom()));
            let p = ui.painter().with_clip_rect(drag);
            let title = match shell.project() {
                Some(project) if project.dirty => format!("{}  •", project.project.name),
                Some(project) => project.project.name.clone(),
                None => "hitsounds-rs".to_owned(),
            };
            let galley = p.layout_no_wrap(
                title,
                egui::FontId::proportional(12.0),
                if shell.project().is_some_and(|project| project.dirty) {
                    t::TEXT
                } else {
                    t::MUTED
                },
            );
            let title_width = galley.size().x.min(drag.width() * 0.4);
            let title_left = drag.right() - t::SPACE - title_width;
            p.with_clip_rect(Rect::from_min_max(
                pos2(title_left, drag.top()),
                pos2(drag.right() - t::SPACE, drag.bottom()),
            ))
            .galley(
                pos2(title_left, drag.center().y - galley.size().y / 2.0),
                galley,
                t::MUTED,
            );
            let room = Rect::from_min_max(
                pos2(left + t::SPACE, bar.top()),
                pos2(title_left - t::GUTTER, bar.bottom()),
            );
            if room.width() > 120.0 {
                p.vline(
                    left + 5.0,
                    bar.shrink2(vec2(0.0, 9.0)).y_range(),
                    t::stroke(t::EDGE),
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(room)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.set_clip_rect(room);
                        widgets::show_hint(ui, shell.hint);
                    },
                );
            }
            if shell.custom_frame {
                chrome::layout(drag, max, ctx.pixels_per_point());
            }
        });
}

fn caption(
    ui: &mut Ui,
    glyph: &str,
    label: &str,
    native_hover: bool,
    hover_colour: Option<Color32>,
) -> Response {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        let hovered = hover_colour.unwrap_or(t::RAISED);
        visuals.widgets.inactive.weak_bg_fill = if native_hover { hovered } else { t::CHROME };
        visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        visuals.widgets.inactive.corner_radius = 0.into();
        visuals.widgets.hovered.corner_radius = 0.into();
        visuals.widgets.active.corner_radius = 0.into();
        visuals.widgets.hovered.weak_bg_fill = hovered;
        visuals.widgets.hovered.bg_stroke = t::stroke(hovered);
        let response = ui.add(
            Button::new(t::icon(glyph).size(14.0))
                .min_size(vec2(t::CAPTION_WIDTH, t::TITLE_HEIGHT)),
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
        response.on_hover_text(label)
    })
    .inner
}

/// One menu row. Closes the menu when chosen.
fn item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    row(ui, false, label, shortcut, enabled)
}

/// A menu row with a check mark for a setting that is on.
fn check(ui: &mut Ui, on: bool, label: &str, shortcut: &str, enabled: bool) -> bool {
    row(ui, on, label, shortcut, enabled)
}

/// Every row keeps the check column, so labels line up whether or not a menu has checks.
fn row(ui: &mut Ui, on: bool, label: &str, shortcut: &str, enabled: bool) -> bool {
    ui.spacing_mut().item_spacing.y = 0.0;
    let mark =
        t::icon(icons::CHECK)
            .size(12.0)
            .color(if on { t::ACCENT } else { Color32::TRANSPARENT });
    let clicked = ui
        .add_enabled(
            enabled,
            Button::new((mark, label)).shortcut_text(t::meta(shortcut, t::DIM)),
        )
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// A nested menu, indented like the rows around it.
fn submenu(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let blank = t::icon(icons::CHECK).size(12.0).color(Color32::TRANSPARENT);
    ui.menu_button((blank, label), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui);
    });
}

fn menu(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    MenuButton::from_button(Button::new(title).frame_when_inactive(false)).ui(ui, |ui| {
        ui.set_min_width(t::MENU_WIDTH);
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui);
    });
}

/// File, Edit, View, Tools, Help. Every action in the app is listed here by name, with
/// its shortcut, so nothing depends on knowing an icon (D-61).
fn menus(ui: &mut Ui, shell: &Shell<'_>, emit: Emit<'_>) {
    let ok = shell.available;
    let project = shell.project();
    let open = ok && project.is_some();
    let editor = shell.editor.filter(|_| open);
    let editing = editor.is_some();

    menu(ui, "File", |ui| {
        if item(ui, "New project from mapset…", "Ctrl+N", ok) {
            emit(Intent::Job(Job::Create));
        }
        if item(ui, "New project from mania hitsound diff…", "", ok) {
            emit(Intent::Job(Job::ImportHitsounds));
        }
        if item(ui, "Open project…", "Ctrl+O", ok) {
            emit(Intent::Job(Job::Open));
        }
        let recent = shell
            .state
            .map(|state| (&state.settings.recent, &state.recent_exists));
        ui.add_enabled_ui(
            ok && recent.is_some_and(|(list, _)| !list.is_empty()),
            |ui| {
                submenu(ui, "Open recent", |ui| {
                    ui.set_min_width(t::MENU_WIDTH);
                    let Some((list, exists)) = recent else {
                        return;
                    };
                    for (recent, exists) in list.iter().zip(exists) {
                        let row = ui
                            .add_enabled(*exists, Button::new(recent.name.as_str()).truncate())
                            .on_disabled_hover_text("The project file could not be found");
                        if row.clicked() {
                            emit(Intent::Job(Job::OpenRecent(recent.file.clone())));
                            ui.close();
                        }
                    }
                });
            },
        );
        ui.separator();
        if item(ui, "Save project", "Ctrl+S", open) {
            emit(Intent::Job(Job::Save));
        }
        if item(ui, "Close project", "Ctrl+W", open) {
            emit(Intent::Job(Job::Close));
        }
        ui.separator();
        if shell.exporting {
            if item(ui, "Back to the editor", "Esc", true) {
                emit(Intent::CloseExport);
            }
        } else if item(ui, "Export…", "Ctrl+E", editing) {
            emit(Intent::OpenExport);
        }
        ui.separator();
        if item(ui, "Project settings…", "", project.is_some()) {
            emit(Intent::Show(Dialog::Project));
        }
        if item(ui, "Change mapset…", "", open && !shell.exporting) {
            emit(Intent::Job(Job::ChangeMapset));
        }
        if item(ui, "App settings…", "Ctrl+,", true) {
            emit(Intent::Show(Dialog::Settings));
        }
        ui.separator();
        if item(ui, "Exit", "Alt+F4", true) {
            emit(Intent::Exit);
        }
    });

    menu(ui, "Edit", |ui| {
        if item(
            ui,
            "Undo",
            "Ctrl+Z",
            open && project.is_some_and(|p| p.can_undo),
        ) {
            emit(Intent::Job(Job::History { redo: false }));
        }
        if item(
            ui,
            "Redo",
            "Ctrl+Shift+Z",
            open && project.is_some_and(|p| p.can_redo),
        ) {
            emit(Intent::Job(Job::History { redo: true }));
        }
        ui.separator();
        let selected = editor.is_some_and(|status| status.selected > 0);
        if item(ui, "Select all hits", "Ctrl+A", editing) {
            emit(Intent::Editor(Act::SelectAll));
        }
        if item(ui, "Deselect", "Esc", selected) {
            emit(Intent::Editor(Act::Deselect));
        }
        if item(ui, "Delete selected hits", "Del", selected) {
            emit(Intent::Editor(Act::DeleteSelection));
        }
        ui.separator();
        if item(ui, "Cut selected hits", "Ctrl+X", selected) {
            emit(Intent::Editor(Act::Cut));
        }
        if item(ui, "Copy selected hits", "Ctrl+C", selected) {
            emit(Intent::Editor(Act::Copy));
        }
        let copied = editor.is_some_and(|status| status.copied > 0);
        if item(ui, "Paste at the playhead", "Ctrl+V", editing && copied) {
            emit(Intent::Editor(Act::Paste));
        }
        ui.separator();
        let channel = editor.and_then(|status| status.channel);
        if item(ui, "Place hit at the playhead", "Enter", channel.is_some()) {
            emit(Intent::Editor(Act::PlaceAtPlayhead));
        }
        ui.separator();
        if item(ui, "Add channel", "", editing) {
            emit(Intent::Job(Job::AddChannel));
        }
        if item(ui, "Rename selected channel", "", channel.is_some())
            && let Some(id) = channel
        {
            emit(Intent::Editor(Act::Rename(id)));
        }
    });

    menu(ui, "View", |ui| {
        for panel in Panel::ALL {
            let on = editor.is_some_and(|status| status.open(panel));
            if check(ui, on, panel.label(), panel.shortcut(), editing) {
                emit(Intent::Editor(Act::Toggle(panel)));
            }
        }
        ui.separator();
        if item(ui, "Zoom in", "Alt+Wheel up", editing) {
            emit(Intent::Editor(Act::ZoomIn));
        }
        if item(ui, "Zoom out", "Alt+Wheel down", editing) {
            emit(Intent::Editor(Act::ZoomOut));
        }
        if item(ui, "Fit the whole song", "", editing) {
            emit(Intent::Editor(Act::Fit));
        }
        if check(
            ui,
            editor.is_some_and(|status| status.follow),
            "Follow the playhead",
            "",
            editing,
        ) {
            emit(Intent::Editor(Act::Follow));
        }
        ui.separator();
        ui.add_enabled_ui(editing, |ui| {
            submenu(ui, "Layer", |ui| {
                ui.set_min_width(t::MENU_WIDTH - 40.0);
                let Some(project) = project else {
                    return;
                };
                let layer = project.layer.file.as_deref();
                if check(ui, layer.is_none(), "Base (all difficulties)", "", true) {
                    emit(Intent::Job(Job::SelectLayer(None)));
                }
                for diff in &project.project.diffs {
                    let exists = project.available_diffs.contains(&diff.file);
                    if check(
                        ui,
                        layer == Some(diff.file.as_str()),
                        &diff.label,
                        "",
                        exists,
                    ) {
                        emit(Intent::Job(Job::SelectLayer(Some(diff.file.clone()))));
                    }
                }
            });
        });
        ui.separator();
        let scale = shell
            .state
            .and_then(|state| state.settings.ui_scale)
            .unwrap_or(1.0);
        ui.add_enabled_ui(ok, |ui| {
            submenu(ui, "Interface scale", |ui| {
                ui.set_min_width(t::MENU_WIDTH - 40.0);
                let next = t::step_scale(ui.ctx(), scale, true);
                if item(ui, "Larger", "Ctrl+=", next.is_some()) {
                    emit(Intent::Job(Job::UiScale(next)));
                }
                let next = t::step_scale(ui.ctx(), scale, false);
                if item(ui, "Smaller", "Ctrl+-", next.is_some()) {
                    emit(Intent::Job(Job::UiScale(next)));
                }
                ui.separator();
                for option in t::UI_SCALES {
                    let shortcut = if option == 1.0 { "Ctrl+0" } else { "" };
                    let fits = t::scale_fits(ui.ctx(), option);
                    if check(ui, scale == option, &t::scale_label(option), shortcut, fits) {
                        emit(Intent::Job(Job::UiScale(Some(option))));
                    }
                }
            });
        });
        ui.separator();
        if item(ui, "Command palette…", "Ctrl+K", true) {
            emit(Intent::Show(Dialog::Palette));
        }
    });

    menu(ui, "Tools", |ui| {
        for tool in Tool::ALL {
            let on = editor.is_some_and(|status| status.tool == tool);
            if check(ui, on, tool.label(), tool.shortcut(), editing) {
                emit(Intent::Editor(Act::Tool(tool)));
            }
        }
        ui.separator();
        ui.add_enabled_ui(editing, |ui| {
            submenu(ui, "Snap", |ui| {
                for divisor in DIVISORS {
                    let on = editor.is_some_and(|status| status.divisor == divisor);
                    if check(ui, on, &format!("1/{divisor}"), "", true) {
                        emit(Intent::Editor(Act::Snap(divisor)));
                    }
                }
            });
            submenu(ui, "Listen to", |ui| {
                let Some(project) = project else {
                    return;
                };
                let stems = project.stem_ready && project.project.audio.monitor == Monitor::Stems;
                if check(ui, !stems, "Song", "T", true) && stems {
                    emit(Intent::Job(Job::Monitor(Monitor::Original)));
                }
                if check(ui, stems, "Drum stem", "T", project.stem_ready) && !stems {
                    emit(Intent::Job(Job::Monitor(Monitor::Stems)));
                }
                if !project.stem_ready {
                    ui.label(t::meta(
                        "No stem yet: separate or import one in Tools",
                        t::DIM,
                    ));
                }
                // Listening mutes (D-77); the levels themselves are in the mixer (F9).
                if let Some(state) = shell.state {
                    ui.separator();
                    let levels = state.settings.levels;
                    if check(ui, levels.music_muted, "Mute music", "", true) {
                        emit(Intent::Job(Job::Levels(hs_session::Levels {
                            music_muted: !levels.music_muted,
                            ..levels
                        })));
                    }
                    if check(ui, levels.hits_muted, "Mute all channels", "", true) {
                        emit(Intent::Job(Job::Levels(hs_session::Levels {
                            hits_muted: !levels.hits_muted,
                            ..levels
                        })));
                    }
                }
            });
        });
        ui.separator();
        let reviewing = editing && project.is_some_and(|p| p.layer.file.is_some());
        if item(ui, "Next issue", "N", reviewing) {
            emit(Intent::StepIssue { backwards: false });
        }
        if item(ui, "Previous issue", "Shift+N", reviewing) {
            emit(Intent::StepIssue { backwards: true });
        }
        ui.separator();
        if item(ui, "Go to the start", "Home", editing) {
            emit(Intent::Transport(Transport::Seek { ms: 0.0 }));
        }
        if item(ui, "Refresh from the mapset", "", open) {
            emit(Intent::Job(Job::Refresh { automatic: false }));
        }
        ui.separator();
        if shell.separating {
            if item(ui, "Cancel drum separation", "", true) {
                emit(Intent::CancelSeparation);
            }
        } else if item(ui, "Separate drum stem from the song", "", open) {
            emit(Intent::Job(Job::Separate));
        }
        if item(ui, "Import drum stem…", "", open) {
            emit(Intent::Job(Job::ImportStem));
        }
    });

    menu(ui, "Help", |ui| {
        if item(ui, "Gestures and shortcuts", "F1", true) {
            emit(Intent::Show(Dialog::Help));
        }
        if item(ui, "Command palette…", "Ctrl+K", true) {
            emit(Intent::Show(Dialog::Palette));
        }
        let warnings = shell.state.map_or(0, |state| {
            usize::from(state.warning.is_some())
                + state.active.as_ref().map_or(0, |p| p.warnings.len())
        });
        if item(ui, &format!("Warnings  {warnings}"), "", warnings > 0) {
            emit(Intent::Show(Dialog::Warnings));
        }
        ui.separator();
        if item(ui, "About hitsounds-rs", "", true) {
            emit(Intent::Show(Dialog::About));
        }
    });
}

pub fn path_label(ui: &mut Ui, path: &Path) {
    let path = path.to_string_lossy();
    let display = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    };
    ui.add(egui::Label::new(t::muted(&display)).truncate())
        .on_hover_text(display);
}

pub fn status(
    ctx: &Context,
    state: Option<&Snapshot>,
    busy: Option<&str>,
    reduced: bool,
    timeline: Option<(&str, Status)>,
    emit: Emit<'_>,
) {
    egui::TopBottomPanel::bottom("status")
        .exact_height(t::STATUS_HEIGHT)
        .frame(
            Frame::new()
                .fill(t::CHROME)
                .inner_margin(Margin::symmetric(t::SPACE as i8, 0)),
        )
        .show(ctx, |ui| {
            let bar = ui.max_rect();
            ui.painter().hline(
                bar.expand2(vec2(t::SPACE, 0.0)).x_range(),
                bar.top(),
                t::stroke(t::EDGE),
            );
            let width = ui.available_width();
            ui.allocate_ui_with_layout(
                vec2(width, t::STATUS_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    if let Some(label) = busy {
                        activity(ui, reduced);
                        ui.label(t::meta(label, t::MUTED));
                    } else if let Some(project) = state.and_then(|state| state.active.as_ref()) {
                        let (label, colour) = if project.dirty {
                            ("Unsaved changes", t::WARNING)
                        } else {
                            ("Saved", t::DIM)
                        };
                        ui.label(t::meta(label, colour));
                    } else {
                        ui.label(t::meta("No project open", t::DIM));
                    }
                    if let Some(state) = state {
                        let warnings = usize::from(state.warning.is_some())
                            + state.active.as_ref().map_or(0, |p| p.warnings.len());
                        if warnings > 0 {
                            let name = if warnings == 1 { "warning" } else { "warnings" };
                            let response = ui.add(
                                Button::new((
                                    t::icon(icons::WARNING).size(t::META_SIZE).color(t::WARNING),
                                    t::meta(format!("{warnings} {name}"), t::WARNING),
                                ))
                                .frame(false),
                            );
                            if widgets::describe(response, "Show the warnings", "").clicked() {
                                emit(Intent::Show(Dialog::Warnings));
                            }
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some((hover, status)) = timeline {
                            ui.label(t::meta(format!("Snap 1/{}", status.divisor), t::DIM));
                            ui.label(t::meta("·", t::EDGE_STRONG));
                            ui.label(t::meta(status.tool.label(), t::DIM));
                            if status.selected > 0 {
                                ui.label(t::meta("·", t::EDGE_STRONG));
                                ui.label(t::meta(
                                    format!("{} selected", status.selected),
                                    t::ACCENT,
                                ));
                            }
                            if !hover.is_empty() {
                                ui.label(t::meta("·", t::EDGE_STRONG));
                                ui.add(egui::Label::new(t::meta(hover, t::MUTED)).truncate());
                            }
                        }
                    });
                },
            );
        });
}

pub fn activity(ui: &mut Ui, reduced: bool) {
    if reduced {
        ui.label(t::icon(icons::CIRCLE_NOTCH).color(t::ACCENT));
    } else {
        ui.add(egui::Spinner::new().size(t::ICON_SIZE).color(t::ACCENT));
    }
}

pub fn landing(
    ui: &mut Ui,
    logo: &TextureHandle,
    state: &Snapshot,
    available: bool,
    entrance: f32,
    emit: Emit<'_>,
) {
    ui.add_space(t::SLIDE * (1.0 - entrance));
    ui.set_opacity(0.82 + entrance * 0.18);
    let wide = ui.available_width() >= t::TWO_COLUMN_WIDTH;
    t::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        if wide {
            ui.columns(2, |cols| {
                hero(&mut cols[0], state, available, emit);
                signal_art(&mut cols[1]);
            });
        } else {
            hero(ui, state, available, emit);
        }
    });
    ui.add_space(t::GAP);
    if wide {
        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.x = t::GUTTER;
            setup(&mut cols[0], state, available, emit);
            recent(&mut cols[1], logo, state, available, emit);
        });
    } else {
        setup(ui, state, available, emit);
        recent(ui, logo, state, available, emit);
    }
}

fn hero(ui: &mut Ui, state: &Snapshot, available: bool, emit: Emit<'_>) {
    ui.set_min_height(t::HERO_HEIGHT);
    ui.add_space(t::GAP);
    ui.label(t::display("hitsounds-rs", t::HERO_SIZE));
    ui.label(t::muted("A hitsound workspace for osu!stable"));
    ui.add_space(t::SPACE);
    ui.add_enabled_ui(available, |ui| {
        ui.horizontal_wrapped(|ui| {
            let (glyph, label, action) = if state.skin_ready {
                (icons::FILE_PLUS, "New project from mapset", Job::Create)
            } else {
                (icons::FOLDER_PLUS, "Set skin folder", Job::ChooseSkin)
            };
            if t::primary(ui, (t::icon(glyph), label)).clicked() {
                emit(Intent::Job(action));
            }
            if ui
                .button((t::icon(icons::FOLDER_OPEN), "Open project…"))
                .on_hover_text("Open project.json (Ctrl+O)")
                .clicked()
            {
                emit(Intent::Job(Job::Open));
            }
            if state.skin_ready
                && ui
                    .button((t::icon(icons::FILE_ARROW_DOWN), "Import hitsound diff…"))
                    .on_hover_text(
                        "Start a project from a mania hitsound layer difficulty: one channel per sample, greenline volume, and its samples copied in",
                    )
                    .clicked()
            {
                emit(Intent::Job(Job::ImportHitsounds));
            }
        });
    });
}

fn signal_art(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), t::HERO_HEIGHT), Sense::hover());
    let p = ui.painter();
    p.rect(
        rect,
        t::CARD_RADIUS,
        t::STAGE,
        t::stroke(t::EDGE),
        StrokeKind::Inside,
    );
    let plot = rect.shrink2(vec2(t::GUTTER, t::GUTTER + t::GAP));
    let cell = plot.width() / 16.0;
    for tick in 0..=16 {
        let x = plot.left() + cell * tick as f32;
        p.vline(
            x,
            plot.y_range(),
            t::stroke(if tick % 4 == 0 { t::EDGE } else { t::GRID }),
        );
    }
    for row in 0..=4 {
        let y = plot.top() + plot.height() * row as f32 / 4.0;
        p.hline(plot.x_range(), y, t::stroke(t::GRID));
    }
    for (row, beats) in [
        (0, &[0.7, 4.3, 8.7, 12.3][..]),
        (1, &[2.5, 6.5, 10.5, 14.5]),
        (2, &[0.7, 2.5, 4.3, 6.5, 8.7, 10.5, 12.3, 14.5]),
        (3, &[2.5, 6.5, 8.7, 14.5]),
    ] {
        let colour = if row == 3 { t::DIFFICULTY } else { t::ACCENT };
        for &beat in beats {
            let x = plot.left() + cell * beat;
            let y = plot.top() + plot.height() * (row as f32 + 0.5) / 4.0;
            let hit = Rect::from_center_size(
                pos2(x, y),
                vec2(t::SMALL + t::HAIRLINE, t::GUTTER - row as f32),
            );
            p.rect_filled(
                hit,
                t::SMALL,
                colour.gamma_multiply(if x < plot.center().x { 0.50 } else { 0.90 }),
            );
        }
    }
    let x = plot.left() + cell * 8.7;
    p.vline(x, plot.y_range(), t::stroke(t::ACCENT));
    p.text(
        pos2(plot.left(), rect.top() + t::SPACE),
        Align2::LEFT_TOP,
        "Base layer",
        egui::FontId::monospace(t::META_SIZE),
        t::DIM,
    );
    p.text(
        pos2(plot.right(), rect.bottom() - t::SPACE),
        Align2::RIGHT_BOTTOM,
        "Difficulty layer",
        egui::FontId::monospace(t::META_SIZE),
        t::DIFFICULTY,
    );
}

fn setup(ui: &mut Ui, state: &Snapshot, available: bool, emit: Emit<'_>) {
    t::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(t::display("Get set up", t::HEADING_SIZE));
        ui.label(t::muted(
            "Choose your samples, then open or create a project",
        ));
        ui.add_space(t::SMALL);
        let has_library = !state.settings.library.is_empty();
        let has_project = state.recent_exists.iter().any(|&exists| exists);
        for (index, name, detail, done, button, job) in [
            (
                1,
                "Skin samples",
                if state.skin_ready {
                    "Default samples configured"
                } else {
                    "For default hitsounds in preview and export"
                },
                state.skin_ready,
                "Choose folder…",
                Job::ChooseSkin,
            ),
            (
                2,
                "osu! Songs folder",
                if state.songs_dir.is_some() {
                    "Mapsets can be picked by name"
                } else {
                    "Pick mapsets by name instead of browsing for folders"
                },
                state.songs_dir.is_some(),
                "Choose folder…",
                Job::ChooseSongs,
            ),
            (
                3,
                "Sample library",
                if has_library {
                    "Library folders configured"
                } else {
                    "Optional folders for your own samples"
                },
                has_library,
                "Add folder…",
                Job::AddLibrary,
            ),
            (
                4,
                "Drum separation",
                if state.model_ready {
                    "Drums model configured"
                } else {
                    "Optional: separate drum stems in the app"
                },
                state.model_ready,
                "Choose model…",
                Job::ChooseModel,
            ),
            (
                5,
                "First project",
                if has_project {
                    "A project is in your recent list"
                } else {
                    "Create from a mapset or a mania hitsound diff"
                },
                has_project,
                "Create…",
                Job::Create,
            ),
        ] {
            ui.separator();
            ui.horizontal(|ui| {
                if done {
                    ui.label(t::icon(icons::CHECK).size(t::META_SIZE).color(t::ACCENT));
                } else {
                    ui.label(t::meta(format!("{index:02}"), t::DIM));
                }
                let text_width = (ui.available_width() - t::SETUP_ACTION_WIDTH - t::GAP)
                    .max(t::MIN_ROW_LABEL_WIDTH);
                ui.allocate_ui_with_layout(
                    vec2(text_width, t::ITEM_HEIGHT),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(text_width);
                        ui.add_space(t::SMALL);
                        ui.label(t::display(name, t::TITLE_SIZE));
                        ui.add(egui::Label::new(t::muted(detail).size(t::META_SIZE)).truncate());
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if done {
                        ui.label(t::meta("Ready", t::MUTED));
                    } else if ui.add_enabled(available, Button::new(button)).clicked() {
                        emit(Intent::Job(job));
                    }
                });
            });
        }
    });
}

fn recent(ui: &mut Ui, logo: &TextureHandle, state: &Snapshot, available: bool, emit: Emit<'_>) {
    t::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(t::display("Recent projects", t::HEADING_SIZE));
        if state.settings.recent.is_empty() {
            ui.label(t::muted("Projects you open will appear here."));
            ui.add_space(t::SPACE);
            Frame::new()
                .fill(t::STAGE)
                .corner_radius(t::CONTROL_RADIUS)
                .inner_margin(t::GUTTER)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.image((logo.id(), vec2(t::CONTROL_HEIGHT, t::CONTROL_HEIGHT)));
                        ui.vertical(|ui| {
                            ui.label(t::display("Pick up where you left off", t::TITLE_SIZE));
                            ui.label(t::muted("Open a project.json to get started."));
                        });
                    });
                });
        } else {
            ui.label(t::muted("Open a saved session"));
            ui.add_space(t::SMALL);
            for (project, exists) in state.settings.recent.iter().zip(&state.recent_exists) {
                ui.push_id(&project.file, |ui| {
                    ui.separator();
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - t::CONTROL_HEIGHT - t::GAP)
                            .max(t::MIN_ROW_LABEL_WIDTH);
                        let glyph = if *exists {
                            icons::FILE_AUDIO
                        } else {
                            icons::WARNING
                        };
                        let response = ui.add_enabled(
                            available && *exists,
                            Button::new((t::icon(glyph), project.name.as_str()))
                                .min_size(vec2(width, t::CONTROL_HEIGHT))
                                .truncate(),
                        );
                        let response = if *exists {
                            response.on_hover_text(project.file.display().to_string())
                        } else {
                            response.on_disabled_hover_text("Project file is missing")
                        };
                        if response.clicked() {
                            emit(Intent::Job(Job::OpenRecent(project.file.clone())));
                        }
                        if ui
                            .add_enabled_ui(available, |ui| {
                                t::icon_button(ui, icons::X, "Remove from recent projects")
                            })
                            .inner
                            .clicked()
                        {
                            emit(Intent::Job(Job::ForgetRecent(project.file.clone())));
                        }
                    });
                    if !exists {
                        ui.label(t::meta("Project file missing", t::WARNING));
                    }
                });
            }
        }
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site that passes app state through"
)]
pub fn dialog(
    ctx: &Context,
    dialog: Dialog,
    state: Option<&Snapshot>,
    available: bool,
    separating: bool,
    reduced: bool,
    logo: &TextureHandle,
    updates: &crate::update::Status,
    emit: Emit<'_>,
) {
    let title = match dialog {
        Dialog::Settings => "App settings",
        Dialog::Project => "Project settings",
        Dialog::Help => "Gestures and shortcuts",
        Dialog::About | Dialog::Encore => "About hitsounds-rs",
        Dialog::Warnings => "Warnings",
        Dialog::Confirm => "Save your changes?",
        Dialog::Palette => "Command palette",
        Dialog::Mapsets => "Choose a mapset",
    };
    let id = Id::new(title);
    let height = ctx.content_rect().height() * 0.67;
    let width = (ctx.content_rect().width() - t::PAGE_MARGIN * 2.0).min(t::MODAL_WIDTH);
    let modal = egui::Modal::new(id)
        .area(
            egui::Modal::default_area(id)
                .default_size(vec2(width, height + t::TITLE_HEIGHT + t::GUTTER * 2.0)),
        )
        .frame(t::overlay())
        .backdrop_color(t::SCRIM)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(t::display(title, t::HEADING_SIZE));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if t::icon_button(ui, icons::X, "Close dialog (Esc)").clicked() {
                        emit(Intent::DismissDialog);
                    }
                });
            });
            ui.add_space(t::GAP);
            egui::ScrollArea::vertical()
                .max_height(height)
                .show(ui, |ui| match dialog {
                    Dialog::Settings => {
                        if let Some(state) = state {
                            ui.add_enabled_ui(available, |ui| {
                                settings(ui, state, reduced, updates, emit)
                            });
                        } else {
                            ui.label(t::muted("Settings are loading."));
                        }
                    }
                    Dialog::Project => {
                        if let Some(state) = state
                            && let Some(project) = state.active.as_deref()
                        {
                            project_settings(ui, state, project, available, separating, emit);
                        } else {
                            ui.label(t::muted("Open a project to change its settings."));
                        }
                    }
                    Dialog::Help => help(ui),
                    Dialog::About | Dialog::Encore => about(ui, logo, dialog == Dialog::Encore),
                    Dialog::Warnings => {
                        if let Some(state) = state {
                            for warning in state
                                .warning
                                .iter()
                                .chain(state.active.iter().flat_map(|p| &p.warnings))
                            {
                                ui.label(t::muted(warning).color(t::WARNING));
                                ui.separator();
                            }
                        }
                    }
                    // Drawn by `Palette::show` and `Picker::show`, which have their own layout.
                    Dialog::Palette | Dialog::Mapsets => {}
                    Dialog::Confirm => {
                        let name = state
                            .and_then(|s| s.active.as_ref())
                            .map_or("this project", |p| p.project.name.as_str());
                        ui.label(t::muted(format!(
                            "Save the changes to {name} before leaving?"
                        )));
                        ui.add_space(t::SPACE);
                        ui.horizontal_wrapped(|ui| {
                            if t::primary(ui, "Save and continue").clicked() {
                                emit(Intent::Confirm(Decision::Save));
                            }
                            if ui.button("Discard changes").clicked() {
                                emit(Intent::Confirm(Decision::Discard));
                            }
                            if ui.button("Cancel").clicked() {
                                emit(Intent::DismissDialog);
                            }
                        });
                    }
                });
        });
    if modal.should_close() {
        emit(Intent::DismissDialog);
    }
}

/// Settings that belong to this project file: its mapset, drum stem and fallback.
fn project_settings(
    ui: &mut Ui,
    state: &Snapshot,
    project: &ProjectView,
    available: bool,
    separating: bool,
    emit: Emit<'_>,
) {
    widgets::help(ui, "These settings are saved in this project only.");
    widgets::section(ui, "Project file", |ui| {
        path_label(ui, &project.file);
    });
    ui.add_enabled_ui(available, |ui| {
        widgets::section(ui, "Mapset", |ui| {
            path_label(ui, &project.project.mapset_dir);
            widgets::help(
                ui,
                "The osu! beatmap folder. hitsounds-rs reloads it when a difficulty changes on disk.",
            );
            ui.horizontal(|ui| {
                if ui
                    .button((t::icon(icons::FOLDER_OPEN), "Change mapset…"))
                    .on_hover_text("Point this project at another folder, such as the real mapset after working from a copy. Channels and hits stay; undo switches back.")
                    .clicked()
                {
                    emit(Intent::Job(Job::ChangeMapset));
                }
                if ui
                    .button((t::icon(icons::ARROWS_CLOCKWISE), "Refresh now"))
                    .clicked()
                {
                    emit(Intent::Job(Job::Refresh { automatic: false }));
                }
            });
        });
        widgets::section(ui, "Drum stem", |ui| {
            drum_stem(ui, state, project, separating, emit);
        });
        widgets::section(ui, "Fallback channel", |ui| {
            let mut fallback = project.project.fallback;
            let current = fallback
                .and_then(|id| project.project.channel(id))
                .map_or("None", |channel| channel.name.as_str());
            egui::ComboBox::from_id_salt("default-fallback")
                .selected_text(current)
                .width(220.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut fallback, None, "None");
                    for channel in &project.project.channels {
                        ui.selectable_value(&mut fallback, Some(channel.id), &channel.name);
                    }
                });
            if fallback != project.project.fallback {
                emit(Intent::Job(Job::Fallback(fallback)));
            }
            widgets::help(
                ui,
                "Plays on every object that no hit lands on. Each difficulty can choose its own in the Difficulty panel.",
            );
        });
        widgets::section(ui, "Hitnormal under lone additions", |ui| {
            let export = &project.project.export;
            let mut choice = export.addition_only_normal;
            egui::ComboBox::from_id_salt("addition-only-normal")
                .selected_text(addition_only_label(choice))
                .width(220.0)
                .show_ui(ui, |ui| {
                    for option in AdditionOnlyNormal::ALL {
                        ui.selectable_value(&mut choice, option, addition_only_label(option));
                    }
                });
            if choice != export.addition_only_normal {
                emit(Intent::Job(Job::Edit(Edit::SetExport {
                    export: ExportSettings {
                        addition_only_normal: choice,
                        ..export.clone()
                    },
                })));
            }
            widgets::help(
                ui,
                "osu! always plays a hitnormal. When a whistle, finish or clap has no hitnormal channel under it, this one plays, in preview and export. A fixed set keeps, say, soft-hitnormal under a normal-set crash.",
            );
        });
    });
}

/// The project's one stem (D-22, D-34): separated in the app (D-70) or imported.
fn drum_stem(
    ui: &mut Ui,
    state: &Snapshot,
    project: &ProjectView,
    separating: bool,
    emit: Emit<'_>,
) {
    let audio = &project.project.audio;
    match &audio.stem {
        Some(stem) => {
            path_label(ui, stem);
            let origin = if audio.stem_origin.is_some() {
                "Separated in hitsounds-rs"
            } else {
                "Imported file"
            };
            ui.label(t::meta(origin, t::DIM));
            if project.stem_outdated {
                ui.label(t::meta(
                    "The song changed since this stem was separated. Separate it again.",
                    t::WARNING,
                ));
            } else if !project.stem_ready {
                ui.label(t::meta("This stem could not be read", t::WARNING));
            }
        }
        None => {
            ui.label(t::muted("No drum stem yet"));
        }
    }
    widgets::help(
        ui,
        "A drum-only track with the same length and offset as the song. Press T in the editor to listen to it instead of the song. Separate runs the HT-Demucs drums model on this computer, in about half the song's length; you can keep editing meanwhile. Import uses a stem made elsewhere, such as a website.",
    );
    ui.horizontal_wrapped(|ui| {
        if separating {
            ui.label(t::meta("Separating…", t::ACCENT));
            if ui.button((t::icon(icons::STOP), "Cancel")).clicked() {
                emit(Intent::CancelSeparation);
            }
        } else {
            let label = if audio.stem_origin.is_some() {
                "Separate again"
            } else {
                "Separate from song"
            };
            let can = state.model_ready && state.runtime_ready;
            let response = if audio.stem.is_none() || project.stem_outdated {
                ui.add_enabled_ui(can, |ui| t::primary(ui, (t::icon(icons::WAVEFORM), label)))
                    .inner
            } else {
                ui.add_enabled(can, Button::new((t::icon(icons::WAVEFORM), label)))
            };
            let why = if !state.runtime_ready {
                "ONNX Runtime (onnxruntime.dll) is missing next to hitsounds-rs. Reinstall it to separate."
            } else {
                "Choose the drums model in App settings first"
            };
            if response.on_disabled_hover_text(why).clicked() {
                emit(Intent::Job(Job::Separate));
            }
        }
        let label = if audio.stem.is_some() {
            "Replace with file…"
        } else {
            "Import file…"
        };
        if ui
            .add_enabled(!separating, Button::new((t::icon(icons::FILE_ARROW_DOWN), label)))
            .clicked()
        {
            emit(Intent::Job(Job::ImportStem));
        }
        if audio.stem.is_some() && ui.add_enabled(!separating, Button::new("Remove")).clicked() {
            emit(Intent::Job(Job::RemoveStem));
        }
    });
    if state.runtime_ready && !state.model_ready && !separating {
        ui.horizontal_wrapped(|ui| {
            ui.label(t::meta("Separation needs the drums model.", t::DIM));
            if ui.link("Set it up in App settings").clicked() {
                emit(Intent::Show(Dialog::Settings));
            }
        });
    }
}

fn settings(
    ui: &mut Ui,
    state: &Snapshot,
    reduced: bool,
    updates: &crate::update::Status,
    emit: Emit<'_>,
) {
    widgets::help(
        ui,
        "These settings apply to every project on this computer.",
    );
    widgets::section(ui, "Skin sample folder", |ui| {
        widgets::help(
            ui,
            "The folder with osu!'s default hitsounds (normal-hitnormal.wav and so on). Channels without their own sample play from here, in the editor and in exports.",
        );
        if let Some(path) = &state.settings.skin_dir {
            path_label(ui, path);
            if !state.skin_ready {
                ui.label(t::meta("This folder is not available", t::WARNING));
            }
        }
        ui.horizontal(|ui| {
            let label = if state.settings.skin_dir.is_some() {
                "Change folder…"
            } else {
                "Choose folder…"
            };
            if ui.button((t::icon(icons::FOLDER_OPEN), label)).clicked() {
                emit(Intent::Job(Job::ChooseSkin));
            }
            if state.settings.skin_dir.is_some() && ui.button("Clear").clicked() {
                emit(Intent::Job(Job::ClearSkin));
            }
        });
    });
    widgets::section(ui, "osu! Songs folder", |ui| {
        widgets::help(
            ui,
            "New project and Change mapset list the mapsets in this folder by name, as osu!'s song select does. The list is kept between runs and checked for changes each time it opens.",
        );
        match (&state.songs_dir, state.songs_detected) {
            (Some(path), true) => {
                path_label(ui, path);
                ui.label(t::meta("Found in your osu! install", t::MUTED));
            }
            (Some(path), false) => path_label(ui, path),
            (None, _) => {
                if let Some(path) = &state.settings.songs_dir {
                    path_label(ui, path);
                    ui.label(t::meta("This folder is not available", t::WARNING));
                } else {
                    ui.label(t::meta(
                        "No osu! install was found in the usual place",
                        t::MUTED,
                    ));
                }
            }
        }
        ui.horizontal(|ui| {
            let label = if state.songs_dir.is_some() {
                "Change folder…"
            } else {
                "Choose folder…"
            };
            if ui.button((t::icon(icons::FOLDER_OPEN), label)).clicked() {
                emit(Intent::Job(Job::ChooseSongs));
            }
            if state.settings.songs_dir.is_some()
                && ui
                    .button("Use the osu! install")
                    .on_hover_text("Forget this folder and use the Songs folder of your osu! install, if there is one")
                    .clicked()
            {
                emit(Intent::Job(Job::ClearSongs));
            }
        });
    });
    widgets::section(ui, "Projects folder", |ui| {
        widgets::help(
            ui,
            "New projects are created here, named after their mapset, without asking. Without a folder, hitsounds-rs asks where each one goes. Projects stay outside the mapset, since unused files must be removed before ranking.",
        );
        if let Some(path) = &state.settings.projects_dir {
            path_label(ui, path);
            if !state.projects_ready {
                ui.label(t::meta(
                    "This folder is not available, so new projects ask where to go",
                    t::WARNING,
                ));
            }
        }
        ui.horizontal(|ui| {
            let label = if state.settings.projects_dir.is_some() {
                "Change folder…"
            } else {
                "Choose folder…"
            };
            if ui.button((t::icon(icons::FOLDER_OPEN), label)).clicked() {
                emit(Intent::Job(Job::ChooseProjectsDir));
            }
            if state.settings.projects_dir.is_some() && ui.button("Ask each time").clicked() {
                emit(Intent::Job(Job::ClearProjectsDir));
            }
        });
    });
    widgets::section(ui, "Sample folders", |ui| {
        widgets::help(
            ui,
            "Folders with your own hitsound samples. They appear in the Browser.",
        );
        for path in &state.settings.library {
            ui.push_id(path, |ui| {
                ui.horizontal(|ui| {
                    if t::icon_button(ui, icons::X, "Remove this folder from the Browser").clicked()
                    {
                        emit(Intent::Job(Job::RemoveLibrary(path.clone())));
                    }
                    path_label(ui, path);
                });
            });
        }
        if ui
            .button((t::icon(icons::FOLDER_PLUS), "Add folder…"))
            .clicked()
        {
            emit(Intent::Job(Job::AddLibrary));
        }
    });
    widgets::section(ui, "Drum separation model", |ui| {
        widgets::help(
            ui,
            "The HT-Demucs FT drums model as an ONNX file, used by Separate from song in Project settings. hitsounds-rs doesn't include it, since the licence of the Demucs weights is unclear. Make it once with tools/demucs-export (see its README), then choose the .onnx file here.",
        );
        if let Some(path) = &state.settings.separation_model {
            path_label(ui, path);
            if !state.model_ready {
                ui.label(t::meta("This file is not available", t::WARNING));
            }
        }
        if !state.runtime_ready {
            ui.label(t::meta(
                "ONNX Runtime (onnxruntime.dll) is missing next to hitsounds-rs, so separation is unavailable.",
                t::WARNING,
            ));
        }
        ui.horizontal(|ui| {
            let label = if state.settings.separation_model.is_some() {
                "Change model…"
            } else {
                "Choose model…"
            };
            if ui
                .add_enabled(
                    state.runtime_ready,
                    Button::new((t::icon(icons::FOLDER_OPEN), label)),
                )
                .on_hover_text("hitsounds-rs loads the file once to check it is the drums model")
                .clicked()
            {
                emit(Intent::Job(Job::ChooseModel));
            }
            if state.settings.separation_model.is_some() && ui.button("Clear").clicked() {
                emit(Intent::Job(Job::ClearModel));
            }
        });
    });
    widgets::section(ui, "Interface scale", |ui| {
        widgets::help(
            ui,
            "Makes text and controls larger or smaller, on top of the Windows display scale. Ctrl+= and Ctrl+- step through the sizes; Ctrl+0 resets.",
        );
        let current = state.settings.ui_scale.unwrap_or(1.0);
        ui.horizontal_wrapped(|ui| {
            for option in t::UI_SCALES {
                let fits = t::scale_fits(ui.ctx(), option);
                let response = ui
                    .add_enabled(
                        fits,
                        Button::selectable(current == option, t::scale_label(option)),
                    )
                    .on_disabled_hover_text(
                        "The window would not fit on this display at this size",
                    );
                if response.clicked() && current != option {
                    emit(Intent::Job(Job::UiScale(Some(option))));
                }
            }
        });
    });
    widgets::section(ui, "Motion", |ui| {
        let mut value = reduced;
        if ui.checkbox(&mut value, "Reduce motion").changed() {
            emit(Intent::Job(Job::ReduceMotion(Some(value))));
        }
        if state.settings.reduce_motion.is_some() {
            if ui.button("Follow the Windows setting").clicked() {
                emit(Intent::Job(Job::ReduceMotion(None)));
            }
        } else {
            widgets::help(ui, "Following the Windows animation setting.");
        }
    });
    updates_section(ui, state, updates, emit);
}

/// D-84: the startup check, its status, and the downloaded update.
fn updates_section(ui: &mut Ui, state: &Snapshot, updates: &crate::update::Status, emit: Emit<'_>) {
    use crate::update::{self, Status};
    widgets::section(ui, "Updates", |ui| {
        widgets::help(
            ui,
            "hitsounds-rs asks GitHub Releases for a newer version when it starts, downloads it in the background and installs it when you close the app. Nothing else is sent.",
        );
        let mut on = state.settings.checks_for_updates();
        if ui
            .checkbox(&mut on, "Check for updates when hitsounds-rs starts")
            .changed()
        {
            emit(Intent::Job(Job::UpdateChecks(on)));
        }
        let colour = if matches!(updates, Status::Failed(_)) {
            t::WARNING
        } else {
            t::MUTED
        };
        ui.label(t::meta(update::describe(updates), colour));
        ui.horizontal_wrapped(|ui| {
            if updates.ready().is_some() {
                if t::primary(ui, (t::icon(icons::ARROW_CLOCKWISE), "Restart to update"))
                    .on_hover_text(RESTART_HINT)
                    .clicked()
                {
                    emit(Intent::RestartToUpdate);
                }
                if ui.button("What's new").clicked() {
                    emit(Intent::UpdateNotes);
                }
            } else if !matches!(updates, Status::Unavailable(_))
                && ui
                    .add_enabled(!updates.busy(), Button::new("Check now"))
                    .clicked()
            {
                emit(Intent::CheckUpdates);
            }
        });
        ui.label(t::meta(format!("Version {}", update::VERSION), t::DIM));
    });
}

const RESTART_HINT: &str =
    "Close hitsounds-rs (asking to save first), install the update and open it again";

fn help(ui: &mut Ui) {
    for (group, shortcuts) in [
        (
            "Placing hits",
            &[
                ("Place a hit", "Click a step"),
                ("Delete a hit", "Right-click it"),
                ("Move hits", "Drag, across time or channels"),
                ("Select hits", "Drag a box"),
                ("Add to the selection", "Ctrl+click"),
                ("Place at the playhead", "Enter"),
                ("Delete the selection", "Delete"),
                ("Select all / clear selection", "Ctrl+A / Esc"),
                ("Copy / cut the selection", "Ctrl+C / Ctrl+X"),
                ("Paste at the playhead", "Ctrl+V"),
            ][..],
        ),
        (
            "Tools",
            &[
                ("Select: box select only", "1"),
                ("Draw: place and move", "2"),
                ("Paint: drag to fill every snap step", "3"),
                ("Delete: click or drag over hits", "4"),
            ],
        ),
        (
            "Listening",
            &[
                ("Play / pause", "Space"),
                ("Go to the start", "Home"),
                ("Song or drum stem", "T"),
                ("Seek", "Click the ruler or waveform"),
                ("Step by snap / four steps", "Wheel / Shift+wheel"),
                ("Zoom at the pointer", "Alt+wheel"),
                ("Change the snap divisor", "Ctrl+wheel"),
            ],
        ),
        (
            "Channels",
            &[
                ("Mute / solo", "Click / right-click the light"),
                ("Volume", "Drag the knob"),
                ("Rename", "Double-click the name"),
                ("Change priority", "Drag the row handle"),
                ("Channel menu", "Right-click the name"),
                ("Scroll channels", "Wheel over the rack"),
            ],
        ),
        (
            "Hit volume",
            &[
                ("Add a point", "Ctrl+click"),
                ("Move a point", "Drag it"),
                ("Switch ramp and step", "Click a point"),
                ("Delete a point", "Right-click it"),
            ],
        ),
        (
            "Difficulties",
            &[
                ("Next / previous issue", "N / Shift+N"),
                ("Add a hit on an object", "Right-click the Objects lane"),
                ("Mute or replace a channel", "Right-click its name"),
            ],
        ),
        (
            "Panels and files",
            &[
                ("Channel settings", "F6"),
                ("Difficulty review", "F7"),
                ("Browser", "F8"),
                ("Mixer", "F9"),
                ("Export", "Ctrl+E"),
                ("Command palette", "Ctrl+K"),
                ("New / open project", "Ctrl+N / Ctrl+O"),
                ("Save / close project", "Ctrl+S / Ctrl+W"),
                ("Undo / redo", "Ctrl+Z / Ctrl+Shift+Z"),
                ("App settings", "Ctrl+,"),
                ("Interface scale larger / smaller", "Ctrl+= / Ctrl+-"),
                ("Interface scale 100%", "Ctrl+0"),
                ("This sheet", "F1"),
            ],
        ),
    ] {
        ui.label(t::display(group, t::TITLE_SIZE));
        for &(action, shortcut) in shortcuts {
            ui.columns(2, |columns| {
                columns[0].add(egui::Label::new(t::muted(action)).wrap());
                if shortcut.contains(' ') {
                    columns[1].add(egui::Label::new(t::meta(shortcut, t::MUTED)).wrap());
                } else {
                    t::keycaps(&mut columns[1], shortcut);
                }
            });
        }
        ui.add_space(t::SPACE);
    }
}

fn about(ui: &mut Ui, logo: &TextureHandle, encore: bool) {
    ui.horizontal(|ui| {
        ui.image((logo.id(), vec2(t::HERO_SIZE, t::HERO_SIZE)));
        ui.vertical(|ui| {
            ui.label(t::display("Introducing hitsounds-rs", t::HEADING_SIZE));
            ui.label(t::muted("Engineered for an audience of one."));
        });
    });
    ui.add_space(t::GAP);
    ui.label(t::meta(
        format!("Version {}  ·  Market of One", env!("CARGO_PKG_VERSION")),
        t::DIM,
    ));
    if encore {
        ui.horizontal_wrapped(|ui| {
            ui.label(t::icon(icons::SPARKLE).color(t::DIFFICULTY));
            ui.label(t::meta(
                "STANDING OVATION FROM 100% OF THE AUDIENCE",
                t::DIFFICULTY,
            ));
        });
    }
}

/// The drum separation progress card (D-72), bottom right above the status bar.
/// Returns its height so the toast can sit above it.
pub fn separation(ctx: &Context, card: &SeparationCard, emit: Emit<'_>) -> f32 {
    let response = egui::Area::new(Id::new("separation"))
        .anchor(
            Align2::RIGHT_BOTTOM,
            vec2(-t::GUTTER, -t::STATUS_HEIGHT - t::SPACE),
        )
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            t::overlay()
                .stroke(t::stroke(t::ACCENT.gamma_multiply(0.45)))
                .show(ui, |ui| {
                    ui.set_width(t::TOAST_WIDTH);
                    ui.horizontal(|ui| {
                        ui.label(t::icon(icons::WAVEFORM).color(t::ACCENT));
                        ui.vertical(|ui| {
                            let title = if card.cancelling {
                                "Cancelling separation…"
                            } else {
                                "Separating drums"
                            };
                            ui.label(RichText::new(title).color(t::TEXT));
                            ui.add(egui::Label::new(t::meta(&card.detail, t::MUTED)).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let response = ui.add_enabled(!card.cancelling, Button::new("Cancel"));
                            if widgets::describe(
                                response,
                                "Stop the separation after the current segment",
                                "",
                            )
                            .clicked()
                            {
                                emit(Intent::CancelSeparation);
                            }
                        });
                    });
                    ui.add_space(t::SMALL);
                    let bar = egui::ProgressBar::new(card.fraction)
                        .desired_height(t::SMALL)
                        .fill(t::ACCENT)
                        .corner_radius(t::SMALL);
                    ui.add(bar).widget_info(|| {
                        WidgetInfo::labeled(WidgetType::ProgressIndicator, true, &card.detail)
                    });
                });
        })
        .response;
    response.rect.height()
}

pub fn toast(ctx: &Context, toast: &Toast, reduced: bool, now: f64, lift: f32, emit: Emit<'_>) {
    let age = now - toast.since;
    let enter = if reduced {
        1.0
    } else {
        t::ease_out((age / t::ENTER_SECONDS) as f32)
    };
    if enter < 1.0 {
        ctx.request_repaint();
    }
    let offset = vec2(
        -t::GUTTER,
        -t::STATUS_HEIGHT - t::SPACE - lift + t::SLIDE * (1.0 - enter),
    );
    egui::Area::new(Id::new("toast"))
        .anchor(Align2::RIGHT_BOTTOM, offset)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_opacity(enter);
            let colour = if toast.error { t::ERROR } else { t::ACCENT };
            t::overlay()
                .stroke(t::stroke(colour.gamma_multiply(0.45)))
                .show(ui, |ui| {
                    ui.set_max_width(t::TOAST_WIDTH);
                    ui.horizontal_top(|ui| {
                        ui.label(
                            t::icon(if toast.error {
                                icons::WARNING
                            } else {
                                icons::CHECK
                            })
                            .color(colour),
                        );
                        let response = ui.add(
                            egui::Label::new(RichText::new(&toast.message).color(t::TEXT)).wrap(),
                        );
                        response.widget_info(|| {
                            WidgetInfo::labeled(WidgetType::Label, true, &toast.message)
                        });
                        if t::icon_button(ui, icons::X, "Dismiss message").clicked() {
                            emit(Intent::DismissToast);
                        }
                    });
                    if toast.action == Some(ToastAction::Update) {
                        ui.horizontal(|ui| {
                            if t::primary(ui, (t::icon(icons::ARROW_CLOCKWISE), "Restart now"))
                                .on_hover_text(RESTART_HINT)
                                .clicked()
                            {
                                emit(Intent::DismissToast);
                                emit(Intent::RestartToUpdate);
                            }
                            if ui.button("What's new").clicked() {
                                emit(Intent::UpdateNotes);
                            }
                            if ui.button("Later").clicked() {
                                emit(Intent::DismissToast);
                            }
                        });
                    }
                    if toast.action == Some(ToastAction::Separate) {
                        ui.horizontal(|ui| {
                            if t::primary(ui, (t::icon(icons::WAVEFORM), "Separate drum stem"))
                                .on_hover_text("Run drum separation on the song now (Project settings has the same button)")
                                .clicked()
                            {
                                emit(Intent::Job(Job::Separate));
                                emit(Intent::DismissToast);
                            }
                            if ui.button("Not now").clicked() {
                                emit(Intent::DismissToast);
                            }
                        });
                    }
                });
        });
}
