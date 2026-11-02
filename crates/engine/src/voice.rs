//! Optional "deeper voice": pitch shift down, applied after denoising.

use crate::HOP;
use signalsmith_stretch::Stretch;

/// Largest supported shift, in semitones.
pub const MAX_DEPTH: f32 = 12.0;
/// How far formants follow the pitch (0 = keep them, 1 = move with it). Halfway sounds like a
/// deeper voice; keeping them sounds like the same throat singing lower, moving them all the
/// way sounds like a monster. Tune by ear.
const FORMANT_FOLLOW: f32 = 0.5;

pub struct Deepen {
    stretch: Stretch,
    depth: f32,
}

impl Default for Deepen {
    fn default() -> Self {
        // 40 ms blocks, 10 ms interval: the default preset uses 120 ms blocks, which is too
        // much latency for live calls.
        Self {
            stretch: Stretch::new(1, 4 * HOP, HOP),
            depth: 0.0,
        }
    }
}

impl Deepen {
    /// Samples by which the output lags the input while active.
    pub fn delay(&self) -> usize {
        self.stretch.input_latency() + self.stretch.output_latency()
    }

    /// Lowers the pitch by `depth` semitones (clamped to 0..=[`MAX_DEPTH`]); 0 is a passthrough.
    pub fn process(&mut self, depth: f32, input: &[f32; HOP], output: &mut [f32; HOP]) {
        let depth = depth.clamp(0.0, MAX_DEPTH);
        if depth == 0.0 {
            self.depth = 0.0;
            *output = *input;
            return;
        }
        if depth != self.depth {
            if self.depth == 0.0 {
                self.stretch.reset(); // drop stale audio from the last time it was on
            }
            // Tonality limit at 8 kHz: Signalsmith's recommendation for keeping speech timbre.
            let tonality = 8000.0 / crate::SAMPLE_RATE as f32;
            self.stretch
                .set_transpose_factor_semitones(-depth, Some(tonality));
            self.stretch
                .set_formant_factor_semitones(-depth * FORMANT_FOLLOW, true);
            self.depth = depth;
        }
        self.stretch.process(input, output);
    }
}
