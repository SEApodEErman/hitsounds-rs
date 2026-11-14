//! The app shell's presentation state. Project behavior lives in hs-session (D-48).

use crate::{
    chrome,
    editor::{Act, Editor, Panel, Tool},
    export_view::{ExportPage, Included},
    keys::{self, Action},
    mapsets::{self, Picker, Purpose, Songs},
    palette::{Offer, Palette},
    theme as t, update, views,
    widgets::{self, Hint},
    worker::{Job, Outcome, Snapshot, Worker},
};
use eframe::egui::{
    self, Align, ColorImage, Context, Event, Frame, IconData, Key, Layout, Modifiers,
    TextureHandle, TextureOptions, ViewportCommand, vec2,
};
use hs_core::{ChannelId, project::Monitor};
use hs_session::{
    Clock, Transport,
    separation::{SeparatedStem, Separation, Stage},
};
use std::{
    sync::{Arc, atomic::Ordering, mpsc::TryRecvError},
    time::Duration,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Settings,
    Project,
    Help,
    About,
    Encore,
    Warnings,
    Confirm,
    Palette,
    /// The mapset picker (D-82), drawn by [`Picker::show`].
    Mapsets,
}

pub enum Decision {
    Save,
    Discard,
}

pub enum Intent {
    Job(Job),
    /// A choice in the mapset picker: close it and run the job, without asking again
    /// about unsaved changes (D-82).
    Pick(Job),
    Show(Dialog),
    Confirm(Decision),
    DismissDialog,
    DismissToast,
    TogglePlayback,
    Transport(Transport),
    /// Hear listening levels while a fader is dragged, without saving them (D-77).
    PreviewLevels(hs_session::Levels),
    SelectChannel(ChannelId),
    ShowDifficulty,
    StepIssue {
        backwards: bool,
    },
    /// A view or selection action of the workspace (tools, zoom, panels).
    Editor(Act),
    OpenExport,
    CloseExport,
    HideExportDiff,
    /// Stop the running drum separation after its current segment (D-72).
    CancelSeparation,
    /// Check GitHub Releases for a newer version now (D-84).
    CheckUpdates,
    /// Close (after the save prompt), install the downloaded update and start again.
    RestartToUpdate,
    /// Open the downloaded update's release page.
    UpdateNotes,
    Exit,
}

/// A button a toast can carry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastAction {
    /// Offer drum separation for a project that was just created (D-70).
    Separate,
    /// A downloaded update is ready to install (D-84).
    Update,
}

pub struct Toast {
    pub message: String,
    pub error: bool,
    pub since: f64,
    pub action: Option<ToastAction>,
}

/// A drum separation on its own thread (D-72) and what its progress card shows.
struct Running {
    job: Separation,
    /// When segments started, for the time estimate.
    segments_from: Option<f64>,
}

/// What the separation progress card shows.
pub struct SeparationCard {
    pub fraction: f32,
    pub detail: String,
    pub cancelling: bool,
}

enum Departure {
    Job(Job),
    Exit,
}

pub struct Studio {
    worker: Worker,
    state: Option<Snapshot>,
    busy: Option<&'static str>,
    offline: bool,
    logo: TextureHandle,
    startup_line: &'static str,
    custom_frame: bool,
    dialog: Option<Dialog>,
    palette: Palette,
    toast: Option<Toast>,
    pending_departure: Option<Departure>,
    after_save: Option<Departure>,
    exit_when_ready: bool,
    exit_approved: bool,
    clock: Clock,
    last_frame: f64,
    editor: Editor,
    export: Option<ExportPage>,
    settle_until: f64,
    refresh_at: Option<f64>,
    entered_at: f64,
    reduced: bool,
    /// Pixels per point the minimum window size was last set for (D-68).
    min_size_ppp: f32,
    separation: Option<Running>,
    /// A finished separation waiting for the worker to be free.
    separated: Option<Box<SeparatedStem>>,
    /// The Songs folder list, kept between pickers (D-82).
    songs: Songs,
    picker: Option<Picker>,
    /// The startup update check has been started, or skipped by the setting (D-84).
    update_started: bool,
    /// The "update ready" toast has been shown.
    update_announced: bool,
}

impl Studio {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        icon: &IconData,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let ctx = &cc.egui_ctx;
        t::install(ctx);
        let owner = chrome::Owner::new(cc)?;
        let custom_frame = chrome::install(owner, ctx.clone());
        let logo = ctx.load_texture(
            "hitsounds-rs mark",
            ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            TextureOptions::LINEAR,
        );
        let worker = Worker::start(ctx.clone(), owner)?;
        #[cfg(debug_assertions)]
        if std::env::var_os("HITSOUNDS_RS_QA_INPUT").is_some()
            || std::env::var_os("HITSOUNDS_RS_QA_CAPTURE").is_some()
        {
            // The QA hooks are polled once per frame, so keep frames coming while idle.
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(33));
                    ctx.request_repaint();
                }
            });
        }
        // D-56: a startup line with the launch parody's tone.
        let startup_lines = [
            "Preparing the stage for one.",
            "Checking the front row.",
            "Counting in the audience.",
        ];
        Ok(Self {
            worker,
            state: None,
            busy: Some("Loading settings"),
            offline: false,
            logo,
            startup_line: startup_lines[std::process::id() as usize % startup_lines.len()],
            custom_frame,
            dialog: None,
            palette: Palette::default(),
            toast: None,
            pending_departure: None,
            after_save: None,
            exit_when_ready: false,
            exit_approved: false,
            clock: Clock::default(),
            last_frame: 0.0,
            editor: Editor::new(cc.storage),
            export: None,
            settle_until: 0.0,
            refresh_at: None,
            entered_at: 0.0,
            reduced: false,
            min_size_ppp: 0.0,
            separation: None,
            separated: None,
            songs: Songs::default(),
            picker: None,
            update_started: false,
            update_announced: false,
        })
    }

    /// Opens the mapset picker and refreshes the Songs list behind it (D-82).
    fn open_picker(&mut self, purpose: Purpose, ctx: &Context) {
        self.picker = Some(Picker::new(purpose));
        self.dialog = Some(Dialog::Mapsets);
        if let Some(root) = self.state.as_ref().and_then(|s| s.songs_dir.clone()) {
            self.songs
                .ensure(&root, crate::worker::songs_cache(), true, ctx);
        }
    }

    fn active(&self) -> Option<&crate::worker::ProjectView> {
        self.state.as_ref()?.active.as_deref()
    }

    fn ready(&self) -> bool {
        self.busy.is_none() && !self.offline
    }

    fn announce(&mut self, message: impl Into<String>, error: bool, now: f64) {
        self.toast = Some(Toast {
            message: message.into(),
            error,
            since: now,
            action: None,
        });
    }

    fn start_separation(
        &mut self,
        request: hs_session::separation::Request,
        ctx: &Context,
        now: f64,
    ) {
        let repaint = ctx.clone();
        match Separation::start(request, Arc::new(move || repaint.request_repaint())) {
            Ok(job) => {
                self.separation = Some(Running {
                    job,
                    segments_from: None,
                });
                self.toast = None;
            }
            Err(e) => self.announce(format!("Could not start the separation: {e}"), true, now),
        }
    }

    /// Picks up a finished separation and hands it to the worker when it is free.
    fn poll_separation(&mut self, now: f64) {
        if let Some(running) = self.separation.as_mut() {
            if let Some(Stage::Separating(p)) = running.job.stage()
                && p.total > 0
            {
                running.segments_from.get_or_insert(now);
            }
            if let Some(result) = running.job.take_result() {
                let cancelled = running.job.cancelling();
                self.separation = None;
                match result {
                    Ok(stem) => self.separated = Some(Box::new(stem)),
                    Err(_) if cancelled => self.announce("Separation cancelled", false, now),
                    Err(error) => self.announce(error, true, now),
                }
            }
        }
        if self.separated.is_some() && self.ready() {
            let stem = self.separated.take().expect("checked above");
            if let Some(export) = self.export.as_mut() {
                export.invalidate();
            }
            self.start(Job::ApplySeparation(stem), now);
        }
    }

    fn separation_card(&self, now: f64) -> Option<SeparationCard> {
        let running = self.separation.as_ref()?;
        let stage = running.job.stage()?;
        let detail = match stage {
            Stage::Preparing => "Reading the song and loading the model".to_owned(),
            Stage::Separating(p) if p.total > 0 => {
                let mut text = format!("Segment {} of {}", (p.done + 1).min(p.total), p.total);
                if let Some(from) = running.segments_from
                    && p.done > 0
                {
                    let left = (now - from) / p.done as f64 * (p.total - p.done) as f64;
                    text.push_str(&if left < 60.0 {
                        " · under a minute left".to_owned()
                    } else {
                        format!(" · about {:.0} min left", (left / 60.0).ceil())
                    });
                }
                text
            }
            Stage::Separating(_) => "Starting".to_owned(),
            Stage::Writing => "Writing and checking stems/drums.ogg".to_owned(),
        };
        Some(SeparationCard {
            fraction: stage.fraction(),
            detail,
            cancelling: running.job.cancelling(),
        })
    }

    fn worker_lost(&mut self, now: f64) {
        self.offline = true;
        self.busy = None;
        self.after_save = None;
        self.announce(
            "The project worker stopped. Restart hitsounds-rs to open or save projects.",
            true,
            now,
        );
    }

    fn start(&mut self, job: Job, now: f64) {
        if self.ready() {
            let label = job.label();
            if self.worker.jobs.send(job).is_ok() {
                self.busy = Some(label);
            } else {
                self.worker_lost(now);
            }
        }
    }

    fn request_departure(&mut self, departure: Departure, ctx: &Context, now: f64) {
        if self.active().is_some_and(|p| p.dirty) {
            self.pending_departure = Some(departure);
            self.dialog = Some(Dialog::Confirm);
        } else {
            self.depart(departure, ctx, now);
        }
    }

    fn depart(&mut self, departure: Departure, ctx: &Context, now: f64) {
        match departure {
            // D-82: a new project starts in the mapset picker.
            Departure::Job(Job::Create) => self.open_picker(Purpose::Create, ctx),
            Departure::Job(job) => self.start(job, now),
            Departure::Exit => {
                self.exit_approved = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    fn request_exit(&mut self, ctx: &Context, now: f64) {
        if self.busy.is_some() {
            self.exit_when_ready = true;
        } else {
            self.request_departure(Departure::Exit, ctx, now);
        }
    }

    /// D-84: one check after startup, and a toast once an update is downloaded. The
    /// toast waits for a quiet moment: no other toast, dialog or separation.
    fn watch_updates(&mut self, ctx: &Context, now: f64) {
        let Some(enabled) = self.state.as_ref().map(|s| s.settings.checks_for_updates()) else {
            return;
        };
        update::set_enabled(enabled);
        if !self.update_started {
            self.update_started = true;
            if enabled {
                update::check(ctx, Duration::from_secs(5));
            }
        }
        if enabled
            && !self.update_announced
            && self.toast.is_none()
            && self.dialog.is_none()
            && self.separation.is_none()
            && let Some(ready) = update::status().ready()
        {
            self.update_announced = true;
            self.toast = Some(Toast {
                message: format!(
                    "hitsounds-rs {} is ready. It installs when you close the app.",
                    ready.version
                ),
                error: false,
                since: now,
                action: Some(ToastAction::Update),
            });
        }
    }

    fn poll(&mut self, ctx: &Context, now: f64) {
        if self.offline {
            return;
        }
        let reply = match self.worker.replies.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.worker_lost(now);
                return;
            }
        };
        let same_project = match (self.active(), reply.snapshot.active.as_deref()) {
            (None, None) => self.state.is_some(),
            (Some(old), Some(new)) => Arc::ptr_eq(&old.mapset_changed, &new.mapset_changed),
            _ => false,
        };
        if !same_project {
            self.entered_at = now;
            self.refresh_at = None;
            self.clock = Clock::default();
            self.settle_until = now + 0.25;
            self.editor.new_project();
            self.export = None;
            // A separation belongs to the project it started in.
            if let Some(running) = self.separation.as_ref() {
                running.job.cancel();
            }
            self.separated = None;
        } else if let (Some(old), Some(new)) = (self.active(), reply.snapshot.active.as_deref())
            && old.layer.file != new.layer.file
        {
            self.editor.layer_changed(new.layer.file.is_some());
        }
        if let Some(view) = &reply.snapshot.active {
            let unchanged = self
                .state
                .as_ref()
                .and_then(|state| state.active.as_ref())
                .is_some_and(|old| Arc::ptr_eq(old, view));
            if !unchanged {
                self.editor.timeline.adopt(view);
            }
        }
        self.state = Some(reply.snapshot);
        let job = self.busy.take();
        let next = self.after_save.take();
        match reply.result {
            Ok(outcome) => {
                match outcome {
                    Some(Outcome::Notice(message)) => self.announce(message, false, now),
                    Some(Outcome::SelectChannel(id)) => self.editor.select_channel(id),
                    Some(Outcome::ExportPreview(review)) => {
                        if let Some(export) = self.export.as_mut() {
                            export.review(review);
                        }
                    }
                    Some(Outcome::ExportDiff(diff)) => {
                        if let Some(export) = self.export.as_mut() {
                            export.compare(diff);
                        }
                    }
                    Some(Outcome::Exported(result)) => {
                        self.announce("Export written", false, now);
                        if let Some(export) = self.export.as_mut() {
                            export.finish(result);
                        }
                    }
                    Some(Outcome::Separate(request)) => self.start_separation(*request, ctx, now),
                    Some(Outcome::MapsetMoved { file, moved }) => {
                        self.open_picker(Purpose::Relocate { file, moved }, ctx);
                    }
                    Some(Outcome::Created(message)) => {
                        self.announce(message, false, now);
                        // D-70: an optional step of new-project setup, offered once.
                        let offer = self.state.as_ref().is_some_and(|s| {
                            s.model_ready
                                && s.runtime_ready
                                && s.active
                                    .as_ref()
                                    .is_some_and(|p| p.project.audio.stem.is_none())
                        });
                        if offer && let Some(toast) = self.toast.as_mut() {
                            toast.action = Some(ToastAction::Separate);
                        }
                    }
                    None => {}
                }
                if let Some(next) = next {
                    self.depart(next, ctx, now);
                }
            }
            Err(error) => {
                if let Some(export) = self.export.as_mut()
                    && matches!(
                        job,
                        Some("Building export preview" | "Comparing .osu lines" | "Writing export")
                    )
                {
                    export.fail(error.clone());
                }
                self.announce(error, true, now);
            }
        }
    }

    fn watch(&mut self, ctx: &Context, now: f64) {
        // Coalesce the multiple filesystem notifications emitted by an editor save (D-20).
        const DEBOUNCE: f64 = 0.36;
        if self
            .active()
            .is_some_and(|project| project.mapset_changed.swap(false, Ordering::Relaxed))
        {
            self.refresh_at = Some(now + DEBOUNCE);
            if let Some(export) = self.export.as_mut() {
                export.invalidate();
            }
        }
        if let Some(due) = self.refresh_at
            && self.ready()
            && self.dialog != Some(Dialog::Confirm)
            && !self.exit_approved
        {
            if now >= due {
                self.refresh_at = None;
                self.start(Job::Refresh { automatic: true }, now);
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(due - now));
            }
        }
    }

    fn dispatch(&mut self, intent: Intent, ctx: &Context, now: f64) {
        match intent {
            Intent::Exit => self.request_exit(ctx, now),
            Intent::Show(dialog) => {
                if dialog == Dialog::Palette {
                    self.palette = Palette::default();
                }
                self.dialog = Some(dialog);
            }
            Intent::DismissDialog => {
                self.dialog = None;
                self.picker = None;
                if matches!(self.pending_departure.take(), Some(Departure::Exit)) {
                    // Staying open also cancels "Restart to update".
                    update::set_relaunch(false);
                }
            }
            Intent::CheckUpdates => update::check(ctx, Duration::ZERO),
            Intent::RestartToUpdate => {
                update::set_relaunch(true);
                self.request_exit(ctx, now);
            }
            Intent::UpdateNotes => {
                if let Some(ready) = update::status().ready() {
                    update::open_notes(&ready.notes);
                }
            }
            Intent::Pick(job) => {
                self.dialog = None;
                self.picker = None;
                if self.ready() {
                    if matches!(job, Job::SetMapset(_) | Job::ChangeMapset)
                        && let Some(export) = self.export.as_mut()
                    {
                        export.invalidate();
                    }
                    self.start(job, now);
                }
            }
            Intent::DismissToast => self.toast = None,
            Intent::CancelSeparation => {
                if let Some(running) = self.separation.as_ref() {
                    running.job.cancel();
                }
            }
            Intent::Job(Job::Separate) if self.separation.is_some() => {
                self.announce("A drum separation is already running.", false, now);
            }
            Intent::SelectChannel(id) => self.editor.select_channel(id),
            Intent::ShowDifficulty => self.editor.show_difficulty(),
            Intent::Editor(act) => {
                if self.export.is_none()
                    && let Some(view) = self.state.as_ref().and_then(|s| s.active.clone())
                {
                    let mut intents = Vec::new();
                    self.editor
                        .act(act, &view, &self.clock, now, self.reduced, &mut |intent| {
                            intents.push(intent)
                        });
                    for intent in intents {
                        self.dispatch(intent, ctx, now);
                    }
                }
            }
            Intent::StepIssue { backwards } => {
                if let Some(view) = self.state.as_ref().and_then(|s| s.active.clone()) {
                    let mut intents = Vec::new();
                    self.editor.timeline.next_issue(
                        &view,
                        self.clock.position_ms,
                        backwards,
                        &mut |intent| intents.push(intent),
                    );
                    self.editor.show_difficulty();
                    for intent in intents {
                        self.dispatch(intent, ctx, now);
                    }
                }
            }
            Intent::OpenExport if self.ready() && self.export.is_none() => {
                if let Some((options, included)) = self
                    .active()
                    .map(|view| (view.project.export.clone(), Included::list(view)))
                {
                    if self.clock.playing {
                        self.transport(Transport::Pause, ctx, now);
                    }
                    self.export = Some(ExportPage::new(options, included));
                }
            }
            Intent::CloseExport if self.busy != Some("Writing export") => self.export = None,
            Intent::HideExportDiff => {
                if let Some(export) = self.export.as_mut() {
                    export.hide_diff();
                }
            }
            Intent::Confirm(decision) => {
                self.dialog = None;
                if let Some(departure) = self.pending_departure.take() {
                    match decision {
                        Decision::Save if self.ready() => {
                            self.after_save = Some(departure);
                            self.start(Job::Save, now);
                        }
                        Decision::Discard => self.depart(departure, ctx, now),
                        Decision::Save => {
                            self.announce("The project cannot be saved right now.", true, now)
                        }
                    }
                }
            }
            Intent::Job(job) if self.ready() => match job {
                Job::Create
                | Job::ImportHitsounds
                | Job::Open
                | Job::OpenRecent(_)
                | Job::Close => {
                    self.request_departure(Departure::Job(job), ctx, now);
                }
                Job::WriteExport => {
                    if self.refresh_at.is_none()
                        && self.export.as_ref().is_some_and(ExportPage::can_write)
                    {
                        self.start(job, now);
                    }
                }
                Job::ChangeMapset => self.open_picker(Purpose::Change, ctx),
                Job::Refresh { .. }
                | Job::History { .. }
                | Job::Fallback(_)
                | Job::ImportStem
                | Job::RemoveStem
                | Job::ChooseSkin
                | Job::ClearSkin => {
                    if let Some(export) = self.export.as_mut() {
                        export.invalidate();
                    }
                    self.start(job, now);
                }
                _ => self.start(job, now),
            },
            Intent::TogglePlayback if self.ready() && self.clock.length_ms > 0.0 => {
                let action = if self.clock.playing {
                    Transport::Pause
                } else {
                    Transport::Play
                };
                self.transport(action, ctx, now);
            }
            Intent::Transport(action) if self.ready() && self.active().is_some() => {
                self.transport(action, ctx, now);
            }
            Intent::PreviewLevels(levels) => self.worker.preview_levels(&levels),
            _ => {}
        }
    }

    fn transport(&mut self, action: Transport, ctx: &Context, now: f64) {
        match self.worker.transport(action) {
            Ok(()) => {
                // Wake the UI while the audio callback consumes the command.
                self.settle_until = now + 0.25;
                ctx.request_repaint();
            }
            Err(error) => self.announce(error, true, now),
        }
    }

    fn ui_scale(&self) -> f32 {
        self.state
            .as_ref()
            .and_then(|s| s.settings.ui_scale)
            .unwrap_or(1.0)
    }

    /// Applies the interface scale and keeps the D-63 minimum window in scaled points.
    fn apply_scale(&mut self, ctx: &Context) {
        let scale = self.ui_scale();
        if ctx.zoom_factor() != scale {
            ctx.set_zoom_factor(scale);
        }
        let ppp = ctx.pixels_per_point();
        if self.min_size_ppp != ppp {
            self.min_size_ppp = ppp;
            let min = t::min_window(ctx);
            if !chrome::set_min_size(min, ppp) {
                ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(min));
            }
        }
    }

    fn shortcut(&self, ctx: &Context, alt_space: bool) -> Option<Intent> {
        if self.dialog.is_some() || egui::Popup::is_any_open(ctx) || text_has_focus(ctx) {
            return None;
        }
        // Read before `input_mut`: the context lock is not re-entrant.
        let scale = self.ui_scale();
        let larger = t::step_scale(ctx, scale, true);
        let smaller = t::step_scale(ctx, scale, false);
        let map = keys::current();
        let pressed = |i: &mut egui::InputState, action| map.pressed(i, action);
        ctx.input_mut(|i| {
            // `?` stays a fixed second key for the gesture sheet unless a binding uses it.
            let question = !map.uses(Key::Questionmark)
                && i.events
                    .iter()
                    .any(|event| matches!(event, Event::Text(text) if text == "?"));
            if pressed(i, Action::Help) || question {
                return Some(Intent::Show(Dialog::Help));
            }
            let ctrl_shift_alt = Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALT;
            if i.consume_key(ctrl_shift_alt, Key::H) {
                // D-56: the encore is a visual detail in About and never edits the project.
                return Some(Intent::Show(Dialog::Encore));
            }
            if pressed(i, Action::Palette) {
                return Some(Intent::Show(Dialog::Palette));
            }
            if pressed(i, Action::Settings) {
                return Some(Intent::Show(Dialog::Settings));
            }
            // D-68: browser-style interface scale keys, saved as an app setting.
            if pressed(i, Action::ScaleUp) {
                return larger.map(|s| Intent::Job(Job::UiScale(Some(s))));
            }
            if pressed(i, Action::ScaleDown) {
                return smaller.map(|s| Intent::Job(Job::UiScale(Some(s))));
            }
            if pressed(i, Action::ScaleReset) && scale != 1.0 {
                return Some(Intent::Job(Job::UiScale(None)));
            }
            if self.export.is_some() {
                return i
                    .consume_key(Modifiers::NONE, Key::Escape)
                    .then_some(Intent::CloseExport);
            }
            if !self.ready() {
                return None;
            }
            if pressed(i, Action::NewProject) {
                return Some(Intent::Job(Job::Create));
            }
            if pressed(i, Action::OpenProject) {
                return Some(Intent::Job(Job::Open));
            }
            let project = self.active()?;
            if pressed(i, Action::Export) {
                return Some(Intent::OpenExport);
            }
            for panel in Panel::ALL {
                if pressed(i, panel.action()) {
                    return Some(Intent::Editor(Act::Toggle(panel)));
                }
            }
            for tool in Tool::ALL {
                if pressed(i, tool.action()) {
                    return Some(Intent::Editor(Act::Tool(tool)));
                }
            }
            if pressed(i, Action::ToggleWaveform) {
                return Some(Intent::Editor(Act::Waveform));
            }
            if project.layer.file.is_some() {
                if pressed(i, Action::NextIssue) {
                    return Some(Intent::StepIssue { backwards: false });
                }
                if pressed(i, Action::PreviousIssue) {
                    return Some(Intent::StepIssue { backwards: true });
                }
            }
            if pressed(i, Action::Save) {
                return Some(Intent::Job(Job::Save));
            }
            if pressed(i, Action::CloseProject) {
                return Some(Intent::Job(Job::Close));
            }
            if project.can_redo && pressed(i, Action::Redo) {
                return Some(Intent::Job(Job::History { redo: true }));
            }
            if project.can_undo && pressed(i, Action::Undo) {
                return Some(Intent::Job(Job::History { redo: false }));
            }
            if pressed(i, Action::GoToStart) {
                return Some(Intent::Transport(Transport::Seek { ms: 0.0 }));
            }
            if project.stem_ready && pressed(i, Action::ToggleMonitor) {
                let source = match project.project.audio.monitor {
                    Monitor::Original => Monitor::Stems,
                    Monitor::Stems => Monitor::Original,
                };
                return Some(Intent::Job(Job::Monitor(source)));
            }
            // D-46: Alt+Space plays and pauses while Space does.
            let alt_space = alt_space && map.has_plain(Action::PlayPause, Key::Space);
            if pressed(i, Action::PlayPause) || alt_space {
                return Some(Intent::TogglePlayback);
            }
            None
        })
    }
}

fn text_has_focus(ctx: &Context) -> bool {
    let focused = ctx.memory(|memory| memory.focused());
    focused.is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some())
}

impl eframe::App for Studio {
    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, ctx: &Context, input: &mut egui::RawInput) {
        let Some(path) = std::env::var_os("HITSOUNDS_RS_QA_INPUT") else {
            return;
        };
        input.focused = true;
        if let Ok(line) = std::fs::read_to_string(&path) {
            let _ = std::fs::remove_file(&path);
            let words: Vec<_> = line.split_whitespace().collect();
            match words.as_slice() {
                [action, x, y] if *action != "key" => {
                    let pos = egui::pos2(x.parse().unwrap(), y.parse().unwrap());
                    input.events.push(Event::PointerMoved(pos));
                    if *action != "move" {
                        let button = if action.starts_with("right") {
                            egui::PointerButton::Secondary
                        } else {
                            egui::PointerButton::Primary
                        };
                        input.events.push(Event::PointerButton {
                            pos,
                            button,
                            pressed: action.ends_with("down"),
                            modifiers: Modifiers::NONE,
                        });
                    }
                }
                ["key", name] | ["key", name, _] => {
                    let modifiers = match words.get(2).copied() {
                        Some("ctrl") => Modifiers::CTRL,
                        Some("shift") => Modifiers::SHIFT,
                        Some("alt") => Modifiers::ALT,
                        _ => Modifiers::NONE,
                    };
                    if let Some(key) = Key::from_name(name) {
                        input.modifiers = modifiers;
                        for pressed in [true, false] {
                            input.events.push(Event::Key {
                                key,
                                physical_key: Some(key),
                                pressed,
                                repeat: false,
                                modifiers,
                            });
                        }
                    }
                }
                ["wheel", x, y, lines, ..] => {
                    let modifiers = match words.get(4).copied() {
                        Some("ctrl") => Modifiers::CTRL,
                        Some("shift") => Modifiers::SHIFT,
                        Some("alt") => Modifiers::ALT,
                        _ => Modifiers::NONE,
                    };
                    input.modifiers = modifiers;
                    input.events.push(Event::PointerMoved(egui::pos2(
                        x.parse().unwrap(),
                        y.parse().unwrap(),
                    )));
                    input.events.push(Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta: egui::vec2(0.0, lines.parse().unwrap()),
                        modifiers,
                    });
                }
                _ => {}
            }
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        #[cfg(debug_assertions)]
        qa_capture(ctx);
        let now = ctx.input(|i| i.time);
        let hovered = widgets::begin_hints(ctx);
        self.poll(ctx, now);
        self.poll_separation(now);
        self.songs.poll(ctx);
        if let Some(state) = &self.state {
            keys::install(&state.settings.keybinds);
        }
        self.watch_updates(ctx, now);
        if self.exit_when_ready && self.busy.is_none() {
            self.exit_when_ready = false;
            self.request_exit(ctx, now);
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.exit_approved
            && (self.busy.is_some() || self.active().is_some_and(|p| p.dirty))
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.request_exit(ctx, now);
        }
        self.apply_scale(ctx);
        let alt_space = chrome::take_alt_space();
        if let Some(intent) = self.shortcut(ctx, alt_space) {
            self.dispatch(intent, ctx, now);
        }
        self.watch(ctx, now);
        if self.ready() && self.dialog.is_none() {
            if let Some(export) = self.export.as_mut() {
                if self.refresh_at.is_none()
                    && let Some(job) = export.build(ctx, now)
                {
                    self.start(job, now);
                }
            } else if self.editor.browser_open
                && self.state.as_ref().is_some_and(|state| {
                    state.active.is_some()
                        && state.library.is_none()
                        && !state.settings.library.is_empty()
                })
            {
                self.start(Job::ScanLibrary, now);
            }
        }
        let reduced = self
            .state
            .as_ref()
            .and_then(|s| s.settings.reduce_motion)
            .unwrap_or_else(chrome::reduce_motion);
        if self.reduced != reduced {
            self.reduced = reduced;
            ctx.style_mut(|style| {
                style.animation_time = if reduced { 0.0 } else { t::FEEDBACK_SECONDS };
                style.scroll_animation = if reduced {
                    egui::style::ScrollAnimation::none()
                } else {
                    Default::default()
                };
            });
        }
        if self.active().is_some() {
            if let Some(clock) = self.worker.clock() {
                self.clock = clock;
            } else {
                if self.clock.playing {
                    self.clock.position_ms = (self.clock.position_ms
                        + (now - self.last_frame).max(0.0) * 1000.0 * self.clock.rate)
                        .min(self.clock.length_ms);
                }
                if self.ready() {
                    ctx.request_repaint_after(Duration::from_millis(16));
                }
            }
            if self.clock.playing || now < self.settle_until {
                ctx.request_repaint();
            }
        }
        self.last_frame = now;
        let entrance = if reduced {
            1.0
        } else {
            t::ease_out(((now - self.entered_at) / t::ENTER_SECONDS) as f32)
        };
        if entrance < 1.0 && self.active().is_none() {
            ctx.request_repaint();
        }

        let mut intents = Vec::new();
        let emit = &mut |intent| intents.push(intent);
        let ready = self.ready();
        let active = self.state.as_ref().and_then(|s| s.active.clone());
        let editing = self.export.is_none() && active.is_some();
        if editing
            && ready
            && self.dialog.is_none()
            && !egui::Popup::is_any_open(ctx)
            && !text_has_focus(ctx)
            && let Some(view) = &active
        {
            self.editor.timeline.keys(ctx, view, &self.clock, emit);
        }
        let status = editing.then(|| self.editor.status());
        // With nothing under the pointer, the hint bar says what the current tool does.
        let hint = match status {
            Some(status) if hovered == Hint::default() => Hint::new(status.tool.summary()),
            _ => hovered,
        };
        views::title_bar(
            ctx,
            &self.logo,
            &views::Shell {
                state: self.state.as_ref(),
                available: ready,
                custom_frame: self.custom_frame,
                editor: status,
                exporting: self.export.is_some(),
                separating: self.separation.is_some(),
                hint: &hint,
            },
            emit,
        );
        views::status(
            ctx,
            self.state.as_ref(),
            self.busy,
            reduced,
            status.map(|status| (self.editor.timeline.hover.as_str(), status)),
            emit,
        );
        if let Some(export) = self.export.as_mut() {
            export.show(ctx, now, self.busy, reduced, emit);
        } else if let (Some(state), Some(view)) = (self.state.as_ref(), active.as_deref()) {
            self.editor
                .show(ctx, state, view, &self.clock, ready, reduced, emit);
        } else {
            egui::CentralPanel::default().frame(Frame::new().fill(t::BASE)).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("home-page").show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let width = (ui.available_width() - t::PAGE_MARGIN * 2.0).min(t::PAGE_WIDTH);
                ui.vertical_centered(|ui| {
                    ui.add_space(t::PAGE_MARGIN);
                    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(width);
                        match self.state.as_ref() {
                            Some(state) => views::landing(ui, &self.logo, state, ready, entrance, emit),
                            None => {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(t::SECTION);
                                    ui.image((self.logo.id(), vec2(t::HERO_SIZE, t::HERO_SIZE)));
                                    ui.label(t::display("hitsounds-rs", t::HEADING_SIZE));
                                    if self.offline {
                                        ui.label(t::muted("Settings are unavailable. Restart hitsounds-rs to try again."));
                                    } else {
                                        // D-56: the startup line is separate from the loading status.
                                        ui.label(t::muted(self.startup_line));
                                        ui.add_space(t::GAP);
                                        views::activity(ui, reduced);
                                    }
                                });
                            }
                        }
                    });
                    ui.add_space(t::PAGE_MARGIN);
                });
            });
        });
        }
        if self.dialog == Some(Dialog::Palette) {
            let offer = Offer {
                project: self.state.as_ref().and_then(|s| s.active.as_deref()),
                ready: self.ready(),
                exporting: self.export.is_some(),
                playing: self.clock.playing,
                separating: self.separation.is_some(),
                larger: t::step_scale(ctx, self.ui_scale(), true),
                smaller: t::step_scale(ctx, self.ui_scale(), false),
                levels: self
                    .state
                    .as_ref()
                    .map(|s| s.settings.levels)
                    .unwrap_or_default(),
            };
            self.palette.show(ctx, &offer, reduced, emit);
        } else if self.dialog == Some(Dialog::Mapsets)
            && let available = self.ready()
            && let Some(picker) = self.picker.as_mut()
        {
            let songs_dir = self.state.as_ref().and_then(|s| s.songs_dir.clone());
            if let Some(root) = &songs_dir {
                let refresh = picker.take_refresh();
                self.songs
                    .ensure(root, crate::worker::songs_cache(), refresh, ctx);
            }
            let projects_dir = self
                .state
                .as_ref()
                .filter(|s| s.projects_ready)
                .and_then(|s| s.settings.projects_dir.clone());
            let env = mapsets::Env {
                songs: &self.songs,
                songs_dir: songs_dir.as_deref(),
                projects_dir: projects_dir.as_deref(),
                available,
            };
            picker.show(ctx, &env, emit);
        } else if let Some(dialog) = self.dialog {
            let updates = if dialog == Dialog::Settings {
                update::status()
            } else {
                update::Status::Idle
            };
            views::dialog(
                ctx,
                dialog,
                self.state.as_ref(),
                self.ready(),
                self.separation.is_some(),
                reduced,
                &self.logo,
                &updates,
                emit,
            );
        }
        let lift = self
            .separation_card(now)
            .map_or(0.0, |card| views::separation(ctx, &card, emit) + t::SPACE);
        if let Some(toast) = self.toast.as_ref() {
            let left = toast.since + t::TOAST_SECONDS - now;
            if toast.error || toast.action.is_some() || left > 0.0 {
                views::toast(ctx, toast, reduced, now, lift, emit);
                // A toast with an action outlives its timer, so `left` goes negative,
                // and `from_secs_f64` panics on a negative value (an abort in release).
                if !toast.error && left > 0.0 {
                    ctx.request_repaint_after(Duration::from_secs_f64(left));
                }
            } else {
                self.toast = None;
            }
        }
        widgets::end_hints(ctx);
        for intent in intents {
            self.dispatch(intent, ctx, now);
            ctx.request_repaint();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        t::BASE.to_normalized_gamma_f32()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.editor.save(storage);
        // eframe has just saved the maximized size as the size to restore to. Keep it
        // maximized, but restore to the size it had before (see `chrome::restore_rect`).
        if let (Some(rect), Some(scale)) = (chrome::restore_rect(), chrome::scale_factor()) {
            eframe::set_value(
                storage,
                "window",
                &SavedWindow {
                    inner_position_pixels: Some(rect.min),
                    outer_position_pixels: Some(rect.min),
                    fullscreen: false,
                    maximized: true,
                    inner_size_points: Some(rect.size() / (scale * self.ui_scale())),
                },
            );
        }
    }
}

/// The fields of eframe's `WindowSettings`, which are private, under the key eframe
/// stores them at. The window has no frame, so its outer and inner rectangles match.
#[derive(serde::Serialize)]
struct SavedWindow {
    inner_position_pixels: Option<egui::Pos2>,
    outer_position_pixels: Option<egui::Pos2>,
    fullscreen: bool,
    maximized: bool,
    inner_size_points: Option<egui::Vec2>,
}

#[cfg(debug_assertions)]
fn qa_capture(ctx: &Context) {
    use eframe::icon_data::IconDataExt as _;
    let Some(trigger) = std::env::var_os("HITSOUNDS_RS_QA_CAPTURE") else {
        return;
    };
    if let Ok(target) = std::fs::read_to_string(&trigger) {
        let _ = std::fs::remove_file(&trigger);
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::new(target)));
    }
    ctx.input(|input| {
        for event in &input.events {
            if let Event::Screenshot {
                user_data, image, ..
            } = event
                && let Some(target) = user_data
                    .data
                    .as_ref()
                    .and_then(|data| data.downcast_ref::<String>())
            {
                let icon = IconData {
                    rgba: image
                        .pixels
                        .iter()
                        .flat_map(|p| p.to_srgba_unmultiplied())
                        .collect(),
                    width: image.size[0] as u32,
                    height: image.size[1] as u32,
                };
                std::fs::write(target, icon.to_png_bytes().unwrap()).unwrap();
            }
        }
    });
}
