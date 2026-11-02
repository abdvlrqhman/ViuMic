//! Level control after the denoiser:
//! - Expander (RNNoise path): below a level threshold, quiet gets quieter, so the noise
//!   RNNoise leaves in pauses sinks while normal speech is untouched. Tuned by ear.
//! - Limiter (always, last): no sample sent to the cable ever exceeds the ceiling.

use crate::{HOP, SAMPLE_RATE};

/// Output ceiling, -1 dBFS: the cable and the apps behind it must never clip.
const CEILING: f32 = 0.891;
/// Limiter release time constant: gain recovers smoothly instead of pumping.
const LIMIT_RELEASE_S: f32 = 0.05;

/// Peak limiter with instant attack (no lookahead, no added delay) and smooth release.
pub(crate) struct Limiter {
    env: f32,
    release: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        Self {
            env: 0.0,
            release: (-1.0 / (LIMIT_RELEASE_S * SAMPLE_RATE as f32)).exp(),
        }
    }
}

impl Limiter {
    pub(crate) fn apply(&mut self, x: &mut [f32]) {
        for s in x {
            // A new peak sets the envelope to itself, so that sample lands exactly on the ceiling.
            self.env = s.abs().max(self.env * self.release);
            if self.env > CEILING {
                *s *= CEILING / self.env;
            }
        }
    }
}

/// Below this output level (dBFS), the signal is pushed down...
const EXPAND_BELOW_DB: f32 = -45.0;
/// ...by this ratio: 1 dB under the threshold becomes 2 dB under.
const EXPAND_RATIO: f32 = 2.0;
/// Level detector release (dB per hop), so word tails don't trigger it mid-word.
const LEVEL_RELEASE_DB: f32 = 1.0;
/// Most it may take off.
const MAX_CUT_DB: f32 = 50.0;
/// Samples over which gain rises (1 ms). Falls ramp across the whole hop.
const ATTACK: usize = 48;

pub(crate) struct Expander {
    level_db: f32,
    gain: f32,
}

impl Default for Expander {
    fn default() -> Self {
        Self {
            level_db: -100.0,
            gain: 1.0,
        }
    }
}

impl Expander {
    pub(crate) fn apply(&mut self, hop: &mut [f32; HOP]) {
        let power = hop.iter().map(|s| s * s).sum::<f32>() / HOP as f32;
        let level = 10.0 * power.max(1e-12).log10();
        self.level_db = level.max(self.level_db - LEVEL_RELEASE_DB);
        let cut_db = (self.level_db - EXPAND_BELOW_DB).min(0.0) * (EXPAND_RATIO - 1.0);
        let gain = 10f32.powf(cut_db.max(-MAX_CUT_DB) / 20.0);
        // Ramp from the old gain: fast when rising, across the hop when falling, so gain
        // changes never click.
        let (start, len) = (self.gain, if gain > self.gain { ATTACK } else { HOP });
        for (i, s) in hop.iter_mut().enumerate() {
            *s *= start + (gain - start) * ((i + 1) as f32 / len as f32).min(1.0);
        }
        self.gain = gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_holds_ceiling_and_leaves_normal_levels() {
        let sine = |amp: f32| -> Vec<f32> {
            (0..SAMPLE_RATE as usize)
                .map(|i| amp * (i as f32 * 0.05).sin())
                .collect()
        };
        let mut loud = sine(2.0);
        Limiter::default().apply(&mut loud);
        let peak = loud.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak <= CEILING + 1e-6, "peak {peak}");

        let quiet = sine(0.5);
        let mut out = quiet.clone();
        Limiter::default().apply(&mut out);
        assert_eq!(out, quiet);
    }
}
