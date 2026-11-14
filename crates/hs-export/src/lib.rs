//! hitsounds-rs export, and the preview that shares its pipeline.
//!
//! Export order (see `docs/architecture.md#export-pipeline`):
//! 1. project every enabled diff, resolve packages, build one mapset-wide schema ([`plan`]);
//! 2. [`apply`] the hits to each diff in memory;
//! 3. read the result back with stable's rules ([`simulate`]) and verify every hit;
//! 4. render the sample files that are actually played ([`library`], [`wav`]) and run
//!    the ranking checks ([`checks`]);
//! 5. on [`export::execute`]: back up, write, remove stale files, save the manifest.
//!
//! This is the only crate that writes into a mapset.

pub mod apply;
pub mod checks;
pub mod encode;
pub mod export;
pub mod library;
pub mod names;
pub mod plan;
pub mod simulate;
mod slider;
pub mod wav;
mod write;

pub use apply::{WrittenHit, apply_hits};
pub use checks::{SampleFix, SampleIssue, auto_fix, check_sample};
pub use export::{ExportError, ExportPlan, ExportReport, Manifest, execute};
pub use library::{RenderError, SampleLibrary};
pub use plan::{DiffPlan, PreviewDiff, PreviewHit, PreviewSound, preview, preview_in_project};
