//! The mapset picker (D-82): the osu! Songs folder as one searchable list, in the
//! manner of osu!'s song select. It chooses the mapset for a new project, for Change
//! mapset (D-67), and for a project whose mapset folder moved.
//!
//! The list comes from `hs_session::songs`, cached between runs and refreshed on its
//! own thread each time the picker opens, so it shows at once and catches up.

use crate::{
    app::Intent,
    theme as t,
    views::{Emit, path_label},
    widgets::{self, Segment},
    worker::Job,
};
use eframe::egui::{
    self, Align, Button, Color32, Context, Id, Key, Label, Layout, Modifiers, ScrollArea, Sense,
    TextEdit, Ui, UiBuilder, vec2,
};
use egui_phosphor::regular as icons;
use hs_session::{
    MovedMapset,
    songs::{MapsetEntry, Scan, SongIndex, now_ms, search_terms},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const ROW_HEIGHT: f32 = 46.0;
const WIDTH: f32 = 760.0;

/// What the chosen mapset is for.
pub enum Purpose {
    Create,
    Change,
    Relocate { file: PathBuf, moved: MovedMapset },
}

impl Purpose {
    fn title(&self) -> &'static str {
        match self {
            Self::Create => "New project",
            Self::Change => "Change mapset",
            Self::Relocate { .. } => "Find the mapset",
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Self::Create => "Create project",
            Self::Change => "Use this mapset",
            Self::Relocate { .. } => "Open with this mapset",
        }
    }

    fn job(&self, mapset: Option<PathBuf>) -> Job {
        match (self, mapset) {
            (Self::Create, Some(dir)) => Job::CreateFrom(dir),
            (Self::Create, None) => Job::Create,
            (Self::Change, Some(dir)) => Job::SetMapset(dir),
            (Self::Change, None) => Job::ChangeMapset,
            (Self::Relocate { file, .. }, mapset) => Job::OpenRelocated {
                file: file.clone(),
                mapset,
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Changed,
    Artist,
    Title,
}

/// The Songs list and its refresh, kept while the app runs so the picker opens at once.
#[derive(Default)]
pub struct Songs {
    index: Option<Arc<SongIndex>>,
    scan: Option<Scan>,
    error: Option<String>,
    /// The folder `index` was loaded or scanned for.
    root: Option<PathBuf>,
}

impl Songs {
    /// Shows the list for `root`: loads the cache when the folder changed, and starts a
    /// refresh when `refresh` is set or the folder changed.
    pub fn ensure(&mut self, root: &Path, cache: Option<PathBuf>, refresh: bool, ctx: &Context) {
        let changed = self.root.as_deref() != Some(root);
        if changed {
            self.root = Some(root.to_path_buf());
            self.index = cache
                .as_deref()
                .and_then(SongIndex::load)
                .filter(|index| index.root == root)
                .map(Arc::new);
            self.scan = None;
            self.error = None;
        }
        if (changed || refresh) && self.scan.is_none() {
            let ctx = ctx.clone();
            self.scan = Some(Scan::start(
                root.to_path_buf(),
                cache,
                self.index.clone(),
                move || ctx.request_repaint(),
            ));
        }
    }

    /// Takes a finished refresh. Call every frame.
    pub fn poll(&mut self, ctx: &Context) {
        let Some(scan) = &self.scan else {
            return;
        };
        match scan.take() {
            Some(Ok(index)) => {
                if self.root.as_deref() == Some(scan.root()) {
                    self.index = Some(Arc::new(index));
                    self.error = None;
                }
                self.scan = None;
            }
            Some(Err(error)) => {
                self.error = Some(error);
                self.scan = None;
            }
            None => ctx.request_repaint_after(Duration::from_millis(120)),
        }
    }
}

pub struct Picker {
    pub purpose: Purpose,
    query: String,
    sort: Sort,
    /// The chosen row, by folder, so it survives a refresh.
    selected: Option<String>,
    /// Rows for the current query and sort, as indices into the index's mapsets.
    rows: Vec<usize>,
    rows_for: Option<(String, Sort, usize)>,
    /// A relocation looked for its mapset in this list already.
    looked: usize,
    /// Refresh was pressed; the shell starts it.
    refresh: bool,
}

/// What the picker needs from the shell.
pub struct Env<'a> {
    pub songs: &'a Songs,
    pub songs_dir: Option<&'a Path>,
    pub projects_dir: Option<&'a Path>,
    pub available: bool,
}

impl Picker {
    pub fn new(purpose: Purpose) -> Self {
        Self {
            purpose,
            query: String::new(),
            sort: Sort::Changed,
            selected: None,
            rows: Vec::new(),
            rows_for: None,
            looked: 0,
            refresh: false,
        }
    }

    /// Whether Refresh was pressed since the last call.
    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    fn update_rows(&mut self, index: &SongIndex) {
        let key = (self.query.clone(), self.sort, std::ptr::from_ref(index) as usize);
        if self.rows_for.as_ref() == Some(&key) {
            return;
        }
        let terms = search_terms(&self.query);
        let mut rows: Vec<usize> = (0..index.mapsets.len())
            .filter(|&i| index.mapsets[i].matches(&terms))
            .collect();
        let lower = |s: &str| s.to_lowercase();
        let m = |i: &usize| &index.mapsets[*i];
        match self.sort {
            Sort::Changed => rows.sort_by_key(|i| std::cmp::Reverse(m(i).modified)),
            Sort::Artist => rows.sort_by_cached_key(|i| (lower(&m(i).artist), lower(&m(i).title))),
            Sort::Title => rows.sort_by_cached_key(|i| (lower(&m(i).title), lower(&m(i).artist))),
        }
        let query_changed = self.rows_for.as_ref().is_none_or(|(q, ..)| *q != self.query);
        self.rows = rows;
        self.rows_for = Some(key);
        let still_listed = self.selected.as_ref().is_some_and(|folder| {
            self.rows.iter().any(|&i| index.mapsets[i].folder == *folder)
        });
        if query_changed || !still_listed {
            self.selected = self
                .rows
                .iter()
                .map(|&i| &index.mapsets[i])
                .find(|m| m.editable())
                .map(|m| m.folder.clone());
        }
    }

    pub fn show(&mut self, ctx: &Context, cx: &Env<'_>, emit: Emit<'_>) {
        let index = cx.songs.index.clone();
        if let Some(index) = &index {
            self.update_rows(index);
            // A moved mapset: once a fresh list arrives, select where it went.
            if let Purpose::Relocate { moved, .. } = &self.purpose
                && self.looked != std::ptr::from_ref(index.as_ref()) as usize
            {
                self.looked = std::ptr::from_ref(index.as_ref()) as usize;
                if let Some(found) = index.relocate(&moved.mapset_dir, &moved.diff_files) {
                    self.selected = index
                        .mapsets
                        .iter()
                        .find(|m| index.path_of(m) == found)
                        .map(|m| m.folder.clone());
                }
            }
        }
        let position = |picker: &Self, index: &SongIndex| {
            picker
                .rows
                .iter()
                .position(|&i| Some(&index.mapsets[i].folder) == picker.selected.as_ref())
        };

        // Arrows move the selection; take them before the search field moves its caret.
        let mut moved = false;
        let mut choose = false;
        if let Some(index) = &index
            && !self.rows.is_empty()
        {
            ctx.input_mut(|i| {
                let step = if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    1
                } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    -1
                } else {
                    0
                };
                if step != 0 {
                    let last = self.rows.len() as isize - 1;
                    let next = position(self, index)
                        .map_or(0, |p| (p as isize + step).clamp(0, last));
                    self.selected = Some(index.mapsets[self.rows[next as usize]].folder.clone());
                    moved = true;
                }
                choose = i.consume_key(Modifiers::NONE, Key::Enter);
            });
        }

        let id = Id::new("mapset-picker");
        let content = ctx.content_rect();
        let width = (content.width() - t::PAGE_MARGIN * 2.0).min(WIDTH);
        let list_height = (content.height() * 0.72 - 190.0).max(ROW_HEIGHT * 3.0);
        let mut chosen: Option<Option<PathBuf>> = None;
        let modal = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).default_size(vec2(width, 0.0)))
            .frame(t::overlay())
            .backdrop_color(t::SCRIM)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(t::display(self.purpose.title(), t::HEADING_SIZE));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if t::icon_button(ui, icons::X, "Close (Esc)").clicked() {
                            emit(Intent::DismissDialog);
                        }
                    });
                });
                widgets::help(ui, &self.subtitle(cx.projects_dir));
                ui.add_space(t::GAP);

                let Some(songs_dir) = cx.songs_dir else {
                    no_songs_folder(ui, cx.available, emit);
                    ui.add_space(t::SPACE);
                    if self.footer(ui, cx, None, emit) {
                        chosen = Some(None);
                    }
                    return;
                };

                ui.horizontal(|ui| {
                    let sort = widgets::segmented(
                        ui,
                        self.sort,
                        &[
                            segment(Sort::Changed, "Newest", "Most recently changed first"),
                            segment(Sort::Artist, "Artist", "By artist, then title"),
                            segment(Sort::Title, "Title", "By title, then artist"),
                        ],
                        true,
                    );
                    if let Some(sort) = sort {
                        self.sort = sort;
                    }
                    let field = ui.add(
                        TextEdit::singleline(&mut self.query)
                            .hint_text("Search artist, title, mapper, difficulty or set ID")
                            .desired_width(f32::INFINITY),
                    );
                    if !field.has_focus() && !ctx.memory(|m| m.focused().is_some()) {
                        field.request_focus();
                    }
                });
                ui.add_space(t::SMALL);
                self.refresh |= self.status_line(ui, cx, songs_dir, index.as_deref());
                ui.add_space(t::SMALL);

                let selected_row = index.as_deref().and_then(|index| position(self, index));
                egui::Frame::new()
                    .fill(t::INSET)
                    .stroke(t::stroke(t::EDGE))
                    .corner_radius(t::CONTROL_RADIUS)
                    .inner_margin(t::SMALL)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_height(list_height);
                        match &index {
                            None => empty(ui, list_height, &scan_text(cx.songs)),
                            Some(_) if self.rows.is_empty() => {
                                let text = if self.query.trim().is_empty() {
                                    "No mapsets in this folder. Choose the Songs folder in App settings, or browse for the mapset.".to_owned()
                                } else {
                                    format!("No mapset matches \"{}\".", self.query.trim())
                                };
                                empty(ui, list_height, &text);
                            }
                            Some(index) => {
                                if let Some(pick) = self.list(ui, index, selected_row, moved) {
                                    chosen = Some(Some(pick));
                                }
                            }
                        }
                    });
                ui.add_space(t::SPACE);
                let selection = index
                    .as_deref()
                    .zip(selected_row)
                    .map(|(index, row)| &index.mapsets[self.rows[row]])
                    .filter(|m| m.editable())
                    .map(|m| index.as_deref().expect("selected from the index").path_of(m));
                if (self.footer(ui, cx, selection.clone(), emit) || choose) && selection.is_some() {
                    chosen = Some(selection);
                }
            });
        if let Some(mapset) = chosen {
            emit(Intent::Pick(self.purpose.job(mapset)));
        } else if modal.should_close() {
            emit(Intent::DismissDialog);
        }
    }

    fn subtitle(&self, projects_dir: Option<&Path>) -> String {
        match &self.purpose {
            Purpose::Create => match projects_dir {
                Some(dir) => format!(
                    "Choose the mapset to hitsound. The project goes in {}.",
                    plain(dir)
                ),
                None => "Choose the mapset to hitsound. You choose where the project goes next."
                    .into(),
            },
            Purpose::Change => {
                "Channels, hits and overrides stay. Difficulty settings carry over by file name."
                    .into()
            }
            Purpose::Relocate { moved, .. } => format!(
                "The mapset folder of {} is gone ({}). Choose where it is now; saving the project keeps the new folder.",
                moved.name,
                plain(&moved.mapset_dir)
            ),
        }
    }

    /// The count, refresh state and folder. True when Refresh was clicked.
    fn status_line(&self, ui: &mut Ui, cx: &Env<'_>, root: &Path, index: Option<&SongIndex>) -> bool {
        let mut refresh = false;
        ui.horizontal(|ui| {
            let count = index.map_or(0, |i| i.mapsets.len());
            let text = if index.is_none() {
                String::new()
            } else if self.query.trim().is_empty() {
                format!("{} mapsets", thousands(count))
            } else {
                format!("{} of {} mapsets", thousands(self.rows.len()), thousands(count))
            };
            ui.label(t::meta(text, t::MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                refresh = ui
                    .add_enabled(cx.songs.scan.is_none(), Button::new((t::icon(icons::ARROW_CLOCKWISE), "Refresh")))
                    .on_hover_text("Read the Songs folder again for new and changed mapsets")
                    .on_disabled_hover_text("Checking the Songs folder for changes")
                    .clicked();
                if cx.songs.scan.is_some() && index.is_some() {
                    ui.label(t::meta(scan_text(cx.songs), t::DIM));
                } else if let Some(error) = &cx.songs.error {
                    ui.label(t::meta(error, t::WARNING));
                } else if let Some(n) = index.map(|i| i.unreadable).filter(|&n| n > 0) {
                    ui.label(t::meta(format!("{n} unreadable folders skipped"), t::DIM));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(t::meta("in", t::DIM));
                    path_label(ui, root);
                });
            });
        });
        refresh
    }

    /// The rows; returns a double-clicked mapset.
    fn list(&mut self, ui: &mut Ui, index: &SongIndex, selected: Option<usize>, moved: bool) -> Option<PathBuf> {
        let mut picked = None;
        let now = now_ms();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in range {
                    let mapset = &index.mapsets[self.rows[row]];
                    let response = mapset_row(ui, mapset, selected == Some(row), now);
                    if moved && selected == Some(row) {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() {
                        self.selected = Some(mapset.folder.clone());
                    }
                    if response.double_clicked() && mapset.editable() {
                        picked = Some(index.path_of(mapset));
                    }
                }
            });
        if selected.is_none() && moved {
            ui.scroll_to_cursor(None);
        }
        picked
    }

    /// Browse, Cancel and the action. True when the action was clicked.
    fn footer(&self, ui: &mut Ui, cx: &Env<'_>, selection: Option<PathBuf>, emit: Emit<'_>) -> bool {
        let mut act = false;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(cx.available, |ui| {
                if ui
                    .button((t::icon(icons::FOLDER_OPEN), "Browse for a folder…"))
                    .on_hover_text("Choose the mapset folder in a file dialog instead")
                    .clicked()
                {
                    emit(Intent::Pick(self.purpose.job(None)));
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(cx.available && selection.is_some(), |ui| {
                    let response = t::primary(ui, self.purpose.action());
                    let response = if cx.songs_dir.is_some() {
                        response.on_disabled_hover_text(
                            "Select a mapset with a standard or catch difficulty",
                        )
                    } else {
                        response
                    };
                    act = response.clicked();
                });
                if ui.button("Cancel").clicked() {
                    emit(Intent::DismissDialog);
                }
                if let Some(dir) = &selection {
                    ui.add(Label::new(t::meta(plain(dir), t::DIM)).truncate());
                }
            });
        });
        act
    }
}

fn segment(value: Sort, label: &'static str, hint: &'static str) -> Segment<'static, Sort> {
    Segment {
        value,
        icon: "",
        label,
        hint,
        shortcut: "",
        enabled: true,
    }
}

fn mapset_row(ui: &mut Ui, mapset: &MapsetEntry, selected: bool, now: u64) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let editable = mapset.editable();
    let fill = if selected {
        t::ACCENT_SURFACE
    } else if response.hovered() {
        t::HOVER
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, t::CONTROL_RADIUS, fill);
    let (name_colour, meta_colour) = match (editable, selected) {
        (false, _) => (t::DIM, t::DIM),
        (true, true) => (t::TEXT, t::MUTED),
        (true, false) => (t::TEXT, t::DIM),
    };
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(t::SPACE, 5.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    child.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !editable {
                widgets::tag(ui, unusable_reason(mapset), t::DIM);
            } else if mapset.diffs.iter().filter(|d| d.editable()).all(|d| d.mode == 2) {
                widgets::tag(ui, "Catch", t::MUTED);
            }
            if let Some(id) = mapset.set_id {
                ui.label(t::meta(format!("#{id}"), t::DIM));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    Label::new(t::display(mapset.display_name(), t::TITLE_SIZE).color(name_colour))
                        .truncate()
                        .selectable(false),
                );
            });
        });
    });
    let mut details = Vec::new();
    if !mapset.creator.is_empty() {
        details.push(format!("mapped by {}", mapset.creator));
    }
    let names: Vec<&str> = mapset
        .diffs
        .iter()
        .filter(|d| d.editable() || !editable)
        .map(|d| d.version.as_str())
        .filter(|v| !v.is_empty())
        .collect();
    if !names.is_empty() {
        details.push(names.join(", "));
    }
    let others = mapset.diffs.iter().filter(|d| editable && !d.editable()).count();
    if others > 0 {
        details.push(format!("+{others} other mode{}", if others == 1 { "" } else { "s" }));
    }
    if mapset.modified > 0 {
        details.push(ago(now.saturating_sub(mapset.modified)));
    }
    child.add(
        Label::new(t::muted(details.join("  ·  ")).size(12.0).color(meta_colour))
            .truncate()
            .selectable(false),
    );
    let response = if editable {
        response
    } else {
        response.on_hover_text("hitsounds-rs works on standard and catch difficulties, and this mapset has neither")
    };
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn unusable_reason(mapset: &MapsetEntry) -> &'static str {
    let modes: Vec<u8> = mapset.diffs.iter().map(|d| d.mode).collect();
    if modes.iter().all(|&m| m == 3) {
        "Mania only"
    } else if modes.iter().all(|&m| m == 1) {
        "Taiko only"
    } else {
        "No standard or catch"
    }
}

fn no_songs_folder(ui: &mut Ui, available: bool, emit: Emit<'_>) {
    egui::Frame::new()
        .fill(t::STAGE)
        .corner_radius(t::CONTROL_RADIUS)
        .inner_margin(t::GUTTER)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(t::display("Set your osu! Songs folder", t::TITLE_SIZE));
            widgets::help(
                ui,
                "hitsounds-rs lists the mapsets in it so you can pick one by name. No osu! install was found in the usual place, so choose the folder once; it's kept in App settings.",
            );
            ui.add_space(t::GAP);
            if ui
                .add_enabled(available, Button::new((t::icon(icons::FOLDER_OPEN), "Choose Songs folder…")))
                .clicked()
            {
                emit(Intent::Job(Job::ChooseSongs));
            }
        });
}

fn empty(ui: &mut Ui, height: f32, text: &str) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            ui.label(t::muted(text));
        },
    );
}

fn scan_text(songs: &Songs) -> String {
    match &songs.scan {
        Some(scan) => {
            let (done, total) = scan.progress();
            let verb = if songs.index.is_some() {
                "Checking for changes"
            } else {
                "Reading the Songs folder"
            };
            if total == 0 {
                format!("{verb}…")
            } else {
                format!("{verb}… {} of {}", thousands(done), thousands(total))
            }
        }
        None => songs
            .error
            .clone()
            .unwrap_or_else(|| "The Songs folder could not be read.".into()),
    }
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "changed today", "changed 3 days ago" and so on.
fn ago(ms: u64) -> String {
    const DAY: u64 = 24 * 60 * 60 * 1000;
    let days = ms / DAY;
    let (n, unit) = match days {
        0 => return "changed today".into(),
        1 => return "changed yesterday".into(),
        2..=13 => (days, "day"),
        14..=59 => (days / 7, "week"),
        60..=729 => (days / 30, "month"),
        _ => (days / 365, "year"),
    };
    format!("changed {n} {unit}s ago")
}

/// A path without the Windows verbatim prefix.
fn plain(path: &Path) -> String {
    let path = path.to_string_lossy();
    path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_ages_read_naturally() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(1_234_567), "1,234,567");
        const DAY: u64 = 24 * 60 * 60 * 1000;
        assert_eq!(ago(DAY / 2), "changed today");
        assert_eq!(ago(DAY), "changed yesterday");
        assert_eq!(ago(5 * DAY), "changed 5 days ago");
        assert_eq!(ago(21 * DAY), "changed 3 weeks ago");
        assert_eq!(ago(400 * DAY), "changed 13 months ago");
        assert_eq!(ago(800 * DAY), "changed 2 years ago");
    }
}
