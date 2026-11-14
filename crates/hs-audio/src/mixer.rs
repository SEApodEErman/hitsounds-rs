//! The real-time mixer: song (original or stem) plus scheduled hitsound voices.
//!
//! It is a plain struct with no device attached, so tests can render it offline.
//! [`crate::engine::Engine`] drives it from the cpal callback. Every buffer handed to
//! the mixer must already be stereo at the mixer's sample rate.

use crate::stretch::Stretcher;
use crate::{AudioBuffer, MonitorSource};
use std::sync::Arc;

/// A hitsound voice that starts when the song reaches `time_ms`.
#[derive(Debug, Clone)]
pub struct Scheduled {
    pub time_ms: f64,
    /// Loop until this song time for a sustained sample.
    pub end_ms: Option<f64>,
    pub buffer: Arc<AudioBuffer>,
    /// Linear gain.
    pub gain: f32,
}

#[derive(Debug, Clone)]
pub enum Command {
    Play,
    Pause,
    Seek(f64),
    SetRate(f64),
    /// Keep the song's pitch when the rate is not 1 (D-90). Off = tape-style
    /// varispeed, lower and slower.
    SetKeepPitch(bool),
    SetMonitor(MonitorSource),
    SetMusic {
        original: Option<Arc<AudioBuffer>>,
        stem: Option<Arc<AudioBuffer>>,
    },
    /// Replaces all scheduled voices. Must be sorted by time.
    SetSchedule(Vec<Scheduled>),
    /// Monitoring levels (D-77), linear. `hitsounds` scales scheduled hits and
    /// `auditions` scales one-shots, so muting every channel still lets the sample
    /// browser audition. Applied to voices already sounding as well.
    SetVolumes {
        music: f32,
        hitsounds: f32,
        auditions: f32,
    },
    /// Shifts the song against the hitsound clock (positive = song later).
    SetMusicOffset(f64),
    /// Plays a buffer immediately, whether or not the transport runs (auditioning).
    OneShot {
        buffer: Arc<AudioBuffer>,
        gain: f32,
    },
}

struct Voice {
    buffer: Arc<AudioBuffer>,
    /// Next frame of `buffer` to play.
    pos: usize,
    gain: f32,
    /// A scheduled hit (scaled by the hitsound level), not an audition one-shot.
    hit: bool,
    /// Output frame within the current block at which the voice starts.
    start: usize,
    end_frame: Option<f64>,
}

/// How long playback continues past the last hitsound when no song is loaded.
const TAIL_MS: f64 = 2000.0;

pub struct Mixer {
    sample_rate: u32,
    original: Option<Arc<AudioBuffer>>,
    stem: Option<Arc<AudioBuffer>>,
    monitor: MonitorSource,
    playing: bool,
    rate: f64,
    keep_pitch: bool,
    stretcher: Stretcher,
    /// Song position in frames at the mixer rate.
    pos: f64,
    music_volume: f32,
    hitsound_volume: f32,
    audition_volume: f32,
    music_offset_frames: f64,
    schedule: Vec<Scheduled>,
    cursor: usize,
    voices: Vec<Voice>,
    scratch: Vec<f32>,
}

impl Mixer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            original: None,
            stem: None,
            monitor: MonitorSource::Original,
            playing: false,
            rate: 1.0,
            keep_pitch: true,
            stretcher: Stretcher::new(sample_rate),
            pos: 0.0,
            music_volume: 0.6,
            hitsound_volume: 1.0,
            audition_volume: 1.0,
            music_offset_frames: 0.0,
            schedule: Vec::new(),
            cursor: 0,
            voices: Vec::with_capacity(64),
            scratch: Vec::new(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    pub fn monitor(&self) -> MonitorSource {
        self.monitor
    }

    pub fn position_ms(&self) -> f64 {
        self.pos * 1000.0 / f64::from(self.sample_rate)
    }

    fn frame_of(&self, ms: f64) -> f64 {
        ms * f64::from(self.sample_rate) / 1000.0
    }

    /// Song length, or the end of the last hitsound's tail when no song is loaded.
    pub fn length_ms(&self) -> f64 {
        let music = [&self.original, &self.stem]
            .into_iter()
            .flatten()
            .map(|b| b.frames() as f64 * 1000.0 / f64::from(self.sample_rate))
            .fold(0.0, f64::max);
        if music > 0.0 {
            music
        } else {
            self.schedule.last().map_or(0.0, |s| s.time_ms + TAIL_MS)
        }
    }

    fn reset_cursor(&mut self) {
        let pos = self.pos;
        let rate = f64::from(self.sample_rate) / 1000.0;
        self.cursor = self.schedule.partition_point(|s| s.time_ms * rate < pos);
        self.voices.retain(|v| v.end_frame.is_none());
        for s in &self.schedule[..self.cursor] {
            if let Some(end) = s.end_ms.filter(|end| end * rate > pos) {
                let elapsed = ((pos - s.time_ms * rate) / self.rate).max(0.0) as usize;
                self.voices.push(Voice {
                    buffer: s.buffer.clone(),
                    pos: elapsed % s.buffer.frames().max(1),
                    gain: s.gain,
                    hit: true,
                    start: 0,
                    end_frame: Some(end * rate),
                });
            }
        }
    }

    pub fn handle(&mut self, command: Command) {
        match command {
            Command::Play => {
                if self.pos >= self.frame_of(self.length_ms()) {
                    self.pos = 0.0;
                    self.stretcher.reset();
                    self.reset_cursor();
                }
                self.playing = true;
            }
            Command::Pause => self.playing = false,
            Command::Seek(ms) => {
                self.pos = self.frame_of(ms.clamp(0.0, self.length_ms().max(0.0)));
                self.voices.clear();
                self.stretcher.reset();
                self.reset_cursor();
            }
            Command::SetRate(rate) => self.rate = rate.clamp(0.1, 2.0),
            Command::SetKeepPitch(on) => self.keep_pitch = on,
            Command::SetMonitor(monitor) => self.monitor = monitor,
            Command::SetMusic { original, stem } => {
                self.original = original;
                self.stem = stem;
                self.stretcher.reset();
            }
            Command::SetSchedule(schedule) => {
                self.schedule = schedule;
                self.reset_cursor();
            }
            Command::SetVolumes { music, hitsounds, auditions } => {
                self.music_volume = music.clamp(0.0, 2.0);
                self.hitsound_volume = hitsounds.clamp(0.0, 2.0);
                self.audition_volume = auditions.clamp(0.0, 2.0);
            }
            Command::SetMusicOffset(ms) => {
                self.music_offset_frames = self.frame_of(ms);
                self.stretcher.reset();
            }
            Command::OneShot { buffer, gain } => self.voices.push(Voice { buffer, pos: 0, gain, hit: false, start: 0, end_frame: None }),
        }
    }

    /// The song buffer for the current monitor source. With no stem loaded, the
    /// stem position falls back to the original so the toggle never goes silent.
    fn music(&self) -> Option<Arc<AudioBuffer>> {
        match self.monitor {
            MonitorSource::Original => self.original.clone(),
            MonitorSource::Stems => self.stem.clone().or_else(|| self.original.clone()),
        }
    }

    /// Renders the next block into `out` (interleaved, `channels` per frame).
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.scratch.clear();
        self.scratch.resize(frames * 2, 0.0);

        let block_start = self.pos;
        let was_playing = self.playing;
        if self.playing {
            let music = self.music();
            let per_ms = f64::from(self.sample_rate) / 1000.0;
            let end = self.frame_of(self.length_ms());
            for i in 0..frames {
                while self.cursor < self.schedule.len() && self.schedule[self.cursor].time_ms * per_ms <= self.pos {
                    let s = &self.schedule[self.cursor];
                    self.voices.push(Voice {
                        buffer: s.buffer.clone(), pos: 0, gain: s.gain, hit: true, start: i,
                        end_frame: s.end_ms.map(|t| t * per_ms),
                    });
                    self.cursor += 1;
                }
                if let Some(m) = &music {
                    let at = self.pos - self.music_offset_frames;
                    let lr = self.stretcher.frame(m, at, self.rate, self.keep_pitch);
                    for (c, v) in lr.into_iter().enumerate() {
                        self.scratch[i * 2 + c] += v * self.music_volume;
                    }
                }
                self.pos += self.rate;
                if self.pos >= end {
                    self.playing = false;
                    break;
                }
            }
        }

        for v in &mut self.voices {
            let total = v.buffer.frames();
            let gain = v.gain * if v.hit { self.hitsound_volume } else { self.audition_volume };
            if let Some(end) = v.end_frame {
                if was_playing && total != 0 {
                    let stop = (((end - block_start) / self.rate).ceil() as usize).min(frames);
                    for i in v.start..stop {
                        self.scratch[i * 2] += v.buffer.samples[v.pos * 2] * gain;
                        self.scratch[i * 2 + 1] += v.buffer.samples[v.pos * 2 + 1] * gain;
                        v.pos += 1;
                        if v.pos == total { v.pos = 0; }
                    }
                }
                v.start = 0;
                continue;
            }
            let n = (frames - v.start.min(frames)).min(total - v.pos.min(total));
            let src = &v.buffer.samples[v.pos * 2..(v.pos + n) * 2];
            let dst = &mut self.scratch[v.start * 2..(v.start + n) * 2];
            for (d, s) in dst.iter_mut().zip(src) {
                *d += s * gain;
            }
            v.pos += n;
            v.start = 0;
        }
        self.voices.retain(|v| v.end_frame.map_or(v.pos < v.buffer.frames(), |end| self.pos < end));

        for (frame, lr) in out.chunks_exact_mut(channels).zip(self.scratch.as_chunks::<2>().0) {
            let (l, r) = (lr[0].clamp(-1.0, 1.0), lr[1].clamp(-1.0, 1.0));
            match frame {
                [mono] => *mono = (l + r) * 0.5,
                [a, b, rest @ ..] => {
                    *a = l;
                    *b = r;
                    rest.fill(0.0);
                }
                [] => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1000; // 1 frame = 1 ms keeps the arithmetic readable

    fn click(frames: usize, value: f32) -> Arc<AudioBuffer> {
        Arc::new(AudioBuffer { sample_rate: RATE, channels: 2, samples: vec![value; frames * 2] })
    }

    fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        mixer.render(&mut out, 2);
        out.chunks(2).map(|f| f[0]).collect()
    }

    #[test]
    fn scheduled_voice_starts_on_its_frame_across_blocks() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::SetMusic { original: Some(click(1000, 0.0)), stem: None });
        m.handle(Command::SetSchedule(vec![Scheduled { time_ms: 150.0, end_ms: None, buffer: click(3, 0.5), gain: 1.0 }]));
        m.handle(Command::Play);
        let mut out = render(&mut m, 100);
        out.extend(render(&mut m, 100));
        let first = out.iter().position(|s| *s != 0.0);
        assert_eq!(first, Some(150));
        assert_eq!(&out[150..154], &[0.5, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn seek_skips_past_events_and_plays_the_one_under_the_playhead() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::SetMusic { original: Some(click(1000, 0.0)), stem: None });
        m.handle(Command::SetSchedule(vec![
            Scheduled { time_ms: 100.0, end_ms: None, buffer: click(1, 0.25), gain: 1.0 },
            Scheduled { time_ms: 200.0, end_ms: None, buffer: click(1, 0.5), gain: 1.0 },
        ]));
        m.handle(Command::Seek(200.0));
        m.handle(Command::Play);
        let out = render(&mut m, 10);
        assert_eq!(out[0], 0.5);
        assert!(out[1..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn monitor_toggle_keeps_position() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::SetMusic { original: Some(click(1000, 0.2)), stem: Some(click(1000, -0.2)) });
        m.handle(Command::SetVolumes { music: 1.0, hitsounds: 1.0, auditions: 1.0 });
        m.handle(Command::Play);
        assert_eq!(render(&mut m, 50)[10], 0.2);
        m.handle(Command::SetMonitor(MonitorSource::Stems));
        assert_eq!(render(&mut m, 50)[10], -0.2);
        assert_eq!(m.position_ms(), 100.0);
    }

    #[test]
    fn half_rate_plays_song_at_half_speed_but_hitsounds_at_full_speed() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::SetMusic { original: Some(click(1000, 0.0)), stem: None });
        m.handle(Command::SetSchedule(vec![Scheduled { time_ms: 50.0, end_ms: None, buffer: click(4, 0.5), gain: 1.0 }]));
        m.handle(Command::SetRate(0.5));
        m.handle(Command::Play);
        let out = render(&mut m, 200);
        // Song time 50 ms is reached after 100 output frames; the click still lasts 4 frames.
        assert_eq!(out.iter().position(|s| *s != 0.0), Some(100));
        assert_eq!(out.iter().filter(|s| **s != 0.0).count(), 4);
        assert_eq!(m.position_ms(), 100.0);
    }

    #[test]
    fn slowed_song_keeps_its_pitch_unless_turned_off() {
        // D-90. 441 Hz has 882 zero crossings a second; tape-style 0.5x halves that.
        const HZ: u32 = 48_000;
        let samples = (0..HZ as usize * 2)
            .flat_map(|i| {
                let s = (i as f32 * 441.0 * std::f32::consts::TAU / HZ as f32).sin() * 0.5;
                [s, s]
            })
            .collect();
        let song = Arc::new(AudioBuffer { sample_rate: HZ, channels: 2, samples });
        let crossings = |keep: bool| {
            let mut m = Mixer::new(HZ);
            m.handle(Command::SetMusic { original: Some(song.clone()), stem: None });
            m.handle(Command::SetVolumes { music: 1.0, hitsounds: 1.0, auditions: 1.0 });
            m.handle(Command::SetRate(0.5));
            m.handle(Command::SetKeepPitch(keep));
            m.handle(Command::Play);
            let out = render(&mut m, HZ as usize);
            assert_eq!(m.position_ms(), 500.0);
            out.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as i64
        };
        assert!((crossings(true) - 882).abs() <= 8);
        assert!((crossings(false) - 441).abs() <= 2);
    }

    #[test]
    fn stops_at_the_end_and_one_shots_play_while_paused() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::SetMusic { original: Some(click(20, 0.0)), stem: None });
        m.handle(Command::Play);
        render(&mut m, 50);
        assert!(!m.playing());
        m.handle(Command::OneShot { buffer: click(2, 0.3), gain: 1.0 });
        assert_eq!(&render(&mut m, 4), &[0.3, 0.3, 0.0, 0.0]);
    }

    #[test]
    fn mono_and_surround_outputs() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::OneShot { buffer: click(1, 0.4), gain: 1.0 });
        let mut mono = vec![0.0; 2];
        m.render(&mut mono, 1);
        assert_eq!(mono, vec![0.4, 0.0]);
        m.handle(Command::OneShot { buffer: click(1, 0.4), gain: 1.0 });
        let mut six = vec![9.0; 6];
        m.render(&mut six, 6);
        assert_eq!(six, vec![0.4, 0.4, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn levels_scale_sounding_voices_and_muting_hits_keeps_auditions() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::SetMusic { original: Some(click(1000, 0.0)), stem: None });
        m.handle(Command::SetSchedule(vec![Scheduled { time_ms: 0.0, end_ms: None, buffer: click(10, 0.5), gain: 1.0 }]));
        m.handle(Command::Play);
        assert_eq!(render(&mut m, 2), vec![0.5, 0.5]);
        // D-77: a level change reaches a hit that is already ringing.
        m.handle(Command::SetVolumes { music: 1.0, hitsounds: 0.5, auditions: 1.0 });
        assert_eq!(render(&mut m, 2), vec![0.25, 0.25]);
        m.handle(Command::SetVolumes { music: 1.0, hitsounds: 0.0, auditions: 0.5 });
        m.handle(Command::OneShot { buffer: click(1, 0.4), gain: 1.0 });
        assert_eq!(render(&mut m, 2), vec![0.2, 0.0]);
    }

    #[test]
    fn music_level_scales_the_song() {
        let mut m = Mixer::new(RATE);
        m.handle(Command::SetMusic { original: Some(click(1000, 0.5)), stem: None });
        m.handle(Command::SetVolumes { music: 0.5, hitsounds: 1.0, auditions: 1.0 });
        m.handle(Command::Play);
        assert_eq!(render(&mut m, 1), vec![0.25]);
        m.handle(Command::SetVolumes { music: 0.0, hitsounds: 1.0, auditions: 1.0 });
        assert_eq!(render(&mut m, 1), vec![0.0]);
    }

    #[test]
    fn looped_slider_sample_resumes_after_seek_and_stops_at_its_tail() {
        let mut mixer = Mixer::new(RATE);
        mixer.handle(Command::SetSchedule(vec![Scheduled {
            time_ms: 10.0, end_ms: Some(20.0), buffer: click(2, 0.5), gain: 1.0,
        }]));
        mixer.handle(Command::Seek(17.0));
        mixer.handle(Command::Play);
        assert_eq!(render(&mut mixer, 6), vec![0.5, 0.5, 0.5, 0.0, 0.0, 0.0]);
    }
}
