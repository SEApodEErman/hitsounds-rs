//! hitsounds-rs core: everything that is pure data + algorithms (no I/O, no audio).
//!
//! Pipeline (see `docs/architecture.md`):
//!
//! ```text
//! Project (channels + base events) ──┐
//! DiffProfile (overrides, fallback) ─┼─► projection ─► packages ─► packing ─► IndexPlan
//! DiffTimeline (objects of a .osu) ──┘     (per diff)                (mapset-wide)
//! ```

pub mod channel;
pub mod diff;
pub mod edit;
pub mod import;
pub mod mute;
pub mod packing;
pub mod project;
pub mod projection;
pub mod resolve;
pub mod volume;

pub use channel::{Channel, ChannelId, SampleSource};
pub use diff::{DiffProfile, Override, TickPolicy, TimeRange};
pub use edit::{ChannelProps, DiffProps, Edit, EditError, History};
pub use project::{Project, ProjectError, StemOrigin};
pub use mute::SliderEndMute;
pub use volume::{Curve, VolumeEnvelope, VolumePoint};
pub use projection::{DiffTimeline, Edge, EdgeKind, HitKind, ProjectedHit, Projection, Target, Warning};

/// Milliseconds, matching osu!'s integer object times.
pub type Ms = i32;
