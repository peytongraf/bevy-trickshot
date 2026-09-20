//! Player-vs-player combat for [`shared::GameMode::FreeForAll`]: health,
//! death, respawn, and the kill-limit win condition. Bots (`crate::bots`)
//! and style-point scoring (`shared::scoring`) are `Freestyle`-only and
//! untouched by any of this.

use bevy::prelude::*;

use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{
    FellToDeath, GameChannel, GameMode, HitMarker, Lobby, PlayerId, PlayerKilledBy, PlayerPose,
    PlayerRespawn,
};

/// Every player starts (and respawns) at this much health. The sniper's
/// 100-damage body shot is a one-shot kill against it — the same feel it
/// already had before PvP existed; the marksman needs two hits (or one
/// headshot).
pub const FULL_HEALTH: f32 = 100.0;

/// Seconds a dead player stays untargetable / unable to fire before they can
/// be hit again — generous enough to outlast the kill-cam the victim's own
/// client plays (buffered ~1.5 s post-kill, then up to ~4.5 s of replay; see
/// `server::killcam`).
const RESPAWN_DELAY_SECS: f32 = 4.5;

/// Seconds after taking damage that a player's health holds before it starts
/// to recover — the same beat the fall-damage health uses client-side.
const REGEN_DELAY_SECS: f32 = 3.0;
/// Health regained per second once recovery starts.
const REGEN_PER_SEC: f32 = 20.0;

/// Per-player combat state. Only present on players in a `FreeForAll` game
/// (see `lobby::on_start`) — `Freestyle` players never get one.
#[derive(Component)]
pub struct PlayerCombat {
    pub health: f32,
    /// `Time::elapsed_secs()` of the last damage taken — health holds for
    /// [`REGEN_DELAY_SECS`] after it, then recovers (see [`tick_respawns`]).
    last_damage: f32,
    pub alive: bool,
    /// `Time::elapsed_secs()` this player becomes targetable / can fire again.
    /// Meaningless while `alive`.
    respawn_at: f32,
}

impl Default for PlayerCombat {
    fn default() -> Self {
        Self {
            health: FULL_HEALTH,
            last_damage: f32::NEG_INFINITY,
            alive: true,
            respawn_at: 0.0,
        }
    }
}

/// A `FreeForAll` player took damage — written by
/// [`crate::sim::resolve_shots`]. A fatal hit additionally fires
/// [`PlayerKilled`] for [`crate::killcam::queue_killcams`] to pick up.
#[derive(Event)]
pub struct PlayerHit {
    pub victim: PeerId,
    pub killer: PeerId,
    pub damage: f32,
}

/// A `FreeForAll` kill — the PvP counterpart of [`crate::bots::BotHit`].
/// Written by [`apply_player_hits`], consumed by
/// [`crate::killcam::queue_killcams`] to queue the victim-only replay.
#[derive(Event)]
pub struct PlayerKilled {
    pub victim: PeerId,
    pub killer: PeerId,
}

pub struct PvpPlugin;

impl Plugin for PvpPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<PlayerHit>()
            .add_event::<PlayerKilled>()
            .add_observer(on_fell_to_death)
            .add_systems(
                FixedUpdate,
                (apply_player_hits, tick_respawns, check_kill_limit).chain(),
            );
    }
}

/// Apply queued damage; a fatal hit marks the victim dead, starts their
/// respawn timer, credits the killer's kill count, and tells the victim
/// where they'll reappear.
#[allow(clippy::too_many_arguments)]
fn apply_player_hits(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut hits: EventReader<PlayerHit>,
    mut killed: EventWriter<PlayerKilled>,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    mut lobbies: Query<&mut Lobby>,
) {
    let server = server.into_inner();
    for ev in hits.read() {
        let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == ev.victim) else {
            continue;
        };
        if !combat.alive {
            continue;
        }
        combat.health -= ev.damage;
        combat.last_damage = time.elapsed_secs();
        if combat.health > 0.0 {
            // Hurt but alive: the shooter gets a hit marker.
            if let Err(e) =
                sender.send::<_, GameChannel>(&HitMarker, server, &NetworkTarget::Single(ev.killer))
            {
                error!("failed to send hit marker to {:?}: {e:?}", ev.killer);
            }
            continue;
        }
        combat.alive = false;
        combat.respawn_at = time.elapsed_secs() + RESPAWN_DELAY_SECS;

        let Some(mut lobby) = lobbies.iter_mut().find(|l| l.has(ev.victim)) else {
            continue;
        };
        if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == ev.killer) {
            m.score += 1;
        }

        let others: Vec<Vec3> = poses
            .iter()
            .filter(|(id, _)| id.0 != ev.victim && lobby.has(id.0))
            .map(|(_, pose)| pose.translation)
            .collect();
        let seed = time.elapsed().as_nanos() as u64 ^ ev.victim.to_bits();
        let (pos, yaw) = shared::spawns::spawn_point(seed, &others, lobby.map);

        // Sent immediately, well ahead of the kill cam (buffered ~1.5s server-side —
        // see `PlayerKilledBy`'s doc comment) so the victim's own death effect can
        // snap their view toward the killer right away.
        if let Some((_, killer_pose)) = poses.iter().find(|(id, _)| id.0 == ev.killer) {
            let killed_by = PlayerKilledBy {
                killer_pos: killer_pose.translation.to_array(),
            };
            if let Err(e) =
                sender.send::<_, GameChannel>(&killed_by, server, &NetworkTarget::Single(ev.victim))
            {
                error!("failed to send killed-by to {:?}: {e:?}", ev.victim);
            }
        }

        let msg = PlayerRespawn {
            pos: pos.to_array(),
            yaw,
        };
        if let Err(e) =
            sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(ev.victim))
        {
            error!("failed to send respawn to {:?}: {e:?}", ev.victim);
        }

        info!("{:?} killed {:?}", ev.killer, ev.victim);
        killed.write(PlayerKilled {
            victim: ev.victim,
            killer: ev.killer,
        });
    }
}

/// The local player's client (client-authoritative movement — same trust
/// model as `PlayerInput`, see `sim::apply_client_pose`) reported falling to
/// their death (see `client::fall_death`). Mirrors `apply_player_hits`'s
/// respawn handling, minus the score credit and `PlayerKilled` — there's no
/// killer, so no kill cam is ever queued. Works in both game modes:
/// `FreeForAll` players have a `PlayerCombat` to mark dead (respecting an
/// already-in-progress death — e.g. shot moments before this arrives — the
/// same way `apply_player_hits` does); `Freestyle` players have no health
/// concept at all (see `PlayerCombat`'s doc comment) and are always "alive",
/// so they just get sent a fresh spot with nothing to mark.
fn on_fell_to_death(
    trigger: Trigger<RemoteTrigger<FellToDeath>>,
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    mut lobbies: Query<&mut Lobby>,
) {
    let server = server.into_inner();
    let peer = trigger.from;
    if let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == peer) {
        if !combat.alive {
            return;
        }
        combat.alive = false;
        combat.respawn_at = time.elapsed_secs() + RESPAWN_DELAY_SECS;
    }

    let Some(lobby) = lobbies.iter_mut().find(|l| l.has(peer)) else {
        return;
    };
    let others: Vec<Vec3> = poses
        .iter()
        .filter(|(id, _)| id.0 != peer && lobby.has(id.0))
        .map(|(_, pose)| pose.translation)
        .collect();
    let seed = time.elapsed().as_nanos() as u64 ^ peer.to_bits();
    let (pos, yaw) = shared::spawns::spawn_point(seed, &others, lobby.map);

    let msg = PlayerRespawn {
        pos: pos.to_array(),
        yaw,
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(peer)) {
        error!("failed to send respawn to {:?}: {e:?}", peer);
    }

    info!("{:?} fell to their death", peer);
}

/// Once a dead player's respawn timer is up, make them targetable / able to
/// fire again. The actual reposition is client-driven (see
/// `client::net::flush_pending_respawn`) — this only ungates hit detection.
fn tick_respawns(time: Res<Time>, mut combats: Query<&mut PlayerCombat>) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    for mut combat in &mut combats {
        if !combat.alive && now >= combat.respawn_at {
            combat.alive = true;
            combat.health = FULL_HEALTH;
        } else if combat.alive
            && combat.health < FULL_HEALTH
            && now - combat.last_damage >= REGEN_DELAY_SECS
        {
            // Hurt but not dead: hold, then climb back linearly.
            combat.health = (combat.health + REGEN_PER_SEC * dt).min(FULL_HEALTH);
        }
    }
}

/// End a `FreeForAll` match the instant someone reaches the lobby's kill
/// limit, rather than waiting for the clock (see `lobby::end_match`).
fn check_kill_limit(
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut best_plays: ResMut<crate::killcam::BestPlays>,
) {
    let server = server.into_inner();
    for (lobby_e, mut lobby) in &mut lobbies {
        if !lobby.started || lobby.mode != GameMode::FreeForAll {
            continue;
        }
        if lobby.members.iter().any(|m| m.score >= lobby.kill_limit) {
            crate::lobby::end_match(lobby_e, &mut lobby, server, &mut sender, &mut best_plays);
        }
    }
}
