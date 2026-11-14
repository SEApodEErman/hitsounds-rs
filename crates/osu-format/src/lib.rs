//! Lossless reader/writer for osu!stable `.osu` files.
//!
//! Design rule: hitsounds-rs writes into a mapper's real difficulties, so anything we
//! do not deliberately change must round-trip byte-for-byte. [`OsuDocument`] keeps
//! every line verbatim; typed views ([`TimingPoint`], [`HitObject`]) are parsed on
//! demand and only re-serialised for lines we modify.
//!
//! See `docs/findings.md` for the osu! semantics encoded here.

pub mod document;
pub mod hitobject;
pub mod sample;
pub mod timing;

pub use document::{OsuDocument, Section};
pub use hitobject::{HitObject, HitObjectKind};
pub use sample::{HitSample, Hitsound, HitsoundFlags, SampleSet};
pub use timing::{TimingPoint, TimingPointChange};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("line {line}: {message}")]
    Invalid { line: usize, message: String },
}

pub type Result<T> = std::result::Result<T, ParseError>;

pub(crate) fn invalid(line: usize, message: impl Into<String>) -> ParseError {
    ParseError::Invalid {
        line,
        message: message.into(),
    }
}

/// Formats a number the way osu! does: integers without a decimal point.
pub(crate) fn fmt_num(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}
