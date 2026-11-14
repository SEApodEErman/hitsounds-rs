use super::{
    canvas::{Geometry, Timeline, Tool},
    grid::BeatGrid,
    model::{Layer, Wave},
    rail::DragItem,
};
use crate::{
    app::Intent,
    theme,
    worker::{Job, ProjectView, Snapshot},
};
use eframe::egui::{
    self, Context, Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, pos2, vec2,
};
use egui_kittest::{Harness, kittest::Queryable};
use hs_core::{
    Channel, Curve, DiffProfile, DiffTimeline, Edge, EdgeKind, Edit, History, Project, Target,
    TickPolicy, projection::Tick,
};
use hs_session::{Clock, Transport, Waveform};
use osu_format::{Hitsound, SampleSet};
use std::{path::PathBuf, sync::Arc};

struct Fixture {
    timeline: Timeline,
    view: ProjectView,
    clock: Clock,
    imported: Vec<(Option<u32>, PathBuf)>,
    transport: Vec<Transport>,
    /// Every edit the timeline sent, in order: one entry is one undo step.
    edits: Vec<Edit>,
    body: Rect,
}

impl Fixture {
    fn new(channels: usize) -> Self {
        let mut project = Project::new("Synthetic", "mapset");
        for i in 0..channels {
            let mut channel = Channel::new(
                0,
                format!("Track {}", i + 1),
                SampleSet::Soft,
                Hitsound::Normal,
            );
            channel.events = vec![500, 1000 + i as i32 * 125];
            project.add_channel(channel);
        }
        let layer = Layer::new(&project, None, BeatGrid::default(), None, None);
        let peaks: Vec<_> = (0..45_000)
            .flat_map(|i| [-(i % 120) as i8, (i % 100) as i8])
            .collect();
        let view = ProjectView {
            file: "project.json".into(),
            project,
            available_diffs: Vec::new(),
            length_ms: 180_000.0,
            stem_ready: false,
            stem_outdated: false,
            can_undo: false,
            can_redo: false,
            dirty: false,
            warnings: Vec::new(),
            waveform: Arc::new(Wave::new(Waveform {
                bucket_ms: 4.0,
                peaks: &peaks,
            })),
            layer,
            mapset_changed: Arc::default(),
        };
        Self {
            timeline: Timeline::default(),
            view,
            clock: Clock {
                length_ms: 180_000.0,
                rate: 1.0,
                ..Default::default()
            },
            imported: Vec::new(),
            transport: Vec::new(),
            edits: Vec::new(),
            body: Rect::NOTHING,
        }
    }

    fn diff(mut self) -> Self {
        let mut profile = DiffProfile::new("hard.osu", "Hard");
        profile.tick_policy = TickPolicy::Render;
        self.view.project.channels[0].events.push(750);
        self.view.project.channels[0].events.sort_unstable();
        self.view.project.diffs.push(profile);
        self.view.available_diffs.push("hard.osu".into());
        let timeline = DiffTimeline {
            edges: [500, 1000, 1500]
                .into_iter()
                .map(|time| Edge {
                    time,
                    kind: EdgeKind::Circle,
                    object: 0,
                    edge: 0,
                    slides: 0,
                    object_duration_ms: 0.0,
                })
                .collect(),
            ticks: vec![Tick {
                time: 750,
                object: 0,
            }],
            redlines: Vec::new(),
        };
        let profile = &self.view.project.diffs[0];
        let projection = hs_core::projection::project(&self.view.project, profile, &timeline);
        let events = profile.apply_overrides(self.view.project.base_events());
        self.view.layer = Layer::new(
            &self.view.project,
            Some("hard.osu"),
            BeatGrid::default(),
            Some(&events),
            Some((&timeline, &projection)),
        );
        self
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.body = Rect::from_min_size(ui.cursor().min, ui.available_size());
        let mut intents = Vec::new();
        let emit = &mut |intent| intents.push(intent);
        let focus = ui.memory(|memory| memory.focused());
        if !focus.is_some_and(|id| egui::TextEdit::load_state(ui.ctx(), id).is_some()) {
            self.timeline.keys(ui.ctx(), &self.view, &self.clock, emit);
        }
        self.timeline
            .show(ui, &self.view, &self.clock, true, true, emit);
        for intent in intents {
            match intent {
                Intent::Job(Job::Edit(edit)) => {
                    self.apply(&edit);
                    self.edits.push(edit);
                }
                Intent::Job(Job::UseSample {
                    channel,
                    path: Some(path),
                }) => self.imported.push((channel, path)),
                Intent::Transport(action) => {
                    match action {
                        Transport::Seek { ms } => self.clock.position_ms = ms,
                        Transport::Play => self.clock.playing = true,
                        Transport::Pause => self.clock.playing = false,
                        Transport::Rate { .. } => {}
                    }
                    self.transport.push(action);
                }
                _ => {}
            }
        }
    }

    /// Applies an edit as the session would and rebuilds the layer from it.
    fn apply(&mut self, edit: &Edit) {
        self.view.project.apply(edit).unwrap();
        let file = self.view.layer.file.as_deref();
        let diff = file.and_then(|f| self.view.project.diff(f));
        let events = diff.map(|d| d.apply_overrides(self.view.project.base_events()));
        let objects = self.view.layer.objects.as_ref().map(|o| &o.timeline);
        let projection = objects
            .zip(diff)
            .map(|(objects, diff)| hs_core::projection::project(&self.view.project, diff, objects));
        self.view.layer = Layer::new(
            &self.view.project,
            file,
            self.view.layer.grid.clone(),
            events.as_deref(),
            objects.zip(projection.as_ref()),
        );
        self.timeline.adopt(&self.view);
    }

    fn geometry(&self) -> Geometry {
        Geometry::new(self.body, self.view.layer.file.is_some())
    }
    fn x(&self, time: f64) -> f32 {
        self.timeline.viewport.x(self.geometry().plot, time)
    }
    fn lane(&self, row: usize, time: f64) -> Pos2 {
        pos2(
            self.x(time),
            self.geometry().row(row, self.timeline.scroll).center().y,
        )
    }
    fn volume(&self, time: f64, percent: f64) -> Pos2 {
        pos2(self.x(time), self.geometry().volume_y(percent))
    }
}

fn harness(fixture: Fixture) -> Harness<'static, Fixture> {
    let mut initialized = false;
    let mut harness = Harness::builder()
        .with_size(vec2(920.0, 560.0))
        .with_step_dt(1.0 / 120.0)
        .build_ui_state(
            move |ui, fixture: &mut Fixture| {
                if initialized {
                    fixture.draw(ui);
                } else {
                    theme::install(ui.ctx());
                    initialized = true;
                }
            },
            fixture,
        );
    harness.step();
    harness
}

#[test]
fn workspace_survives_tiny_frames_during_refresh_and_restore() {
    for reduced in [false, true] {
        let fixture = Fixture::new(2).diff();
        let channel = fixture.view.project.channels[0].id;
        let mut editor = super::Editor::new(None);
        editor.mixer_open = true;
        editor.layer_changed(true);
        editor.select_channel(channel);
        editor.timeline.selection.insert((channel, 500));
        let state = Snapshot {
            settings: Default::default(),
            skin_ready: false,
            model_ready: false,
            runtime_ready: false,
            recent_exists: Vec::new(),
            songs_dir: None,
            songs_detected: false,
            projects_ready: false,
            warning: None,
            active: Some(Arc::new(fixture.view)),
            library: None,
        };
        let view = state.active.as_ref().unwrap();
        let project_before = view.project.clone();
        let ctx = Context::default();
        theme::install(&ctx);
        let mut intents = Vec::new();
        for (i, size) in [
            vec2(theme::INITIAL_WIDTH, theme::INITIAL_HEIGHT),
            // The reported panic had a 43.640625 px upper bound (30% of this width).
            vec2(145.46875, 32.0),
            vec2(0.0, 0.0),
            vec2(1.0, 1.0),
            vec2(733.0, theme::MIN_HEIGHT),
            vec2(theme::MIN_WIDTH, theme::MIN_HEIGHT),
            vec2(theme::INITIAL_WIDTH, theme::INITIAL_HEIGHT),
        ]
        .into_iter()
        .enumerate()
        {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(i as f64 / 60.0),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                editor.show(
                    ctx,
                    &state,
                    view,
                    &fixture.clock,
                    !(1..=3).contains(&i), // The session worker is refreshing while tiny.
                    reduced,
                    &mut |intent| intents.push(intent),
                );
            });
        }
        assert!(intents.is_empty(), "resizing must not edit the project");
        assert_eq!(view.project, project_before);
        assert!(editor.browser_open && editor.inspector_open && editor.mixer_open);
        assert!(editor.inspector.tab == super::Tab::Review);
        assert_eq!(editor.timeline.channel, Some(channel));
        assert!(editor.timeline.selection.contains(&(channel, 500)));
    }
}

fn pointer(
    h: &Harness<'_, Fixture>,
    pos: Pos2,
    button: PointerButton,
    pressed: bool,
    mods: Modifiers,
) {
    h.event_modifiers(
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: mods,
        },
        mods,
    );
}

fn click(h: &mut Harness<'_, Fixture>, pos: Pos2, button: PointerButton, mods: Modifiers) {
    h.hover_at(pos);
    pointer(h, pos, button, true, mods);
    pointer(h, pos, button, false, mods);
    h.step();
}

fn drag(h: &mut Harness<'_, Fixture>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    pointer(h, from, PointerButton::Primary, true, Modifiers::NONE);
    h.hover_at(to);
    pointer(h, to, PointerButton::Primary, false, Modifiers::NONE);
    h.step();
}

fn wheel(h: &mut Harness<'_, Fixture>, pos: Pos2, delta: f32, mods: Modifiers) {
    h.hover_at(pos);
    h.event_modifiers(
        Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, delta),
            modifiers: mods,
        },
        mods,
    );
    h.step();
}

#[test]
fn stable_wheel_seek_shift_playing_zoom_divisor_and_channel_scroll() {
    let mut h = harness(Fixture::new(30));
    let plot = h.state().geometry().plot;
    let cursor = plot.center();
    wheel(&mut h, cursor, -1.0, Modifiers::NONE);
    assert_eq!(h.state().clock.position_ms, 125.0);
    wheel(&mut h, cursor, -1.0, Modifiers::SHIFT);
    assert_eq!(h.state().clock.position_ms, 625.0);
    h.state_mut().clock.playing = true;
    wheel(&mut h, cursor, -1.0, Modifiers::NONE);
    assert_eq!(h.state().clock.position_ms, 1000.0);
    wheel(&mut h, cursor, 1.0, Modifiers::CTRL);
    assert_eq!(h.state().timeline.divisor, 8);
    h.state_mut().timeline.follow = false;
    let anchor = h.state().timeline.viewport.time(plot, cursor.x);
    let scale = h.state().timeline.viewport.scale;
    wheel(&mut h, cursor, 1.0, Modifiers::ALT);
    assert!(h.state().timeline.viewport.scale > scale);
    assert!((h.state().timeline.viewport.time(plot, cursor.x) - anchor).abs() < 0.02);
    let rail = h.state().geometry().rail.center();
    wheel(&mut h, rail, -1.0, Modifiers::NONE);
    assert!(h.state().timeline.scroll > 0.0);
    assert_eq!(h.state().clock.position_ms, 1000.0);
}

#[test]
fn ruler_draws_stable_subdivisions_and_bar_labels() {
    let h = harness(Fixture::new(2));
    let shapes = &h.output().shapes;
    let blue = egui::Color32::from_rgb(50, 128, 255).gamma_multiply(0.78);
    assert!(shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == blue)));
    assert!(
        shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "1")
        )
    );
}

#[test]
fn place_select_move_and_delete_through_timeline() {
    let mut h = harness(Fixture::new(2));
    let at = h.state().lane(0, 138.0);
    click(&mut h, at, PointerButton::Primary, Modifiers::NONE);
    assert!(h.state().view.layer.contains(&(1, 125)));
    let other = h.state().lane(1, 500.0);
    click(&mut h, other, PointerButton::Primary, Modifiers::CTRL);
    assert_eq!(h.state().timeline.selection.len(), 2);
    click(&mut h, other, PointerButton::Primary, Modifiers::CTRL);
    assert_eq!(h.state().timeline.selection.len(), 1);
    let existing = h.state().lane(0, 500.0);
    let destination = h.state().lane(1, 750.0);
    drag(&mut h, existing, destination);
    assert!(!h.state().view.layer.contains(&(1, 500)));
    assert!(h.state().view.layer.contains(&(2, 750)));
    let start = h.state().lane(0, 40.0);
    let end = h.state().lane(1, 1185.0);
    drag(&mut h, start, end);
    assert_eq!(h.state().timeline.selection.len(), 5);
    h.key_press(Key::Escape);
    h.step();
    assert!(h.state().timeline.selection.is_empty());
    click(
        &mut h,
        destination,
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    assert!(!h.state().view.layer.contains(&(2, 750)));
    h.state_mut().clock.position_ms = 1500.0;
    h.key_press(Key::Enter);
    h.step();
    let channel = h.state().timeline.channel.expect("an active channel");
    assert!(h.state().view.layer.contains(&(channel, 1500)));
    h.key_press_modifiers(Modifiers::CTRL, Key::A);
    h.step();
    assert_eq!(h.state().timeline.selection.len(), 5);
    h.key_press(Key::Delete);
    h.step();
    assert!(h.state().view.project.base_events().next().is_none());
}

#[test]
fn volume_points_need_ctrl_to_add_and_a_click_switches_ramp_and_step() {
    let mut h = harness(Fixture::new(2));
    let a = h.state().volume(500.0, 80.0);
    let b = h.state().volume(1250.0, 60.0);
    // D-89: a plain click on the lane adds nothing.
    click(&mut h, a, PointerButton::Primary, Modifiers::NONE);
    assert!(h.state().view.project.volume.points.is_empty());
    assert!(h.state().edits.is_empty());
    click(&mut h, a, PointerButton::Primary, Modifiers::CTRL);
    click(&mut h, b, PointerButton::Primary, Modifiers::CTRL);
    assert_eq!(h.state().view.project.volume.points.len(), 2);
    let empty = h.state().volume(2000.0, 50.0);
    click(&mut h, empty, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().view.project.volume.points.len(), 2);
    let destination = h.state().volume(1000.0, 40.0);
    drag(&mut h, b, destination);
    assert_eq!(
        (
            h.state().view.project.volume.points[1].time,
            h.state().view.project.volume.points[1].volume
        ),
        (1000, 40)
    );
    // Dragging a point never switches its curve.
    assert_eq!(h.state().view.project.volume.points[0].curve, Curve::Linear);
    click(&mut h, a, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().view.project.volume.points[0].curve, Curve::Step);
    assert_eq!(h.state().view.project.volume.points.len(), 2);
    let between = h.state().volume(750.0, 60.0);
    h.hover_at(between);
    h.step();
    assert!(h.state().timeline.hover.contains("Volume 80%"));
    click(&mut h, a, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().view.project.volume.points[0].curve, Curve::Linear);
    // The last point's curve leads nowhere, so a click on it changes nothing.
    let edits = h.state().edits.len();
    click(&mut h, destination, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().edits.len(), edits);
    click(
        &mut h,
        destination,
        PointerButton::Secondary,
        Modifiers::NONE,
    );
    assert_eq!(h.state().view.project.volume.points.len(), 1);
}

#[test]
fn paste_replaces_the_span_at_the_playhead_as_one_undo_step() {
    let mut h = harness(Fixture::new(2));
    // Track 1 has 500 and 1000, Track 2 has 500 and 1125.
    h.state_mut().apply(&Edit::AddEvents {
        diff: None,
        events: vec![(1, 4250), (1, 4700), (2, 4250)],
    });
    h.state_mut().timeline.selection = [(1, 500), (1, 1000)].into();
    h.event(Event::Copy);
    h.step();
    assert_eq!(h.state().timeline.clipboard_len(), 2);
    // Copying edits nothing.
    assert!(h.state().edits.is_empty());

    let before = h.state().view.project.clone();
    // Off the grid: the paste lands on the snapped playhead, as Enter does.
    h.state_mut().clock.position_ms = 4003.0;
    h.state_mut().timeline.selection.clear();
    h.event(Event::Paste("anything".into()));
    h.step();
    let lanes = &h.state().view.layer.lanes;
    // 4250 sat inside the pasted span on a pasted channel, so it is replaced; 4700 is
    // past the span's end and stays.
    assert_eq!(lanes[0].hits, [500, 1000, 4000, 4500, 4700]);
    // Track 2 holds nothing in the clipboard, so its hits stay.
    assert_eq!(lanes[1].hits, [500, 1125, 4250]);
    assert_eq!(h.state().timeline.selection, [(1, 4000), (1, 4500)].into());
    assert_eq!(h.state().edits.len(), 1, "a paste is one undo step");

    // Undo restores exactly what was there.
    let mut history = History::default();
    let mut project = h.state().view.project.clone();
    history.record(before.clone());
    assert!(history.undo(&mut project));
    assert_eq!(project, before);

    // The pasted hits are selected, so a drag moves the whole paste.
    let from = h.state().lane(0, 4000.0);
    let to = h.state().lane(0, 4250.0);
    drag(&mut h, from, to);
    assert_eq!(
        h.state().view.layer.lanes[0].hits,
        [500, 1000, 4250, 4700, 4750]
    );
}

#[test]
fn cut_and_paste_through_the_menu_acts_and_into_a_difficulty() {
    let mut h = harness(Fixture::new(1).diff());
    let base = h.state().view.project.base_events().collect::<Vec<_>>();
    h.state_mut().timeline.selection = [(1, 750)].into();
    {
        let state = h.state_mut();
        let mut intents = Vec::new();
        state
            .timeline
            .cut_selection(&state.view, &mut |intent| intents.push(intent));
        for intent in intents {
            if let Intent::Job(Job::Edit(edit)) = intent {
                state.apply(&edit);
            }
        }
    }
    assert!(!h.state().view.layer.contains(&(1, 750)));
    assert_eq!(h.state().timeline.clipboard_len(), 1);
    h.state_mut().clock.position_ms = 2000.0;
    h.event(Event::Paste(String::new()));
    h.step();
    assert!(h.state().view.layer.contains(&(1, 2000)));
    // In a difficulty a paste is overrides; the base layer is untouched (D-17).
    assert_eq!(
        h.state().view.project.base_events().collect::<Vec<_>>(),
        base
    );
}

#[test]
fn listening_levels_take_a_typed_value() {
    let fixture = Fixture::new(1);
    let levels = hs_session::Levels {
        master: 1.0,
        ..Default::default()
    };
    let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = sent.clone();
    let mut h = Harness::builder()
        .with_size(vec2(900.0, 260.0))
        .build_ui(move |ui| {
            super::mixer::show(
                ui,
                &fixture.view,
                &levels,
                None,
                &fixture.clock,
                true,
                &mut |intent| {
                    if let Intent::Job(Job::Levels(levels)) = intent {
                        log.borrow_mut().push(levels);
                    }
                },
            );
        });
    h.step();
    let value = h.get_by_label("Master volume value").rect().center();
    h.hover_at(value);
    h.event(Event::PointerButton {
        pos: value,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerButton {
        pos: value,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(3);
    // The old value is selected, so typing replaces it.
    h.event(Event::Text("45%".into()));
    h.step();
    h.key_press(Key::Enter);
    h.run_steps(3);
    let sent = sent.borrow();
    assert_eq!(sent.len(), 1, "one level change: {sent:?}");
    assert!((sent[0].master - 0.45).abs() < 1e-6, "{:?}", sent[0]);
    assert_eq!(sent[0].music, levels.music);
}

#[test]
fn objects_ticks_and_difficulty_removal_stay_separate_from_base() {
    let mut h = harness(Fixture::new(1).diff());
    let p = pos2(h.state().x(750.0), h.state().geometry().objects.center().y);
    click(&mut h, p, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().timeline.object, Some(Target::Tick(0)));
    assert_eq!(h.state().clock.position_ms, 750.0);
    let edge = pos2(h.state().x(1500.0), h.state().geometry().objects.center().y);
    click(&mut h, edge, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().timeline.object, Some(Target::Edge(2)));
    assert_eq!(h.state().view.layer.issues.len(), 2);
    let hit = h.state().lane(0, 500.0);
    click(&mut h, hit, PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(h.state().view.layer.lanes[0].removed, [500]);
    assert!(h.state().view.project.channels[0].events.contains(&500));
}

#[test]
fn follow_fit_priority_and_sample_drop() {
    let mut h = harness(Fixture::new(3));
    h.state_mut().clock.position_ms = 90_000.0;
    h.state_mut().clock.playing = true;
    h.step();
    assert!(
        h.state()
            .geometry()
            .plot
            .x_range()
            .contains(h.state().x(90_000.0))
    );
    {
        let state = h.state_mut();
        state.timeline.fit(&state.view, 0.0, true);
        state.clock.position_ms = 0.0;
    }
    h.step();
    assert!(
        h.state()
            .geometry()
            .plot
            .x_range()
            .contains(h.state().x(180_000.0))
    );
    let g = h.state().geometry();
    let first = h.get_by_label("Reorder Track 1").rect().center();
    let second = pos2(g.rail.center().x, g.row(1, 0.0).center().y);
    drag(&mut h, first, second);
    assert_eq!(h.state().view.project.channels[1].id, 1);
    let path = PathBuf::from("synthetic.wav");
    egui::DragAndDrop::set_payload(&h.ctx, DragItem::Sample(path.clone()));
    h.hover_at(second);
    pointer(&h, second, PointerButton::Primary, false, Modifiers::NONE);
    h.step();
    assert_eq!(h.state().imported, [(Some(1), path)]);
}

#[test]
fn rail_name_mute_and_solo_edit_the_selected_channel() {
    let mut h = harness(Fixture::new(2));
    let name = h.get_by_label("Track 1").rect().center();
    click(&mut h, name, PointerButton::Primary, Modifiers::NONE);
    click(&mut h, name, PointerButton::Primary, Modifiers::NONE);
    h.step();
    h.key_press_modifiers(Modifiers::CTRL | Modifiers::COMMAND, Key::A);
    h.event(Event::Text("Kick".into()));
    h.key_press(Key::Enter);
    h.step();
    assert_eq!(h.state().view.project.channels[0].name, "Kick");
    let mute = h.get_by_label("Mute Kick").rect().center();
    click(&mut h, mute, PointerButton::Primary, Modifiers::NONE);
    let solo = h.get_by_label("Solo Kick").rect().center();
    click(&mut h, solo, PointerButton::Primary, Modifiers::NONE);
    assert!(h.state().view.project.channels[0].muted);
    assert!(h.state().view.project.channels[0].solo);
    // FL's shortcut: a right-click on the mute light toggles solo.
    click(&mut h, mute, PointerButton::Secondary, Modifiers::NONE);
    assert!(!h.state().view.project.channels[0].solo);
    assert!(h.state().view.project.channels[0].muted);
}

#[test]
fn rack_knob_sets_the_channel_volume_as_one_edit() {
    let mut h = harness(Fixture::new(2));
    let knob = h.get_by_label("Volume of Track 2").rect().center();
    drag(&mut h, knob, knob + vec2(0.0, 50.0));
    let volume = h.state().view.project.channels[1].volume;
    assert!(
        (volume - 0.7).abs() < 0.011,
        "50 points down is 30 %, got {volume}"
    );
    assert_eq!(h.state().view.project.channels[0].volume, 1.0);
}

#[test]
fn paint_tool_fills_every_snap_step_of_one_channel() {
    let mut h = harness(Fixture::new(2));
    h.state_mut().timeline.tool = Tool::Paint;
    let from = h.state().lane(1, 2000.0);
    // The stroke wanders onto the other row; a paint stroke stays on the row it began on.
    let to = h.state().lane(0, 2500.0);
    drag(&mut h, from, to);
    let hits = &h.state().view.layer.lanes[1].hits;
    for time in [2000, 2125, 2250, 2375, 2500] {
        assert!(hits.contains(&time), "missing {time} in {hits:?}");
    }
    assert_eq!(h.state().view.layer.lanes[0].hits, [500, 1000]);
    // Painting over existing hits adds nothing twice.
    let again = h.state().lane(1, 2250.0);
    drag(&mut h, from, again);
    assert_eq!(h.state().view.layer.lanes[1].hits.len(), 7);
}

#[test]
fn delete_tool_deletes_the_hits_it_is_dragged_across() {
    let mut h = harness(Fixture::new(2));
    h.state_mut().timeline.tool = Tool::Delete;
    let from = h.state().lane(0, 400.0);
    let to = h.state().lane(0, 1100.0);
    drag(&mut h, from, to);
    assert!(h.state().view.layer.lanes[0].hits.is_empty());
    assert_eq!(h.state().view.layer.lanes[1].hits, [500, 1125]);
    // A click on empty space with the Delete tool places nothing.
    let empty = h.state().lane(1, 3000.0);
    click(&mut h, empty, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(h.state().view.layer.lanes[1].hits, [500, 1125]);
}

#[test]
fn select_tool_never_places_and_a_click_clears_the_selection() {
    let mut h = harness(Fixture::new(2));
    h.state_mut().timeline.tool = Tool::Select;
    let start = h.state().lane(0, 300.0);
    let end = h.state().lane(1, 1300.0);
    drag(&mut h, start, end);
    assert_eq!(h.state().timeline.selection.len(), 4);
    let empty = h.state().lane(0, 3000.0);
    click(&mut h, empty, PointerButton::Primary, Modifiers::NONE);
    assert!(h.state().timeline.selection.is_empty());
    assert_eq!(h.state().view.layer.lanes[0].hits, [500, 1000]);
}

#[test]
fn step_pads_appear_only_when_a_step_is_wide_enough() {
    let mut h = harness(Fixture::new(1));
    let view = &h.state().view;
    // 120 BPM at 1/4 is 125 ms a step: 21 points at the default zoom.
    assert!(h.state().timeline.pad_width(view, 0.0).is_some());
    {
        let state = h.state_mut();
        state.timeline.fit(&state.view, 0.0, true);
    }
    h.step();
    assert!(h.state().timeline.pad_width(&h.state().view, 0.0).is_none());
}

#[test]
fn overview_scrolls_the_timeline_without_seeking() {
    let mut h = harness(Fixture::new(1));
    let bar = h.state().geometry().hscroll;
    let from = pos2(bar.left() + 4.0, bar.center().y);
    drag(&mut h, from, pos2(bar.center().x, bar.center().y));
    assert!(h.state().timeline.viewport.left > 1000.0);
    assert_eq!(h.state().clock.position_ms, 0.0);
    // Paused, the view stays where it was scrolled to.
    h.step();
    assert!(h.state().timeline.viewport.left > 1000.0);
}

fn seeks(h: &Harness<'_, Fixture>) -> usize {
    h.state()
        .transport
        .iter()
        .filter(|action| matches!(action, Transport::Seek { .. }))
        .count()
}

#[test]
fn holding_the_playhead_while_playing_pauses_instead_of_looping() {
    let mut h = harness(Fixture::new(1));
    h.state_mut().clock.playing = true;
    let ruler = h.state().geometry().ruler;
    let at = pos2(ruler.center().x, ruler.center().y);
    h.hover_at(at);
    pointer(&h, at, PointerButton::Primary, true, Modifiers::NONE);
    h.step();
    assert!(matches!(
        h.state().transport[..],
        [Transport::Pause, Transport::Seek { .. }]
    ));
    assert!(!h.state().clock.playing);
    // The audio callback may still advance the clock for a frame or two: a still pointer
    // must not seek back to itself every frame (the buzz).
    for _ in 0..5 {
        h.state_mut().clock.position_ms += 8.0;
        h.step();
    }
    assert_eq!(seeks(&h), 1);
    pointer(&h, at, PointerButton::Primary, false, Modifiers::NONE);
    h.step();
    assert!(matches!(h.state().transport.last(), Some(Transport::Play)));
    assert!(h.state().clock.playing);
}

#[test]
fn clicking_the_ruler_while_paused_only_seeks() {
    let mut h = harness(Fixture::new(1));
    let ruler = h.state().geometry().ruler;
    click(
        &mut h,
        ruler.center(),
        PointerButton::Primary,
        Modifiers::NONE,
    );
    assert!(matches!(h.state().transport[..], [Transport::Seek { .. }]));
    assert!(!h.state().clock.playing);
}

#[test]
fn dragging_the_playhead_into_an_edge_scrolls_the_view() {
    let mut h = harness(Fixture::new(1));
    let ruler = h.state().geometry().ruler;
    let plot = h.state().geometry().plot;
    let y = ruler.center().y;
    h.hover_at(pos2(plot.center().x, y));
    pointer(
        &h,
        pos2(plot.center().x, y),
        PointerButton::Primary,
        true,
        Modifiers::NONE,
    );
    h.step();
    let right = pos2(plot.right() + 40.0, y);
    h.hover_at(right);
    for _ in 0..30 {
        h.step();
    }
    let scrolled = h.state().timeline.viewport.left;
    assert!(scrolled > 500.0, "the view scrolled right: {scrolled}");
    // The playhead stays pinned to the right edge while the view moves under it.
    let edge = h.state().timeline.viewport.time(plot, plot.right());
    assert!((h.state().clock.position_ms - edge).abs() < 50.0);
    let left = pos2(plot.left() - 40.0, y);
    h.hover_at(left);
    for _ in 0..10 {
        h.step();
    }
    assert!(h.state().timeline.viewport.left < scrolled);
    pointer(&h, left, PointerButton::Primary, false, Modifiers::NONE);
    h.step();
    // Released, the view no longer scrolls.
    let still = h.state().timeline.viewport.left;
    h.step();
    assert_eq!(h.state().timeline.viewport.left, still);
}

#[test]
#[ignore = "run in release mode to measure the 144 Hz frame budget"]
fn fit_to_song_frame_budget() {
    let mut fixture = Fixture::new(30);
    for (row, channel) in fixture.view.project.channels.iter_mut().enumerate() {
        channel.events = (0..1440).map(|i| i * 125 + row as i32 * 3).collect();
    }
    fixture.view.layer = Layer::new(&fixture.view.project, None, BeatGrid::default(), None, None);
    fixture.clock.playing = true;
    // Fit to song draws every hit as a thin marker; the default zoom draws step pads (D-61).
    for fit in [true, false] {
        frame_budget(&mut fixture, fit);
    }
}

fn frame_budget(fixture: &mut Fixture, fit: bool) {
    fixture.timeline = Timeline::default();
    let ctx = Context::default();
    theme::install(&ctx);
    let mut samples = Vec::new();
    let size = vec2(1280.0, 720.0);
    for i in 0..300 {
        let start = std::time::Instant::now();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(f64::from(i) / 144.0),
            ..Default::default()
        };
        fixture.clock.position_ms = 70_000.0 + f64::from(i) * 1000.0 / 144.0;
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| {
                    if i == 1 && fit {
                        fixture.timeline.fit(&fixture.view, 0.0, true);
                    }
                    fixture.draw(ui);
                })
                .inner
        });
        let _ = std::hint::black_box(ctx.tessellate(output.shapes, output.pixels_per_point));
        if i >= 30 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(f64::total_cmp);
    let p95 = samples[samples.len() * 95 / 100];
    eprintln!(
        "180 s, 30 channels, 43,200 hits, 1280×720, {} and playing. CPU p50 {:.2} ms, p95 {p95:.2} ms",
        if fit { "fit" } else { "step pads" },
        samples[samples.len() / 2]
    );
    assert!(p95 < 1000.0 / 144.0, "CPU p95 exceeded the 144 Hz budget");
}
