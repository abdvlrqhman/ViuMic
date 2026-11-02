//! Downward expander for the RNNoise path: below a level threshold, quiet gets quieter, so
//! the noise RNNoise leaves in pauses sinks while normal speech is untouched. Tuned by ear.

use crate::HOP;

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
