# References

## osu! documentation
- `.osu` file format: https://osu.ppy.sh/wiki/en/Client/File_formats/osu_(file_format)
  (timing points, hit objects, `hitSample`, `edgeSounds`/`edgeSets`, sample lookup order)
- Storyboard scripting / sound samples: https://osu.ppy.sh/wiki/en/Storyboard/Scripting/Audio
- Hitsounds overview: https://osu.ppy.sh/wiki/en/Gameplay/Hitsound
- Custom samples / skinning sounds: https://osu.ppy.sh/wiki/en/Skinning/Sounds
- Ranking Criteria (General + Audio sections): https://osu.ppy.sh/wiki/en/Ranking_criteria
- 44-byte blank sound file: linked from the Ranking Criteria Audio section

## Mapping Tools (MIT, OliBomby and contributors)
- Repository: https://github.com/OliBomby/Mapping_Tools. Source reviewed at commit
  `1fa4171354145ee8dcfc476522019fa82e40bdea` (2026-11-07).
- Files that define the conventions hitsounds-rs follows:

| Topic | Path |
|---|---|
| Hitsound Studio algorithms | `Mapping_Tools.Core/Tools/HitsoundStudio/HitsoundStudioEngine.cs` |
| Packages and the hierarchy rule | `Mapping_Tools.Core/HitsoundStuff/SamplePackage.cs` |
| Custom index fits/merge | `Mapping_Tools.Core/HitsoundStuff/CustomIndex.cs` |
| Sample schema (persisted indices) | `Mapping_Tools.Core/HitsoundStuff/SampleSchema.cs` |
| Hitsound Studio options and defaults | `Mapping_Tools.Application/Tools/HitsoundStudio/Models/HitsoundStudioServiceOptions.cs` |
| Hitsound Copier | `Mapping_Tools.Core/Tools/HitsoundCopier/HitsoundCopierEngine.cs`, `Models/HitsoundCopierEngineOptions.cs` |
| Hitsound Preview Helper | `Mapping_Tools.Core/Tools/HitsoundPreviewHelper/HitsoundPreviewHelperEngine.cs` |
| Greenline merge | `Mapping_Tools.Core/BeatmapHelper/TimingPointChange.cs` |
| Sample resolution (what plays) | `Mapping_Tools.Core/BeatmapHelper/TimelineObject.cs`, `Timeline.cs` (`+5 ms` lookup) |
| Filename parsing | `Mapping_Tools.Core/HitsoundStuff/HitsoundFilename.cs` |

## Rust / app stack
- egui and eframe (UI): https://github.com/emilk/egui · egui_kittest (UI tests): https://github.com/emilk/egui/tree/main/crates/egui_kittest
- cpal (audio output): https://github.com/RustAudio/cpal
- symphonia (decoding): https://github.com/pdeljanov/Symphonia
- rubato (resampling): https://github.com/HEnquist/rubato
- notify (file watching): https://github.com/notify-rs/notify
- rosu-map (another `.osu` parser, useful as a cross-check; hitsounds-rs uses its own
  line-preserving parser for byte-identical writes): https://github.com/MaxOhn/rosu-map
