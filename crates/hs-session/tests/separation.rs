//! End-to-end drum separation through the session's thread (D-72, D-73), with the
//! real model. Needs `HS_ORT_DYLIB` (`onnxruntime.dll`) and `HS_DEMUCS_MODEL` (the
//! export from `tools/demucs-export`), so it is ignored by default:
//!
//! ```text
//! cargo test -p hs-session --release --test separation -- --ignored --nocapture
//! ```

use hs_audio::{AudioBuffer, decode_file};
use hs_session::separation::{Request, Separation, Stage};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn env(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name}"))
}

/// 12 s at 48 kHz (so resampling runs too): a low tone with kick-like hits every 0.5 s.
fn song() -> AudioBuffer {
    let rate = 48_000;
    let frames = rate * 12 + 77;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / rate as f32;
        let since = t % 0.5;
        let kick = 0.8 * (-since * 25.0).exp() * (std::f32::consts::TAU * 55.0 * since).sin();
        let pad = 0.2 * (std::f32::consts::TAU * 220.0 * t).sin();
        samples.extend([kick + pad, kick + pad]);
    }
    AudioBuffer {
        sample_rate: rate as u32,
        channels: 2,
        samples,
    }
}

#[test]
#[ignore = "needs ONNX Runtime and the exported model"]
fn separates_writes_and_reuses() {
    let dir = tempfile::tempdir().unwrap();
    let song_path = dir.path().join("audio.wav");
    let original = song();
    std::fs::write(&song_path, hs_export::wav::encode_pcm16(&original)).unwrap();
    let request = Request {
        root: dir.path().to_path_buf(),
        song: song_path,
        model: env("HS_DEMUCS_MODEL"),
        runtime: env("HS_ORT_DYLIB"),
        known: Vec::new(),
    };

    let started = Instant::now();
    let mut job = Separation::start(request.clone(), Arc::new(|| {})).unwrap();
    let mut stages = Vec::new();
    let result = loop {
        if let Some(stage) = job.stage()
            && stages.last() != Some(&stage)
        {
            stages.push(stage);
        }
        if let Some(result) = job.take_result() {
            break result;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stem = result.unwrap();
    println!(
        "separated in {:.1} s, stages {stages:?}",
        started.elapsed().as_secs_f32()
    );
    assert!(!stem.reused);
    assert_eq!(stem.path, PathBuf::from("stems").join("drums.ogg"));
    assert!(
        stages
            .iter()
            .any(|s| matches!(s, Stage::Separating(p) if p.total > 0))
    );

    let drums = decode_file(&dir.path().join(&stem.path)).unwrap();
    assert_eq!(
        (drums.sample_rate, drums.channels, drums.frames()),
        (48_000, 2, original.frames()),
        "the stem keeps the song's rate and exact length"
    );
    assert!(drums.peak() > 0.05 && drums.peak() <= 1.0);

    // Same song and model: reused without running the model.
    let mut again = request;
    again.known = vec![(stem.path.clone(), stem.origin.clone())];
    let started = Instant::now();
    let reused = again
        .run(&std::sync::atomic::AtomicBool::new(false), &mut |_| {})
        .unwrap();
    assert!(reused.reused && reused.path == stem.path);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
#[ignore = "needs ONNX Runtime and the exported model"]
fn cancelling_stops_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let song_path = dir.path().join("audio.wav");
    std::fs::write(&song_path, hs_export::wav::encode_pcm16(&song())).unwrap();
    let mut job = Separation::start(
        Request {
            root: dir.path().to_path_buf(),
            song: song_path,
            model: env("HS_DEMUCS_MODEL"),
            runtime: env("HS_ORT_DYLIB"),
            known: Vec::new(),
        },
        Arc::new(|| {}),
    )
    .unwrap();
    job.cancel();
    let result = loop {
        if let Some(result) = job.take_result() {
            break result;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(!dir.path().join("stems").exists());
}
