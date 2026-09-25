//! The local player's health, *as the server reports it*. Health is entirely
//! server-side (`server::pvp`): shots and falls both take it off
//! `PlayerCombat::health`, it holds for a few seconds after damage and then
//! regenerates, and the result is replicated as [`shared::PlayerHealth`]. The
//! client only displays it — the red tint and blood splatter overlay (more
//! opaque the lower the health) and the looping heartbeat (loudest at low
//! health, fading as it recovers, gone at full) — and never changes it. The
//! fall-damage rules themselves live in `shared::health`.

use bevy::audio::Volume;
use bevy::prelude::*;
use lightyear::prelude::*;

use shared::health::FULL_HEALTH;
use shared::{PlayerHealth, PlayerId};

use crate::death_effect::DeathEffect;
use crate::fall_death::FallDeathState;
use crate::killcam::ActiveKillCam;
use crate::net::GameClient;
use crate::{AppState, GameSounds, SoundVolumes};

/// Red tint alpha at zero health (fully opaque overlay) — the same base tint
/// the death overlay uses (`death_effect::show_overlay_and_hide_weapon`).
const TINT_ALPHA: f32 = 0.35;

/// The local player's health as last replicated by the server (display-only).
#[derive(Resource)]
pub(crate) struct LocalHealth {
    pub(crate) health: f32,
}

impl Default for LocalHealth {
    fn default() -> Self {
        Self {
            health: FULL_HEALTH,
        }
    }
}

impl LocalHealth {
    /// `0.0` at full health … `1.0` at none — the overlay's opacity and the
    /// heartbeat's loudness.
    pub(crate) fn hurt_fraction(&self) -> f32 {
        (1.0 - self.health / FULL_HEALTH).clamp(0.0, 1.0)
    }
}

pub(crate) struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalHealth>()
            .add_systems(
                OnEnter(AppState::InGame),
                (reset_health, spawn_damage_overlay),
            )
            .add_systems(
                Update,
                (reset_on_respawn, mirror_health, sync_damage_overlay, update_heartbeat)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// A fresh game starts at full health until the server's first update lands.
fn reset_health(mut health: ResMut<LocalHealth>) {
    *health = LocalHealth::default();
}

/// Respawning is a fresh start: full health at once — the display would
/// otherwise keep showing whatever the player died with (a full-strength red
/// tint) until the server's next update lands.
fn reset_on_respawn(
    mut respawned: EventReader<crate::net::LocalPlayerRespawned>,
    mut health: ResMut<LocalHealth>,
) {
    if respawned.read().count() > 0 {
        *health = LocalHealth::default();
    }
}

/// Track the server's replicated health for our own player.
fn mirror_health(
    local: Query<&LocalId, With<GameClient>>,
    players: Query<(&PlayerId, &PlayerHealth)>,
    mut health: ResMut<LocalHealth>,
) {
    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    if let Some((_, server)) = players.iter().find(|(id, _)| id.0 == me) {
        if health.health != server.0 {
            health.health = server.0;
        }
    }
}

/// Whether the hurt overlay / heartbeat should stay quiet: the player is dead
/// or dying, or a kill cam is playing the world back.
fn suppressed(death: &DeathEffect, fall: &FallDeathState, killcam: &ActiveKillCam) -> bool {
    death.is_active() || fall.effect_running() || killcam.0.is_some()
}

/// Root of the damage tint + blood overlay.
#[derive(Component)]
struct DamageOverlay;

/// The blood-splatter image under [`DamageOverlay`].
#[derive(Component)]
struct DamageBlood;

/// The looping heartbeat.
#[derive(Component)]
struct Heartbeat;

fn spawn_damage_overlay(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands
        .spawn((
            DamageOverlay,
            StateScoped(AppState::InGame),
            // Under the death overlay (10), which takes over on death.
            GlobalZIndex(9),
            Visibility::Hidden,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.45, 0.0, 0.0, 0.0)),
        ))
        .with_child((
            DamageBlood,
            ImageNode::new(asset_server.load("textures/hud/blood_overlay.png"))
                .with_color(Color::srgba(1.0, 1.0, 1.0, 0.0)),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ));
}

/// Tint + blood splatter, each as opaque as the player is hurt: barely there
/// at high health, fully opaque near zero, gone at full health.
fn sync_damage_overlay(
    health: Res<LocalHealth>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    killcam: Res<ActiveKillCam>,
    mut root: Single<(&mut Visibility, &mut BackgroundColor), With<DamageOverlay>>,
    mut blood: Single<&mut ImageNode, With<DamageBlood>>,
) {
    let opacity = health.hurt_fraction();
    // Dead (health at or below zero — waiting on the kill cam / respawn) isn't
    // "hurt": the death overlay covers that, and this one must not be left up
    // at full strength across the respawn.
    let show = opacity > 0.0 && health.health > 0.0 && !suppressed(&death, &fall, &killcam);
    let (vis, tint) = &mut *root;
    vis.set_if_neq(if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    tint.0 = Color::srgba(0.45, 0.0, 0.0, TINT_ALPHA * opacity);
    blood.color = Color::srgba(1.0, 1.0, 1.0, opacity);
}

/// The heartbeat plays (looping) whenever health is below full — that's only
/// ever after damage — loudest when health is lowest, fading as it recovers
/// and stopping entirely at full. A fatal fall never starts it (health hits
/// zero, which stops / never starts it), and it's muted while dying or while a
/// replay plays.
fn update_heartbeat(
    health: Res<LocalHealth>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    killcam: Res<ActiveKillCam>,
    sounds: Res<GameSounds>,
    vols: Res<SoundVolumes>,
    global_volume: Res<GlobalVolume>,
    mut beats: Query<(Entity, &mut AudioSink), With<Heartbeat>>,
    have_beat: Query<(), With<Heartbeat>>,
    mut commands: Commands,
) {
    let muted = suppressed(&death, &fall, &killcam);
    if health.health >= FULL_HEALTH || health.health <= 0.0 {
        for (e, _) in &beats {
            commands.entity(e).try_despawn();
        }
        return;
    }
    if have_beat.is_empty() {
        if !muted {
            commands.spawn((
                Heartbeat,
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.heartbeat.clone()),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
            ));
        }
        return;
    }
    let loudness = if muted {
        0.0
    } else {
        health.hurt_fraction() * vols.heartbeat
    };
    for (_, mut sink) in &mut beats {
        sink.set_volume(Volume::Linear(loudness.max(0.0)) * global_volume.volume);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respawning_restores_full_health_display_immediately() {
        let mut app = App::new();
        app.add_event::<crate::net::LocalPlayerRespawned>()
            .insert_resource(LocalHealth { health: -8.0 })
            .add_systems(Update, reset_on_respawn);
        app.update();
        assert_eq!(app.world().resource::<LocalHealth>().health, -8.0, "not until a respawn");
        app.world_mut().send_event(crate::net::LocalPlayerRespawned);
        app.update();
        let h = app.world().resource::<LocalHealth>();
        assert_eq!(h.health, FULL_HEALTH);
        assert_eq!(h.hurt_fraction(), 0.0);
    }

    #[test]
    fn zero_health_is_hurt_fraction_one_which_is_why_dead_hides_the_overlay() {
        assert_eq!(LocalHealth { health: 0.0 }.hurt_fraction(), 1.0);
        assert_eq!(LocalHealth { health: -30.0 }.hurt_fraction(), 1.0);
    }
}
