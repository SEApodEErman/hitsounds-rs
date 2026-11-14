# Demucs drums export (D-70 to D-73)

hitsounds-rs separates the drum stem with the drums model of HT-Demucs FT. It never
ships or downloads the weights, because their licence is unclear (D-71). You make
the model file once with these scripts, then choose it in hitsounds-rs under
**App settings → Drum separation model**. hitsounds-rs loads it once to check it.

## Make the model

Tested with Python 3.12, PyTorch 2.15 (CPU) and ONNX Runtime 1.24:

```sh
uv venv --python 3.12 .venv
uv pip install --python .venv/Scripts/python.exe torch torchaudio --index-url https://download.pytorch.org/whl/cpu
uv pip install --python .venv/Scripts/python.exe demucs==4.1.0 onnx onnxruntime soundfile numpy
.venv/Scripts/python.exe export_drums_onnx.py --out htdemucs_ft_drums.onnx
```

This downloads `f7e0c4bc.safetensors` (80 MB) from the Hugging Face repo
`adefossez/HTDemucs-ft` and writes a 185 MB ONNX file. The file's metadata records
the source and the weights' SHA-256.

## Check it

- `reference.py verify --model htdemucs_ft_drums.onnx` compares the full pipeline
  (NumPy pre/post-processing plus the ONNX core) with PyTorch Demucs on a synthetic
  signal. It must pass (SNR above 60 dB).
- `reference.py bench --model …` times one 7.8 s segment and extrapolates to a song.
- `reference.py separate --model … song.mp3 drums.wav` separates a real song for
  listening. Keep songs and stems in `sandbox/`, never in the repository.

## Phase 0 results (2026-11-13)

| Check | Result |
|---|---|
| Export | Opset 17 with the legacy exporter. The fused attention kernel has no ONNX mapping, so the script turns PyTorch's attention fast path off. |
| Parity with PyTorch Demucs (20 s synthetic, shifts 0) | 92.5 dB SNR, max error 3.3e-5, output length exact |
| Plain STFT round trip | 5.6e-16 |
| CPU, 6-core desktop | 2.7 s per segment, about 113 s for 4 minutes (2.1× real time) |
| DirectML, entry-level 2 GB GPU | 4.8 s per segment, and the output is wrong (−90 dB against the CPU, with graph optimisation on or off). v1 is CPU only (D-72). |

| Real song (246 s MP3, 44.1 kHz), CPU | 120 s. Drums peak at 1.20, over full scale on 70 samples. |
| MP3 decoders | libsndfile trimmed 1105 leading frames from the test song, about 25 ms. hitsounds-rs now trims the same tagged delay. BASS with osu!stable's `MP3_OLDGAPS` setting trimmed it too (findings §11). Use a WAV of hitsounds-rs's own decode when comparing a file whose decoder handling differs. |

Notes for the Rust port (`hs-separate`):

- Separate the buffer `hs_audio::decode_file` returns, never another decoder's (D-73).
- Scale a stem that peaks over full scale down as a whole before encoding; the Vorbis
  encoder clamps (D-73).

- Demucs' `_spec`/`_ispec` pair is lossy on purpose. It drops the Nyquist bin and
  the two outer frames at each end, so test the plain STFT pair for round trip, and
  test the Demucs pair only through parity.
- The last chunk is padded with real audio before it (centred, `TensorChunk.padded`),
  not with zeros. Only the track's ends are zero-padded.
- hitsounds-rs uses `shifts = 0` so the result is deterministic and can be reused (D-73).
  The Demucs CLI's default of one random shift is worth up to about 0.2 dB SDR.
