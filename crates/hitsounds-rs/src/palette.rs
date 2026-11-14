//! The Ctrl+K command palette: every action the shell and editor offer, searchable by name.
//!
//! The palette only builds [`Intent`]s. It never reaches into the session, so it cannot do
//! anything the menus, buttons and shortcuts cannot (D-48: behaviour lives in `hs-session`).

use crate::{
    app::{Dialog, Intent},
    editor::{Act, Panel, Tool},
    keys::{self, Action},
    theme as t,
    views::Emit,
    worker::{Job, ProjectView},
};
use eframe::egui::{
    self, Align2, Button, Context, Id, Key, Modifiers, RichText, ScrollArea, TextEdit, vec2,
};
use egui_phosphor::regular as icons;
use hs_core::project::Monitor;
use hs_session::Transport;

const MAX_LIST_HEIGHT: f32 = 340.0;

pub struct Command {
    label: String,
    shortcut: &'static str,
    icon: &'static str,
    intent: Intent,
}

impl Command {
    fn new(
        label: impl Into<String>,
        shortcut: &'static str,
        icon: &'static str,
        intent: Intent,
    ) -> Self {
        Self {
            label: label.into(),
            shortcut,
            icon,
            intent,
        }
    }
}

#[derive(Default)]
pub struct Palette {
    query: String,
    selected: usize,
}

/// What the palette is allowed to offer right now.
pub struct Offer<'a> {
    pub project: Option<&'a ProjectView>,
    /// False while a job runs: only dialogs are offered, since the app ignores the rest.
    pub ready: bool,
    pub exporting: bool,
    pub playing: bool,
    /// A drum separation is running (D-72).
    pub separating: bool,
    /// The next interface scale up and down that fits, if any (D-68).
    pub larger: Option<f32>,
    pub smaller: Option<f32>,
    /// Listening levels, for the mute commands (D-77).
    pub levels: hs_session::Levels,
}

/// Every command that currently makes sense, in the order they show with an empty query.
pub fn commands(cx: &Offer<'_>) -> Vec<Command> {
    let mut list = Vec::new();
    let dialogs = |list: &mut Vec<Command>| {
        list.push(Command::new(
            "App settings",
            keys::label(Action::Settings),
            icons::GEAR,
            Intent::Show(Dialog::Settings),
        ));
        list.push(Command::new(
            "Gestures and shortcuts",
            keys::label(Action::Help),
            icons::QUESTION,
            Intent::Show(Dialog::Help),
        ));
        list.push(Command::new(
            "About hitsounds-rs",
            "",
            icons::INFO,
            Intent::Show(Dialog::About),
        ));
    };
    // D-68: offered whenever jobs can run, in or out of a project.
    let scales = |list: &mut Vec<Command>| {
        if let Some(scale) = cx.larger {
            list.push(Command::new(
                format!("Interface scale: larger ({})", t::scale_label(scale)),
                keys::label(Action::ScaleUp),
                icons::MAGNIFYING_GLASS_PLUS,
                Intent::Job(Job::UiScale(Some(scale))),
            ));
        }
        if let Some(scale) = cx.smaller {
            list.push(Command::new(
                format!("Interface scale: smaller ({})", t::scale_label(scale)),
                keys::label(Action::ScaleDown),
                icons::MAGNIFYING_GLASS_MINUS,
                Intent::Job(Job::UiScale(Some(scale))),
            ));
        }
        list.push(Command::new(
            "Interface scale: 100%",
            keys::label(Action::ScaleReset),
            icons::MAGNIFYING_GLASS,
            Intent::Job(Job::UiScale(None)),
        ));
    };
    if !cx.ready {
        dialogs(&mut list);
        return list;
    }
    let Some(project) = cx.project else {
        list.push(Command::new(
            "New project from mapset",
            keys::label(Action::NewProject),
            icons::FILE_PLUS,
            Intent::Job(Job::Create),
        ));
        list.push(Command::new(
            "Open project",
            keys::label(Action::OpenProject),
            icons::FOLDER_OPEN,
            Intent::Job(Job::Open),
        ));
        list.push(Command::new(
            "New project from mania hitsound diff",
            "",
            icons::FILE_ARROW_DOWN,
            Intent::Job(Job::ImportHitsounds),
        ));
        list.push(Command::new(
            "Choose skin folder",
            "",
            icons::FOLDER,
            Intent::Job(Job::ChooseSkin),
        ));
        list.push(Command::new(
            "Add sample library folder",
            "",
            icons::FOLDER_PLUS,
            Intent::Job(Job::AddLibrary),
        ));
        scales(&mut list);
        dialogs(&mut list);
        return list;
    };

    if cx.exporting {
        list.push(Command::new(
            "Back to the editor",
            "Esc",
            icons::ARROW_LEFT,
            Intent::CloseExport,
        ));
        list.push(Command::new(
            "Save project",
            keys::label(Action::Save),
            icons::FLOPPY_DISK,
            Intent::Job(Job::Save),
        ));
        dialogs(&mut list);
        return list;
    }

    let layer = project.layer.file.as_deref();
    list.push(Command::new(
        if cx.playing { "Pause" } else { "Play" },
        keys::label(Action::PlayPause),
        if cx.playing {
            icons::PAUSE
        } else {
            icons::PLAY
        },
        Intent::TogglePlayback,
    ));
    list.push(Command::new(
        "Go to start",
        keys::label(Action::GoToStart),
        icons::SKIP_BACK,
        Intent::Transport(Transport::Seek { ms: 0.0 }),
    ));
    let levels = cx.levels;
    list.push(Command::new(
        if levels.hits_muted {
            "Unmute all channels"
        } else {
            "Mute all channels"
        },
        "",
        icons::SPEAKER_SIMPLE_SLASH,
        Intent::Job(Job::Levels(hs_session::Levels {
            hits_muted: !levels.hits_muted,
            ..levels
        })),
    ));
    list.push(Command::new(
        if levels.music_muted {
            "Unmute music"
        } else {
            "Mute music"
        },
        "",
        icons::SPEAKER_SIMPLE_SLASH,
        Intent::Job(Job::Levels(hs_session::Levels {
            music_muted: !levels.music_muted,
            ..levels
        })),
    ));
    if project.stem_ready {
        let (label, to) = match project.project.audio.monitor {
            Monitor::Original => ("Listen to the drum stem", Monitor::Stems),
            Monitor::Stems => ("Listen to the song", Monitor::Original),
        };
        list.push(Command::new(
            label,
            keys::label(Action::ToggleMonitor),
            icons::WAVEFORM,
            Intent::Job(Job::Monitor(to)),
        ));
    }
    if layer.is_some() {
        list.push(Command::new(
            "Next issue",
            keys::label(Action::NextIssue),
            icons::ARROW_DOWN,
            Intent::StepIssue { backwards: false },
        ));
        list.push(Command::new(
            "Previous issue",
            keys::label(Action::PreviousIssue),
            icons::ARROW_UP,
            Intent::StepIssue { backwards: true },
        ));
        list.push(Command::new(
            "Edit the base layer",
            "",
            icons::STACK,
            Intent::Job(Job::SelectLayer(None)),
        ));
    }
    for diff in &project.project.diffs {
        if layer == Some(diff.file.as_str()) || !project.available_diffs.contains(&diff.file) {
            continue;
        }
        list.push(Command::new(
            format!("Edit difficulty: {}", diff.label),
            "",
            icons::STACK,
            Intent::Job(Job::SelectLayer(Some(diff.file.clone()))),
        ));
    }
    for channel in &project.project.channels {
        list.push(Command::new(
            format!("Go to channel: {}", channel.name),
            "",
            icons::MUSIC_NOTES,
            Intent::SelectChannel(channel.id),
        ));
    }
    list.push(Command::new(
        "Add channel",
        "",
        icons::PLUS,
        Intent::Job(Job::AddChannel),
    ));
    list.push(Command::new(
        "Review and export",
        keys::label(Action::Export),
        icons::EXPORT,
        Intent::OpenExport,
    ));
    list.push(Command::new(
        "Save project",
        keys::label(Action::Save),
        icons::FLOPPY_DISK,
        Intent::Job(Job::Save),
    ));
    if project.can_undo {
        list.push(Command::new(
            "Undo",
            keys::label(Action::Undo),
            icons::ARROW_COUNTER_CLOCKWISE,
            Intent::Job(Job::History { redo: false }),
        ));
    }
    if project.can_redo {
        list.push(Command::new(
            "Redo",
            keys::label(Action::Redo),
            icons::ARROW_CLOCKWISE,
            Intent::Job(Job::History { redo: true }),
        ));
    }
    for tool in Tool::ALL {
        list.push(Command::new(
            format!("Tool: {}", tool.label()),
            tool.shortcut(),
            icons::CURSOR,
            Intent::Editor(Act::Tool(tool)),
        ));
    }
    for panel in Panel::ALL {
        list.push(Command::new(
            format!("Show or hide: {}", panel.label()),
            panel.shortcut(),
            panel.icon(),
            Intent::Editor(Act::Toggle(panel)),
        ));
    }
    list.push(Command::new(
        "Show or hide the waveform",
        keys::label(Action::ToggleWaveform),
        icons::WAVEFORM,
        Intent::Editor(Act::Waveform),
    ));
    for (label, act) in [
        ("Zoom in", Act::ZoomIn),
        ("Zoom out", Act::ZoomOut),
        ("Fit the whole song", Act::Fit),
        ("Follow the playhead on or off", Act::Follow),
        ("Select all hits", Act::SelectAll),
        ("Copy selected hits", Act::Copy),
        ("Cut selected hits", Act::Cut),
        ("Paste hits at the playhead", Act::Paste),
    ] {
        list.push(Command::new(
            label,
            "",
            icons::MAGNIFYING_GLASS,
            Intent::Editor(act),
        ));
    }
    list.push(Command::new(
        "Project settings",
        "",
        icons::SLIDERS,
        Intent::Show(Dialog::Project),
    ));
    list.push(Command::new(
        "Refresh from the mapset",
        "",
        icons::ARROWS_CLOCKWISE,
        Intent::Job(Job::Refresh { automatic: false }),
    ));
    list.push(Command::new(
        "Change mapset",
        "",
        icons::FOLDER_OPEN,
        Intent::Job(Job::ChangeMapset),
    ));
    if cx.separating {
        list.push(Command::new(
            "Cancel drum separation",
            "",
            icons::STOP,
            Intent::CancelSeparation,
        ));
    } else {
        list.push(Command::new(
            if project.project.audio.stem_origin.is_some() {
                "Separate drum stem again"
            } else {
                "Separate drum stem from the song"
            },
            "",
            icons::WAVEFORM,
            Intent::Job(Job::Separate),
        ));
    }
    list.push(Command::new(
        "Import drum stem",
        "",
        icons::WAVEFORM,
        Intent::Job(Job::ImportStem),
    ));
    if project.stem_ready {
        list.push(Command::new(
            "Remove drum stem",
            "",
            icons::TRASH,
            Intent::Job(Job::RemoveStem),
        ));
    }
    if !project.warnings.is_empty() {
        list.push(Command::new(
            "Show project warnings",
            "",
            icons::WARNING,
            Intent::Show(Dialog::Warnings),
        ));
    }
    list.push(Command::new(
        "Close project",
        keys::label(Action::CloseProject),
        icons::X,
        Intent::Job(Job::Close),
    ));
    list.push(Command::new(
        "New project from mapset",
        keys::label(Action::NewProject),
        icons::FILE_PLUS,
        Intent::Job(Job::Create),
    ));
    list.push(Command::new(
        "New project from mania hitsound diff",
        "",
        icons::FILE_ARROW_DOWN,
        Intent::Job(Job::ImportHitsounds),
    ));
    list.push(Command::new(
        "Open project",
        keys::label(Action::OpenProject),
        icons::FOLDER_OPEN,
        Intent::Job(Job::Open),
    ));
    scales(&mut list);
    dialogs(&mut list);
    list
}

/// Indices of the commands matching `query`, best first. Every word must appear in the
/// label; matches at the start of the label or of a word rank higher, and ties keep the
/// list's own order so the result is predictable.
pub fn rank(labels: &[&str], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut scored: Vec<(usize, usize)> = labels
        .iter()
        .enumerate()
        .filter_map(|(i, label)| {
            let label = label.to_lowercase();
            let mut score = 0;
            for word in &words {
                let at = label.find(word.as_str())?;
                let boundary = at == 0 || !label[..at].chars().next_back()?.is_alphanumeric();
                score += if at == 0 {
                    0
                } else if boundary {
                    1
                } else {
                    3
                };
            }
            Some((score, i))
        })
        .collect();
    scored.sort_by_key(|&(score, i)| (score, i));
    scored.into_iter().map(|(_, i)| i).collect()
}

impl Palette {
    pub fn show(&mut self, ctx: &Context, cx: &Offer<'_>, reduced: bool, emit: Emit<'_>) {
        let mut list = commands(cx);
        let labels: Vec<&str> = list.iter().map(|c| c.label.as_str()).collect();
        let matches = rank(&labels, &self.query);
        self.selected = self.selected.min(matches.len().saturating_sub(1));

        // Arrows move the selection. Take them before the text field sees them, since a
        // single-line field would otherwise use them to move its caret.
        let mut moved = false;
        let activate = ctx.input_mut(|i| {
            if !matches.is_empty() {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    self.selected = (self.selected + 1) % matches.len();
                    moved = true;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    self.selected = (self.selected + matches.len() - 1) % matches.len();
                    moved = true;
                }
            }
            i.consume_key(Modifiers::NONE, Key::Enter)
        });

        let id = Id::new("command-palette");
        let width = (ctx.content_rect().width() - t::PAGE_MARGIN * 2.0).min(t::MODAL_WIDTH);
        let mut chosen = activate
            .then(|| matches.get(self.selected).copied())
            .flatten();
        let modal = egui::Modal::new(id)
            .area(
                egui::Modal::default_area(id)
                    .anchor(Align2::CENTER_TOP, vec2(0.0, t::SECTION * 3.0))
                    .default_size(vec2(width, 0.0)),
            )
            .frame(t::overlay())
            .backdrop_color(t::SCRIM)
            .show(ctx, |ui| {
                ui.set_width(width);
                let before = self.query.clone();
                let field = ui.add(
                    TextEdit::singleline(&mut self.query)
                        .hint_text("Type a command")
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Body),
                );
                field.request_focus();
                if self.query != before {
                    self.selected = 0;
                }
                ui.add_space(t::GAP);
                if matches.is_empty() {
                    ui.label(t::muted("No command matches."));
                }
                ScrollArea::vertical()
                    .max_height(MAX_LIST_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (row, &index) in matches.iter().enumerate() {
                            let command = &list[index];
                            let selected = row == self.selected;
                            let button = Button::new((
                                RichText::new(command.icon).color(t::MUTED),
                                RichText::new(&command.label).color(t::TEXT),
                            ))
                            .selected(selected)
                            .right_text(t::meta(command.shortcut, t::DIM))
                            .min_size(vec2(ui.available_width(), t::CONTROL_HEIGHT));
                            let response = ui.add(button);
                            if selected && moved {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                chosen = Some(index);
                            }
                        }
                    });
            });
        let _ = reduced; // The palette appears instantly, so there is no motion to reduce.
        if modal.should_close() {
            emit(Intent::DismissDialog);
        } else if let Some(index) = chosen {
            emit(Intent::DismissDialog);
            emit(list.swap_remove(index).intent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_must_match_and_prefixes_rank_first() {
        let labels = [
            "Save project",
            "Go to start",
            "Show project warnings",
            "Open project",
        ];
        assert_eq!(
            rank(&labels, "pro"),
            vec![0, 2, 3],
            "word starts outrank the rest"
        );
        assert_eq!(rank(&labels, "open pro"), vec![3]);
        assert_eq!(rank(&labels, "SAVE"), vec![0]);
        assert!(rank(&labels, "zzz").is_empty());
        assert_eq!(
            rank(&labels, "  "),
            vec![0, 1, 2, 3],
            "an empty query lists everything"
        );
    }

    #[test]
    fn label_start_beats_word_start_beats_mid_word() {
        let labels = ["Re-open project", "Open project", "Reopen project"];
        assert_eq!(rank(&labels, "open"), vec![1, 0, 2]);
    }

    #[test]
    fn without_a_project_only_start_and_dialog_commands_show() {
        let cx = Offer {
            project: None,
            ready: true,
            exporting: false,
            playing: false,
            separating: false,
            larger: Some(1.1),
            smaller: None,
            levels: hs_session::Levels::default(),
        };
        let labels: Vec<_> = commands(&cx).into_iter().map(|c| c.label).collect();
        assert!(labels.contains(&"New project from mapset".to_owned()));
        assert!(
            !labels
                .iter()
                .any(|l| l.contains("Save") || l.contains("export"))
        );
    }

    #[test]
    fn a_running_job_leaves_only_dialogs() {
        let cx = Offer {
            project: None,
            ready: false,
            exporting: false,
            playing: false,
            separating: false,
            larger: Some(1.1),
            smaller: None,
            levels: hs_session::Levels::default(),
        };
        let list = commands(&cx);
        assert!(list.iter().all(|c| matches!(c.intent, Intent::Show(_))));
        assert!(!list.is_empty());
    }

    fn run(keys: &[(Key, Modifiers)], text: &str) -> Vec<String> {
        use egui_kittest::Harness;
        let mut state = (Palette::default(), Vec::<String>::new());
        let mut harness = Harness::builder()
            .with_size(vec2(920.0, 560.0))
            .build_state(
                |ctx, (palette, got): &mut (Palette, Vec<String>)| {
                    theme_once(ctx);
                    let offer = Offer {
                        project: None,
                        ready: true,
                        exporting: false,
                        playing: false,
                        separating: false,
                        larger: Some(1.1),
                        smaller: None,
                        levels: hs_session::Levels::default(),
                    };
                    let mut intents = Vec::new();
                    palette.show(ctx, &offer, true, &mut |intent| intents.push(intent));
                    for intent in intents {
                        got.push(match intent {
                            Intent::DismissDialog => "dismiss".to_owned(),
                            Intent::Show(Dialog::Settings) => "settings".to_owned(),
                            Intent::Show(Dialog::Help) => "help".to_owned(),
                            Intent::Job(Job::Create) => "create".to_owned(),
                            Intent::Job(Job::Open) => "open".to_owned(),
                            _ => "other".to_owned(),
                        });
                    }
                },
                std::mem::take(&mut state),
            );
        harness.step();
        if !text.is_empty() {
            harness.event(egui::Event::Text(text.to_owned()));
            harness.step();
        }
        for &(key, modifiers) in keys {
            harness.event_modifiers(
                egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
                modifiers,
            );
            harness.step();
        }
        harness.state().1.clone()
    }

    fn theme_once(ctx: &Context) {
        thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
        if !DONE.with(|d| d.replace(true)) {
            t::install(ctx);
        }
    }

    #[test]
    fn typing_then_enter_runs_the_best_match() {
        assert_eq!(
            run(&[(Key::Enter, Modifiers::NONE)], "sett"),
            ["dismiss", "settings"]
        );
    }

    #[test]
    fn arrows_move_the_selection_and_wrap() {
        // Empty query lists: New, Open, ... so one Down then Enter runs "Open project".
        assert_eq!(
            run(
                &[
                    (Key::ArrowDown, Modifiers::NONE),
                    (Key::Enter, Modifiers::NONE)
                ],
                ""
            ),
            ["dismiss", "open"]
        );
        // Up from the first row wraps to the last command (About), which is a dialog.
        assert_eq!(
            run(
                &[
                    (Key::ArrowUp, Modifiers::NONE),
                    (Key::Enter, Modifiers::NONE)
                ],
                ""
            ),
            ["dismiss", "other"]
        );
    }

    #[test]
    fn enter_with_no_match_does_nothing() {
        assert!(run(&[(Key::Enter, Modifiers::NONE)], "zzzzzz").is_empty());
    }
}
