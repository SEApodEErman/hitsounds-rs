//! Sample-rate conversion (rubato's synchronous FFT resampler).

use crate::AudioBuffer;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

#[derive(Debug, thiserror::Error)]
#[error("resampling {from} Hz to {to} Hz failed: {message}")]
pub struct ResampleError {
    from: u32,
    to: u32,
    message: String,
}

/// Converts `buf` to `rate` Hz. The result starts at the same instant as the input
/// (the resampler's delay is trimmed) and has the same duration to within a frame.
pub fn resample(buf: &AudioBuffer, rate: u32) -> Result<AudioBuffer, ResampleError> {
    if buf.sample_rate == rate || buf.frames() == 0 {
        return Ok(AudioBuffer { sample_rate: rate, ..buf.clone() });
    }
    let fail = |message: String| ResampleError { from: buf.sample_rate, to: rate, message };
    let channels = buf.channels.max(1) as usize;
    let frames = buf.frames();
    let mut resampler = Fft::<f32>::new(buf.sample_rate as usize, rate as usize, 1024, channels, FixedSync::Input)
        .map_err(|e| fail(e.to_string()))?;
    let capacity = resampler.process_all_needed_output_len(frames);
    let mut out = vec![0.0f32; capacity * channels];
    let input = InterleavedSlice::new(&buf.samples, channels, frames).map_err(|e| fail(e.to_string()))?;
    let mut output = InterleavedSlice::new_mut(&mut out, channels, capacity).map_err(|e| fail(e.to_string()))?;
    let (_, written) = resampler
        .process_all_into_buffer(&input, &mut output, frames, None)
        .map_err(|e| fail(e.to_string()))?;
    // `process_all` may flush a little past the true end; cut to the exact duration.
    let expected = (frames as f64 * f64::from(rate) / f64::from(buf.sample_rate)).round() as usize;
    out.truncate(written.min(expected) * channels);
    Ok(AudioBuffer { sample_rate: rate, channels: buf.channels, samples: out })
}

/// Converts to the given rate and channel count.
pub fn conform(buf: &AudioBuffer, rate: u32, channels: u16) -> Result<AudioBuffer, ResampleError> {
    Ok(resample(buf, rate)?.with_channels(channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f32, frames: usize) -> AudioBuffer {
        AudioBuffer {
            sample_rate: rate,
            channels: 1,
            samples: (0..frames).map(|i| (i as f32 * hz * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect(),
        }
    }

    #[test]
    fn keeps_duration_and_pitch() {
        let src = sine(44_100, 441.0, 44_100);
        let out = resample(&src, 48_000).unwrap();
        assert_eq!(out.sample_rate, 48_000);
        assert!((out.frames() as i64 - 48_000).abs() <= 1, "got {} frames", out.frames());
        // A 441 Hz sine has 882 zero crossings per second at any sample rate.
        let crossings = out.samples.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((crossings as i64 - 882).abs() <= 4, "got {crossings} zero crossings");
    }

    #[test]
    fn stays_aligned_to_the_start() {
        // An impulse at 100 ms must still be at 100 ms after conversion.
        let mut src = AudioBuffer::silent(44_100, 1, 22_050);
        src.samples[4_410] = 1.0;
        let out = resample(&src, 48_000).unwrap();
        let peak_ms = out.peak_frame() as f64 * 1000.0 / 48_000.0;
        assert!((peak_ms - 100.0).abs() < 0.1, "impulse moved to {peak_ms} ms");
    }

    #[test]
    fn same_rate_is_a_copy() {
        let src = sine(48_000, 100.0, 100);
        assert_eq!(resample(&src, 48_000).unwrap(), src);
    }
}
