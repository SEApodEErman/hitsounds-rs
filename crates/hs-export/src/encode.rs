//! Encoding rendered samples in the chosen export format (D-69): 16-bit or float WAV,
//! or Ogg Vorbis, plus deciding when a source file can be copied as it is.

use crate::wav::{encode_float32, encode_pcm16, wav_encoding};
use hs_audio::AudioBuffer;
use hs_core::project::SampleFormat;
use std::num::{NonZeroU8, NonZeroU32};
use std::path::Path;

/// libvorbis quality (-0.2 to 1.0). 0.6 is about 190 kbit/s for stereo at 44.1 kHz.
/// It sits a step above Mapping Tools' 0.5 so drum transients keep less pre-echo.
pub const VORBIS_QUALITY: f32 = 0.6;
/// A fixed Ogg stream serial, so the same audio always encodes to the same bytes and a
/// re-export doesn't count unchanged samples as changed.
const OGG_SERIAL: i32 = 0x4853_5354;
/// Frames handed to libvorbis per call ("1024 is a reasonable choice", libvorbis docs).
const BLOCK_FRAMES: usize = 1024;

/// What a source file on disk already is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Wav16,
    WavFloat,
    /// Any other WAV encoding (8/24-bit PCM, ADPCM, ...).
    WavOther,
    Ogg,
    /// MP3, FLAC: never copied, since the ranking criteria allow only WAV and OGG.
    Other,
}

impl SourceKind {
    pub fn of(path: &Path, bytes: &[u8]) -> Self {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("ogg") => Self::Ogg,
            Some("wav") => match wav_encoding(bytes) {
                Some((1, 16)) => Self::Wav16,
                Some((3, 32)) => Self::WavFloat,
                _ => Self::WavOther,
            },
            _ => Self::Other,
        }
    }

    /// Whether this file can be copied unchanged when `format` is asked for, as
    /// Mapping Tools' `IsCopyCompatible` decides.
    pub fn copies_as(self, format: SampleFormat) -> bool {
        match format {
            SampleFormat::Source => self != Self::Other,
            SampleFormat::Wav16 => self == Self::Wav16,
            SampleFormat::WavFloat => self == Self::WavFloat,
            SampleFormat::Ogg => self == Self::Ogg,
        }
    }

    /// The format a re-encoded copy of this source gets under [`SampleFormat::Source`].
    pub fn source_format(self) -> SampleFormat {
        match self {
            Self::Ogg => SampleFormat::Ogg,
            Self::WavFloat => SampleFormat::WavFloat,
            _ => SampleFormat::Wav16,
        }
    }
}

/// The file extension a format is written with.
pub fn extension(format: SampleFormat) -> &'static str {
    match format {
        SampleFormat::Ogg => "ogg",
        _ => "wav",
    }
}

/// Encodes `buf`. 16-bit WAV and Vorbis clip at full scale; float WAV does not.
/// `Source` encodes as 16-bit WAV (the caller resolves it per source first).
pub fn encode(buf: &AudioBuffer, format: SampleFormat) -> Result<Vec<u8>, String> {
    match format {
        SampleFormat::Source | SampleFormat::Wav16 => Ok(encode_pcm16(buf)),
        SampleFormat::WavFloat => Ok(encode_float32(buf)),
        SampleFormat::Ogg => encode_vorbis(buf).map_err(|e| format!("Ogg Vorbis encoding failed: {e}")),
    }
}

/// Ogg Vorbis at [`VORBIS_QUALITY`], same rate and channel count as `buf`.
///
/// libvorbis sets the final granule position to the input length, so a gapless decoder
/// (symphonia here, BASS in osu!) returns exactly the input frames with no added delay.
/// Mapping Tools' C# encoder needs a 1024-frame lead-in to get there; libvorbis doesn't,
/// and [`tests`] check it.
pub fn encode_vorbis(buf: &AudioBuffer) -> Result<Vec<u8>, vorbis_rs::VorbisError> {
    let channels = usize::from(buf.channels.max(1));
    let rate = NonZeroU32::new(buf.sample_rate).unwrap_or(NonZeroU32::MIN);
    let count = NonZeroU8::new(buf.channels.clamp(1, 255) as u8).unwrap_or(NonZeroU8::MIN);
    let mut encoder = vorbis_rs::VorbisEncoderBuilder::new_with_serial(rate, count, Vec::new(), OGG_SERIAL)
        .bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr {
            target_quality: VORBIS_QUALITY,
        })
        .build()?;
    let mut planes: Vec<Vec<f32>> = vec![Vec::with_capacity(BLOCK_FRAMES); channels];
    for frame in buf.samples.chunks(channels) {
        for (plane, sample) in planes.iter_mut().zip(frame) {
            plane.push(sample.clamp(-1.0, 1.0));
        }
        if planes[0].len() == BLOCK_FRAMES {
            encoder.encode_audio_block(&planes)?;
            planes.iter_mut().for_each(Vec::clear);
        }
    }
    if !planes[0].is_empty() {
        encoder.encode_audio_block(&planes)?;
    }
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::impact_delay_ms;

    /// A drum-like hit: `lead` frames of silence, then a decaying 200 Hz tone.
    fn hit(rate: u32, channels: u16, lead: usize, frames: usize) -> AudioBuffer {
        let mut samples = Vec::with_capacity(frames * usize::from(channels));
        for i in 0..frames {
            let v = if i < lead {
                0.0
            } else {
                let t = (i - lead) as f32 / rate as f32;
                0.9 * (-t * 30.0).exp() * (2.0 * std::f32::consts::PI * 200.0 * t).sin()
            };
            samples.extend(std::iter::repeat_n(v, usize::from(channels)));
        }
        AudioBuffer { sample_rate: rate, channels, samples }
    }

    fn first_loud(buf: &AudioBuffer) -> usize {
        buf.first_frame_above(0.05).unwrap()
    }

    #[test]
    fn vorbis_keeps_length_and_timing() {
        for (rate, channels) in [(44_100, 1), (44_100, 2), (48_000, 2), (22_050, 1)] {
            let lead = rate as usize / 100; // 10 ms
            let source = hit(rate, channels, lead, rate as usize / 4);
            let bytes = encode_vorbis(&source).unwrap();
            assert!(bytes.len() > 1024, "osu! plays files under 1 KB as silence");
            let decoded = hs_audio::decode_bytes(bytes, "ogg").unwrap();
            assert_eq!((decoded.sample_rate, decoded.channels), (rate, channels));
            assert_eq!(decoded.frames(), source.frames(), "{rate} Hz x{channels}: no padding or truncation");
            let shift = first_loud(&decoded) as i64 - first_loud(&source) as i64;
            assert!(shift.abs() <= 2, "{rate} Hz x{channels}: onset moved {shift} frames");
            assert!((impact_delay_ms(&decoded) - impact_delay_ms(&source)).abs() < 0.5);
        }
    }

    #[test]
    fn vorbis_handles_a_sample_shorter_than_one_block() {
        let source = hit(44_100, 1, 0, 300);
        let decoded = hs_audio::decode_bytes(encode_vorbis(&source).unwrap(), "ogg").unwrap();
        assert_eq!(decoded.frames(), 300);
    }

    #[test]
    fn vorbis_is_deterministic() {
        let source = hit(44_100, 2, 100, 4_000);
        assert_eq!(encode_vorbis(&source).unwrap(), encode_vorbis(&source).unwrap());
    }

    #[test]
    fn copy_rules_follow_mapping_tools() {
        use SampleFormat::*;
        let pcm = SourceKind::of(Path::new("k.WAV"), &encode_pcm16(&hit(44_100, 1, 0, 10)));
        assert_eq!(pcm, SourceKind::Wav16);
        assert!(pcm.copies_as(Source) && pcm.copies_as(Wav16));
        assert!(!pcm.copies_as(Ogg) && !pcm.copies_as(WavFloat));
        assert!(SourceKind::Ogg.copies_as(Source) && SourceKind::Ogg.copies_as(Ogg));
        assert!(SourceKind::WavOther.copies_as(Source) && !SourceKind::WavOther.copies_as(Wav16));
        assert!(!SourceKind::of(Path::new("k.mp3"), &[]).copies_as(Source), "MP3 is never a hitsound");
        assert_eq!(SourceKind::Ogg.source_format(), Ogg);
        assert_eq!(SourceKind::Other.source_format(), Wav16);
    }
}
