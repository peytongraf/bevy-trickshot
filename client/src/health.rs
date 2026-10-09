//! The local player's health, *as the server reports it*. Health is entirely
//! server-side (`server::pvp`): shots and falls both take it off
//! `PlayerCombat::health`, it holds for a few seconds after damage and then
//! regenerates, and the result is replicated as [`shared::PlayerHealth`]. The
//! client only displays it — the red tint and blood splatter overlay (more
//! opaque the lower the health) and the looping heartbeat (loudest at low
//! health, fading as it recovers, gone at full) — and never changes it. The
//! fall-damage rules themselves live in `shared::health`.
//!
//! How hard both hit is [`HurtEffectSettings`] (the debug panel's "Hurt
//! effects" section): against the player's real maximum health (more with
//! Juggernog in `Zombies`), they're there from the first point lost and at
//! full strength by [`HurtEffectSettings::full_at`] — half health. The
//! damage indicator (`hud::damage_indicator`) tunes there too.

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

/// How the hurt effects look and sound — the debug panel's "Hurt effects"
/// section (`debug_ui`).
#[derive(Resource, Clone)]
pub(crate) struct HurtEffectSettings {
    /// At full strength: the blood splatter's opacity, and the red tint's.
    pub(crate) blood_opacity: f32,
    pub(crate) tint_opacity: f32,
    /// How it comes on, as a fraction of the player's max health: it starts
    /// once health drops below `start_at`, is at full strength by `full_at`,
    /// and `curve` shapes it in between (under 1: strong early).
    pub(crate) start_at: f32,
    pub(crate) full_at: f32,
    pub(crate) curve: f32,
    /// The heartbeat at full strength (× the "heartbeat" sound volume), and
    /// how much of that it already has the moment it starts.
    pub(crate) heartbeat_volume: f32,
    pub(crate) heartbeat_min: f32,
    /// The damage indicator (`hud::damage_indicator`): how long one shows
    /// (s), the last stretch of which (s) it fades over; how far out from
    /// the middle of the screen it sits and how wide it is (× the screen's
    /// height); and its opacity.
    pub(crate) indicator_secs: f32,
    pub(crate) indicator_fade_secs: f32,
    pub(crate) indicator_radius: f32,
    pub(crate) indicator_size: f32,
    pub(crate) indicator_opacity: f32,
}

impl Default for HurtEffectSettings {
    fn default() -> Self {
        Self {
            blood_opacity: 0.2,
            tint_opacity: 0.55,
            start_at: 1.0,
            full_at: 0.5,
            curve: 0.6,
            heartbeat_volume: 2.0,
            heartbeat_min: 0.35,
            indicator_secs: 2.5,
            indicator_fade_secs: 1.2,
            indicator_radius: 0.32,
            indicator_size: 0.22,
            indicator_opacity: 0.9,
        }
    }
}

impl HurtEffectSettings {
    /// How strong the hurt effects are at `health` of `max`: `0` unhurt …
    /// `1` at (or below) [`Self::full_at`].
    pub(crate) fn strength(&self, health: f32, max: f32) -> f32 {
        if health >= max || max <= 0.0 {
            return 0.0;
        }
        let frac = health / max;
        let t = ((self.start_at - frac) / (self.start_at - self.full_at).max(1e-3)).clamp(0.0, 1.0);
        t.powf(self.curve.max(0.05))
    }
}

/// The local player's health as last replicated by the server (display-only),
/// and the most it can be (more with Juggernog in `Zombies`).
#[derive(Resource)]
pub(crate) struct LocalHealth {
    pub(crate) health: f32,
    pub(crate) max: f32,
}

impl Default for LocalHealth {
    fn default() -> Self {
        Self {
            health: FULL_HEALTH,
            max: FULL_HEALTH,
        }
    }
}

pub(crate) struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalHealth>()
            .init_resource::<HurtEffectSettings>()
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

/// Track the server's replicated health for our own player, and how much
/// we can have (`Zombies`' Juggernog raises it).
fn mirror_health(
    local: Query<&LocalId, With<GameClient>>,
    players: Query<(&PlayerId, &PlayerHealth)>,
    lobbies: Query<&shared::Lobby>,
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
    let max = lobbies
        .iter()
        .find(|l| l.has(me) && l.mode == shared::GameMode::Zombies)
        .and_then(|l| l.members.iter().find(|m| m.peer == me))
        .map_or(FULL_HEALTH, |m| shared::perks::max_health(&m.perks));
    if health.max != max {
        health.max = max;
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
    settings: Res<HurtEffectSettings>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    killcam: Res<ActiveKillCam>,
    revive: Res<crate::revive::LocalRevive>,
    spectating: Res<crate::revive::Spectate>,
    mut root: Single<(&mut Visibility, &mut BackgroundColor), With<DamageOverlay>>,
    mut blood: Single<&mut ImageNode, With<DamageBlood>>,
) {
    // Down in `Zombies`: it fades in as the bleed-out runs (full once out).
    let down = revive.downed || revive.bled_out;
    let opacity = if revive.bled_out {
        1.0
    } else if revive.downed {
        0.15 + 0.85 * revive.progress
    } else {
        settings.strength(health.health, health.max)
    };
    // Dead (health at or below zero — waiting on the kill cam / respawn) isn't
    // "hurt": the death overlay covers that, and this one must not be left up
    // at full strength across the respawn.
    // (Not over a teammate we're watching.)
    let show = opacity > 0.0
        && (health.health > 0.0 || down)
        && !suppressed(&death, &fall, &killcam)
        && !spectating.active();
    let (vis, tint) = &mut *root;
    vis.set_if_neq(if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    tint.0 = Color::srgba(0.45, 0.0, 0.0, (settings.tint_opacity * opacity).clamp(0.0, 1.0));
    blood.color = Color::srgba(1.0, 1.0, 1.0, (settings.blood_opacity * opacity).clamp(0.0, 1.0));
}

/// The heartbeat plays (looping) whenever health is below full — that's only
/// ever after damage, in every mode — already clear from the first hit
/// (`HurtEffectSettings::heartbeat_min`), loudest when health is lowest,
/// fading as it recovers and stopping entirely at full. A fatal fall never starts it (health hits
/// zero, which stops / never starts it), and it's muted while dying or while a
/// replay plays.
fn update_heartbeat(
    health: Res<LocalHealth>,
    settings: Res<HurtEffectSettings>,
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
    let strength = settings.strength(health.health, health.max);
    if health.health >= health.max || health.health <= 0.0 {
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
        let min = settings.heartbeat_min.clamp(0.0, 1.0);
        (min + (1.0 - min) * strength) * settings.heartbeat_volume * vols.heartbeat
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
            .insert_resource(LocalHealth { health: -8.0, max: FULL_HEALTH })
            .add_systems(Update, reset_on_respawn);
        app.update();
        assert_eq!(app.world().resource::<LocalHealth>().health, -8.0, "not until a respawn");
        app.world_mut().send_event(crate::net::LocalPlayerRespawned);
        app.update();
        let h = app.world().resource::<LocalHealth>();
        assert_eq!(h.health, FULL_HEALTH);
        assert_eq!(HurtEffectSettings::default().strength(h.health, h.max), 0.0);
    }

    #[test]
    fn the_hurt_effect_is_there_from_the_first_hit_and_full_by_half_health() {
        let s = HurtEffectSettings::default();
        assert_eq!(s.strength(100.0, 100.0), 0.0);
        assert!(s.strength(95.0, 100.0) > 0.1, "barely hurt still shows");
        assert!(s.strength(80.0, 100.0) > 0.5, "a hit or two is plain");
        assert_eq!(s.strength(50.0, 100.0), 1.0);
        assert_eq!(s.strength(0.0, 100.0), 1.0);
        // Juggernog: 120 of 150 is hurt, though over 100.
        assert!(s.strength(120.0, 150.0) > 0.3);
        assert_eq!(s.strength(75.0, 150.0), 1.0);
    }
}
