"""Builds a synthetic demo mapset, project and app config for screenshots and manual QA.

Everything is generated here (a sine-and-noise drum loop, made-up samples and two
difficulties), so nothing copyrighted is involved and the output can be shared.

    python tools/demo-mapset/make_demo.py [out_dir]      # default: sandbox/demo

Then run the app against it with its own config folder:

    $env:HITSOUNDS_RS_CONFIG_DIR = "<out_dir>\\config"; cargo run -p hitsounds-rs

Standard library only.
"""

import json
import math
import random
import struct
import sys
import wave
from pathlib import Path

RATE = 44100
BPM = 120
BEAT = 60_000 / BPM  # ms
OFFSET = 200  # ms of the first beat
BARS = 20
SET_NAME = "900001 hitsounds-rs - Demo Beat"

random.seed(7)


def write_wav(path: Path, frames: list[float], channels: int = 1) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    peak = max(1e-9, max(abs(s) for s in frames))
    scale = 0.9 / peak if peak > 0.9 else 1.0
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(s * scale * 32767)) for s in frames))


def kick(seconds=0.35):
    n = int(RATE * seconds)
    out = []
    phase = 0.0
    for i in range(n):
        t = i / RATE
        freq = 45 + 110 * math.exp(-t * 30)
        phase += 2 * math.pi * freq / RATE
        out.append(math.sin(phase) * math.exp(-t * 9))
    return out


def snare(seconds=0.25):
    n = int(RATE * seconds)
    return [
        (0.6 * (random.random() * 2 - 1) + 0.4 * math.sin(2 * math.pi * 190 * i / RATE))
        * math.exp(-i / RATE * 18)
        for i in range(n)
    ]


def hat(seconds=0.08):
    n = int(RATE * seconds)
    prev = 0.0
    out = []
    for i in range(n):
        x = random.random() * 2 - 1
        out.append((x - prev) * math.exp(-i / RATE * 60) * 0.5)  # crude high-pass
        prev = x
    return out


def crash(seconds=1.2):
    n = int(RATE * seconds)
    prev = 0.0
    out = []
    for i in range(n):
        x = random.random() * 2 - 1
        out.append((x - 0.6 * prev) * math.exp(-i / RATE * 3.5) * 0.5)
        prev = x
    return out


def tone(freq, seconds=0.12, decay=25):
    n = int(RATE * seconds)
    return [math.sin(2 * math.pi * freq * i / RATE) * math.exp(-i / RATE * decay) * 0.6 for i in range(n)]


def mix_into(buf, sample, at_ms, gain=1.0):
    start = int(at_ms / 1000 * RATE)
    for i, s in enumerate(sample):
        if 0 <= start + i < len(buf):
            buf[start + i] += s * gain


# Pattern per bar (in 1/2 beats): kick, snare, hats, crash every 4 bars.
KICKS = [0, 3, 4]  # half-beat steps within a bar of 8
SNARES = [2, 6]


def song() -> list[float]:
    length_ms = OFFSET + BARS * 4 * BEAT + 1500
    buf = [0.0] * int(length_ms / 1000 * RATE)
    k, s, h, c = kick(), snare(), hat(), crash()
    roots = [55.0, 55.0, 65.41, 49.0]  # A1, A1, C2, G1
    for bar in range(BARS):
        bar_ms = OFFSET + bar * 4 * BEAT
        for step in range(8):
            t = bar_ms + step * BEAT / 2
            if step in KICKS:
                mix_into(buf, k, t, 0.9)
            if step in SNARES:
                mix_into(buf, s, t, 0.6)
            mix_into(buf, h, t, 0.35 if step % 2 else 0.5)
        if bar % 4 == 0:
            mix_into(buf, c, bar_ms, 0.4)
        # A soft bass note per bar so the waveform isn't only drums.
        root = roots[bar % 4]
        n = int(4 * BEAT / 1000 * RATE)
        start = int(bar_ms / 1000 * RATE)
        for i in range(n):
            if start + i < len(buf):
                env = min(1.0, i / 2000) * math.exp(-i / RATE * 0.8)
                buf[start + i] += 0.25 * env * math.sin(2 * math.pi * root * i / RATE)
    return buf


def osu_file(version: str, objects: list[str], od: float, ar: float) -> str:
    lines = [
        "osu file format v14",
        "",
        "[General]",
        "AudioFilename: audio.wav",
        "AudioLeadIn: 0",
        "PreviewTime: -1",
        "Countdown: 0",
        "SampleSet: Soft",
        "StackLeniency: 0.7",
        "Mode: 0",
        "LetterboxInBreaks: 0",
        "WidescreenStoryboard: 1",
        "",
        "[Editor]",
        "DistanceSpacing: 1",
        "BeatDivisor: 4",
        "GridSize: 32",
        "TimelineZoom: 1",
        "",
        "[Metadata]",
        "Title:Demo Beat",
        "TitleUnicode:Demo Beat",
        "Artist:hitsounds-rs",
        "ArtistUnicode:hitsounds-rs",
        "Creator:hitsounds-rs",
        f"Version:{version}",
        "Source:",
        "Tags:demo synthetic",
        "BeatmapID:0",
        "BeatmapSetID:-1",
        "",
        "[Difficulty]",
        "HPDrainRate:5",
        "CircleSize:4",
        f"OverallDifficulty:{od}",
        f"ApproachRate:{ar}",
        "SliderMultiplier:1.4",
        "SliderTickRate:1",
        "",
        "[Events]",
        "//Background and Video events",
        "//Break Periods",
        "//Storyboard Layer 0 (Background)",
        "//Storyboard Layer 1 (Fail)",
        "//Storyboard Layer 2 (Pass)",
        "//Storyboard Layer 3 (Foreground)",
        "//Storyboard Layer 4 (Overlays)",
        "//Storyboard Sound Samples",
        "",
        "[TimingPoints]",
        f"{OFFSET},{BEAT},4,2,0,60,1,0",
        "",
        "",
        "[HitObjects]",
        *objects,
        "",
    ]
    return "\r\n".join(lines)


def position(i: int) -> tuple[int, int]:
    angle = i * 0.9
    return int(256 + 150 * math.cos(angle)), int(192 + 110 * math.sin(angle))


def circle(i, t, new_combo=False):
    x, y = position(i)
    return f"{x},{y},{int(round(t))},{5 if new_combo else 1},0,0:0:0:0:"


def slider(i, t, beats, new_combo=False):
    x, y = position(i)
    x2, y2 = min(500, x + 90), max(10, y - 40)
    length = round(140 * beats)  # SliderMultiplier 1.4 at SV 1: 140 px per beat
    return f"{x},{y},{int(round(t))},{6 if new_combo else 2},0,B|{x2}:{y2},1,{length}"


def normal_diff() -> list[str]:
    objects, i = [], 0
    for bar in range(BARS):
        bar_ms = OFFSET + bar * 4 * BEAT
        if bar % 2 == 0:
            objects.append(slider(i, bar_ms, 1, new_combo=True)); i += 1
            objects.append(circle(i, bar_ms + 2 * BEAT)); i += 1
            objects.append(circle(i, bar_ms + 3 * BEAT)); i += 1
        else:
            for beat in range(4):
                objects.append(circle(i, bar_ms + beat * BEAT, new_combo=beat == 0)); i += 1
    return objects


def hard_diff() -> list[str]:
    objects, i = [], 0
    for bar in range(BARS):
        bar_ms = OFFSET + bar * 4 * BEAT
        # Sliders end before the next object, never on it (one hit per moment).
        objects.append(slider(i, bar_ms, 0.5, new_combo=True)); i += 1
        for step in (2, 3, 4, 6):
            t = bar_ms + step * BEAT / 2
            if step == 4 and bar % 2:
                objects.append(slider(i, t, 0.5)); i += 1
            elif not (step == 6 and bar % 2):
                objects.append(circle(i, t)); i += 1
        objects.append(circle(i, bar_ms + 7 * BEAT / 2)); i += 1
    return objects


def main() -> None:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / "sandbox" / "demo").resolve()
    songs = root / "Songs"
    mapset = songs / SET_NAME
    mapset.mkdir(parents=True, exist_ok=True)
    write_wav(mapset / "audio.wav", song())
    diffs = {"Normal": (normal_diff(), 5, 6), "Hard": (hard_diff(), 7, 8)}
    files = {}
    for version, (objects, od, ar) in diffs.items():
        name = f"hitsounds-rs - Demo Beat (hitsounds-rs) [{version}].osu"
        (mapset / name).write_bytes(osu_file(version, objects, od, ar).encode("utf-8"))
        files[version] = name

    # A synthetic "skin" with every default hitsound, so preview works out of the box.
    skin = root / "skin"
    sounds = {"hitnormal": 330, "hitwhistle": 880, "hitfinish": 520, "hitclap": 660}
    for i, sample_set in enumerate(("normal", "soft", "drum")):
        for sound, freq in sounds.items():
            write_wav(skin / f"{sample_set}-{sound}.wav", tone(freq * (1 + 0.15 * i)))

    library = root / "library"
    write_wav(library / "kick.wav", kick())
    write_wav(library / "snare.wav", snare())
    write_wav(library / "hat.wav", hat())
    write_wav(library / "crash.wav", crash())
    write_wav(library / "rim.wav", tone(1200, 0.06, 60))

    project_dir = root / "projects" / "Demo Beat.hsproj"
    samples = project_dir / "samples"
    for name in ("kick", "snare", "hat", "crash"):
        (samples).mkdir(parents=True, exist_ok=True)
        (samples / f"{name}.wav").write_bytes((library / f"{name}.wav").read_bytes())

    def steps(predicate):
        return [
            int(round(OFFSET + bar * 4 * BEAT + step * BEAT / 2))
            for bar in range(BARS)
            for step in range(8)
            if predicate(bar, step)
        ]

    channels = [
        ("Kick", "#e5484d", "drum", "normal", "samples/kick.wav", 1.0, steps(lambda b, s: s in KICKS)),
        ("Snare", "#f5a524", "normal", "clap", "samples/snare.wav", 0.9, steps(lambda b, s: s in SNARES)),
        ("Hi-hat", "#30a46c", "soft", "whistle", "samples/hat.wav", 0.6, steps(lambda b, s: True)),
        ("Crash", "#3e63dd", "soft", "finish", "samples/crash.wav", 0.8, steps(lambda b, s: s == 0 and b % 4 == 0)),
    ]
    project = {
        "format_version": 1,
        "name": "hitsounds-rs - Demo Beat",
        "mapset_dir": str(mapset),
        "audio": {"monitor": "original", "stem": None},
        "channels": [
            {
                "id": i + 1,
                "name": name,
                "color": color,
                "sample_set": sample_set,
                "hitsound": hitsound,
                "sample": {"kind": "file", "path": path},
                "volume": volume,
                "events": events,
            }
            for i, (name, color, sample_set, hitsound, path, volume, events) in enumerate(channels)
        ]
        + [
            {
                "id": 5,
                "name": "Fallback",
                "color": "#8e8c99",
                "sample_set": "soft",
                "hitsound": "normal",
                "sample": {"kind": "skin_default"},
                "volume": 0.7,
                "events": [],
            }
        ],
        "fallback": 5,
        "diffs": [{"file": files[v], "label": v} for v in diffs],
        "next_channel_id": 6,
    }
    (project_dir / "project.json").write_text(json.dumps(project, indent=2), encoding="utf-8")

    config = root / "config"
    config.mkdir(parents=True, exist_ok=True)
    settings = {
        "skin_dir": str(skin),
        "library": [str(library)],
        "songs_dir": str(songs),
        "projects_dir": str(root / "projects"),
        "update_checks": False,
        "recent": [
            {
                "file": str(project_dir / "project.json"),
                "name": project["name"],
                "mapset_dir": str(mapset),
                "opened_at": 1_763_000_000_000,
            }
        ],
    }
    (config / "settings.json").write_text(json.dumps(settings, indent=2), encoding="utf-8")
    print(f"Demo written to {root}")
    print(f"Config folder: {config}")


if __name__ == "__main__":
    main()
