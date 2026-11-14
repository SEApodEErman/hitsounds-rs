"""NumPy reference for hitsounds-rs's drum separation around the exported ONNX core (D-72).

Everything outside the ONNX graph lives here: track normalisation, Demucs' padded
STFT, complex-as-channels packing, the iSTFT, and the chunked overlap-add of
`demucs/apply.py` (shifts = 0). `hs-separate` mirrors these functions in Rust; when
they disagree, this file plus PyTorch Demucs is the arbiter.

    python reference.py verify --model htdemucs_ft_drums.onnx
        Synthetic signal: compare against PyTorch Demucs (`apply_model`, shifts=0).
    python reference.py bench --model htdemucs_ft_drums.onnx [--seconds 240]
        Time the ONNX core on DirectML and CPU, per 7.8 s segment and per song.
    python reference.py separate --model htdemucs_ft_drums.onnx song.mp3 drums.wav
        Separate a real song for listening tests. Keep songs and stems in sandbox/,
        never in the repository.

Pre- and post-processing follow Demucs (MIT, github.com/adefossez/demucs):
`api.Separator.separate_tensor`, `apply.apply_model`, `htdemucs.HTDemucs._spec`,
`_ispec`, `_magnitude`, `_mask`, and `spec.spectro`/`ispectro`.
"""

import argparse
import math
import time

import numpy as np

SR = 44100
NFFT = 4096
HOP = NFFT // 4
SEGMENT = 343980  # int(39/5 * 44100), the model's training length
OVERLAP = 0.25


# ---- STFT, matching torch.stft/istft(normalized=True, center=True, hann) ----------

def hann(n):
    # torch.hann_window is periodic by default.
    return (0.5 - 0.5 * np.cos(2 * np.pi * np.arange(n) / n)).astype(np.float64)


def stft(x):
    """[C, N] -> complex [C, NFFT/2+1, frames]; center=True with reflect padding."""
    x = np.pad(x, ((0, 0), (NFFT // 2, NFFT // 2)), mode="reflect")
    frames = 1 + (x.shape[-1] - NFFT) // HOP
    idx = np.arange(NFFT)[None, :] + HOP * np.arange(frames)[:, None]
    seg = x[:, idx] * hann(NFFT)  # [C, frames, NFFT]
    z = np.fft.rfft(seg, axis=-1) / math.sqrt(NFFT)
    return np.swapaxes(z, -1, -2)


def istft(z, length):
    """complex [C, NFFT/2+1, frames] -> [C, length]; inverse of `stft`."""
    frames = z.shape[-1]
    seg = np.fft.irfft(np.swapaxes(z, -1, -2), n=NFFT, axis=-1) * math.sqrt(NFFT)
    w = hann(NFFT)
    total = NFFT + HOP * (frames - 1)
    out = np.zeros((z.shape[0], total))
    env = np.zeros(total)
    for t in range(frames):
        out[:, t * HOP: t * HOP + NFFT] += seg[:, t] * w
        env[t * HOP: t * HOP + NFFT] += w * w
    start = NFFT // 2
    out = out[:, start: start + length]
    env = env[start: start + length]
    return out / np.where(env > 1e-11, env, 1.0)


def demucs_spec(x):
    """HTDemucs._spec: [C, L] -> complex [C, 2048, ceil(L/HOP)]."""
    length = x.shape[-1]
    le = -(-length // HOP)
    pad = HOP // 2 * 3
    x = np.pad(x, ((0, 0), (pad, pad + le * HOP - length)), mode="reflect")
    z = stft(x)[:, :-1, :]
    assert z.shape[-1] == le + 4
    return z[..., 2: 2 + le]


def demucs_ispec(z, length):
    """HTDemucs._ispec: complex [C, 2048, T] -> [C, length]."""
    z = np.pad(z, ((0, 0), (0, 1), (2, 2)))
    pad = HOP // 2 * 3
    le = HOP * -(-length // HOP) + 2 * pad
    return istft(z, le)[:, pad: pad + length]


def to_cac(z):
    """complex [C, F, T] -> [C*2, F, T] as re0, im0, re1, im1 (HTDemucs._magnitude)."""
    return np.stack([z.real, z.imag], axis=1).reshape(-1, *z.shape[1:])


def from_cac(m):
    """[C*2, F, T] -> complex [C, F, T] (HTDemucs._mask with cac)."""
    m = m.reshape(-1, 2, *m.shape[1:])
    return m[:, 0] + 1j * m[:, 1]


# ---- One segment through the core, then the whole track --------------------------

class Core:
    def __init__(self, path, provider="CPUExecutionProvider"):
        import onnxruntime as ort

        opts = ort.SessionOptions()
        opts.log_severity_level = 3
        self.session = ort.InferenceSession(path, opts, providers=[provider])
        self.provider = self.session.get_providers()[0]

    def segment(self, mix):
        """[2, SEGMENT] (already track-normalised) -> drums [2, SEGMENT]."""
        assert mix.shape == (2, SEGMENT)
        spec = to_cac(demucs_spec(mix.astype(np.float64)))[None].astype(np.float32)
        spec_out, time_out = self.session.run(
            None, {"mix": mix[None].astype(np.float32), "spec": spec})
        freq = demucs_ispec(from_cac(spec_out[0].astype(np.float64)), SEGMENT)
        return time_out[0].astype(np.float64) + freq


def padded(mix, offset, length, target):
    """apply.TensorChunk.padded: centre `target` samples on the chunk, borrowing real
    neighbouring audio where the track has it and zeros past either end."""
    delta = target - length
    start = offset - delta // 2
    end = start + target
    lo, hi = max(0, start), min(mix.shape[-1], end)
    return np.pad(mix[:, lo:hi], ((0, 0), (lo - start, end - hi)))


def separate(mix, core, progress=None):
    """apply_model(split=True, shifts=0, overlap=0.25) wrapped in the track
    normalisation of api.Separator. mix: [2, N] at 44.1 kHz -> drums [2, N]."""
    ref = mix.mean(0)
    mean, std = ref.mean(), ref.std(ddof=1) + 1e-8
    x = (mix - mean) / std

    n = x.shape[-1]
    stride = int((1 - OVERLAP) * SEGMENT)
    half = SEGMENT // 2
    weight = np.concatenate([np.arange(1, half + 1), np.arange(SEGMENT - half, 0, -1)])
    weight = weight / weight.max()
    out = np.zeros((2, n))
    total = np.zeros(n)
    offsets = list(range(0, n, stride))
    for i, offset in enumerate(offsets):
        length = min(SEGMENT, n - offset)
        chunk = core.segment(padded(x, offset, length, SEGMENT))
        trim = (SEGMENT - length) // 2  # utils.center_trim
        chunk = chunk[:, trim: trim + length]
        out[:, offset: offset + length] += weight[:length] * chunk
        total[offset: offset + length] += weight[:length]
        if progress:
            progress(i + 1, len(offsets))
    return out / total * std + mean


# ---- Synthetic test material (no copyrighted audio, CLAUDE.md) ---------------------

def synthetic(seconds, seed=0):
    rng = np.random.default_rng(seed)
    n = int(seconds * SR)
    t = np.arange(n) / SR
    beat = 0.5  # 120 BPM
    out = np.zeros((2, n))
    for b in np.arange(0, seconds, beat):
        i = int(b * SR)
        k = np.arange(min(int(0.25 * SR), n - i)) / SR
        kick = np.sin(2 * np.pi * (50 + 120 * np.exp(-k * 30)) * k) * np.exp(-k * 12)
        out[:, i: i + len(k)] += 0.8 * kick
        j = int((b + beat / 2) * SR)
        if j < n:
            h = min(int(0.05 * SR), n - j)
            hat = np.diff(rng.standard_normal(h + 1)) * np.exp(-np.arange(h) / SR * 80)
            out[0, j: j + h] += 0.3 * hat
            out[1, j: j + h] += 0.25 * hat
    chord = sum(np.sin(2 * np.pi * f * t) for f in (220.0, 277.18, 329.63)) * 0.08
    bass = np.sin(2 * np.pi * 55.0 * t) * 0.2
    out += chord + bass
    return out.astype(np.float32)


# ---- Commands ---------------------------------------------------------------------

def cmd_verify(args):
    import torch
    from export_drums_onnx import load_drums_model
    from demucs.apply import apply_model

    mix = synthetic(args.seconds)
    model, _ = load_drums_model()
    torch.backends.mha.set_fastpath_enabled(False)
    ref = mix.mean(0)
    mean, std = ref.mean(), ref.std(ddof=1) + 1e-8
    with torch.no_grad():
        want = apply_model(model, torch.from_numpy((mix - mean) / std)[None],
                           shifts=0, split=True, overlap=OVERLAP)[0, 0].numpy()
    want = want * std + mean

    got = separate(mix, Core(args.model))
    assert got.shape == want.shape == mix.shape
    err = got - want
    snr = 10 * np.log10(np.sum(want ** 2) / max(np.sum(err ** 2), 1e-30))
    print(f"length       {got.shape[-1]} samples (input {mix.shape[-1]})")
    print(f"max |error|  {np.abs(err).max():.2e}  (peak of reference {np.abs(want).max():.3f})")
    print(f"SNR vs torch {snr:.1f} dB")
    # The plain STFT pair must be exact. (Demucs' _spec/_ispec pair is lossy on
    # purpose: it drops the Nyquist bin and the two outer frames at each end, where
    # the overlap-add weight is near zero, so it is not a round-trip test.)
    seg = mix[:, :SEGMENT].astype(np.float64)
    rt = istft(stft(seg), SEGMENT)
    print(f"STFT round trip max |error| {np.abs(rt - seg).max():.2e}")
    ok = snr > 60
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


def torch_drums(mix):
    """PyTorch Demucs' drums for `mix` [2, N], the way hitsounds-rs separates (shifts=0)."""
    import torch
    from export_drums_onnx import load_drums_model
    from demucs.apply import apply_model

    model, _ = load_drums_model()
    torch.backends.mha.set_fastpath_enabled(False)
    ref = mix.mean(0)
    mean, std = ref.mean(), ref.std(ddof=1) + 1e-8
    with torch.no_grad():
        out = apply_model(model, torch.from_numpy((mix - mean) / std)[None],
                          shifts=0, split=True, overlap=OVERLAP)[0, 0].numpy()
    return out * std + mean


def cmd_fixture(args):
    """Synthetic input and PyTorch's drums as interleaved little-endian f32, for
    hs-separate's ignored parity test (`HS_DEMUCS_FIXTURE`)."""
    from pathlib import Path

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    mix = synthetic(args.seconds)
    mix.T.astype("<f4").tofile(out / "input.f32")
    torch_drums(mix).T.astype("<f4").tofile(out / "expected.f32")
    print(f"wrote {out / 'input.f32'} and {out / 'expected.f32'} ({mix.shape[-1]} frames, 44.1 kHz stereo)")


def cmd_bench(args):
    import onnxruntime as ort

    mix = synthetic(SEGMENT / SR)
    segments = len(range(0, int(args.seconds * SR), int((1 - OVERLAP) * SEGMENT)))
    for provider in ("DmlExecutionProvider", "CPUExecutionProvider"):
        if provider not in ort.get_available_providers():
            continue
        try:
            t0 = time.perf_counter()
            core = Core(args.model, provider)
            load = time.perf_counter() - t0
            core.segment(mix)  # warm-up
            runs = []
            for _ in range(args.runs):
                t0 = time.perf_counter()
                core.segment(mix)
                runs.append(time.perf_counter() - t0)
        except Exception as exc:  # noqa: BLE001 - report and continue with CPU
            print(f"{provider}: failed: {exc}")
            continue
        per = min(runs)
        print(f"{core.provider}: load {load:.1f} s, {per:.2f} s per segment, "
              f"~{per * segments:.0f} s for {args.seconds:.0f} s of audio "
              f"({segments} segments, {args.seconds / (per * segments):.1f}x real time)")


def cmd_separate(args):
    import soundfile as sf

    data, sr = sf.read(args.input, dtype="float32", always_2d=True)
    mix = data.T
    if mix.shape[0] == 1:
        mix = np.repeat(mix, 2, axis=0)
    mix = mix[:2]
    if sr != SR:
        import torch
        import torchaudio.functional as F
        mix = F.resample(torch.from_numpy(mix), sr, SR).numpy()
    core = Core(args.model, args.provider)
    t0 = time.perf_counter()
    drums = separate(mix, core, lambda i, n: print(f"\r{i}/{n}", end="", flush=True))
    print(f"\n{core.provider}: {time.perf_counter() - t0:.1f} s")
    if sr != SR:
        import torch
        import torchaudio.functional as F
        drums = F.resample(torch.from_numpy(drums.astype(np.float32)), SR, sr).numpy()
    sf.write(args.output, drums.T, sr, subtype="FLOAT")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="cmd", required=True)
    v = sub.add_parser("verify")
    v.add_argument("--model", required=True)
    v.add_argument("--seconds", type=float, default=20.0)
    b = sub.add_parser("bench")
    b.add_argument("--model", required=True)
    b.add_argument("--seconds", type=float, default=240.0)
    b.add_argument("--runs", type=int, default=3)
    f = sub.add_parser("fixture")
    f.add_argument("--out", required=True)
    f.add_argument("--seconds", type=float, default=20.0)
    s = sub.add_parser("separate")
    s.add_argument("--model", required=True)
    s.add_argument("--provider", default="DmlExecutionProvider")
    s.add_argument("input")
    s.add_argument("output")
    args = parser.parse_args()
    raise SystemExit({"verify": cmd_verify, "bench": cmd_bench, "fixture": cmd_fixture,
                      "separate": cmd_separate}[args.cmd](args) or 0)


if __name__ == "__main__":
    main()
