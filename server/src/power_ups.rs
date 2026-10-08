//! `Zombies` power-ups (see `shared::power_ups`): roll a drop for each
//! zombie a player kills ([`ZombieKilled`]), keep each drop around for its
//! lifetime (blinking near the end), set it off when a living player walks
//! into it, and run the timed ones (Insta-Kill, Double Points) — publishing
//! their seconds left on [`Lobby::active_power_ups`] for the HUD.
//!
//! What each does lives here, except Insta-Kill's and Double Points' effect
//! on a hit / kill, which `crate::pvp::apply_player_hits` reads straight off
//! the lobby, and Max Ammo's refill, which each client does itself on
//! [`PowerUpGrabbed`] (ammo is client-side).
//!
//! A Nuke doesn't kill at once: each zombie up gets a [`NukeDeath`] — a
//! random moment within [`NUKE_KILL_SECS`] — and [`run_nuke_deaths`] drops
//! them one by one, telling every client ([`ZombieNuked`]) to set the body
//! alight.
//!
//! Everything resets with the game: drops are removed once their lobby's
//! game isn't running, and the timers with it (`Lobby::active_power_ups` is
//! also cleared when a game starts or ends); a [`NukeDeath`] lives on its
//! zombie, which `zombies::cull_zombies` removes with the game.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::bot_players::is_bot_peer;
use shared::power_ups::{
    in_pickup_range, roll, PowerUp, BLINK_SECS, BONUS_POINTS, DROP_LIFETIME_SECS, NUKE_KILL_SECS,
    NUKE_POINTS, NUKE_SPAWN_PAUSE_SECS, TIMED_SECS,
};
use shared::{
    DropPowerUp, GameChannel, GameMode, Lobby, PlayerId, PlayerPose, PowerUpDrop, PowerUpGrabbed, ScoreLine,
    TrickScore, ZombieNuked,
};

use crate::lobby::LobbyPlayer;
use crate::pvp::{PlayerCombat, ZombieKilled};
use crate::sim::EYE_HEIGHT;
use crate::zombies::{Zombie, ZombieRounds};

/// The server's side of one [`PowerUpDrop`].
#[derive(Component)]
struct DropSim {
    lobby: Entity,
    /// Seconds it's been lying there (not counting pauses).
    age: f32,
}

/// A zombie a Nuke has doomed: it stands frozen (`ai::drive_bots`) and dies
/// once `left` (seconds, not counting pauses) runs out.
#[derive(Component)]
pub(crate) struct NukeDeath {
    left: f32,
}

/// Each lobby's running timed power-ups: when (`Time::elapsed_secs`) each
/// runs out, in the order they started.
#[derive(Resource, Default)]
struct PowerUpClocks(HashMap<Entity, Vec<(PowerUp, f32)>>);

pub struct PowerUpsPlugin;

impl Plugin for PowerUpsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PowerUpClocks>()
            .add_observer(on_drop_power_up)
            .add_systems(
            FixedUpdate,
            (roll_drops, run_power_ups, run_nuke_deaths)
                .chain()
                .after(crate::pvp::apply_player_hits),
        );
    }
}

/// Drop a `kind` power-up at `pos` in `lobby` (seen by its `peers`) — the
/// same as a zombie dropping one.
pub(crate) fn spawn_drop(commands: &mut Commands, lobby: Entity, peers: Vec<PeerId>, kind: PowerUp, pos: Vec3) {
    commands.spawn((
        Name::from("PowerUpDrop"),
        PowerUpDrop {
            kind,
            pos,
            blinking: false,
        },
        DropSim { lobby, age: 0.0 },
        Replicate::to_clients(NetworkTarget::Only(peers)),
    ));
}

/// Maybe drop a power-up where each zombie a player killed fell (never on
/// a dog round).
fn roll_drops(
    time: Res<Time>,
    mut kills: EventReader<ZombieKilled>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (i, ev) in kills.read().enumerate() {
        let Ok(lobby) = lobbies.get(ev.lobby) else {
            continue;
        };
        if !lobby.started || lobby.mode != GameMode::Zombies {
            continue;
        }
        let seed = time.elapsed().as_nanos() as u64
            ^ (ev.feet.x.to_bits() as u64) << 32
            ^ ev.feet.z.to_bits() as u64
            ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let Some(kind) = roll(shared::bots::rand01(seed), lobby.power_up_test) else {
            continue;
        };
        commands.spawn((
            Name::from("PowerUpDrop"),
            PowerUpDrop {
                kind,
                pos: ev.feet,
                blinking: false,
            },
            DropSim {
                lobby: ev.lobby,
                age: 0.0,
            },
            Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
        ));
        info!("lobby {:?}: a zombie dropped {}", ev.lobby, kind.label());
    }
}

/// Debug: the leader drops a power-up at their own feet (picked up at once,
/// so it goes off for real) — only in a running, unpaused `Zombies` game.
fn on_drop_power_up(
    trigger: Trigger<RemoteTrigger<DropPowerUp>>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<(Entity, &Lobby)>,
    players: Query<(&PlayerId, &PlayerPose)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let kind = trigger.trigger.kind;
    let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| {
        l.leader == peer && l.started && l.mode == GameMode::Zombies && !l.paused
    }) else {
        return;
    };
    if endings.is_ending(lobby_e) {
        return;
    }
    let Some((_, pose)) = players.iter().find(|(id, _)| id.0 == peer) else {
        return;
    };
    commands.spawn((
        Name::from("PowerUpDrop"),
        PowerUpDrop {
            kind,
            pos: pose.translation - Vec3::Y * EYE_HEIGHT,
            blinking: false,
        },
        DropSim { lobby: lobby_e, age: 0.0 },
        Replicate::to_clients(NetworkTarget::Only(lobby.real_peers())),
    ));
    info!("lobby {lobby_e:?}: {peer:?} dropped a {} (debug)", kind.label());
}

/// Tick the timers and the drops; set off any drop a player walked into.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_power_ups(
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut clocks: ResMut<PowerUpClocks>,
    mut lobbies: Query<(Entity, &mut Lobby, Option<&mut ZombieRounds>)>,
    players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, Option<&PlayerCombat>), Without<Zombie>>,
    // (A boss, `shared::boss`, shrugs a Nuke off.)
    zombies: Query<(Entity, &LobbyPlayer, &PlayerCombat, Has<NukeDeath>), (With<Zombie>, Without<crate::boss::Boss>)>,
    mut drops: Query<(Entity, &mut DropSim, &mut PowerUpDrop)>,
    mut commands: Commands,
    mut quotes: EventWriter<crate::quotes::SayQuote>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let server = server.into_inner();

    // Timers: forget games that aren't running, hold still while paused,
    // drop the ones that ran out, and publish whole seconds left.
    clocks.0.retain(|e, _| {
        lobbies
            .get(*e)
            .is_ok_and(|(_, l, _)| l.started && l.mode == GameMode::Zombies)
    });
    for (lobby_e, mut lobby, _) in &mut lobbies {
        let Some(clock) = clocks.0.get_mut(&lobby_e) else {
            if !lobby.active_power_ups.is_empty() {
                lobby.active_power_ups.clear();
            }
            continue;
        };
        if lobby.paused || endings.is_ending(lobby_e) {
            for (_, end) in clock.iter_mut() {
                *end += dt;
            }
        }
        clock.retain(|(_, end)| *end > now);
        publish(&mut lobby, clock, now);
    }

    for (drop_e, mut sim, mut drop) in &mut drops {
        let Ok((lobby_e, mut lobby, rounds)) = lobbies.get_mut(sim.lobby) else {
            commands.entity(drop_e).try_despawn();
            continue;
        };
        if !lobby.started || lobby.mode != GameMode::Zombies {
            commands.entity(drop_e).try_despawn();
            continue;
        }
        // Paused, or the game's ending: it just hangs there.
        if lobby.paused || endings.is_ending(lobby_e) {
            continue;
        }
        sim.age += dt;
        if sim.age >= DROP_LIFETIME_SECS {
            commands.entity(drop_e).try_despawn();
            continue;
        }
        let blinking = sim.age >= DROP_LIFETIME_SECS - BLINK_SECS;
        if drop.blinking != blinking {
            drop.blinking = blinking;
        }

        // The first living player in reach sets it off.
        let Some(by) = players
            .iter()
            .filter(|(id, _, lp, combat)| {
                lp.lobby == lobby_e && !is_bot_peer(id.0) && combat.is_none_or(|c| c.alive)
            })
            .find(|(_, pose, ..)| in_pickup_range(pose.translation - Vec3::Y * EYE_HEIGHT, drop.pos))
            .map(|(id, ..)| id.0)
        else {
            continue;
        };
        commands.entity(drop_e).try_despawn();
        let kind = drop.kind;
        let double = lobby.power_up_active(PowerUp::DoublePoints);
        let real = lobby.real_peers();
        let mut award = |lobby: &mut Lobby, peer: PeerId, label: &str, points: u32| {
            if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
                m.score += points;
            }
            if is_bot_peer(peer) {
                return;
            }
            let trick = TrickScore {
                shooter: peer,
                total: points,
                lines: vec![ScoreLine {
                    label: label.into(),
                    points,
                }],
            };
            if let Err(e) = sender.send::<_, GameChannel>(&trick, server, &NetworkTarget::Single(peer)) {
                error!("failed to send power-up points: {e:?}");
            }
        };
        match kind {
            PowerUp::InstaKill | PowerUp::DoublePoints => {
                let clock = clocks.0.entry(lobby_e).or_default();
                // Again while it's running: the timer starts over, same place.
                match clock.iter_mut().find(|(p, _)| *p == kind) {
                    Some((_, end)) => *end = now + TIMED_SECS,
                    None => clock.push((kind, now + TIMED_SECS)),
                }
                publish(&mut lobby, clock, now);
            }
            // Ammo is client-side: every client fills its own on the message.
            PowerUp::MaxAmmo => {}
            PowerUp::Nuke => {
                // Every zombie up is doomed, each dying at its own random
                // moment over the next few seconds (`run_nuke_deaths`). One
                // already doomed by an earlier Nuke keeps its moment.
                let mut doomed = 0;
                for (zombie_e, lp, combat, already) in &zombies {
                    if lp.lobby == lobby_e && combat.alive && !already {
                        let seed = (now.to_bits() as u64) << 32 ^ zombie_e.to_bits().wrapping_mul(0x9e37_79b9_7f4a_7c15);
                        let left = shared::bots::rand01(seed) * NUKE_KILL_SECS;
                        commands.entity(zombie_e).insert(NukeDeath { left });
                        doomed += 1;
                    }
                }
                // No more rise while they drop (if the round goes on at all
                // — clearing it starts the usual break instead).
                if let Some(mut rounds) = rounds {
                    rounds.next_spawn_at = rounds.next_spawn_at.max(now + NUKE_SPAWN_PAUSE_SECS);
                }
                let points = if double { NUKE_POINTS * 2 } else { NUKE_POINTS };
                let everyone: Vec<PeerId> = lobby.members.iter().map(|m| m.peer).collect();
                for peer in everyone {
                    award(&mut lobby, peer, "NUKE", points);
                }
                info!("lobby {lobby_e:?}: nuke dooms {doomed} zombies");
            }
            PowerUp::BonusPoints => award(&mut lobby, by, "BONUS POINTS", BONUS_POINTS),
        }
        let msg = PowerUpGrabbed { kind, by };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(real)) {
            error!("failed to send power-up grab: {e:?}");
        }
        quotes.write(crate::quotes::SayQuote::anyone(lobby_e, shared::quotes::Quote::PowerUp(kind)));
        info!("{by:?} grabbed {}", kind.label());
    }
}

/// Count down each Nuke-doomed zombie (holding still while its game is
/// paused or ending) and kill it when its moment comes — no points, no drop,
/// like any Nuke kill — telling the lobby so every client sets it alight.
#[allow(clippy::type_complexity)]
fn run_nuke_deaths(
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<&Lobby>,
    mut zombies: Query<(Entity, &mut NukeDeath, &LobbyPlayer, &PlayerId, &mut PlayerCombat), With<Zombie>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let server = server.into_inner();
    for (zombie_e, mut death, lp, id, mut combat) in &mut zombies {
        // (A game that's over takes its zombies with it — `cull_zombies`.)
        let Ok(lobby) = lobbies.get(lp.lobby) else {
            continue;
        };
        if lobby.paused || endings.is_ending(lp.lobby) {
            continue;
        }
        death.left -= dt;
        if death.left > 0.0 {
            continue;
        }
        commands.entity(zombie_e).remove::<NukeDeath>();
        // Someone may have shot it first.
        if !combat.alive {
            continue;
        }
        combat.alive = false;
        combat.health = 0.0;
        combat.respawn_at = f32::INFINITY;
        let msg = ZombieNuked { peer: id.0 };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(lobby.real_peers())) {
            error!("failed to send zombie nuked: {e:?}");
        }
    }
}

/// Put `clock`'s whole seconds left onto the lobby (only when they change,
/// so replication only sends once a second).
fn publish(lobby: &mut Mut<Lobby>, clock: &[(PowerUp, f32)], now: f32) {
    let want: Vec<(PowerUp, u16)> = clock
        .iter()
        .map(|(p, end)| (*p, (end - now).max(0.0).ceil() as u16))
        .collect();
    if lobby.active_power_ups != want {
        lobby.active_power_ups = want;
    }
}
