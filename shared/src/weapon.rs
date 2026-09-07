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
    /// Body-shot damage at point-blank, before range falloff.
    pub base_damage: f32,
    /// Multiplier applied on a head hit.
    pub headshot_multiplier: f32,
    /// Fraction of `base_damage` still dealt at `max_range` (linear falloff in
    /// between). `1.0` disables falloff.
    pub min_damage_fraction: f32,
}

impl WeaponId {
    pub const fn spec(self) -> WeaponSpec {
        match self {
            WeaponId::Sniper => WeaponSpec {
                max_range: 600.0,
                muzzle_velocity: f32::INFINITY,
                gravity: 0.0,
                base_damage: 100.0,
                headshot_multiplier: 2.0,
                min_damage_fraction: 1.0,
            },
            WeaponId::Marksman => WeaponSpec {
                max_range: 350.0,
                muzzle_velocity: 900.0,
                gravity: 9.81,
                base_damage: 65.0,
                headshot_multiplier: 2.0,
                min_damage_fraction: 0.55,
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
