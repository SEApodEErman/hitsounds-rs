Thanks for testing hitsounds-rs. A good report gets a bug fixed much faster.

## Before you report

- Check you're on the latest version: **Help** > **About** shows yours, and
  [Releases](https://github.com/SEApodEErman/hitsounds-rs/releases) the latest.
- Search the [existing issues](https://github.com/SEApodEErman/hitsounds-rs/issues)
  and the [[FAQ]].

## What to include

[Open a bug report](https://github.com/SEApodEErman/hitsounds-rs/issues/new?template=bug_report.yml).
The form asks for:

1. **What happened, and what you expected.**
2. **Steps to reproduce**, as exact as you can.
3. **Your version** (**Help** > **About**) and whether you use the installer or the
   portable build.
4. **`crash.log`**, if the app closed or showed a crash dialog (see below).
5. For wrong sounds in game: the difficulty, the time in the map (for example
   `01:23:456`), what you placed, what osu! played, and the **Compare .osu** output
   for that spot if you can.

## Finding crash.log

When hitsounds-rs crashes, it writes `crash.log` and offers to show it in File
Explorer. You can also open it yourself:

1. Press **Win+R**, type `%APPDATA%\dev.hitsounds-rs.app` and press **Enter**.
2. Attach `crash.log` (and `crash.old.log`, if there is one) to the issue.

The log has the time, version, thread, error message and a backtrace. It doesn't
contain your projects, but file paths in it may include your Windows user name; edit
them out if you prefer.

## About maps and samples

Please don't attach songs, mapsets, skins or sample packs you don't own. Instead:

- link the beatmap page, if the map is public, or
- describe the timing and objects around the problem, or
- attach your `project.json` (from the `.hsproj` folder). It holds channels, hits and
  settings, but no audio. Paths in it may include your Windows user name.

## Feature requests

Have an idea? [Request a feature](https://github.com/SEApodEErman/hitsounds-rs/issues/new?template=feature_request.yml).
Describe the problem you're trying to solve, not only the solution: that helps find
the best fit for the workflow.
