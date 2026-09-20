//! The local player's health, for fall damage: landing from far enough up
//! hurts in proportion to how far past the minimum the drop was, and the
//! maximum drop kills outright (`fall_death`). A hurt player sees the red tint
//! and blood splatter overlay — more opaque the lower their health — and hears
//! a heartbeat that's loudest at low health and fades as they recover; after
//! taking damage their health holds for a few seconds, then builds back up
//! linearly.
//!
//! Health is client-local. It only ever changes from falls (the sniper's
//! bullets are one-shot kills, so `FreeForAll`'s server-side `PlayerCombat`
//! health never sits at a partial value); death from a fall still goes through
//! the existing `shared::FellToDeath` path. Reset to full on entering a game
//! and on respawning.

use bevy::audio::Volume;
use bevy::prelude::*;

use crate::death_effect::DeathEffect;
use crate::fall_death::FallDeathState;
use crate::killcam::ActiveKillCam;
use crate::net::LocalPlayerRespawned;
use crate::{AppState, GameSounds, SoundVolumes};

/// Full health.
pub(crate) const MAX_HEALTH: f32 = 100.0;

/// Red tint alpha at zero health (fully opaque overlay) — the same base tint
/// the death overlay uses (`death_effect::show_overlay_and_hide_weapon`).
const TINT_ALPHA: f32 = 0.35;

/// Panel-adjustable fall damage + recovery (the "Health & fall damage"
/// section).
#[derive(Resource)]
pub(crate) struct FallDamageSettings {
    /// A fall (apex → landing, metres) shorter than this does no damage.
    pub(crate) min_distance: f32,
    /// A fall this far or farther kills. Between `min_distance` and this,
    /// damage rises linearly from nothing to a full health bar.
    pub(crate) max_distance: f32,
    /// Seconds after taking damage that health holds before it starts to
    /// recover.
    pub(crate) regen_delay_secs: f32,
    /// Health (of [`MAX_HEALTH`]) regained per second once recovery starts.
    pub(crate) regen_per_sec: f32,
}

impl Default for FallDamageSettings {
    fn default() -> Self {
        Self {
            min_distance: 16.0,
            max_distance: 30.0,
            regen_delay_secs: 3.0,
            regen_per_sec: 20.0,
        }
    }
}

impl FallDamageSettings {
    /// Health lost landing from a fall of `distance` metres — `0` at or below
    /// `min_distance`, [`MAX_HEALTH`] at or above `max_distance`, linear in
    /// between.
    pub(crate) fn damage_for(&self, distance: f32) -> f32 {
        let span = (self.max_distance - self.min_distance).max(1e-3);
        (((distance - self.min_distance) / span).clamp(0.0, 1.0)) * MAX_HEALTH
    }
}

/// The local player's health.
#[derive(Resource)]
pub(crate) struct PlayerHealth {
    pub(crate) health: f32,
    /// Seconds since health last dropped.
    since_damage: f32,
}

impl Default for PlayerHealth {
    fn default() -> Self {
        Self {
            health: MAX_HEALTH,
            since_damage: f32::MAX,
        }
    }
}

impl PlayerHealth {
    /// Take `amount` damage and restart the hold-then-recover timer. Returns
    /// whether the player is still alive.
    pub(crate) fn damage(&mut self, amount: f32) -> bool {
        if amount > 0.0 {
            self.health = (self.health - amount).max(0.0);
            self.since_damage = 0.0;
        }
        self.health > 0.0
    }

    /// `0.0` at full health … `1.0` at none — the overlay's opacity and the
    /// heartbeat's loudness.
    pub(crate) fn hurt_fraction(&self) -> f32 {
        (1.0 - self.health / MAX_HEALTH).clamp(0.0, 1.0)
    }
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

pub(crate) struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerHealth>()
            .init_resource::<FallDamageSettings>()
            .add_systems(
                OnEnter(AppState::InGame),
                (reset_health, spawn_damage_overlay),
            )
            .add_systems(
                Update,
                (
                    reset_on_respawn,
                    regen_health,
                    sync_damage_overlay,
                    update_heartbeat,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn reset_health(mut health: ResMut<PlayerHealth>) {
    *health = PlayerHealth::default();
}

fn reset_on_respawn(mut respawned: EventReader<LocalPlayerRespawned>, mut health: ResMut<PlayerHealth>) {
    if respawned.read().count() > 0 {
        *health = PlayerHealth::default();
    }
}

/// Whether the hurt overlay / heartbeat should stay quiet: the player is dead
/// or dying, or a kill cam is playing the world back.
fn suppressed(death: &DeathEffect, fall: &FallDeathState, killcam: &ActiveKillCam) -> bool {
    death.is_active() || fall.effect_running() || killcam.0.is_some()
}

/// Hold for [`FallDamageSettings::regen_delay_secs`] after the last damage,
/// then climb back to full at [`FallDamageSettings::regen_per_sec`].
fn regen_health(
    time: Res<Time>,
    settings: Res<FallDamageSettings>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    killcam: Res<ActiveKillCam>,
    mut health: ResMut<PlayerHealth>,
) {
    if suppressed(&death, &fall, &killcam) || health.health >= MAX_HEALTH {
        return;
    }
    let dt = time.delta_secs();
    health.since_damage = (health.since_damage + dt).min(1.0e6);
    if health.since_damage > settings.regen_delay_secs {
        health.health = (health.health + settings.regen_per_sec * dt).min(MAX_HEALTH);
    }
}

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
            ImageNode::new(asset_server.load("textures/blur-blood-splatter-overlay.png"))
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
    health: Res<PlayerHealth>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    killcam: Res<ActiveKillCam>,
    mut root: Single<(&mut Visibility, &mut BackgroundColor), With<DamageOverlay>>,
    mut blood: Single<&mut ImageNode, With<DamageBlood>>,
) {
    let opacity = health.hurt_fraction();
    let show = opacity > 0.0 && !suppressed(&death, &fall, &killcam);
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
    health: Res<PlayerHealth>,
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
    if health.health >= MAX_HEALTH || health.health <= 0.0 {
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
    fn fall_damage_is_zero_under_min_linear_after_and_full_at_max() {
        let s = FallDamageSettings {
            min_distance: 6.0,
            max_distance: 12.0,
            ..default()
        };
        assert_eq!(s.damage_for(3.0), 0.0);
        assert_eq!(s.damage_for(6.0), 0.0);
        assert!((s.damage_for(9.0) - 50.0).abs() < 1e-3);
        assert!((s.damage_for(12.0) - MAX_HEALTH).abs() < 1e-3);
        assert!((s.damage_for(40.0) - MAX_HEALTH).abs() < 1e-3);
    }

    #[test]
    fn max_fall_kills_and_smaller_ones_do_not() {
        let s = FallDamageSettings::default();
        let mut h = PlayerHealth::default();
        let mid = (s.min_distance + s.max_distance) * 0.5;
        assert!(h.damage(s.damage_for(mid)), "a mid fall shouldn't kill");
        assert!(h.health > 0.0 && h.health < MAX_HEALTH);
        let mut h = PlayerHealth::default();
        assert!(!h.damage(s.damage_for(s.max_distance)), "the max fall kills");
        assert_eq!(h.health, 0.0);
    }

    #[test]
    fn hurt_fraction_is_the_opposite_of_health() {
        let mut h = PlayerHealth::default();
        assert_eq!(h.hurt_fraction(), 0.0);
        h.damage(75.0);
        assert!((h.hurt_fraction() - 0.75).abs() < 1e-3);
    }
}
