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

## Commits

- **Commit without being asked** once a change is finished and verified
  (`cargo test --workspace` and clippy pass). Commit on the current branch. Never push,
  amend, or rewrite history unless asked.
- **Use Conventional Commits:** `type(scope): summary`.
  - **Types:** `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `style`, `build`, `ci`,
    `chore`.
  - **Scope** (optional): the crate or area, such as `editor`, `hs-core`, `export` or
    `audio`.
  - **Summary:** imperative and lower case, no full stop, at most 72 characters.
    Example: `feat(editor): show hit volume values in a hover bubble`.
  - **Body** (optional): say why, and cite decision IDs (`D-93`). Mark a breaking change
    with `!` after the type, as in `feat!: ...`.
- **One logical change per commit.** The code, its tests and its doc updates go
  together.
- **Stage only the files you changed, by path** (`git add <paths>`). Never use
  `git add -A` or `git add .`: the working tree often holds the user's own uncommitted
  work. If a file you edited already had the user's uncommitted changes, leave it out
  and say so, since hunks can't be split non-interactively here.
- **Long commit messages:** write them to a scratchpad file and use `git commit -F`,
  not an inline heredoc (see below).

## Editing files on this machine (Windows)

- **Files are LF.** Python's text mode on Windows writes CRLF, so a Python script that
  rewrites a file must use `open(p, 'w', encoding='utf-8', newline='\n')`. Prefer the
  Edit tool, which keeps line endings. After any scripted rewrite, check that the file
  has no `\r\n`.
- **No inline heredoc scripts in the Bash tool.** `python - <<'EOF' ... EOF` with a long
  body fails with "unexpected EOF while looking for matching `''", and so do one-liners
  that mix `$(...)` with `$'\r'`. Write the script to the scratchpad with the Write tool,
  then run `python <path>`. Use PowerShell for byte-level checks such as counting CRLF.
- **Don't run `cargo fmt` on a whole crate.** The tree is not rustfmt-clean, so it
  reformats files you never touched. Format only the files you changed:
  `rustfmt --edition 2024 <files>`.
- **QA hook files take no trailing newline.** Write `HITSOUNDS_RS_QA_CAPTURE` and
  `HITSOUNDS_RS_QA_INPUT` with `printf '%s'`, not `echo`; a newline in the capture path
  panics the app.
