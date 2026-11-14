//! The output device: a cpal stream that drives a [`Mixer`].
//!
//! The stream lives on its own thread (cpal streams are not `Send` on every
//! platform). Commands cross over a channel that the audio callback drains without
//! blocking, and the callback publishes a clock snapshot the UI reads. The mixer
//! survives stream replacement when Windows invalidates the output endpoint.

use crate::mixer::{Command, Mixer};
use crate::{MonitorSource, TransportState};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    ErrorKind, FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig,
    SupportedStreamConfig,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("no audio output device is available")]
    NoDevice,
    #[error("audio device error: {0}")]
    Device(String),
    #[error("unsupported output sample format {0}")]
    Format(String),
}

/// What the listener hears at instant `at`.
#[derive(Debug, Clone, Copy)]
struct Clock {
    position_ms: f64,
    at: Instant,
    playing: bool,
    rate: f64,
    monitor: MonitorSource,
    length_ms: f64,
    /// Output latency of the last callback: a seek is heard this long after it is sent.
    latency: Duration,
    /// Seeks sent to the render thread. While the render thread has handled fewer, its
    /// position is from before the latest seek and must not replace it (D-92).
    seeks: u64,
}

/// Latency reports above this are treated as bogus and clamped.
const MAX_LATENCY: Duration = Duration::from_millis(250);
const RETRY_INTERVAL: Duration = Duration::from_millis(200);

struct RenderState {
    rx: Receiver<Command>,
    mixer: Mixer,
    block: Vec<f32>,
    /// Seeks the mixer has handled, compared with [`Clock::seeks`].
    seeks: u64,
}

#[derive(Clone)]
struct Output {
    render: Arc<Mutex<RenderState>>,
    clock: Arc<Mutex<Clock>>,
    restart: Arc<AtomicBool>,
    worker: std::thread::Thread,
}

pub struct Engine {
    tx: Sender<Command>,
    clock: Arc<Mutex<Clock>>,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    /// Opens the default output device and starts the stream.
    pub fn start() -> Result<Self, EngineError> {
        let (tx, rx) = mpsc::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, EngineError>>();
        let clock = Arc::new(Mutex::new(Clock {
            position_ms: 0.0,
            at: Instant::now(),
            playing: false,
            rate: 1.0,
            monitor: MonitorSource::Original,
            length_ms: 0.0,
            latency: Duration::ZERO,
            seeks: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let thread = {
            let clock = clock.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("hs-audio".into())
                .spawn(move || run_output(rx, clock, stop, ready_tx))
                .map_err(|e| EngineError::Device(e.to_string()))?
        };
        let sample_rate = ready_rx.recv().map_err(|_| EngineError::Device("audio thread exited".into()))??;
        Ok(Self { tx, clock, sample_rate, stop, thread: Some(thread) })
    }

    /// The mixer rate. Buffers sent to the engine must be stereo at this rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn send(&self, command: Command) {
        if let Command::Seek(ms) = command {
            // Show the new position at once instead of after the next callback. It is
            // heard one output latency from now, which is also what the callback will
            // report, so a seek during playback doesn't jump back a moment later.
            if let Ok(mut c) = self.clock.lock() {
                c.position_ms = ms.max(0.0);
                c.at = Instant::now() + c.latency;
                // Counted before sending, so the render thread never sees more seeks
                // than the clock has.
                c.seeks += 1;
            }
        }
        let _ = self.tx.send(command);
    }

    pub fn state(&self) -> TransportState {
        let c = *self.clock.lock().unwrap_or_else(|e| e.into_inner());
        let mut position_ms = c.position_ms;
        if c.playing {
            let now = Instant::now();
            let delta = if now >= c.at {
                now.duration_since(c.at).as_secs_f64()
            } else {
                -c.at.duration_since(now).as_secs_f64()
            };
            position_ms += delta * 1000.0 * c.rate;
        }
        TransportState {
            playing: c.playing,
            position_ms: position_ms.clamp(0.0, c.length_ms.max(0.0)),
            rate: c.rate,
            monitor: c.monitor,
            length_ms: c.length_ms,
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

fn default_output() -> Result<(cpal::Device, SupportedStreamConfig), EngineError> {
    let device = cpal::default_host().default_output_device().ok_or(EngineError::NoDevice)?;
    let supported = device.default_output_config().map_err(|e| EngineError::Device(e.to_string()))?;
    Ok((device, supported))
}

fn run_output(
    rx: Receiver<Command>,
    clock: Arc<Mutex<Clock>>,
    stop: Arc<AtomicBool>,
    ready: Sender<Result<u32, EngineError>>,
) {
    let initial = default_output().and_then(|(device, supported)| {
        let rate = supported.sample_rate();
        let output = Output {
            render: Arc::new(Mutex::new(RenderState {
                rx,
                mixer: Mixer::new(rate),
                block: Vec::new(),
                seeks: 0,
            })),
            clock,
            restart: Arc::new(AtomicBool::new(false)),
            worker: std::thread::current(),
        };
        let stream = open_stream(&device, supported, rate, &output)?;
        Ok((output, stream, rate))
    });
    let (output, stream, rate) = match initial {
        Ok(initial) => initial,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    let _ = ready.send(Ok(rate));

    let mut stream = Some(stream);
    while !stop.load(Ordering::Relaxed) {
        if output.restart.swap(false, Ordering::Relaxed) {
            drop(stream.take());
            // The old endpoint can keep rendering after invalidation, but its
            // timestamps can no longer drive the playhead. Freeze it until the
            // replacement publishes its own clock. The mixer keeps its state.
            let render = output.render.lock().unwrap_or_else(|e| e.into_inner());
            let mut clock = output.clock.lock().unwrap_or_else(|e| e.into_inner());
            clock.position_ms = render.mixer.position_ms();
            clock.playing = false;
        }
        if stream.is_none() {
            // WASAPI shared mode converts to the endpoint's rate. Keep the
            // mixer rate so the already-conformed project buffers stay valid.
            stream = default_output()
                .and_then(|(device, config)| open_stream(&device, config, rate, &output))
                .ok();
        }
        std::thread::park_timeout(RETRY_INTERVAL);
    }
}

fn open_stream(
    device: &cpal::Device,
    supported: SupportedStreamConfig,
    rate: u32,
    output: &Output,
) -> Result<cpal::Stream, EngineError> {
    let format = supported.sample_format();
    let mut config: StreamConfig = supported.into();
    config.sample_rate = rate;
    let stream = match format {
        SampleFormat::F32 => build::<f32>(device, config, output),
        SampleFormat::I16 => build::<i16>(device, config, output),
        SampleFormat::I32 => build::<i32>(device, config, output),
        SampleFormat::U16 => build::<u16>(device, config, output),
        SampleFormat::U8 => build::<u8>(device, config, output),
        SampleFormat::F64 => build::<f64>(device, config, output),
        other => return Err(EngineError::Format(other.to_string())),
    }?;
    stream.play().map_err(|e| EngineError::Device(e.to_string()))?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    output: &Output,
) -> Result<cpal::Stream, EngineError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let Output {
        render,
        clock,
        restart,
        worker,
    } = output.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                // Only the current stream renders. The worker drops that
                // stream before inspecting the mixer or opening a replacement.
                let Ok(mut state) = render.try_lock() else {
                    return;
                };
                let RenderState { rx, mixer, block, seeks } = &mut *state;
                while let Ok(command) = rx.try_recv() {
                    if matches!(command, Command::Seek(_)) {
                        *seeks += 1;
                    }
                    mixer.handle(command);
                }
                let stamp = info.timestamp();
                let latency = stamp.playback.duration_since(stamp.callback).min(MAX_LATENCY);
                let heard_at = Instant::now() + latency;
                let position_ms = mixer.position_ms();

                block.clear();
                block.resize(data.len(), 0.0);
                mixer.render(block, channels);
                for (out, sample) in data.iter_mut().zip(block.iter()) {
                    *out = T::from_sample(*sample);
                }

                // Never block the audio thread on the UI: skip the update if contended.
                if let Ok(mut c) = clock.try_lock() {
                    // A seek sent while this block rendered is not in `position_ms`
                    // yet: keep the position `Engine::send` set for it (D-92).
                    let current = c.seeks == *seeks;
                    *c = Clock {
                        position_ms: if current { position_ms } else { c.position_ms },
                        at: if current { heard_at } else { c.at },
                        // A block that ran into the end reports the stopped state.
                        playing: mixer.playing(),
                        rate: mixer.rate(),
                        monitor: mixer.monitor(),
                        length_ms: mixer.length_ms(),
                        latency,
                        seeks: c.seeks,
                    };
                    if !mixer.playing() && current {
                        c.position_ms = mixer.position_ms();
                    }
                }
            },
            move |err| {
                eprintln!("hs-audio: stream error: {err}");
                if cfg!(windows)
                    && matches!(
                        err.kind(),
                        ErrorKind::StreamInvalidated | ErrorKind::DeviceNotAvailable
                    )
                {
                    restart.store(true, Ordering::Relaxed);
                    worker.unpark();
                }
            },
            None,
        )
        .map_err(|e| EngineError::Device(e.to_string()))
}
