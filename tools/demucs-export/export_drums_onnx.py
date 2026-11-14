"""Export the drums model of HT-Demucs FT to ONNX for hitsounds-rs (D-70, D-71).

hitsounds-rs never ships or downloads model weights (D-71): their licence is unclear
(see the Demucs README). Run this once yourself, then point hitsounds-rs's settings at
the .onnx file it writes.

    python export_drums_onnx.py --out htdemucs_ft_drums.onnx

The model comes from the maintained Demucs fork (github.com/adefossez/demucs, MIT
code), Hugging Face repo `adefossez/HTDemucs-ft`. `htdemucs_ft` is a bag of four
specialists whose bag weights are the identity, so drums come from the first model
(`f7e0c4bc`) alone and only that one is exported.

The STFT and iSTFT stay outside the graph, as in other Demucs ONNX exports, because
ONNX has no complex tensors. The graph takes one 7.8 s segment and returns the drums
source of both branches, already de-normalised:

    inputs   mix       float32 [1, 2, 343980]        stereo, 44.1 kHz
             spec      float32 [1, 4, 2048, 336]     complex-as-channels STFT of mix
    outputs  spec_out  float32 [1, 4, 2048, 336]     drums, frequency branch
             time_out  float32 [1, 2, 343980]        drums, time branch

drums = time_out + iSTFT(spec_out). `reference.py` implements the outside parts in
NumPy and is what `hs-separate` mirrors in Rust.
"""

import argparse
import hashlib
import json
from pathlib import Path

import torch
from torch import nn

REPO = "adefossez/HTDemucs-ft"
DRUMS_SIG = "f7e0c4bc"  # first model of the bag; bag weights row 0 = drums only
OPSET = 17


def load_drums_model():
    from huggingface_hub import hf_hub_download
    from demucs.hf import load_safetensors_model

    path = hf_hub_download(REPO, f"{DRUMS_SIG}.safetensors")
    model = load_safetensors_model(path)
    model.eval()
    assert model.sources[0] == "drums", model.sources
    assert model.cac and model.nfft == 4096 and model.samplerate == 44100
    return model, Path(path)


class DrumsCore(nn.Module):
    """HTDemucs.forward without the STFT, masking and iSTFT.

    Rather than copying the forward pass, it swaps the model's spectral helpers for
    pass-throughs: `_spec` returns the given spectrogram, `_magnitude` keeps it,
    `_mask` hands back the frequency-branch output (complex-as-channels), and
    `_ispec` contributes zeros so the model's own return value is the time branch.
    """

    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, mix, spec):
        m = self.model
        captured = []

        def mask(_z, x):
            captured.append(x)  # [B, S, 4, F, T], de-normalised
            return x

        def ispec(zout, length, scale=0):
            b, s = zout.shape[:2]
            return torch.zeros(b, s, 2, length, dtype=mix.dtype)

        m._spec = lambda _x: spec
        m._magnitude = lambda z: z
        m._mask = mask
        m._ispec = ispec
        try:
            time_out = m(mix)  # xt + zeros
        finally:
            for name in ("_spec", "_magnitude", "_mask", "_ispec"):
                del m.__dict__[name]
        return captured[0][:, 0], time_out[:, 0]


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", type=Path, default=Path("htdemucs_ft_drums.onnx"))
    args = parser.parse_args()

    model, weights = load_drums_model()
    length = int(model.segment * model.samplerate)  # 343980
    frames = -(-length // (model.nfft // 4))  # ceil, 336
    mix = torch.randn(1, 2, length)
    spec = torch.randn(1, 4, model.nfft // 2, frames)

    core = DrumsCore(model).eval()
    # The fused attention kernel (aten::_native_multi_head_attention) has no ONNX
    # mapping; the unfused path computes the same thing from plain ops.
    torch.backends.mha.set_fastpath_enabled(False)
    with torch.no_grad():
        torch.onnx.export(
            core,
            (mix, spec),
            str(args.out),
            input_names=["mix", "spec"],
            output_names=["spec_out", "time_out"],
            opset_version=OPSET,
            dynamo=False,
        )

    import onnx

    onnx_model = onnx.load(str(args.out))
    onnx.checker.check_model(onnx_model)
    meta = {
        "source": f"hf://{REPO}/{DRUMS_SIG}.safetensors",
        "weights_sha256": hashlib.sha256(weights.read_bytes()).hexdigest(),
        "samplerate": model.samplerate,
        "segment_samples": length,
        "nfft": model.nfft,
        "hop": model.nfft // 4,
        "opset": OPSET,
        "torch": torch.__version__,
    }
    for key, value in meta.items():
        entry = onnx_model.metadata_props.add()
        entry.key, entry.value = f"hitsounds_rs.{key}", str(value)
    onnx.save(onnx_model, str(args.out))
    digest = hashlib.sha256(args.out.read_bytes()).hexdigest()
    print(json.dumps({**meta, "onnx": str(args.out), "onnx_sha256": digest}, indent=2))


if __name__ == "__main__":
    main()
