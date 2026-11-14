# Third-party notices

## Mapping Tools

hitsounds-rs's hitsound merging, custom-index allocation, and greenline-merge
behaviour are modelled on [Mapping Tools](https://github.com/OliBomby/Mapping_Tools)
by OliBomby and contributors. Mapping Tools is licensed under the MIT License.

Where code is ported (translated) rather than re-implemented from a description,
the Rust file must carry a header comment naming the original Mapping Tools file,
and the MIT notice below applies to that portion.

```
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

Ported so far:

| hitsounds-rs file | Mapping Tools origin |
|---|---|
| `crates/hs-core/src/packing.rs` (priority resolution, `CustomIndex` fits/merge/optimize, index matching) | `Mapping_Tools.Core/HitsoundStuff/SamplePackage.cs`, `CustomIndex.cs`, `Tools/HitsoundStudio/HitsoundStudioEngine.cs` |
| `crates/osu-format/src/timing.rs` (`TimingPointChange` merge semantics) | `Mapping_Tools.Core/BeatmapHelper/TimingPointChange.cs` |

## Demucs

Drum stem separation (D-70 to D-74) follows [Demucs](https://github.com/adefossez/demucs)
by Alexandre Défossez, Meta Platforms, Inc. and contributors, licensed under the MIT
License (the notice above, with "Copyright (c) Meta Platforms, Inc. and affiliates.").
The MIT licence covers Demucs' **code** only. hitsounds-rs does not ship, host or download
Demucs model weights, whose licence the Demucs README calls unclear (D-71); users make
the model file themselves with `tools/demucs-export/`.

Ported so far:

| hitsounds-rs file | Demucs origin |
|---|---|
| `crates/hs-separate/src/stft.rs` (torch-compatible STFT, Demucs spectrogram padding, complex-as-channels) | `demucs/spec.py` (`spectro`, `ispectro`), `demucs/htdemucs.py` (`HTDemucs._spec`, `_ispec`, `_magnitude`, `_mask`) |
| `crates/hs-separate/src/apply.rs` (normalisation, segments, triangular overlap-add) | `demucs/apply.py` (`apply_model`, `TensorChunk.padded`), `demucs/utils.py` (`center_trim`), `demucs/api.py` (`Separator.separate_tensor`) |
| `tools/demucs-export/reference.py` (the NumPy twin of the above) | the same files |

## ONNX Runtime

Drum separation runs on [ONNX Runtime](https://github.com/microsoft/onnxruntime)
(Copyright (c) Microsoft Corporation, MIT License), through the `ort` crate (MIT or
Apache-2.0). hitsounds-rs ships Microsoft's CPU build 1.31.1 unchanged as
`onnxruntime.dll` next to the exe (D-74), together with its licence
(`onnxruntime-LICENSE.txt`) and its own third-party notices
(`onnxruntime-ThirdPartyNotices.txt`), as `tools/fetch-onnxruntime.ps1` fetches them.
The MIT notice above applies to it, with Microsoft's copyright line.

## libvorbis (aoTuV) and libogg

OGG sample export (D-69) compiles the aoTuV/Lancer build of libvorbis and libogg into
hitsounds-rs, through the `vorbis_rs`, `aotuv_lancer_vorbis_sys` and `ogg_next_sys`
crates (BSD-3-Clause).

libvorbis: aoTuV - Copyright (c) 2003-2021 Aoyumi. Copyright (c) 2002-2020 Xiph.org Foundation.
libogg: Copyright (c) 2002, Xiph.org Foundation.

```
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of the Xiph.org Foundation nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## osu! default samples

The osu! default skin samples are not freely redistributable and are **not**
bundled with hitsounds-rs (Mapping Tools does not bundle them either). Users point
hitsounds-rs at a skin folder of their own (see `docs/decisions.md`, D-30).

## UI fonts and icons

| Asset | Source | Licence |
|---|---|---|
| Inter Regular, Inter Display Medium | [Inter 4.1](https://github.com/rsms/inter/releases/tag/v4.1) | [SIL OFL 1.1](crates/hitsounds-rs/assets/Inter-OFL.txt) |
| IBM Plex Mono Medium | [IBM Plex 6.4.0](https://github.com/IBM/plex/releases/tag/v6.4.0) | [SIL OFL 1.1](crates/hitsounds-rs/assets/IBMPlexMono-OFL.txt) |
| Phosphor Regular | [Phosphor Icons](https://github.com/phosphor-icons/web), via `egui-phosphor` | [MIT](crates/hitsounds-rs/assets/Phosphor-MIT.txt) |
