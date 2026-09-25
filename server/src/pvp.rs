//! Player health, death and respawn — the server owns all of it. Shots
//! (`FreeForAll` PvP) and falls (both modes) take health off
//! [`PlayerCombat::health`]; it's replicated to the clients as
//! [`shared::PlayerHealth`] for the damage overlay and heartbeat, holds for a
//! few seconds after damage and then regenerates. This module also owns the
//! `FreeForAll` kill-limit win condition. Bots (`crate::bots`) and style-point
//! scoring (`shared::scoring`) are `Freestyle`-only and untouched by any of
//! this.

use bevy::prelude::*;

use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{
    FallDeath, FallLanded, FellToDeath, GameChannel, GameMode, HitMarker, Lobby, PlayerHealth,
    PlayerId, PlayerKilledBy, PlayerPose, PlayerRespawn, RespawnReady, ScoreLine, TrickScore,
    ZOMBIE_KILL_POINTS,
};

use shared::bot_players::is_bot_peer;
use shared::health::{
    fall_damage, FULL_HEALTH, REGEN_DELAY_SECS, REGEN_PER_SEC,
};

/// Seconds a dead player stays untargetable / unable to fire before they can
/// be hit again — generous enough to outlast the kill-cam the victim's own
/// client plays (buffered ~1.5 s post-kill, then up to ~4.5 s of replay; see
/// `server::killcam`).
const RESPAWN_DELAY_SECS: f32 = 4.5;

/// Seconds a player who died *falling* (no kill cam) stays dead — just past
/// the client's own wait before it respawns them
/// (`client::net::RESPAWN_KILLCAM_TIMEOUT_SECS`, 3 s), so they're never
/// respawned-but-still-dead for long.
const FALL_RESPAWN_DELAY_SECS: f32 = 3.2;

/// Per-player combat state, on every player in a started game (see
/// `lobby::on_start`) in either mode.
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

impl PlayerCombat {
    /// Back to life: full health, targetable, no damage history — everything a
    /// respawn resets. Shared by the respawn timer and the client's
    /// `RespawnReady`.
    fn respawn(&mut self) {
        *self = Self::default();
    }
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
            .add_observer(on_fall_landed)
            .add_observer(on_respawn_ready)
            .add_systems(
                FixedUpdate,
                (
                    apply_player_hits,
                    tick_respawns,
                    sync_health,
                    check_kill_limit,
                )
                    .chain(),
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
    mut lobbies: Query<(Entity, &mut Lobby)>,
    (mut endings, clock): (
        ResMut<crate::killcam::EndingLobbies>,
        Res<crate::killcam::ReplayClock>,
    ),
) {
    let server = server.into_inner();
    for ev in hits.read() {
        // (A `Zombies` zombie isn't a lobby member — its lobby is its victim's
        // or its killer's.)
        let in_lobby = |l: &Lobby| l.has(ev.victim) || l.has(ev.killer);
        // The match is over: nothing counts (see `EndingLobbies`).
        if lobbies
            .iter()
            .any(|(e, l)| in_lobby(l) && (endings.is_frozen(e) || endings.is_ending(e) && l.mode == GameMode::Zombies))
        {
            continue;
        }
        let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == ev.victim) else {
            continue;
        };
        if !combat.alive {
            continue;
        }
        // Liquid Courage (a `Zombies` perk) softens every hit.
        let perks: &[shared::perks::Perk] = lobbies
            .iter()
            .find_map(|(_, l)| l.members.iter().find(|m| m.peer == ev.victim))
            .map_or(&[], |m| m.perks.as_slice());
        combat.health -= shared::perks::damage_taken(perks, ev.damage);
        combat.last_damage = time.elapsed_secs();
        if combat.health > 0.0 {
            // Hurt but alive: the shooter gets a hit marker (a bot shooter has
            // no client to show it to).
            if !is_bot_peer(ev.killer) {
                if let Err(e) = sender.send::<_, GameChannel>(
                    &HitMarker,
                    server,
                    &NetworkTarget::Single(ev.killer),
                ) {
                    error!("failed to send hit marker to {:?}: {e:?}", ev.killer);
                }
            }
            continue;
        }
        combat.alive = false;
        combat.respawn_at = time.elapsed_secs() + RESPAWN_DELAY_SECS;

        let Some((lobby_e, mut lobby)) = lobbies.iter_mut().find(|(_, l)| in_lobby(l)) else {
            continue;
        };

        // `Zombies`: nobody respawns. A dead zombie scores its killer and is
        // cleared away by `crate::zombies`; a dead *player* ends the game for
        // everyone. No kill cams either way.
        if lobby.mode == GameMode::Zombies {
            combat.respawn_at = f32::INFINITY;
            if is_bot_peer(ev.victim) {
                if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == ev.killer) {
                    m.score += ZOMBIE_KILL_POINTS;
                    m.kills += 1;
                    let trick = TrickScore {
                        shooter: ev.killer,
                        total: ZOMBIE_KILL_POINTS,
                        lines: vec![ScoreLine {
                            label: "KILL".into(),
                            points: ZOMBIE_KILL_POINTS,
                        }],
                    };
                    if let Err(e) = sender.send::<_, GameChannel>(
                        &trick,
                        server,
                        &NetworkTarget::Single(ev.killer),
                    ) {
                        error!("failed to send zombie kill score: {e:?}");
                    }
                }
            } else {
                info!("{:?} was killed — zombies game over", ev.victim);
                endings.begin(lobby_e, clock.0, false);
            }
            continue;
        }
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

        // A bot victim has no client: nothing to tell, and `ai::drive_bots`
        // respawns it itself when its timer is up.
        let victim_is_bot = is_bot_peer(ev.victim);

        // Sent immediately, well ahead of the kill cam (buffered ~1.5s server-side —
        // see `PlayerKilledBy`'s doc comment) so the victim's own death effect can
        // snap their view toward the killer right away.
        if let Some((_, killer_pose)) = poses
            .iter()
            .find(|(id, _)| id.0 == ev.killer && !victim_is_bot)
        {
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
            immediate: false,
        };
        if !victim_is_bot {
            if let Err(e) =
                sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(ev.victim))
            {
                error!("failed to send respawn to {:?}: {e:?}", ev.victim);
            }
        }

        info!("{:?} killed {:?}", ev.killer, ev.victim);
        killed.write(PlayerKilled {
            victim: ev.victim,
            killer: ev.killer,
        });
    }
}

/// The local player's client reported falling out of the world (below the
/// void floor — see `client::fall_death`; movement is client-authoritative,
/// same trust model as `PlayerInput`, see `sim::apply_client_pose`). There's no
/// height to grade, so it's a straight kill — see [`fall_kill`].
fn on_fell_to_death(
    trigger: Trigger<RemoteTrigger<FellToDeath>>,
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    (mut endings, clock): (
        ResMut<crate::killcam::EndingLobbies>,
        Res<crate::killcam::ReplayClock>,
    ),
) {
    fall_kill(
        trigger.from,
        None,
        &time,
        server.into_inner(),
        &mut sender,
        &mut combats,
        &poses,
        &mut lobbies,
        (&mut endings, clock.0),
    );
    info!("{:?} fell out of the world", trigger.from);
}

/// The local player's client reported landing after a fall of some height
/// ([`shared::FallLanded`]). The *server* turns the distance into damage
/// ([`fall_damage`]): under the minimum nothing happens; over it, health drops
/// (and is replicated back for the damage overlay + heartbeat); if that kills,
/// [`fall_kill`] runs and the victim is told to play the fall-death effect.
/// Works in both game modes — every player has a [`PlayerCombat`].
#[allow(clippy::too_many_arguments)]
fn on_fall_landed(
    trigger: Trigger<RemoteTrigger<FallLanded>>,
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    (mut endings, clock): (
        ResMut<crate::killcam::EndingLobbies>,
        Res<crate::killcam::ReplayClock>,
    ),
) {
    let peer = trigger.from;
    let landed = trigger.trigger;
    // Liquid Courage (a `Zombies` perk) softens falls too.
    let perks: &[shared::perks::Perk] = lobbies
        .iter()
        .find_map(|(_, l)| l.members.iter().find(|m| m.peer == peer))
        .map_or(&[], |m| m.perks.as_slice());
    let damage = shared::perks::damage_taken(perks, fall_damage(landed.distance));
    if damage <= 0.0 {
        return;
    }
    let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == peer) else {
        return;
    };
    if !combat.alive {
        return;
    }
    combat.health = (combat.health - damage).max(0.0);
    combat.last_damage = time.elapsed_secs();
    let dead = combat.health <= 0.0;
    drop(combat);
    if dead {
        fall_kill(
            peer,
            Some(landed.speed),
            &time,
            server.into_inner(),
            &mut sender,
            &mut combats,
            &poses,
            &mut lobbies,
            (&mut endings, clock.0),
        );
        info!("{peer:?} died from a {:.1} m fall", landed.distance);
    } else {
        info!("{peer:?} took {damage:.0} fall damage");
    }
}

/// Kill `peer` from a fall (no killer, so no kill cam is ever queued): mark
/// them dead — respecting a death already in progress, e.g. shot moments
/// before this arrives — start the respawn timer, and send where they'll
/// reappear. If the fall was a graded landing (`speed` is `Some`) the victim
/// is also told to play the fall-death effect ([`shared::FallDeath`]); a void
/// fall already started it client-side.
#[allow(clippy::too_many_arguments)]
fn fall_kill(
    peer: PeerId,
    speed: Option<f32>,
    time: &Time,
    server: &Server,
    sender: &mut ServerMultiMessageSender,
    combats: &mut Query<(&PlayerId, &mut PlayerCombat)>,
    poses: &Query<(&PlayerId, &PlayerPose)>,
    lobbies: &mut Query<(Entity, &mut Lobby)>,
    (endings, clock): (&mut crate::killcam::EndingLobbies, u64),
) {
    let Some((lobby_e, lobby)) = lobbies.iter_mut().find(|(_, l)| l.has(peer)) else {
        return;
    };
    let zombies = lobby.mode == GameMode::Zombies;
    if let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == peer) {
        // Only a player who was already dead is skipped: a landing that just
        // took health to zero arrives here with `alive` still true.
        if !combat.alive {
            return;
        }
        combat.alive = false;
        combat.health = 0.0;
        combat.respawn_at = if zombies {
            f32::INFINITY
        } else {
            time.elapsed_secs() + FALL_RESPAWN_DELAY_SECS
        };
    }

    // `Zombies`: any member dying ends the game — no respawn.
    if zombies {
        if let Some(speed) = speed {
            if let Err(e) = sender.send::<_, GameChannel>(
                &FallDeath { speed },
                server,
                &NetworkTarget::Single(peer),
            ) {
                error!("failed to send fall death to {peer:?}: {e:?}");
            }
        }
        info!("{peer:?} fell to their death — zombies game over");
        endings.begin(lobby_e, clock, false);
        return;
    }
    let others: Vec<Vec3> = poses
        .iter()
        .filter(|(id, _)| id.0 != peer && lobby.has(id.0))
        .map(|(_, pose)| pose.translation)
        .collect();
    let seed = time.elapsed().as_nanos() as u64 ^ peer.to_bits();
    let (pos, yaw) = shared::spawns::spawn_point(seed, &others, lobby.map);

    if let Some(speed) = speed {
        if let Err(e) =
            sender.send::<_, GameChannel>(&FallDeath { speed }, server, &NetworkTarget::Single(peer))
        {
            error!("failed to send fall death to {peer:?}: {e:?}");
        }
    }
    let msg = PlayerRespawn {
        pos: pos.to_array(),
        yaw,
        immediate: false,
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(peer)) {
        error!("failed to send respawn to {peer:?}: {e:?}");
    }
}

/// Copy each player's health onto their replicated [`PlayerHealth`] — only when
/// it changed, so an idle player costs no replication.
fn sync_health(mut players: Query<(&PlayerCombat, &mut PlayerHealth)>) {
    for (combat, mut health) in &mut players {
        let value = combat.health.max(0.0);
        if health.0 != value {
            health.0 = value;
        }
    }
}

/// The player's client has respawned (see [`RespawnReady`]): they're alive now,
/// whatever the timer says. Ignored if they're already alive.
fn on_respawn_ready(
    trigger: Trigger<RemoteTrigger<RespawnReady>>,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
) {
    let peer = trigger.from;
    if let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == peer) {
        if !combat.alive {
            combat.respawn();
            info!("{peer:?} respawned");
        }
    }
}

/// Once a dead player's respawn timer is up, make them targetable / able to
/// fire again. The actual reposition is client-driven (see
/// `client::net::flush_pending_respawn`) — this only ungates hit detection.
fn tick_respawns(
    time: Res<Time>,
    lobbies: Query<&Lobby>,
    mut combats: Query<(&mut PlayerCombat, Option<&crate::lobby::LobbyPlayer>)>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    for (mut combat, lp) in &mut combats {
        // Paused: push the respawn and the regen hold back by the pause, so
        // neither timer runs down while the game's frozen.
        if lp.is_some_and(|lp| lobbies.get(lp.lobby).is_ok_and(|l| l.paused)) {
            combat.respawn_at += dt;
            combat.last_damage += dt;
            continue;
        }
        if !combat.alive && now >= combat.respawn_at {
            combat.respawn();
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
    clock: Res<crate::killcam::ReplayClock>,
    lobbies: Query<(Entity, &Lobby)>,
    mut endings: ResMut<crate::killcam::EndingLobbies>,
) {
    for (lobby_e, lobby) in &lobbies {
        if !lobby.started || lobby.mode != GameMode::FreeForAll {
            continue;
        }
        if lobby.members.iter().any(|m| m.score >= lobby.kill_limit) {
            endings.begin(lobby_e, clock.0, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respawning_restores_everything_a_death_or_damage_changed() {
        let mut c = PlayerCombat {
            health: -12.0,
            last_damage: 40.0,
            alive: false,
            respawn_at: 99.0,
        };
        c.respawn();
        assert!(c.alive);
        assert_eq!(c.health, FULL_HEALTH);
        // No leftover damage timer holding off (or restarting) regeneration.
        assert_eq!(c.last_damage, f32::NEG_INFINITY);
        assert_eq!(c.respawn_at, 0.0);
    }
}

