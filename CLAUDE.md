# hitsounds-rs: notes for Claude

- **The app is a Windows-only egui app** (`crates/hitsounds-rs`, D-48 to D-65). Before UI
  work, read `docs/design.md` (start at §11, "Where we left off") and the roadmap. The
  bar is flagship polish (D-55).
- A DAW-style osu!stable hitsounding tool. Read `docs/decisions.md` before changing
  behaviour, and cite decision IDs (`D-09`) in code comments where a rule comes from one.
- osu! semantics live in `docs/findings.md`. If you change a rule, update findings and
  mark anything not verified in game as **unverified**.
- Keep `osu-format` and `hs-core` pure (no audio, no GUI, no I/O beyond explicit
  load/save). `hs-separate` (drum separation, D-70 to D-73) is pure the same way: audio
  in, drums out, loading only ONNX Runtime and the model from given paths. Never commit
  model weights or `.onnx` files (D-71). `hs-session` holds the editor session with no GUI dependency (D-48);
  `hitsounds-rs` holds UI state only and calls it directly.
- Untouched `.osu` lines must round-trip byte-for-byte. Add a round-trip test for any
  parser change.
- Code ported from Mapping Tools (MIT) gets a header comment naming the source file,
  plus an entry in `THIRD_PARTY_NOTICES.md`.
- Never add copyrighted test material (mapsets, songs, skins, samples). Use synthetic
  fixtures.
- Commands: `cargo run -p hitsounds-rs` (Windows; run
  `tools/fetch-onnxruntime.ps1` once so drum separation has its runtime, D-74),
  `cargo test --workspace`, `cargo clippy --workspace --all-targets`.
