//! D-45: a virtualized tree/search view. The worker scans folders and auditions files.

use super::rail::DragItem;
use crate::{
    app::Intent,
    theme as t,
    views::Emit,
    widgets,
    worker::{Job, Snapshot},
};
use eframe::egui::{
    self, Align, Button, Id, Key, Layout, Modifiers, ScrollArea, Sense, TextEdit, Ui,
};
use egui_phosphor::regular as icons;
use hs_core::Channel;
use hs_session::library::LibraryFolder;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_RESULTS: usize = 500;
const ROW: f32 = 22.0;

#[derive(Default)]
pub struct Library {
    listing: Option<Arc<Vec<LibraryFolder>>>,
    expanded: HashSet<PathBuf>,
    rows: Vec<Row>,
    query: String,
    selected: Option<PathBuf>,
    focus: bool,
    reveal: Option<usize>,
}

enum Row {
    Folder {
        path: PathBuf,
        depth: usize,
        count: Option<usize>,
    },
    Sample {
        path: PathBuf,
        depth: usize,
    },
    Note(String),
}

impl Library {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        state: &Snapshot,
        channel: Option<&Channel>,
        ready: bool,
        emit: Emit<'_>,
    ) {
        if state.settings.library.is_empty() {
            ui.add_space(t::GAP);
            ui.label(t::display("No sample folders yet", t::TITLE_SIZE));
            widgets::help(
                ui,
                "Add the folders where you keep your hitsound samples. You can then preview them here and drag them onto channels.",
            );
            ui.add_space(t::SMALL);
            if ui
                .add_enabled(
                    ready,
                    Button::new((t::icon(icons::FOLDER_PLUS), "Add folder…")),
                )
                .clicked()
            {
                emit(Intent::Job(Job::AddLibrary));
            }
            return;
        }
        if !same_listing(self.listing.as_ref(), state.library.as_ref()) {
            let first = self.listing.is_none();
            self.listing = state.library.clone();
            if first && let Some(list) = &self.listing {
                self.expanded
                    .extend(list.iter().map(|folder| folder.root.clone()));
            }
            self.rebuild();
        }
        let search = ui.add(
            TextEdit::singleline(&mut self.query)
                .id(search_id())
                .hint_text("Search samples")
                .desired_width(ui.available_width()),
        );
        if search.changed() {
            self.rebuild();
        }
        // The action bar stays at the bottom of the dock, whatever the list scrolls to.
        egui::TopBottomPanel::bottom("browser-actions")
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                top: t::GAP as i8,
                ..Default::default()
            }))
            .show_separator_line(true)
            .show_inside(ui, |ui| {
                let Some(path) = self.selected.clone() else {
                    widgets::help(
                        ui,
                        "Click a sample to hear it. Double-click it, or drag it onto a channel, to use it.",
                    );
                    return;
                };
                let name = path
                    .file_name()
                    .map_or_else(|| "Sample".into(), |name| name.to_string_lossy());
                ui.add(egui::Label::new(t::meta(name, t::TEXT)).truncate());
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(ready, |ui| {
                        if t::icon_button(ui, icons::PLAY, "Preview the selected sample").clicked()
                        {
                            emit(Intent::Job(Job::AuditionFile(path.clone())));
                        }
                        if let Some(channel) = channel {
                            let load = t::primary(ui, format!("Load into {}", channel.name));
                            if widgets::describe(
                                load,
                                "Replace the selected channel's sample with this file",
                                "Double-click",
                            )
                            .clicked()
                            {
                                use_sample(emit, &path, Some(channel));
                            }
                        }
                        let create = ui.button("New channel");
                        if widgets::describe(create, "Create a channel that plays this sample", "")
                            .clicked()
                        {
                            use_sample(emit, &path, None);
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                top: t::SMALL as i8,
                ..Default::default()
            }))
            .show_inside(ui, |ui| self.rows(ui, channel, ready, emit));
    }

    fn rows(&mut self, ui: &mut Ui, channel: Option<&Channel>, ready: bool, emit: Emit<'_>) {
        let search_focused = ui.memory(|memory| memory.has_focus(search_id()));
        if self.listing.is_none() {
            ui.label(t::muted("Reading folders…"));
            return;
        }
        if self.rows.is_empty() {
            ui.label(t::muted("No samples match."));
            return;
        }
        let focused = search_focused
            || self
                .selected
                .as_ref()
                .and_then(|path| ui.data(|data| data.get_temp::<Id>(sample_id(path))))
                .is_some_and(|id| ui.memory(|memory| memory.has_focus(id)));
        if ready && focused {
            let move_by = ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    1
                } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    -1
                } else {
                    0
                }
            });
            if move_by != 0 {
                self.step(move_by, !search_focused, emit);
            }
        }
        let mut scroll = ScrollArea::vertical()
            .id_salt("library-rows")
            .auto_shrink([false, false]);
        if let Some(index) = self.reveal.take() {
            let height = ui.available_height();
            scroll = scroll.vertical_scroll_offset(
                (index as f32 * (ROW + ui.spacing().item_spacing.y) - height / 2.0).max(0.0),
            );
        }
        let mut toggle = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.spacing_mut().interact_size.y = ROW;
        scroll.show_rows(ui, ROW, self.rows.len(), |ui, range| {
            for row in &self.rows[range] {
                match row {
                    Row::Folder { path, depth, count } => {
                        let name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                        let label =
                            count.map_or_else(|| name.clone(), |count| format!("{name}  {count}"));
                        let glyph = if self.expanded.contains(path) {
                            icons::CARET_DOWN
                        } else {
                            icons::CARET_RIGHT
                        };
                        if row_button(
                            ui,
                            *depth,
                            (t::icon(glyph).size(t::META_SIZE), label),
                            false,
                            None,
                            Sense::click(),
                        )
                        .clicked()
                        {
                            toggle = Some(path.clone());
                        }
                    }
                    Row::Sample { path, depth } => {
                        let name = path
                            .file_name()
                            .map_or_else(|| "Sample".into(), |name| name.to_string_lossy());
                        let selected = self.selected.as_ref() == Some(path);
                        let response = row_button(
                            ui,
                            *depth,
                            (
                                t::icon(icons::FILE_AUDIO).size(t::META_SIZE),
                                name.into_owned(),
                            ),
                            selected,
                            Some(sample_id(path)),
                            Sense::click_and_drag(),
                        )
                        .on_hover_text(path.display().to_string());
                        if response.contains_pointer() {
                            widgets::hint(
                                ui.ctx(),
                                widgets::Hint::new("Sample")
                                    .key("Click", "Preview")
                                    .key("Double-click", "Load into the selected channel")
                                    .key("Drag", "Drop on a channel"),
                            );
                        }
                        if selected && self.focus {
                            response.request_focus();
                            self.focus = false;
                        }
                        if ready {
                            response.dnd_set_drag_payload(DragItem::Sample(path.clone()));
                            if response.double_clicked() {
                                use_sample(emit, path, channel);
                            } else if response.clicked() {
                                self.selected = Some(path.clone());
                                emit(Intent::Job(Job::AuditionFile(path.clone())));
                            }
                        }
                    }
                    Row::Note(note) => {
                        ui.add_sized(
                            [ui.available_width(), ROW],
                            egui::Label::new(t::muted(note)).truncate(),
                        )
                        .on_hover_text(note);
                    }
                }
            }
        });
        if let Some(path) = toggle {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
            self.rebuild();
        }
    }

    fn step(&mut self, direction: i32, focus: bool, emit: Emit<'_>) {
        let current = self.rows.iter().position(
            |row| matches!(row, Row::Sample { path, .. } if Some(path) == self.selected.as_ref()),
        );
        let index = if direction > 0 {
            (current.map_or(0, |i| i + 1)..self.rows.len())
                .find(|&i| matches!(self.rows[i], Row::Sample { .. }))
        } else {
            (0..current.unwrap_or(self.rows.len()))
                .rev()
                .find(|&i| matches!(self.rows[i], Row::Sample { .. }))
        };
        if let Some(i) = index
            && let Row::Sample { path, .. } = &self.rows[i]
        {
            self.selected = Some(path.clone());
            self.reveal = Some(i);
            self.focus = focus;
            emit(Intent::Job(Job::AuditionFile(path.clone())));
        }
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        let Some(listing) = &self.listing else {
            return;
        };
        let terms: Vec<_> = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        if !terms.is_empty() {
            for folder in &**listing {
                for file in &folder.files {
                    let lower = file.to_lowercase();
                    if terms.iter().all(|term| lower.contains(term)) {
                        self.rows.push(Row::Sample {
                            path: folder.root.join(file),
                            depth: 0,
                        });
                        if self.rows.len() == MAX_RESULTS {
                            self.rows.push(Row::Note(
                                "Showing the first 500 results. Refine the search to see others."
                                    .into(),
                            ));
                            return;
                        }
                    }
                }
            }
        } else {
            for folder in &**listing {
                self.rows.push(Row::Folder {
                    path: folder.root.clone(),
                    depth: 0,
                    count: Some(folder.files.len()),
                });
                if !self.expanded.contains(&folder.root) {
                    continue;
                }
                if let Some(error) = &folder.error {
                    self.rows.push(Row::Note(error.clone()));
                } else if folder.truncated {
                    self.rows.push(Row::Note(format!(
                        "Showing the first {} files.",
                        folder.files.len()
                    )));
                }
                let mut ancestors: Vec<(&str, PathBuf, bool)> = Vec::new();
                for file in &folder.files {
                    let dirs = file.rsplit_once('/').map_or("", |(dirs, _)| dirs);
                    let parts: Vec<_> = dirs.split('/').filter(|part| !part.is_empty()).collect();
                    let common = ancestors
                        .iter()
                        .zip(&parts)
                        .take_while(|(a, part)| a.0 == **part)
                        .count();
                    ancestors.truncate(common);
                    for (index, part) in parts.iter().enumerate().skip(common) {
                        let path = ancestors.last().map_or(&folder.root, |a| &a.1).join(part);
                        let visible = ancestors.last().is_none_or(|a| a.2);
                        let open = visible && self.expanded.contains(&path);
                        if visible {
                            self.rows.push(Row::Folder {
                                path: path.clone(),
                                depth: index + 1,
                                count: None,
                            });
                        }
                        ancestors.push((part, path, open));
                    }
                    if ancestors.last().is_none_or(|a| a.2) {
                        self.rows.push(Row::Sample {
                            path: folder.root.join(file),
                            depth: ancestors.len() + 1,
                        });
                    }
                }
            }
        }
    }
}

fn same_listing(a: Option<&Arc<Vec<LibraryFolder>>>, b: Option<&Arc<Vec<LibraryFolder>>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn search_id() -> Id {
    Id::new("browser-search")
}

fn sample_id(path: &Path) -> Id {
    Id::new(("sample-file", path))
}

fn row_button<'a>(
    ui: &mut Ui,
    depth: usize,
    atoms: impl egui::IntoAtoms<'a>,
    selected: bool,
    id: Option<Id>,
    sense: Sense,
) -> egui::Response {
    // A fixed row height: a centred layout would otherwise float a short list mid-panel.
    let size = egui::vec2(ui.available_width(), ROW);
    let result = ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(depth.min(10) as f32 * t::GAP);
        ui.spacing_mut().button_padding = egui::vec2(t::SMALL, 0.0);
        let add = |ui: &mut Ui| {
            ui.add(
                Button::new(atoms)
                    .selected(selected)
                    .frame_when_inactive(selected)
                    .min_size(egui::vec2(0.0, ROW))
                    .sense(sense)
                    .truncate(),
            )
        };
        let response = if let Some(id) = id {
            ui.push_id(id, add).inner
        } else {
            add(ui)
        };
        if let Some(id) = id {
            // Mirror the real widget id for keyboard navigation between virtual rows.
            ui.data_mut(|data| data.insert_temp(id, response.id));
        }
        response
    });
    result.inner
}

fn use_sample(emit: Emit<'_>, path: &Path, channel: Option<&Channel>) {
    emit(Intent::Job(Job::UseSample {
        channel: channel.map(|c| c.id),
        path: Some(path.to_owned()),
    }));
}
