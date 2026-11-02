//! ViuMic engine: microphone → neural denoiser → output device.
//!
//! The output is normally VB-CABLE's "CABLE Input"; other apps then pick the
//! "CABLE Output" microphone and hear the clean signal.
//!
//! Threads: the cpal capture and playback callbacks only move samples through lock-free
//! ring buffers. The model runs on its own thread in 10 ms hops.

mod denoise;
mod dpdfnet;
mod dynamics;

pub use denoise::{Denoiser, HOP, Model, SAMPLE_RATE, UNLIMITED_DB};

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// VB-CABLE's playback endpoint. Audio written here comes out of the "CABLE Output" mic.
/// VB-CABLE's device description; stays the same when the endpoints are renamed.
pub const CABLE_DEVICE: &str = "VB-Audio Virtual Cable";
/// Names ViuMic gives the cable's two endpoints (see the app's installer).
pub const VIUMIC_MIC: &str = "ViuMic Microphone";
pub const VIUMIC_FEED: &str = "ViuMic Feed";
/// VB-CABLE's own name for its playback endpoint, before renaming.
pub const CABLE_INPUT: &str = "CABLE Input";
pub const CABLE_URL: &str = "https://vb-audio.com/Cable/";

const MS: usize = SAMPLE_RATE as usize / 1000;
/// Adaptive jitter buffer: the player waits for `prime` audio before (re)starting. It starts
/// small and grows a step after each underrun, so latency only rises on setups whose
/// callbacks are uneven (VB-CABLE: 4 underruns in 10 min at a fixed 20 ms).
const PRIME_START: usize = 20 * MS;
const PRIME_STEP: usize = 5 * MS;
const PRIME_MAX: usize = 50 * MS;
/// More than `prime` + this queued: drop a hop. Bounds latency when the mic and output
/// clocks drift apart.
const DRIFT_MARGIN: usize = 40 * MS;

#[derive(Clone, Debug)]
pub struct Config {
    /// Microphone name (exact or substring). `None` = system default mic.
    pub input: Option<String>,
    /// Output device name (exact or substring). `None` = VB-CABLE.
    pub output: Option<String>,
    pub model: Model,
    /// Maximum noise attenuation in dB; [`UNLIMITED_DB`] = no limit.
    pub strength_db: f32,
    /// Noise removal on/off.
    pub enabled: bool,
    /// Mains hum to remove: 50 Hz (most of the world) or 60 Hz (the Americas), `None` = off.
    pub hum_hz: Option<f32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            model: Model::default(),
            strength_db: UNLIMITED_DB,
            enabled: true,
            hum_hz: Some(50.0),
        }
    }
}

/// Offline helper: runs hop-based `f` over a whole buffer and removes its `delay`, so the
/// output lines up with the input and has the same length.
pub fn process_buffer(
    input: &[f32],
    delay: usize,
    mut f: impl FnMut(&[f32; HOP], &mut [f32; HOP]) -> Result<()>,
) -> Result<Vec<f32>> {
    let mut padded = input.to_vec();
    padded.resize((input.len() + delay).div_ceil(HOP) * HOP, 0.0);
    let mut out = vec![0.0; padded.len()];
    for (i, o) in padded.chunks_exact(HOP).zip(out.chunks_exact_mut(HOP)) {
        f(i.try_into()?, o.try_into()?)?;
    }
    out.drain(..delay);
    out.truncate(input.len());
    Ok(out)
}

#[derive(Clone, Debug, Default)]
pub struct Devices {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub default_input: Option<String>,
    pub default_output: Option<String>,
}

impl Devices {
    pub fn has_cable(&self) -> bool {
        self.outputs.iter().any(|n| n.contains(CABLE_DEVICE))
    }
}

pub fn list_devices() -> Result<Devices> {
    let host = cpal::default_host();
    let names = |it: Box<dyn Iterator<Item = cpal::Device>>| it.filter_map(|d| name(&d)).collect();
    Ok(Devices {
        inputs: names(Box::new(host.input_devices()?)),
        outputs: names(Box::new(host.output_devices()?)),
        default_input: host.default_input_device().and_then(|d| name(&d)),
        default_output: host.default_output_device().and_then(|d| name(&d)),
    })
}

/// Snapshot of engine health. Peaks are linear (0..1) and reset on every read.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub in_peak: f32,
    pub out_peak: f32,
    /// Mic-to-output latency added by ViuMic: device buffers + ring buffers + effects.
    pub latency_ms: f32,
    /// Part of `latency_ms` spent in Windows/device buffers (Bluetooth makes this large).
    pub device_ms: f32,
    /// Samples dropped to keep latency bounded (clock drift or a stalled model).
    pub dropped: u32,
    /// Times the output ran dry and had to re-prime.
    pub underruns: u32,
    /// Glitches reported by the audio devices themselves.
    pub xruns: u32,
    /// Worst DSP time per 10 ms hop since the last read, as a fraction of the 10 ms budget.
    /// Above 1.0 the model can't keep up and audio will drop.
    pub load: f32,
}

#[derive(Default)]
struct Shared {
    running: AtomicBool,
    enabled: AtomicBool,
    strength_db: AtomicF32,
    in_peak: AtomicF32,
    out_peak: AtomicF32,
    capture_us: AtomicU32,
    playback_us: AtomicU32,
    /// Samples waiting in both ring buffers, as seen by the DSP thread.
    buffered: AtomicU32,
    dropped: AtomicU32,
    underruns: AtomicU32,
    /// Current jitter-buffer target, in samples (see `PRIME_START`).
    prime: AtomicU32,
    xruns: AtomicU32,
    load: AtomicF32,
    error: Mutex<Option<String>>,
}

impl Shared {
    fn fail(&self, msg: impl ToString) {
        if let Ok(mut e) = self.error.lock() {
            *e = Some(msg.to_string());
        }
    }
}

pub struct Engine {
    shared: Arc<Shared>,
    /// Model delay, in samples.
    delay: usize,
    pub input_name: String,
    pub output_name: String,
    dsp: Option<JoinHandle<()>>,
    // Dropped after `dsp` is joined (see Drop).
    _streams: (cpal::Stream, cpal::Stream),
}

impl Engine {
    pub fn start(cfg: &Config) -> Result<Self> {
        let host = cpal::default_host();
        let input = input_device(&host, cfg.input.as_deref())?;
        let output = match &cfg.output {
            Some(q) => find(host.output_devices()?, q)
                .with_context(|| format!("output \"{q}\" not found"))?,
            // "ViuMic Feed (": the bracket keeps "ViuMic Spare 16ch" and the like out.
            None => find(host.output_devices()?, &format!("{VIUMIC_FEED} ("))
                .or_else(|| find(host.output_devices().ok()?, CABLE_INPUT))
                .with_context(|| format!("VB-CABLE not found. Install it from {CABLE_URL}"))?,
        };
        let input_name = name(&input).unwrap_or_default();
        let output_name = name(&output).unwrap_or_default();
        if input_name.contains(CABLE_DEVICE) && output_name.contains(CABLE_DEVICE) {
            bail!("\"{input_name}\" is ViuMic's own output; pick your real microphone");
        }

        let in_cfg = input.default_input_config()?;
        if in_cfg.sample_format() != cpal::SampleFormat::F32 {
            bail!(
                "\"{input_name}\" uses {:?} samples; only f32 is supported",
                in_cfg.sample_format()
            );
        }
        // ponytail: 48 kHz mics only; add rubato on the input side when a non-48k mic shows up.
        if in_cfg.sample_rate() != SAMPLE_RATE {
            bail!(
                "\"{input_name}\" runs at {} Hz; only 48000 Hz is supported yet",
                in_cfg.sample_rate()
            );
        }
        let in_ch = in_cfg.channels() as usize;
        let out_ch = output.default_output_config()?.channels();
        // WASAPI converts the rate on output streams, so 48 kHz always works here.
        let out_cfg = cpal::StreamConfig {
            channels: out_ch,
            sample_rate: SAMPLE_RATE,
            buffer_size: cpal::BufferSize::Default,
        };
        let out_ch = out_ch as usize;

        let shared = Arc::new(Shared::default());
        shared.running.store(true, Relaxed);
        shared.enabled.store(cfg.enabled, Relaxed);
        shared.strength_db.set(cfg.strength_db);
        shared.prime.store(PRIME_START as u32, Relaxed);

        let (mut mic_tx, mic_rx) = RingBuffer::<f32>::new(SAMPLE_RATE as usize);
        let (out_tx, mut out_rx) = RingBuffer::<f32>::new(SAMPLE_RATE as usize);

        let sh = shared.clone();
        let input_stream = input.build_input_stream(
            in_cfg.config(),
            move |data: &[f32], info: &cpal::InputCallbackInfo| {
                let t = info.timestamp();
                sh.capture_us.store(
                    t.callback.saturating_duration_since(t.capture).as_micros() as u32,
                    Relaxed,
                );
                for frame in data.chunks_exact(in_ch) {
                    if mic_tx
                        .push(frame.iter().sum::<f32>() / in_ch as f32)
                        .is_err()
                    {
                        sh.dropped.fetch_add(1, Relaxed);
                    }
                }
            },
            stream_error(&shared),
            None,
        )?;

        let sh = shared.clone();
        let mut priming = true;
        let output_stream = output.build_output_stream(
            out_cfg,
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let t = info.timestamp();
                sh.playback_us.store(
                    t.playback.saturating_duration_since(t.callback).as_micros() as u32,
                    Relaxed,
                );
                if priming && out_rx.slots() < sh.prime.load(Relaxed) as usize {
                    data.fill(0.0);
                    return;
                }
                priming = false;
                for frame in data.chunks_exact_mut(out_ch) {
                    let s = out_rx.pop().unwrap_or_else(|_| {
                        priming = true;
                        0.0
                    });
                    frame.fill(s);
                }
                if priming {
                    sh.underruns.fetch_add(1, Relaxed);
                    let grown = (sh.prime.load(Relaxed) as usize + PRIME_STEP).min(PRIME_MAX);
                    sh.prime.store(grown as u32, Relaxed);
                }
            },
            stream_error(&shared),
            None,
        )?;

        // The model loads on the DSP thread (it isn't Send); wait for it before playing.
        let (ready_tx, ready_rx) = mpsc::channel();
        let (sh, model, strength, hum) = (shared.clone(), cfg.model, cfg.strength_db, cfg.hum_hz);
        let dsp = thread::Builder::new()
            .name("viumic-dsp".into())
            .spawn(move || match Denoiser::new(model, strength, hum) {
                Ok(mut den) => {
                    let _ = ready_tx.send(Ok(den.delay()));
                    dsp_loop(&sh, &mut den, mic_rx, out_tx);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
        let delay = ready_rx.recv().context("DSP thread died")??;

        input_stream.play()?;
        output_stream.play()?;
        Ok(Self {
            shared,
            delay,
            input_name,
            output_name,
            dsp: Some(dsp),
            _streams: (input_stream, output_stream),
        })
    }

    pub fn set_enabled(&self, on: bool) {
        self.shared.enabled.store(on, Relaxed);
    }

    pub fn set_strength(&self, db: f32) {
        self.shared.strength_db.set(db);
    }

    pub fn stats(&self) -> Stats {
        let s = &self.shared;
        let model = if s.enabled.load(Relaxed) {
            self.delay
        } else {
            0
        };
        let samples = s.buffered.load(Relaxed) as usize + model;
        let device_ms = (s.capture_us.load(Relaxed) + s.playback_us.load(Relaxed)) as f32 / 1000.0;
        Stats {
            in_peak: s.in_peak.take(),
            out_peak: s.out_peak.take(),
            latency_ms: device_ms + samples as f32 / MS as f32,
            device_ms,
            dropped: s.dropped.load(Relaxed),
            underruns: s.underruns.load(Relaxed),
            xruns: s.xruns.load(Relaxed),
            load: s.load.take(),
        }
    }

    /// Last error from a stream or the model, if any. Clears it.
    pub fn take_error(&self) -> Option<String> {
        self.shared.error.lock().ok()?.take()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shared.running.store(false, Relaxed);
        if let Some(dsp) = self.dsp.take() {
            let _ = dsp.join();
        }
    }
}

fn dsp_loop(sh: &Shared, den: &mut Denoiser, mut mic: Consumer<f32>, mut out: Producer<f32>) {
    boost_thread_priority();
    let (mut input, mut output) = ([0.0; HOP], [0.0; HOP]);
    let mut limiter = dynamics::Limiter::default();
    while sh.running.load(Relaxed) {
        let Ok(chunk) = mic.read_chunk(HOP) else {
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        let (a, b) = chunk.as_slices();
        input[..a.len()].copy_from_slice(a);
        input[a.len()..].copy_from_slice(b);
        chunk.commit_all();

        let started = std::time::Instant::now();
        // ponytail: bypass skips the model's delay, so toggling causes one tiny jump.
        if sh.enabled.load(Relaxed) {
            den.set_strength(sh.strength_db.get());
            if let Err(e) = den.process(&input, &mut output) {
                sh.fail(format!("model: {e}"));
                output = input;
            }
        } else {
            output = input;
        }
        limiter.apply(&mut output);
        sh.load
            .max(started.elapsed().as_secs_f32() / (HOP as f32 / SAMPLE_RATE as f32));
        sh.in_peak.max(peak(&input));
        sh.out_peak.max(peak(&output));

        let queued = out.buffer().capacity() - out.slots();
        sh.buffered.store((mic.slots() + queued) as u32, Relaxed);
        // ponytail: drop a hop on clock drift; switch to adaptive resampling if clicks are audible.
        if queued > sh.prime.load(Relaxed) as usize + DRIFT_MARGIN {
            sh.dropped.fetch_add(HOP as u32, Relaxed);
            continue;
        }
        if let Ok(chunk) = out.write_chunk_uninit(HOP) {
            chunk.fill_from_iter(output);
        }
    }
}

/// MMCSS "Pro Audio" keeps the model on schedule while games load the CPU.
#[cfg(windows)]
fn boost_thread_priority() {
    use windows::{Win32::System::Threading::AvSetMmThreadCharacteristicsW, core::w};
    let mut task = 0;
    // Best effort: without it we run at normal priority.
    let _ = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) };
}

#[cfg(not(windows))]
fn boost_thread_priority() {}

fn stream_error(sh: &Arc<Shared>) -> impl FnMut(cpal::Error) + Send + 'static {
    let sh = sh.clone();
    move |e| match e.kind() {
        cpal::ErrorKind::Xrun => {
            sh.xruns.fetch_add(1, Relaxed);
        }
        _ => sh.fail(format!("audio device: {e}")),
    }
}

/// Records `secs` of raw, unprocessed mic audio (interleaved) for diagnosing a mic.
/// Returns the samples, channel count and sample rate.
pub fn record(input: Option<&str>, secs: f32) -> Result<(Vec<f32>, u16, u32)> {
    let dev = input_device(&cpal::default_host(), input)?;
    let cfg = dev.default_input_config()?;
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        bail!(
            "mic uses {:?} samples; only f32 is supported",
            cfg.sample_format()
        );
    }
    let (ch, rate) = (cfg.channels(), cfg.sample_rate());
    let n = (secs * rate as f32) as usize * ch as usize;
    let (mut tx, mut rx) = RingBuffer::new(n);
    let stream = dev.build_input_stream(
        cfg.config(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for s in data {
                let _ = tx.push(*s);
            }
        },
        |e| eprintln!("audio device: {e}"),
        None,
    )?;
    stream.play()?;
    thread::sleep(Duration::from_secs_f32(secs));
    drop(stream);
    Ok((std::iter::from_fn(|| rx.pop().ok()).collect(), ch, rate))
}

fn input_device(host: &cpal::Host, query: Option<&str>) -> Result<cpal::Device> {
    match query {
        Some(q) => find(host.input_devices()?, q).with_context(|| format!("mic \"{q}\" not found")),
        None => host.default_input_device().context("no default microphone"),
    }
}

fn name(d: &cpal::Device) -> Option<String> {
    Some(d.description().ok()?.name().to_string())
}

/// Exact name match first, then case-insensitive substring.
fn find(devices: impl Iterator<Item = cpal::Device>, query: &str) -> Option<cpal::Device> {
    let mut named: Vec<_> = devices.filter_map(|d| Some((name(&d)?, d))).collect();
    let q = query.to_lowercase();
    let i = named.iter().position(|(n, _)| n == query).or_else(|| {
        named
            .iter()
            .position(|(n, _)| n.to_lowercase().contains(&q))
    })?;
    Some(named.swap_remove(i).1)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

/// f32 stored as bits. For non-negative floats, bit order equals numeric order,
/// so `fetch_max` on the bits is a lock-free float max.
#[derive(Default)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }
    fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed);
    }
    fn max(&self, v: f32) {
        self.0.fetch_max(v.to_bits(), Relaxed);
    }
    fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Relaxed))
    }
}
