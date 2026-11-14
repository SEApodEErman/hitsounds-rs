//! Beat ruler, stable wheel stepping, and time conversion (D-25, D-32, D-42).

use eframe::egui::Color32;
use hs_core::{Ms, projection::Redline};

pub const DIVISORS: [u32; 8] = [1, 2, 3, 4, 6, 8, 12, 16];
const DEFAULT_LINE: Redline = Redline {
    time: 0.0,
    beat_length: 500.0,
    meter: 4,
};

#[derive(Clone)]
pub struct BeatGrid {
    lines: Vec<Redline>,
    bars: Vec<i64>,
}

impl Default for BeatGrid {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl BeatGrid {
    pub fn new(mut lines: Vec<Redline>) -> Self {
        if lines.is_empty() {
            lines.push(DEFAULT_LINE);
        }
        let mut bars = vec![1_i64];
        for pair in lines.windows(2) {
            let a = pair[0];
            let beats = (pair[1].time - a.time) / a.beat_length;
            let measures = ((beats / f64::from(a.meter)) - 0.000001).ceil() as i64;
            bars.push(bars.last().copied().unwrap_or(1).saturating_add(measures));
        }
        Self { lines, bars }
    }

    fn active(&self, time: f64) -> usize {
        self.lines
            .partition_point(|line| line.time <= time)
            .saturating_sub(1)
    }

    /// The length of one snap step at `time`, in ms.
    pub fn step_ms(&self, time: f64, divisor: u32) -> f64 {
        self.lines[self.active(time)].beat_length / f64::from(divisor.max(1))
    }

    pub fn bar_beat(&self, time: f64) -> (i64, u32) {
        let i = self.active(time);
        let line = self.lines[i];
        let beat = ((time - line.time + 0.000001) / line.beat_length).floor() as i64;
        let meter = i64::from(line.meter);
        (
            self.bars[i].saturating_add(beat.div_euclid(meter)),
            (beat.rem_euclid(meter) + 1) as u32,
        )
    }

    pub fn snap(&self, time: f64, divisor: u32) -> Ms {
        let index = self.active(time);
        let line = self.lines[index];
        let step = line.beat_length / f64::from(divisor);
        let rounded = line.time + ((time - line.time) / step + 0.5).floor() * step;
        let nearest = self
            .lines
            .get(index + 1)
            .filter(|next| (next.time - time).abs() < (rounded - time).abs())
            .map_or(rounded, |next| next.time);
        nearest.max(0.0).round() as Ms
    }

    /// A redline is a tick in its own right. The 1 ms tolerance prevents rounded
    /// integer hit times from stalling the wheel at the same tick (D-42).
    pub fn step(&self, mut time: f64, divisor: u32, steps: i32) -> f64 {
        for _ in 0..steps.unsigned_abs() {
            let forward = steps > 0;
            let i = self.active(if forward { time } else { time - 1.0 });
            let line = self.lines[i];
            let size = line.beat_length / f64::from(divisor);
            time = if forward {
                let tick = line.time + (((time - line.time + 1.0) / size).floor() + 1.0) * size;
                self.lines.get(i + 1).map_or(tick, |next| {
                    if tick > next.time - 1.0 {
                        next.time
                    } else {
                        tick
                    }
                })
            } else {
                line.time + (((time - line.time - 1.0) / size).ceil() - 1.0) * size
            };
        }
        time.max(0.0)
    }

    pub fn ticks(
        &self,
        from: f64,
        until: f64,
        scale: f64,
        divisor: u32,
        mut draw: impl FnMut(Tick),
    ) {
        let first = self.active(from);
        for i in first..self.lines.len() {
            let line = self.lines[i];
            if line.time > until && i != first {
                break;
            }
            let start = if i == 0 { from } else { from.max(line.time) };
            let end = self
                .lines
                .get(i + 1)
                .map_or(until, |next| until.min(next.time - 0.000001));
            let step = line.beat_length / f64::from(divisor);
            let measure = i64::from(line.meter) * i64::from(divisor);
            let stride = if step * scale >= 7.0 {
                1
            } else if line.beat_length * scale >= 9.0 {
                i64::from(divisor)
            } else {
                let measure_ms = line.beat_length * f64::from(line.meter);
                measure.saturating_mul((14.0 / (measure_ms * scale)).ceil().max(1.0) as i64)
            };
            let mut index = (((start - line.time) / step - 0.000001) / stride as f64).ceil() as i64;
            let mut limit = ((end - start).max(0.0) * scale / 5.0).ceil() as usize + 3;
            while limit > 0 {
                let tick = index.saturating_mul(stride);
                let time = line.time + tick as f64 * step;
                if time > end {
                    break;
                }
                let bar = (tick.rem_euclid(measure) == 0)
                    .then(|| self.bars[i].saturating_add(tick.div_euclid(measure)));
                draw(Tick {
                    time,
                    beat: tick.rem_euclid(i64::from(divisor)) == 0,
                    every: stride == 1,
                    alternate: tick.div_euclid(i64::from(divisor)).rem_euclid(2) == 1,
                    bar,
                    color: snap_color(tick, divisor),
                });
                index += 1;
                limit -= 1;
            }
        }
    }
}

pub struct Tick {
    pub time: f64,
    pub beat: bool,
    /// True when every snap step is being drawn, not only beats or bars.
    pub every: bool,
    /// Odd beats, for the alternating shade of the step pads (D-61).
    pub alternate: bool,
    pub bar: Option<i64>,
    pub color: Color32,
}

/// Stable editor snap colours, verified in findings §9 (D-42).
fn snap_color(index: i64, divisor: u32) -> Color32 {
    let divisor = i64::from(divisor);
    let tick = index.rem_euclid(divisor);
    if tick == 0 {
        return Color32::WHITE;
    }
    [
        (2, [255, 0, 0]),
        (3, [200, 0, 200]),
        (4, [50, 128, 255]),
        (6, [144, 64, 144]),
        (8, [255, 255, 0]),
    ]
    .into_iter()
    .find(|(factor, _)| (tick * factor) % divisor == 0)
    .map_or(Color32::from_gray(160), |(_, [r, g, b])| {
        Color32::from_rgb(r, g, b)
    })
}

pub fn step_divisor(divisor: u32, up: bool) -> u32 {
    if up {
        DIVISORS
            .into_iter()
            .find(|&d| d >= divisor * 2)
            .unwrap_or(divisor)
    } else {
        DIVISORS
            .into_iter()
            .rev()
            .find(|&d| d <= divisor / 2)
            .unwrap_or(1)
    }
}

pub fn nearest<T>(items: &[T], time: f64, at: impl Fn(&T) -> Ms, limit: f64) -> Option<usize> {
    let index = items.partition_point(|value| f64::from(at(value)) < time);
    [index.checked_sub(1), (index < items.len()).then_some(index)]
        .into_iter()
        .flatten()
        .map(|i| (i, (time - f64::from(at(&items[i]))).abs()))
        .filter(|(_, distance)| *distance <= limit)
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(index, _)| index)
}

pub fn visible<T>(
    items: &[T],
    from: f64,
    to: f64,
    time: impl Fn(&T) -> Ms,
) -> std::ops::Range<usize> {
    let start = items.partition_point(|item| f64::from(time(item)) < from);
    let end = items.partition_point(|item| f64::from(time(item)) <= to);
    start..end.max(start)
}

pub fn timecode(time: f64) -> String {
    let ms = time.max(0.0).floor() as u64;
    format!("{:02}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
}
