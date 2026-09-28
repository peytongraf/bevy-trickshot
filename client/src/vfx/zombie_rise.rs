//! A `Zombies` zombie breaking up out of the ground: the same rock + dust
//! debris as a bullet hitting the ground (`impacts::spawn_debris`), thrown up
//! in a stream of small bursts around where it's coming out — faint and
//! sparse at first, building to a full, wide churn as it nears the surface
//! ([`PEAK_AT`]), and kept at that until it's all the way out.
//!
//! The server marks a rising zombie's pose [`ZombieAnim::Rising`]; how far
//! along it is comes from how deep its feet still are
//! ([`ZOMBIE_RISE_DEPTH`] under the ground at the start). The ground height
//! is taken from where its feet were when it was first seen rising — right
//! as it spawned — and corrected upward if it ever turns out higher.
//! [`RiseFx`] lives on the avatar, so it goes with it; the particles are
//! `StateScoped(InGame)` — nothing to reset between games.

use bevy::prelude::*;
use shared::zombies::ZOMBIE_RISE_DEPTH;
use shared::{PlayerPose, ZombieAnim};

use super::impacts::{spawn_debris, DebrisBurst, DustSettings, ImpactAssets, ImpactParticle, RockSettings, IMPACT_MAX};
use crate::net::RemoteAvatar;
use crate::util::rand01;
use crate::ZombieVisual;

/// How far through the climb (0..1) the effect reaches full strength — just
/// before the zombie's head breaks the surface — and holds from there.
const PEAK_AT: f32 = 0.75;
/// Bursts per second, at the start and at full strength...
const RATE: (f32, f32) = (4.0, 18.0);
/// ...rocks and dust puffs in each...
const ROCKS: (f32, f32) = (1.0, 4.0);
const DUST: (f32, f32) = (1.0, 3.0);
/// ...how hard they're thrown / how big (1 = a bullet hit's)...
const POWER: (f32, f32) = (0.35, 1.05);
/// ...and how far (m) from the zombie's centre they can come up.
const RADIUS: (f32, f32) = (0.2, 0.6);

/// On a zombie avatar while it rises: the ground it's coming out of, and
/// the bursts owed so far (fractional).
#[derive(Component)]
pub(crate) struct RiseFx {
    ground_y: f32,
    owed: f32,
}

fn lerp((a, b): (f32, f32), t: f32) -> f32 {
    a + (b - a) * t
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn zombie_rise_debris(
    time: Res<Time>,
    assets: Res<ImpactAssets>,
    rock_settings: Res<RockSettings>,
    dust_settings: Res<DustSettings>,
    poses: Query<&PlayerPose>,
    mut avatars: Query<(Entity, &RemoteAvatar, Option<&mut RiseFx>), With<ZombieVisual>>,
    existing: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let dt = time.delta_secs();
    let mut budget = IMPACT_MAX.saturating_sub(existing.iter().count());
    for (entity, avatar, fx) in &mut avatars {
        let pose = poses.get(avatar.src).ok();
        let rising = pose.is_some_and(|p| p.alive && p.zombie == ZombieAnim::Rising);
        let Some(pose) = pose.filter(|_| rising) else {
            if fx.is_some() {
                commands.entity(entity).remove::<RiseFx>();
            }
            continue;
        };
        let feet = pose.translation - Vec3::Y * crate::EYE_HEIGHT;
        let Some(mut fx) = fx else {
            commands.entity(entity).insert(RiseFx {
                ground_y: feet.y + ZOMBIE_RISE_DEPTH,
                owed: 0.0,
            });
            continue;
        };
        fx.ground_y = fx.ground_y.max(feet.y);
        let progress = 1.0 - ((fx.ground_y - feet.y) / ZOMBIE_RISE_DEPTH).clamp(0.0, 1.0);
        // Builds slowly, then quickly toward the peak.
        let t = (progress / PEAK_AT).clamp(0.0, 1.0).powi(2);

        fx.owed += lerp(RATE, t) * dt;
        while fx.owed >= 1.0 {
            fx.owed -= 1.0;
            *seq = seq.wrapping_add(1);
            let s = seq.wrapping_mul(2_654_435_761) ^ entity.index();
            let angle = rand01(s ^ 0x1) * core::f32::consts::TAU;
            let r = lerp(RADIUS, t) * rand01(s ^ 0x2).sqrt();
            let at = Vec3::new(feet.x + r * angle.cos(), fx.ground_y, feet.z + r * angle.sin());
            // (Rounding the counts randomly, so e.g. 1.5 is one or two.)
            let count = |range: (f32, f32), salt: u32| {
                let n = lerp(range, t);
                n.floor() as u32 + u32::from(rand01(s ^ salt) < n.fract())
            };
            let burst = DebrisBurst {
                rocks: count(ROCKS, 0x3),
                dust: count(DUST, 0x4),
                power: lerp(POWER, t),
            };
            spawn_debris(
                &mut commands,
                &mut materials,
                &assets,
                &rock_settings,
                &dust_settings,
                at,
                burst,
                *seq,
                &mut budget,
            );
        }
    }
}
