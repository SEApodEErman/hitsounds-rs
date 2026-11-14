hitsounds-rs can separate a song's drums into a stem, so you hear the drums you are
hitsounding instead of the full mix. This is optional: you can import a stem from any
other tool instead.

## Why you make the model yourself

Separation uses the drums model of **HT-Demucs FT** (Meta's Demucs, MIT). The licence of
its trained weights isn't clear, so hitsounds-rs never ships, hosts or downloads them.
You convert the official model to a file once, with a script in the repository, and
point hitsounds-rs at it.

## Make the model file (once)

You need Python 3.12 and [uv](https://docs.astral.sh/uv/). From a copy of the
repository:

```sh
cd tools/demucs-export
uv venv --python 3.12 .venv
uv pip install --python .venv/Scripts/python.exe torch torchaudio --index-url https://download.pytorch.org/whl/cpu
uv pip install --python .venv/Scripts/python.exe demucs==4.1.0 onnx onnxruntime soundfile numpy
.venv/Scripts/python.exe export_drums_onnx.py --out htdemucs_ft_drums.onnx
```

This downloads the official weights (80 MB) and writes `htdemucs_ft_drums.onnx`
(185 MB). Keep the file somewhere permanent. The
[export README](https://github.com/SEApodEErman/hitsounds-rs/tree/main/tools/demucs-export)
has the details and a check you can run.

## Use it

1. **App settings** > **Drum separation model** > choose the `.onnx` file. hitsounds-rs
   loads it once to check that it's the drums model.
2. Open a project, then **Project settings** > **Drum stem** > **Separate from song**.
   New projects offer it right after they're created.
3. A progress card shows the segments and the time left. You can keep editing and
   playing while it runs, or cancel it.

The stem is saved as `stems\drums.ogg` in the project and lines up with the song.
Press **T** to switch between the song and the stem.

## Good to know

- It runs on the CPU. Expect roughly half the song's length on a 6-core desktop CPU.
- Separating the same song with the same model again reuses the stem.
- If the mapset's song changes, hitsounds-rs says the stem is out of date and offers to
  separate it again.
- Applying a stem is one undo step.
