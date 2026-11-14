//! Hardware-style controls (D-63) and the hint bar (D-61) shared by the workspace.
//!
//! Everything here is presentation only: a control reports what the user asked for and
//! the caller turns that into an `Intent`, so no behaviour hides in a widget (D-48).

use crate::theme as t;
use eframe::egui::{
    self, Align, Align2, Button, Color32, Context, FontId, Frame, Id, Key, Label, Layout, Margin,
    Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType,
    epaint::PathShape, pos2, vec2,
};

/// What the hint bar says about the thing under the pointer: a plain sentence, then the
/// keys or mouse gestures that act on it.
#[derive(Clone, Default, PartialEq)]
pub struct Hint {
    pub text: String,
    pub keys: Vec<(String, String)>,
}

impl Hint {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            keys: Vec::new(),
        }
    }

    pub fn key(mut self, key: impl Into<String>, action: impl Into<String>) -> Self {
        self.keys.push((key.into(), action.into()));
        self
    }
}

fn hint_ids() -> (Id, Id) {
    (Id::new("hint-next"), Id::new("hint-now"))
}

/// Offers a hint for the hint bar. The last widget to call this in a frame wins.
pub fn hint(ctx: &Context, hint: Hint) {
    let (next, now) = hint_ids();
    let changed = ctx.data_mut(|data| {
        let changed = data.get_temp::<Hint>(now).as_ref() != Some(&hint);
        data.insert_temp(next, hint);
        changed
    });
    if changed {
        ctx.request_repaint();
    }
}

/// Promotes the hint gathered during the previous frame. Call once, before any widget.
pub fn begin_hints(ctx: &Context) -> Hint {
    let (next, now) = hint_ids();
    ctx.data_mut(|data| {
        let hint = data.remove_temp::<Hint>(next).unwrap_or_default();
        data.insert_temp(now, hint.clone());
        hint
    })
}

/// Clears a stale hint once the pointer has left every widget. Call once, after all widgets.
pub fn end_hints(ctx: &Context) {
    let (next, now) = hint_ids();
    let stale = ctx.data(|data| {
        data.get_temp::<Hint>(next).is_none()
            && data
                .get_temp::<Hint>(now)
                .is_some_and(|hint| hint != Hint::default())
    });
    if stale {
        ctx.request_repaint();
    }
}

/// Gives a control its tooltip and its hint-bar line in one call.
pub fn describe(response: Response, text: &str, shortcut: &str) -> Response {
    if response.contains_pointer() {
        let mut line = Hint::new(text);
        if !shortcut.is_empty() {
            line = line.key(shortcut, "");
        }
        hint(&response.ctx, line);
    }
    if shortcut.is_empty() {
        response.on_hover_text(text)
    } else {
        response.on_hover_text(format!("{text}  ({shortcut})"))
    }
}

pub fn keycap(ui: &mut Ui, key: &str) {
    Frame::new()
        .fill(t::RAISED)
        .stroke(t::stroke(t::EDGE))
        .corner_radius(t::SMALL as u8)
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.add(Label::new(t::meta(key, t::TEXT)).selectable(false));
        });
}

pub fn show_hint(ui: &mut Ui, hint: &Hint) {
    ui.spacing_mut().item_spacing.x = t::SMALL + 2.0;
    if !hint.text.is_empty() {
        ui.add(
            Label::new(RichText::new(&hint.text).color(t::TEXT))
                .selectable(false)
                .truncate(),
        );
        ui.add_space(t::SMALL);
    }
    for (key, action) in &hint.keys {
        if ui.available_width() < 40.0 {
            break;
        }
        keycap(ui, key);
        if !action.is_empty() {
            ui.add(Label::new(t::muted(action)).selectable(false));
            ui.add_space(t::SMALL);
        }
    }
}

/// A round status light. `glow` (0–1) adds the flash used when a channel's hit plays.
pub fn led(ui: &Ui, rect: Rect, id: Id, on: bool, glow: f32, color: Color32) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    let center = rect.center();
    let radius = 4.5;
    let p = ui.painter();
    p.circle_filled(center, radius + 1.5, t::INSET);
    if on {
        if glow > 0.0 {
            p.circle_filled(
                center,
                radius + 2.0 + glow * 3.0,
                color.gamma_multiply(0.3 * glow),
            );
        }
        p.circle_filled(center, radius, color);
        p.circle_filled(
            center - vec2(1.3, 1.3),
            1.3,
            Color32::from_white_alpha(if glow > 0.0 { 190 } else { 90 }),
        );
    } else {
        p.circle_filled(center, radius, t::LED_OFF);
    }
    if response.hovered() {
        p.circle_stroke(center, radius + 2.0, t::stroke(t::EDGE_STRONG));
    }
    response
}

#[derive(Clone, Copy)]
struct Pending {
    value: f32,
    /// Infinite while the pointer is down. Afterwards the value is held until the
    /// worker's edit comes back, so the control never snaps to the old value.
    until: f64,
}

const SETTLE_SECONDS: f64 = 0.5;

fn settle(ui: &Ui, id: Id, current: f32, mut pending: Option<Pending>) -> f32 {
    let now = ui.input(|input| input.time);
    if let Some(held) = pending
        && held.until.is_finite()
    {
        if now > held.until || (held.value - current).abs() < 0.0005 {
            pending = None;
        } else {
            ui.ctx().request_repaint();
        }
    }
    ui.data_mut(|data| match pending {
        Some(value) => data.insert_temp(id, value),
        None => data.remove::<Pending>(id),
    });
    pending.map_or(current, |held| held.value)
}

/// A rotary control for a 0–1 value. Drag up or right to raise it, hold Shift for fine
/// steps, double-click to reset. Returns the value to draw and, on release, the value to
/// commit as one edit (D-47: one gesture, one undo step).
pub fn knob(
    ui: &Ui,
    rect: Rect,
    id: Id,
    current: f32,
    reset: f32,
    color: Color32,
) -> (Response, f32, Option<f32>) {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let now = ui.input(|input| input.time);
    let mut pending = ui.data(|data| data.get_temp::<Pending>(id));
    let mut commit = None;
    if response.double_clicked() {
        commit = Some(reset);
        pending = Some(Pending {
            value: reset,
            until: now + SETTLE_SECONDS,
        });
    } else if response.dragged() {
        let from = pending
            .filter(|held| held.until.is_infinite())
            .map_or(current, |held| held.value);
        let delta = response.drag_delta();
        let fine = if ui.input(|input| input.modifiers.shift) {
            0.25
        } else {
            1.0
        };
        pending = Some(Pending {
            value: (from + (delta.x - delta.y) * 0.006 * fine).clamp(0.0, 1.0),
            until: f64::INFINITY,
        });
    } else if response.drag_stopped()
        && let Some(held) = pending
    {
        let value = (held.value * 100.0).round() / 100.0;
        if (value - current).abs() > 0.0005 {
            commit = Some(value);
        }
        pending = Some(Pending {
            value,
            until: now + SETTLE_SECONDS,
        });
    }
    let shown = settle(ui, id, current, pending);

    let center = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0 - 1.0;
    let angle = |value: f32| (135.0 + 270.0 * value).to_radians();
    let at = |value: f32, radius: f32| {
        let a = angle(value);
        center + vec2(a.cos(), a.sin()) * radius
    };
    let arc = |until: f32| -> Vec<Pos2> {
        let steps = (until * 24.0).ceil().max(1.0) as usize;
        (0..=steps)
            .map(|i| at(until * i as f32 / steps as f32, radius))
            .collect()
    };
    let p = ui.painter();
    p.add(Shape::Path(PathShape::line(
        arc(1.0),
        Stroke::new(2.0_f32, t::LED_OFF),
    )));
    if shown > 0.005 {
        p.add(Shape::Path(PathShape::line(
            arc(shown),
            Stroke::new(2.0_f32, color),
        )));
    }
    let body = if response.hovered() || response.dragged() {
        t::HOVER
    } else {
        t::RAISED
    };
    p.circle(center, radius - 3.0, body, t::stroke(t::EDGE_STRONG));
    p.line_segment(
        [at(shown, radius * 0.25), at(shown, radius - 4.0)],
        Stroke::new(1.5_f32, t::TEXT),
    );
    (response, shown, commit)
}

/// A vertical fader for a 0–1 value: click or drag to set, double-click to reset.
pub fn fader(
    ui: &Ui,
    rect: Rect,
    id: Id,
    current: f32,
    reset: f32,
    color: Color32,
) -> (Response, f32, Option<f32>) {
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let now = ui.input(|input| input.time);
    let track = rect.shrink2(vec2(0.0, 8.0));
    let mut pending = ui.data(|data| data.get_temp::<Pending>(id));
    let mut commit = None;
    if response.double_clicked() {
        commit = Some(reset);
        pending = Some(Pending {
            value: reset,
            until: now + SETTLE_SECONDS,
        });
    } else if response.dragged()
        && let Some(point) = response.interact_pointer_pos()
    {
        let value = 1.0 - (point.y - track.top()) / track.height().max(1.0);
        pending = Some(Pending {
            value: (value.clamp(0.0, 1.0) * 100.0).round() / 100.0,
            until: f64::INFINITY,
        });
    } else if response.drag_stopped()
        && let Some(held) = pending
    {
        if (held.value - current).abs() > 0.0005 {
            commit = Some(held.value);
        }
        pending = Some(Pending {
            value: held.value,
            until: now + SETTLE_SECONDS,
        });
    }
    let shown = settle(ui, id, current, pending);

    let p = ui.painter();
    let x = rect.center().x;
    let y = track.bottom() - track.height() * shown;
    p.rect_filled(
        Rect::from_min_max(pos2(x - 2.0, track.top()), pos2(x + 2.0, track.bottom())),
        2,
        t::INSET,
    );
    p.rect_filled(
        Rect::from_min_max(pos2(x - 2.0, y), pos2(x + 2.0, track.bottom())),
        2,
        color.gamma_multiply(0.75),
    );
    for mark in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let y = track.bottom() - track.height() * mark;
        p.hline((x - 11.0)..=(x - 6.0), y, t::stroke(t::EDGE));
    }
    let cap = Rect::from_center_size(pos2(x, y), vec2(22.0, 11.0));
    let fill = if response.hovered() || response.dragged() {
        t::HOVER
    } else {
        t::RAISED
    };
    p.rect(cap, 3, fill, t::stroke(t::EDGE_STRONG), StrokeKind::Inside);
    p.hline(cap.shrink2(vec2(4.0, 0.0)).x_range(), y, t::stroke(t::TEXT));
    (response, shown, commit)
}

/// What a percent readout is being typed into: the draft, and whether the field still
/// has to take focus (it appears one frame after the click).
#[derive(Clone)]
struct Typing {
    draft: String,
    focus: bool,
}

/// A percent readout under a fader that becomes a text field when clicked (D-87).
/// Enter or clicking away sets the typed value, Esc keeps the old one. Returns the
/// readout's response and the value typed, from 0 to 1.
pub fn percent_entry(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    shown: f32,
    color: Color32,
) -> (Response, Option<f32>) {
    let field = id.with("field");
    let font = FontId::monospace(t::META_SIZE);
    if let Some(mut typing) = ui.data(|data| data.get_temp::<Typing>(id)) {
        let mut output = egui::TextEdit::singleline(&mut typing.draft)
            .id(field)
            .font(font)
            .horizontal_align(Align::Center)
            .margin(Margin::symmetric(2, 0))
            .desired_width(rect.width())
            .char_limit(5)
            .show(&mut ui.new_child(egui::UiBuilder::new().max_rect(rect)));
        let response = output.response.clone();
        if typing.focus {
            typing.focus = false;
            response.request_focus();
            // Select the old value, so typing replaces it.
            let all = egui::text::CCursorRange::select_all(&output.galley);
            output.state.cursor.set_char_range(Some(all));
            output.state.store(ui.ctx(), field);
            ui.data_mut(|data| data.insert_temp(id, typing));
            return (response, None);
        }
        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        if escape || !response.has_focus() {
            ui.data_mut(|data| data.remove::<Typing>(id));
            let typed = typing
                .draft
                .trim()
                .trim_end_matches('%')
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && !escape)
                .map(|v| (v.clamp(0.0, 100.0)).round() / 100.0);
            return (response, typed);
        }
        ui.data_mut(|data| data.insert_temp(id, typing));
        return (response, None);
    }
    let response = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    if response.hovered() {
        p.rect_filled(rect, 3, t::HOVER);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("{:.0}%", shown * 100.0),
        font,
        color,
    );
    if response.clicked() {
        ui.data_mut(|data| {
            data.insert_temp(
                id,
                Typing {
                    draft: format!("{:.0}", shown * 100.0),
                    focus: true,
                },
            );
        });
        ui.ctx().request_repaint();
    }
    (response, None)
}

pub struct Segment<'a, T> {
    pub value: T,
    pub icon: &'a str,
    pub label: &'a str,
    pub hint: &'a str,
    pub shortcut: &'a str,
    pub enabled: bool,
}

/// A row of mutually exclusive buttons in one recessed well. Returns the newly picked value.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    current: T,
    segments: &[Segment<'_, T>],
    labels: bool,
) -> Option<T> {
    let mut picked = None;
    Frame::new()
        .fill(t::INSET)
        .stroke(t::stroke(t::EDGE))
        .corner_radius(t::CONTROL_RADIUS + 1)
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().button_padding = vec2(if labels { 8.0 } else { 6.0 }, 3.0);
                for segment in segments {
                    let selected = segment.value == current;
                    let colour = if selected { t::ACCENT } else { t::MUTED };
                    let button = match (segment.icon.is_empty(), labels || segment.icon.is_empty())
                    {
                        (true, _) => {
                            Button::selectable(selected, RichText::new(segment.label).color(colour))
                        }
                        (false, true) => Button::selectable(
                            selected,
                            (
                                t::icon(segment.icon).color(colour),
                                RichText::new(segment.label).color(colour),
                            ),
                        ),
                        (false, false) => {
                            Button::selectable(selected, t::icon(segment.icon).color(colour))
                        }
                    };
                    let response = ui.add_enabled(
                        segment.enabled,
                        button
                            .frame_when_inactive(false)
                            .min_size(vec2(t::CONTROL_HEIGHT, t::CONTROL_HEIGHT - 2.0)),
                    );
                    response.widget_info(|| {
                        WidgetInfo::selected(
                            WidgetType::Button,
                            segment.enabled,
                            selected,
                            segment.label,
                        )
                    });
                    if describe(response, segment.hint, segment.shortcut).clicked() && !selected {
                        picked = Some(segment.value);
                    }
                }
            });
        });
    picked
}

/// A titled block of a side panel. Sections are separated by one hairline.
pub fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(t::SMALL);
    ui.label(t::display(title, t::TITLE_SIZE));
    ui.add_space(2.0);
    add(ui);
    ui.add_space(t::SMALL);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, t::stroke(t::EDGE));
}

/// One form row: a fixed-width label on the left, the control on the right.
pub fn field<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(
            vec2(t::FIELD_LABEL_WIDTH, t::CONTROL_HEIGHT),
            Sense::hover(),
        );
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(t::BODY_SIZE),
            t::MUTED,
        );
        ui.with_layout(Layout::left_to_right(Align::Center), add)
            .inner
    })
    .inner
}

/// A short explanation under a control or section title.
pub fn help(ui: &mut Ui, text: &str) {
    ui.add(Label::new(RichText::new(text).size(12.0).color(t::DIM)).wrap());
}

/// A small static tag, such as a count or a state.
pub fn tag(ui: &mut Ui, text: impl Into<String>, colour: Color32) -> Response {
    Frame::new()
        .fill(colour.gamma_multiply(0.14))
        .corner_radius(t::SMALL as u8)
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.add(Label::new(t::meta(text, colour)).selectable(false));
        })
        .response
}

/// A toolbar button: an icon with an optional text label, lit while `selected`.
pub fn tool_button(
    ui: &mut Ui,
    glyph: &str,
    label: &str,
    show_label: bool,
    selected: Option<bool>,
) -> Response {
    let on = selected == Some(true);
    let colour = if on { t::ACCENT } else { t::TEXT };
    let icon = t::icon(glyph).color(colour);
    let button = match (selected, show_label) {
        (Some(on), true) => Button::selectable(on, (icon, RichText::new(label).color(colour))),
        (Some(on), false) => Button::selectable(on, icon),
        (None, true) => Button::new((icon, label)),
        (None, false) => Button::new(icon),
    };
    let response = ui.add(button.min_size(vec2(t::CONTROL_HEIGHT + 2.0, t::CONTROL_HEIGHT + 2.0)));
    response.widget_info(|| match selected {
        Some(on) => WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), on, label),
        None => WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label),
    });
    response
}

/// A vertical hairline between toolbar groups.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, t::CONTROL_HEIGHT), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), t::stroke(t::EDGE));
}
