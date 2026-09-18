//! The player rig's transform + physics state, ground/wall collision, and
//! walk/sprint/jump/gravity — the base locomotion every other player system
//! (slide, footsteps, teleport, camera) reads or drives.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use bevy_rapier3d::prelude::*;

use crate::keybinds::KeyBindings;
use crate::killcam;
use crate::{GameSounds, Weapon, WeaponSlot};

use super::slide::{Slide, SlideSettings, Stance, DIVE_CLEARANCE};

/// Body capsule dimensions (metres) — a rough humanoid silhouette for the
/// shadow, not a real collider.
pub(crate) const BODY_CAPSULE_RADIUS: f32 = 0.25;
pub(crate) const BODY_CAPSULE_HEIGHT: f32 = 1.6;

/// How far above the current feet position [`resolve_wall_collisions`]'s
/// probe capsule starts — clear of flush-flat ground or a shallow ramp, so
/// sweeping across it never reads as hitting a wall. Containers/crates stand
/// much taller than this, so their sides are still caught.
pub(crate) const WALL_PROBE_CLEARANCE: f32 = 0.3;

/// How far [`sweep_and_slide`] biases a wall-slide redirect away from the
/// wall, so it reads as separating rather than landing exactly on the
/// into-vs-away knife-edge — see its use there for why an exact-zero
/// tangent isn't safe.
pub(crate) const WALL_SLIDE_SKIN: f32 = 0.02;

/// Keep the player out of the selected map's solid geometry — the same
/// mesh-derived `Collider`s [`sync_map_model`] generates for whatever's
/// loaded, so there's no per-map wall list to maintain. Runs right after
/// [`move_player`] each frame: sweeps a short vertical capsule (from
/// [`WALL_PROBE_CLEARANCE`] above the feet up to head height) along this
/// frame's horizontal move, sliding along whatever wall it hits regardless of
/// the angle it's hit at (see [`sweep_and_slide`]).
pub(crate) fn resolve_wall_collisions(
    time: Res<Time>,
    rapier: ReadRapierContext,
    mut player: Single<(&mut Transform, &PlayerPhysics), With<Player>>,
) {
    let Ok(rapier) = rapier.single() else {
        return;
    };
    let (transform, physics) = &mut *player;
    let dt = time.delta_secs();
    let delta = Vec3::new(
        physics.horizontal_velocity.x * dt,
        0.0,
        physics.horizontal_velocity.z * dt,
    );
    if delta.length_squared() < 1.0e-10 {
        return;
    }
    // `move_player`, right before this in the system chain, already
    // integrated this frame's move into `transform` — back it out to recover
    // the (already-valid, non-penetrating) position the sweep starts from.
    let start = transform.translation - delta;
    let feet = start.y - EYE_HEIGHT;
    let probe_bottom = feet + WALL_PROBE_CLEARANCE;
    let probe_top = feet + BODY_CAPSULE_HEIGHT;
    let half_height = ((probe_top - probe_bottom) / 2.0 - BODY_CAPSULE_RADIUS).max(0.01);
    let probe = Collider::capsule_y(half_height, BODY_CAPSULE_RADIUS);

    let mut pos = Vec3::new(start.x, (probe_bottom + probe_top) / 2.0, start.z);
    pos += sweep_and_slide(&rapier, pos, delta, &probe);
    transform.translation.x = pos.x;
    transform.translation.z = pos.z;
}

/// Sweeps `probe` from `origin` toward `delta`, sliding along any wall-like
/// hit instead of stopping dead: the leftover distance for that hit is
/// projected onto the wall's tangent plane (dropping only the component that
/// points into the wall) and the sweep continues from there. Doing this off
/// the hit's actual surface normal — rather than resolving the move one world
/// axis at a time — means a wall met at any angle lets the player keep
/// sliding along it, not just one hit square-on or exactly parallel to a
/// world axis. A hit whose surface normal is mostly vertical is a floor or a
/// shallow ramp, not a wall — that doesn't redirect anything, the sweep just
/// steps past it and keeps going with whatever distance remains, so grazing
/// the ground can't stall horizontal movement either. Repeats a few times so
/// two walls in a row (a corner) still resolve.
pub(crate) fn sweep_and_slide(
    rapier: &RapierContext,
    origin: Vec3,
    delta: Vec3,
    probe: &Collider,
) -> Vec3 {
    if delta.length_squared() < 1.0e-10 {
        return Vec3::ZERO;
    }
    // `stop_at_penetration: false` matters here: once a frame's sweep stops
    // the player flush against a wall (within `target_distance`), *every*
    // subsequent frame starts already touching it. With `true`, a touching
    // start is always reported as an immediate (`time_of_impact: 0`) hit
    // regardless of which way `delta` points, which pins the player against
    // the wall forever — stepping away can't out-run its own starting
    // contact. `false` discards that trivial hit specifically when `delta`
    // is separating (moving away) and searches forward for a real impact
    // instead, so walking away from a wall you're resting on works, while
    // walking further into it still blocks immediately.
    let options = ShapeCastOptions {
        max_time_of_impact: 1.0,
        target_distance: 0.01,
        stop_at_penetration: false,
        compute_impact_geometry_on_penetration: true,
    };
    let mut pos = origin;
    let mut remaining = delta;
    let mut travelled = Vec3::ZERO;
    for _ in 0..4 {
        if remaining.length_squared() < 1.0e-10 {
            break;
        }
        let Some((_, hit)) = rapier.cast_shape(
            pos,
            Quat::IDENTITY,
            remaining,
            probe,
            options,
            QueryFilter::default(),
        ) else {
            travelled += remaining;
            break;
        };
        let step = remaining * hit.time_of_impact;
        travelled += step;
        pos += step;
        let leftover = remaining - step;

        let Some(normal) = hit.details.map(|d| d.normal1) else {
            break; // no normal to slide off of — stop here, as before.
        };
        if normal.y.abs() >= 0.5 {
            // Floor / shallow ramp — not a wall, keep going the same direction.
            remaining = leftover;
            continue;
        }
        // Wall — slide: drop the leftover move's into-wall component, keep
        // the rest (tangent to the wall's surface) for the next sweep. Biased
        // by `WALL_SLIDE_SKIN` to land just barely *separating* rather than
        // exactly tangent (dot-with-normal exactly 0): `stop_at_penetration:
        // false` above only treats a touching start as free to move when
        // `remaining` is separating, so an exact-zero tangent sits right on
        // that knife-edge and floating-point noise in `normal` flips it
        // frame to frame — some frames read as "moving further in" and get
        // blocked at `time_of_impact: 0`, others read as separating and
        // slide the full distance, which is exactly the slow/fast/slow
        // stutter sliding along a wall at an angle used to have.
        let n = Vec3::new(normal.x, 0.0, normal.z).normalize_or_zero();
        remaining = leftover - n * (leftover.dot(n) - WALL_SLIDE_SKIN);
    }
    travelled
}

/// Player camera height above the feet — used to test the feet against surfaces.
pub(crate) const EYE_HEIGHT: f32 = 1.7;

/// Defaults for the "Movement" panel section (all live-adjustable).
pub(crate) const WALK_SPEED: f32 = 3.0;
pub(crate) const SPRINT_SPEED: f32 = 8.0;
/// Multiplier on speed while strafing (left/right, no forward/back held).
pub(crate) const STRAFE_SPEED_MULT: f32 = 0.8;
/// Multiplier on speed while moving backward.
pub(crate) const BACKWARD_SPEED_MULT: f32 = 0.8;
pub(crate) const GRAVITY: f32 = 22.0;
pub(crate) const JUMP_SPEED: f32 = 8.0;
/// Feet within this distance above a surface still count as standing on it.
pub(crate) const GROUND_SNAP: f32 = 0.5;

/// Movement is this much faster in every stance while the (model-less) secondary
/// is equipped — the knife is lighter than the sniper.
pub(crate) const SECONDARY_MOVE_MULT: f32 = 1.12;

/// Root of the player rig. Carries position and yaw (horizontal look).
#[derive(Component)]
pub(crate) struct Player;

/// Player physics: horizontal velocity (input-driven on the ground, frozen in
/// the air so a jump carries momentum), falling speed, and whether the feet are
/// resting on a surface.
#[derive(Component, Default)]
pub(crate) struct PlayerPhysics {
    pub(crate) horizontal_velocity: Vec3,
    pub(crate) vertical_velocity: f32,
    pub(crate) grounded: bool,
}

/// Panel-adjustable locomotion + gravity tuning.
#[derive(Resource)]
pub(crate) struct MovementSettings {
    /// Horizontal speed while walking (m/s).
    pub(crate) walk_speed: f32,
    /// Horizontal speed while sprinting (m/s).
    pub(crate) sprint_speed: f32,
    /// Multiplier applied to speed while strafing (left/right held, neither
    /// forward nor back) — forward held takes priority over this even if
    /// also strafing, so this only ever kicks in for a pure sideways step.
    pub(crate) strafe_speed_mult: f32,
    /// Multiplier applied to speed while moving backward (back held) —
    /// takes priority over the strafe multiplier if both back and a strafe
    /// key are held, since forward/back matters more for aim/visibility.
    pub(crate) backward_speed_mult: f32,
    /// Downward acceleration magnitude (m/s²).
    pub(crate) gravity: f32,
    /// Upward launch speed when jumping (m/s).
    pub(crate) jump_speed: f32,
}

impl Default for MovementSettings {
    fn default() -> Self {
        Self {
            walk_speed: WALK_SPEED,
            sprint_speed: SPRINT_SPEED,
            strafe_speed_mult: STRAFE_SPEED_MULT,
            backward_speed_mult: BACKWARD_SPEED_MULT,
            gravity: GRAVITY,
            jump_speed: JUMP_SPEED,
        }
    }
}

/// Sprint toggle state (Left Shift flips it). Persists through slides and
/// dives — only actually crouching or going prone suppresses its effect
/// (see `crouch_slide`), and it resumes on its own when you stand back up.
#[derive(Resource, Default)]
pub(crate) struct Sprinting(pub(crate) bool);

/// True from the moment `jump` launches until `apply_gravity` detects a
/// landing. Read (not drained) by `net::write_input` into
/// `PlayerInput::jumping`, so remote avatars can play the jump animation.
///
/// Sustained across the whole jump arc rather than a single-tick pulse
/// deliberately: `PlayerPose`'s interpolation on other clients only keeps the
/// *last* of any confirmed ticks it has to skip over to catch up (see
/// `lightyear_interpolation::update_interpolate_status`'s `pop_until_tick`),
/// so a value that's only ever true for one tick can silently vanish before a
/// remote client ever sees it — same reasoning as `crouching`/`reloading`.
#[derive(Resource, Default)]
pub(crate) struct Jumping(pub(crate) bool);

/// Sprint control (Call-of-Duty MW3 style): the sprint key flips sprint on /
/// off, and sprint also drops on its own the moment the player stops feeding
/// a movement key (so it never "sticks" while standing still). The toggle
/// otherwise stays on through slides and dives; `crouch_slide` doesn't clear
/// it for those, it just doesn't apply while actually crouched or prone (see
/// the explicit `Stance::Crouching`/`Stance::Prone` arms in `move_player` and
/// `footsteps`), so sprint resumes on its own the moment the player stands
/// back up — no need to press the button again. Pressing sprint out of a
/// crouch or prone still stands the player up with sprint already active.
pub(crate) fn toggle_sprint(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut sprinting: ResMut<Sprinting>,
) {
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }
    let move_held = binds.forward.pressed(&keys, &mouse)
        || binds.back.pressed(&keys, &mouse)
        || binds.left.pressed(&keys, &mouse)
        || binds.right.pressed(&keys, &mouse);

    if binds.sprint.just_pressed(&keys, &mouse) {
        sprinting.0 = !sprinting.0;
    } else if sprinting.0 && !move_held {
        sprinting.0 = false; // stopped moving — sprint drops
    }
}

pub(crate) fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    settings: Res<MovementSettings>,
    slide_cfg: Res<SlideSettings>,
    sprinting: Res<Sprinting>,
    slide: Res<Slide>,
    weapon: Res<Weapon>,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    let (mut transform, mut physics) = player.into_inner();
    // The lighter secondary lets the player move a touch quicker in every stance.
    let weapon_mult = if weapon.slot == WeaponSlot::Secondary {
        SECONDARY_MOVE_MULT
    } else {
        1.0
    };

    // A slide ignores steering entirely — `crouch_slide` owns the velocity and
    // bleeds it off with friction (already scaled for the equipped weapon).
    if slide.stance == Stance::Sliding {
        physics.horizontal_velocity = slide.velocity;
    } else if physics.grounded {
        // Input only steers you while your feet are on something — in the air
        // the velocity from the moment you left the ground carries you.
        let mut direction = Vec3::ZERO;
        let forward = *transform.forward();
        let right = *transform.right();
        let forward_held = binds.forward.pressed(&keys, &mouse);
        let back_held = binds.back.pressed(&keys, &mouse);
        let right_held = binds.right.pressed(&keys, &mouse);
        let left_held = binds.left.pressed(&keys, &mouse);
        if forward_held {
            direction += forward;
        }
        if back_held {
            direction -= forward;
        }
        if right_held {
            direction += right;
        }
        if left_held {
            direction -= right;
        }
        direction.y = 0.0;

        let speed = match slide.stance {
            Stance::Crouching => slide_cfg.crouch_speed,
            Stance::Prone => slide_cfg.prone_speed,
            _ if sprinting.0 => settings.sprint_speed,
            _ => settings.walk_speed,
        };
        // Forward held always moves at full speed, even mixed with a strafe
        // key; back held is the next priority (a pure sideways step is the
        // only case the strafe multiplier applies to).
        let dir_mult = if forward_held {
            1.0
        } else if back_held {
            settings.backward_speed_mult
        } else if right_held || left_held {
            settings.strafe_speed_mult
        } else {
            1.0
        };
        physics.horizontal_velocity =
            direction.normalize_or_zero() * speed * weapon_mult * dir_mult;
    }

    transform.translation += physics.horizontal_velocity * time.delta_secs();
}

/// Launches the player upward when they're standing on something. While
/// crouched or sliding the jump key is spoken for (stand up / slide-cancel — see
/// `crouch_slide`), so this bails on anything but a plain standing jump.
#[allow(clippy::too_many_arguments)]
pub(crate) fn jump(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    settings: Res<MovementSettings>,
    slide: Res<Slide>,
    mut jumping: ResMut<Jumping>,
    mut physics: Single<&mut PlayerPhysics, With<Player>>,
) {
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }
    if slide.stance != Stance::Standing || slide.ate_jump {
        return;
    }
    if physics.grounded && binds.jump.just_pressed(&keys, &mouse) {
        physics.vertical_velocity = settings.jump_speed;
        physics.grounded = false;
        jumping.0 = true;
    }
}

/// Max distance [`apply_gravity`]'s ground raycast searches below the feet —
/// generous enough to cover a fall from anywhere on any of this game's maps,
/// down to the always-present fallback floor (`setup_world`'s
/// `ProceduralGround` collider) if the map's own mesh doesn't cover a point.
pub(crate) const GROUND_RAY_MAX_DIST: f32 = 500.0;

/// Pull the player down and stop them on whichever surface is under them —
/// found with a raycast straight down against the current map's mesh
/// collider (see `sync_map_model`), so this works the same way for every map
/// without any per-map data. Walk off an edge and there's nothing under the
/// feet, so the player falls.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_gravity(
    time: Res<Time>,
    settings: Res<MovementSettings>,
    slide: Res<Slide>,
    rapier: ReadRapierContext,
    sounds: Res<GameSounds>,
    mut commands: Commands,
    mut jumping: ResMut<Jumping>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    player: Single<(&mut Transform, &mut PlayerPhysics), With<Player>>,
) {
    let dt = time.delta_secs();
    let (mut transform, mut physics) = player.into_inner();
    let was_grounded = physics.grounded;

    physics.vertical_velocity -= settings.gravity * dt;

    let feet_now = transform.translation.y - EYE_HEIGHT;
    let feet_next = feet_now + physics.vertical_velocity * dt;

    // Only land on the map's surface from above/at its level — not when
    // walking through its base at ground height. Cast down from a
    // `GROUND_SNAP` margin above the current feet, so a surface further below
    // only counts once the fall actually reaches it.
    let mut surface = f32::NEG_INFINITY;
    if let Ok(rapier) = rapier.single() {
        let origin = Vec3::new(
            transform.translation.x,
            feet_now + GROUND_SNAP,
            transform.translation.z,
        );
        if let Some((_, toi)) = rapier.cast_ray(
            origin,
            Vec3::NEG_Y,
            GROUND_RAY_MAX_DIST,
            true,
            QueryFilter::default(),
        ) {
            surface = origin.y - toi;
        }
    }

    // A dolphin dive lands on the belly: it also counts as touching down once
    // the tucked camera (`transform.y + slide.drop`, drop ≤ 0) gets within
    // `DIVE_CLEARANCE` of the surface — sooner than the standing feet would.
    let dive_landed = slide.stance == Stance::Diving
        && transform.translation.y + slide.drop + physics.vertical_velocity * dt
            <= surface + DIVE_CLEARANCE;

    if feet_next <= surface || dive_landed {
        transform.translation.y = surface + EYE_HEIGHT;
        physics.vertical_velocity = 0.0;
        physics.grounded = true;
    } else {
        transform.translation.y = feet_next + EYE_HEIGHT;
        physics.grounded = false;
    }

    if !was_grounded && physics.grounded {
        jumping.0 = false;
    }

    // Landing thump: airborne to grounded this frame. A dive lands on the
    // belly and already has its own sound (`SND_DIVE`), so it's excluded here.
    if !was_grounded && physics.grounded && !dive_landed {
        commands.spawn((
            AudioPlayer::new(sounds.jump_land.clone()),
            PlaybackSettings::DESPAWN,
        ));
        snd.note(killcam::SND_JUMP_LAND);
    }
}

/// Dev convenience: `P` logs the player's world position to the console.
/// Deliberately not a `KeyBindings` entry — always `P`, not rebindable.
pub(crate) fn log_player_position(
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    player: Single<&Transform, With<Player>>,
) {
    if window.cursor_options.grab_mode == CursorGrabMode::None {
        return;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        let pos = player.translation;
        info!("player position: {:.2}, {:.2}, {:.2}", pos.x, pos.y, pos.z);
    }
}
