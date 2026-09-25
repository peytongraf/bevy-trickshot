//! A perk screen effect's "kick-in": right after it switches on (the perk's
//! bought, or the debug toggle ticked), the effect climbs well past its tuned
//! strength, holds there a few seconds, then eases back down to that baseline
//! — which is how it's played from then on. Shared by the shroom and drunk
//! passes; it only scales their screen strength, never gameplay.

/// Panel-tunable shape of the kick.
#[derive(Clone)]
pub(crate) struct KickSettings {
    /// Strength at the top of the kick, as a multiple of the baseline.
    pub(crate) peak: f32,
    /// Seconds to climb to the peak...
    pub(crate) rise_secs: f32,
    /// ...to hold it...
    pub(crate) hold_secs: f32,
    /// ...and to ease back to the baseline.
    pub(crate) fall_secs: f32,
}

impl Default for KickSettings {
    fn default() -> Self {
        Self {
            peak: 3.0,
            rise_secs: 3.0,
            hold_secs: 3.0,
            fall_secs: 3.0,
        }
    }
}

impl KickSettings {
    fn total(&self) -> f32 {
        self.rise_secs + self.hold_secs + self.fall_secs
    }

    /// The strength multiplier for a kick `boost` (see [`KickState::update`]):
    /// 1 at the baseline, `peak` at the top.
    pub(crate) fn multiplier(&self, boost: f32) -> f32 {
        1.0 + (self.peak - 1.0) * boost
    }

    /// How far into the kick `t` seconds in is: 0 → 1 up the ramp, 1 while
    /// holding, 1 → 0 back down.
    fn boost(&self, t: f32) -> f32 {
        let ease = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        if t < self.rise_secs {
            ease(t / self.rise_secs.max(1e-3))
        } else if t < self.rise_secs + self.hold_secs {
            1.0
        } else {
            1.0 - ease((t - self.rise_secs - self.hold_secs) / self.fall_secs.max(1e-3))
        }
    }
}

/// Where a kick is up to. Cleared whenever the effect is off, so the next
/// time it switches on (a new purchase, a new game) kicks in afresh.
#[derive(Default)]
pub(crate) struct KickState {
    /// Seconds into the running kick, if one is.
    elapsed: Option<f32>,
    was_on: bool,
}

impl KickState {
    /// Advance by `dt` with the effect `on`, returning this frame's kick
    /// boost, 0..=1 (0 = settled at the baseline) — turn it into a strength
    /// multiplier with [`KickSettings::multiplier`].
    pub(crate) fn update(&mut self, on: bool, dt: f32, settings: &KickSettings) -> f32 {
        if on && !self.was_on {
            self.elapsed = Some(0.0);
        }
        if !on {
            self.elapsed = None;
        }
        self.was_on = on;
        let Some(t) = &mut self.elapsed else {
            return 0.0;
        };
        *t += dt;
        if *t >= settings.total() {
            self.elapsed = None;
            return 0.0;
        }
        settings.boost(*t)
    }

    /// Debug: run the kick again (only while the effect is on).
    pub(crate) fn replay(&mut self) {
        if self.was_on {
            self.elapsed = Some(0.0);
        }
    }

    #[cfg(test)]
    fn running(&self) -> bool {
        self.elapsed.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn climbs_to_the_peak_holds_then_settles_back_to_one() {
        let s = KickSettings::default();
        let mut k = KickState::default();
        assert_eq!(k.update(false, 0.1, &s), 0.0);
        let start = k.update(true, 0.0, &s);
        assert!(start.abs() < 1e-4, "starts at the baseline");
        let top = k.update(true, 4.0, &s);
        assert!((s.multiplier(top) - s.peak).abs() < 1e-4, "held at the peak");
        let settled = k.update(true, 10.0, &s);
        assert_eq!(s.multiplier(settled), 1.0, "back to the baseline for good");
        assert!(!k.running());
        // Off and on again (a new game) kicks again.
        k.update(false, 0.1, &s);
        k.update(true, 0.0, &s);
        assert!(k.running());
    }
}
