//! Waveform overview data for the timeline.

use crate::AudioBuffer;

/// Min/max of every `bucket_ms` slice, across channels, as signed bytes:
/// `[min0, max0, min1, max1, ...]`. Small enough to send to the UI in one piece
/// (a four-minute song at 2 ms is about 240 kB).
pub fn peaks(buf: &AudioBuffer, bucket_ms: f64) -> Vec<i8> {
    let channels = buf.channels.max(1) as usize;
    let frames = buf.frames();
    let per_bucket = (f64::from(buf.sample_rate) * bucket_ms / 1000.0).max(1.0);
    let buckets = (frames as f64 / per_bucket).ceil() as usize;
    let mut out = Vec::with_capacity(buckets * 2);
    for b in 0..buckets {
        let start = (b as f64 * per_bucket) as usize;
        let end = (((b + 1) as f64 * per_bucket) as usize).min(frames).max(start + 1).min(frames);
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for s in &buf.samples[start * channels..end * channels] {
            lo = lo.min(*s);
            hi = hi.max(*s);
        }
        out.push((lo.clamp(-1.0, 0.0) * 127.0).round() as i8);
        out.push((hi.clamp(0.0, 1.0) * 127.0).round() as i8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_min_max_pair_per_bucket() {
        let buf = AudioBuffer { sample_rate: 1000, channels: 1, samples: vec![0.5, -1.0, 0.0, 0.25, 1.0] };
        assert_eq!(peaks(&buf, 2.0), vec![-127, 64, 0, 32, 0, 127]);
    }
}
