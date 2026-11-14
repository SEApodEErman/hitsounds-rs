/// Interleaved f32 PCM.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn silent(sample_rate: u32, channels: u16, frames: usize) -> Self {
        Self {
            sample_rate,
            channels,
            samples: vec![0.0; frames * channels as usize],
        }
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_ms(&self) -> f64 {
        self.frames() as f64 * 1000.0 / f64::from(self.sample_rate.max(1))
    }

    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    /// Frame index of the loudest sample.
    pub fn peak_frame(&self) -> usize {
        let ch = self.channels.max(1) as usize;
        self.samples
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map_or(0, |(i, _)| i / ch)
    }

    /// The loudest channel of frame `i`, as an absolute value.
    pub fn frame_level(&self, i: usize) -> f32 {
        let ch = self.channels.max(1) as usize;
        self.samples[i * ch..(i + 1) * ch].iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    /// First frame whose level reaches `threshold`, if any.
    pub fn first_frame_above(&self, threshold: f32) -> Option<usize> {
        (0..self.frames()).find(|&i| self.frame_level(i) >= threshold)
    }

    /// Converts between mono and stereo (other layouts keep their first two channels).
    pub fn with_channels(&self, channels: u16) -> AudioBuffer {
        let from = self.channels.max(1) as usize;
        let to = channels.max(1) as usize;
        if from == to {
            return self.clone();
        }
        let mut samples = Vec::with_capacity(self.frames() * to);
        for frame in self.samples.chunks_exact(from) {
            match (from, to) {
                (1, _) => samples.extend(std::iter::repeat_n(frame[0], to)),
                (_, 1) => samples.push(frame.iter().sum::<f32>() / from as f32),
                _ => samples.extend((0..to).map(|c| frame.get(c).copied().unwrap_or(0.0))),
            }
        }
        AudioBuffer { sample_rate: self.sample_rate, channels, samples }
    }

    /// Drops the first `frames` frames.
    pub fn trim_start(&mut self, frames: usize) {
        let n = (frames * self.channels.max(1) as usize).min(self.samples.len());
        self.samples.drain(..n);
    }

    /// Appends silence until the buffer is at least `frames` long.
    pub fn pad_to(&mut self, frames: usize) {
        let want = frames * self.channels.max(1) as usize;
        if self.samples.len() < want {
            self.samples.resize(want, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_conversion() {
        let mono = AudioBuffer { sample_rate: 8, channels: 1, samples: vec![0.5, -0.5] };
        let stereo = mono.with_channels(2);
        assert_eq!(stereo.samples, vec![0.5, 0.5, -0.5, -0.5]);
        assert_eq!(stereo.with_channels(1), mono);
    }

    #[test]
    fn trim_and_pad() {
        let mut b = AudioBuffer { sample_rate: 8, channels: 2, samples: vec![0.0, 0.0, 0.1, 0.2, 0.3, 0.4] };
        assert_eq!(b.first_frame_above(0.05), Some(1));
        b.trim_start(1);
        assert_eq!(b.samples, vec![0.1, 0.2, 0.3, 0.4]);
        b.pad_to(4);
        assert_eq!(b.frames(), 4);
    }
}
