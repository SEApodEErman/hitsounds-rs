//! Demucs' spectrogram helpers on a torch-compatible STFT.
//!
//! Ported from Demucs (MIT, github.com/adefossez/demucs; see THIRD_PARTY_NOTICES.md):
//! `demucs/spec.py` (`spectro`, `ispectro`) and `demucs/htdemucs.py`
//! (`HTDemucs._spec`, `_ispec`, `_magnitude`, `_mask` with `cac`). The NumPy twin is
//! `tools/demucs-export/reference.py`, which is checked against PyTorch.
//!
//! [`stft`]/[`istft`] match `torch.stft`/`torch.istft` with `normalized=True`,
//! `center=True`, reflect padding and a periodic Hann window. [`demucs_spec`] and
//! [`demucs_ispec`] add Demucs' extra padding and trimming; that pair is lossy on
//! purpose (it drops the Nyquist bin and the outer frames), so only the plain pair
//! round-trips.

use realfft::RealFftPlanner;
use realfft::num_complex::Complex32;

pub const NFFT: usize = 4096;
pub const HOP: usize = NFFT / 4;
/// Bins of the plain STFT.
pub const FULL_BINS: usize = NFFT / 2 + 1;
/// Bins Demucs keeps (the Nyquist bin is dropped).
pub const BINS: usize = NFFT / 2;
/// `HTDemucs._spec` pads by 1.5 hops on each side before the STFT.
const DEMUCS_PAD: usize = HOP / 2 * 3;

fn hann() -> Vec<f32> {
    // torch.hann_window is periodic by default.
    (0..NFFT)
        .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / NFFT as f64).cos()) as f32)
        .collect()
}

/// numpy/torch "reflect" padding (the edge sample is not repeated). Needs `left` and
/// `right` shorter than `x`; Demucs segments are far longer than any pad used here.
fn reflect_pad(x: &[f32], left: usize, right: usize) -> Vec<f32> {
    let n = x.len();
    assert!(
        left < n && right < n,
        "reflect padding of {left}/{right} needs more than {n} samples"
    );
    let mut out = Vec::with_capacity(left + n + right);
    out.extend((0..left).map(|i| x[left - i]));
    out.extend_from_slice(x);
    out.extend((0..right).map(|k| x[n - 2 - k]));
    out
}

/// Frame-major spectrum: `frames × FULL_BINS`, frame `t` at `[t * FULL_BINS..]`.
pub struct Spectrum {
    pub frames: usize,
    pub data: Vec<Complex32>,
}

/// `torch.stft(x, NFFT, HOP, hann, normalized=True, center=True, pad_mode="reflect")`.
pub fn stft(x: &[f32]) -> Spectrum {
    let padded = reflect_pad(x, NFFT / 2, NFFT / 2);
    let frames = 1 + (padded.len() - NFFT) / HOP;
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(NFFT);
    let window = hann();
    let scale = 1.0 / (NFFT as f32).sqrt();
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let mut data = Vec::with_capacity(frames * FULL_BINS);
    for t in 0..frames {
        let frame = &padded[t * HOP..t * HOP + NFFT];
        for ((slot, &s), &w) in input.iter_mut().zip(frame).zip(&window) {
            *slot = s * w;
        }
        fft.process_with_scratch(&mut input, &mut output, &mut scratch)
            .expect("buffer sizes come from the planner");
        data.extend(output.iter().map(|c| c * scale));
    }
    Spectrum { frames, data }
}

/// `torch.istft(z, NFFT, HOP, hann, normalized=True, center=True, length)`: the
/// inverse of [`stft`]. Frames may be `None` for an all-zero frame; it still counts
/// toward the window envelope, as in torch.
pub fn istft(frames: &[Option<&[Complex32]>], length: usize) -> Vec<f32> {
    let ifft = RealFftPlanner::<f32>::new().plan_fft_inverse(NFFT);
    let window = hann();
    // realfft's inverse is unnormalised; torch divides by NFFT and `normalized`
    // multiplies by sqrt(NFFT).
    let scale = 1.0 / (NFFT as f32).sqrt();
    let total = NFFT + HOP * (frames.len().saturating_sub(1));
    let mut out = vec![0.0f32; total];
    let mut env = vec![0.0f32; total];
    let mut spectrum = ifft.make_input_vec();
    let mut signal = ifft.make_output_vec();
    let mut scratch = ifft.make_scratch_vec();
    for (t, frame) in frames.iter().enumerate() {
        let at = t * HOP;
        for (e, &w) in env[at..at + NFFT].iter_mut().zip(&window) {
            *e += w * w;
        }
        let Some(frame) = frame else { continue };
        spectrum.copy_from_slice(&frame[..FULL_BINS]);
        // irfft ignores the imaginary parts of DC and Nyquist; realfft insists they are 0.
        spectrum[0].im = 0.0;
        spectrum[FULL_BINS - 1].im = 0.0;
        ifft.process_with_scratch(&mut spectrum, &mut signal, &mut scratch)
            .expect("DC and Nyquist are real and sizes come from the planner");
        for ((o, &s), &w) in out[at..at + NFFT].iter_mut().zip(&signal).zip(&window) {
            *o += s * w * scale;
        }
    }
    (0..length)
        .map(|k| {
            let i = NFFT / 2 + k;
            match (out.get(i), env.get(i)) {
                (Some(&o), Some(&e)) if e > 1e-11 => o / e,
                (Some(&o), Some(_)) => o,
                _ => 0.0, // torch zero-pads past the last frame
            }
        })
        .collect()
}

/// Bin-major Demucs spectrogram of one channel: `BINS × frames`, bin `b` at
/// `[b * frames..]`.
pub struct DemucsSpec {
    pub frames: usize,
    pub data: Vec<Complex32>,
}

/// `HTDemucs._spec` for one channel: `ceil(len / HOP)` frames of `BINS` bins.
pub fn demucs_spec(x: &[f32]) -> DemucsSpec {
    let n = x.len();
    let le = n.div_ceil(HOP);
    let padded = reflect_pad(x, DEMUCS_PAD, DEMUCS_PAD + le * HOP - n);
    let full = stft(&padded);
    debug_assert_eq!(full.frames, le + 4);
    let mut data = vec![Complex32::default(); BINS * le];
    for t in 0..le {
        let frame = &full.data[(t + 2) * FULL_BINS..(t + 3) * FULL_BINS];
        for b in 0..BINS {
            data[b * le + t] = frame[b];
        }
    }
    DemucsSpec { frames: le, data }
}

/// `HTDemucs._ispec` for one channel: back to `length` samples.
pub fn demucs_ispec(z: &DemucsSpec, length: usize) -> Vec<f32> {
    let t_len = z.frames;
    // Re-add the Nyquist bin (zero) and transpose to frame-major.
    let mut frames = vec![Complex32::default(); t_len * FULL_BINS];
    for b in 0..BINS {
        for t in 0..t_len {
            frames[t * FULL_BINS + b] = z.data[b * t_len + t];
        }
    }
    // Two zero frames on each side, as `F.pad(z, (2, 2))`.
    let mut list: Vec<Option<&[Complex32]>> = vec![None, None];
    list.extend(
        frames
            .as_chunks::<FULL_BINS>()
            .0
            .iter()
            .map(|f| Some(&f[..])),
    );
    list.extend([None, None]);
    let le = HOP * length.div_ceil(HOP) + 2 * DEMUCS_PAD;
    let x = istft(&list, le);
    x[DEMUCS_PAD..DEMUCS_PAD + length].to_vec()
}

/// `HTDemucs._magnitude` with `cac`: two channels' spectra as four real planes, in the
/// order re0, im0, re1, im1, each `BINS × frames`.
pub fn to_cac(left: &DemucsSpec, right: &DemucsSpec) -> Vec<f32> {
    debug_assert_eq!(left.frames, right.frames);
    let plane = left.data.len();
    let mut out = vec![0.0f32; 4 * plane];
    for (c, spec) in [left, right].into_iter().enumerate() {
        for (i, z) in spec.data.iter().enumerate() {
            out[(2 * c) * plane + i] = z.re;
            out[(2 * c + 1) * plane + i] = z.im;
        }
    }
    out
}

/// `HTDemucs._mask` with `cac`: four real planes back to two channels' spectra.
pub fn from_cac(planes: &[f32], frames: usize) -> [DemucsSpec; 2] {
    let plane = BINS * frames;
    assert_eq!(
        planes.len(),
        4 * plane,
        "expected four planes of {BINS} × {frames}"
    );
    let channel = |c: usize| DemucsSpec {
        frames,
        data: (0..plane)
            .map(|i| Complex32::new(planes[(2 * c) * plane + i], planes[(2 * c + 1) * plane + i]))
            .collect(),
    };
    [channel(0), channel(1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        // Small LCG: deterministic, no extra dependency.
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    fn low_sines(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / 44_100.0;
                0.3 * (std::f32::consts::TAU * 110.0 * t).sin()
                    + 0.2 * (std::f32::consts::TAU * 1_234.5 * t).sin()
            })
            .collect()
    }

    #[test]
    fn reflect_pad_matches_numpy() {
        // np.pad([1, 2, 3, 4], (2, 3), mode="reflect") == [3, 2, 1, 2, 3, 4, 3, 2, 1]
        assert_eq!(
            reflect_pad(&[1.0, 2.0, 3.0, 4.0], 2, 3),
            [3.0, 2.0, 1.0, 2.0, 3.0, 4.0, 3.0, 2.0, 1.0]
        );
    }

    #[test]
    fn plain_stft_round_trips() {
        let x = noise(50_000, 7);
        let s = stft(&x);
        let frames: Vec<_> = s
            .data
            .as_chunks::<FULL_BINS>()
            .0
            .iter()
            .map(|f| Some(&f[..]))
            .collect();
        let y = istft(&frames, x.len());
        let err = x
            .iter()
            .zip(&y)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(err < 1e-5, "round trip error {err}");
    }

    #[test]
    fn stft_is_torch_normalised() {
        // A full-scale DC signal: every interior frame's DC bin is sum(window)/sqrt(NFFT)
        // = (NFFT/2)/sqrt(NFFT) = 32 for NFFT = 4096.
        let s = stft(&vec![1.0; 20_000]);
        let dc = s.data[5 * FULL_BINS];
        assert!(
            (dc.re - 32.0).abs() < 1e-3 && dc.im.abs() < 1e-3,
            "DC bin {dc}"
        );
    }

    #[test]
    fn demucs_spec_has_one_frame_per_hop() {
        assert_eq!(demucs_spec(&noise(343_980, 1)).frames, 336);
        assert_eq!(demucs_spec(&noise(10 * HOP, 1)).frames, 10);
    }

    #[test]
    fn demucs_pair_is_exact_away_from_edges_without_nyquist_content() {
        let x = low_sines(100_000);
        let y = demucs_ispec(&demucs_spec(&x), x.len());
        assert_eq!(y.len(), x.len());
        let err = x[NFFT..x.len() - NFFT]
            .iter()
            .zip(&y[NFFT..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(err < 1e-4, "interior error {err}");
    }

    #[test]
    fn cac_order_and_round_trip() {
        let a = demucs_spec(&noise(8 * HOP, 2));
        let b = demucs_spec(&noise(8 * HOP, 3));
        let planes = to_cac(&a, &b);
        let plane = BINS * a.frames;
        // re0, im0, re1, im1
        assert_eq!(planes[5], a.data[5].re);
        assert_eq!(planes[plane + 5], a.data[5].im);
        assert_eq!(planes[2 * plane + 5], b.data[5].re);
        assert_eq!(planes[3 * plane + 5], b.data[5].im);
        let [a2, b2] = from_cac(&planes, a.frames);
        assert_eq!(a2.data, a.data);
        assert_eq!(b2.data, b.data);
    }
}
