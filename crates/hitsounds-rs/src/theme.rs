//! Shared colour, type, spacing, elevation and motion tokens (D-55, D-60, D-63).

use eframe::egui::{
    self, Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, IntoAtoms, Margin, Response, RichText, Stroke, TextStyle, Ui, Vec2, WidgetInfo,
    WidgetType, vec2,
};

// D-60: one graphite ramp for every surface, one accent, magenta only for difficulty context.
pub const BASE: Color32 = Color32::from_rgb(20, 20, 21);
pub const CHROME: Color32 = Color32::from_rgb(15, 15, 16);
pub const SURFACE: Color32 = Color32::from_rgb(25, 25, 27);
pub const RAISED: Color32 = Color32::from_rgb(35, 35, 37);
pub const HOVER: Color32 = Color32::from_rgb(44, 44, 47);
pub const STAGE: Color32 = Color32::from_rgb(23, 23, 25);
pub const EDGE: Color32 = Color32::from_rgb(48, 48, 50);
pub const EDGE_STRONG: Color32 = Color32::from_rgb(70, 70, 74);
pub const GRID: Color32 = Color32::from_rgb(40, 40, 43);
pub const TEXT: Color32 = Color32::from_rgb(237, 237, 237);
pub const MUTED: Color32 = Color32::from_rgb(161, 161, 166);
pub const DIM: Color32 = Color32::from_rgb(133, 133, 139);
pub const ACCENT: Color32 = Color32::from_rgb(77, 214, 200);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(120, 228, 216);
pub const ACCENT_PRESS: Color32 = Color32::from_rgb(62, 184, 172);
pub const ACCENT_SURFACE: Color32 = Color32::from_rgb(24, 50, 52);
pub const DIFFICULTY: Color32 = Color32::from_rgb(222, 120, 200);
/// The song waveform: neutral, so the accent marks the Hit volume curve (D-92).
pub const WAVE: Color32 = Color32::from_rgb(104, 110, 118);
pub const WARNING: Color32 = Color32::from_rgb(229, 181, 103);
pub const ERROR: Color32 = Color32::from_rgb(240, 106, 110);
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(178, 46, 52);
pub const SCRIM: Color32 = Color32::from_black_alpha(166);
// D-63: recessed wells (time display, knob tracks) and the step pads of the channel rack.
pub const INSET: Color32 = Color32::from_rgb(10, 10, 11);
pub const PAD: Color32 = Color32::from_rgb(34, 34, 37);
pub const PAD_ALT: Color32 = Color32::from_rgb(44, 41, 46);
pub const LED_OFF: Color32 = Color32::from_rgb(54, 54, 58);
// D-54: projection colours pair with distinct marks in the objects lane.
pub const MATCHED: Color32 = Color32::from_rgb(185, 231, 118);
pub const FALLBACK: Color32 = WARNING;
pub const MUTED_HIT: Color32 = Color32::from_rgb(160, 160, 168);
pub const EMPTY_HIT: Color32 = Color32::from_rgb(255, 128, 109);
pub const TICK_HIT: Color32 = Color32::from_rgb(157, 174, 255);
pub const ORPHAN_HIT: Color32 = Color32::from_rgb(255, 174, 130);

pub const HAIRLINE: f32 = 1.0;
pub const SMALL: f32 = 4.0;
pub const GAP: f32 = 8.0;
pub const SPACE: f32 = 12.0;
pub const GUTTER: f32 = 20.0;
pub const SECTION: f32 = 28.0;
pub const PAGE_MARGIN: f32 = 32.0;
pub const PAGE_WIDTH: f32 = 1120.0;
pub const TWO_COLUMN_WIDTH: f32 = 820.0;
pub const PANEL_RADIUS: u8 = 10;
pub const CARD_RADIUS: u8 = 8;
pub const CONTROL_RADIUS: u8 = 5;
pub const CONTROL_HEIGHT: f32 = 26.0;
pub const TITLE_HEIGHT: f32 = 34.0;
pub const CAPTION_WIDTH: f32 = 44.0;
pub const STATUS_HEIGHT: f32 = 24.0;
pub const HERO_HEIGHT: f32 = 220.0;
pub const MODAL_WIDTH: f32 = 540.0;
pub const MENU_WIDTH: f32 = 248.0;
pub const SETUP_ACTION_WIDTH: f32 = 128.0;
pub const MIN_ROW_LABEL_WIDTH: f32 = 100.0;
pub const TOAST_WIDTH: f32 = 420.0;
pub const ITEM_HEIGHT: f32 = 48.0;
pub const EXPORT_OPTIONS_WIDTH: f32 = 320.0;
pub const DIFF_VIEW_HEIGHT: f32 = 216.0;
pub const DIFF_LINE_HEIGHT: f32 = 20.0;
pub const EDITOR_PANEL_MIN: f32 = 220.0;
pub const LIBRARY_WIDTH: f32 = 248.0;
pub const INSPECTOR_WIDTH: f32 = 300.0;
pub const ROW_HEIGHT: f32 = 30.0;
pub const RAIL_WIDTH: f32 = 304.0;
pub const COMPACT_RAIL_WIDTH: f32 = 236.0;
pub const RULER_HEIGHT: f32 = 24.0;
pub const WAVE_HEIGHT: f32 = 48.0;
/// The waveform strip with the waveform hidden: just the Song / Stem switch (D-92).
pub const WAVE_HIDDEN_HEIGHT: f32 = 30.0;
pub const VOLUME_HEIGHT: f32 = 52.0;
pub const OBJECTS_HEIGHT: f32 = 28.0;
/// The gap between a Hit volume point and its value bubble or entry field (D-93).
pub const BUBBLE_OFFSET: f32 = 10.0;
/// The height of the field a double-click on a volume point opens (D-93).
pub const ENTRY_HEIGHT: f32 = 28.0;
pub const TIMELINE_SCROLLBAR: f32 = 10.0;
pub const HIT_RADIUS: f32 = 7.0;
pub const DRAG_THRESHOLD: f32 = 4.0;
pub const HIT_PULSE_MS: f64 = 150.0;
pub const LAYER_BAR_HEIGHT: f32 = 34.0;
pub const TOOLBAR_HEIGHT: f32 = 46.0;
pub const MIXER_HEIGHT: f32 = 212.0;
pub const STRIP_WIDTH: f32 = 68.0;
/// Step pads appear once one snap step is at least this wide (D-61).
pub const PAD_MIN_STEP: f32 = 10.0;
pub const PAD_MAX_WIDTH: f32 = 30.0;
pub const PAD_INSET: f32 = 4.0;
pub const FIELD_LABEL_WIDTH: f32 = 92.0;
pub const MIN_WIDTH: f32 = 940.0;
pub const MIN_HEIGHT: f32 = 560.0;
pub const INITIAL_WIDTH: f32 = 1440.0;
pub const INITIAL_HEIGHT: f32 = 860.0;

/// Interface scale choices (D-68), on top of the Windows display scale.
pub const UI_SCALES: [f32; 7] = [0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
/// Share of the monitor the scaled minimum window may take, leaving room for the taskbar.
const SCALE_MONITOR_SHARE: f32 = 0.92;

/// The monitor in unscaled points (what the window has at 100 %), if known.
fn monitor_at_one(ctx: &Context) -> Option<Vec2> {
    let monitor = ctx.input(|i| i.viewport().monitor_size)?;
    Some(monitor * ctx.zoom_factor())
}

/// Whether the D-63 minimum window still fits on this monitor at `scale`.
pub fn scale_fits(ctx: &Context, scale: f32) -> bool {
    monitor_at_one(ctx).is_none_or(|m| {
        MIN_WIDTH * scale <= m.x * SCALE_MONITOR_SHARE
            && MIN_HEIGHT * scale <= m.y * SCALE_MONITOR_SHARE
    })
}

/// The next larger (`up`) or smaller fitting scale, for Ctrl+= and Ctrl+-.
pub fn step_scale(ctx: &Context, current: f32, up: bool) -> Option<f32> {
    let fits = |s: &&f32| scale_fits(ctx, **s);
    if up {
        UI_SCALES
            .iter()
            .filter(fits)
            .find(|&&s| s > current + 0.001)
            .copied()
    } else {
        UI_SCALES
            .iter()
            .rev()
            .find(|&&s| s < current - 0.001)
            .copied()
    }
}

/// The minimum window in points at the current scale: D-63's size, capped to the monitor.
pub fn min_window(ctx: &Context) -> Vec2 {
    let min = vec2(MIN_WIDTH, MIN_HEIGHT);
    match ctx.input(|i| i.viewport().monitor_size) {
        Some(monitor) => min.min(monitor * SCALE_MONITOR_SHARE),
        None => min,
    }
}

pub fn scale_label(scale: f32) -> String {
    format!("{:.0}%", scale * 100.0)
}

pub const BODY_SIZE: f32 = 13.0;
pub const META_SIZE: f32 = 11.0;
pub const ICON_SIZE: f32 = 16.0;
pub const HERO_SIZE: f32 = 56.0;
pub const HEADING_SIZE: f32 = 20.0;
pub const TITLE_SIZE: f32 = 14.0;
pub const TIME_SIZE: f32 = 19.0;

pub const ENTER_SECONDS: f64 = 0.28;
pub const FEEDBACK_SECONDS: f32 = 0.12;
pub const TOAST_SECONDS: f64 = 4.5;
pub const SLIDE: f32 = 8.0;
pub const ZOOM_SECONDS: f64 = 0.14;
pub const OPTION_DEBOUNCE: f64 = 0.16;

pub fn channel_default(id: u32) -> &'static str {
    const COLORS: [&str; 7] = [
        "#ED8B8F", "#F2BD76", "#79D6A6", "#7FB9F8", "#C4A1F2", "#E7A8D5", "#9ED4D8",
    ];
    COLORS[id.saturating_sub(1) as usize % COLORS.len()]
}

pub fn channel_color(hex: &str) -> Color32 {
    Color32::from_hex(hex).unwrap_or(DIM)
}

fn display_family() -> FontFamily {
    FontFamily::Name("Display".into())
}

pub fn icon_family() -> FontFamily {
    FontFamily::Name("Icons".into())
}

pub fn install(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    // Inter assigns some of the same private-use codepoints to stylistic alternates.
    // Select the icon family explicitly so text fallback never replaces a symbol.
    fonts.font_data.insert(
        "Phosphor".into(),
        egui_phosphor::Variant::Regular.font_data().into(),
    );
    fonts
        .families
        .insert(icon_family(), vec!["Phosphor".into()]);
    for (family, name, font) in [
        (
            FontFamily::Proportional,
            "Inter",
            include_bytes!("../assets/Inter-Regular.ttf").as_slice(),
        ),
        (
            FontFamily::Monospace,
            "IBM Plex Mono",
            include_bytes!("../assets/IBMPlexMono-Medium.ttf").as_slice(),
        ),
        (
            display_family(),
            "Inter Display",
            include_bytes!("../assets/InterDisplay-Medium.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(font).into());
        let display = family == display_family();
        let fallback = display.then(|| fonts.families[&FontFamily::Proportional].clone());
        let entry = fonts.families.entry(family).or_default();
        if let Some(fallback) = fallback {
            entry.extend(fallback);
        }
        entry.insert(0, name.into());
    }
    ctx.set_fonts(fonts);

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.weak_text_color = Some(DIM);
    visuals.panel_fill = BASE;
    visuals.window_fill = SURFACE;
    visuals.window_stroke = stroke(EDGE);
    visuals.window_corner_radius = CornerRadius::same(PANEL_RADIUS);
    visuals.window_shadow = shadow();
    visuals.popup_shadow = shadow();
    visuals.extreme_bg_color = BASE;
    visuals.faint_bg_color = STAGE;
    visuals.selection.bg_fill = ACCENT_SURFACE;
    visuals.selection.stroke = stroke(ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    for (widget, fill, border, fg) in [
        (&mut visuals.widgets.noninteractive, SURFACE, EDGE, MUTED),
        (&mut visuals.widgets.inactive, RAISED, EDGE, TEXT),
        (&mut visuals.widgets.hovered, HOVER, EDGE_STRONG, TEXT),
        (&mut visuals.widgets.active, RAISED, EDGE_STRONG, TEXT),
        (&mut visuals.widgets.open, RAISED, EDGE_STRONG, TEXT),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = stroke(border);
        widget.fg_stroke = stroke(fg);
        widget.corner_radius = CornerRadius::same(CONTROL_RADIUS);
        widget.expansion = 0.0;
    }
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.animation_time = FEEDBACK_SECONDS;
        style.spacing.item_spacing = vec2(GAP, 6.0);
        style.spacing.button_padding = vec2(10.0, SMALL);
        style.spacing.icon_width = 14.0;
        style.spacing.combo_height = 320.0;
        style.spacing.slider_rail_height = 4.0;
        style.spacing.interact_size.y = CONTROL_HEIGHT;
        style.spacing.menu_margin = Margin::same(GAP as i8);
        style.spacing.window_margin = Margin::same(GUTTER as i8);
        style.interaction.selectable_labels = false;
        style.text_styles.extend([
            (TextStyle::Body, FontId::proportional(BODY_SIZE)),
            (TextStyle::Button, FontId::proportional(BODY_SIZE)),
            (TextStyle::Small, FontId::proportional(META_SIZE)),
            (TextStyle::Monospace, FontId::monospace(META_SIZE)),
            (
                TextStyle::Heading,
                FontId::new(HEADING_SIZE, display_family()),
            ),
        ]);
    });
    // Caption hit-testing and the egui controls use the same per-monitor DPI.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
}

pub fn stroke(colour: Color32) -> Stroke {
    Stroke::new(HAIRLINE, colour)
}

fn shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(80),
    }
}

pub fn card() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(stroke(EDGE))
        .corner_radius(CARD_RADIUS)
        .inner_margin(GUTTER)
}

pub fn overlay() -> Frame {
    card().shadow(shadow())
}

/// The small raised frame of the Hit volume value bubble and its entry field (D-93).
pub fn bubble() -> Frame {
    Frame::new()
        .fill(RAISED)
        .stroke(stroke(EDGE_STRONG))
        .corner_radius(SMALL)
        .inner_margin(Margin::symmetric(4, 3))
        .shadow(shadow())
}

pub fn display(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(FontId::new(size, display_family()))
}

pub fn meta(text: impl Into<String>, colour: Color32) -> RichText {
    RichText::new(text)
        .font(FontId::monospace(META_SIZE))
        .color(colour)
}

pub fn mono_time(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(FontId::monospace(TIME_SIZE))
        .color(ACCENT)
}

/// Small keycaps for a key combination such as `Ctrl+Shift+Z`.
pub fn keycaps(ui: &mut Ui, shortcut: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SMALL;
        for key in shortcut.split('+') {
            Frame::new()
                .fill(RAISED)
                .stroke(stroke(EDGE))
                .corner_radius(SMALL as u8)
                .inner_margin(Margin::symmetric(5, 1))
                .show(ui, |ui| {
                    ui.label(meta(key.trim(), MUTED));
                });
        }
    });
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}

pub fn icon(glyph: &str) -> RichText {
    RichText::new(glyph).font(FontId::new(ICON_SIZE, icon_family()))
}

/// A square icon-only button. Its label is the tooltip and the hint-bar line (D-61).
pub fn icon_button(ui: &mut Ui, glyph: &str, label: &str) -> Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding.x = SMALL;
        let response = ui.add(
            Button::new(icon(glyph))
                .frame_when_inactive(false)
                .min_size(vec2(CONTROL_HEIGHT, CONTROL_HEIGHT)),
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
        crate::widgets::describe(response, label, "")
    })
    .inner
}

pub fn primary<'a>(ui: &mut Ui, content: impl IntoAtoms<'a>) -> Response {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.override_text_color = Some(BASE);
        for (widget, fill, border) in [
            (&mut visuals.widgets.inactive, ACCENT, ACCENT),
            (&mut visuals.widgets.hovered, ACCENT_HOVER, ACCENT_HOVER),
            (&mut visuals.widgets.active, ACCENT_PRESS, ACCENT_PRESS),
        ] {
            widget.weak_bg_fill = fill;
            widget.bg_fill = fill;
            widget.bg_stroke = stroke(border);
            widget.fg_stroke = stroke(BASE);
        }
        ui.button(content)
    })
    .inner
}

pub fn ease_out(progress: f32) -> f32 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}
