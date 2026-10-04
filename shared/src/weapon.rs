//! Weapon tuning. These numbers are deliberately in one place so the client's
//! feel and the server's hit validation can never disagree.

use serde::{Deserialize, Serialize};

/// The weapons a trickshot can be taken with. Add variants here and give them a
/// [`WeaponSpec`] in [`WeaponId::spec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WeaponId {
    #[default]
    Sniper,
    Marksman,
    /// The AK-74 assault rifle — full-auto, picked in the loadout
    /// ([`crate::LobbyMember::loadout`]) for `Zombies` or `FreeForAll`.
    Ak74,
    /// The Ray Gun — `Zombies`' wonder weapon, Call of Duty's: semi-auto,
    /// each shot a green bolt that bursts where it lands, hurting every
    /// zombie close by ([`raygun_splash_damage`]).
    RayGun,
}

/// How far (m) from where a Ray Gun bolt lands its burst reaches...
pub const RAYGUN_SPLASH_RADIUS: f32 = 2.5;
/// ...and the burst's damage at its middle, fading to...
pub const RAYGUN_SPLASH_DAMAGE_MAX: f32 = 180.0;
/// ...this at its edge.
pub const RAYGUN_SPLASH_DAMAGE_MIN: f32 = 50.0;

/// A Ray Gun burst's damage to something `distance` m from where it landed
/// (before Pack-a-Punch) — none past [`RAYGUN_SPLASH_RADIUS`].
pub fn raygun_splash_damage(distance: f32) -> f32 {
    if distance > RAYGUN_SPLASH_RADIUS {
        return 0.0;
    }
    let t = (distance / RAYGUN_SPLASH_RADIUS).clamp(0.0, 1.0);
    RAYGUN_SPLASH_DAMAGE_MAX + (RAYGUN_SPLASH_DAMAGE_MIN - RAYGUN_SPLASH_DAMAGE_MAX) * t
}

/// The primary weapons a player can pick in the loadout, for the modes that
/// have one ([`crate::GameMode::has_loadout`]; `Freestyle` is always the
/// sniper).
pub const LOADOUT_WEAPONS: [WeaponId; 2] = [WeaponId::Sniper, WeaponId::Ak74];

/// What fills one of a player's two weapon slots: a gun, or the knife. A
/// player carries two different ones — a `Zombies` wall buy or a dropped
/// weapon picked up takes the place of the one in their hands
/// ([`crate::LobbyMember::weapons`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SlotWeapon {
    Gun(WeaponId),
    Knife,
}

impl SlotWeapon {
    /// What a player starts a life with: `primary`, and the knife.
    pub const fn starting(primary: WeaponId) -> [SlotWeapon; 2] {
        [SlotWeapon::Gun(primary), SlotWeapon::Knife]
    }

    pub const fn label(self) -> &'static str {
        match self {
            SlotWeapon::Gun(gun) => gun.label(),
            SlotWeapon::Knife => "KNIFE",
        }
    }

    pub const fn gun(self) -> Option<WeaponId> {
        match self {
            SlotWeapon::Gun(gun) => Some(gun),
            SlotWeapon::Knife => None,
        }
    }
}

/// The guns a `Zombies` wall buy sells ([`crate::level::WallBuy`]).
pub const WALL_BUY_WEAPONS: [WeaponId; 2] = [WeaponId::Sniper, WeaponId::Ak74];

/// What a `Zombies` wall buy charges for `gun`.
pub const fn wall_buy_cost(gun: WeaponId) -> u32 {
    match gun {
        WeaponId::Ak74 => 1000,
        _ => 1500,
    }
}

/// `FreeForAll`, Call of Duty style: a loadout change takes effect on the
/// spot if the player spawned at most this many seconds ago and hasn't fired
/// since; otherwise it waits for their next spawn.
pub const LOADOUT_SWAP_GRACE_SECS: f32 = 10.0;

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

/// The sniper's first-person feel, as the client plays it — timings and shake
/// numbers a real player's client records into every kill-cam frame
/// (`PlayerInput`). A bot has no client, so `server::ai` runs the same numbers
/// to fill those fields in, and its kill cam then reads like a human's: a gradual
/// scope-in, the fire animation, recoil and shake. **These mirror the client's
/// `weapons::ads::ADS_DURATION`, `weapons::recoil::SHAKE_*` and
/// `weapons::view_model::SEGMENTS` / `AnimationSettings::rechamber_speed`** —
/// keep them in step if those are retuned.
pub mod sniper_feel {
    /// Seconds from hip to full aim-down-sight (and back), linear.
    pub const ADS_SECS: f32 = 0.4;
    /// The `Shoot` segment of the sniper clip ends this far (s) into it...
    pub const SHOOT_END_SECS: f32 = 9.0 / 24.0;
    /// ...then `Rechamber` runs from there to here (clip seconds)...
    pub const RECHAMBER_END_SECS: f32 = 48.0 / 24.0;
    /// ...at this many times normal speed.
    pub const RECHAMBER_SPEED: f32 = 1.7;
    /// Camera-shake trauma one shot adds (capped at 1), and how much decays per second.
    pub const SHAKE_ADD: f32 = 0.85;
    pub const SHAKE_DECAY: f32 = 2.0;
    /// Oscillation speed of the shake `phase` (per second, while there's trauma).
    pub const SHAKE_FREQ: f32 = 75.0;
    /// Backward camera kick per shot (m), and how fast it eases home (1/s).
    pub const RECOIL_KICK: f32 = 0.05;
    pub const RECOIL_RETURN: f32 = 5.0;
    /// The sniper clip's playhead (seconds) `since` seconds after a shot:
    /// `Shoot` at normal speed, then `Rechamber` (sped up), then back at rest.
    pub fn anim_time(since: f32) -> f32 {
        if since < 0.0 {
            0.0
        } else if since < SHOOT_END_SECS {
            since
        } else {
            let t = SHOOT_END_SECS + (since - SHOOT_END_SECS) * RECHAMBER_SPEED;
            if t < RECHAMBER_END_SECS {
                t
            } else {
                0.0
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_fire_animation_plays_shoot_then_rechamber_then_rests() {
            assert_eq!(anim_time(-1.0), 0.0);
            assert!((anim_time(0.2) - 0.2).abs() < 1e-6);
            assert!(anim_time(SHOOT_END_SECS + 0.1) > SHOOT_END_SECS);
            assert!(anim_time(SHOOT_END_SECS + 0.1) < RECHAMBER_END_SECS);
            assert_eq!(anim_time(10.0), 0.0);
        }
    }
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
            WeaponId::Ak74 => WeaponSpec {
                // A full-auto rifle: many lighter hits rather than one big
                // one. Against round 1's 100-health zombies a body shot does
                // 40 up close (three to kill), a headshot 60 (two); each
                // Pack-a-Punch level doubles it. Full damage out to 30 m, then
                // down to 60% at its 200 m reach.
                max_range: 200.0,
                muzzle_velocity: f32::INFINITY,
                gravity: 0.0,
                base_damage: 40.0,
                falloff_start: 30.0,
                headshot_multiplier: 1.5,
                min_damage_fraction: 0.6,
                lower_body_multiplier: 0.85,
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
            WeaponId::RayGun => WeaponSpec {
                // The bolt's direct hit, on top of its burst
                // (`raygun_splash_damage`) — a zombie's head is no softer to
                // it. Its burst is the real point: a crowd at once.
                max_range: 150.0,
                muzzle_velocity: f32::INFINITY,
                gravity: 0.0,
                base_damage: 300.0,
                falloff_start: 150.0,
                headshot_multiplier: 1.0,
                min_damage_fraction: 1.0,
                lower_body_multiplier: 1.0,
            },
        }
    }

    /// Wire representation used in [`crate::protocol::PlayerInput::weapon`].
    pub const fn as_u8(self) -> u8 {
        match self {
            WeaponId::Sniper => 0,
            WeaponId::Marksman => 1,
            WeaponId::Ak74 => 2,
            WeaponId::RayGun => 3,
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(WeaponId::Sniper),
            1 => Some(WeaponId::Marksman),
            2 => Some(WeaponId::Ak74),
            3 => Some(WeaponId::RayGun),
            _ => None,
        }
    }

    /// Its name in the loadout and the HUD.
    pub const fn label(self) -> &'static str {
        match self {
            WeaponId::Sniper => "SNIPER",
            WeaponId::Marksman => "MARKSMAN",
            WeaponId::Ak74 => "AK-74",
            WeaponId::RayGun => "RAY GUN",
        }
    }

    /// Its weapon class, as the loadout lists it.
    pub const fn class_label(self) -> &'static str {
        match self {
            WeaponId::Sniper => "SNIPER RIFLE",
            WeaponId::Marksman => "MARKSMAN RIFLE",
            WeaponId::Ak74 => "ASSAULT RIFLE",
            WeaponId::RayGun => "WONDER WEAPON",
        }
    }

    /// Multiplier on its damage to players in `FreeForAll` — the AK-74's
    /// tuned for zombies, a little too strong player-on-player.
    pub const fn pvp_damage_mult(self) -> f32 {
        match self {
            WeaponId::Ak74 => 0.8,
            _ => 1.0,
        }
    }

    /// Whether it keeps firing while the trigger's held.
    pub const fn full_auto(self) -> bool {
        matches!(self, WeaponId::Ak74)
    }

    /// Whether a shot keeps going through what it hits (the sniper's
    /// collaterals) — the Ray Gun's bolt bursts on the first.
    pub const fn pierces(self) -> bool {
        !matches!(self, WeaponId::RayGun)
    }

    /// True when the weapon has no travel time and no drop, so the server can
    /// resolve it with a single ray instead of stepping a trajectory.
    pub fn is_hitscan(self) -> bool {
        let s = self.spec();
        s.muzzle_velocity.is_infinite() && s.gravity == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_gun_burst_fades_out_to_its_edge_and_stops_there() {
        assert_eq!(raygun_splash_damage(0.0), RAYGUN_SPLASH_DAMAGE_MAX);
        assert!((raygun_splash_damage(RAYGUN_SPLASH_RADIUS) - RAYGUN_SPLASH_DAMAGE_MIN).abs() < 1e-3);
        assert!(raygun_splash_damage(1.0) < RAYGUN_SPLASH_DAMAGE_MAX);
        assert_eq!(raygun_splash_damage(RAYGUN_SPLASH_RADIUS + 0.01), 0.0);
        assert_eq!(WeaponId::from_u8(WeaponId::RayGun.as_u8()), Some(WeaponId::RayGun));
        assert!(!WeaponId::RayGun.pierces() && WeaponId::Sniper.pierces());
    }
}
