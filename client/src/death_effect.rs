//! The moment-of-death screen effect for a PvP kill specifically: a
//! blood-splatter overlay + red tint plus a quick, forced look-at onto the
//! killer, covering the brief gap between the fatal hit landing
//! (`shared::PlayerKilledBy`, sent instantly) and the kill cam actually
//! taking over the rig (`killcam::start_killcam`, which — per
//! `server::killcam::flush_killcams` — can arrive up to ~1.5s later). Camera
//! *position* never moves here, only its look direction; the kill cam itself
//! is untouched and takes over exactly as it already did.
//!
//! The overlay + weapon-hide (but not the killer-pan, which doesn't apply)
//! are shared with `fall_death` via [`show_overlay_and_hide_weapon`] — see
//! [`DeathEffect`]'s doc comment for how the two effects stay out of each
//! other's way.

use std::f32::consts::TAU;

use bevy::prelude::*;
use lightyear::prelude::*;

use shared::PlayerKilledBy;

use crate::killcam::ActiveKillCam;
use crate::net::LocalPlayerRespawned;
use crate::util::ease;
use crate::{AppState, KnifeViewModel, Player, PlayerHead, ViewModel, PITCH_LIMIT};

/// How long the forced look-at pan onto the killer takes. Quick, not
/// instant, so it still reads as a snap-turn rather than a hard cut.
const PAN_SECS: f32 = 0.25;

/// Full-screen blood-splatter + red-tint root, up for exactly as long as
/// [`DeathEffect::active`] is true.
#[derive(Component)]
pub(crate) struct DeathOverlay;

/// The overlay + weapon-hide, and (for a kill specifically) the one-shot pan
/// onto the killer. `active` covers both a kill (`on_killed_by`) and a fatal
/// fall (`fall_death::check_fall_death`, via [`show_overlay_and_hide_weapon`])
/// — `pan` only ever applies to the former, since a fall drives its own
/// continuous look-at onto the falling body instead (`fall_death::pan_to_body`).
/// Kept separate so the two effects' camera control never fights: `pan`'s
/// consumer (`pan_to_killer`) only ever runs for a kill.
#[derive(Resource, Default)]
pub(crate) struct DeathEffect {
    /// True from the instant a kill or fatal fall is detected until the kill
    /// cam starts (kill only) or the player respawns anyway — see
    /// [`clear_on_killcam_or_respawn`].
    active: bool,
    pan: Option<Pan>,
    /// Whichever view model was force-hidden for the effect, if any —
    /// restored by `clear_on_killcam_or_respawn` on the no-kill-cam fallback
    /// path (always taken for a fall — there's never a kill cam). On a kill's
    /// normal path, `killcam::start_killcam` consumes (and clears) this
    /// itself instead, to recover the true pre-death visibility rather than
    /// the live one it deliberately hid — see that function's use of it.
    pub(crate) hidden_weapon: Option<HiddenWeapon>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HiddenWeapon {
    Sniper,
    Knife,
}

struct Pan {
    start_yaw: f32,
    /// Shortest-path target yaw — already unwrapped relative to `start_yaw`
    /// so a plain `lerp` never spins the long way around.
    target_yaw: f32,
    start_pitch: f32,
    target_pitch: f32,
    elapsed: f32,
}

impl DeathEffect {
    /// Whether a kill / fatal-fall effect is currently running — see the
    /// `active` field.
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    #[cfg(test)]
    pub(crate) fn set_active_for_test(&mut self, on: bool) {
        self.active = on;
    }
}

pub(crate) struct DeathEffectPlugin;

impl Plugin for DeathEffectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DeathEffect>();
        app.add_systems(
            Update,
            (
                on_killed_by,
                pan_to_killer.after(on_killed_by),
                clear_on_killcam_or_respawn,
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// True while the kill-specific pan onto the killer is active — not a fatal
/// fall's own pan (see `fall_death::effect_active`), which drives its own
/// continuous look-at instead of this module's `pan_to_killer`.
/// [`crate::look_around`] defers to this (and to `fall_death::effect_active`)
/// so mouse input can't fight either forced look-at.
pub(crate) fn death_effect_active(effect: Res<DeathEffect>) -> bool {
    effect.pan.is_some()
}

/// Wrap `to - from` into `(-PI, PI]` — the short way around, whichever
/// direction that is.
fn shortest_delta(from: f32, to: f32) -> f32 {
    let diff = (to - from).rem_euclid(TAU);
    if diff > std::f32::consts::PI {
        diff - TAU
    } else {
        diff
    }
}

/// Spawns the blood/red-tint overlay (if not already up) and instantly hides
/// whichever weapon is currently drawn — same mechanism (a bare
/// `Visibility::Hidden`, no animation) as the throwing-knife key's own
/// instant hide of the sniper (`weapon::weapon_system`). Shared by a kill
/// (`on_killed_by`) and a fatal fall (`fall_death::check_fall_death`), so both
/// look identical and clean up through the same [`clear_on_killcam_or_respawn`].
pub(crate) fn show_overlay_and_hide_weapon(
    effect: &mut DeathEffect,
    commands: &mut Commands,
    asset_server: &AssetServer,
    overlay_already_up: bool,
    sniper_vis: &mut Visibility,
    knife_vis: &mut Visibility,
) {
    effect.active = true;

    if !overlay_already_up {
        commands
            .spawn((
                DeathOverlay,
                GlobalZIndex(10),
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.45, 0.0, 0.0, 0.35)),
            ))
            .with_children(|root| {
                root.spawn((
                    ImageNode::new(asset_server.load("textures/blur-blood-splatter-overlay.png")),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                ));
            });
    }

    // Guarded so a second call (shouldn't happen, but defensively) doesn't
    // stomp a still-set `hidden_weapon` before it's been consumed.
    if effect.hidden_weapon.is_none() {
        if *sniper_vis != Visibility::Hidden {
            *sniper_vis = Visibility::Hidden;
            effect.hidden_weapon = Some(HiddenWeapon::Sniper);
        } else if *knife_vis != Visibility::Hidden {
            *knife_vis = Visibility::Hidden;
            effect.hidden_weapon = Some(HiddenWeapon::Knife);
        }
    }
}

fn on_killed_by(
    mut receivers: Query<&mut MessageReceiver<PlayerKilledBy>>,
    mut effect: ResMut<DeathEffect>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    player: Single<&Transform, (With<Player>, Without<PlayerHead>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<Player>)>,
    existing_overlay: Query<(), With<DeathOverlay>>,
    mut sniper_vis: Single<&mut Visibility, (With<ViewModel>, Without<KnifeViewModel>)>,
    mut knife_vis: Single<&mut Visibility, (With<KnifeViewModel>, Without<ViewModel>)>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let killer_pos = Vec3::from_array(msg.killer_pos);
            let direction = (killer_pos - player.translation).normalize_or_zero();

            let (start_yaw, _, _) = player.rotation.to_euler(EulerRot::YXZ);
            let (_, start_pitch, _) = head.rotation.to_euler(EulerRot::YXZ);
            // Degenerate case (killer exactly at our own eye position): hold
            // the current look direction rather than skip the effect.
            let (target_yaw, target_pitch) = if direction == Vec3::ZERO {
                (start_yaw, start_pitch)
            } else {
                let (yaw, pitch, _) = Transform::IDENTITY
                    .looking_to(direction, Vec3::Y)
                    .rotation
                    .to_euler(EulerRot::YXZ);
                (yaw, pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT))
            };
            effect.pan = Some(Pan {
                start_yaw,
                target_yaw: start_yaw + shortest_delta(start_yaw, target_yaw),
                start_pitch,
                target_pitch,
                elapsed: 0.0,
            });

            show_overlay_and_hide_weapon(
                &mut effect,
                &mut commands,
                &asset_server,
                !existing_overlay.is_empty(),
                &mut **sniper_vis,
                &mut **knife_vis,
            );
        }
    }
}

fn pan_to_killer(
    time: Res<Time>,
    mut effect: ResMut<DeathEffect>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    let Some(pan) = effect.pan.as_mut() else {
        return;
    };
    pan.elapsed = (pan.elapsed + time.delta_secs()).min(PAN_SECS);
    let t = ease(pan.elapsed / PAN_SECS);
    player.rotation = Quat::from_rotation_y(pan.start_yaw.lerp(pan.target_yaw, t));
    head.rotation = Quat::from_rotation_x(pan.start_pitch.lerp(pan.target_pitch, t));
}

/// Clear the pan + despawn the overlay the moment the kill cam actually
/// starts, or — the fallback path, if one never arrives in time (see
/// `net::flush_pending_respawn`'s own timeout) — the moment the player
/// respawns anyway. Either way this is the *only* place the overlay comes
/// down; the kill cam itself is never touched.
fn clear_on_killcam_or_respawn(
    active: Res<ActiveKillCam>,
    mut respawned: EventReader<LocalPlayerRespawned>,
    mut effect: ResMut<DeathEffect>,
    mut commands: Commands,
    overlay: Query<Entity, With<DeathOverlay>>,
    mut sniper_vis: Single<&mut Visibility, (With<ViewModel>, Without<KnifeViewModel>)>,
    mut knife_vis: Single<&mut Visibility, (With<KnifeViewModel>, Without<ViewModel>)>,
) {
    let killcam_started = active.is_changed() && active.0.is_some();
    let just_respawned = respawned.read().count() > 0;
    if !effect.active || !(killcam_started || just_respawned) {
        return;
    }
    effect.active = false;
    effect.pan = None;
    for entity in &overlay {
        commands.entity(entity).try_despawn();
    }
    // If a kill cam is starting instead, leave `hidden_weapon` alone —
    // `killcam::start_killcam` reads and clears it itself (see that
    // function), so there's no race over which of us gets there first.
    if !killcam_started {
        match effect.hidden_weapon.take() {
            Some(HiddenWeapon::Sniper) => **sniper_vis = Visibility::Inherited,
            Some(HiddenWeapon::Knife) => **knife_vis = Visibility::Inherited,
            None => {}
        }
    }
}
