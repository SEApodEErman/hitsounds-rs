//! The right dock: Channel settings and the per-difficulty review (D-61).
//!
//! Every field applies on change as one undo step (D-43). Rows are a label on the left
//! and a control on the right, grouped under plain section titles.

use super::{
    canvas::{Timeline, edit, seek},
    grid::timecode,
    model::{Issue, IssueKind},
    paint::issue_color,
    rail::{SETS, sample_name, set_name, slot_file, slot_matrix},
};
use crate::{
    app::Intent,
    theme as t,
    views::Emit,
    widgets::{self, Segment},
    worker::{Job, ProjectView},
};
use eframe::egui::{
    self, Align, Button, Color32, ComboBox, DragValue, Key, Label, Layout, Response, RichText,
    ScrollArea, Sense, Slider, StrokeKind, TextEdit, Ui, vec2,
};
use egui_phosphor::regular as icons;
use hs_core::{
    Channel, ChannelId, ChannelProps, DiffProfile, DiffProps, Edit, HitKind, Ms, Override,
    SampleSource, TickPolicy, TimeRange,
};
use osu_format::SampleSet;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Channel,
    Review,
}

#[derive(Default)]
pub struct Inspector {
    pub tab: Tab,
    filter: Option<IssueKind>,
}

const SWATCHES: [&str; 10] = [
    "#ED8B8F", "#F2BD76", "#E8DC7A", "#79D6A6", "#9ED4D8", "#7FB9F8", "#A6A8F5", "#C4A1F2",
    "#E7A8D5", "#B8B8BE",
];

impl Inspector {
    /// Draws the dock. Returns true when its close button was pressed.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        view: &ProjectView,
        timeline: &mut Timeline,
        ready: bool,
        emit: Emit<'_>,
    ) -> bool {
        let mut close = false;
        ui.horizontal(|ui| {
            let picked = widgets::segmented(
                ui,
                self.tab,
                &[
                    Segment {
                        value: Tab::Channel,
                        icon: icons::SLIDERS_HORIZONTAL,
                        label: "Channel",
                        hint: "Channel settings: sample, osu! hitsound, volume and colour",
                        shortcut: "F6",
                        enabled: true,
                    },
                    Segment {
                        value: Tab::Review,
                        icon: icons::LIST_CHECKS,
                        label: "Difficulty",
                        hint: "Difficulty review: issues, rules and changes for one difficulty",
                        shortcut: "F7",
                        enabled: true,
                    },
                ],
                true,
            );
            if let Some(tab) = picked {
                self.tab = tab;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = t::icon_button(ui, icons::X, "Hide this panel").clicked();
            });
        });
        ui.add_space(t::SMALL);
        ScrollArea::vertical()
            .id_salt("inspector-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.tab {
                Tab::Channel => {
                    if let Some(channel) = timeline.channel.and_then(|id| view.project.channel(id))
                    {
                        ui.add_enabled_ui(ready, |ui| channel_settings(ui, channel, emit));
                    } else {
                        empty(
                            ui,
                            "No channel selected",
                            "A channel is one sound: a sample and the osu! hitsound it becomes. Add one, then click steps on its row to place hits.",
                        );
                        if ui
                            .add_enabled(ready, Button::new((t::icon(icons::PLUS), "Add channel")))
                            .clicked()
                        {
                            emit(Intent::Job(Job::AddChannel));
                        }
                    }
                }
                Tab::Review => {
                    if let Some(diff) = view.layer.file.as_deref().and_then(|f| view.project.diff(f))
                    {
                        self.review(ui, view, diff, timeline, ready, emit);
                    } else {
                        pick_difficulty(ui, view, ready, emit);
                    }
                }
            });
        close
    }

    fn review(
        &mut self,
        ui: &mut Ui,
        view: &ProjectView,
        profile: &DiffProfile,
        timeline: &mut Timeline,
        ready: bool,
        emit: Emit<'_>,
    ) {
        ui.horizontal(|ui| {
            ui.add(Label::new(t::display(&profile.label, 16.0).color(t::DIFFICULTY)).truncate());
        });
        ui.horizontal_wrapped(|ui| {
            if profile.enabled {
                widgets::tag(ui, "Included in export", t::MATCHED);
            } else {
                widgets::tag(ui, "Left out of export", t::DIM);
            }
        });
        if !view.available_diffs.contains(&profile.file) {
            ui.colored_label(
                t::WARNING,
                "This difficulty's .osu file is missing. Restore it, then use Tools, Refresh from mapset.",
            );
        }

        widgets::section(ui, "How the base layer lands here", |ui| {
            let (mut matched, mut fallback, mut muted, mut empty) = (0, 0, 0, 0);
            let mut ticks = 0;
            if let Some(objects) = &view.layer.objects {
                for kind in &objects.edges {
                    match kind {
                        HitKind::Matched => matched += 1,
                        HitKind::Fallback => fallback += 1,
                        HitKind::Muted => muted += 1,
                        HitKind::Empty => empty += 1,
                    }
                }
                ticks = objects.ticks.iter().filter(|&&custom| custom).count();
            }
            let orphaned = view.layer.issue_groups[2].len();
            ui.columns(2, |columns| {
                for (i, (mark, label, count, colour, what)) in [
                    (
                        icons::CHECK,
                        "Matched",
                        matched,
                        t::MATCHED,
                        "Objects with a hit from the base layer or this difficulty",
                    ),
                    (
                        icons::X,
                        "Empty",
                        empty,
                        t::EMPTY_HIT,
                        "Objects with no hitsound. They play the default hitnormal",
                    ),
                    (
                        icons::CARET_RIGHT,
                        "Fallback",
                        fallback,
                        t::FALLBACK,
                        "Objects with no hit that are filled by the fallback channel",
                    ),
                    (
                        icons::DIAMOND,
                        "Orphaned",
                        orphaned,
                        t::ORPHAN_HIT,
                        "Hits with no object under them in this difficulty. They are not exported",
                    ),
                    (
                        icons::MINUS,
                        "Muted ends",
                        muted,
                        t::MUTED_HIT,
                        "Slider ends silenced by the slider end rule below",
                    ),
                    (
                        icons::DOT,
                        "Custom ticks",
                        ticks,
                        t::TICK_HIT,
                        "Slider ticks that get their own sample",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let ui = &mut columns[i % 2];
                    let response = ui
                        .horizontal(|ui| {
                            ui.label(t::icon(mark).size(12.0).color(colour));
                            ui.label(RichText::new(label).color(t::MUTED));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(t::meta(
                                    count.to_string(),
                                    if count > 0 { t::TEXT } else { t::DIM },
                                ));
                            });
                        })
                        .response;
                    widgets::describe(response, what, "");
                }
            });
        });

        let issues = view.layer.issues.len();
        widgets::section(ui, &format!("Issues  {issues}"), |ui| {
            if issues == 0 {
                widgets::help(ui, "Nothing left to review in this difficulty.");
                return;
            }
            widgets::help(
                ui,
                "Objects and hits that still need a decision. Click one to jump to it.",
            );
            ui.horizontal(|ui| {
                for (glyph, label, shortcut, backwards) in [
                    (icons::ARROW_UP, "Previous", "Shift+N", true),
                    (icons::ARROW_DOWN, "Next", "N", false),
                ] {
                    let response =
                        ui.add_enabled(ready, Button::new((t::icon(glyph).size(13.0), label)));
                    if widgets::describe(
                        response,
                        if backwards {
                            "Jump to the previous issue"
                        } else {
                            "Jump to the next issue"
                        },
                        shortcut,
                    )
                    .clicked()
                    {
                        emit(Intent::StepIssue { backwards });
                    }
                }
                ComboBox::from_id_salt("issue-filter")
                    .selected_text(self.filter.map_or("All kinds", IssueKind::label))
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.filter, None, "All kinds");
                        for kind in IssueKind::ALL {
                            ui.selectable_value(&mut self.filter, Some(kind), kind.label());
                        }
                    });
            });
            let shown: Vec<Issue> = view
                .layer
                .issues
                .iter()
                .copied()
                .filter(|issue| self.filter.is_none_or(|kind| kind == issue.kind))
                .collect();
            if shown.is_empty() {
                widgets::help(ui, "No issues of this kind.");
                return;
            }
            egui::Frame::new()
                .fill(t::INSET)
                .stroke(t::stroke(t::EDGE))
                .corner_radius(t::CONTROL_RADIUS)
                .inner_margin(2)
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .id_salt("issue-list")
                        .max_height(t::DIFF_VIEW_HEIGHT)
                        .auto_shrink([false, true])
                        .show_rows(ui, 22.0, shown.len(), |ui, range| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for issue in &shown[range] {
                                let colour = issue_color(issue.kind);
                                let (bar, beat) = view.layer.grid.bar_beat(f64::from(issue.time));
                                // The current issue is tinted in the difficulty colour, so the
                                // list shows where N and Shift+N have taken you.
                                let current = timeline.issue == Some(*issue);
                                let response = ui.add_enabled(
                                    ready,
                                    Button::new((
                                        t::icon(issue_mark(*issue)).size(12.0).color(colour),
                                        t::meta(timecode(f64::from(issue.time)), t::TEXT),
                                        RichText::new(issue.kind.label())
                                            .size(12.0)
                                            .color(t::MUTED),
                                    ))
                                    .right_text(t::meta(format!("{bar}:{beat}"), t::DIM))
                                    .fill(if current {
                                        t::DIFFICULTY.gamma_multiply(0.2)
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .frame_when_inactive(current)
                                    .min_size(vec2(ui.available_width(), 22.0)),
                                );
                                if response.clicked() {
                                    timeline.select_issue(view, *issue, emit);
                                }
                            }
                        });
                });
        });

        if let Some(target) = timeline.object
            && let Some(objects) = &view.layer.objects
            && let Some(time) = objects.time(target)
        {
            let kind = match target {
                hs_core::Target::Edge(i) => super::edge_name(objects.timeline.edges[i].kind),
                hs_core::Target::Tick(_) => "Slider tick",
            };
            ui.add_space(t::SMALL);
            ui.horizontal_wrapped(|ui| {
                ui.label(t::muted("Selected object"));
                ui.label(t::meta(
                    format!("{kind}  {}", timecode(f64::from(time))),
                    t::DIFFICULTY,
                ));
            });
            widgets::help(ui, "Right-click it in the Objects lane to add a hit there.");
        }

        ui.add_enabled_ui(ready, |ui| {
            widgets::section(ui, "Rules for this difficulty", |ui| {
                diff_settings(ui, view, profile, emit)
            });
            widgets::section(
                ui,
                &format!("Changes from the base layer  {}", profile.overrides.len()),
                |ui| {
                    if profile.overrides.is_empty() {
                        widgets::help(
                            ui,
                            "None yet. Place, move or delete hits while this difficulty is open and they are listed here.",
                        );
                    }
                    for (index, value) in profile.overrides.iter().enumerate() {
                        ui.push_id(index, |ui| {
                            let (label, time) = override_label(view, value);
                            ui.horizontal(|ui| {
                                let width = ui.available_width() - t::CONTROL_HEIGHT - t::GAP;
                                let jump = ui.add(
                                    Button::new(RichText::new(label).size(12.0))
                                        .frame_when_inactive(false)
                                        .truncate()
                                        .min_size(vec2(width.max(60.0), 22.0)),
                                );
                                if widgets::describe(jump, "Jump to this change", "").clicked() {
                                    timeline.jump(view, time);
                                    seek(emit, f64::from(time));
                                }
                                if t::icon_button(ui, icons::X, "Undo this change for the difficulty")
                                    .clicked()
                                {
                                    edit(
                                        emit,
                                        Edit::RemoveOverride {
                                            file: profile.file.clone(),
                                            index,
                                        },
                                    );
                                }
                            });
                        });
                    }
                },
            );
        });
    }
}

fn empty(ui: &mut Ui, title: &str, body: &str) {
    ui.add_space(t::GAP);
    ui.label(t::display(title, t::TITLE_SIZE));
    widgets::help(ui, body);
    ui.add_space(t::GAP);
}

/// The Difficulty tab on the base layer: explain the model and offer the way in.
fn pick_difficulty(ui: &mut Ui, view: &ProjectView, ready: bool, emit: Emit<'_>) {
    empty(
        ui,
        "You are on the base layer",
        "Hits on the base layer play in every difficulty, wherever that difficulty has an object. Open a difficulty to see how they land, fix what is missing and make changes for that difficulty only.",
    );
    if view.project.diffs.is_empty() {
        widgets::help(ui, "This mapset has no difficulties yet.");
    }
    for diff in &view.project.diffs {
        let exists = view.available_diffs.contains(&diff.file);
        ui.horizontal(|ui| {
            let response = ui.add_enabled(
                ready && exists,
                Button::new(RichText::new(&diff.label).color(t::DIFFICULTY))
                    .truncate()
                    .min_size(vec2(ui.available_width() - 76.0, t::CONTROL_HEIGHT)),
            );
            if response.clicked() {
                emit(Intent::Job(Job::SelectLayer(Some(diff.file.clone()))));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match view.layer.badges.get(&diff.file).copied() {
                    _ if !exists => widgets::tag(ui, "Missing", t::WARNING),
                    Some(0) => widgets::tag(ui, "Clear", t::MATCHED),
                    Some(count) => widgets::tag(ui, format!("{count} issues"), t::EMPTY_HIT),
                    None => widgets::tag(ui, "Not checked", t::DIM),
                };
            });
        });
    }
}

fn issue_mark(issue: Issue) -> &'static str {
    match issue.kind {
        IssueKind::Empty => icons::X,
        IssueKind::Fallback => icons::CARET_RIGHT,
        IssueKind::Orphaned => icons::DIAMOND,
        IssueKind::Tick => icons::DOT,
    }
}

fn channel_settings(ui: &mut Ui, channel: &Channel, emit: Emit<'_>) {
    ui.push_id(channel.id, |ui| {
        let original = ChannelProps::from(channel);
        let mut props = original.clone();
        let colour = t::channel_color(&channel.color);
        ui.horizontal(|ui| {
            let (swatch, _) = ui.allocate_exact_size(vec2(6.0, 20.0), Sense::hover());
            ui.painter().rect_filled(swatch, 2, colour);
            ui.add(Label::new(t::display(&channel.name, 16.0)).truncate());
        });

        widgets::section(ui, "Sample", |ui| {
            widgets::field(ui, "Plays", |ui| {
                ui.add(Label::new(sample_name(&channel.sample)).truncate())
                    .on_hover_text(match &channel.sample {
                        SampleSource::SkinDefault => "The sample from your skin folder".to_owned(),
                        SampleSource::File { path } => path.display().to_string(),
                    });
            });
            ui.horizontal_wrapped(|ui| {
                let preview = ui.button((t::icon(icons::PLAY).size(13.0), "Preview"));
                if widgets::describe(preview, "Play this channel's sample once", "").clicked() {
                    emit(Intent::Job(Job::AuditionChannel(channel.id)));
                }
                let browse = ui.button("Choose file…");
                if widgets::describe(
                    browse,
                    "Pick an audio file. You can also drag one from the browser onto the channel",
                    "",
                )
                .clicked()
                {
                    emit(Intent::Job(Job::UseSample {
                        channel: Some(channel.id),
                        path: None,
                    }));
                }
                if props.sample != SampleSource::SkinDefault {
                    let skin = ui.button("Use skin sample");
                    if widgets::describe(
                        skin,
                        "Drop the custom file and play the skin's sample for the slot below",
                        "",
                    )
                    .clicked()
                    {
                        props.sample = SampleSource::SkinDefault;
                    }
                }
            });
        });

        widgets::section(ui, "Hitsound in osu!", |ui| {
            slot_matrix(ui, channel, emit);
            ui.add_space(2.0);
            widgets::help(
                ui,
                &format!(
                    "Exports as {}. Rows are sample sets, columns are sounds.",
                    slot_file(channel.sample_set, channel.hitsound)
                ),
            );
        });

        widgets::section(ui, "Level", |ui| {
            widgets::field(ui, "Volume", |ui| {
                let percent = (props.volume * 100.0).round() as u32;
                if let Some(value) = committed(ui, "volume", percent, |ui, value| {
                    ui.spacing_mut().slider_width = (ui.available_width() - 62.0).max(60.0);
                    ui.add(Slider::new(value, 0..=100).suffix("%"))
                }) {
                    props.volume = value as f32 / 100.0;
                }
            });
            widgets::field(ui, "Listening", |ui| {
                let mut muted = channel.muted;
                let mut solo = channel.solo;
                let mute = ui.add(Button::selectable(muted, "Mute"));
                if mute.clicked() {
                    muted = !muted;
                }
                let solo_button = ui.add(Button::selectable(solo, "Solo"));
                if solo_button.clicked() {
                    solo = !solo;
                }
                if muted != channel.muted || solo != channel.solo {
                    edit(
                        emit,
                        Edit::SetMonitor {
                            id: channel.id,
                            muted,
                            solo,
                        },
                    );
                }
            });
            widgets::help(
                ui,
                "Mute and solo only change what you hear. A muted channel is still exported.",
            );
        });

        widgets::section(ui, "Appearance", |ui| {
            if let Some(name) = text_field(ui, "Name", &channel.name) {
                let name = name.trim();
                if !name.is_empty() {
                    props.name = name.into();
                }
            }
            widgets::field(ui, "Colour", |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for hex in SWATCHES {
                    let (rect, response) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                    let swatch = t::channel_color(hex);
                    ui.painter().rect_filled(rect, 3, swatch);
                    if props.color.eq_ignore_ascii_case(hex) {
                        ui.painter().rect_stroke(
                            rect.expand(1.5),
                            4,
                            t::stroke(t::TEXT),
                            StrokeKind::Outside,
                        );
                    } else if response.hovered() {
                        ui.painter().rect_stroke(
                            rect.expand(1.5),
                            4,
                            t::stroke(t::EDGE_STRONG),
                            StrokeKind::Outside,
                        );
                    }
                    if response.clicked() {
                        props.color = hex.into();
                    }
                }
                ui.add_space(3.0);
                custom_colour(ui, &mut props);
            });
        });

        if props != original {
            edit(
                emit,
                Edit::SetChannel {
                    id: channel.id,
                    props,
                },
            );
        }
        ui.add_space(t::GAP);
        let delete = ui.button((
            t::icon(icons::TRASH).size(13.0).color(t::ERROR),
            RichText::new("Delete channel").color(t::ERROR),
        ));
        if widgets::describe(
            delete,
            "Remove this channel and all of its hits. Undo brings it back",
            "",
        )
        .clicked()
        {
            edit(emit, Edit::RemoveChannel { id: channel.id });
        }
    });
}

fn custom_colour(ui: &mut Ui, props: &mut ChannelProps) {
    let id = ui.id().with("channel-colour");
    let original = t::channel_color(&props.color);
    let mut color = ui.data(|d| d.get_temp::<Color32>(id)).unwrap_or(original);
    let response = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut color,
        egui::color_picker::Alpha::Opaque,
    );
    widgets::describe(response, "Pick any colour", "");
    if ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.insert_temp(id, color));
    } else {
        ui.data_mut(|d| d.remove::<Color32>(id));
        if color != original {
            props.color = format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b());
        }
    }
}

fn diff_settings(ui: &mut Ui, view: &ProjectView, profile: &DiffProfile, emit: Emit<'_>) {
    ui.push_id(&profile.file, |ui| {
        let original = DiffProps::from(profile);
        let mut props = original.clone();

        widgets::field(ui, "Export", |ui| {
            ui.checkbox(&mut props.enabled, "Include this difficulty");
        });

        widgets::field(ui, "Match within", |ui| {
            if let Some(value) = committed(ui, "leniency", props.match_leniency_ms, |ui, value| {
                ui.add(
                    DragValue::new(value)
                        .range(0.0..=250.0)
                        .suffix(" ms")
                        .speed(0.1)
                        .update_while_editing(false),
                )
            }) {
                props.match_leniency_ms = value;
            }
        });
        widgets::help(
            ui,
            "How far a hit may sit from an object and still count as landing on it.",
        );

        widgets::field(ui, "Empty objects", |ui| {
            let fallback = props
                .fallback
                .and_then(|id| view.project.channel(id))
                .map_or("Project fallback", |c| c.name.as_str());
            ComboBox::from_id_salt("diff-fallback")
                .selected_text(fallback)
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut props.fallback, None, "Project fallback");
                    for channel in &view.project.channels {
                        ui.selectable_value(&mut props.fallback, Some(channel.id), &channel.name);
                    }
                });
        });
        widgets::help(
            ui,
            "The channel that fills objects no hit lands on. The project fallback is set in File, Project settings.",
        );

        widgets::field(ui, "Slider ticks", |ui| {
            let picked = widgets::segmented(
                ui,
                props.tick_policy,
                &[
                    Segment {
                        value: TickPolicy::Ignore,
                        icon: "",
                        label: "Leave",
                        hint: "Hits on slider ticks are ignored in this difficulty",
                        shortcut: "",
                        enabled: true,
                    },
                    Segment {
                        value: TickPolicy::Render,
                        icon: "",
                        label: "Custom sound",
                        hint: "Hits on slider ticks become custom slidertick samples",
                        shortcut: "",
                        enabled: true,
                    },
                ],
                true,
            );
            if let Some(policy) = picked {
                props.tick_policy = policy;
            }
        });

        widgets::field(ui, "Slider ends", |ui| {
            let mut mute = props.slider_end_mute.is_some();
            if ui.checkbox(&mut mute, "Mute ends without a hit").changed() {
                props.slider_end_mute = mute.then(Default::default);
            }
        });
        if let Some(rule) = &mut props.slider_end_mute {
            egui::Frame::new()
                .fill(t::STAGE)
                .stroke(t::stroke(t::EDGE))
                .corner_radius(t::CONTROL_RADIUS)
                .inner_margin(t::GAP)
                .show(ui, |ui| {
                    widgets::help(
                        ui,
                        "A slider end is muted when it has no hit and sits on one of the chosen snaps.",
                    );
                    widgets::field(ui, "Shortest slider", |ui| {
                        if let Some(value) =
                            committed(ui, "min-length", rule.min_length_beats, |ui, value| {
                                ui.add(
                                    DragValue::new(value)
                                        .range(0.0..=256.0)
                                        .speed(0.1)
                                        .suffix(" beats")
                                        .update_while_editing(false),
                                )
                            })
                        {
                            rule.min_length_beats = value;
                        }
                    });
                    widgets::field(ui, "Spinners", |ui| {
                        ui.checkbox(&mut rule.spinner_ends, "Mute spinner ends too");
                    });
                    let priority = rule
                        .divisors
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    if let Some(text) = text_field(ui, "Snaps to test", &priority) {
                        let parsed: Option<Vec<u32>> = text
                            .split(',')
                            .map(|part| part.trim().parse::<u32>().ok().filter(|d| *d > 0))
                            .collect();
                        if let Some(divisors) = parsed {
                            ui.data_mut(|d| d.remove::<bool>(ui.id().with("bad-divisors")));
                            let mut seen = std::collections::BTreeSet::new();
                            rule.divisors = divisors
                                .into_iter()
                                .filter(|divisor| seen.insert(*divisor))
                                .collect();
                        } else {
                            ui.data_mut(|d| d.insert_temp(ui.id().with("bad-divisors"), true));
                        }
                    }
                    if ui.data(|d| d.get_temp::<bool>(ui.id().with("bad-divisors"))) == Some(true) {
                        ui.colored_label(
                            t::WARNING,
                            "Enter snap divisors separated by commas, such as 1, 2, 4.",
                        );
                    }
                    widgets::help(
                        ui,
                        "Snap divisors in the order they are tried. The first one an end sits on decides.",
                    );
                    widgets::field(ui, "Mute on", |ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for &divisor in &rule.divisors {
                            let selected = rule.muted.contains(&divisor);
                            if ui
                                .add(Button::selectable(selected, format!("1/{divisor}")))
                                .clicked()
                            {
                                if selected {
                                    rule.muted.retain(|&value| value != divisor);
                                } else {
                                    rule.muted.push(divisor);
                                }
                            }
                        }
                    });
                    widgets::field(ui, "Muted set", |ui| {
                        ComboBox::from_id_salt("muted-set")
                            .selected_text(set_name(rule.sample_set))
                            .show_ui(ui, |ui| {
                                for set in std::iter::once(SampleSet::Auto).chain(SETS) {
                                    ui.selectable_value(&mut rule.sample_set, set, set_name(set));
                                }
                            });
                    });
                    widgets::field(ui, "Muted index", |ui| {
                        let mut use_index = rule.index.is_some();
                        if ui.checkbox(&mut use_index, "Set").changed() {
                            rule.index = use_index.then_some(0);
                        }
                        if let Some(index) = rule.index
                            && let Some(value) = committed(ui, "muted-index", index, |ui, value| {
                                ui.add(
                                    DragValue::new(value)
                                        .range(0..=i32::MAX as u32)
                                        .update_while_editing(false),
                                )
                            })
                        {
                            rule.index = Some(value);
                        }
                    });
                    widgets::field(ui, "Muted volume", |ui| {
                        if let Some(value) =
                            committed(ui, "muted-volume", rule.volume, |ui, value| {
                                ui.add(
                                    DragValue::new(value)
                                        .range(5..=100)
                                        .suffix("%")
                                        .update_while_editing(false),
                                )
                            })
                        {
                            rule.volume = value;
                        }
                    });
                    let muted = view.layer.objects.as_ref().map_or(0, |o| {
                        o.edges
                            .iter()
                            .filter(|&&state| state == HitKind::Muted)
                            .count()
                    });
                    ui.label(t::meta(
                        format!("{muted} ends muted right now"),
                        t::MUTED_HIT,
                    ));
                });
        }
        if props != original {
            edit(
                emit,
                Edit::SetDiff {
                    file: profile.file.clone(),
                    props,
                },
            );
        }
    });
}

/// Runs a value control and reports the value once the user lets go, so one drag is
/// one edit. The value is then held until the worker's edit comes back.
pub fn committed<T: Copy + PartialEq + Send + Sync + 'static>(
    ui: &mut Ui,
    salt: &str,
    original: T,
    control: impl FnOnce(&mut Ui, &mut T) -> Response,
) -> Option<T> {
    let id = ui.id().with(("draft", salt));
    let now = ui.input(|input| input.time);
    let mut held = ui.data(|data| data.get_temp::<(T, f64)>(id));
    if let Some((value, until)) = held
        && until.is_finite()
        && (value == original || now > until)
    {
        held = None;
    }
    let mut value = held.map_or(original, |held| held.0);
    let before = value;
    let response = control(ui, &mut value);
    let mut commit = None;
    if response.dragged() {
        held = Some((value, f64::INFINITY));
    } else if response.drag_stopped() || value != before {
        if value != original {
            commit = Some(value);
            held = Some((value, now + 0.5));
        } else {
            held = None;
        }
    }
    if held.is_some_and(|held| held.1.is_finite()) {
        ui.ctx().request_repaint();
    }
    ui.data_mut(|data| match held {
        Some(held) => data.insert_temp(id, held),
        None => data.remove::<(T, f64)>(id),
    });
    commit
}

/// A labelled one-line text field that applies on Enter or when it loses focus.
pub fn text_field(ui: &mut Ui, label: &str, original: &str) -> Option<String> {
    let id = ui.id().with(label);
    let mut draft = ui
        .data(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| original.into());
    let response = widgets::field(ui, label, |ui| {
        ui.add(
            TextEdit::singleline(&mut draft)
                .id(id.with("input"))
                .desired_width(ui.available_width() - 8.0),
        )
    });
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, ui.is_enabled(), label)
    });
    if ui.input(|input| input.key_pressed(Key::Escape)) {
        ui.data_mut(|data| data.remove::<String>(id));
        None
    } else if response.lost_focus()
        || response.has_focus() && ui.input(|input| input.key_pressed(Key::Enter))
    {
        ui.data_mut(|data| data.remove::<String>(id));
        (draft != original).then_some(draft)
    } else {
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(id, draft));
        }
        None
    }
}

fn override_label(view: &ProjectView, value: &Override) -> (String, Ms) {
    let name = |id: ChannelId| {
        view.project
            .channel(id)
            .map_or("Removed channel", |channel| channel.name.as_str())
    };
    let at = |time: Ms| timecode(f64::from(time));
    let span = |range: &Option<TimeRange>| {
        range.map_or_else(
            || "whole difficulty".into(),
            |range| format!("{}–{}", at(range.start), at(range.end)),
        )
    };
    match value {
        Override::AddEvent { channel, time } => {
            (format!("Added {} at {}", name(*channel), at(*time)), *time)
        }
        Override::RemoveEvent { channel, time } => (
            format!("Removed {} at {}", name(*channel), at(*time)),
            *time,
        ),
        Override::MuteChannel { channel, range } => (
            format!("Muted {}, {}", name(*channel), span(range)),
            range.map_or(0, |range| range.start),
        ),
        Override::ReplaceChannel { from, to, range } => (
            format!("{} plays as {}, {}", name(*from), name(*to), span(range)),
            range.map_or(0, |range| range.start),
        ),
        Override::SliderBody {
            slider_time,
            sample_set,
            whistle,
        } => (
            format!(
                "Slider body {} {} at {}",
                set_name(*sample_set),
                if *whistle { "slide + whistle" } else { "slide" },
                at(*slider_time)
            ),
            *slider_time,
        ),
    }
}
