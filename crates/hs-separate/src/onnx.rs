//! The exported HT-Demucs FT drums model, run with ONNX Runtime on the CPU (D-72).
//!
//! The runtime (`onnxruntime.dll`) is loaded at run time, so a missing runtime only
//! disables separation. The model file is the user's own export from
//! `tools/demucs-export/export_drums_onnx.py` (D-71); [`DrumsModel::load`] checks
//! its inputs and outputs before anything runs.

use std::path::Path;
use std::sync::OnceLock;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{Tensor, ValueType};

use crate::SeparateError;
use crate::apply::{FRAMES, SEGMENT, SegmentModel, SegmentOutput};
use crate::stft::BINS;

/// The model's inputs and outputs, as `export_drums_onnx.py` writes them.
const SIGNATURE: [(&str, &[i64]); 4] = [
    ("mix", &[1, 2, SEGMENT as i64]),
    ("spec", &[1, 4, BINS as i64, FRAMES as i64]),
    ("spec_out", &[1, 4, BINS as i64, FRAMES as i64]),
    ("time_out", &[1, 2, SEGMENT as i64]),
];

static RUNTIME: OnceLock<Result<(), String>> = OnceLock::new();

/// Loads ONNX Runtime from `dll` (`onnxruntime.dll`, 1.17 or newer). Only the first
/// call in a process loads anything; later calls return its result.
pub fn load_runtime(dll: &Path) -> Result<(), SeparateError> {
    let fail = |message: String| SeparateError::Runtime {
        path: dll.display().to_string(),
        message,
    };
    RUNTIME
        .get_or_init(|| {
            let builder = ort::init_from(dll).map_err(|e| e.to_string())?;
            builder.commit();
            Ok(())
        })
        .clone()
        .map_err(fail)
}

/// The drums model, ready to run segments.
pub struct DrumsModel {
    session: Session,
}

impl DrumsModel {
    /// Opens `path` and checks it is a hitsounds-rs drums export. Call
    /// [`load_runtime`] first.
    pub fn load(path: &Path) -> Result<Self, SeparateError> {
        let shown = path.display().to_string();
        if RUNTIME.get().is_none_or(Result::is_err) {
            return Err(SeparateError::Runtime {
                path: shown,
                message: "ONNX Runtime is not loaded".into(),
            });
        }
        let model = |reason: String| SeparateError::Model {
            path: shown.clone(),
            reason,
        };
        let session = Session::builder()
            .and_then(|b| {
                b.with_optimization_level(GraphOptimizationLevel::Level3)
                    .map_err(Into::into)
            })
            .and_then(|mut b| b.commit_from_file(path))
            .map_err(|e| model(e.to_string()))?;
        let outlets = session.inputs().iter().chain(session.outputs());
        let found: Vec<(&str, Option<Vec<i64>>)> = outlets
            .map(|o| {
                let shape = match o.dtype() {
                    ValueType::Tensor {
                        ty: ort::value::TensorElementType::Float32,
                        shape,
                        ..
                    } => Some(shape.iter().copied().collect()),
                    _ => None,
                };
                (o.name(), shape)
            })
            .collect();
        for (name, shape) in SIGNATURE {
            match found.iter().find(|(n, _)| *n == name) {
                None => return Err(model(format!("it has no `{name}`"))),
                Some((_, None)) => return Err(model(format!("`{name}` is not a float32 tensor"))),
                Some((_, Some(s))) if s != shape => {
                    return Err(model(format!(
                        "`{name}` has shape {s:?}, expected {shape:?}"
                    )));
                }
                Some(_) => {}
            }
        }
        Ok(Self { session })
    }
}

impl SegmentModel for DrumsModel {
    fn run(&mut self, mix: &[f32], spec: &[f32]) -> Result<SegmentOutput, SeparateError> {
        let fail = |e: ort::Error| SeparateError::Inference(e.to_string());
        let mix = Tensor::from_array(([1usize, 2, SEGMENT], mix.to_vec().into_boxed_slice()))
            .map_err(fail)?;
        let spec =
            Tensor::from_array(([1usize, 4, BINS, FRAMES], spec.to_vec().into_boxed_slice()))
                .map_err(fail)?;
        let outputs = self
            .session
            .run(ort::inputs!["mix" => mix, "spec" => spec])
            .map_err(fail)?;
        let take = |name: &str| -> Result<Vec<f32>, SeparateError> {
            let (_, data) = outputs[name].try_extract_tensor::<f32>().map_err(fail)?;
            Ok(data.to_vec())
        };
        Ok(SegmentOutput {
            spec: take("spec_out")?,
            time: take("time_out")?,
        })
    }
}
