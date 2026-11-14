//! Rebindable keyboard shortcuts (D-92).
//!
//! Every shortcut the app reacts to is an [`Action`] with default bindings. The user's
//! changes are stored in `settings.json` as `keybinds` (action id to binding text, an
//! empty list meaning unbound) and installed with [`install`]. Menus, the palette,
//! tooltips and the gesture sheet read their labels from [`label`], so they always show
//! the key that works.
//!
//! A few keys stay fixed: Esc (cancel and close), Ctrl+C / Ctrl+X / Ctrl+V (egui-winit
//! turns them into clipboard events, never key presses), `?` for the gesture sheet, and
//! the About encore.

use eframe::egui::{Event, InputState, Key, Modifiers};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    sync::Mutex,
};

/// Every rebindable command. The id strings are stored in `settings.json`: never rename one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    PlayPause,
    GoToStart,
    ToggleMonitor,
    ToolSelect,
    ToolDraw,
    ToolPaint,
    ToolDelete,
    PlaceHit,
    SelectAll,
    DeleteSelection,
    Undo,
    Redo,
    NextIssue,
    PreviousIssue,
    PanelChannel,
    PanelReview,
    PanelBrowser,
    PanelMixer,
    NewProject,
    OpenProject,
    Save,
    CloseProject,
    Export,
    Palette,
    Settings,
    Help,
    ScaleUp,
    ScaleDown,
    ScaleReset,
}

impl Action {
    /// In the order App settings lists them.
    pub const ALL: [Self; 29] = [
        Self::PlayPause,
        Self::GoToStart,
        Self::ToggleMonitor,
        Self::ToolSelect,
        Self::ToolDraw,
        Self::ToolPaint,
        Self::ToolDelete,
        Self::PlaceHit,
        Self::SelectAll,
        Self::DeleteSelection,
        Self::Undo,
        Self::Redo,
        Self::NextIssue,
        Self::PreviousIssue,
        Self::PanelChannel,
        Self::PanelReview,
        Self::PanelBrowser,
        Self::PanelMixer,
        Self::ScaleUp,
        Self::ScaleDown,
        Self::ScaleReset,
        Self::NewProject,
        Self::OpenProject,
        Self::Save,
        Self::CloseProject,
        Self::Export,
        Self::Palette,
        Self::Settings,
        Self::Help,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::PlayPause => "play_pause",
            Self::GoToStart => "go_to_start",
            Self::ToggleMonitor => "toggle_monitor",
            Self::ToolSelect => "tool_select",
            Self::ToolDraw => "tool_draw",
            Self::ToolPaint => "tool_paint",
            Self::ToolDelete => "tool_delete",
            Self::PlaceHit => "place_hit",
            Self::SelectAll => "select_all",
            Self::DeleteSelection => "delete_selection",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::NextIssue => "next_issue",
            Self::PreviousIssue => "previous_issue",
            Self::PanelChannel => "panel_channel",
            Self::PanelReview => "panel_review",
            Self::PanelBrowser => "panel_browser",
            Self::PanelMixer => "panel_mixer",
            Self::NewProject => "new_project",
            Self::OpenProject => "open_project",
            Self::Save => "save",
            Self::CloseProject => "close_project",
            Self::Export => "export",
            Self::Palette => "palette",
            Self::Settings => "settings",
            Self::Help => "help",
            Self::ScaleUp => "scale_up",
            Self::ScaleDown => "scale_down",
            Self::ScaleReset => "scale_reset",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PlayPause => "Play / pause",
            Self::GoToStart => "Go to the start",
            Self::ToggleMonitor => "Switch song and drum stem",
            Self::ToolSelect => "Select tool",
            Self::ToolDraw => "Draw tool",
            Self::ToolPaint => "Paint tool",
            Self::ToolDelete => "Delete tool",
            Self::PlaceHit => "Place a hit at the playhead",
            Self::SelectAll => "Select all hits",
            Self::DeleteSelection => "Delete the selected hits",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::NextIssue => "Next issue",
            Self::PreviousIssue => "Previous issue",
            Self::PanelChannel => "Channel settings",
            Self::PanelReview => "Difficulty review",
            Self::PanelBrowser => "Browser",
            Self::PanelMixer => "Mixer",
            Self::NewProject => "New project",
            Self::OpenProject => "Open project",
            Self::Save => "Save project",
            Self::CloseProject => "Close project",
            Self::Export => "Review and export",
            Self::Palette => "Command palette",
            Self::Settings => "App settings",
            Self::Help => "Gestures and shortcuts",
            Self::ScaleUp => "Larger interface",
            Self::ScaleDown => "Smaller interface",
            Self::ScaleReset => "Default interface size",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::PlayPause | Self::GoToStart | Self::ToggleMonitor => "Playback",
            Self::ToolSelect | Self::ToolDraw | Self::ToolPaint | Self::ToolDelete => "Tools",
            Self::PlaceHit
            | Self::SelectAll
            | Self::DeleteSelection
            | Self::Undo
            | Self::Redo
            | Self::NextIssue
            | Self::PreviousIssue => "Editing",
            Self::PanelChannel
            | Self::PanelReview
            | Self::PanelBrowser
            | Self::PanelMixer
            | Self::ScaleUp
            | Self::ScaleDown
            | Self::ScaleReset => "View",
            Self::NewProject
            | Self::OpenProject
            | Self::Save
            | Self::CloseProject
            | Self::Export
            | Self::Palette
            | Self::Settings
            | Self::Help => "App",
        }
    }

    pub fn defaults(self) -> Vec<Binding> {
        let plain = Binding::plain;
        let ctrl = Binding::ctrl;
        match self {
            Self::PlayPause => vec![plain(Key::Space)],
            Self::GoToStart => vec![plain(Key::Home)],
            Self::ToggleMonitor => vec![plain(Key::T)],
            // D-86: number keys for the tools.
            Self::ToolSelect => vec![plain(Key::Num1)],
            Self::ToolDraw => vec![plain(Key::Num2)],
            Self::ToolPaint => vec![plain(Key::Num3)],
            Self::ToolDelete => vec![plain(Key::Num4)],
            Self::PlaceHit => vec![plain(Key::Enter)],
            Self::SelectAll => vec![ctrl(Key::A)],
            Self::DeleteSelection => vec![plain(Key::Delete)],
            Self::Undo => vec![ctrl(Key::Z)],
            Self::Redo => vec![ctrl(Key::Z).with_shift(), ctrl(Key::Y)],
            Self::NextIssue => vec![plain(Key::N)],
            Self::PreviousIssue => vec![plain(Key::N).with_shift()],
            // D-61: FL Studio's function keys for the docks.
            Self::PanelChannel => vec![plain(Key::F6)],
            Self::PanelReview => vec![plain(Key::F7)],
            Self::PanelBrowser => vec![plain(Key::F8)],
            Self::PanelMixer => vec![plain(Key::F9)],
            Self::NewProject => vec![ctrl(Key::N)],
            Self::OpenProject => vec![ctrl(Key::O)],
            Self::Save => vec![ctrl(Key::S)],
            Self::CloseProject => vec![ctrl(Key::W)],
            Self::Export => vec![ctrl(Key::E)],
            Self::Palette => vec![ctrl(Key::K)],
            Self::Settings => vec![ctrl(Key::Comma)],
            Self::Help => vec![plain(Key::F1)],
            // D-68: browser-style interface scale keys.
            Self::ScaleUp => vec![ctrl(Key::Equals), ctrl(Key::Plus)],
            Self::ScaleDown => vec![ctrl(Key::Minus)],
            Self::ScaleReset => vec![ctrl(Key::Num0)],
        }
    }

    /// Held keys repeat these; every other action fires once per press.
    fn repeats(self) -> bool {
        matches!(
            self,
            Self::Undo | Self::Redo | Self::NextIssue | Self::PreviousIssue
        )
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }
}

/// One key with its modifiers. Ctrl stands for Ctrl or the command key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Binding {
    pub const fn plain(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    pub const fn ctrl(key: Key) -> Self {
        Self {
            ctrl: true,
            ..Self::plain(key)
        }
    }

    pub const fn with_shift(self) -> Self {
        Self {
            shift: true,
            ..self
        }
    }

    /// The binding a key press makes. Shift is dropped for symbols that need it to be
    /// typed at all, so `Ctrl+Plus` matches however the keyboard produces `+`.
    pub fn from_press(key: Key, modifiers: Modifiers) -> Self {
        Self {
            key,
            ctrl: modifiers.ctrl || modifiers.command,
            shift: modifiers.shift && !shifted_symbol(key),
            alt: modifiers.alt,
        }
    }

    /// Exact modifiers, so `N` and `Shift+N` are different bindings.
    fn matches(self, key: Key, modifiers: Modifiers) -> bool {
        self == Self::from_press(key, modifiers)
    }

    /// Keys that never reach the app as key presses or that it keeps for itself.
    pub fn reserved(self) -> Option<&'static str> {
        let clipboard = (self.ctrl && matches!(self.key, Key::C | Key::X | Key::V | Key::Insert))
            || (self.shift && matches!(self.key, Key::Delete | Key::Insert));
        if clipboard {
            Some("Ctrl+C, Ctrl+X and Ctrl+V stay copy, cut and paste.")
        } else if self.key == Key::Escape {
            Some("Esc always cancels and closes.")
        } else if self.key == Key::Tab {
            Some("Tab moves between controls.")
        } else if self.ctrl && self.shift && self.alt && self.key == Key::H {
            Some("That one is taken.")
        } else {
            None
        }
    }

    /// The form stored in `settings.json`, such as `Ctrl+Shift+Z` or `Ctrl+Plus`.
    pub fn text(self) -> String {
        self.join(self.key.name())
    }

    /// The form shown on screen, such as `Ctrl+=`.
    pub fn display(self) -> String {
        self.join(display_key(self.key))
    }

    fn join(self, key: &str) -> String {
        let mut text = String::new();
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.shift, "Shift+"),
            (self.alt, "Alt+"),
        ] {
            if on {
                text.push_str(name);
            }
        }
        text.push_str(key);
        text
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let key = Key::from_name(parts.pop()?)?;
        let mut binding = Self::plain(key);
        for part in parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "command" => binding.ctrl = true,
                "shift" => binding.shift = true,
                "alt" => binding.alt = true,
                _ => return None,
            }
        }
        binding.shift &= !shifted_symbol(key);
        Some(binding)
    }
}

fn shifted_symbol(key: Key) -> bool {
    matches!(
        key,
        Key::Plus
            | Key::Questionmark
            | Key::Exclamationmark
            | Key::Colon
            | Key::Pipe
            | Key::OpenCurlyBracket
            | Key::CloseCurlyBracket
    )
}

/// Key names as the menus have always shown them. `+` is spelled out because it joins
/// the parts, and arrows are words because the UI fonts lack the arrow glyphs.
fn display_key(key: Key) -> &'static str {
    match key {
        Key::Plus | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp => key.name(),
        Key::Minus => "-",
        _ => key.symbol_or_name(),
    }
}

/// Every action's current bindings.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    bindings: BTreeMap<Action, Vec<Binding>>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            bindings: Action::ALL
                .into_iter()
                .map(|action| (action, action.defaults()))
                .collect(),
        }
    }
}

impl Keymap {
    /// Defaults with the stored changes applied. Unknown ids and unreadable bindings are
    /// skipped, so a settings file from a newer version still loads.
    pub fn from_overrides(overrides: &BTreeMap<String, Vec<String>>) -> Self {
        let mut map = Self::default();
        for (id, texts) in overrides {
            if let Some(action) = Action::from_id(id) {
                let bindings = texts.iter().filter_map(|t| Binding::parse(t)).collect();
                map.bindings.insert(action, bindings);
            }
        }
        map
    }

    /// Only what differs from the defaults, for `settings.json`.
    pub fn overrides(&self) -> BTreeMap<String, Vec<String>> {
        self.bindings
            .iter()
            .filter(|(action, bindings)| **bindings != action.defaults())
            .map(|(action, bindings)| {
                (
                    action.id().to_owned(),
                    bindings.iter().map(|b| b.text()).collect(),
                )
            })
            .collect()
    }

    pub fn get(&self, action: Action) -> &[Binding] {
        self.bindings.get(&action).map_or(&[], Vec::as_slice)
    }

    /// Gives `binding` to `action` alone, taking it from any other action. Returns the
    /// action it was taken from.
    pub fn assign(&mut self, action: Action, binding: Binding) -> Option<Action> {
        let mut taken = None;
        for (&other, bindings) in &mut self.bindings {
            if other != action && bindings.contains(&binding) {
                bindings.retain(|b| *b != binding);
                taken = Some(other);
            }
        }
        self.bindings.insert(action, vec![binding]);
        taken
    }

    pub fn clear(&mut self, action: Action) {
        self.bindings.insert(action, Vec::new());
    }

    /// Back to the default keys, taking them from any action that has them now.
    pub fn reset(&mut self, action: Action) {
        let defaults = action.defaults();
        for (&other, bindings) in &mut self.bindings {
            if other != action {
                bindings.retain(|b| !defaults.contains(b));
            }
        }
        self.bindings.insert(action, defaults);
    }

    pub fn is_default(&self, action: Action) -> bool {
        self.get(action) == action.defaults()
    }

    pub fn label(&self, action: Action) -> String {
        self.get(action)
            .first()
            .map(|b| b.display())
            .unwrap_or_default()
    }

    /// Whether a bare press of `key` triggers `action`.
    pub fn has_plain(&self, action: Action, key: Key) -> bool {
        self.get(action).contains(&Binding::plain(key))
    }

    /// Whether a press of `action` is waiting in `input`. Matching presses are consumed,
    /// including key repeats of actions that fire once per press.
    pub fn pressed(&self, input: &mut InputState, action: Action) -> bool {
        let bindings = self.get(action);
        if bindings.is_empty() {
            return false;
        }
        let mut fired = false;
        input.events.retain(|event| match *event {
            Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } if bindings.iter().any(|b| b.matches(key, modifiers)) => {
                fired |= !repeat || action.repeats();
                false
            }
            _ => true,
        });
        fired
    }

    /// Whether any action uses this key, ignoring modifiers.
    pub fn uses(&self, key: Key) -> bool {
        self.bindings.values().flatten().any(|b| b.key == key)
    }
}

thread_local! {
    /// The keymap of this UI thread, with the overrides it was built from.
    static CURRENT: RefCell<(BTreeMap<String, Vec<String>>, Keymap)> =
        RefCell::new((BTreeMap::new(), Keymap::default()));
}

/// Builds the keymap from the stored overrides when they changed.
pub fn install(overrides: &BTreeMap<String, Vec<String>>) {
    CURRENT.with_borrow_mut(|(source, map)| {
        if source != overrides {
            *map = Keymap::from_overrides(overrides);
            source.clone_from(overrides);
        }
    });
}

pub fn with<R>(f: impl FnOnce(&Keymap) -> R) -> R {
    CURRENT.with_borrow(|(_, map)| f(map))
}

pub fn current() -> Keymap {
    with(Clone::clone)
}

pub fn pressed(input: &mut InputState, action: Action) -> bool {
    with(|map| map.pressed(input, action))
}

/// The first binding of `action` as the menus show it, or `""` when it has none.
///
/// Labels are interned, so each distinct label is allocated once for the life of the
/// app and the many `&'static str` shortcut fields keep working.
pub fn label(action: Action) -> &'static str {
    static LABELS: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let text = with(|map| map.label(action));
    let mut labels = LABELS.lock().unwrap_or_else(|e| e.into_inner());
    let labels = labels.get_or_insert_with(HashSet::new);
    if let Some(&label) = labels.get(text.as_str()) {
        return label;
    }
    let label: &'static str = Box::leak(text.into_boxed_str());
    labels.insert(label);
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: Modifiers, repeat: bool) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat,
            modifiers,
        }
    }

    fn input(events: Vec<Event>) -> InputState {
        let mut input = InputState::default();
        input.events = events;
        input
    }

    #[test]
    fn settings_list_each_group_once() {
        let mut groups: Vec<_> = Action::ALL.iter().map(|a| a.group()).collect();
        groups.dedup();
        let count = groups.len();
        groups.sort_unstable();
        groups.dedup();
        assert_eq!(groups.len(), count, "a group is split in two");
    }

    #[test]
    fn defaults_have_no_shared_binding() {
        let map = Keymap::default();
        let mut seen = HashSet::new();
        for action in Action::ALL {
            for binding in map.get(action) {
                assert!(seen.insert(*binding), "{binding:?} is bound twice");
                assert!(binding.reserved().is_none(), "{binding:?} is reserved");
            }
        }
        let ids: HashSet<_> = Action::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(ids.len(), Action::ALL.len());
    }

    #[test]
    fn bindings_round_trip_through_their_stored_text() {
        for action in Action::ALL {
            for binding in action.defaults() {
                assert_eq!(Binding::parse(&binding.text()), Some(binding));
            }
        }
        assert_eq!(Binding::ctrl(Key::Plus).text(), "Ctrl+Plus");
        assert_eq!(Binding::ctrl(Key::Equals).display(), "Ctrl+=");
        assert_eq!(Binding::ctrl(Key::Z).with_shift().display(), "Ctrl+Shift+Z");
        assert_eq!(
            Binding::parse("ctrl+shift+Plus"),
            Some(Binding::ctrl(Key::Plus))
        );
        assert_eq!(Binding::parse("Hyper+A"), None);
        assert_eq!(Binding::parse(""), None);
    }

    #[test]
    fn modifiers_match_exactly_except_for_typed_symbols() {
        let map = Keymap::default();
        let mut i = input(vec![press(Key::N, Modifiers::SHIFT, false)]);
        assert!(!map.pressed(&mut i, Action::NextIssue));
        assert!(map.pressed(&mut i, Action::PreviousIssue));
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        let mut i = input(vec![press(Key::Plus, ctrl_shift, false)]);
        assert!(map.pressed(&mut i, Action::ScaleUp));
        let mut i = input(vec![press(Key::Z, ctrl_shift, false)]);
        assert!(!map.pressed(&mut i, Action::Undo));
        assert!(map.pressed(&mut i, Action::Redo));
    }

    #[test]
    fn a_held_key_fires_once_unless_the_action_repeats() {
        let map = Keymap::default();
        let mut i = input(vec![press(Key::Space, Modifiers::NONE, true)]);
        assert!(!map.pressed(&mut i, Action::PlayPause));
        assert!(
            i.events.is_empty(),
            "the repeat is consumed, not left for widgets"
        );
        let mut i = input(vec![press(Key::Z, Modifiers::CTRL, true)]);
        assert!(map.pressed(&mut i, Action::Undo));
    }

    #[test]
    fn assigning_takes_the_key_from_its_old_action_and_overrides_store_only_changes() {
        let mut map = Keymap::default();
        assert!(map.overrides().is_empty());
        // osu!'s editor plays and pauses with X and C; here, X on play/pause.
        assert_eq!(map.assign(Action::PlayPause, Binding::plain(Key::X)), None);
        assert_eq!(
            map.assign(Action::Save, Binding::plain(Key::Num1)),
            Some(Action::ToolSelect)
        );
        assert!(map.get(Action::ToolSelect).is_empty());
        map.clear(Action::Help);
        let overrides = map.overrides();
        assert_eq!(overrides["play_pause"], ["X"]);
        assert_eq!(overrides["save"], ["1"]);
        assert_eq!(overrides["tool_select"], Vec::<String>::new());
        assert_eq!(overrides["help"], Vec::<String>::new());
        assert_eq!(overrides.len(), 4);
        assert_eq!(Keymap::from_overrides(&overrides), map);

        map.reset(Action::ToolSelect);
        assert!(map.is_default(Action::ToolSelect));
        assert!(map.get(Action::Save).is_empty(), "reset takes its key back");
    }

    #[test]
    fn unknown_actions_and_bad_bindings_in_settings_are_skipped() {
        let overrides = BTreeMap::from([
            ("from_the_future".to_owned(), vec!["Ctrl+Q".to_owned()]),
            (
                "save".to_owned(),
                vec!["Nonsense".to_owned(), "F2".to_owned()],
            ),
        ]);
        let map = Keymap::from_overrides(&overrides);
        assert_eq!(map.get(Action::Save), [Binding::plain(Key::F2)]);
        assert!(map.is_default(Action::Undo));
    }
}
