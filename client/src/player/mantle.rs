//! Ledge mantling (Call of Duty style): if the player is airborne and would
//! otherwise just bonk into a wall and fall, but there's a walkable ledge
//! within reach right above the impact point, catch them and lift them up
//! onto it instead. Purely a movement/physics feature for now — no climb
//! animation; the view model just rides along however it normally would.
//!
//! [`try_mantle`] looks for one every frame the player is airborne and moving
//! toward something (gated by [`crate::settings::AutoMantle`]); once found,
//! [`drive_mantle`] eases the player up and over it, and every other
//! movement/gravity/firing system pauses for the climb — see
//! [`not_mantling`].

use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::settings::{AutoMantle, Settings};
use crate::util::ease;

use super::movement::{Jumping, Player, PlayerPhysics, BODY_CAPSULE_HEIGHT, EYE_HEIGHT};
use super::slide::{Slide, Stance};

/// Ledge must clear at least this much above the feet to be worth mantling
/// over — anything shorter, ordinary ground-snapping in `apply_gravity`
/// already handles as a step.
pub(crate) const MANTLE_MIN_HEIGHT: f32 = 0.35;
/// Highest a ledge can be above the feet and still be within reach.
pub(crate) const MANTLE_MAX_HEIGHT: f32 = 2.1;
/// Forward wall probe height above the feet ("chest height") — confirms
/// there's something to catch on, and, via its hit normal, that it's a wall
/// face rather than a floor/ramp underfoot.
pub(crate) const MANTLE_PROBE_HEIGHT: f32 = 1.1;
/// How far ahead the forward wall probe reaches.
pub(crate) const MANTLE_FORWARD_DIST: f32 = 0.6;
/// How far past the wall face the downward ledge-top probe is placed, so it
/// lands beyond the wall's own thickness instead of on top of it.
pub(crate) const MANTLE_LEDGE_PROBE_FORWARD: f32 = 0.35;
/// Seconds the climb itself takes, start to finish.
pub(crate) const MANTLE_DURATION: f32 = 0.45;

/// Panel-adjustable mantle tuning ("Mantle" debug-panel section).
#[derive(Resource)]
pub(crate) struct MantleSettings {
    pub(crate) min_height: f32,
    pub(crate) max_height: f32,
    pub(crate) probe_height: f32,
    pub(crate) forward_dist: f32,
    pub(crate) ledge_probe_forward: f32,
    pub(crate) duration: f32,
}

impl Default for MantleSettings {
    fn default() -> Self {
        Self {
            min_height: MANTLE_MIN_HEIGHT,
            max_height: MANTLE_MAX_HEIGHT,
            probe_height: MANTLE_PROBE_HEIGHT,
            forward_dist: MANTLE_FORWARD_DIST,
            ledge_probe_forward: MANTLE_LEDGE_PROBE_FORWARD,
            duration: MANTLE_DURATION,
        }
    }
}

/// An in-progress climb: eases the player from `start` up onto `end` (both
/// eye-height world positions) over `MantleSettings::duration` — see
/// [`drive_mantle`].
pub(crate) struct MantleRun {
    start: Vec3,
    end: Vec3,
    elapsed: f32,
}

/// `Some` for the whole duration of a climb — see [`not_mantling`].
#[derive(Resource, Default)]
pub(crate) struct Mantle {
    pub(crate) active: Option<MantleRun>,
}

/// Run condition: every other movement/gravity/firing system pauses for the
/// whole climb, the same way `killcam::no_killcam` pauses them for a replay —
/// `drive_mantle` alone drives `Player`'s transform until it finishes.
pub(crate) fn not_mantling(mantle: Res<Mantle>) -> bool {
    mantle.active.is_none()
}

/// Looks for a valid mantle from `feet` toward `dir` (a normalized
/// horizontal direction) and, if found, returns the landing feet position
/// (world space). Standard three-probe ledge check: a forward probe finds
/// the wall to catch on, a downward probe finds its top, and an upward probe
/// at the landing spot confirms there's room to actually stand there.
fn find_mantle(rapier: &RapierContext, feet: Vec3, dir: Vec3, cfg: &MantleSettings) -> Option<Vec3> {
    let filter = QueryFilter::default();

    // Chest-height probe: something to catch on ahead, and — via its hit
    // normal — a wall face rather than a floor/ramp underfoot.
    let chest = feet + Vec3::Y * cfg.probe_height;
    let (_, wall_hit) =
        rapier.cast_ray_and_get_normal(chest, dir, cfg.forward_dist, true, filter)?;
    if wall_hit.normal.y.abs() >= 0.5 {
        return None;
    }

    // The wall must not keep going above the highest reachable ledge — if it
    // does, this is just a tall wall, not something to climb over.
    let top = feet + Vec3::Y * cfg.max_height;
    if rapier.cast_ray(top, dir, cfg.forward_dist, true, filter).is_some() {
        return None;
    }

    // Ledge top: straight down from above the wall, a bit past its face so
    // the ray starts clear of the wall's own thickness instead of on top of it.
    let past_wall = wall_hit.point + dir * cfg.ledge_probe_forward;
    let down_from = Vec3::new(past_wall.x, feet.y + cfg.max_height, past_wall.z);
    let (_, ledge_hit) = rapier.cast_ray_and_get_normal(
        down_from,
        Vec3::NEG_Y,
        cfg.max_height - cfg.min_height + 0.5,
        true,
        filter,
    )?;
    if ledge_hit.normal.y < 0.5 {
        return None; // too steep to stand on
    }
    let ledge_y = down_from.y - ledge_hit.time_of_impact;
    let height = ledge_y - feet.y;
    if height < cfg.min_height || height > cfg.max_height {
        return None;
    }

    // Headroom at the landing spot: nothing overhead for the player's full height.
    let land = Vec3::new(past_wall.x, ledge_y + 0.05, past_wall.z);
    if rapier
        .cast_ray(land, Vec3::Y, BODY_CAPSULE_HEIGHT, true, filter)
        .is_some()
    {
        return None;
    }

    Some(Vec3::new(past_wall.x, ledge_y, past_wall.z))
}

/// Looks for a mantle opportunity while airborne and, if one's found, hands
/// control over to [`drive_mantle`] for the climb. Gated by
/// `Settings::auto_mantle` — Call of Duty's own "Automatic Mantle" option:
/// `Off` never triggers; `SemiAuto` only while the player actively jumped
/// (a deliberate running jump at a ledge); `FullAuto` any time they're
/// airborne and moving toward one, jumped or not (e.g. walking or falling
/// off a ledge onto a lower one). Only from `Stance::Standing` — crouch,
/// slide, prone and dive don't attempt a mantle.
pub(crate) fn try_mantle(
    settings: Res<Settings>,
    cfg: Res<MantleSettings>,
    jumping: Res<Jumping>,
    slide: Res<Slide>,
    rapier: ReadRapierContext,
    mut mantle: ResMut<Mantle>,
    player: Single<(&Transform, &PlayerPhysics), With<Player>>,
) {
    if settings.auto_mantle == AutoMantle::Off {
        return;
    }
    if settings.auto_mantle == AutoMantle::SemiAuto && !jumping.0 {
        return;
    }
    let (transform, physics) = player.into_inner();
    if physics.grounded || slide.stance != Stance::Standing {
        return;
    }
    let dir = Vec3::new(physics.horizontal_velocity.x, 0.0, physics.horizontal_velocity.z)
        .normalize_or_zero();
    if dir == Vec3::ZERO {
        return;
    }
    let Ok(rapier) = rapier.single() else {
        return;
    };
    let feet = Vec3::new(
        transform.translation.x,
        transform.translation.y - EYE_HEIGHT,
        transform.translation.z,
    );
    let Some(landing_feet) = find_mantle(&rapier, feet, dir, &cfg) else {
        return;
    };
    mantle.active = Some(MantleRun {
        start: transform.translation,
        end: landing_feet + Vec3::Y * EYE_HEIGHT,
        elapsed: 0.0,
    });
}

/// Eases the player up and onto the ledge [`try_mantle`] found — rising over
/// the first part of the climb, then moving forward over the last part (a
/// deliberate overlap), so the camera clears the ledge's vertical face
/// instead of cutting straight through it on the way up. Ends by handing the
/// player back to normal grounded movement, feet planted on the ledge.
pub(crate) fn drive_mantle(
    time: Res<Time>,
    cfg: Res<MantleSettings>,
    mut mantle: ResMut<Mantle>,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    let Some(run) = mantle.active.as_mut() else {
        return;
    };
    run.elapsed += time.delta_secs();
    let t = (run.elapsed / cfg.duration.max(0.01)).clamp(0.0, 1.0);
    let rise = ease((t / 0.6).clamp(0.0, 1.0));
    let advance = ease(((t - 0.4) / 0.6).clamp(0.0, 1.0));

    let (mut transform, mut physics) = player.into_inner();
    transform.translation.y = run.start.y.lerp(run.end.y, rise);
    transform.translation.x = run.start.x.lerp(run.end.x, advance);
    transform.translation.z = run.start.z.lerp(run.end.z, advance);

    if t >= 1.0 {
        transform.translation = run.end;
        physics.vertical_velocity = 0.0;
        physics.horizontal_velocity = Vec3::ZERO;
        physics.grounded = true;
        mantle.active = None;
    }
}
