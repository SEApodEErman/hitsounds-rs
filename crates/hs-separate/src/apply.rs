//! Whole-track separation: normalise, cut into overlapping segments, run the model on
//! each, and blend them back with triangular weights.
//!
//! Ported from Demucs (MIT, github.com/adefossez/demucs; see THIRD_PARTY_NOTICES.md):
//! `demucs/apply.py` (`apply_model` with `split=True`, `shifts=0`, `overlap=0.25`,
//! `TensorChunk.padded`), `demucs/utils.py` (`center_trim`) and the track
//! normalisation of `demucs/api.py` (`Separator.separate_tensor`). hitsounds-rs uses no
//! random shift, so the same song and model always give the same stem (D-73).

use std::sync::atomic::{AtomicBool, Ordering};

use crate::SeparateError;
use crate::stft::{BINS, HOP, demucs_ispec, demucs_spec, from_cac, to_cac};

/// The model's sample rate.
pub const SAMPLE_RATE: u32 = 44_100;
/// One segment: `int(39/5 * 44100)`, the training length of `htdemucs_ft`.
pub const SEGMENT: usize = 343_980;
/// STFT frames per segment.
pub const FRAMES: usize = SEGMENT.div_ceil(HOP);
/// Samples between segment starts (25 % overlap).
pub const STRIDE: usize = (SEGMENT as f64 * 0.75) as usize;

/// What one segment gives back, both halves already de-normalised by the model.
pub struct SegmentOutput {
    /// Drums, frequency branch: four planes (re0, im0, re1, im1) of `BINS × FRAMES`.
    pub spec: Vec<f32>,
    /// Drums, time branch: two planes of `SEGMENT` samples.
    pub time: Vec<f32>,
}

/// The network between the STFT and the iSTFT, for one segment. The ONNX model is the
/// real one; tests use stand-ins so they need no weights.
pub trait SegmentModel {
    /// `mix`: two planes of `SEGMENT` samples (track-normalised). `spec`: its
    /// complex-as-channels spectrogram, four planes of `BINS × FRAMES`.
    fn run(&mut self, mix: &[f32], spec: &[f32]) -> Result<SegmentOutput, SeparateError>;
}

/// How far a separation has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

/// Number of segments for `frames` samples.
pub fn segment_count(frames: usize) -> usize {
    frames.div_ceil(STRIDE)
}

/// Separates planar stereo at 44.1 kHz into planar stereo drums of the same length.
pub fn separate_planar(
    channels: [&[f32]; 2],
    model: &mut dyn SegmentModel,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<[Vec<f32>; 2], SeparateError> {
    let n = channels[0].len();
    assert_eq!(n, channels[1].len(), "channels differ in length");
    if n == 0 {
        return Err(SeparateError::Empty);
    }

    // api.Separator: normalise the whole track by its mono mean and (unbiased) std.
    let mono = |i: usize| (f64::from(channels[0][i]) + f64::from(channels[1][i])) / 2.0;
    let mean = (0..n).map(mono).sum::<f64>() / n as f64;
    let var = (0..n).map(|i| (mono(i) - mean).powi(2)).sum::<f64>() / (n.max(2) - 1) as f64;
    let std = var.sqrt() + 1e-8;
    let norm: [Vec<f32>; 2] = channels.map(|c| {
        c.iter()
            .map(|&s| ((f64::from(s) - mean) / std) as f32)
            .collect()
    });

    let weight = segment_weight();
    let mut out = [vec![0.0f32; n], vec![0.0f32; n]];
    let mut total = vec![0.0f32; n];
    let count = segment_count(n);
    // Reported before the first segment too, so a UI can show "1 of N" while it runs.
    progress(Progress {
        done: 0,
        total: count,
    });
    for (i, offset) in (0..n).step_by(STRIDE).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(SeparateError::Cancelled);
        }
        let length = SEGMENT.min(n - offset);
        let drums = run_segment(
            model,
            [
                padded(&norm[0], offset, length),
                padded(&norm[1], offset, length),
            ],
        )?;
        let trim = (SEGMENT - length) / 2; // utils.center_trim
        for (acc, seg) in out.iter_mut().zip(&drums) {
            for k in 0..length {
                acc[offset + k] += weight[k] * seg[trim + k];
            }
        }
        for k in 0..length {
            total[offset + k] += weight[k];
        }
        progress(Progress {
            done: i + 1,
            total: count,
        });
    }
    for acc in &mut out {
        for (s, &w) in acc.iter_mut().zip(&total) {
            *s = (f64::from(*s / w) * std + mean) as f32;
        }
    }
    Ok(out)
}

/// apply.py's blend weight: a triangle peaking mid-segment, normalised to 1.
fn segment_weight() -> Vec<f32> {
    let half = SEGMENT / 2;
    let max = half.max(SEGMENT - half) as f32;
    (0..SEGMENT)
        .map(|i| if i < half { i + 1 } else { SEGMENT - i } as f32 / max)
        .collect()
}

/// `TensorChunk.padded`: `SEGMENT` samples centred on the chunk at `offset..offset +
/// length`, borrowing real audio around it and zeros past either end of the track.
fn padded(x: &[f32], offset: usize, length: usize) -> Vec<f32> {
    let delta = (SEGMENT - length) as isize;
    let start = offset as isize - delta / 2;
    (0..SEGMENT as isize)
        .map(|k| {
            let i = start + k;
            if i >= 0 && (i as usize) < x.len() {
                x[i as usize]
            } else {
                0.0
            }
        })
        .collect()
}

/// One segment through STFT, model and iSTFT: both branches summed, per channel.
fn run_segment(
    model: &mut dyn SegmentModel,
    mix: [Vec<f32>; 2],
) -> Result<[Vec<f32>; 2], SeparateError> {
    let spec = to_cac(&demucs_spec(&mix[0]), &demucs_spec(&mix[1]));
    let mut planar = Vec::with_capacity(2 * SEGMENT);
    planar.extend_from_slice(&mix[0]);
    planar.extend_from_slice(&mix[1]);
    let output = model.run(&planar, &spec)?;
    if output.spec.len() != 4 * BINS * FRAMES || output.time.len() != 2 * SEGMENT {
        return Err(SeparateError::Inference(format!(
            "the model returned {} and {} values, expected {} and {}",
            output.spec.len(),
            output.time.len(),
            4 * BINS * FRAMES,
            2 * SEGMENT
        )));
    }
    let freq = from_cac(&output.spec, FRAMES);
    Ok([0, 1].map(|c| {
        let mut x = demucs_ispec(&freq[c], SEGMENT);
        for (s, &t) in x
            .iter_mut()
            .zip(&output.time[c * SEGMENT..(c + 1) * SEGMENT])
        {
            *s += t;
        }
        x
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Gives the mix back as "drums" through the time branch: the whole pipeline must
    /// then reproduce its input exactly.
    pub struct TimeIdentity {
        pub calls: usize,
    }
    impl SegmentModel for TimeIdentity {
        fn run(&mut self, mix: &[f32], spec: &[f32]) -> Result<SegmentOutput, SeparateError> {
            self.calls += 1;
            Ok(SegmentOutput {
                spec: vec![0.0; spec.len()],
                time: mix.to_vec(),
            })
        }
    }

    /// Gives the spectrogram back through the frequency branch.
    struct SpectralIdentity;
    impl SegmentModel for SpectralIdentity {
        fn run(&mut self, mix: &[f32], spec: &[f32]) -> Result<SegmentOutput, SeparateError> {
            Ok(SegmentOutput {
                spec: spec.to_vec(),
                time: vec![0.0; mix.len()],
            })
        }
    }

    struct WrongSize;
    impl SegmentModel for WrongSize {
        fn run(&mut self, _: &[f32], _: &[f32]) -> Result<SegmentOutput, SeparateError> {
            Ok(SegmentOutput {
                spec: vec![0.0; 3],
                time: vec![0.0; 3],
            })
        }
    }

    fn track(n: usize) -> [Vec<f32>; 2] {
        let t = |i: usize| i as f32 / 44_100.0;
        [
            (0..n)
                .map(|i| 0.4 * (std::f32::consts::TAU * 97.0 * t(i)).sin() + 0.05)
                .collect(),
            (0..n)
                .map(|i| 0.3 * (std::f32::consts::TAU * 431.0 * t(i)).cos() - 0.02)
                .collect(),
        ]
    }

    fn max_err(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f32::max)
    }

    fn run(
        x: &[Vec<f32>; 2],
        model: &mut dyn SegmentModel,
    ) -> Result<[Vec<f32>; 2], SeparateError> {
        separate_planar([&x[0], &x[1]], model, &AtomicBool::new(false), &mut |_| {})
    }

    #[test]
    fn constants_match_the_model() {
        assert_eq!(FRAMES, 336);
        assert_eq!(STRIDE, 257_985);
    }

    #[test]
    fn overlap_add_reproduces_the_input_at_any_length() {
        // Shorter than a segment, exactly one, and several with a ragged last chunk.
        for n in [1_000, SEGMENT, 3 * SEGMENT + 12_345] {
            let x = track(n);
            let y = run(&x, &mut TimeIdentity { calls: 0 }).unwrap();
            for c in 0..2 {
                assert_eq!(y[c].len(), n);
                let err = max_err(&x[c], &y[c]);
                assert!(err < 1e-5, "{n} samples, channel {c}: error {err}");
            }
        }
    }

    #[test]
    fn a_click_keeps_its_sample() {
        let mut x = [vec![0.0f32; 2 * SEGMENT], vec![0.0f32; 2 * SEGMENT]];
        x[0][300_001] = 1.0; // inside the overlap of the first two segments
        x[1][300_001] = -1.0;
        let y = run(&x, &mut TimeIdentity { calls: 0 }).unwrap();
        let peak = y[0]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert_eq!(peak, 300_001);
    }

    #[test]
    fn spectral_branch_lines_up_with_the_time_branch() {
        // Through the frequency branch, low-frequency content comes back in place
        // away from the track's ends (Demucs' own spec pair is lossy only at the
        // segment edges and the Nyquist bin).
        let n = 2 * SEGMENT;
        let x = track(n);
        let y = run(&x, &mut SpectralIdentity).unwrap();
        for c in 0..2 {
            let err = max_err(&x[c][8_192..n - 8_192], &y[c][8_192..n - 8_192]);
            assert!(err < 1e-3, "channel {c}: error {err}");
        }
    }

    #[test]
    fn reports_every_segment_and_stops_when_cancelled() {
        let n = 3 * SEGMENT;
        let x = track(n);
        let mut seen = Vec::new();
        separate_planar(
            [&x[0], &x[1]],
            &mut TimeIdentity { calls: 0 },
            &AtomicBool::new(false),
            &mut |p| seen.push(p),
        )
        .unwrap();
        let total = segment_count(n);
        assert_eq!(
            seen,
            (0..=total)
                .map(|done| Progress { done, total })
                .collect::<Vec<_>>()
        );

        let cancel = AtomicBool::new(false);
        let mut model = TimeIdentity { calls: 0 };
        let result = separate_planar([&x[0], &x[1]], &mut model, &cancel, &mut |p| {
            if p.done == 1 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert!(matches!(result, Err(SeparateError::Cancelled)));
        assert_eq!(model.calls, 1);
    }

    #[test]
    fn a_model_of_the_wrong_shape_is_an_error() {
        let x = track(1_000);
        assert!(matches!(
            run(&x, &mut WrongSize),
            Err(SeparateError::Inference(_))
        ));
    }

    #[test]
    fn an_empty_track_is_an_error() {
        let x = [Vec::new(), Vec::new()];
        assert!(matches!(
            run(&x, &mut TimeIdentity { calls: 0 }),
            Err(SeparateError::Empty)
        ));
    }
}
