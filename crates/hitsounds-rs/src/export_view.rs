//! D-52: a full export workspace with a live plan and a changed-lines comparison.

use crate::{app::Intent, theme as t, views, widgets, worker::Job};
use eframe::egui::{self, Align, Button, Context, Frame, Layout, ScrollArea, Ui};
use egui_phosphor::regular as icons;
use hs_core::project::{AdditionOnlyNormal, ExportSettings, SampleFormat, WriteMode};
use hs_export::{
    ExportPlan, SampleFix, SampleIssue,
    export::{DiffExport, DiffStats},
};
use hs_session::CompletedExport;
use similar::{ChangeTag, TextDiff};
use std::time::Duration;

pub struct ExportReview {
    mode: WriteMode,
    diffs: Vec<Difficulty>,
    samples: Vec<Sample>,
    problems: Vec<String>,
    warnings: Vec<String>,
    stale: Vec<String>,
    taken_over: Vec<String>,
}

struct Difficulty {
    file: String,
    label: String,
    /// The base hitsound difficulty (D-95).
    base: bool,
    stats: DiffStats,
}

struct Sample {
    name: String,
    content: String,
    size: usize,
    issues: Vec<SampleIssue>,
    fixes: Vec<SampleFix>,
}

impl ExportReview {
    pub fn new(plan: &ExportPlan) -> Self {
        Self {
            mode: plan.mode,
            diffs: plan
                .diffs
                .iter()
                .map(|d| Difficulty {
                    file: d.file.clone(),
                    label: d.label.clone(),
                    base: d.base,
                    stats: d.stats.clone(),
                })
                .collect(),
            samples: plan
                .samples
                .iter()
                .map(|s| Sample {
                    name: s.name.clone(),
                    content: s.content.clone(),
                    size: s.size,
                    issues: s.issues.clone(),
                    fixes: s.fixes.clone(),
                })
                .collect(),
            problems: plan.problems.clone(),
            warnings: plan.warnings.clone(),
            stale: plan.stale.clone(),
            taken_over: plan.taken_over.clone(),
        }
    }
}

pub struct LineDiff {
    file: String,
    label: String,
    rows: Vec<Line>,
}

struct Line {
    number: usize,
    added: bool,
    text: String,
}

impl LineDiff {
    pub fn new(diff: &DiffExport) -> Self {
        let changes = TextDiff::configure()
            .timeout(Duration::from_secs(1))
            .diff_lines(&diff.original, &diff.updated);
        let rows = changes
            .iter_all_changes()
            .filter_map(|line| {
                let (added, index) = match line.tag() {
                    ChangeTag::Equal => return None,
                    ChangeTag::Insert => (true, line.new_index()?),
                    ChangeTag::Delete => (false, line.old_index()?),
                };
                Some(Line {
                    added,
                    number: index + 1,
                    text: line.value().trim_end_matches(['\r', '\n']).into(),
                })
            })
            .collect();
        Self {
            file: diff.file.clone(),
            label: diff.label.clone(),
            rows,
        }
    }
}

/// A difficulty as the export view lists it, with its include switch (D-94).
#[derive(Debug, Clone, PartialEq)]
pub struct Included {
    pub file: String,
    pub label: String,
    /// The `.osu` file is in the mapset.
    pub exists: bool,
    pub enabled: bool,
}

impl Included {
    pub fn list(view: &crate::worker::ProjectView) -> Vec<Self> {
        view.project
            .diffs
            .iter()
            .map(|d| Self {
                file: d.file.clone(),
                label: d.label.clone(),
                exists: view.available_diffs.contains(&d.file),
                enabled: d.enabled,
            })
            .collect()
    }
}

/// What a preview is built from: the export options and which difficulties to write.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportRequest {
    pub settings: ExportSettings,
    /// Each difficulty's file and whether it is included (D-94).
    pub included: Vec<(String, bool)>,
}

pub struct ExportPage {
    options: ExportSettings,
    included: Vec<Included>,
    requested: Option<ExportRequest>,
    build_at: f64,
    review: Option<Box<ExportReview>>,
    comparison: Option<LineDiff>,
    completed: Option<CompletedExport>,
    error: Option<String>,
}

impl ExportPage {
    pub fn new(options: ExportSettings, included: Vec<Included>) -> Self {
        Self {
            options,
            included,
            requested: None,
            build_at: 0.0,
            review: None,
            comparison: None,
            completed: None,
            error: None,
        }
    }

    pub fn invalidate(&mut self) {
        self.requested = None;
        self.review = None;
        self.comparison = None;
        self.error = None;
    }

    fn request(&self) -> ExportRequest {
        ExportRequest {
            settings: self.options.clone(),
            included: self
                .included
                .iter()
                .map(|d| (d.file.clone(), d.enabled))
                .collect(),
        }
    }

    fn current(&self) -> bool {
        self.requested.as_ref() == Some(&self.request())
    }

    pub fn build(&mut self, ctx: &Context, now: f64) -> Option<Job> {
        if self.current() || self.completed.is_some() {
            return None;
        }
        if now < self.build_at {
            ctx.request_repaint_after(Duration::from_secs_f64(self.build_at - now));
            return None;
        }
        let request = self.request();
        self.requested = Some(request.clone());
        Some(Job::BuildExport(request))
    }

    pub fn review(&mut self, review: Box<ExportReview>) {
        if self.current() {
            self.review = Some(review);
        }
    }

    pub fn compare(&mut self, diff: LineDiff) {
        if self.review.is_some() {
            self.comparison = Some(diff);
        }
    }

    pub fn hide_diff(&mut self) {
        self.comparison = None;
    }

    pub fn finish(&mut self, report: CompletedExport) {
        self.completed = Some(report);
        self.review = None;
        self.comparison = None;
    }

    pub fn fail(&mut self, error: String) {
        self.review = None;
        self.comparison = None;
        self.error = Some(error);
    }

    pub fn can_write(&self) -> bool {
        self.current()
            && self
                .review
                .as_ref()
                .is_some_and(|plan| plan.problems.is_empty() && !plan.diffs.is_empty())
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        now: f64,
        busy: Option<&str>,
        reduced: bool,
        emit: views::Emit<'_>,
    ) {
        egui::TopBottomPanel::top("export-title")
            .frame(Frame::new().fill(t::CHROME).inner_margin(t::SPACE))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button((t::icon(icons::ARROW_LEFT), "Back to editor"))
                        .clicked()
                    {
                        emit(Intent::CloseExport);
                    }
                    ui.label(t::display("Export hitsounds", t::HEADING_SIZE));
                    if let Some(action) = busy {
                        views::activity(ui, reduced);
                        ui.label(t::muted(action));
                    }
                });
            });
        egui::SidePanel::left("export-options")
            .resizable(false)
            .exact_width(t::EXPORT_OPTIONS_WIDTH.min(ctx.content_rect().width() * 0.41))
            .frame(Frame::new().fill(t::SURFACE).inner_margin(t::GUTTER))
            .show(ctx, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    let before = self.request();
                    ui.add_enabled_ui(busy != Some("Writing export"), |ui| {
                        options(ui, &mut self.options, &mut self.included)
                    });
                    if self.request() != before {
                        self.completed = None;
                        self.invalidate();
                        // Coalesce consecutive changes from a number drag.
                        self.build_at = now + t::OPTION_DEBOUNCE;
                        ctx.request_repaint_after(Duration::from_secs_f64(t::OPTION_DEBOUNCE));
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(Frame::new().fill(t::BASE).inner_margin(t::GUTTER))
            .show(ctx, |ui| {
                if let Some(report) = &self.completed {
                    done(ui, report);
                } else if let Some(error) = &self.error {
                    ui.label(t::display("Export needs attention", t::HEADING_SIZE));
                    ui.colored_label(t::ERROR, error);
                    if ui
                        .add_enabled(busy.is_none(), Button::new("Refresh preview"))
                        .clicked()
                    {
                        self.invalidate();
                    }
                } else if let Some(plan) = &self.review {
                    self.preview(ui, plan, busy, emit);
                } else {
                    ui.horizontal(|ui| {
                        views::activity(ui, reduced);
                        ui.label(t::display("Preparing preview", t::HEADING_SIZE));
                    });
                    ui.label(t::muted(
                        "Checking the difficulties, samples, and output paths.",
                    ));
                }
            });
    }

    fn preview(&self, ui: &mut Ui, plan: &ExportReview, busy: Option<&str>, emit: views::Emit<'_>) {
        let count = plan.diffs.iter().filter(|d| !d.base).count();
        let base = plan.diffs.iter().any(|d| d.base);
        ui.horizontal_wrapped(|ui| {
            ui.label(t::display("Review", t::HEADING_SIZE));
            widgets::tag(ui, plural(count, "difficulty", "difficulties"), t::ACCENT);
            if base {
                widgets::tag(ui, "Base hitsound difficulty", t::DIFFICULTY);
            }
            widgets::tag(
                ui,
                plural(plan.samples.len(), "sample", "samples"),
                t::MUTED,
            );
        });
        ui.add_enabled_ui(busy.is_none() && self.can_write(), |ui| {
            if t::primary(
                ui,
                (t::icon(icons::EXPORT), write_label(plan.mode, count, base)),
            )
            .clicked()
            {
                emit(Intent::Job(Job::WriteExport));
            }
        });
        ui.add_space(t::GAP);
        ScrollArea::vertical().id_salt("export-review").show(ui, |ui| {
            if !plan.problems.is_empty() {
                ui.label(t::display("Export blocked", t::TITLE_SIZE).color(t::ERROR));
                for problem in &plan.problems { ui.colored_label(t::ERROR, problem); }
            }
            if !plan.warnings.is_empty() {
                ui.collapsing(format!("Warnings ({})", plan.warnings.len()), |ui| {
                    for warning in &plan.warnings { ui.colored_label(t::WARNING, warning); }
                });
            }
            if plan.mode == WriteMode::Direct && plan.diffs.iter().any(|d| d.stats.empty > 0) {
                ui.label(t::muted("Empty edges will return to default hitsounds. Changed difficulties are backed up."));
            }
            if plan.diffs.is_empty() && plan.problems.is_empty() {
                ui.label(t::muted("Choose a difficulty under What to export."));
            }
            for diff in &plan.diffs {
                ui.push_id(&diff.file, |ui| {
                    ui.separator();
                    ui.label(t::display(&diff.label, t::TITLE_SIZE));
                    let s = &diff.stats;
                    let removed = if s.greenlines_removed > 0 {
                        format!(" ({} redundant removed)", s.greenlines_removed)
                    } else {
                        String::new()
                    };
                    if diff.base {
                        // Every circle carries a hit, so the projection counts say nothing.
                        ui.label(t::meta(&diff.file, t::DIM));
                        ui.label(t::muted(format!("{} circles, one per base-layer moment  ·  {} greenlines", s.edges, s.greenlines_after)));
                    } else {
                        ui.label(t::muted(format!("{} objects changed  ·  {} → {} greenlines{removed}", s.objects_changed, s.greenlines_before, s.greenlines_after)));
                        ui.label(t::meta(format!("{} matched  ·  {} fallback  ·  {} muted  ·  {} empty  ·  {} orphaned", s.matched, s.fallback, s.muted, s.empty, s.orphaned), t::DIM));
                    }
                    if s.ticks > 0 || s.slider_bodies > 0 {
                        ui.label(t::muted(format!("{} custom ticks  ·  {} slider body overrides", s.ticks, s.slider_bodies)));
                    }
                    if let Some(comparison) = self.comparison.as_ref().filter(|item| item.file == diff.file) {
                        if ui.button("Hide comparison").clicked() { emit(Intent::HideExportDiff); }
                        show_diff(ui, comparison);
                    } else if ui.add_enabled(busy.is_none(), Button::new("Compare .osu")).clicked() {
                        emit(Intent::Job(Job::ReadDiff(diff.file.clone())));
                    }
                });
            }
            ui.separator();
            ui.collapsing(format!("Generated samples ({})", plan.samples.len()), |ui| {
                if plan.samples.is_empty() { ui.label(t::muted("This export uses skin samples.")); }
                for sample in &plan.samples {
                    ui.label(t::display(&sample.name, t::TITLE_SIZE));
                    ui.label(t::muted(format!("{}  ·  {} bytes", sample.content, sample.size)));
                    for issue in &sample.issues {
                        let message = match issue {
                            SampleIssue::TooShort { ms } => format!("Too short ({ms:.1} ms)"),
                            SampleIssue::LatePeak { ms } => format!("Late impact ({ms:.1} ms)"),
                            SampleIssue::SilentNotBlank => "Silent sample is longer than 44 bytes".into(),
                            SampleIssue::TooSmall { bytes } => format!("Under 1 KB ({bytes} bytes): osu! plays it as silence"),
                        };
                        ui.colored_label(t::WARNING, message);
                    }
                    for fix in &sample.fixes {
                        let message = match fix {
                            SampleFix::TrimmedLeadingSilence { ms } => format!("Trimmed {ms:.1} ms of leading silence"),
                            SampleFix::PaddedTo { ms } => format!("Padded to {ms:.1} ms"),
                        };
                        ui.colored_label(t::ACCENT, message);
                    }
                }
            });
            for (label, files) in [("Remove previous generated files", &plan.stale), ("Back up existing mapset samples", &plan.taken_over)] {
                if !files.is_empty() {
                    ui.collapsing(format!("{label} ({})", files.len()), |ui| {
                        for file in files { ui.label(t::muted(file)); }
                    });
                }
            }
        });
    }
}

fn options(ui: &mut Ui, options: &mut ExportSettings, included: &mut [Included]) {
    widgets::section(ui, "Where to write", |ui| {
        ui.radio_value(
            &mut options.write_mode,
            WriteMode::Direct,
            "Into the mapset",
        );
        widgets::help(
            ui,
            "Updates every included difficulty in place. Each changed file is backed up first.",
        );
        ui.radio_value(
            &mut options.write_mode,
            WriteMode::HitsoundOnly,
            "Hitsound-only copies",
        );
        widgets::help(
            ui,
            "Writes separate .osu files and samples into the project folder. The mapset is not touched.",
        );
    });
    // D-94: which difficulties to write, the same switch as each difficulty's Rules.
    widgets::section(ui, "What to export", |ui| {
        if included.is_empty() {
            widgets::help(ui, "This project has no difficulties yet.");
        }
        for diff in included.iter_mut() {
            let label = if diff.exists {
                diff.label.clone()
            } else {
                format!("{} (file missing)", diff.label)
            };
            let response =
                ui.add_enabled(diff.exists, egui::Checkbox::new(&mut diff.enabled, label));
            widgets::describe(response, &diff.file, "");
        }
        widgets::help(
            ui,
            "Difficulties left out are not touched and keep their hitsounds. The same switch is in each difficulty's Rules.",
        );
        // D-95: the base layer as one hitsound difficulty, the way hitsounds were made
        // before tools wrote into every difficulty.
        ui.add_space(t::SMALL);
        ui.label(t::meta("Also write", t::DIM));
        ui.checkbox(&mut options.base_hitsound_diff, "Base hitsound difficulty");
        widgets::help(
            ui,
            match options.write_mode {
                WriteMode::Direct => {
                    "An extra difficulty in the mapset with a circle on every base-layer hit. Copy it into your difficulties with Mapping Tools' Hitsound Copier."
                }
                WriteMode::HitsoundOnly => {
                    "An extra difficulty with a circle on every base-layer hit, in the export folder. Copy it into your difficulties with Mapping Tools' Hitsound Copier."
                }
            },
        );
    });
    // D-69: the file type of generated samples, as in Mapping Tools' Hitsound Studio.
    widgets::section(ui, "Sample files", |ui| {
        format_field(
            ui,
            "Single",
            "single-format",
            &mut options.single_format,
            true,
        );
        format_field(
            ui,
            "Mixed",
            "mixed-format",
            &mut options.mixed_format,
            false,
        );
        widgets::help(
            ui,
            "Single: a sample made from one source. Same as source copies it as it is when it is a WAV or OGG at full volume. Mixed: a sample that combines several sources. Silent samples are always the 44-byte WAV.",
        );
        if options.single_format == SampleFormat::Ogg || options.mixed_format == SampleFormat::Ogg {
            widgets::help(
                ui,
                "OGG files are checked after encoding: the decoded audio must keep its length and timing.",
            );
        }
    });
    widgets::section(ui, "Sample indices", |ui| {
        widgets::field(ui, "Start at", |ui| {
            ui.add(
                egui::DragValue::new(&mut options.first_custom_index)
                    .range(1..=i32::MAX as u32)
                    .update_while_editing(false),
            );
        });
        widgets::field(ui, "Ticks start at", |ui| {
            ui.add(
                egui::DragValue::new(&mut options.body_sample_first_index)
                    .range(1..=i32::MAX as u32)
                    .update_while_editing(false),
            );
        });
        widgets::help(
            ui,
            "The first custom sample index used for hits, and the first one used for custom slider ticks.",
        );
        ui.checkbox(&mut options.allow_growth, "Allow new indices when needed");
    });
    widgets::section(ui, "Mixing", |ui| {
        widgets::field(ui, "Group within", |ui| {
            ui.add(
                egui::DragValue::new(&mut options.grouping_leniency_ms)
                    .range(0.0..=250.0)
                    .max_decimals(2)
                    .suffix(" ms")
                    .update_while_editing(false),
            );
        });
        widgets::help(
            ui,
            "Hits closer together than this are mixed into one sample.",
        );
        widgets::field(ui, "Lone additions", |ui| {
            ui.label(addition_only_label(options.addition_only_normal));
        });
        widgets::help(
            ui,
            "The hitnormal osu! plays under a whistle, finish or clap with no hitnormal channel. Set it in File, Project settings.",
        );
    });
    widgets::section(ui, "Checks and cleanup", |ui| {
        ui.checkbox(
            &mut options.auto_fix,
            "Fix short samples and leading silence",
        );
        ui.checkbox(
            &mut options.mute_slider_slides,
            "Blank slider slides at custom indices",
        );
        ui.checkbox(
            &mut options.mute_slider_ticks,
            "Blank slider ticks at custom indices",
        );
        ui.checkbox(
            &mut options.remove_redundant_greenlines,
            "Remove greenlines that change nothing",
        )
        .on_hover_text("Includes the mapper's own. SV, kiai and every sound stay the same.");
        if options.write_mode == WriteMode::Direct {
            ui.checkbox(
                &mut options.take_over_samples,
                "Replace existing mapset samples (backed up)",
            );
        }
    });
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The write button names what it will do (§5.7).
fn write_label(mode: WriteMode, count: usize, base: bool) -> String {
    let diffs = match mode {
        WriteMode::Direct => format!(
            "Back up and write {}",
            plural(count, "difficulty", "difficulties")
        ),
        WriteMode::HitsoundOnly => format!(
            "Write {}",
            plural(count, "hitsound-only copy", "hitsound-only copies")
        ),
    };
    match (count, base) {
        (0, true) => "Write the base hitsound difficulty".into(),
        (_, true) => format!("{diffs} and the base hitsound difficulty"),
        _ => diffs,
    }
}

/// How the hitnormal under a lone addition is described in Project settings and Export
/// (D-78, D-91).
pub fn addition_only_label(choice: AdditionOnlyNormal) -> &'static str {
    match choice {
        AdditionOnlyNormal::Difficulty => "Skin hitnormal of the difficulty's set",
        AdditionOnlyNormal::AdditionsSet => "Skin hitnormal of the additions' set",
        AdditionOnlyNormal::SkinSoft => "Skin soft-hitnormal",
        AdditionOnlyNormal::SkinNormal => "Skin normal-hitnormal",
        AdditionOnlyNormal::SkinDrum => "Skin drum-hitnormal",
        AdditionOnlyNormal::Silent => "Silent",
    }
}

fn format_label(format: SampleFormat) -> &'static str {
    match format {
        SampleFormat::Source => "Same as source",
        SampleFormat::Wav16 => "WAV 16-bit",
        SampleFormat::WavFloat => "WAV 32-bit float",
        SampleFormat::Ogg => "OGG Vorbis",
    }
}

fn format_field(ui: &mut Ui, label: &str, id: &str, value: &mut SampleFormat, single: bool) {
    widgets::field(ui, label, |ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(format_label(*value))
            .show_ui(ui, |ui| {
                // A mix has no single source to follow.
                let choices: &[SampleFormat] = if single {
                    &[
                        SampleFormat::Source,
                        SampleFormat::Wav16,
                        SampleFormat::WavFloat,
                        SampleFormat::Ogg,
                    ]
                } else {
                    &[
                        SampleFormat::Wav16,
                        SampleFormat::WavFloat,
                        SampleFormat::Ogg,
                    ]
                };
                for &choice in choices {
                    ui.selectable_value(value, choice, format_label(choice));
                }
            });
    });
}

fn show_diff(ui: &mut Ui, diff: &LineDiff) {
    ui.separator();
    ui.label(t::display(
        format!("Changed lines · {}", diff.label),
        t::TITLE_SIZE,
    ));
    if diff.rows.is_empty() {
        ui.label(t::muted("The difficulty has no changed lines."));
        return;
    }
    ScrollArea::both()
        .id_salt("changed-lines")
        .max_height(t::DIFF_VIEW_HEIGHT)
        .show_rows(ui, t::DIFF_LINE_HEIGHT, diff.rows.len(), |ui, range| {
            for line in &diff.rows[range] {
                let (sign, colour) = if line.added {
                    ('+', t::MATCHED)
                } else {
                    ('−', t::ERROR)
                };
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(t::meta(format!("{sign} {:>5}", line.number), colour));
                    ui.add(egui::Label::new(t::meta(&line.text, colour)).extend());
                });
            }
        });
}

fn done(ui: &mut Ui, completed: &CompletedExport) {
    let report = &completed.report;
    ui.label(t::display("Export complete", t::HEADING_SIZE));
    ui.label(t::muted(format!(
        "{} difficulties written  ·  {} samples written  ·  {} files removed",
        report.diffs_written, report.samples_written, report.files_removed
    )));
    ui.label(t::meta("Output folder", t::DIM));
    views::path_label(ui, &report.out_dir);
    if let Some(backup) = &report.backup_dir {
        ui.label(t::meta("Backup folder", t::DIM));
        views::path_label(ui, backup);
    }
    if let Some(warning) = &completed.warning {
        ui.colored_label(t::WARNING, warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_keeps_only_changed_lines_and_uses_their_source_numbers() {
        let diff = DiffExport {
            file: "hard.osu".into(),
            base: false,
            out_file: "hard.osu".into(),
            label: "Hard".into(),
            original: "osu file format v14\r\n[TimingPoints]\r\n0,500,4,1,0,80,1,0\r\n[HitObjects]\r\n".into(),
            updated: "osu file format v14\r\n[TimingPoints]\r\n0,500,4,1,0,90,1,0\r\n500,-100,4,1,2,90,0,0\r\n[HitObjects]\r\n".into(),
            changed: true,
            stats: DiffStats::default(),
            warnings: Vec::new(),
        };
        let change = LineDiff::new(&diff);
        let rows: Vec<_> = change
            .rows
            .iter()
            .map(|line| (line.added, line.number, line.text.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (false, 3, "0,500,4,1,0,80,1,0"),
                (true, 3, "0,500,4,1,0,90,1,0"),
                (true, 4, "500,-100,4,1,2,90,0,0"),
            ]
        );
    }

    #[test]
    fn pending_preview_is_discarded_when_export_options_change() {
        let mut page = ExportPage::new(ExportSettings::default(), Vec::new());
        let ctx = Context::default();
        assert!(matches!(page.build(&ctx, 0.0), Some(Job::BuildExport(_))));
        page.options.write_mode = WriteMode::HitsoundOnly;
        page.review(Box::new(ExportReview {
            mode: WriteMode::Direct,
            diffs: vec![Difficulty {
                file: "hard.osu".into(),
                label: "Hard".into(),
                base: false,
                stats: DiffStats::default(),
            }],
            samples: vec![],
            problems: vec![],
            warnings: vec![],
            stale: vec![],
            taken_over: vec![],
        }));
        assert!(page.review.is_none());
        assert!(!page.can_write());
        assert!(
            matches!(page.build(&ctx, 1.0), Some(Job::BuildExport(request)) if request.settings.write_mode == WriteMode::HitsoundOnly)
        );
    }

    #[test]
    fn write_button_names_the_base_hitsound_difficulty() {
        assert_eq!(
            write_label(WriteMode::Direct, 1, false),
            "Back up and write 1 difficulty"
        );
        assert_eq!(
            write_label(WriteMode::Direct, 3, true),
            "Back up and write 3 difficulties and the base hitsound difficulty"
        );
        assert_eq!(
            write_label(WriteMode::HitsoundOnly, 0, true),
            "Write the base hitsound difficulty"
        );
        assert_eq!(
            write_label(WriteMode::HitsoundOnly, 2, false),
            "Write 2 hitsound-only copies"
        );
    }

    #[test]
    fn leaving_a_difficulty_out_rebuilds_the_preview_with_it_excluded() {
        let diff = |file: &str| Included {
            file: file.into(),
            label: file.into(),
            exists: true,
            enabled: true,
        };
        let mut page = ExportPage::new(
            ExportSettings::default(),
            vec![diff("easy.osu"), diff("hard.osu")],
        );
        let ctx = Context::default();
        assert!(matches!(page.build(&ctx, 0.0), Some(Job::BuildExport(_))));
        assert!(page.build(&ctx, 0.0).is_none());
        page.included[1].enabled = false;
        assert!(!page.current());
        assert!(matches!(
            page.build(&ctx, 1.0),
            Some(Job::BuildExport(request))
                if request.included == [("easy.osu".to_owned(), true), ("hard.osu".to_owned(), false)]
        ));
    }
}
