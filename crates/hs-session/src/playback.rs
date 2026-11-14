//! Binds project audio and the export simulator to the real-time mixer.

use hs_audio::{
    AudioBuffer, Command, Engine, MonitorSource, Mp3Trim, Scheduled, conform,
    decode_file_with_mp3_trim, peaks::peaks,
};
use hs_core::packing::SourceMix;
use hs_core::project::Monitor;
use hs_export::{SampleLibrary, auto_fix};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

const BUCKET_MS: f64 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq)]
struct SongTrim {
    trim: Mp3Trim,
    sample_rate: u32,
    frames: usize,
}

impl SongTrim {
    /// D-73: Older generated stems include the MP3 delay and padding. The caller
    /// verifies the source hash. Only a stem with the exact old layout is trimmed.
    fn trim_legacy_stem(self, stem: &mut AudioBuffer) {
        if self.frames > 0
            && stem.sample_rate == self.sample_rate
            && stem.frames() == self.frames + self.trim.start + self.trim.end
        {
            stem.samples
                .truncate((self.trim.start + self.frames) * usize::from(stem.channels));
            stem.trim_start(self.trim.start);
        }
    }
}

#[derive(Default)]
struct Track {
    path: Option<PathBuf>,
    /// Size and modified time, so a file replaced under the same name reloads (D-73).
    stamp: Option<(u64, Option<SystemTime>)>,
    mp3_trim: Option<SongTrim>,
    song_trim: Option<SongTrim>,
    audio: Option<Arc<AudioBuffer>>,
    peaks: Vec<i8>,
    warning: Option<String>,
}

impl Track {
    fn load(
        &mut self,
        path: Option<&Path>,
        rate: Option<u32>,
        label: &str,
        song_trim: Option<SongTrim>,
    ) -> bool {
        let stamp = path
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| (m.len(), m.modified().ok()));
        if self.path.as_deref() == path && self.stamp == stamp && self.song_trim == song_trim {
            return false;
        }
        *self = Self {
            path: path.map(Path::to_path_buf),
            stamp,
            song_trim,
            ..Self::default()
        };
        if let Some(path) = path {
            match decode_file_with_mp3_trim(path)
                .map_err(|e| e.to_string())
                .and_then(|(mut audio, mp3_trim)| {
                    if audio.frames() == 0 {
                        return Err("the audio file is empty".into());
                    }
                    self.mp3_trim = mp3_trim.map(|trim| SongTrim {
                        trim,
                        sample_rate: audio.sample_rate,
                        frames: audio.frames(),
                    });
                    if let Some(song) = song_trim {
                        song.trim_legacy_stem(&mut audio);
                    }
                    match rate {
                        Some(rate) => conform(&audio, rate, 2).map_err(|e| e.to_string()),
                        None => Ok(audio),
                    }
                }) {
                Ok(audio) => {
                    self.peaks = peaks(&audio, BUCKET_MS);
                    self.audio = Some(Arc::new(audio));
                }
                Err(e) => self.warning = Some(format!("{label}: {e}")),
            }
        }
        true
    }

    fn length_ms(&self) -> f64 {
        self.audio.as_ref().map_or(0.0, |a| {
            a.frames() as f64 * 1000.0 / f64::from(a.sample_rate)
        })
    }
}

#[derive(Serialize)]
pub struct Waveform<'a> {
    pub bucket_ms: f64,
    /// Interleaved minimum and maximum for each bucket.
    pub peaks: &'a [i8],
}

#[derive(Debug, Default, Serialize)]
pub struct Clock {
    pub playing: bool,
    pub position_ms: f64,
    pub rate: f64,
    pub length_ms: f64,
}

type Renders = HashMap<(BTreeSet<SourceMix>, bool), Result<Arc<AudioBuffer>, String>>;

pub struct Playback {
    engine: Result<Engine, String>,
    original: Track,
    stem: Track,
    renders: Renders,
    preview_warnings: BTreeSet<String>,
    pub waveform_version: u32,
}

impl Playback {
    #[expect(
        clippy::new_without_default,
        reason = "new() opens the audio device; Default would hide that"
    )]
    pub fn new() -> Self {
        Self {
            engine: Engine::start().map_err(|e| e.to_string()),
            original: Track::default(),
            stem: Track::default(),
            renders: HashMap::new(),
            preview_warnings: BTreeSet::new(),
            waveform_version: 0,
        }
    }

    pub fn sync_tracks(&mut self, original: Option<&Path>, stem: Option<&Path>, monitor: Monitor) {
        self.sync_project_tracks(original, stem, monitor, false);
    }

    pub(crate) fn sync_project_tracks(
        &mut self,
        original: Option<&Path>,
        stem: Option<&Path>,
        monitor: Monitor,
        stem_matches_song: bool,
    ) {
        let rate = self.engine.as_ref().ok().map(Engine::sample_rate);
        let song_changed = self.original.load(original, rate, "Song", None);
        let song_trim = self.original.mp3_trim.filter(|_| stem_matches_song);
        let stem_changed = self.stem.load(stem, rate, "Stem", song_trim);
        if song_changed || stem_changed {
            self.waveform_version += 1;
            if let Ok(engine) = &self.engine {
                engine.send(Command::SetMusic {
                    original: self.original.audio.clone(),
                    stem: self.stem.audio.clone(),
                });
            }
        }
        self.monitor(monitor);
    }

    pub fn monitor(&self, monitor: Monitor) {
        if let Ok(engine) = &self.engine {
            let source = match monitor {
                Monitor::Stems if self.stem.audio.is_some() => MonitorSource::Stems,
                _ => MonitorSource::Original,
            };
            engine.send(Command::SetMonitor(source));
        }
    }

    pub fn stem_ready(&self) -> bool {
        self.stem.audio.is_some()
    }

    pub fn length_ms(&self) -> f64 {
        self.original.length_ms().max(self.stem.length_ms())
    }

    pub fn waveform(&self, monitor: Monitor) -> Waveform<'_> {
        let track = if monitor == Monitor::Stems && self.stem_ready() {
            &self.stem
        } else {
            &self.original
        };
        Waveform {
            bucket_ms: BUCKET_MS,
            peaks: &track.peaks,
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if let Err(e) = &self.engine {
            warnings.push(format!("Audio output: {e}"));
        }
        if self.original.path.is_none() {
            warnings.push("The reference difficulty has no readable AudioFilename. Hitsound preview can play on its own.".into());
        }
        warnings.extend(
            self.original
                .warning
                .iter()
                .chain(self.stem.warning.iter())
                .cloned(),
        );
        let original = self.original.length_ms();
        let stem = self.stem.length_ms();
        if original > 0.0 && stem > 0.0 && (original - stem).abs() > 100.0 {
            warnings.push(format!(
                "The song and stem differ in length by {:.0} ms. Check that the stem is aligned.",
                (original - stem).abs()
            ));
        }
        warnings.extend(self.preview_warnings.iter().cloned());
        warnings
    }

    pub fn clear_samples(&mut self) {
        self.renders.clear();
    }

    pub fn set_schedule(
        &mut self,
        preview: Result<Vec<hs_export::PreviewHit>, hs_core::packing::PackingError>,
        library: &mut SampleLibrary,
        fix: bool,
    ) {
        self.preview_warnings.clear();
        let Ok(engine) = &self.engine else { return };
        let mut schedule = Vec::new();
        match preview {
            Ok(hits) => {
                for hit in hits {
                    for sound in hit.sounds {
                        let rate = engine.sample_rate();
                        let key = (sound.content, fix && sound.generated);
                        let buffer = self.renders.entry(key.clone()).or_insert_with(|| {
                            let mut audio = library.render(&key.0).map_err(|e| e.to_string())?;
                            if key.1 && audio.frames() > 0 {
                                auto_fix(&mut audio);
                            }
                            conform(&audio, rate, 2)
                                .map(Arc::new)
                                .map_err(|e| e.to_string())
                        });
                        match buffer {
                            Ok(buf) if buf.frames() > 0 => schedule.push(Scheduled {
                                time_ms: f64::from(hit.time),
                                end_ms: sound.end_ms.map(f64::from),
                                buffer: buf.clone(),
                                gain: sound.gain,
                            }),
                            Ok(_) => {}
                            Err(e) => {
                                self.preview_warnings.insert(e.clone());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                self.preview_warnings.insert(e.to_string());
            }
        }
        schedule.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        engine.send(Command::SetSchedule(schedule));
    }

    pub fn audition(&self, audio: &AudioBuffer, gain: f32) -> Result<(), String> {
        let engine = self.engine.as_ref().map_err(Clone::clone)?;
        let buffer = conform(audio, engine.sample_rate(), 2).map_err(|e| e.to_string())?;
        engine.send(Command::OneShot {
            buffer: Arc::new(buffer),
            gain,
        });
        Ok(())
    }

    pub fn command(&self, command: Command) -> Result<(), String> {
        self.engine.as_ref().map_err(Clone::clone)?.send(command);
        Ok(())
    }

    pub fn clock(&self) -> Clock {
        self.engine.as_ref().map_or_else(
            |_| Clock {
                rate: 1.0,
                length_ms: self.length_ms(),
                ..Clock::default()
            },
            |engine| {
                let state = engine.state();
                Clock {
                    playing: state.playing,
                    position_ms: state.position_ms,
                    rate: state.rate,
                    length_ms: state.length_ms,
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_mp3_stem_uses_the_same_trim_for_audio_and_peaks() {
        let dir = tempfile::tempdir().unwrap();
        let song_path = dir.path().join("song.mp3");
        std::fs::write(
            &song_path,
            include_bytes!("../../hs-audio/tests/fixtures/gapless-sine.mp3"),
        )
        .unwrap();
        let stem_path = dir.path().join("stem.ogg");
        let mut padded = AudioBuffer::silent(44_100, 2, 3456);
        let marker = 1105 + 220;
        padded.samples[2 * marker..2 * marker + 2].fill(0.8);
        std::fs::write(
            &stem_path,
            hs_export::encode::encode_vorbis(&padded).unwrap(),
        )
        .unwrap();

        let mut playback = Playback {
            engine: Err("no output device for this test".into()),
            original: Track::default(),
            stem: Track::default(),
            renders: HashMap::new(),
            preview_warnings: BTreeSet::new(),
            waveform_version: 0,
        };
        let original = Some(song_path.as_path());
        let stem = Some(stem_path.as_path());

        playback.sync_project_tracks(original, stem, Monitor::Stems, true);
        let audio = playback.stem.audio.as_deref().unwrap();
        assert_eq!(audio.frames(), 1764);
        assert_eq!(audio.peak_frame(), 220);
        assert_eq!(
            playback.waveform(Monitor::Stems).peaks,
            peaks(audio, BUCKET_MS)
        );
        let version = playback.waveform_version;

        // The same file imported manually or belonging to a different song hash.
        playback.sync_project_tracks(original, stem, Monitor::Stems, false);
        let audio = playback.stem.audio.as_deref().unwrap();
        assert_eq!(audio.frames(), 3456);
        assert_eq!(audio.peak_frame(), marker);
        assert!(playback.waveform_version > version);

        // A new separation already has the trimmed song's frame count.
        let trimmed = hs_audio::decode_file(&song_path).unwrap();
        let mut current = trimmed.with_channels(2);
        current.samples.fill(0.0);
        current.samples[440..442].fill(0.8);
        std::fs::write(
            &stem_path,
            hs_export::encode::encode_vorbis(&current).unwrap(),
        )
        .unwrap();
        playback.sync_project_tracks(original, stem, Monitor::Stems, true);
        let audio = playback.stem.audio.as_deref().unwrap();
        assert_eq!(audio.frames(), 1764);
        assert_eq!(audio.peak_frame(), 220);
    }
}
