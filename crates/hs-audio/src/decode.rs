//! Decoding audio files (mp3, ogg vorbis, wav, flac) into [`AudioBuffer`]s.

use crate::AudioBuffer;
use std::fs::File;
use std::path::Path;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::audio::well_known::CODEC_ID_MP3;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("cannot open {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot decode {path}: {message}")]
    Decode { path: String, message: String },
}

/// Leading and trailing frames described by an MP3's gapless metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp3Trim {
    pub start: usize,
    pub end: usize,
}

/// Decodes a whole file to interleaved f32, with codec delay and padding trimmed
/// when signalled in the file (D-26, findings §11).
pub fn decode_file(path: &Path) -> Result<AudioBuffer, DecodeError> {
    decode_file_with_mp3_trim(path).map(|(audio, _)| audio)
}

/// Also returns the MP3 trim so playback can recognise a stem generated from the
/// older, untrimmed MP3 decode (D-73).
pub fn decode_file_with_mp3_trim(path: &Path) -> Result<(AudioBuffer, Option<Mp3Trim>), DecodeError> {
    let shown = path.display().to_string();
    let file = File::open(path).map_err(|source| DecodeError::Open { path: shown.clone(), source })?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    decode(MediaSourceStream::new(Box::new(file), Default::default()), ext, shown)
}

/// Decodes an in-memory file, e.g. an encoded sample before it is written, so export
/// can check what osu! will actually hear. `ext` (`"ogg"`, `"wav"`) is a format hint.
pub fn decode_bytes(bytes: Vec<u8>, ext: &str) -> Result<AudioBuffer, DecodeError> {
    let source = std::io::Cursor::new(bytes);
    decode(
        MediaSourceStream::new(Box::new(source), Default::default()),
        Some(ext.to_ascii_lowercase()),
        format!("<{ext} data>"),
    )
    .map(|(audio, _)| audio)
}

fn decode(
    mss: MediaSourceStream,
    ext: Option<String>,
    shown: String,
) -> Result<(AudioBuffer, Option<Mp3Trim>), DecodeError> {
    let fail = |message: String| DecodeError::Decode { path: shown.clone(), message };
    let mut hint = Hint::new();
    if let Some(ext) = &ext {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| fail(e.to_string()))?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| fail("no audio track".into()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| fail("no audio codec parameters".into()))?;
    let start = track.delay.unwrap_or(0) as usize;
    let end = track.padding.unwrap_or(0) as usize;
    let mp3_trim =
        (params.codec == CODEC_ID_MP3 && (start > 0 || end > 0)).then_some(Mp3Trim { start, end });
    // BASS with MP3_OLDGAPS still honours Xing/Info delay and padding metadata.
    // With no recognised tag, the MP3 demuxer supplies no trim (findings §11).
    let options = AudioDecoderOptions::default().gapless(true);
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &options)
        .map_err(|e| fail(e.to_string()))?;

    let mut out = AudioBuffer { sample_rate: 0, channels: 0, samples: Vec::new() };
    let mut chunk: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            // Truncated files are common among hitsound samples; keep what decoded.
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(fail(e.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                if out.sample_rate == 0 {
                    out.sample_rate = buf.spec().rate();
                    out.channels = buf.spec().channels().count() as u16;
                }
                chunk.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut chunk);
                out.samples.extend_from_slice(&chunk);
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(fail(e.to_string())),
        }
    }
    if out.sample_rate == 0 || out.channels == 0 {
        // A header with no audio data, e.g. the 44-byte blank WAV.
        return Ok((AudioBuffer::silent(44_100, 2, 0), None));
    }
    Ok((out, mp3_trim))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 16-bit PCM WAV, built by hand so the test needs no fixture file.
    fn wav_bytes(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hs-audio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn decodes_pcm_wav() {
        let path = temp_file("stereo.wav", &wav_bytes(22_050, 2, &[0, 16_384, -16_384, 32_767]));
        let buf = decode_file(&path).unwrap();
        assert_eq!((buf.sample_rate, buf.channels, buf.frames()), (22_050, 2, 2));
        assert!((buf.samples[1] - 0.5).abs() < 1e-3 && (buf.samples[2] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn blank_wav_decodes_to_empty() {
        let path = temp_file("blank.wav", &wav_bytes(44_100, 1, &[]));
        assert_eq!(decode_file(&path).unwrap().frames(), 0);
    }

    #[test]
    fn mp3_trims_only_the_delay_and_padding_in_the_tag() {
        // Synthetic 40 ms, 440 Hz sine encoded with libmp3lame at 44.1 kHz.
        // The first 208 bytes are the Info frame carrying the gapless metadata.
        let tagged = include_bytes!("../tests/fixtures/gapless-sine.mp3");
        let path = temp_file("tagged.mp3", tagged);
        let (audio, trim) = decode_file_with_mp3_trim(&path).unwrap();
        assert_eq!(trim, Some(Mp3Trim { start: 1105, end: 587 }));
        assert_eq!(
            (audio.sample_rate, audio.channels, audio.frames()),
            (44_100, 1, 1764)
        );

        let untagged = temp_file("untagged.mp3", &tagged[208..]);
        let (padded, trim) = decode_file_with_mp3_trim(&untagged).unwrap();
        assert_eq!(trim, None);
        assert_eq!(padded.frames(), 3456);
        assert_eq!(audio.samples, padded.samples[1105..1105 + 1764]);
    }

    #[test]
    fn missing_file_is_an_open_error() {
        let err = decode_file(Path::new("definitely/not/here.wav")).unwrap_err();
        assert!(matches!(err, DecodeError::Open { .. }));
    }
}
