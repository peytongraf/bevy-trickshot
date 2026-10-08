//! `Zombies`: Call of Duty zombies style co-op rounds against bots.
//!
//! Each round sends a set number of zombies — a few at first, more every
//! round — spawned a handful at a time. A zombie is a `FreeForAll`-style bot
//! player (`ai::BotBrain`, driven by `ai::drive_bots` through the same input
//! path as a real client) that isn't a lobby member: it has a fake bot peer
//! id, only goes after real players, and never respawns. Its skill climbs
//! with the round ([`zombie_skill`]). It rises up out of the ground somewhere
//! a body can stand (a `nav` graph node, so never inside a map object) within
//! [`SPAWN_MIN_DIST`]..[`SPAWN_MAX_DIST`] of a random living member, walkably
//! connected to them. Once a round's zombies are all dead, a short break,
//! then the next round.
//!
//! Every fifth round is a dog round instead (`shared::dogs`): only
//! hellhounds come, each announced by a lightning strike where it'll appear
//! — this queues them ([`PendingDog`]); `crate::dogs` strikes the lightning,
//! brings each one in when it's due and turns their deaths into explosions.
//!
//! Some rounds bring a boss along too (`shared::boss`): this queues it
//! ([`crate::boss::PendingBoss`]) a little way into the round, and
//! `crate::boss` strikes it in and runs its blasts. The round isn't over
//! until it's dead.
//!
//! On rounds 11, 21, 31, ... the party can call the exfil instead (`crate::exfil`):
//! the round's enemies are cleared away and its wave is sent from here, kept
//! topped up to exactly the kills still needed.
//!
//! Scoring and the game ending (any member dying) live in `pvp`; the round
//! number is replicated as `Lobby::round` for the HUD and results screen.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::*;

use shared::bot_players::{bot_peer, is_bot_peer, BotDifficulty, BotSkill};
use shared::bots::rand01;
use shared::exfil::Exfil;
use shared::{
    AmmoBought, BuyAmmo, BuyPap, BuyPerk, GameChannel, GameMode, TurnOnPower, Lobby, PlayerId, PlayerInput,
    PlayerName, PlayerPose, ProneAtPerk, ProneBonus,
};

use crate::ai::{BotBrain, NextBotId};
use crate::lobby::LobbyPlayer;
use crate::nav::NavGraphs;
use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;

/// Nearest / farthest (m) from the chosen player a zombie spawns.
const SPAWN_MIN_DIST: f32 = 12.0;
const SPAWN_MAX_DIST: f32 = 30.0;
/// ...and it never comes up closer than this to *any* living player.
const SPAWN_CLEAR_OF_PLAYERS: f32 = 8.0;
/// Seconds a zombie takes to climb out of the ground.
const RISE_SECS: f32 = shared::zombies::ZOMBIE_RISE_SECS;
/// Seconds before round 1's first zombie, once everyone has loaded in.
const FIRST_ROUND_DELAY_SECS: f32 = 4.0;
/// Seconds between the last zombie of a round dying and the next round.
const ROUND_BREAK_SECS: f32 = 7.0;
/// Seconds a dead zombie lies there (its death animation) before it's removed.
const CORPSE_SECS: f32 = 3.0;
/// Seconds before retrying a spawn that found nowhere to go.
const SPAWN_RETRY_SECS: f32 = 0.25;

/// How many enemies round `round` sends, for `players` members: zombies, or
/// on a dog round dogs.
pub fn enemies_in_round(round: u32, players: usize) -> u32 {
    if shared::dogs::is_dog_round(round) {
        shared::dogs::dogs_in_round(round, players)
    } else {
        zombies_in_round(round, players)
    }
}

/// How many zombies round `round` sends, for `players` members.
pub fn zombies_in_round(round: u32, players: usize) -> u32 {
    let round = round.max(1);
    (4 + 2 * (round - 1) + 2 * (players.max(1) as u32 - 1)).min(60)
}

/// Most zombies up at once: they keep coming (one per [`spawn_interval`])
/// until this many are standing, and the rest of the round waits its turn,
/// each one rising as another dies.
const MAX_ALIVE: usize = 150;

/// Seconds between spawns in `round` — quicker as the rounds go on.
fn spawn_interval(round: u32) -> f32 {
    (2.5 - 0.15 * (round.max(1) - 1) as f32).max(0.6)
}

/// How sharp round `round`'s zombies are: well under a `Recruit` bot at first
/// (slow to notice you, slow to turn), climbing to about a `Veteran` by round
/// 15. They don't shoot (they swipe — `ai::ZombieBody`), and how fast they
/// walk or run is `shared::zombies::zombie_speed`, so the aim / fire / sprint
/// parts of this go unused.
pub fn zombie_skill(round: u32) -> BotSkill {
    let t = ((round.max(1) - 1) as f32 / 14.0).clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    let veteran = BotDifficulty::Veteran.skill();
    BotSkill {
        reaction_secs: lerp(2.2, veteran.reaction_secs),
        aim_error_deg: lerp(12.0, veteran.aim_error_deg),
        fire_interval_secs: lerp(5.0, veteran.fire_interval_secs),
        turn_deg_per_sec: lerp(60.0, veteran.turn_deg_per_sec),
        sight_range: lerp(45.0, veteran.sight_range),
        sprints: false,
    }
}

/// On a zombie's player entity (a dog-round hellhound's too).
#[derive(Component)]
pub struct Zombie;

/// A dog round's hellhound, struck in by lightning but not here yet: it
/// appears at `feet`, facing `yaw`, at `spawn_at` (`Time::elapsed_secs`).
pub(crate) struct PendingDog {
    pub(crate) feet: Vec3,
    pub(crate) yaw: f32,
    pub(crate) spawn_at: f32,
    /// Its lightning's been sent to the lobby (`crate::dogs`).
    pub(crate) announced: bool,
}

/// When a zombie died (`Time::elapsed_secs`), for [`CORPSE_SECS`].
#[derive(Component)]
struct ZombieDeath(f32);

/// A running `Zombies` game's round state, on its lobby entity (server-only;
/// the round number itself is replicated as `Lobby::round`).
#[derive(Component)]
pub(crate) struct ZombieRounds {
    /// `0` during the pre-game countdown (`countdown_left`), then the round
    /// being played.
    pub(crate) round: u32,
    /// A dog round's hellhounds on their way in ([`PendingDog`]).
    pub(crate) pending_dogs: Vec<PendingDog>,
    /// Where this round's last hellhound died (`crate::dogs`) — a Max Ammo
    /// drops there once the dog round's over.
    pub(crate) last_dog_at: Option<Vec3>,
    /// A boss round's bosses still to strike in, and when the next one's
    /// due ([`shared::boss::BOSS_ROUND_DELAY_SECS`] into the round)...
    bosses_to_spawn: u32,
    boss_due_at: f32,
    /// The exfil, once it's called (`crate::exfil`) — this round's last.
    pub(crate) exfil: Option<crate::exfil::ExfilRun>,
    /// ...and the ones struck in but not here yet (`crate::boss`; the
    /// leader's debug button queues them here too).
    pub(crate) pending_bosses: Vec<crate::boss::PendingBoss>,
    /// Seconds of the pre-game countdown still to go (`Lobby::countdown_secs`
    /// at the start): the party roams, buys and turns the power on, and no
    /// zombie comes until it's out.
    countdown_left: f32,
    /// Zombies this round still has to send.
    to_spawn: u32,
    /// (A Nuke pushes this back — `crate::power_ups`.)
    pub(crate) next_spawn_at: f32,
    /// No spawning until then (the start delay / the break between rounds).
    break_until: f32,
    /// Perk machines whose prone bonus has been claimed this game
    /// ([`on_prone_at_perk`]) — goes with this component when the game ends.
    prone_claimed: Vec<shared::perks::Perk>,
}

impl ZombieRounds {
    /// The round its zombies and dogs are as tough as: the round's own, or
    /// a few above it for an exfil's wave.
    pub(crate) fn enemy_round(&self) -> u32 {
        self.round + if self.exfil.is_some() { shared::exfil::ROUND_BOOST } else { 0 }
    }
}

pub struct ZombiesPlugin;

impl Plugin for ZombiesPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_buy_perk)
            .add_observer(on_buy_pap)
            .add_observer(on_turn_on_power)
            .add_observer(on_buy_ammo)
            .add_observer(on_prone_at_perk)
            .add_systems(
                FixedUpdate,
                (run_rounds, clear_dead_zombies, cull_zombies).before(crate::ai::drive_bots),
            )
            .add_systems(
                FixedUpdate,
                (
                    publish_zombie_anims.after(crate::sim::apply_client_pose),
                    announce_zombie_swipes.after(crate::ai::drive_bots),
                ),
            );
    }
}

/// Start, advance and feed every running `Zombies` game.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn run_rounds(
    time: Res<Time>,
    navs: Res<NavGraphs>,
    clock: Res<crate::killcam::ReplayClock>,
    mut endings: ResMut<crate::killcam::EndingLobbies>,
    mut next_id: ResMut<NextBotId>,
    mut kills: EventReader<crate::pvp::ZombieKilled>,
    mut lobbies: Query<(Entity, &mut Lobby, Option<&mut ZombieRounds>)>,
    players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &PlayerCombat)>,
    zombies: Query<(Entity, &PlayerId, &LobbyPlayer, &PlayerCombat, Has<crate::boss::Boss>), With<Zombie>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    // This tick's kills, for an exfil to count (read here, with the deaths
    // they go with, so the two always agree).
    let kills: Vec<(Entity, PeerId, PeerId)> = kills.read().map(|k| (k.lobby, k.killer, k.victim)).collect();
    for (lobby_e, mut lobby, rounds) in &mut lobbies {
        if !lobby.started || lobby.mode != GameMode::Zombies {
            if rounds.is_some() {
                commands.entity(lobby_e).remove::<ZombieRounds>();
            }
            if lobby.countdown_left != 0 {
                lobby.countdown_left = 0;
            }
            continue;
        }
        // Game over (`pvp` started the ending), or still loading in.
        if endings.is_ending(lobby_e) || !lobby.members.iter().all(|m| m.loaded) {
            continue;
        }
        let members = lobby.real_count();
        // Paused: push every pending time back by the pause, so the break /
        // next spawn is exactly as far off when it resumes.
        if lobby.paused {
            if let Some(mut rounds) = rounds {
                rounds.break_until += time.delta_secs();
                rounds.next_spawn_at += time.delta_secs();
                for dog in &mut rounds.pending_dogs {
                    dog.spawn_at += time.delta_secs();
                }
                rounds.boss_due_at += time.delta_secs();
                for boss in &mut rounds.pending_bosses {
                    boss.spawn_at += time.delta_secs();
                }
                if let Some(exfil) = rounds.exfil.as_mut() {
                    exfil.pause(time.delta_secs());
                }
            }
            continue;
        }
        let Some(mut rounds) = rounds else {
            // Everyone's in: the pre-game countdown (if the leader set one)
            // starts now; the first round follows it.
            let countdown = lobby.countdown_secs.min(shared::zombies::MAX_COUNTDOWN_SECS);
            lobby.countdown_left = countdown;
            commands.entity(lobby_e).insert(ZombieRounds {
                round: 0,
                pending_dogs: Vec::new(),
                last_dog_at: None,
                bosses_to_spawn: 0,
                boss_due_at: now,
                pending_bosses: Vec::new(),
                exfil: None,
                countdown_left: countdown as f32,
                to_spawn: 0,
                next_spawn_at: now,
                break_until: now,
                prone_claimed: Vec::new(),
            });
            if countdown > 0 {
                info!("lobby {lobby_e:?}: zombies starts in {countdown} s");
            }
            continue;
        };
        if rounds.round == 0 {
            rounds.countdown_left -= time.delta_secs();
            let shown = rounds.countdown_left.max(0.0).ceil() as u32;
            if lobby.countdown_left != shown {
                lobby.countdown_left = shown;
            }
            if rounds.countdown_left > 0.0 {
                continue;
            }
            // The leader can start on a later round (`Lobby::start_round`).
            // (After a countdown the party's ready: no extra delay.)
            let first = lobby.start_round.max(1);
            rounds.round = first;
            rounds.to_spawn = enemies_in_round(first, members);
            rounds.break_until = now + if lobby.countdown_secs > 0 { 0.0 } else { FIRST_ROUND_DELAY_SECS };
            queue_round_bosses(&mut rounds, members);
            lobby.round = first;
            lobby.enemies_left = rounds.to_spawn + rounds.bosses_to_spawn;
            info!("lobby {lobby_e:?}: zombies round {first}");
            continue;
        }
        // (Its fields borrowed apart below.)
        let rounds: &mut ZombieRounds = &mut rounds;
        // An exfil called: its own wave instead of the round's (see
        // `crate::exfil`).
        if let Some(exfil) = rounds.exfil.as_mut() {
            // The kills from inside the area count.
            if let Some((_, area)) = shared::exfil::layout(lobby.map) {
                for &(_, killer, victim) in kills.iter().filter(|(l, ..)| *l == lobby_e) {
                    let inside = players
                        .iter()
                        .find(|(id, ..)| id.0 == killer)
                        .is_some_and(|(_, pose, ..)| area.contains(pose.translation - Vec3::Y * EYE_HEIGHT));
                    let boss = zombies.iter().any(|(_, id, _, _, boss)| id.0 == victim && boss);
                    exfil.count_kill(inside, boss);
                }
            }
            // Half-way into the white-out: the round's enemies go.
            if exfil.wants_clear(now) {
                for (e, _, lp, ..) in &zombies {
                    if lp.lobby == lobby_e {
                        commands.entity(e).try_despawn();
                    }
                }
                exfil.clear_round(&mut rounds.to_spawn);
                rounds.pending_dogs.clear();
                rounds.pending_bosses.clear();
                rounds.bosses_to_spawn = 0;
                rounds.last_dog_at = None;
                lobby.enemies_left = 0;
                lobby.enemies_active = 0;
                continue;
            }
            let exfil = rounds.exfil.as_mut().expect("checked above");
            if exfil.begin_if_due(now).is_some() {
                rounds.next_spawn_at = now;
                rounds.boss_due_at = now;
                rounds.break_until = now;
                info!("lobby {lobby_e:?}: the exfil wave is on");
            }
            if !exfil.begun() {
                continue;
            }
            let exfil = rounds.exfil.as_ref().expect("checked above");
            let state = if exfil.kills_left == 0 {
                Exfil::Escaped
            } else if exfil.timed_out(now) {
                Exfil::Failed
            } else {
                Exfil::Active { secs_left: exfil.secs_left(now) }
            };
            if lobby.exfil != state {
                lobby.exfil = state;
            }
            if lobby.enemies_left != exfil.kills_left {
                lobby.enemies_left = exfil.kills_left;
            }
            if matches!(state, Exfil::Escaped | Exfil::Failed) {
                info!("lobby {lobby_e:?}: exfil {state:?} on round {}", rounds.round);
                endings.begin(lobby_e, clock.0, false);
                continue;
            }
            // Exactly as many on their way as kills still needed.
            let mine = || zombies.iter().filter(|(_, _, lp, c, _)| lp.lobby == lobby_e && c.alive);
            let alive_bosses = mine().filter(|z| z.4).count();
            let alive = mine().count();
            let bosses_there = (alive_bosses + rounds.pending_bosses.len()) as u32;
            let others_there = (alive - alive_bosses + rounds.pending_dogs.len()) as u32;
            let (bosses, others) = exfil.top_up(bosses_there, others_there);
            rounds.bosses_to_spawn = bosses;
            rounds.to_spawn = others;
            if lobby.enemies_active != alive as u32 {
                lobby.enemies_active = alive as u32;
            }
        } else {
            if now < rounds.break_until {
                continue;
            }
            let alive = zombies
                .iter()
                .filter(|(_, _, lp, c, _)| lp.lobby == lobby_e && c.alive)
                .count();
            // A dog round's hellhounds struck in but not here yet, and a boss
            // round's bosses still to come.
            let coming = rounds.pending_dogs.len() + rounds.pending_bosses.len() + rounds.bosses_to_spawn as usize;
            // For the HUD's "enemies left": still to come plus still standing.
            let left = rounds.to_spawn + (alive + coming) as u32;
            if lobby.enemies_left != left {
                lobby.enemies_left = left;
            }
            // ...and the HUD's "active enemies": just the ones standing.
            if lobby.enemies_active != alive as u32 {
                lobby.enemies_active = alive as u32;
            }

            // Round cleared: a breather, then the next one.
            if rounds.to_spawn == 0 && alive == 0 && coming == 0 {
                // A dog round over: its last dog leaves a Max Ammo behind.
                if let Some(at) = rounds.last_dog_at.take().filter(|_| shared::dogs::is_dog_round(rounds.round)) {
                    crate::power_ups::spawn_drop(
                        &mut commands,
                        lobby_e,
                        lobby.real_peers(),
                        shared::power_ups::PowerUp::MaxAmmo,
                        at,
                    );
                    info!("lobby {lobby_e:?}: the last dog dropped a Max Ammo");
                }
                rounds.round += 1;
                rounds.to_spawn = enemies_in_round(rounds.round, members);
                rounds.break_until = now + ROUND_BREAK_SECS;
                rounds.next_spawn_at = rounds.break_until;
                queue_round_bosses(rounds, members);
                lobby.round = rounds.round;
                lobby.enemies_left = rounds.to_spawn + rounds.bosses_to_spawn;
                info!("lobby {lobby_e:?}: zombies round {}", rounds.round);
                continue;
            }
        }
        let alive = zombies
            .iter()
            .filter(|(_, _, lp, c, _)| lp.lobby == lobby_e && c.alive)
            .count();
        let coming = rounds.pending_dogs.len() + rounds.pending_bosses.len() + rounds.bosses_to_spawn as usize;

        // Somewhere near a random living member, as far off as a zombie
        // comes up.
        let living: Vec<Vec3> = players
            .iter()
            .filter(|(id, _, lp, c)| lp.lobby == lobby_e && !is_bot_peer(id.0) && c.alive)
            .map(|(_, pose, ..)| pose.translation - Vec3::Y * EYE_HEIGHT)
            .collect();
        let nav = navs.graph(lobby.map);
        let pick_spot = |seed: u64| -> Option<(Vec3, Vec3)> {
            if living.is_empty() {
                return None;
            }
            let near = living[(rand01(seed) * living.len() as f32) as usize % living.len()];
            nav.random_spot_near(near, SPAWN_MIN_DIST, SPAWN_MAX_DIST, seed ^ 0x5eed)
                .filter(|p| living.iter().all(|l| l.distance(*p) >= SPAWN_CLEAR_OF_PLAYERS))
                .map(|feet| (feet, near))
        };

        // A boss round's boss, once it's due: struck in by lightning
        // (`crate::boss`), on top of the round's zombies.
        if rounds.bosses_to_spawn > 0 && now >= rounds.boss_due_at {
            let seed = (now.to_bits() as u64) << 24 ^ 0xb055 ^ lobby_e.to_bits();
            match pick_spot(seed) {
                Some((feet, near)) => {
                    rounds.pending_bosses.push(crate::boss::PendingBoss::new(feet, near, now));
                    rounds.bosses_to_spawn -= 1;
                    // (Several a few seconds apart.)
                    rounds.boss_due_at = now + if rounds.exfil.is_some() { 1.5 } else { 4.0 };
                    info!("lobby {lobby_e:?}: a boss is coming");
                }
                None => rounds.boss_due_at = now + SPAWN_RETRY_SECS,
            }
        }

        if rounds.to_spawn == 0 || alive + coming >= MAX_ALIVE || now < rounds.next_spawn_at {
            continue;
        }

        let seed = (now.to_bits() as u64) << 20 ^ next_id.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ lobby_e.to_bits();
        let Some((feet, near)) = pick_spot(seed) else {
            rounds.next_spawn_at = now + SPAWN_RETRY_SECS;
            continue;
        };

        // The exfil's wave comes quick, and on top of the stronger zombies
        // (`ZombieRounds::enemy_round`), part of it's hellhounds.
        let interval = if rounds.exfil.is_some() {
            crate::exfil::SPAWN_INTERVAL_SECS
        } else {
            spawn_interval(rounds.round)
        };
        let dog = match rounds.exfil {
            Some(_) => rand01(seed ^ 0xd06) < crate::exfil::DOG_SHARE,
            None => shared::dogs::is_dog_round(rounds.round),
        };
        // A dog round: lightning strikes there first, and the hellhound
        // follows once it's done (`crate::dogs`).
        if dog {
            let to_player = near - feet;
            rounds.pending_dogs.push(PendingDog {
                feet,
                yaw: f32::atan2(-to_player.x, -to_player.z),
                spawn_at: now + shared::dogs::DOG_PRE_SPAWN_SECS,
                announced: false,
            });
            rounds.to_spawn -= 1;
            rounds.next_spawn_at = now + interval;
            continue;
        }

        let peer = bot_peer(next_id.0);
        next_id.0 += 1;
        // How fast (the round's speed, give or take a little), and so
        // whether it walks or runs.
        let round = rounds.enemy_round();
        let (runner, speed) = shared::zombies::zombie_speed(round, rand01(seed ^ 0x2a2a));
        let to_player = near - feet;
        let yaw = f32::atan2(-to_player.x, -to_player.z);
        let real = lobby.real_peers();
        commands.spawn((
            Name::from("Zombie"),
            Zombie,
            LobbyPlayer { lobby: lobby_e },
            PlayerId(peer),
            PlayerName("Zombie".into()),
            PlayerPose {
                translation: feet + Vec3::Y * EYE_HEIGHT,
                yaw,
                zombie: shared::ZombieAnim::Rising,
                ..default()
            },
            ActionState::<PlayerInput>::default(),
            BotBrain::new(BotDifficulty::Recruit, feet, seed)
                .facing(yaw)
                .with_skill(zombie_skill(round))
                .rising(now, RISE_SECS)
                .zombie(runner, speed),
            // Tougher every round (`shared::zombies::zombie_health`).
            PlayerCombat::zombie(shared::zombies::zombie_health(round)),
            shared::PlayerHealth(shared::zombies::zombie_health(round)),
            Replicate::to_clients(NetworkTarget::Only(real.clone())),
            InterpolationTarget::to_clients(NetworkTarget::Only(real)),
        ));
        rounds.to_spawn -= 1;
        rounds.next_spawn_at = now + interval;
    }
}

/// Set up `rounds`' (just started) round's bosses, for `members` players:
/// none, or a boss round's, due [`shared::boss::BOSS_ROUND_DELAY_SECS`] in.
fn queue_round_bosses(rounds: &mut ZombieRounds, members: usize) {
    rounds.bosses_to_spawn = if shared::boss::is_boss_round(rounds.round) {
        shared::boss::bosses_in_round(rounds.round, members)
    } else {
        0
    };
    rounds.boss_due_at = rounds.break_until + shared::boss::BOSS_ROUND_DELAY_SECS;
}

/// Tell every member of the lobby where a zombie's swipe just landed, for
/// the hit sound.
fn announce_zombie_swipes(
    server: Single<&lightyear::prelude::server::Server>,
    mut sender: lightyear::prelude::ServerMultiMessageSender,
    mut swipes: EventReader<crate::ai::ZombieSwipeLanded>,
    lobbies: Query<&Lobby>,
) {
    let server = server.into_inner();
    for swipe in swipes.read() {
        let Ok(lobby) = lobbies.get(swipe.lobby) else {
            continue;
        };
        let msg = shared::ZombieSwipeLanded {
            at: swipe.at.to_array(),
        };
        if let Err(e) = sender.send::<_, shared::GameChannel>(
            &msg,
            server,
            &NetworkTarget::Only(lobby.real_peers()),
        ) {
            error!("failed to send zombie swipe: {e:?}");
        }
    }
}

/// Copy each zombie's animation state from its brain onto its replicated
/// pose (after `sim::apply_client_pose` has rewritten the rest from input).
fn publish_zombie_anims(mut zombies: Query<(&BotBrain, &mut PlayerPose), With<Zombie>>) {
    for (brain, mut pose) in &mut zombies {
        let anim = brain.zombie_anim();
        if pose.zombie != anim {
            pose.zombie = anim;
        }
    }
}

/// A member wants to buy a perk: they must be in a running `Zombies` game,
/// alive, standing at its machine with the power on
/// (`shared::power::has_power`) — or at an active Der Wunderfizz
/// (`shared::wunderfizz`, any of the classic perks) — with a little slack
/// (their pose is a moment old), not already own it, and have the points.
/// Anything else is ignored.
fn on_buy_perk(
    trigger: Trigger<RemoteTrigger<BuyPerk>>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let BuyPerk { perk, wunderfizz } = trigger.trigger;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused {
        return;
    }
    let Some((_, pose, combat)) = players.iter().find(|(id, ..)| id.0 == peer) else {
        return;
    };
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    // (Only the lobby's own set is on sale.)
    if !combat.alive || perk.set() != lobby.perk_set {
        return;
    }
    let at_seller = if wunderfizz {
        shared::wunderfizz::active(&lobby) && shared::wunderfizz::in_range(lobby.map, feet, 0.75)
    } else {
        // (A machine sells nothing until the power's on.)
        perk.has_machine()
            && shared::power::has_power(lobby.map, lobby.power_on)
            && shared::perks::in_range(perk, lobby.map, feet, 0.75)
    };
    if !at_seller {
        return;
    }
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    if member.perks.contains(&perk) || member.score < perk.cost() {
        return;
    }
    member.score -= perk.cost();
    member.perks.push(perk);
    info!("{peer:?} bought {}", perk.label());
}

/// A member went prone at a perk machine: the first in the game to do so at
/// each machine gets [`shared::perks::PRONE_BONUS_POINTS`] (power or not),
/// and only they hear about it ([`ProneBonus`]). They must be in a running,
/// unpaused `Zombies` game, alive and at the machine (a little slack).
fn on_prone_at_perk(
    trigger: Trigger<RemoteTrigger<ProneAtPerk>>,
    endings: Res<crate::killcam::EndingLobbies>,
    server: Single<&lightyear::prelude::server::Server>,
    mut sender: lightyear::prelude::ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby, &mut ZombieRounds)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let perk = trigger.trigger.perk;
    let Some((lobby_e, mut lobby, mut rounds)) = lobbies
        .iter_mut()
        .find(|(_, l, _)| l.started && l.mode == GameMode::Zombies && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused || rounds.prone_claimed.contains(&perk) {
        return;
    }
    let Some((_, pose, combat)) = players.iter().find(|(id, ..)| id.0 == peer) else {
        return;
    };
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    if !combat.alive
        || perk.set() != lobby.perk_set
        || !perk.has_machine()
        || !shared::perks::in_range(perk, lobby.map, feet, 0.75)
    {
        return;
    }
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    let points = shared::perks::PRONE_BONUS_POINTS;
    member.score += points;
    rounds.prone_claimed.push(perk);
    if let Err(e) = sender.send::<_, GameChannel>(
        &ProneBonus { points },
        server.into_inner(),
        &NetworkTarget::Single(peer),
    ) {
        error!("failed to send prone bonus: {e:?}");
    }
    info!("{peer:?} went prone at {} for {points} points", perk.label());
}

/// A member wants to Pack-a-Punch the weapon in their hands: they must be in
/// a running `Zombies` game with the power on, alive, standing at the
/// machine (a little slack), asking for a level above the one it's at (up to
/// `shared::pap::MAX_LEVEL` — skipping levels is fine, they pay for each),
/// and have the points. Anything else is ignored.
fn on_buy_pap(
    trigger: Trigger<RemoteTrigger<BuyPap>>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let BuyPap { weapon, level } = trigger.trigger;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused || !shared::power::has_power(lobby.map, lobby.power_on) {
        return;
    }
    let Some((_, pose, combat)) = players.iter().find(|(id, ..)| id.0 == peer) else {
        return;
    };
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    if !combat.alive || !shared::pap::in_range(lobby.map, feet, 0.75) {
        return;
    }
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    // (Only a weapon they're carrying.)
    if !member.weapons.iter().any(|&w| shared::pap::PapWeapon::of(w) == weapon) {
        return;
    }
    let current = member.pap.get(weapon);
    let cost = shared::pap::cost_to(current, level);
    if level > shared::pap::MAX_LEVEL || level <= current || member.score < cost {
        return;
    }
    member.score -= cost;
    member.pap.set(weapon, level);
    info!("{peer:?} packed their {} to level {level}", weapon.label());
}

/// A member wants to turn the power on: they must be in a running `Zombies`
/// game on a map with a switch, alive, standing at it (a little slack), with
/// the points, and it mustn't be on already. Anything else is ignored.
fn on_turn_on_power(
    trigger: Trigger<RemoteTrigger<TurnOnPower>>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused || lobby.power_on {
        return;
    }
    let Some((_, pose, combat)) = players.iter().find(|(id, ..)| id.0 == peer) else {
        return;
    };
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    if !combat.alive || !shared::power::in_range(lobby.map, feet, 0.75) {
        return;
    }
    let cost = shared::power::POWER_COST;
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    if member.score < cost {
        return;
    }
    member.score -= cost;
    lobby.power_on = true;
    info!("{peer:?} turned the power on");
}

/// A member wants a sniper ammo refill at the ammo crate: they must be in a
/// running `Zombies` game, alive, with the points. (Where the crate stands
/// and whether they're already full are the client's to check — see
/// `shared::ammo`.) Takes the points and tells them to fill up.
fn on_buy_ammo(
    trigger: Trigger<RemoteTrigger<BuyAmmo>>,
    endings: Res<crate::killcam::EndingLobbies>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused {
        return;
    }
    if !players.iter().any(|(id, c)| id.0 == peer && c.alive) {
        return;
    }
    let cost = shared::ammo::AMMO_COST;
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    if member.score < cost {
        return;
    }
    member.score -= cost;
    if let Err(e) =
        sender.send::<_, GameChannel>(&AmmoBought, server.into_inner(), &NetworkTarget::Single(peer))
    {
        error!("failed to send ammo purchase: {e:?}");
    }
    info!("{peer:?} bought ammo");
}

/// Let a dead zombie lie for [`CORPSE_SECS`] (its death animation), then
/// remove it. (A dead hellhound is gone at once — `crate::dogs`.)
fn clear_dead_zombies(
    time: Res<Time>,
    fresh: Query<(Entity, &PlayerCombat), (With<Zombie>, Without<ZombieDeath>, Without<crate::dogs::Dog>)>,
    dead: Query<(Entity, &ZombieDeath)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (entity, combat) in &fresh {
        if !combat.alive {
            commands.entity(entity).insert(ZombieDeath(now));
        }
    }
    for (entity, death) in &dead {
        if now - death.0 >= CORPSE_SECS {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Remove every zombie whose game is over (or whose lobby is gone).
fn cull_zombies(
    zombies: Query<(Entity, &LobbyPlayer), With<Zombie>>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (entity, lp) in &zombies {
        let over = lobbies
            .get(lp.lobby)
            .map(|l| !l.started || l.mode != GameMode::Zombies)
            .unwrap_or(true);
        if over {
            commands.entity(entity).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::map::CollisionWorld;

    /// A started `Zombies` lobby on Break Point with one loaded real player
    /// standing at `BOT_AREA_CENTER`, running just the round systems.
    fn game() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            core::time::Duration::from_secs_f64(1.0 / 64.0),
        ));
        let (colliders, _) = crate::nav::tests::built();
        app.insert_resource(NavGraphs::build(colliders));
        app.init_resource::<crate::killcam::EndingLobbies>();
        app.init_resource::<crate::killcam::ReplayClock>();
        app.add_event::<crate::pvp::ZombieKilled>();
        app.init_resource::<NextBotId>();
        app.add_systems(Update, (run_rounds, clear_dead_zombies, cull_zombies).chain());
        let me = PeerId::Netcode(1);
        let lobby = app
            .world_mut()
            .spawn(Lobby {
                name: "test".into(),
                leader: me,
                mode: GameMode::Zombies,
                map: shared::MapId::BreakPoint,
                started: true,
                time_limit_secs: 300,
                time_left_secs: 300,
                kill_limit: 30,
                end_cam: shared::EndCam::default(),
                round: 0,
                enemies_left: 0,
                enemies_active: 0,
                paused: false,
                bots_passive: false,
                bots_frozen: false,
                exfil: Default::default(),
                power_up_test: false,
                molotov_test: false,
                active_power_ups: Vec::new(),
                bomb_test: false,
                start_round: 1,
                start_points: 0,
                perk_set: Default::default(),
                countdown_secs: 0,
                countdown_left: 0,
                power_on: false,
                mystery_box: None,
                members: vec![shared::LobbyMember {
                    peer: me,
                    name: "me".into(),
                    score: 0,
                    loaded: true,
                    bot: None,
                    kills: 0,
                    perks: Vec::new(),
                    armor: Default::default(),
                    field_upgrade: Default::default(),
                    pap: Default::default(),
                    loadout: Default::default(),
                    operator: Default::default(),
                    primary: Default::default(),
                    weapons: shared::weapon::SlotWeapon::starting(Default::default()),
                    critical_kills: 0,
                    revives: 0,
                    downs: 0,
                }],
            })
            .id();
        app.world_mut().spawn((
            PlayerId(me),
            LobbyPlayer { lobby },
            PlayerCombat::default(),
            PlayerPose {
                translation: shared::bots::BOT_AREA_CENTER + Vec3::Y * EYE_HEIGHT,
                ..default()
            },
        ));
        app.update(); // (the first update has no time delta)
        // (...and sets up the rounds; with no countdown, the next begins
        // round 1.)
        app.update();
        (app, lobby)
    }

    fn zombies_of(app: &mut App) -> Vec<Entity> {
        app.world_mut()
            .query_filtered::<Entity, With<Zombie>>()
            .iter(app.world())
            .collect()
    }

    fn run(app: &mut App, secs: f32) {
        for _ in 0..(secs * 64.0) as usize {
            app.update();
        }
    }

    #[test]
    fn the_pre_game_countdown_holds_the_first_round_and_its_zombies_back() {
        let (mut app, lobby) = game();
        // Start again, this time with a 5 s countdown.
        app.world_mut().entity_mut(lobby).remove::<ZombieRounds>();
        {
            let mut l = app.world_mut().get_mut::<Lobby>(lobby).unwrap();
            l.countdown_secs = 5;
            l.round = 0;
        }
        app.update();
        assert_eq!(app.world().get::<Lobby>(lobby).unwrap().countdown_left, 5);
        run(&mut app, 4.5);
        let l = app.world().get::<Lobby>(lobby).unwrap();
        assert_eq!(l.round, 0, "no round during the countdown");
        assert_eq!(l.countdown_left, 1);
        assert!(zombies_of(&mut app).is_empty(), "no zombies during the countdown");
        run(&mut app, 1.0);
        let l = app.world().get::<Lobby>(lobby).unwrap();
        assert_eq!(l.round, 1);
        assert_eq!(l.countdown_left, 0);
        // ...and straight into it: zombies come without the usual opening
        // delay.
        run(&mut app, 3.0);
        assert!(!zombies_of(&mut app).is_empty());
    }

    #[test]
    fn a_round_sends_its_zombies_then_the_next_round_starts_once_theyre_dead() {
        let (mut app, lobby) = game();
        let left = |app: &App| app.world().get::<Lobby>(lobby).unwrap().enemies_left;
        assert_eq!(app.world().get::<Lobby>(lobby).unwrap().round, 1);
        assert_eq!(left(&app), zombies_in_round(1, 1));
        // Nothing during the opening delay...
        run(&mut app, FIRST_ROUND_DELAY_SECS - 0.5);
        assert!(zombies_of(&mut app).is_empty());
        // ...then round 1's four, a couple of seconds apart.
        run(&mut app, 12.0);
        let first = zombies_of(&mut app);
        assert_eq!(first.len() as u32, zombies_in_round(1, 1));
        for &z in &first {
            // Every one on walkable ground in range of the player.
            let pose = app.world().get::<PlayerPose>(z).unwrap();
            let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
            let d = Vec3::new(feet.x, 0.0, feet.z)
                .distance(Vec3::new(shared::bots::BOT_AREA_CENTER.x, 0.0, shared::bots::BOT_AREA_CENTER.z));
            assert!((SPAWN_MIN_DIST - 0.01..=SPAWN_MAX_DIST + 0.01).contains(&d), "{d} m away");
            assert!(is_bot_peer(app.world().get::<PlayerId>(z).unwrap().0));
        }

        // Kill them all: they lie there a moment, then go, and round 2 comes.
        // Killing one takes one off the count.
        assert_eq!(left(&app), zombies_in_round(1, 1));
        app.world_mut().get_mut::<PlayerCombat>(first[0]).unwrap().alive = false;
        app.update();
        assert_eq!(left(&app), zombies_in_round(1, 1) - 1);
        for &z in &first {
            app.world_mut().get_mut::<PlayerCombat>(z).unwrap().alive = false;
        }
        run(&mut app, CORPSE_SECS + 0.5);
        assert!(zombies_of(&mut app).is_empty());
        assert_eq!(app.world().get::<Lobby>(lobby).unwrap().round, 2);
        assert_eq!(left(&app), zombies_in_round(2, 1));
        run(&mut app, ROUND_BREAK_SECS + 20.0);
        assert_eq!(zombies_of(&mut app).len() as u32, zombies_in_round(2, 1));
    }

    #[test]
    fn a_dog_round_queues_only_dogs_and_isnt_over_until_theyre_in_and_dead() {
        let (mut app, lobby) = game();
        // Start again on round 5.
        app.world_mut().entity_mut(lobby).remove::<ZombieRounds>();
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().start_round = 5;
        app.update();
        app.update();
        let l = app.world().get::<Lobby>(lobby).unwrap();
        assert_eq!(l.round, 5);
        assert_eq!(l.enemies_left, shared::dogs::dogs_in_round(5, 1));
        run(&mut app, FIRST_ROUND_DELAY_SECS + 20.0);
        // (Bringing them in is `crate::dogs`', not running here): every one
        // queued behind its lightning, no zombie in sight, and the round
        // still on.
        assert!(zombies_of(&mut app).is_empty());
        let queued = app.world().get::<ZombieRounds>(lobby).unwrap().pending_dogs.len() as u32;
        assert_eq!(queued, shared::dogs::dogs_in_round(5, 1));
        let l = app.world().get::<Lobby>(lobby).unwrap();
        assert_eq!(l.round, 5);
        assert_eq!(l.enemies_left, queued);

        // The last of them dies (`crate::dogs` notes where): the round's
        // over, and a Max Ammo lies there.
        let at = Vec3::new(3.0, 0.0, -4.0);
        {
            let mut rounds = app.world_mut().get_mut::<ZombieRounds>(lobby).unwrap();
            rounds.pending_dogs.clear();
            rounds.last_dog_at = Some(at);
        }
        app.update();
        assert_eq!(app.world().get::<Lobby>(lobby).unwrap().round, 6);
        let drops: Vec<shared::PowerUpDrop> = app
            .world_mut()
            .query::<&shared::PowerUpDrop>()
            .iter(app.world())
            .cloned()
            .collect();
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].kind, shared::power_ups::PowerUp::MaxAmmo);
        assert_eq!(drops[0].pos, at);
    }

    #[test]
    fn the_zombies_go_when_the_game_ends() {
        let (mut app, lobby) = game();
        run(&mut app, FIRST_ROUND_DELAY_SECS + 4.0);
        assert!(!zombies_of(&mut app).is_empty());
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().started = false;
        app.update();
        app.update();
        assert!(zombies_of(&mut app).is_empty());
    }

    #[test]
    fn a_member_at_the_machine_with_the_points_buys_the_perk_once() {
        let (mut app, lobby) = game();
        app.add_observer(on_buy_perk);
        let me = PeerId::Netcode(1);
        let perk = shared::perks::Perk::Juggernog;
        let map = app.world().get::<Lobby>(lobby).unwrap().map;
        // Stand at the machine with 3000 points.
        let at_machine = perk.machine_pos(map) + Vec3::Y * EYE_HEIGHT;
        let player = app
            .world_mut()
            .query_filtered::<Entity, (With<PlayerCombat>, Without<Zombie>)>()
            .iter(app.world())
            .next()
            .unwrap();
        app.world_mut().get_mut::<PlayerPose>(player).unwrap().translation = at_machine;
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().members[0].score = 3000;
        // (Break Point has a power switch: machines only sell with it on.)
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().power_on = true;
        let buy = |app: &mut App| {
            app.world_mut().trigger(RemoteTrigger {
                trigger: BuyPerk { perk, wunderfizz: false },
                from: me,
            });
            app.world_mut().flush();
        };
        buy(&mut app);
        let member = app.world().get::<Lobby>(lobby).unwrap().members[0].clone();
        assert_eq!(member.perks, vec![perk]);
        assert_eq!(member.score, 3000 - perk.cost());
        // A second press doesn't charge again.
        buy(&mut app);
        let member = app.world().get::<Lobby>(lobby).unwrap().members[0].clone();
        assert_eq!(member.score, 3000 - perk.cost());
    }

    #[test]
    fn no_perk_sells_until_the_power_is_on() {
        let (mut app, lobby) = game();
        app.add_observer(on_buy_perk);
        let me = PeerId::Netcode(1);
        let perk = shared::perks::Perk::Juggernog;
        let map = shared::MapId::BreakPointNight;
        {
            let mut l = app.world_mut().get_mut::<Lobby>(lobby).unwrap();
            l.map = map;
            l.members[0].score = 3000;
        }
        let player = app
            .world_mut()
            .query_filtered::<Entity, (With<PlayerCombat>, Without<Zombie>)>()
            .iter(app.world())
            .next()
            .unwrap();
        app.world_mut().get_mut::<PlayerPose>(player).unwrap().translation =
            perk.machine_pos(map) + Vec3::Y * EYE_HEIGHT;
        let buy = |app: &mut App| {
            app.world_mut().trigger(RemoteTrigger {
                trigger: BuyPerk { perk, wunderfizz: false },
                from: me,
            });
            app.world_mut().flush();
        };
        buy(&mut app);
        let member = app.world().get::<Lobby>(lobby).unwrap().members[0].clone();
        assert!(member.perks.is_empty(), "sold with the power off");
        assert_eq!(member.score, 3000);
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().power_on = true;
        buy(&mut app);
        assert_eq!(app.world().get::<Lobby>(lobby).unwrap().members[0].perks, vec![perk]);
    }

    #[test]
    fn a_perk_cant_be_bought_from_across_the_map_or_without_the_points() {
        let (mut app, lobby) = game();
        app.add_observer(on_buy_perk);
        let perk = shared::perks::Perk::Juggernog;
        // Standing at the map centre, far from Break Point's machine.
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().members[0].score = 10_000;
        app.world_mut().trigger(RemoteTrigger {
            trigger: BuyPerk { perk, wunderfizz: false },
            from: PeerId::Netcode(1),
        });
        app.world_mut().flush();
        assert!(app.world().get::<Lobby>(lobby).unwrap().members[0].perks.is_empty());
    }

    #[test]
    fn the_power_goes_on_once_for_whoever_pays_at_the_switch() {
        let (mut app, lobby) = game();
        app.add_observer(on_turn_on_power);
        let me = PeerId::Netcode(1);
        let flip = |app: &mut App| {
            app.world_mut().trigger(RemoteTrigger { trigger: TurnOnPower, from: me });
            app.world_mut().flush();
        };
        let player = app
            .world_mut()
            .query_filtered::<Entity, (With<PlayerCombat>, Without<Zombie>)>()
            .iter(app.world())
            .next()
            .unwrap();
        {
            let mut l = app.world_mut().get_mut::<Lobby>(lobby).unwrap();
            l.members[0].score = 3000;
            // The basic map has no switch.
            l.map = shared::MapId::BasicMap;
        }
        let switch = shared::power::switch_pos(shared::MapId::BreakPointNight).unwrap();
        let at_switch = switch + Vec3::Y * EYE_HEIGHT;
        app.world_mut().get_mut::<PlayerPose>(player).unwrap().translation = at_switch;
        flip(&mut app);
        assert!(!app.world().get::<Lobby>(lobby).unwrap().power_on, "no switch on this map");
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().map = shared::MapId::BreakPointNight;
        // Too far away.
        app.world_mut().get_mut::<PlayerPose>(player).unwrap().translation = at_switch + Vec3::X * 5.0;
        flip(&mut app);
        assert!(!app.world().get::<Lobby>(lobby).unwrap().power_on);
        app.world_mut().get_mut::<PlayerPose>(player).unwrap().translation = at_switch;
        flip(&mut app);
        let l = app.world().get::<Lobby>(lobby).unwrap();
        assert!(l.power_on);
        assert_eq!(l.members[0].score, 3000 - shared::power::POWER_COST);
        // Already on: not charged again.
        flip(&mut app);
        assert_eq!(app.world().get::<Lobby>(lobby).unwrap().members[0].score, 3000 - shared::power::POWER_COST);
    }

    #[test]
    fn rounds_start_small_and_grow() {
        assert_eq!(zombies_in_round(1, 1), 4);
        assert!(zombies_in_round(2, 1) > zombies_in_round(1, 1));
        assert!(zombies_in_round(10, 1) > zombies_in_round(5, 1));
        // More players, more zombies.
        assert!(zombies_in_round(1, 3) > zombies_in_round(1, 1));
    }

    #[test]
    fn zombies_start_worse_than_a_recruit_and_end_up_a_veteran() {
        let recruit = BotDifficulty::Recruit.skill();
        let first = zombie_skill(1);
        assert!(first.reaction_secs > recruit.reaction_secs);
        assert!(first.aim_error_deg > recruit.aim_error_deg);
        assert!(first.fire_interval_secs > recruit.fire_interval_secs);
        assert!(!first.sprints);
        // Strictly better every round until it tops out.
        for r in 1..15 {
            let (a, b) = (zombie_skill(r), zombie_skill(r + 1));
            assert!(b.aim_error_deg < a.aim_error_deg && b.reaction_secs < a.reaction_secs);
        }
        assert_eq!(zombie_skill(15), zombie_skill(40));
        let top = zombie_skill(15);
        let veteran = BotDifficulty::Veteran.skill();
        assert!((top.aim_error_deg - veteran.aim_error_deg).abs() < 1e-4);
        assert!((top.reaction_secs - veteran.reaction_secs).abs() < 1e-4);
    }

    #[test]
    fn a_spawn_spot_is_walkable_in_range_and_connected() {
        let (colliders, navs) = crate::nav::tests::built();
        for map in [shared::MapId::BasicMap, shared::MapId::Shipment, shared::MapId::BreakPoint] {
            let nav = navs.graph(map);
            let world = colliders.world(map);
            let center = shared::bots::BOT_AREA_CENTER;
            let mut found = 0;
            for seed in 0..40u64 {
                let Some(p) = nav.random_spot_near(center, SPAWN_MIN_DIST, SPAWN_MAX_DIST, seed) else {
                    continue;
                };
                found += 1;
                let flat = Vec3::new(p.x - center.x, 0.0, p.z - center.z).length();
                assert!((SPAWN_MIN_DIST..=SPAWN_MAX_DIST).contains(&flat), "{map:?}: {flat} m");
                assert!(nav.connected(center, p), "{map:?}: {p:?} can't be walked to");
                // Standing room: nothing solid in the body at chest height.
                let chest = p + Vec3::Y * crate::nav::CHEST_HEIGHT;
                assert!(
                    world.sweep_sphere(chest, chest + Vec3::Y * 0.01, crate::nav::BODY_RADIUS).is_none(),
                    "{map:?}: {p:?} is inside something"
                );
            }
            assert!(found >= 30, "{map:?}: only {found}/40 spawns found a spot");
        }
    }
}
