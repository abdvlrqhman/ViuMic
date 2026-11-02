//! Neural denoisers. Both run on 48 kHz mono audio in 480-sample (10 ms) hops.
//!
//! Chain: model → strength mix-back → 80 Hz high-pass + hum notches → (RNNoise only) expander.

use crate::dpdfnet::{self, DpdfNet};
use crate::dynamics::Expander;
use anyhow::Result;
use nnnoiseless::DenoiseState;
use std::collections::VecDeque;

pub const SAMPLE_RATE: u32 = 48_000;
pub const HOP: usize = 480;
/// Strength at or above this means "remove as much noise as possible".
pub const UNLIMITED_DB: f32 = 100.0;

// Tuned by ear on a real laptop mic.
/// Rumble below this is cut; voices start around 85 Hz.
const HIGHPASS_HZ: f64 = 80.0;
/// Mains hum: notch the fundamental and this many harmonics in total (50/100/150/200 Hz).
/// A real laptop mic had a steady 50 Hz hum (+25 dB line) that models only partly remove.
const HUM_HARMONICS: u32 = 4;
/// Notch width. Narrow enough that voice harmonics only graze it as pitch moves.
const HUM_WIDTH_HZ: f64 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Model {
    /// DPDFNet-2 (48 kHz): best quality; rated "perfect" on a noisy laptop mic where
    /// RNNoise and DeepFilterNet3 left electrical noise under the voice.
    #[default]
    DpdfNet,
    /// RNNoise: very light on CPU, for weak machines.
    RNNoise,
}

pub struct Denoiser {
    kind: Kind,
    strength_db: f32,
    /// Input delayed to line up with the model output, for the strength mix-back.
    dry: VecDeque<f32>,
    filters: Filters,
    expander: Expander,
}

enum Kind {
    Dpdf(Box<DpdfNet>),
    Rnn(Box<DenoiseState<'static>>),
}

impl Denoiser {
    /// Loads the model. `hum_hz`: mains frequency to notch out (50 or 60), or `None`.
    pub fn new(model: Model, strength_db: f32, hum_hz: Option<f32>) -> Result<Self> {
        let kind = match model {
            Model::DpdfNet => Kind::Dpdf(Box::new(DpdfNet::new()?)),
            Model::RNNoise => Kind::Rnn(DenoiseState::new()),
        };
        let mut d = Self {
            kind,
            strength_db,
            dry: VecDeque::new(),
            filters: Filters::new(hum_hz),
            expander: Expander::default(),
        };
        d.dry = VecDeque::from(vec![0.0; d.delay()]);
        Ok(d)
    }

    /// Maximum noise attenuation in dB. At or above [`UNLIMITED_DB`] there is no limit.
    pub fn set_strength(&mut self, db: f32) {
        self.strength_db = db;
    }

    /// Samples by which the output lags the input.
    pub fn delay(&self) -> usize {
        match &self.kind {
            Kind::Dpdf(_) => dpdfnet::DELAY,
            Kind::Rnn(_) => HOP,
        }
    }

    /// Denoises one hop. Runs on the DSP thread (not the audio callbacks); ONNX Runtime
    /// allocates small output tensors per call.
    pub fn process(&mut self, input: &[f32; HOP], output: &mut [f32; HOP]) -> Result<()> {
        match &mut self.kind {
            Kind::Dpdf(model) => model.process(input, output)?,
            Kind::Rnn(state) => {
                // nnnoiseless works on i16-scaled floats.
                state.process_frame(output, &input.map(|s| s * 32768.0));
                output.iter_mut().for_each(|s| *s /= 32768.0);
            }
        }

        self.dry.extend(input);
        let dry = dry_gain(self.strength_db);
        for (o, d) in output.iter_mut().zip(self.dry.drain(..HOP)) {
            *o = *o * (1.0 - dry) + d * dry;
        }

        self.filters.run(output);
        // RNNoise leaves noise in pauses that DPDFNet doesn't; push it down, at max only (a
        // strength limit asks for some noise floor).
        if matches!(self.kind, Kind::Rnn(_)) && self.strength_db >= UNLIMITED_DB {
            self.expander.apply(output);
        }
        Ok(())
    }
}

/// Fixed clean-up the models can't do: they let rumble and mains hum through while you talk
/// (it comes and goes with your voice). 4th-order Butterworth high-pass + hum notches.
struct Filters(Vec<Biquad>);

impl Filters {
    fn new(hum_hz: Option<f32>) -> Self {
        let mut f = vec![Biquad::highpass(0.5412), Biquad::highpass(1.3066)];
        if let Some(hz) = hum_hz {
            f.extend((1..=HUM_HARMONICS).map(|k| Biquad::notch(hz as f64 * k as f64)));
        }
        Self(f)
    }

    fn run(&mut self, x: &mut [f32]) {
        for s in x {
            *s = self.0.iter_mut().fold(*s, |v, b| b.run(v));
        }
    }
}

struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    /// RBJ audio-EQ-cookbook high-pass at [`HIGHPASS_HZ`] with quality `q`.
    fn highpass(q: f64) -> Self {
        let w = std::f64::consts::TAU * HIGHPASS_HZ / SAMPLE_RATE as f64;
        let (cos, alpha) = (w.cos(), w.sin() / (2.0 * q));
        let a0 = 1.0 + alpha;
        let b0 = (1.0 + cos) / 2.0 / a0;
        Self {
            b: [b0, -2.0 * b0, b0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        }
    }

    /// RBJ notch at `hz`, [`HUM_WIDTH_HZ`] wide.
    fn notch(hz: f64) -> Self {
        let w = std::f64::consts::TAU * hz / SAMPLE_RATE as f64;
        let (cos, alpha) = (w.cos(), w.sin() / (2.0 * hz / HUM_WIDTH_HZ));
        let a0 = 1.0 + alpha;
        Self {
            b: [1.0 / a0, -2.0 * cos / a0, 1.0 / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            z: [0.0; 2],
        }
    }

    /// Transposed direct form II, in f64: 80 Hz at 48 kHz is too low for f32 to stay clean.
    fn run(&mut self, x: f32) -> f32 {
        let x = x as f64;
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y as f32
    }
}

/// Linear gain of the untouched signal that keeps attenuation at most `db`.
fn dry_gain(db: f32) -> f32 {
    if db >= UNLIMITED_DB {
        0.0
    } else {
        10f32.powf(-db.abs() / 20.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_buffer;

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|s| (*s as f64).powi(2)).sum()
    }

    #[test]
    fn noise_is_suppressed_and_length_kept() {
        // Low-passed white noise (~750 Hz corner): inside the bands both models process.
        let (mut rng, mut lp) = (0x9E37_79B9u32, 0f32);
        let noise: Vec<f32> = (0..SAMPLE_RATE as usize * 3)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                lp += 0.1 * ((rng as f32 / u32::MAX as f32 - 0.5) - lp);
                lp
            })
            .collect();
        let warmup = SAMPLE_RATE as usize; // let the models settle for 1 s

        let removed_db = |model, strength| {
            let mut d = Denoiser::new(model, strength, Some(50.0)).unwrap();
            let out = process_buffer(&noise, d.delay(), |i, o| d.process(i, o)).unwrap();
            assert_eq!(out.len(), noise.len(), "{model:?}");
            -10.0 * (energy(&out[warmup..]) / energy(&noise[warmup..])).log10()
        };
        for model in [Model::DpdfNet, Model::RNNoise] {
            let full = removed_db(model, UNLIMITED_DB);
            assert!(full > 15.0, "{model:?} only removed {full:.1} dB");
            // The strength limit must hold (and proves the output isn't just silence).
            let limited = removed_db(model, 6.0);
            assert!(
                (4.0..8.0).contains(&limited),
                "{model:?} at 6 dB removed {limited:.1} dB"
            );
        }
    }

    #[test]
    fn filters_cut_rumble_and_hum_keep_voice() {
        let gain_db = |hz: f32| {
            let mut x: Vec<f32> = (0..SAMPLE_RATE as usize * 2)
                .map(|i| (std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE as f32).sin())
                .collect();
            let before = energy(&x[SAMPLE_RATE as usize..]);
            Filters::new(Some(50.0)).run(&mut x);
            10.0 * (energy(&x[SAMPLE_RATE as usize..]) / before).log10()
        };
        for (hz, lo, hi) in [
            (40.0, f32::MIN, -20.0),  // rumble
            (100.0, f32::MIN, -30.0), // hum harmonics
            (150.0, f32::MIN, -30.0),
            (125.0, -1.0, 1.0), // voice between harmonics
            (300.0, -0.5, 0.5),
            (1000.0, -0.5, 0.5),
        ] {
            let g = gain_db(hz);
            assert!((lo..=hi).contains(&(g as f32)), "{hz} Hz: {g:.1} dB");
        }
    }
}
