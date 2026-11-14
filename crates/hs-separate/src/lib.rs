//! hitsounds-rs drum stem separation (D-70 to D-73).
//!
//! Runs the drums model of HT-Demucs FT (Demucs, MIT code) on a song and returns the
//! drum stem at the song's sample rate and exact length. Pure like `hs-core`: audio
//! in, drum audio out. The only I/O is loading ONNX Runtime and the model from the
//! paths it is given (D-72). Writing the stem, reuse and the project's
//! `stem_origin` belong to `hs-session`.
//!
//! * [`stft`]: torch-compatible STFT and Demucs' spectrogram packing.
//! * [`apply`]: normalisation, segments and overlap-add around a [`SegmentModel`].
//! * [`onnx`]: the ONNX model ([`DrumsModel`]).
//!
//! `tools/demucs-export/reference.py` is the NumPy twin of this crate, checked
//! against PyTorch Demucs.

pub mod apply;
pub mod onnx;
pub mod stft;

use std::sync::atomic::AtomicBool;

use hs_audio::AudioBuffer;

pub use apply::{Progress, SegmentModel, SegmentOutput};
pub use onnx::{DrumsModel, load_runtime};

#[derive(Debug, thiserror::Error)]
pub enum SeparateError {
    #[error("the song has no audio")]
    Empty,
    #[error("separation was cancelled")]
    Cancelled,
    #[error(transparent)]
    Resample(#[from] hs_audio::resample::ResampleError),
    #[error("cannot load ONNX Runtime from {path}: {message}")]
    Runtime { path: String, message: String },
    #[error("{path} is not a drums model made by hitsounds-rs's export script: {reason}")]
    Model { path: String, reason: String },
    #[error("the separation model failed: {0}")]
    Inference(String),
}

/// Separates the drums from `song`.
///
/// The result is stereo at `song`'s sample rate with exactly `song.frames()` frames,
/// aligned sample for sample. Pass the buffer the app plays (`hs_audio::decode_file`)
/// so the stem uses the same MP3 gapless timing (D-73). Values are not
/// limited and may exceed full scale.
pub fn separate(
    song: &AudioBuffer,
    model: &mut dyn SegmentModel,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<AudioBuffer, SeparateError> {
    if song.frames() == 0 {
        return Err(SeparateError::Empty);
    }
    let input = hs_audio::conform(song, apply::SAMPLE_RATE, 2)?;
    let planar: [Vec<f32>; 2] =
        [0, 1].map(|c| input.samples.iter().skip(c).step_by(2).copied().collect());
    let drums = apply::separate_planar([&planar[0], &planar[1]], model, cancel, progress)?;
    let interleaved = AudioBuffer {
        sample_rate: apply::SAMPLE_RATE,
        channels: 2,
        samples: drums[0]
            .iter()
            .zip(&drums[1])
            .flat_map(|(&l, &r)| [l, r])
            .collect(),
    };
    let mut out = hs_audio::resample(&interleaved, song.sample_rate)?;
    // Resampling keeps the duration only to within a frame; the stem must match exactly.
    out.samples.resize(song.frames() * 2, 0.0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::tests::TimeIdentity;

    fn sine(rate: u32, channels: u16, frames: usize) -> AudioBuffer {
        AudioBuffer {
            sample_rate: rate,
            channels,
            samples: (0..frames * channels as usize)
                .map(|i| {
                    0.5 * (std::f32::consts::TAU * 220.0 * (i / channels as usize) as f32
                        / rate as f32)
                        .sin()
                })
                .collect(),
        }
    }

    #[test]
    fn keeps_the_songs_rate_and_exact_length() {
        for (rate, channels, frames) in [
            (44_100, 2, 123_457),
            (48_000, 2, 480_001),
            (22_050, 1, 70_000),
        ] {
            let song = sine(rate, channels, frames);
            let out = separate(
                &song,
                &mut TimeIdentity { calls: 0 },
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
            assert_eq!(
                (out.sample_rate, out.channels, out.frames()),
                (rate, 2, frames)
            );
        }
    }

    #[test]
    fn a_click_stays_put_through_resampling() {
        // 48 kHz song: resampled to 44.1 kHz for the model and back.
        let mut song = AudioBuffer::silent(48_000, 2, 600_000);
        song.samples[2 * 432_100] = 1.0;
        song.samples[2 * 432_100 + 1] = 1.0;
        let out = separate(
            &song,
            &mut TimeIdentity { calls: 0 },
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(
            (out.peak_frame() as i64 - 432_100).abs() <= 1,
            "click moved to {}",
            out.peak_frame()
        );
    }

    #[test]
    fn an_empty_song_is_an_error() {
        let song = AudioBuffer::silent(44_100, 2, 0);
        let result = separate(
            &song,
            &mut TimeIdentity { calls: 0 },
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(matches!(result, Err(SeparateError::Empty)));
    }
}
