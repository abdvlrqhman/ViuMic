//! DPDFNet (Ceva, Apache-2.0): causal 48 kHz speech enhancement, run with ONNX Runtime.
//!
//! A port of the `dpdfnet` Python package's `StreamEnhancer`: 960-sample Vorbis-windowed
//! frames, 480-sample hop, the model's recurrent state carried from frame to frame, and
//! overlap-add synthesis. Matches its output to float precision.

use crate::HOP;
use anyhow::{Context, Result, anyhow};
use ort::environment::Environment;
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::TensorRef;
use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::sync::{Arc, LazyLock};

const WIN: usize = 2 * HOP;
const BINS: usize = WIN / 2 + 1;
/// Output lags input by 5 hops: 4 of model lookahead + 1 of framing (measured on real speech).
pub(crate) const DELAY: usize = 5 * HOP;

/// `dpdfnet2_48khz_hr.onnx`; see models/README.md for source and checksum.
static MODEL: &[u8] = include_bytes!("../../../models/dpdfnet2_48khz_hr.onnx");

/// ONNX Runtime allows one environment per process; every model (and engine restart) shares it.
static ENV: LazyLock<Result<Environment, String>> = LazyLock::new(|| {
    ort::init()
        .with_name("viumic")
        .with_telemetry(false)
        .build()
        .map_err(|e| e.to_string())
});

pub(crate) struct DpdfNet {
    session: Session,
    state: Vec<f32>,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    /// The last two input hops (one analysis frame).
    frame: Vec<f32>,
    /// Second half of the previous synthesis frame, waiting to be overlap-added.
    overlap: Vec<f32>,
    /// The first hop only fills the frame; the model runs once two hops are in (as upstream).
    primed: bool,
    // Scratch, allocated once.
    time: Vec<f32>,
    spec: Vec<Complex<f32>>,
    spec_ri: Vec<f32>,
}

impl DpdfNet {
    pub(crate) fn new() -> Result<Self> {
        let env = ENV
            .as_ref()
            .map_err(|e| anyhow!("starting ONNX Runtime: {e}"))?;
        let builder_err = |e: ort::Error<_>| anyhow!("configuring DPDFNet: {e}");
        let mut builder = Session::builder(env)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(builder_err)?
            // One thread, like upstream: a thread pool would fight the audio callbacks.
            .with_intra_threads(1)
            .map_err(builder_err)?
            .with_inter_threads(1)
            .map_err(builder_err)?;
        let session = builder.commit_from_memory(MODEL)?;
        let state = initial_state(&session)?;
        let mut fft = RealFftPlanner::<f32>::new();
        Ok(Self {
            session,
            state,
            window: (0..WIN).map(vorbis).collect(),
            fft: fft.plan_fft_forward(WIN),
            ifft: fft.plan_fft_inverse(WIN),
            frame: vec![0.0; WIN],
            overlap: vec![0.0; HOP],
            primed: false,
            time: vec![0.0; WIN],
            spec: vec![Complex::default(); BINS],
            spec_ri: vec![0.0; 2 * BINS],
        })
    }

    pub(crate) fn process(&mut self, input: &[f32; HOP], output: &mut [f32; HOP]) -> Result<()> {
        self.frame.copy_within(HOP.., 0);
        self.frame[HOP..].copy_from_slice(input);
        if !self.primed {
            self.primed = true;
            output.fill(0.0);
            return Ok(());
        }

        // Analysis: windowed real FFT, packed as [bin][re, im].
        for ((t, x), w) in self.time.iter_mut().zip(&self.frame).zip(&self.window) {
            *t = x * w;
        }
        self.fft.process(&mut self.time, &mut self.spec)?;
        for (ri, c) in self.spec_ri.chunks_exact_mut(2).zip(&self.spec) {
            ri.copy_from_slice(&[c.re, c.im]);
        }

        let outputs = self.session.run(ort::inputs![
            "spec" => TensorRef::from_array_view(([1usize, 1, BINS, 2], &self.spec_ri[..]))?,
            "state_in" => TensorRef::from_array_view(([self.state.len()], &self.state[..]))?,
        ])?;
        let (_, enhanced) = outputs["spec_e"].try_extract_tensor::<f32>()?;
        for (c, ri) in self.spec.iter_mut().zip(enhanced.chunks_exact(2)) {
            *c = Complex::new(ri[0], ri[1]);
        }
        let (_, state) = outputs["state_out"].try_extract_tensor::<f32>()?;
        self.state.copy_from_slice(state);
        drop(outputs);

        // Synthesis: numpy's irfft ignores the imaginary part of DC and Nyquist and divides
        // by N; realfft does neither.
        self.spec[0].im = 0.0;
        self.spec[BINS - 1].im = 0.0;
        self.ifft.process(&mut self.spec, &mut self.time)?;
        // Window again, then overlap-add: emit old tail + new head, keep the new tail.
        let scale = 1.0 / WIN as f32;
        for (t, w) in self.time.iter_mut().zip(&self.window) {
            *t *= w * scale;
        }
        let (head, tail) = self.time.split_at(HOP);
        for ((o, prev), h) in output.iter_mut().zip(&self.overlap).zip(head) {
            *o = prev + h;
        }
        self.overlap.copy_from_slice(tail);
        Ok(())
    }
}

/// Vorbis window, as upstream: sin(pi/2 * sin^2(pi * (n + 0.5) / WIN)).
fn vorbis(n: usize) -> f32 {
    use std::f64::consts::FRAC_PI_2;
    let s = (FRAC_PI_2 * (n as f64 + 0.5) / HOP as f64).sin();
    (FRAC_PI_2 * s * s).sin() as f32
}

/// Zeros, except the ERB and spectral normalisation states, whose start values live in the
/// model's metadata.
fn initial_state(session: &Session) -> Result<Vec<f32>> {
    let meta = session.metadata()?;
    let get = |key: &str| {
        meta.custom(key)
            .with_context(|| format!("model metadata lacks {key}"))
    };
    let floats = |key: &str| -> Result<Vec<f32>> {
        get(key)?
            .split(',')
            .map(|v| v.trim().parse().with_context(|| format!("bad {key}")))
            .collect()
    };
    let mut state = vec![0.0; get("state_size")?.parse()?];
    let erb = floats("erb_norm_init")?;
    let spec = floats("spec_norm_init")?;
    state[..erb.len()].copy_from_slice(&erb);
    state[erb.len()..erb.len() + spec.len()].copy_from_slice(&spec);
    Ok(state)
}
