//! Playback state. The audio thread owns the clock; the UI reads position from it.

/// What plays under the hitsounds (D-22: a simple toggle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonitorSource {
    #[default]
    Original,
    Stems,
}

/// A snapshot of the transport, as the listener hears it right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransportState {
    pub playing: bool,
    pub position_ms: f64,
    /// osu! editor-style playback rate (0.25..=1.0). The song slows down and drops in
    /// pitch; hitsounds keep their own speed and pitch.
    pub rate: f64,
    pub monitor: MonitorSource,
    /// End of the song (or of the last hitsound when no song is loaded).
    pub length_ms: f64,
}
