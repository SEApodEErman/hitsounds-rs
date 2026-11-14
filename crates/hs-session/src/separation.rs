//! Drum stem separation in the app (D-70 to D-73): the session side around
//! `hs-separate`. A [`Request`] is made from the session, runs on its own thread
//! ([`Separation`]) while editing and playback go on, and its [`SeparatedStem`] is
//! applied back as one undo step (`Session::apply_separation`).

use hs_audio::{AudioBuffer, decode_bytes, decode_file};
use hs_core::StemOrigin;
use hs_separate::{DrumsModel, Progress, SeparateError, load_runtime};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// The separated stem's folder and base name inside the project (D-73).
pub const STEM_DIR: &str = "stems";
const STEM_NAME: &str = "drums";
/// A stem louder than full scale is scaled down to this peak, a little under 1.0 so
/// Vorbis' small overshoots stay unclipped (D-73).
const PEAK_TARGET: f32 = 0.98;
/// Start comparing the waveform near the first sample that reaches this level.
const ONSET_LEVEL: f32 = 0.05;

/// Everything the separation thread needs, taken from the session up front.
#[derive(Debug, Clone)]
pub struct Request {
    /// The project folder.
    pub root: PathBuf,
    /// The song as the mapset names it (the reference difficulty's `AudioFilename`).
    pub song: PathBuf,
    pub model: PathBuf,
    /// `onnxruntime.dll`.
    pub runtime: PathBuf,
    /// Separated stems the project has had (current, saved and in its history), as
    /// project-relative paths, for reuse.
    pub known: Vec<(PathBuf, StemOrigin)>,
}

/// A finished separation, ready for `Session::apply_separation`.
#[derive(Debug, Clone, PartialEq)]
pub struct SeparatedStem {
    pub root: PathBuf,
    pub song: PathBuf,
    /// Relative to the project folder.
    pub path: PathBuf,
    pub origin: StemOrigin,
    /// An earlier stem for the same song and model was used instead of running again.
    pub reused: bool,
    /// How far the stem was turned down to stay under full scale, in dB.
    pub scaled_db: Option<f32>,
}

/// Where a running separation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Hashing the files, decoding the song and loading the model.
    Preparing,
    Separating(Progress),
    /// Encoding and checking the OGG.
    Writing,
}

impl Stage {
    /// Overall progress from 0 to 1.
    pub fn fraction(self) -> f32 {
        match self {
            Self::Preparing => 0.0,
            Self::Separating(p) if p.total > 0 => 0.02 + 0.93 * p.done as f32 / p.total as f32,
            Self::Separating(_) => 0.02,
            Self::Writing => 0.97,
        }
    }
}

/// Loads ONNX Runtime and `model` once to check it is a hitsounds-rs drums export
/// (D-71), so a wrong file is reported when it is chosen, not minutes into a run.
pub fn check_model(model: &Path, runtime: &Path) -> Result<(), String> {
    load_runtime(runtime).map_err(|e| e.to_string())?;
    DrumsModel::load(model).map(drop).map_err(|e| e.to_string())
}

/// The model's SHA-256, remembered per path, size and modified time for the life of the
/// process: the file is 185 MB and doesn't change between runs.
fn model_sha256(path: &Path) -> std::io::Result<String> {
    type Key = (PathBuf, u64, Option<std::time::SystemTime>);
    static CACHE: Mutex<Option<(Key, String)>> = Mutex::new(None);
    let meta = std::fs::metadata(path)?;
    let key = (path.to_path_buf(), meta.len(), meta.modified().ok());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached, hash)) = cache.as_ref()
        && *cached == key
    {
        return Ok(hash.clone());
    }
    let hash = sha256_file(path)?;
    *cache = Some((key, hash.clone()));
    Ok(hash)
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut block)?;
        if read == 0 {
            break;
        }
        hasher.update(&block[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

impl Request {
    /// Runs the separation. Blocks for about half the song's length on a 6-core CPU.
    pub fn run(
        self,
        cancel: &AtomicBool,
        stage: &mut dyn FnMut(Stage),
    ) -> Result<SeparatedStem, String> {
        stage(Stage::Preparing);
        let origin = StemOrigin::Separated {
            model_sha256: model_sha256(&self.model).map_err(io(&self.model))?,
            song_sha256: sha256_file(&self.song).map_err(io(&self.song))?,
        };
        let done = |path: PathBuf, reused, scaled_db| SeparatedStem {
            root: self.root.clone(),
            song: self.song.clone(),
            path,
            origin: origin.clone(),
            reused,
            scaled_db,
        };
        // D-73: the same song and model give the same stem, so reuse one that still exists.
        if let Some((path, _)) = self
            .known
            .iter()
            .find(|(p, o)| *o == origin && self.root.join(p).is_file())
        {
            return Ok(done(path.clone(), true, None));
        }
        // D-73: use the same decode as Song monitoring, including tagged MP3 trim.
        let song = decode_file(&self.song).map_err(|e| format!("{}: {e}", self.song.display()))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(SeparateError::Cancelled.to_string());
        }
        load_runtime(&self.runtime).map_err(|e| e.to_string())?;
        let mut model = DrumsModel::load(&self.model).map_err(|e| e.to_string())?;
        let mut drums = hs_separate::separate(&song, &mut model, cancel, &mut |p| {
            stage(Stage::Separating(p));
        })
        .map_err(|e| e.to_string())?;
        drop(model);
        stage(Stage::Writing);
        let scaled_db = limit_peak(&mut drums);
        let bytes = encode_checked(&drums)?;
        let path = write_new(&self.root, &bytes)?;
        Ok(done(path, false, scaled_db))
    }
}

/// D-73: scales the whole stem down when it peaks above full scale, instead of
/// letting the encoder clip it. Returns the change in dB.
fn limit_peak(buf: &mut AudioBuffer) -> Option<f32> {
    let peak = buf.peak();
    if peak <= 1.0 {
        return None;
    }
    let gain = PEAK_TARGET / peak;
    buf.samples.iter_mut().for_each(|s| *s *= gain);
    Some(20.0 * gain.log10())
}

/// Encodes with the D-69 Vorbis encoder, then checks the decoded format and
/// waveform alignment (D-73).
fn encode_checked(stem: &AudioBuffer) -> Result<Vec<u8>, String> {
    let bytes = hs_export::encode::encode_vorbis(stem)
        .map_err(|e| format!("Ogg Vorbis encoding failed: {e}"))?;
    let decoded = decode_bytes(bytes.clone(), "ogg")
        .map_err(|e| format!("The new stem won't decode: {e}"))?;
    if decoded.frames() != stem.frames()
        || decoded.sample_rate != stem.sample_rate
        || decoded.channels != stem.channels
    {
        return Err(format!(
            "The encoded stem has {} frames at {} Hz with {} channels, expected {} frames at {} Hz with {} channels.",
            decoded.frames(),
            decoded.sample_rate,
            decoded.channels,
            stem.frames(),
            stem.sample_rate,
            stem.channels,
        ));
    }
    if let Some(moved) = first_attack_shift_ms(stem, &decoded)
        && moved > 1.0
    {
        return Err(format!(
            "Encoding moved the stem's first hit by {moved:.1} ms."
        ));
    }
    Ok(bytes)
}

/// Cross-correlate a short window around the first strong attack. Vorbis can move
/// a threshold crossing by changing amplitude even when the waveform stays aligned.
fn first_attack_shift_ms(stem: &AudioBuffer, decoded: &AudioBuffer) -> Option<f64> {
    let onset = stem.first_frame_above(ONSET_LEVEL)?;
    let rate = stem.sample_rate as usize;
    let channels = usize::from(stem.channels);
    let radius = (rate * 20 / 1000).max(1);
    let before = radius.min(onset);
    let after = radius.min(stem.frames() - onset);
    let reference = &stem.samples[(onset - before) * channels..(onset + after) * channels];
    let reference_energy: f64 = reference.iter().map(|&s| f64::from(s).powi(2)).sum();
    let score = |start: usize| {
        let mut dot = 0.0;
        let mut energy = 0.0;
        let samples = &decoded.samples[start * channels..][..reference.len()];
        for (&a, &b) in reference.iter().zip(samples) {
            let b = f64::from(b);
            dot += f64::from(a) * b;
            energy += b * b;
        }
        dot / (reference_energy * energy).sqrt()
    };

    let aligned = onset - before;
    let aligned_score = score(aligned);
    let first = aligned.saturating_sub(radius);
    let last = (aligned + radius).min(decoded.frames() - before - after);
    let (mut best_start, mut best_score) = (aligned, aligned_score);
    for start in first..=last {
        let candidate = score(start);
        if candidate.is_finite() && (candidate > best_score || !best_score.is_finite()) {
            (best_start, best_score) = (start, candidate);
        }
    }
    if best_score < 0.9 || !best_score.is_finite() {
        return None;
    }
    // Similar scores at zero lag and at the best lag can occur with periodic audio.
    let moved = if best_score - aligned_score < 0.02 {
        0
    } else {
        best_start.abs_diff(aligned)
    };
    Some(moved as f64 * 1000.0 / f64::from(stem.sample_rate))
}

/// Writes `stems/drums.ogg`, or `drums (2).ogg` and so on when that name is taken.
/// An existing file is never replaced, so undo can always go back to it.
fn write_new(root: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = root.join(STEM_DIR);
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    let mut tmp = tempfile::NamedTempFile::new_in(&dir).map_err(io(&dir))?;
    std::io::Write::write_all(&mut tmp, bytes).map_err(io(&dir))?;
    tmp.as_file().sync_all().map_err(io(&dir))?;
    for number in 1.. {
        let name = if number == 1 {
            format!("{STEM_NAME}.ogg")
        } else {
            format!("{STEM_NAME} ({number}).ogg")
        };
        let relative = Path::new(STEM_DIR).join(name);
        match tmp.persist_noclobber(root.join(&relative)) {
            Ok(_) => return Ok(relative),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => tmp = e.file,
            Err(e) => return Err(io(&root.join(&relative))(e.error)),
        }
    }
    unreachable!()
}

enum Status {
    Running(Stage),
    Done(Result<SeparatedStem, String>),
    Taken,
}

/// A separation running on its own thread (D-72), never the session worker.
pub struct Separation {
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    thread: Option<JoinHandle<()>>,
}

impl Separation {
    /// Starts `request`. `notify` is called from the thread on every change of stage
    /// and when it ends, e.g. to repaint the UI.
    pub fn start(request: Request, notify: Arc<dyn Fn() + Send + Sync>) -> std::io::Result<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(Status::Running(Stage::Preparing)));
        let (flag, shared) = (Arc::clone(&cancel), Arc::clone(&status));
        let thread = std::thread::Builder::new()
            .name("hs-separate".into())
            .spawn(move || {
                let set = |s| *shared.lock().unwrap_or_else(|e| e.into_inner()) = s;
                let result = request.run(&flag, &mut |stage| {
                    set(Status::Running(stage));
                    notify();
                });
                set(Status::Done(result));
                notify();
            })?;
        Ok(Self {
            cancel,
            status,
            thread: Some(thread),
        })
    }

    /// Asks the thread to stop after the current segment.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The current stage, or `None` once it has finished.
    pub fn stage(&self) -> Option<Stage> {
        match &*self.status.lock().unwrap_or_else(|e| e.into_inner()) {
            Status::Running(stage) => Some(*stage),
            _ => None,
        }
    }

    /// The result, once, after the thread has finished.
    pub fn take_result(&mut self) -> Option<Result<SeparatedStem, String>> {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*status, Status::Done(_)) {
            return None;
        }
        let Status::Done(result) = std::mem::replace(&mut *status, Status::Taken) else {
            unreachable!()
        };
        drop(status);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        Some(result)
    }
}

impl Drop for Separation {
    fn drop(&mut self) {
        // Closing the app mid-run stops at the next segment instead of finishing.
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drums(rate: u32, frames: usize, peak: f32) -> AudioBuffer {
        let mut buf = AudioBuffer::silent(rate, 2, frames);
        for hit in (rate as usize / 10..frames).step_by(rate as usize / 2) {
            for i in 0..2_000.min(frames - hit) {
                let v = peak
                    * (-(i as f32) / 300.0).exp()
                    * (std::f32::consts::TAU * 90.0 * i as f32 / rate as f32).cos();
                buf.samples[2 * (hit + i)] = v;
                buf.samples[2 * (hit + i) + 1] = v;
            }
        }
        buf
    }

    #[test]
    fn a_loud_stem_is_scaled_down_as_a_whole() {
        let mut buf = drums(44_100, 44_100, 1.2);
        let before = buf.samples.clone();
        let db = limit_peak(&mut buf).unwrap();
        assert!((buf.peak() - PEAK_TARGET).abs() < 1e-6);
        assert!((db - 20.0 * (PEAK_TARGET / 1.2f32).log10()).abs() < 1e-4);
        let ratio = buf.samples[2 * 4_410 + 10] / before[2 * 4_410 + 10];
        assert!(
            (ratio - PEAK_TARGET / 1.2).abs() < 1e-6,
            "one gain for every sample"
        );

        let mut quiet = drums(44_100, 44_100, 0.7);
        assert_eq!(limit_peak(&mut quiet), None);
        assert!((quiet.peak() - 0.7).abs() < 1e-6);
    }

    #[test]
    fn the_ogg_keeps_length_and_onset() {
        for rate in [44_100, 48_000] {
            let stem = drums(rate, rate as usize * 2 + 123, 0.9);
            let bytes = encode_checked(&stem).unwrap();
            let back = decode_bytes(bytes, "ogg").unwrap();
            assert_eq!(back.frames(), stem.frames());
        }
    }

    #[test]
    fn attack_alignment_distinguishes_level_change_from_delay() {
        let mut stem = drums(44_100, 22_050, 0.9);
        let lead = 44_100 / 10;
        // A quiet precursor reaches the old onset threshold by a narrow margin.
        stem.samples[2 * (lead - 220)..2 * (lead - 219)].fill(0.051);
        let mut decoded = stem.clone();
        decoded.samples[2 * (lead - 220)..2 * (lead - 219)].fill(0.049);
        assert_eq!(decoded.first_frame_above(ONSET_LEVEL), Some(lead));
        assert_eq!(first_attack_shift_ms(&stem, &decoded), Some(0.0));

        decoded.samples.rotate_right(2 * 220);
        decoded.samples[..2 * 220].fill(0.0);
        let moved = first_attack_shift_ms(&stem, &decoded).unwrap();
        assert!((moved - 4.99).abs() < 0.03, "measured {moved} ms");
    }

    #[test]
    fn new_stems_never_replace_an_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let first = write_new(root.path(), b"one").unwrap();
        let second = write_new(root.path(), b"two").unwrap();
        assert_eq!(first, Path::new("stems").join("drums.ogg"));
        assert_eq!(second, Path::new("stems").join("drums (2).ogg"));
        assert_eq!(std::fs::read(root.path().join(&first)).unwrap(), b"one");
        assert_eq!(std::fs::read(root.path().join(&second)).unwrap(), b"two");
    }

    #[test]
    fn a_known_stem_for_the_same_song_and_model_is_reused_without_a_runtime() {
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("song.mp3");
        let model = root.path().join("drums.onnx");
        std::fs::write(&song, b"song bytes").unwrap();
        std::fs::write(&model, b"model bytes").unwrap();
        std::fs::create_dir(root.path().join("stems")).unwrap();
        std::fs::write(root.path().join("stems/drums.ogg"), b"ogg").unwrap();
        let origin = StemOrigin::Separated {
            model_sha256: sha256_file(&model).unwrap(),
            song_sha256: sha256_file(&song).unwrap(),
        };
        let other = StemOrigin::Separated {
            model_sha256: "0".repeat(64),
            song_sha256: sha256_file(&song).unwrap(),
        };
        let request = Request {
            root: root.path().to_path_buf(),
            song,
            model,
            runtime: root.path().join("missing.dll"),
            known: vec![
                ("stems/gone.ogg".into(), origin.clone()),
                ("stems/other.ogg".into(), other),
                ("stems/drums.ogg".into(), origin.clone()),
            ],
        };
        let stem = request.run(&AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(stem.reused);
        assert_eq!(stem.path, Path::new("stems/drums.ogg"));
        assert_eq!(stem.origin, origin);
    }

    #[test]
    fn sha256_matches_a_known_value() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("abc");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
