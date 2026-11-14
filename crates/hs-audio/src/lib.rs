//! hitsounds-rs audio. No osu! knowledge lives here (docs/architecture.md).
//!
//! * [`buffer`] / [`mix`]: deterministic offline mixing. Export renders merged sample
//!   files with it, and preview plays those same renders, so what you hear matches
//!   what is written.
//! * [`decode`] / [`resample`]: files to PCM at a chosen rate.
//! * [`mixer`]: the real-time mix of song and scheduled hitsound voices; [`engine`]
//!   runs it on the output device.
//! * [`stretch`]: keeps the song's pitch when playback is slowed (D-90).
//! * [`peaks`]: waveform overview for the timeline.

pub mod buffer;
pub mod decode;
pub mod engine;
pub mod mix;
pub mod mixer;
pub mod peaks;
pub mod resample;
pub mod stretch;
pub mod transport;

pub use buffer::AudioBuffer;
pub use decode::{DecodeError, Mp3Trim, decode_bytes, decode_file, decode_file_with_mp3_trim};
pub use engine::{Engine, EngineError};
pub use mixer::{Command, Mixer, Scheduled};
pub use resample::{conform, resample};
pub use transport::{MonitorSource, TransportState};
