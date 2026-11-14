//! 16-bit PCM WAV writing (hitsounds must be WAV or OGG; WAV first).

use hs_audio::AudioBuffer;

/// A valid 44-byte WAV with no audio data: what the ranking criteria require for
/// completely silent hitsound files.
pub fn blank_wav() -> Vec<u8> {
    encode_pcm16(&AudioBuffer::silent(44_100, 1, 0))
}

pub fn encode_pcm16(buf: &AudioBuffer) -> Vec<u8> {
    let channels = buf.channels.max(1);
    let data_len = (buf.samples.len() * 2) as u32;
    let byte_rate = buf.sample_rate * u32::from(channels) * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&buf.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in &buf.samples {
        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// 32-bit IEEE float WAV (format tag 3). Values are written unclipped.
pub fn encode_float32(buf: &AudioBuffer) -> Vec<u8> {
    let channels = buf.channels.max(1);
    let data_len = (buf.samples.len() * 4) as u32;
    let byte_rate = buf.sample_rate * u32::from(channels) * 4;
    let mut out = Vec::with_capacity(58 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    // Non-PCM formats carry a cbSize field and a fact chunk (WAVEFORMATEX rules).
    out.extend_from_slice(&(50 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&18u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&buf.sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&(channels * 4).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    out.extend_from_slice(b"fact");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&(buf.frames() as u32).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in &buf.samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The sample encoding of a WAV file's `fmt ` chunk: (format tag, bits per sample).
/// `WAVE_FORMAT_EXTENSIBLE` reports the tag of its sub-format.
pub fn wav_encoding(bytes: &[u8]) -> Option<(u16, u16)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..at + 8 + size)?;
        if id == b"fmt " && body.len() >= 16 {
            let mut tag = u16::from_le_bytes([body[0], body[1]]);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16::from_le_bytes([body[24], body[25]]);
            }
            return Some((tag, bits));
        }
        at += 8 + size + (size & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_is_44_bytes() {
        assert_eq!(super::blank_wav().len(), 44);
    }

    #[test]
    fn float_wav_round_trips_through_the_decoder() {
        let buf = AudioBuffer { sample_rate: 48_000, channels: 2, samples: vec![0.25, -0.5, 0.75, 1.5] };
        let bytes = encode_float32(&buf);
        assert_eq!(wav_encoding(&bytes), Some((3, 32)));
        let back = hs_audio::decode_bytes(bytes, "wav").unwrap();
        assert_eq!((back.sample_rate, back.channels), (48_000, 2));
        assert_eq!(back.samples, buf.samples, "float keeps values above full scale");
        assert_eq!(wav_encoding(&encode_pcm16(&buf)), Some((1, 16)));
    }
}
