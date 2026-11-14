//! Ranking-criteria checks for exported hitsound files (D-12: warn + auto-fix).

use hs_audio::AudioBuffer;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SampleIssue {
    /// RC: every hitsound file must be at least 25 ms long.
    TooShort { ms: f64 },
    /// RC: a clicked hitsound must have no perceivable delay (5 ms or less). `ms` is
    /// how long the sample takes to first reach half of its peak level.
    LatePeak { ms: f64 },
    /// RC: completely silent files must be the standard 44-byte file.
    SilentNotBlank,
    /// osu!stable loads a sample file under 1024 bytes as silence (findings 2).
    TooSmall { bytes: usize },
}

/// What [`auto_fix`] changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SampleFix {
    TrimmedLeadingSilence { ms: f64 },
    PaddedTo { ms: f64 },
}

pub const MIN_LENGTH_MS: f64 = 25.0;
/// Smaller sample files play as silence in osu!stable (findings §2).
pub const MIN_AUDIBLE_BYTES: usize = 1024;
pub const MAX_PEAK_DELAY_MS: f64 = 5.0;
/// The impact is taken to arrive when the signal first reaches this share of its peak.
/// Modelled on Mapset Verifier's delay check; the exact share is **unverified**.
const IMPACT_SHARE: f32 = 0.5;
/// Below this share of the peak (-60 dB) the start of a sample counts as silence.
const SILENCE_SHARE: f32 = 0.001;

fn frames_to_ms(buf: &AudioBuffer, frames: usize) -> f64 {
    frames as f64 * 1000.0 / f64::from(buf.sample_rate.max(1))
}

/// Time until the signal first reaches half of its peak level.
pub fn impact_delay_ms(buf: &AudioBuffer) -> f64 {
    let peak = buf.peak();
    buf.first_frame_above(peak * IMPACT_SHARE).map_or(0.0, |f| frames_to_ms(buf, f))
}

/// Checks one rendered sample. `is_silent_slot` = intentionally silent (write the blank file).
pub fn check_sample(buf: &AudioBuffer, is_silent_slot: bool) -> Vec<SampleIssue> {
    let mut issues = Vec::new();
    if buf.peak() == 0.0 {
        if !is_silent_slot {
            issues.push(SampleIssue::SilentNotBlank);
        }
        return issues;
    }
    let ms = buf.duration_ms();
    if ms < MIN_LENGTH_MS {
        issues.push(SampleIssue::TooShort { ms });
    }
    let delay = impact_delay_ms(buf);
    if delay > MAX_PEAK_DELAY_MS {
        issues.push(SampleIssue::LatePeak { ms: delay });
    }
    issues
}

/// Applies the fixes that cannot change how a sample sounds: cut silence in front of
/// a late impact, and pad a short file with silence up to 25 ms.
pub fn auto_fix(buf: &mut AudioBuffer) -> Vec<SampleFix> {
    let mut fixes = Vec::new();
    let peak = buf.peak();
    if peak == 0.0 {
        return fixes;
    }
    if impact_delay_ms(buf) > MAX_PEAK_DELAY_MS
        && let Some(lead) = buf.first_frame_above(peak * SILENCE_SHARE)
        && lead > 0
    {
        fixes.push(SampleFix::TrimmedLeadingSilence { ms: frames_to_ms(buf, lead) });
        buf.trim_start(lead);
    }
    if buf.duration_ms() < MIN_LENGTH_MS {
        let frames = (MIN_LENGTH_MS * f64::from(buf.sample_rate) / 1000.0).ceil() as usize;
        buf.pad_to(frames);
        fixes.push(SampleFix::PaddedTo { ms: buf.duration_ms() });
    }
    fixes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_short_and_late_samples() {
        let mut b = AudioBuffer::silent(1000, 1, 20);
        b.samples[10] = 1.0;
        let issues = check_sample(&b, false);
        assert!(issues.contains(&SampleIssue::TooShort { ms: 20.0 }));
        assert!(issues.contains(&SampleIssue::LatePeak { ms: 10.0 }));
    }

    #[test]
    fn auto_fix_trims_leading_silence_and_pads() {
        let mut b = AudioBuffer::silent(1000, 1, 20);
        b.samples[10] = 1.0;
        let fixes = auto_fix(&mut b);
        assert_eq!(
            fixes,
            vec![SampleFix::TrimmedLeadingSilence { ms: 10.0 }, SampleFix::PaddedTo { ms: 25.0 }]
        );
        assert_eq!(b.samples[0], 1.0);
        assert!(check_sample(&b, false).is_empty());
    }

    #[test]
    fn slow_attack_without_silence_is_reported_but_not_cut() {
        // Ramps up over 20 ms: late, but there is no silence to remove.
        let mut b = AudioBuffer { sample_rate: 1000, channels: 1, samples: (1..=40).map(|i| (i as f32 / 20.0).min(1.0)).collect() };
        let before = b.clone();
        assert!(auto_fix(&mut b).is_empty());
        assert_eq!(b, before);
        assert!(matches!(check_sample(&b, false).as_slice(), [SampleIssue::LatePeak { .. }]));
    }

    #[test]
    fn silence_is_only_fine_for_a_silent_slot() {
        let b = AudioBuffer::silent(44_100, 1, 100);
        assert_eq!(check_sample(&b, false), vec![SampleIssue::SilentNotBlank]);
        assert!(check_sample(&b, true).is_empty());
    }
}
