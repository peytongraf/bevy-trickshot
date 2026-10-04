//! Aim recoil, Call of Duty style: every shot really throws the aim off —
//! up, plus a random amount left or right — and it stays thrown off; pulling
//! the mouse back down is the player's job. (The camera shake / view punch in
//! [`super::recoil`] is separate and purely visual: it always settles back to
//! exactly where the aim was.)
//!
//! Each kick isn't a snap: it's eased in over a few frames
//! ([`AimRecoilSettings::kick_speed`]), so it reads as the gun climbing.
//! It turns the real body yaw / head pitch, so the kill cam replays it too.

use bevy::prelude::*;
use shared::weapon::WeaponId;

use super::ads::Ads;
use super::weapon::LocalShot;
use crate::util::rand01;
use crate::{Player, PlayerHead, PITCH_LIMIT};

/// One weapon's per-shot kick.
#[derive(Clone, Copy)]
pub(crate) struct WeaponRecoil {
    /// Degrees the aim climbs per shot.
    pub(crate) vertical_deg: f32,
    /// Most degrees it's thrown left or right per shot (random each shot).
    pub(crate) horizontal_deg: f32,
    /// Leans the random side-to-side one way: `-1` always left … `0` even …
    /// `1` always right.
    pub(crate) horizontal_bias: f32,
    /// Fraction of the kick left fully aimed (it eases in with ADS).
    pub(crate) ads_mult: f32,
}

/// Tuning — the debug panel's "Aim recoil" section.
#[derive(Resource, Clone)]
pub(crate) struct AimRecoilSettings {
    pub(crate) enabled: bool,
    /// How fast a kick is worked into the aim (1/s): most of it lands in
    /// about `3 / kick_speed` seconds.
    pub(crate) kick_speed: f32,
    pub(crate) sniper: WeaponRecoil,
    pub(crate) ak: WeaponRecoil,
    pub(crate) raygun: WeaponRecoil,
}

impl Default for AimRecoilSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            kick_speed: 30.0,
            // A bolt-action's one big kick: the scope jumps a good way up
            // and a little to the side, ready to be pulled back down while
            // the bolt cycles.
            sniper: WeaponRecoil {
                vertical_deg: 2.2,
                horizontal_deg: 0.6,
                horizontal_bias: 0.0,
                ads_mult: 0.7,
            },
            // Small kicks that add up over a burst — about 4° of climb over
            // ten rounds aimed, wandering side to side with a slight pull
            // right, like CoD's AK.
            ak: WeaponRecoil {
                vertical_deg: 0.55,
                horizontal_deg: 0.35,
                horizontal_bias: 0.15,
                ads_mult: 0.7,
            },
            // A sharp little hop each shot, straight up.
            raygun: WeaponRecoil {
                vertical_deg: 1.4,
                horizontal_deg: 0.25,
                horizontal_bias: 0.0,
                ads_mult: 0.6,
            },
        }
    }
}

impl AimRecoilSettings {
    pub(crate) fn for_weapon(&self, weapon: WeaponId) -> WeaponRecoil {
        match weapon {
            WeaponId::Ak74 => self.ak,
            WeaponId::RayGun => self.raygun,
            _ => self.sniper,
        }
    }
}

/// Kick still to be worked into the aim — `(yaw, pitch)` radians, left and
/// up positive — and a shot counter seeding the side-to-side roll.
#[derive(Resource, Default)]
pub(crate) struct AimRecoil {
    pending: Vec2,
    shots: u32,
}

/// Add each of our shots' kick, then ease what's pending into the body yaw
/// and head pitch. Runs right after mouse look, under the same conditions.
pub(crate) fn aim_recoil(
    time: Res<Time>,
    settings: Res<AimRecoilSettings>,
    ads: Res<Ads>,
    mut shots: EventReader<LocalShot>,
    mut state: ResMut<AimRecoil>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    for shot in shots.read() {
        if !settings.enabled {
            continue;
        }
        let w = settings.for_weapon(shot.weapon);
        let scale = 1.0f32.lerp(w.ads_mult, ads.t.clamp(0.0, 1.0));
        state.shots = state.shots.wrapping_add(1);
        let side = (rand01(state.shots.wrapping_mul(0x27D4_EB2F) ^ 0x1656_67B1) * 2.0 - 1.0 + w.horizontal_bias)
            .clamp(-1.0, 1.0);
        // Right is a negative yaw.
        state.pending += Vec2::new((-side * w.horizontal_deg).to_radians(), w.vertical_deg.to_radians()) * scale;
    }
    if state.pending.length_squared() < 1.0e-10 {
        state.pending = Vec2::ZERO;
        return;
    }
    let step = state.pending * (1.0 - (-settings.kick_speed.max(0.1) * time.delta_secs()).exp());
    state.pending -= step;
    player.rotate_y(step.x);
    let (_, pitch, _) = head.rotation.to_euler(EulerRot::YXZ);
    head.rotation = Quat::from_rotation_x((pitch + step.y).clamp(-PITCH_LIMIT, PITCH_LIMIT));
}

/// Forget any kick still pending — a new life or a new game starts with
/// steady aim.
pub(crate) fn reset_aim_recoil(mut state: ResMut<AimRecoil>) {
    state.pending = Vec2::ZERO;
}
