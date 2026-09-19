//! Death by falling: an absolute "void" floor for maps with no ground under a
//! long drop (see `apply_gravity`'s downward raycast — a miss just lets the
//! player fall forever), plus CoD-style fall damage for maps that do have
//! ground down there. Client-authoritative, like all local movement (see
//! `server::sim::apply_client_pose`) — the client decides it happened and
//! tells the server (`shared::FellToDeath`), which sends back a
//! `PlayerRespawn` (marking the player dead first, if they have a
//! `PlayerCombat` — `FreeForAll` only; `Freestyle` players are always
//! "alive" and just get a fresh spot, see `server::pvp::on_fell_to_death`),
//! but queues no kill cam either way (there's no killer).
//!
//! The effect mirrors `death_effect` (and reuses its overlay/weapon-hide via
//! [`death_effect::show_overlay_and_hide_weapon`]): every system that would
//! move the rig defers to [`no_fall_death`] the same way it already defers to
//! `killcam::no_killcam`, so the camera holds the exact position/orientation
//! it had at the moment of death. Unlike `death_effect`'s one-shot pan onto a
//! stationary killer, this one tracks a moving target — a one-off
//! `models/soldier.glb` body (the local player normally has no third-person
//! model at all; see `net::spawn_remote_avatars`, which only ever spawns one
//! for *other* players) that keeps falling and slowly tumbling below. Ends
//! the same way `death_effect` does: on `net::LocalPlayerRespawned`.

use std::time::Duration;

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::{AnimationPlayer, RepeatAnimation};
use bevy::prelude::*;
use lightyear::prelude::*;

use shared::{FellToDeath, LobbyChannel};

use crate::death_effect;
use crate::net::{GameClient, LocalPlayerRespawned};
use crate::util::{ease, rand01};
use crate::{
    AppState, KnifeViewModel, MovementSettings, Player, PlayerHead, PlayerPhysics,
    RemoteAvatarSettings, SoldierAnimState, SoldierAnimationPlayer, SoldierAnimations,
    SoldierVisual, ViewModel, EYE_HEIGHT, PITCH_LIMIT,
};

/// Net descent (apex → landing) beyond which landing on solid ground is
/// lethal — a starting point pitched at "you'd die falling off about a 4-5
/// story building," CoD-style fall damage. Tune to taste.
const LETHAL_FALL_DISTANCE: f32 = 12.0;

/// World-space feet-Y below which the player has fallen into the void and
/// dies outright — the map has no ground to ever land on down there, so
/// without this they'd fall forever.
const VOID_DEATH_Y: f32 = -30.0;

/// How long the dropped body takes to complete one full tumble.
const TUMBLE_SECS: f32 = 4.0;

/// How long the initial camera pan onto the body takes.
const PAN_SECS: f32 = 0.3;

/// Marks the one-off falling-body entity spawned for this effect.
#[derive(Component)]
struct FallDeathBody;

#[derive(Resource, Default)]
pub(crate) struct FallDeathState {
    effect: Option<FallEffect>,
}

struct FallEffect {
    body: Entity,
    fall_speed: f32,
    /// Carried over from `PlayerPhysics::horizontal_velocity` at the moment
    /// of death, same as the live player's own airborne physics treats it
    /// (frozen, not re-accelerated) — so the body keeps drifting the way the
    /// player was actually moving instead of dropping straight down.
    horizontal_velocity: Vec3,
    spin_axis: Vec3,
    start_yaw: f32,
    start_pitch: f32,
    elapsed: f32,
}

/// Per-frame fall tracking, independent of [`FallDeathState`] so it keeps
/// working across an ordinary landing (no death) the same way before and
/// after this feature existed.
#[derive(Resource, Default)]
struct FallTracking {
    /// Highest feet-Y observed since the player was last grounded.
    apex_y: f32,
    was_grounded: bool,
    /// `PlayerPhysics::vertical_velocity` as of last frame — `apply_gravity`
    /// zeroes it the instant it detects a landing, so by the time this
    /// module's own `.after(apply_gravity)` system sees a landing this frame,
    /// the live value is already gone; this is the last one worth anything.
    prev_vertical_velocity: f32,
}

pub(crate) struct FallDeathPlugin;

impl Plugin for FallDeathPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FallDeathState>();
        app.init_resource::<FallTracking>();
        app.add_systems(
            Update,
            (
                // Also defers to an in-progress kill-death effect (see
                // `death_effect`) — the movement/gravity freeze on death by
                // another player isn't instant (a known gap; see that
                // module), so without this a fall crossed during that brief
                // window could fire a second, competing effect on top.
                check_fall_death.after(crate::apply_gravity).run_if(
                    crate::menu::game_active
                        .and(crate::killcam::no_killcam)
                        .and(no_fall_death)
                        .and(not(crate::death_effect::death_effect_active)),
                ),
                pan_to_body.after(check_fall_death),
                pose_fall_death_body,
                clear_on_respawn,
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// True while nothing is falling to its death right now — every system that
/// moves the rig (movement, gravity, mouse look, ...) defers to this the same
/// way it already defers to `killcam::no_killcam`.
pub(crate) fn no_fall_death(state: Res<FallDeathState>) -> bool {
    state.effect.is_none()
}

/// True while the fall-death pan/hold is active. `look_around` defers to
/// this, same idea as `death_effect::death_effect_active`.
pub(crate) fn effect_active(state: Res<FallDeathState>) -> bool {
    state.effect.is_some()
}

#[allow(clippy::too_many_arguments)]
fn check_fall_death(
    time: Res<Time>,
    mut tracking: ResMut<FallTracking>,
    mut state: ResMut<FallDeathState>,
    mut sender_q: Query<&mut TriggerSender<FellToDeath>, With<GameClient>>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    remote_avatar_settings: Res<RemoteAvatarSettings>,
    mut death_effect: ResMut<death_effect::DeathEffect>,
    existing_overlay: Query<(), With<death_effect::DeathOverlay>>,
    mut sniper_vis: Single<&mut Visibility, (With<ViewModel>, Without<KnifeViewModel>)>,
    mut knife_vis: Single<&mut Visibility, (With<KnifeViewModel>, Without<ViewModel>)>,
    player: Single<(&Transform, &PlayerPhysics), With<Player>>,
    head: Single<&Transform, (With<PlayerHead>, Without<Player>)>,
) {
    if state.effect.is_some() {
        return;
    }
    let (transform, physics) = player.into_inner();
    let feet_y = transform.translation.y - EYE_HEIGHT;

    let mut lethal_fall_speed = None;
    if physics.grounded {
        let just_landed = !tracking.was_grounded;
        if just_landed && tracking.apex_y - feet_y >= LETHAL_FALL_DISTANCE {
            lethal_fall_speed = Some(tracking.prev_vertical_velocity.abs());
        }
        tracking.apex_y = feet_y;
    } else {
        if tracking.was_grounded {
            tracking.apex_y = feet_y;
        } else {
            tracking.apex_y = tracking.apex_y.max(feet_y);
        }
        if feet_y < VOID_DEATH_Y {
            lethal_fall_speed = Some(physics.vertical_velocity.abs());
        }
    }
    tracking.was_grounded = physics.grounded;
    tracking.prev_vertical_velocity = physics.vertical_velocity;

    let Some(fall_speed) = lethal_fall_speed else {
        return;
    };

    if let Ok(mut sender) = sender_q.single_mut() {
        sender.trigger::<LobbyChannel>(FellToDeath);
    }

    death_effect::show_overlay_and_hide_weapon(
        &mut death_effect,
        &mut commands,
        &asset_server,
        !existing_overlay.is_empty(),
        &mut **sniper_vis,
        &mut **knife_vis,
    );

    let (_, start_pitch, _) = head.rotation.to_euler(EulerRot::YXZ);
    let (start_yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);

    let seed = time.elapsed().as_nanos() as u32;
    let theta = rand01(seed) * std::f32::consts::TAU;
    let phi = rand01(seed.wrapping_add(1)) * std::f32::consts::PI;
    let spin_axis = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin())
        .normalize_or(Vec3::Y);

    let body = commands
        .spawn((
            StateScoped(AppState::InGame),
            FallDeathBody,
            SoldierVisual,
            Transform {
                translation: transform.translation - Vec3::Y * EYE_HEIGHT,
                rotation: Quat::from_rotation_y(start_yaw + std::f32::consts::PI),
                scale: Vec3::splat(remote_avatar_settings.scale),
            },
            Visibility::default(),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/soldier.glb"))),
        ))
        .observe(crate::start_soldier_animation)
        .id();

    state.effect = Some(FallEffect {
        body,
        fall_speed,
        horizontal_velocity: physics.horizontal_velocity,
        spin_axis,
        start_yaw,
        start_pitch,
        elapsed: 0.0,
    });
}

/// Force the one-off body straight into its collapsed "dead" pose the moment
/// its `AnimationPlayer` is ready — it's never been alive, so there's no idle
/// pose worth blending from (`Duration::ZERO`, unlike the fade a live→dead
/// remote avatar gets in `net::animate_remote_avatars`).
fn pose_fall_death_body(
    mut anim_players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    bodies: Query<&SoldierAnimationPlayer, (With<FallDeathBody>, Added<SoldierAnimationPlayer>)>,
    anims: Res<SoldierAnimations>,
) {
    for anim_player in &bodies {
        if let Ok((mut player, mut transitions)) = anim_players.get_mut(anim_player.0) {
            transitions
                .play(&mut player, anims.node_for(SoldierAnimState::Dead), Duration::ZERO)
                .set_repeat(RepeatAnimation::Never);
        }
    }
}

fn pan_to_body(
    time: Res<Time>,
    settings: Res<MovementSettings>,
    mut state: ResMut<FallDeathState>,
    mut bodies: Query<&mut Transform, (With<FallDeathBody>, Without<Player>, Without<PlayerHead>)>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>, Without<FallDeathBody>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>, Without<FallDeathBody>)>,
) {
    let Some(effect) = state.effect.as_mut() else {
        return;
    };
    let Ok(mut body_tf) = bodies.get_mut(effect.body) else {
        return;
    };

    // Keep falling and tumbling exactly like the player would have.
    let dt = time.delta_secs();
    effect.fall_speed += settings.gravity * dt;
    body_tf.translation.y -= effect.fall_speed * dt;
    body_tf.translation += effect.horizontal_velocity * dt;
    body_tf.rotate(Quat::from_axis_angle(
        effect.spin_axis,
        (std::f32::consts::TAU / TUMBLE_SECS) * dt,
    ));

    // Live look-at onto the (moving) body, eased in from the look direction
    // at the moment of death rather than a hard cut.
    let direction = (body_tf.translation - player.translation).normalize_or_zero();
    let (target_yaw, target_pitch) = if direction == Vec3::ZERO {
        (effect.start_yaw, effect.start_pitch)
    } else {
        let (yaw, pitch, _) = Transform::IDENTITY
            .looking_to(direction, Vec3::Y)
            .rotation
            .to_euler(EulerRot::YXZ);
        (yaw, pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT))
    };

    effect.elapsed = (effect.elapsed + dt).min(PAN_SECS);
    let t = ease(effect.elapsed / PAN_SECS);
    let yaw_delta = shortest_delta(effect.start_yaw, target_yaw);
    player.rotation = Quat::from_rotation_y(effect.start_yaw.lerp(effect.start_yaw + yaw_delta, t));
    head.rotation = Quat::from_rotation_x(effect.start_pitch.lerp(target_pitch, t));
}

/// Wrap `to - from` into `(-PI, PI]` — the short way around, whichever
/// direction that is.
fn shortest_delta(from: f32, to: f32) -> f32 {
    let diff = (to - from).rem_euclid(std::f32::consts::TAU);
    if diff > std::f32::consts::PI {
        diff - std::f32::consts::TAU
    } else {
        diff
    }
}

fn clear_on_respawn(
    mut respawned: EventReader<LocalPlayerRespawned>,
    mut state: ResMut<FallDeathState>,
    mut tracking: ResMut<FallTracking>,
    mut commands: Commands,
) {
    if respawned.read().count() == 0 {
        return;
    }
    if let Some(effect) = state.effect.take() {
        commands.entity(effect.body).try_despawn();
    }
    *tracking = FallTracking::default();
}
