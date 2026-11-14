//! Offline mixing of samples into one file (merged hitsounds, slidertick renders).

use crate::AudioBuffer;

#[derive(Debug, thiserror::Error)]
pub enum MixError {
    /// Inputs must be converted to a common rate and layout first (`resample::conform`).
    #[error("inputs have different sample rates or channel counts")]
    FormatMismatch,
}

/// Sums `inputs` (each with linear gain) starting at frame 0. Output length is the
/// longest input; the result is normalised to peak 1.0 only if it would clip.
pub fn mix(inputs: &[(&AudioBuffer, f32)]) -> Result<AudioBuffer, MixError> {
    let Some((first, _)) = inputs.first() else {
        return Ok(AudioBuffer::silent(44_100, 2, 0));
    };
    if inputs
        .iter()
        .any(|(b, _)| b.sample_rate != first.sample_rate || b.channels != first.channels)
    {
        return Err(MixError::FormatMismatch);
    }
    let len = inputs.iter().map(|(b, _)| b.samples.len()).max().unwrap_or(0);
    let mut out = AudioBuffer {
        sample_rate: first.sample_rate,
        channels: first.channels,
        samples: vec![0.0; len],
    };
    for (buf, gain) in inputs {
        for (o, s) in out.samples.iter_mut().zip(&buf.samples) {
            *o += s * gain;
        }
    }
    let peak = out.peak();
    if peak > 1.0 {
        for s in &mut out.samples {
            *s /= peak;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixes_with_gain_and_extends_to_longest() {
        let a = AudioBuffer { sample_rate: 44_100, channels: 1, samples: vec![0.5, 0.5] };
        let b = AudioBuffer { sample_rate: 44_100, channels: 1, samples: vec![0.25, 0.25, 0.25] };
        let m = mix(&[(&a, 1.0), (&b, 0.5)]).unwrap();
        assert_eq!(m.samples, vec![0.625, 0.625, 0.125]);
    }

    #[test]
    fn normalises_only_when_clipping() {
        let a = AudioBuffer { sample_rate: 44_100, channels: 1, samples: vec![0.75] };
        let m = mix(&[(&a, 1.0), (&a, 1.0)]).unwrap();
        assert!((m.peak() - 1.0).abs() < 1e-6);
    }
}
