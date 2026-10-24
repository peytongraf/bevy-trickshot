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
//! A Nuke doesn't kill at once: [`NUKE_KILL_DELAY_SECS`] after the grab
//! each zombie up gets a [`NukeDeath`] — a random moment within
//! [`NUKE_KILL_SECS`] — and [`run_nuke_deaths`] drops
//! them one by one, telling every client ([`ZombieNuked`]) to set the body
//! alight.
//!
//! A Carpenter doesn't fix the windows at once either: [`run_carpenters`]
//! puts a board back every [`CARPENTER_BOARD_SECS`], nearest window to the
//! drop first, straight onto `Lobby::windows_torn` — so every client sees
//! and hears each one fly in like a player's — and pays everyone
//! [`CARPENTER_POINTS`] once they're all up.
//!
//! Everything resets with the game: drops are removed once their lobby's
//! game isn't running, and the timers and Carpenters with it
//! (`Lobby::active_power_ups` is also cleared when a game starts or ends); a
//! [`NukeDeath`] lives on its zombie, which `zombies::cull_zombies` removes
//! with the game.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::bot_players::is_bot_peer;
use shared::map::CollisionWorld;
use shared::power_ups::{
    drop_spot, in_pickup_range, roll, PowerUp, BLINK_SECS, BONUS_POINTS, CARPENTER_BOARD_SECS, CARPENTER_POINTS,
    DROP_LIFETIME_SECS, NUKE_KILL_DELAY_SECS, NUKE_KILL_SECS, NUKE_POINTS, NUKE_SPAWN_PAUSE_SECS, TIMED_SECS,
};
use shared::{
    DropPowerUp, GameChannel, GameMode, Lobby, PlayerId, PlayerPose, PowerUpDrop, PowerUpGrabbed, ScoreLine,
    TrickScore, ZombieNuked, ZombiesDoomed,
};

use crate::lobby::LobbyPlayer;
use crate::pvp::{PlayerCombat, ZombieKilled};
use crate::sim::EYE_HEIGHT;
use crate::zombies::{Zombie, ZombieRounds};

/// The server's side of one [`PowerUpDrop`].
#[derive(Component)]
pub(crate) struct DropSim {
    pub(crate) lobby: Entity,
    /// Seconds it's been lying there (not counting pauses).
    pub(crate) age: f32,
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

/// Each lobby's drop that's owed: one a zombie rolled while still outside
/// its window (out of reach), held over for the next zombie killed inside
/// to drop instead — like CoD, which never wastes a drop on the far side
/// of a barrier.
#[derive(Resource, Default)]
struct PendingDrops(HashMap<Entity, PowerUp>);

/// A Carpenter at work in a lobby ([`run_carpenters`]).
struct CarpenterJob {
    /// The map's windows, nearest the drop first — fixed in that order.
    order: Vec<usize>,
    /// Seconds (not counting pauses) till the next board goes back.
    wait: f32,
    /// Carpenters grabbed while it's been at work — each pays out once
    /// it's done.
    owed: u32,
}

/// Nukes grabbed whose zombies haven't started dying yet: each lobby's
/// seconds (not counting pauses) till they do ([`NUKE_KILL_DELAY_SECS`]),
/// and where the drop was — [`fire_nukes`].
#[derive(Resource, Default)]
struct PendingNukes(HashMap<Entity, (f32, Vec3)>);

/// Each lobby's running Carpenter, if any.
#[derive(Resource, Default)]
struct Carpenters(HashMap<Entity, CarpenterJob>);

pub struct PowerUpsPlugin;

impl Plugin for PowerUpsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PowerUpClocks>()
            .init_resource::<PendingDrops>()
            .init_resource::<Carpenters>()
            .init_resource::<PendingNukes>()
            .add_observer(on_drop_power_up)
            .add_systems(
            FixedUpdate,
            (roll_drops, run_power_ups, fire_nukes, run_nuke_deaths, run_carpenters)
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
/// a dog round, whose only drop is its last dog's Max Ammo —
/// `zombies::run_rounds`) — or, if a player's close enough to grab it there at once,
/// just out of their reach ([`drop_spot`]). One rolled by a zombie still
/// outside its window waits in [`PendingDrops`] for the next kill inside.
fn roll_drops(
    time: Res<Time>,
    mut kills: EventReader<ZombieKilled>,
    lobbies: Query<&Lobby>,
    rounds: Query<&ZombieRounds>,
    colliders: Res<crate::collision::MapColliders>,
    players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, Option<&PlayerCombat>), Without<Zombie>>,
    mut pending: ResMut<PendingDrops>,
    mut commands: Commands,
) {
    // Forget what's owed in games that aren't running.
    pending.0.retain(|e, _| {
        lobbies
            .get(*e)
            .is_ok_and(|l| l.started && l.mode == GameMode::Zombies)
    });
    for (i, ev) in kills.read().enumerate() {
        let Ok(lobby) = lobbies.get(ev.lobby) else {
            continue;
        };
        if !lobby.started || lobby.mode != GameMode::Zombies {
            continue;
        }
        // A dog round drops nothing but its last dog's Max Ammo. (A dog
        // outside an exfil is always a dog round's — even when the last
        // one's kill is read after the round's already moved on.)
        if rounds.get(ev.lobby).is_ok_and(|r| {
            r.exfil.is_none() && (shared::dogs::is_dog_round(r.round) || ev.anim.is_dog())
        }) {
            // (And nothing owed from the round before turns up in it.)
            pending.0.remove(&ev.lobby);
            continue;
        }
        let seed = time.elapsed().as_nanos() as u64
            ^ (ev.feet.x.to_bits() as u64) << 32
            ^ ev.feet.z.to_bits() as u64
            ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        // One owed from a kill outside a window comes before a fresh roll.
        let owed = if ev.outside { None } else { pending.0.remove(&ev.lobby) };
        let torn = shared::windows::total_torn(lobby.map, &lobby.windows_torn);
        let Some(kind) = owed.or_else(|| roll(shared::bots::rand01(seed), lobby.power_up_test, torn)) else {
            continue;
        };
        if ev.outside {
            pending.0.entry(ev.lobby).or_insert(kind);
            continue;
        }
        // (Who could grab it — the same players `run_power_ups` lets.)
        let feet: Vec<Vec3> = players
            .iter()
            .filter(|(id, _, lp, combat)| {
                lp.lobby == ev.lobby && !is_bot_peer(id.0) && combat.is_none_or(|c| c.alive)
            })
            .map(|(_, pose, ..)| pose.translation - Vec3::Y * EYE_HEIGHT)
            .collect();
        let pos = drop_spot(ev.feet, &feet, &colliders.for_lobby(lobby));
        commands.spawn((
            Name::from("PowerUpDrop"),
            PowerUpDrop {
                kind,
                pos,
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

/// How far (m) in front of the leader a debug drop with `ahead` lands —
/// brought in short of any wall.
const DEBUG_DROP_AHEAD: f32 = 4.0;

/// Debug: the leader drops a power-up at their own feet (picked up at once,
/// so it goes off for real), or [`DEBUG_DROP_AHEAD`] in front of them on the
/// ground there — only in a running, unpaused `Zombies` game.
fn on_drop_power_up(
    trigger: Trigger<RemoteTrigger<DropPowerUp>>,
    endings: Res<crate::killcam::EndingLobbies>,
    colliders: Res<crate::collision::MapColliders>,
    lobbies: Query<(Entity, &Lobby)>,
    players: Query<(&PlayerId, &PlayerPose)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let DropPowerUp { kind, ahead } = trigger.trigger;
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
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    let pos = if ahead {
        let world = colliders.for_lobby(lobby);
        let chest = feet + Vec3::Y;
        let facing = Quat::from_rotation_y(pose.yaw) * Vec3::NEG_Z;
        // Out to the full distance, or as far short of a wall as it takes,
        // then down onto the ground there.
        (1..=8)
            .rev()
            .map(|k| chest + facing * DEBUG_DROP_AHEAD * k as f32 / 8.0)
            .find(|&at| !world.segment_blocked(chest, at))
            .and_then(|at| world.raycast(at, Vec3::NEG_Y, 4.0).map(|hit| at - Vec3::Y * hit.distance))
            .unwrap_or(feet)
    } else {
        feet
    };
    commands.spawn((
        Name::from("PowerUpDrop"),
        PowerUpDrop {
            kind,
            pos,
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
    mut carpenters: ResMut<Carpenters>,
    mut nukes: ResMut<PendingNukes>,
    mut lobbies: Query<(Entity, &mut Lobby, Option<&mut ZombieRounds>)>,
    players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, Option<&PlayerCombat>), Without<Zombie>>,
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
            award(&mut sender, server, lobby, peer, label, points);
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
                // Its zombies start dying a moment later (`fire_nukes`) —
                // a second one before then changes nothing.
                nukes.0.entry(lobby_e).or_insert((NUKE_KILL_DELAY_SECS, drop.pos));
                // No more rise till they've dropped (if the round goes on at
                // all — clearing it starts the usual break instead).
                if let Some(mut rounds) = rounds {
                    rounds.next_spawn_at = rounds
                        .next_spawn_at
                        .max(now + NUKE_KILL_DELAY_SECS + NUKE_SPAWN_PAUSE_SECS);
                }
                let points = if double { NUKE_POINTS * 2 } else { NUKE_POINTS };
                let everyone: Vec<PeerId> = lobby.members.iter().map(|m| m.peer).collect();
                for peer in everyone {
                    award(&mut lobby, peer, "NUKE", points);
                }
            }
            PowerUp::BonusPoints => award(&mut lobby, by, "BONUS POINTS", BONUS_POINTS),
            PowerUp::Carpenter => {
                let windows = shared::windows::windows(lobby.map);
                let mut order: Vec<usize> = (0..windows.len()).collect();
                let from = drop.pos;
                order.sort_by(|&a, &b| {
                    windows[a].opening().0.distance_squared(from).total_cmp(&windows[b].opening().0.distance_squared(from))
                });
                // Another while one's at work: it starts over from this
                // drop, and pays out for both.
                let owed = carpenters.0.get(&lobby_e).map_or(0, |j| j.owed);
                carpenters.0.insert(lobby_e, CarpenterJob { order, wait: 0.0, owed: owed + 1 });
                lobby.carpenter = true;
            }
        }
        let msg = PowerUpGrabbed { kind, by, pos: drop.pos };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(real)) {
            error!("failed to send power-up grab: {e:?}");
        }
        quotes.write(crate::quotes::SayQuote {
            delay: crate::quotes::QuoteDelay::Announcer(kind),
            ..crate::quotes::SayQuote::anyone(lobby_e, shared::quotes::Quote::PowerUp(kind))
        });
        info!("{by:?} grabbed {}", kind.label());
    }
}

/// Score `points` for `peer` in `lobby`, and show them the `label` line.
fn award(
    sender: &mut ServerMultiMessageSender,
    server: &Server,
    lobby: &mut Lobby,
    peer: PeerId,
    label: &str,
    points: u32,
) {
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
}

/// Put each running Carpenter's next boards back (holding still while its
/// game is paused or ending): the first window in its order still missing
/// any gets one. None missing: everyone's paid [`CARPENTER_POINTS`] for each
/// Carpenter it owes, and it's done. Games that aren't running forget theirs.
fn run_carpenters(
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut carpenters: ResMut<Carpenters>,
    mut lobbies: Query<&mut Lobby>,
) {
    let dt = time.delta_secs();
    let server = server.into_inner();
    carpenters.0.retain(|lobby_e, job| {
        let Ok(mut lobby) = lobbies.get_mut(*lobby_e) else {
            return false;
        };
        if !lobby.started || lobby.mode != GameMode::Zombies {
            return false;
        }
        if lobby.paused || endings.is_ending(*lobby_e) {
            return true;
        }
        job.wait -= dt;
        while job.wait <= 0.0 {
            job.wait += CARPENTER_BOARD_SECS;
            let next = job
                .order
                .iter()
                .copied()
                .find(|&i| shared::windows::boards_left(&lobby.windows_torn, i) < shared::windows::BOARDS);
            let Some(i) = next else {
                let points = CARPENTER_POINTS
                    * job.owed
                    * if lobby.power_up_active(PowerUp::DoublePoints) { 2 } else { 1 };
                let everyone: Vec<PeerId> = lobby.members.iter().map(|m| m.peer).collect();
                for peer in everyone {
                    award(&mut sender, server, &mut lobby, peer, "CARPENTER", points);
                }
                lobby.carpenter = false;
                info!("lobby {lobby_e:?}: carpenter done");
                return false;
            };
            // (Torn below `BOARDS`, so there's an entry.)
            if let Some(torn) = lobby.windows_torn.get_mut(i) {
                *torn = torn.saturating_sub(1);
            }
        }
        true
    });
}

/// Set off each Nuke whose moment's come (holding still while its game is
/// paused or ending): every zombie up is doomed, each dying at its own
/// random moment over the next [`NUKE_KILL_SECS`] (`run_nuke_deaths`). One already
/// doomed by an earlier Nuke keeps its moment. Games that aren't running
/// forget theirs.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
fn fire_nukes(
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut nukes: ResMut<PendingNukes>,
    lobbies: Query<&Lobby>,
    // (A boss, `shared::boss`, shrugs a Nuke off.)
    zombies: Query<
        (Entity, &PlayerId, &LobbyPlayer, &PlayerCombat, Has<NukeDeath>),
        (With<Zombie>, Without<crate::boss::Boss>),
    >,
    poses: Query<&PlayerPose>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let server = server.into_inner();
    nukes.0.retain(|&lobby_e, (left, at)| {
        let Ok(lobby) = lobbies.get(lobby_e) else {
            return false;
        };
        if !lobby.started || lobby.mode != GameMode::Zombies {
            return false;
        }
        if lobby.paused || endings.is_ending(lobby_e) {
            return true;
        }
        *left -= dt;
        if *left > 0.0 {
            return true;
        }
        let victims: Vec<_> = zombies
            .iter()
            .filter(|(_, _, lp, combat, already)| lp.lobby == lobby_e && combat.alive && !already)
            .collect();
        // The one nearest where it went off burns first, at once; the rest
        // at random moments after.
        let first = victims
            .iter()
            .min_by(|a, b| {
                let d = |z: Entity| poses.get(z).map_or(f32::INFINITY, |p| p.translation.distance_squared(*at));
                d(a.0).total_cmp(&d(b.0))
            })
            .map(|v| v.0);
        let mut doomed = Vec::new();
        for &(zombie_e, id, ..) in &victims {
            let seed = (now.to_bits() as u64) << 32 ^ zombie_e.to_bits().wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let left = if Some(zombie_e) == first { 0.0 } else { shared::bots::rand01(seed) * NUKE_KILL_SECS };
            commands.entity(zombie_e).insert(NukeDeath { left });
            doomed.push(id.0);
        }
        info!("lobby {lobby_e:?}: nuke dooms {} zombies", doomed.len());
        if !doomed.is_empty() {
            let msg = ZombiesDoomed { peers: doomed };
            if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(lobby.real_peers())) {
                error!("failed to send zombies doomed: {e:?}");
            }
        }
        false
    });
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
