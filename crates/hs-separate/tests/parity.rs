//! The real model against PyTorch Demucs. Needs the user's model file (D-71) and ONNX
//! Runtime, so it is ignored by default:
//!
//! ```text
//! python tools/demucs-export/reference.py fixture --out sandbox/separation/fixture
//! set HS_ORT_DYLIB=...\onnxruntime.dll
//! set HS_DEMUCS_MODEL=...\htdemucs_ft_drums.onnx
//! set HS_DEMUCS_FIXTURE=sandbox\separation\fixture
//! cargo test --release -p hs-separate --test parity -- --ignored
//! ```

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use hs_audio::AudioBuffer;
use hs_separate::{DrumsModel, load_runtime, separate};

fn env(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var(name)
            .unwrap_or_else(|_| panic!("set {name} (see the top of tests/parity.rs)")),
    )
}

fn read_f32(path: PathBuf) -> Vec<f32> {
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

#[test]
#[ignore = "needs ONNX Runtime, the user's model file and a fixture from reference.py"]
fn matches_pytorch_demucs() {
    load_runtime(&env("HS_ORT_DYLIB")).unwrap();
    let mut model = DrumsModel::load(&env("HS_DEMUCS_MODEL")).unwrap();
    let fixture = env("HS_DEMUCS_FIXTURE");
    let song = AudioBuffer {
        sample_rate: 44_100,
        channels: 2,
        samples: read_f32(fixture.join("input.f32")),
    };
    let want = read_f32(fixture.join("expected.f32"));

    let started = std::time::Instant::now();
    let got = separate(&song, &mut model, &AtomicBool::new(false), &mut |_| {}).unwrap();
    let seconds = started.elapsed().as_secs_f64();

    assert_eq!(got.samples.len(), want.len());
    let signal: f64 = want.iter().map(|&s| f64::from(s).powi(2)).sum();
    let noise: f64 = got
        .samples
        .iter()
        .zip(&want)
        .map(|(&a, &b)| f64::from(a - b).powi(2))
        .sum();
    let snr = 10.0 * (signal / noise.max(1e-30)).log10();
    let max = got
        .samples
        .iter()
        .zip(&want)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    println!(
        "SNR against PyTorch {snr:.1} dB, max error {max:.2e}, {seconds:.1} s for {:.1} s",
        song.duration_ms() / 1000.0
    );
    assert!(snr > 60.0, "SNR {snr:.1} dB");
}

#[test]
#[ignore = "needs ONNX Runtime"]
fn rejects_a_file_that_is_not_the_drums_model() {
    load_runtime(&env("HS_ORT_DYLIB")).unwrap();
    let dir = std::env::temp_dir().join("hs-separate-not-a-model.onnx");
    std::fs::write(&dir, b"not an onnx model").unwrap();
    let err = DrumsModel::load(&dir).err().expect("garbage must not load");
    assert!(
        matches!(err, hs_separate::SeparateError::Model { .. }),
        "{err}"
    );
    let _ = std::fs::remove_file(dir);
}
