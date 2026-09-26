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
//! Scoring and the game ending (any member dying) live in `pvp`; the round
//! number is replicated as `Lobby::round` for the HUD and results screen.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::*;

use shared::bot_players::{bot_peer, is_bot_peer, BotDifficulty, BotSkill};
use shared::bots::rand01;
use shared::{BuyPerk, GameMode, TurnOnPower, Lobby, PlayerId, PlayerInput, PlayerName, PlayerPose};

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
const RISE_SECS: f32 = 1.6;
/// Seconds before round 1's first zombie, once everyone has loaded in.
const FIRST_ROUND_DELAY_SECS: f32 = 4.0;
/// Seconds between the last zombie of a round dying and the next round.
const ROUND_BREAK_SECS: f32 = 7.0;
/// Seconds a dead zombie lies there (its death animation) before it's removed.
const CORPSE_SECS: f32 = 3.0;
/// Seconds before retrying a spawn that found nowhere to go.
const SPAWN_RETRY_SECS: f32 = 0.25;

/// How many zombies round `round` sends, for `players` members.
pub fn zombies_in_round(round: u32, players: usize) -> u32 {
    let round = round.max(1);
    (4 + 2 * (round - 1) + 2 * (players.max(1) as u32 - 1)).min(60)
}

/// Most zombies up at once (the rest of the round waits its turn).
fn max_alive(players: usize) -> usize {
    (6 + 2 * players).min(16)
}

/// Seconds between spawns in `round` — quicker as the rounds go on.
fn spawn_interval(round: u32) -> f32 {
    (2.5 - 0.15 * (round.max(1) - 1) as f32).max(0.6)
}

/// How good round `round`'s zombies are: well under a `Recruit` bot at first
/// (slow to notice you, slow to turn, wild aim, slow to fire), climbing to
/// about a `Veteran` by round 15. They start sprinting at round 10.
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
        sprints: round >= 10,
    }
}

/// On a zombie's player entity.
#[derive(Component)]
pub struct Zombie;

/// When a zombie died (`Time::elapsed_secs`), for [`CORPSE_SECS`].
#[derive(Component)]
struct ZombieDeath(f32);

/// A running `Zombies` game's round state, on its lobby entity (server-only;
/// the round number itself is replicated as `Lobby::round`).
#[derive(Component)]
struct ZombieRounds {
    round: u32,
    /// Zombies this round still has to send.
    to_spawn: u32,
    next_spawn_at: f32,
    /// No spawning until then (the start delay / the break between rounds).
    break_until: f32,
}

pub struct ZombiesPlugin;

impl Plugin for ZombiesPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_buy_perk).add_observer(on_turn_on_power).add_systems(
            FixedUpdate,
            (run_rounds, clear_dead_zombies, cull_zombies)
                .before(crate::ai::drive_bots),
        );
    }
}

/// Start, advance and feed every running `Zombies` game.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_rounds(
    time: Res<Time>,
    navs: Res<NavGraphs>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut next_id: ResMut<NextBotId>,
    mut lobbies: Query<(Entity, &mut Lobby, Option<&mut ZombieRounds>)>,
    players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &PlayerCombat)>,
    zombies: Query<(&LobbyPlayer, &PlayerCombat), With<Zombie>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (lobby_e, mut lobby, rounds) in &mut lobbies {
        if !lobby.started || lobby.mode != GameMode::Zombies {
            if rounds.is_some() {
                commands.entity(lobby_e).remove::<ZombieRounds>();
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
            }
            continue;
        }
        let Some(mut rounds) = rounds else {
            lobby.round = 1;
            lobby.enemies_left = zombies_in_round(1, members);
            commands.entity(lobby_e).insert(ZombieRounds {
                round: 1,
                to_spawn: zombies_in_round(1, members),
                next_spawn_at: now,
                break_until: now + FIRST_ROUND_DELAY_SECS,
            });
            info!("lobby {lobby_e:?}: zombies round 1");
            continue;
        };
        if now < rounds.break_until {
            continue;
        }

        let alive = zombies
            .iter()
            .filter(|(lp, c)| lp.lobby == lobby_e && c.alive)
            .count();
        // For the HUD's "enemies left": still to come plus still standing.
        let left = rounds.to_spawn + alive as u32;
        if lobby.enemies_left != left {
            lobby.enemies_left = left;
        }

        // Round cleared: a breather, then the next one.
        if rounds.to_spawn == 0 && alive == 0 {
            rounds.round += 1;
            rounds.to_spawn = zombies_in_round(rounds.round, members);
            rounds.break_until = now + ROUND_BREAK_SECS;
            rounds.next_spawn_at = rounds.break_until;
            lobby.round = rounds.round;
            lobby.enemies_left = rounds.to_spawn;
            info!("lobby {lobby_e:?}: zombies round {}", rounds.round);
            continue;
        }

        if rounds.to_spawn == 0 || alive >= max_alive(members) || now < rounds.next_spawn_at {
            continue;
        }

        // Somewhere near a random living member.
        let living: Vec<Vec3> = players
            .iter()
            .filter(|(id, _, lp, c)| lp.lobby == lobby_e && !is_bot_peer(id.0) && c.alive)
            .map(|(_, pose, ..)| pose.translation - Vec3::Y * EYE_HEIGHT)
            .collect();
        if living.is_empty() {
            continue;
        }
        let seed = (now.to_bits() as u64) << 20 ^ next_id.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ lobby_e.to_bits();
        let near = living[(rand01(seed) * living.len() as f32) as usize % living.len()];
        let nav = navs.graph(lobby.map);
        let spot = nav
            .random_spot_near(near, SPAWN_MIN_DIST, SPAWN_MAX_DIST, seed ^ 0x5eed)
            .filter(|p| living.iter().all(|l| l.distance(*p) >= SPAWN_CLEAR_OF_PLAYERS));
        let Some(feet) = spot else {
            rounds.next_spawn_at = now + SPAWN_RETRY_SECS;
            continue;
        };

        let peer = bot_peer(next_id.0);
        next_id.0 += 1;
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
                ..default()
            },
            ActionState::<PlayerInput>::default(),
            BotBrain::new(BotDifficulty::Recruit, feet, seed)
                .facing(yaw)
                .with_skill(zombie_skill(rounds.round))
                .rising(now, RISE_SECS),
            PlayerCombat::default(),
            shared::PlayerHealth(shared::health::FULL_HEALTH),
            Replicate::to_clients(NetworkTarget::Only(real.clone())),
            InterpolationTarget::to_clients(NetworkTarget::Only(real)),
        ));
        rounds.to_spawn -= 1;
        rounds.next_spawn_at = now + spawn_interval(rounds.round);
    }
}

/// A member wants to buy a perk: they must be in a running `Zombies` game,
/// alive, standing at its machine (a little slack — their pose is a moment
/// old), not already own it, and have the points. Anything else is ignored.
fn on_buy_perk(
    trigger: Trigger<RemoteTrigger<BuyPerk>>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let perk = trigger.trigger.perk;
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
    if !combat.alive || !shared::perks::in_range(perk, lobby.map, feet, 0.75) {
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

/// Let a dead zombie lie for [`CORPSE_SECS`] (its death animation), then
/// remove it.
fn clear_dead_zombies(
    time: Res<Time>,
    fresh: Query<(Entity, &PlayerCombat), (With<Zombie>, Without<ZombieDeath>)>,
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
                paused: false,
                bots_passive: false,
                power_on: false,
                members: vec![shared::LobbyMember {
                    peer: me,
                    name: "me".into(),
                    score: 0,
                    loaded: true,
                    bot: None,
                    kills: 0,
                    perks: Vec::new(),
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
        let perk = shared::perks::Perk::ShroomTea;
        let map = app.world().get::<Lobby>(lobby).unwrap().map;
        // Stand at the machine with 250 points.
        let at_machine = perk.machine_pos(map) + Vec3::Y * EYE_HEIGHT;
        let player = app
            .world_mut()
            .query_filtered::<Entity, (With<PlayerCombat>, Without<Zombie>)>()
            .iter(app.world())
            .next()
            .unwrap();
        app.world_mut().get_mut::<PlayerPose>(player).unwrap().translation = at_machine;
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().members[0].score = 250;
        let buy = |app: &mut App| {
            app.world_mut().trigger(RemoteTrigger {
                trigger: BuyPerk { perk },
                from: me,
            });
            app.world_mut().flush();
        };
        buy(&mut app);
        let member = app.world().get::<Lobby>(lobby).unwrap().members[0].clone();
        assert_eq!(member.perks, vec![perk]);
        assert_eq!(member.score, 250 - perk.cost());
        // A second press doesn't charge again.
        buy(&mut app);
        let member = app.world().get::<Lobby>(lobby).unwrap().members[0].clone();
        assert_eq!(member.score, 250 - perk.cost());
    }

    #[test]
    fn a_perk_cant_be_bought_from_across_the_map_or_without_the_points() {
        let (mut app, lobby) = game();
        app.add_observer(on_buy_perk);
        let perk = shared::perks::Perk::ShroomTea;
        // Standing at the map centre, far from Break Point's machine.
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().members[0].score = 1000;
        app.world_mut().trigger(RemoteTrigger {
            trigger: BuyPerk { perk },
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
            l.members[0].score = 250;
            // Break Point (day) has no switch.
            l.map = shared::MapId::BreakPoint;
        }
        let at_switch = Vec3::ZERO + Vec3::Y * EYE_HEIGHT;
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
        assert_eq!(l.members[0].score, 250 - shared::power::POWER_COST);
        // Already on: not charged again.
        flip(&mut app);
        assert_eq!(app.world().get::<Lobby>(lobby).unwrap().members[0].score, 250 - shared::power::POWER_COST);
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
