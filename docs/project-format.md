# Project file format (`project.json`, format_version 1)

`hs_core::Project` is the authoritative definition. The file is serialised with
serde, and fields with defaults may be omitted. Paths are relative to the project
folder unless absolute.

```jsonc
{
  "format_version": 1,
  "name": "Artist - Title",
  "mapset_dir": "C:/Users/me/AppData/Local/osu!/Songs/123456 Artist - Title",
  "audio": {
    "monitor": "original",                       // or "stems"
    "stem": "stems/drums.ogg",                   // one file (D-34), or null
    // D-73: set only for a stem hitsounds-rs separated itself; omitted for an imported
    // stem. Lets the app reuse it and warn when the song changes.
    "stem_origin": {
      "kind": "separated",
      "model_sha256": "…",                       // the user's ONNX model file (D-71)
      "song_sha256": "…"                         // the song file it was made from
    }
  },
  // Order = priority (index 0 wins clashes, D-09)
  "channels": [
    {
      "id": 1, "name": "Kick (acoustic)", "color": "#e5484d",
      "sample_set": "drum", "hitsound": "normal",
      "sample": { "kind": "file", "path": "samples/kick_acoustic.wav" },
      "volume": 1.0, "muted": false, "solo": false,
      "events": [1234, 1696, 2158]
    },
    {
      "id": 5, "name": "Fallback", "color": "#8e8c99",
      "sample_set": "soft", "hitsound": "normal",
      "sample": { "kind": "skin_default" },
      "volume": 0.7, "events": []
    }
  ],
  "fallback": 5,
  // Master volume envelope (D-44). Empty or missing = 100 % everywhere.
  // curve is the segment to the next point: "linear" (ramp) or "step" (hold).
  "volume": [
    { "time": 0, "volume": 100, "curve": "linear" },
    { "time": 64000, "volume": 60, "curve": "step" }
  ],
  "diffs": [
    {
      "file": "Artist - Title (me) [Hard].osu",
      "label": "Hard",
      "enabled": true,
      "match_leniency_ms": 5.0,
      "fallback": null,                         // null = project fallback
      "tick_policy": "ignore",                  // or "render"
      "slider_end_mute": {                      // null = off (D-33)
        "divisors": [1, 4, 3, 8, 6, 16, 12],    // priority order
        "muted": [4, 3, 8, 6, 16, 12],
        "min_length_beats": 0.5,
        "spinner_ends": true,
        "sample_set": "auto",
        "index": null,                          // null = keep current index
        "volume": 5
      },
      "volume": null,                           // own envelope replacing the base one, or null
      "overrides": [
        { "op": "replace_channel", "from": 1, "to": 3, "range": { "start": 0, "end": 45000 } },
        { "op": "remove_event", "channel": 2, "time": 20123 },
        { "op": "add_event", "channel": 4, "time": 30500 },
        { "op": "mute_channel", "channel": 6, "range": null },
        { "op": "slider_body", "slider_time": 41000, "sample_set": "soft", "whistle": false }
      ]
    }
  ],
  "export": {
    "write_mode": "direct",                     // or "hitsound_only"
    "first_custom_index": 1,
    "body_sample_first_index": 100,
    "grouping_leniency_ms": 15.0,
    "addition_only_normal": "skin_default",     // or "skin_soft", "skin_normal", "skin_drum" (D-78), "silent" (OQ-1)
    "allow_growth": true,
    "auto_fix": true,
    "take_over_samples": false,
    "mute_slider_slides": false,
    "mute_slider_ticks": false,
    "single_format": "wav16",                   // or "source", "wav_float", "ogg" (D-69)
    "mixed_format": "wav16",                    // "wav16", "wav_float" or "ogg"
    "remove_redundant_greenlines": true          // D-80
  },
  "next_channel_id": 7,
  "schema": []                                 // custom indices saved after a direct export
}
```

The skin folder is not part of the project (D-40). It is an app setting. Older files
may still contain `"skin_dir"`. It is read once on open (as the app-wide folder, when
none is set yet) and dropped on the next save.

## Compatibility rules
- Bump `format_version` for any change that older builds would misread. `Project::load`
  refuses files that are too new.
- Add new fields with `#[serde(default)]` so older files keep loading.
- Channel IDs are never reused, so overrides referencing a deleted channel can be
  detected and reported rather than silently retargeted.

Direct exports store the generated sample names and previous slider writes in
`exports/manifest.json`. Keep this file with the project to retain ownership and
slider edit history across exports.
