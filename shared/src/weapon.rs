//! Weapon tuning. These numbers are deliberately in one place so the client's
//! feel and the server's hit validation can never disagree.

use serde::{Deserialize, Serialize};

/// The weapons a trickshot can be taken with. Add variants here and give them a
/// [`WeaponSpec`] in [`WeaponId::spec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeaponId {
    Sniper,
    Marksman,
}

/// Ballistic + damage parameters for one weapon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponSpec {
    /// Shots past this range (metres) always miss.
    pub max_range: f32,
    /// Muzzle speed, m/s. `f32::INFINITY` means instant hitscan (no travel time).
    pub muzzle_velocity: f32,
    /// Downward acceleration on the projectile, m/s². `0.0` means no bullet drop.
    pub gravity: f32,
    /// Body-shot damage at point-blank, before range falloff and the hit-zone
    /// multipliers.
    pub base_damage: f32,
    /// Range (metres) out to which a shot still does full `base_damage`; past
    /// it damage falls off linearly, reaching `min_damage_fraction` of it at
    /// `max_range`. `0.0` starts the falloff at the muzzle.
    pub falloff_start: f32,
    /// Multiplier applied on a head hit.
    pub headshot_multiplier: f32,
    /// Fraction of `base_damage` still dealt at `max_range` (linear falloff
    /// from `falloff_start`). `1.0` disables falloff.
    pub min_damage_fraction: f32,
    /// Multiplier on a hit to the lower body (waist down — see
    /// [`crate::ballistics::HitZone::Legs`]). Below `1.0`, so a leg shot from
    /// a weapon that one-shots the torso leaves the target alive, Call of Duty
    /// style.
    pub lower_body_multiplier: f32,
}

impl WeaponId {
    pub const fn spec(self) -> WeaponSpec {
        match self {
            WeaponId::Sniper => WeaponSpec {
                // A modern CoD sniper (this is an AX-50-style bolt action) is
                // hitscan with no felt "out of range" — its maps are always
                // smaller than the rifle's reach, so a shot only ever misses
                // from bad aim. 300 m clears this map's full ground-plane
                // diagonal (~283 m) with room to spare, so it reads the same
                // way in practice while still being a real, tunable cap
                // instead of the arbitrary 600 m this used to be.
                max_range: 300.0,
                muzzle_velocity: f32::INFINITY,
                gravity: 0.0,
                // Damage is tuned against a 100-health bar so *where* and *how
                // far* a shot lands decide whether it kills, Call of Duty
                // style. Peak 200, full out to `falloff_start`, then a linear
                // fade to 10% at `max_range`; with the zone multipliers below:
                //   * a leg shot (×0.6) kills out to ~72 m, but not beyond;
                //   * a torso shot (×1) kills out to ~175 m;
                //   * a headshot (×2) kills out to ~253 m — past that even a
                //     headshot leaves the target alive.
                // (`ballistics`' tests pin these thresholds.)
                base_damage: 200.0,
                falloff_start: 20.0,
                headshot_multiplier: 2.0,
                min_damage_fraction: 0.1,
                lower_body_multiplier: 0.6,
            },
            WeaponId::Marksman => WeaponSpec {
                max_range: 350.0,
                muzzle_velocity: 900.0,
                gravity: 9.81,
                base_damage: 65.0,
                falloff_start: 0.0,
                headshot_multiplier: 2.0,
                min_damage_fraction: 0.55,
                lower_body_multiplier: 0.6,
            },
        }
    }

    /// Wire representation used in [`crate::protocol::PlayerInput::weapon`].
    pub const fn as_u8(self) -> u8 {
        match self {
            WeaponId::Sniper => 0,
            WeaponId::Marksman => 1,
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(WeaponId::Sniper),
            1 => Some(WeaponId::Marksman),
            _ => None,
        }
    }

    /// True when the weapon has no travel time and no drop, so the server can
    /// resolve it with a single ray instead of stepping a trajectory.
    pub fn is_hitscan(self) -> bool {
        let s = self.spec();
        s.muzzle_velocity.is_infinite() && s.gravity == 0.0
    }
}
