//! Pitch-preserving time stretch for slowed playback (D-90), by WSOLA.
//!
//! The song is played as short grains that overlap by a few milliseconds. Each
//! grain is placed where the song clock says it belongs, then moved within a small
//! search window to the spot whose waveform best continues the previous grain, and
//! the two are crossfaded. Samples always play at their own rate, so the pitch is
//! kept; slowing down only shortens the step between grains. The cost is one
//! cross-correlation search per grain (about 40 a second).
//!
//! The whole song is in memory, so grains read the buffer directly, and each one is
//! placed from the clock rather than from the previous grain. The music therefore
//! never drifts from the hitsounds: it wanders by at most half the search window.
//!
//! At 1x, or with stretching turned off, [`Stretcher::frame`] is the plain
//! interpolated read the mixer always used. Switching between the two crossfades,
//! so changing speed does not click.

use crate::AudioBuffer;

/// Grain length, including the crossfade.
const SEQUENCE_MS: f64 = 30.0;
/// Crossfade between grains.
const OVERLAP_MS: f64 = 4.0;
/// Search window at 0.5x and slower. More of each grain repeats the further the song
/// is slowed, so the search is wider there.
const SEEK_SLOW_MS: f64 = 25.0;
/// Search window at 2x and faster.
const SEEK_FAST_MS: f64 = 15.0;

pub struct Stretcher {
    sample_rate: f64,
    overlap: usize,
    /// Output frames per grain: the grain length less the crossfade.
    hop: usize,
    state: State,
    /// The weighted end of the previous grain, interleaved stereo.
    reference: Vec<f32>,
    /// The candidate source frames for the next grain, interleaved stereo.
    region: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
enum State {
    /// Plain interpolated read.
    Direct,
    /// A grain starts on the next frame. `cont` is where the previous one would
    /// have gone on, which the new grain fades in over.
    Boundary { cont: isize },
    /// `j` frames into the grain that starts at source frame `start`.
    Grain { start: isize, cont: isize, j: usize },
    /// Crossfading from the last grain's continuation to the plain read.
    Handoff { cont: isize, j: usize },
}

impl Stretcher {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = f64::from(sample_rate.max(1));
        let frames = |ms: f64| ((ms * sample_rate / 1000.0).round() as usize).max(1);
        let overlap = frames(OVERLAP_MS);
        let hop = frames(SEQUENCE_MS).saturating_sub(overlap).max(overlap);
        let seek = frames(SEEK_SLOW_MS);
        Self {
            sample_rate,
            overlap,
            hop,
            state: State::Direct,
            reference: Vec::with_capacity(overlap * 2),
            // Room for the widest window, so the audio thread never allocates.
            region: Vec::with_capacity((seek + 2 + overlap) * 2),
        }
    }

    /// Forgets the grain in progress, after a seek or a new song.
    pub fn reset(&mut self) {
        self.state = State::Direct;
    }

    /// The song's stereo frame at clock position `at` (source frames, fractional),
    /// advancing one output frame. `buf` must be stereo. `stretch` = keep the pitch
    /// when `rate` is not 1.
    pub fn frame(&mut self, buf: &AudioBuffer, at: f64, rate: f64, stretch: bool) -> [f32; 2] {
        let wanted = stretch && rate > 0.0 && (rate - 1.0).abs() > 1e-9;
        if let State::Direct = self.state {
            if !wanted {
                return direct(buf, at);
            }
            // Start from where the plain read is, so the first grain fades in from it.
            self.state = State::Boundary { cont: at.round() as isize };
        }
        if let State::Boundary { cont } = self.state {
            self.state = if wanted {
                // Centre the grain on the clock: over a grain the source runs at 1x
                // while the clock runs at `rate`, so start it half that gap early.
                let target = at - (1.0 - rate) * self.hop as f64 / 2.0;
                State::Grain { start: self.search(buf, cont, target, rate), cont, j: 0 }
            } else {
                State::Handoff { cont, j: 0 }
            };
        }
        match self.state {
            State::Grain { start, cont, j } => {
                let new = read(buf, start + j as isize);
                let out = if j < self.overlap {
                    crossfade(read(buf, cont + j as isize), new, self.fade(j))
                } else {
                    new
                };
                self.state = if j + 1 == self.hop {
                    State::Boundary { cont: start + self.hop as isize }
                } else {
                    State::Grain { start, cont, j: j + 1 }
                };
                out
            }
            State::Handoff { cont, j } => {
                let out = crossfade(read(buf, cont + j as isize), direct(buf, at), self.fade(j));
                self.state = if j + 1 >= self.overlap {
                    State::Direct
                } else {
                    State::Handoff { cont, j: j + 1 }
                };
                out
            }
            State::Direct | State::Boundary { .. } => direct(buf, at),
        }
    }

    fn fade(&self, j: usize) -> f32 {
        (j as f32 + 0.5) / self.overlap as f32
    }

    fn seek_frames(&self, rate: f64) -> usize {
        let slope = (SEEK_FAST_MS - SEEK_SLOW_MS) / 1.5;
        let ms = (SEEK_SLOW_MS + (rate - 0.5) * slope).clamp(SEEK_FAST_MS, SEEK_SLOW_MS);
        (ms * self.sample_rate / 1000.0).round() as usize
    }

    /// The grain start near `target` whose opening best matches the source after
    /// `cont`, by normalised cross-correlation over the crossfade.
    fn search(&mut self, buf: &AudioBuffer, cont: isize, target: f64, rate: f64) -> isize {
        let overlap = self.overlap;
        let seek = self.seek_frames(rate);
        let lo = (target - seek as f64 / 2.0).round() as isize;
        let nearest = target.round() as isize;

        // Weight the middle of the crossfade most, where both grains are loudest.
        self.reference.clear();
        let mut reference_energy = 0.0f64;
        for i in 0..overlap {
            let w = (i * (overlap - i)) as f32;
            let [l, r] = read(buf, cont + i as isize);
            self.reference.extend([l * w, r * w]);
            reference_energy += f64::from(l * l + r * r);
        }
        if reference_energy < 1e-12 {
            // Silence has nothing to line up; keep to the clock.
            return nearest;
        }

        self.region.clear();
        for i in 0..seek + 1 + overlap {
            self.region.extend(read(buf, lo + i as isize));
        }
        let mut energy: f64 = self.region[..overlap * 2].iter().map(|s| f64::from(s * s)).sum();
        let mut best = (nearest, f64::NEG_INFINITY);
        for k in 0..=seek {
            let window = &self.region[k * 2..(k + overlap) * 2];
            let dot: f32 = dot(&self.reference, window);
            let score = f64::from(dot) / (energy.max(0.0) + 1e-9).sqrt();
            if score > best.1 {
                best = (lo + k as isize, score);
            }
            if k < seek {
                let (out, inc) = (k * 2, (k + overlap) * 2);
                let sq = |i: usize| f64::from(self.region[i] * self.region[i]);
                energy += sq(inc) + sq(inc + 1) - sq(out) - sq(out + 1);
            }
        }
        best.0
    }
}

/// Four running sums, so the compiler can vectorise the search.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 4];
    let (a4, a_rest) = a.as_chunks::<4>();
    let (b4, b_rest) = b.as_chunks::<4>();
    for (x, y) in a4.iter().zip(b4) {
        for i in 0..4 {
            acc[i] += x[i] * y[i];
        }
    }
    let tail: f32 = a_rest.iter().zip(b_rest).map(|(x, y)| x * y).sum();
    acc.iter().sum::<f32>() + tail
}

fn read(buf: &AudioBuffer, frame: isize) -> [f32; 2] {
    match usize::try_from(frame) {
        Ok(i) if i < buf.frames() => [buf.samples[i * 2], buf.samples[i * 2 + 1]],
        _ => [0.0; 2],
    }
}

/// Linear interpolation at a fractional frame; silent before the start.
fn direct(buf: &AudioBuffer, at: f64) -> [f32; 2] {
    let total = buf.frames();
    if at < 0.0 || total == 0 {
        return [0.0; 2];
    }
    let i0 = at as usize;
    if i0 >= total {
        return [0.0; 2];
    }
    let frac = (at - i0 as f64) as f32;
    let i1 = (i0 + 1).min(total - 1);
    let mut out = [0.0; 2];
    for (c, o) in out.iter_mut().enumerate() {
        let a = buf.samples[i0 * 2 + c];
        let b = buf.samples[i1 * 2 + c];
        *o = a + (b - a) * frac;
    }
    out
}

fn crossfade(from: [f32; 2], to: [f32; 2], w: f32) -> [f32; 2] {
    [from[0] + (to[0] - from[0]) * w, from[1] + (to[1] - from[1]) * w]
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(hz: f32, seconds: f32) -> AudioBuffer {
        let frames = (seconds * RATE as f32) as usize;
        let samples = (0..frames)
            .flat_map(|i| {
                let s = (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin() * 0.5;
                [s, s]
            })
            .collect();
        AudioBuffer { sample_rate: RATE, channels: 2, samples }
    }

    /// Plays `buf` from 0 with the clock at `rate` per output frame; left channel.
    fn play(s: &mut Stretcher, buf: &AudioBuffer, from: f64, rate: f64, stretch: bool, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| s.frame(buf, from + i as f64 * rate, rate, stretch)[0]).collect()
    }

    fn crossings(out: &[f32]) -> usize {
        out.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }

    fn max_step(out: &[f32]) -> f32 {
        out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn slowed_sine_keeps_its_pitch() {
        let buf = sine(441.0, 3.0);
        for rate in [0.25, 0.5, 0.75] {
            let mut s = Stretcher::new(RATE);
            let out = play(&mut s, &buf, 0.0, rate, true, RATE as usize);
            // 441 Hz has 882 zero crossings a second at any speed.
            let n = crossings(&out) as i64;
            assert!((n - 882).abs() <= 8, "{rate}x: {n} crossings");
        }
    }

    #[test]
    fn without_stretch_the_pitch_drops_with_the_speed() {
        let buf = sine(441.0, 2.0);
        let mut s = Stretcher::new(RATE);
        let n = crossings(&play(&mut s, &buf, 0.0, 0.5, false, RATE as usize)) as i64;
        assert!((n - 441).abs() <= 2, "{n} crossings");
    }

    #[test]
    fn at_full_speed_it_is_the_plain_read() {
        let buf = sine(300.0, 0.2);
        let mut s = Stretcher::new(RATE);
        let out = play(&mut s, &buf, 0.0, 1.0, true, buf.frames());
        let left: Vec<f32> = buf.samples.iter().step_by(2).copied().collect();
        assert_eq!(out, left);
    }

    #[test]
    fn grains_and_speed_changes_do_not_click() {
        // A clean 100 Hz sine moves at most ~0.0065 per sample; a click jumps far more.
        let buf = sine(100.0, 4.0);
        let mut s = Stretcher::new(RATE);
        let mut out = play(&mut s, &buf, 0.0, 1.0, true, 4_800);
        out.extend(play(&mut s, &buf, 4_800.0, 0.5, true, 48_000));
        out.extend(play(&mut s, &buf, 28_800.0, 1.0, true, 4_800));
        out.extend(play(&mut s, &buf, 33_600.0, 0.25, true, 48_000));
        out.extend(play(&mut s, &buf, 45_600.0, 0.25, false, 4_800));
        assert!(max_step(&out) < 0.02, "max step {}", max_step(&out));
    }

    #[test]
    fn music_stays_on_the_clock() {
        // Silence, then a tone from 500 ms. At 0.5x the clock reaches it after 1 s of
        // output; the stretched tone must start within the search window of that.
        let mut buf = sine(441.0, 2.0);
        buf.samples[..RATE as usize].fill(0.0);
        let mut s = Stretcher::new(RATE);
        let out = play(&mut s, &buf, 0.0, 0.5, true, 2 * RATE as usize);
        let onset = out.iter().position(|v| v.abs() > 0.01).unwrap();
        let ms = onset as f64 * 1000.0 / f64::from(RATE);
        assert!((ms - 1000.0).abs() < 20.0, "tone starts at {ms} ms of output");
    }
}
