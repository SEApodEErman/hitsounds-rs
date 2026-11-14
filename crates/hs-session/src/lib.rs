//! The editor session behind the hitsounds-rs UI: one open project with its undo history,
//! the cached mapset and its file watcher, playback, drum separation (D-70) and the
//! app-wide settings.
//!
//! This crate has no GUI dependency (D-48). The UI shell calls it directly and gets
//! told about outside changes through callbacks. See docs/architecture.md#app-shell.

pub mod import;
pub mod library;
pub mod mapset;
mod osudb;
pub mod playback;
pub mod separation;
pub mod session;
pub mod settings;
pub mod songs;

pub use playback::{Clock, Waveform};
pub use session::{
    CompletedExport, MovedMapset, OnMapsetChange, Session, Transport, Workspace, moved_mapset,
};
pub use settings::{Levels, Settings};
