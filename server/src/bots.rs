//! Server-owned target bots.
//!
//! Bots are capsules replicated to the members of one lobby's game (drawn as
//! the same soldier model as a remote player). They wander the map at walking
//! pace ([`wander_bots`]) — routed, wall-sliding and ground-snapped by the
//! same `ai` / `nav` code as `FreeForAll`'s bot players — but never look for,
//! aim at or shoot anyone: they're just moving trickshot targets.
//! A hit (resolved in [`crate::sim`]) sends a [`BotHit`] event; here we mark the
//! bot dead, tip it over, score the shooter, and after a couple of seconds
//! despawn it. [`ensure_bots`] then refills the lobby's bot count at a fresh
//! spot near the map centre — so respawn is just "despawn + top up".

use bevy::prelude::*;

use lightyear::prelude::*;

use shared::bots::{rand01, respawn_pose, BOTS_ALIVE, BOT_DEAD_SECS, BOT_FALL_SECS, PING_SECS};
use shared::{Bot, GameMode, Lobby, PingBot};

use crate::ai::{look_angles, move_bot, shortest_angle, WALK_SPEED};
use crate::collision::MapColliders;
use crate::nav::{NavGraph, NavGraphs};

/// How fast a wandering bot turns to face where it's walking (degrees / s).
const TURN_DEG_PER_SEC: f32 = 300.0;
/// A bot stands still for somewhere in this range (s) between walks.
const IDLE_SECS: (f32, f32) = (0.5, 3.0);
/// Blocked this long (s) while walking → take a random detour...
const STUCK_SECS: f32 = 0.6;
/// ...lasting this long (s), then pick a fresh destination.
const DETOUR_SECS: f32 = 1.2;
/// Give up on a destination (and pick another) after this long (s) — a route
/// that keeps failing shouldn't pin a bot in place forever.
const MAX_WALK_SECS: f32 = 25.0;
/// A bot that ends up this far below the map is put back on it.
const VOID_Y: f32 = -40.0;

/// A hit on a bot, written by [`crate::sim::resolve_shots`].
#[derive(Event)]
pub struct BotHit {
    pub bot: Entity,
    pub by: PeerId,
    /// Style points the kill is worth (`shared::scoring::score_kill`).
    pub points: u32,
}

/// A bot's health (server-only — clients only see whether it's alive). Shots
/// deduct from it (`sim::resolve_shots`) and the bot dies at zero, just like a
/// `FreeForAll` player's `PlayerCombat::health`. A knife stab or throw kills
/// outright via [`BotHit`] without touching it.
#[derive(Component)]
pub struct BotHealth(pub f32);

/// Every bot spawns with this much health — the same bar a player has.
pub const BOT_HEALTH: f32 = 100.0;

/// Which lobby's game a bot belongs to.
#[derive(Component)]
pub struct LobbyBot {
    pub lobby: Entity,
}

/// A living bot's walking state (server-only).
#[derive(Component)]
pub struct BotWander {
    vertical_velocity: f32,
    /// The current route (feet waypoints) and how far along it the bot is;
    /// empty while standing around.
    path: Vec<Vec3>,
    path_index: usize,
    /// Standing still until this time (`Time::elapsed_secs`).
    idle_until: f32,
    /// When the current walk started, for [`MAX_WALK_SECS`].
    walk_started: f32,
    blocked_for: f32,
    detour_until: f32,
    detour_dir: Vec3,
    /// Running counter feeding [`rand01`], so every roll differs.
    rng: u64,
}

impl BotWander {
    fn new(seed: u64, now: f32) -> Self {
        let mut w = Self {
            vertical_velocity: 0.0,
            path: Vec::new(),
            path_index: 0,
            idle_until: 0.0,
            walk_started: 0.0,
            blocked_for: 0.0,
            detour_until: 0.0,
            detour_dir: Vec3::ZERO,
            rng: seed,
        };
        // Stagger the first walk so a fresh set of bots doesn't all set off
        // on the same tick.
        w.idle_until = now + w.roll() * IDLE_SECS.1;
        w
    }

    fn roll(&mut self) -> f32 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        rand01(self.rng)
    }

    /// Stop and stand around for a random [`IDLE_SECS`] stretch.
    fn rest(&mut self, now: f32) {
        self.path.clear();
        self.path_index = 0;
        self.detour_until = 0.0;
        self.blocked_for = 0.0;
        self.idle_until = now + IDLE_SECS.0 + self.roll() * (IDLE_SECS.1 - IDLE_SECS.0);
    }
}

/// When (`Time::elapsed_secs`) a pinged bot's `Bot::pinged` clears. Absent
/// while it isn't pinged.
#[derive(Component)]
struct BotPing {
    until: f32,
}

/// Wall-clock time (seconds) a bot was shot. Absent while alive.
#[derive(Component)]
struct BotDeath {
    at: f32,
}

pub struct BotsPlugin;

impl Plugin for BotsPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<BotHit>()
            .add_observer(on_ping_bot)
            .add_systems(
                FixedUpdate,
                (
                    apply_bot_hits,
                    tick_bots,
                    cull_orphan_bots,
                    ensure_bots,
                    wander_bots,
                    expire_pings,
                ),
            );
    }
}

// --- systems -----------------------------------------------------------

/// Keep every started lobby topped up to [`BOTS_ALIVE`] bots.
fn ensure_bots(
    time: Res<Time>,
    mut seq: Local<u64>,
    lobbies: Query<(Entity, &Lobby)>,
    bots: Query<&LobbyBot>,
    mut commands: Commands,
) {
    for (lobby_e, lobby) in &lobbies {
        // `FreeForAll` is pure PvP — no bots.
        if !lobby.started || lobby.mode != GameMode::Freestyle {
            continue;
        }
        let have = bots.iter().filter(|b| b.lobby == lobby_e).count();
        if have >= BOTS_ALIVE {
            continue;
        }
        let members: Vec<PeerId> = lobby.real_peers();
        for _ in have..BOTS_ALIVE {
            *seq = seq.wrapping_add(1);
            let seed = (time.elapsed().as_nanos() as u64)
                ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ lobby_e.to_bits();
            let (pos, yaw) = respawn_pose(seed, lobby.map);
            commands.spawn((
                Name::from("Bot"),
                LobbyBot { lobby: lobby_e },
                BotHealth(BOT_HEALTH),
                BotWander::new(seed, time.elapsed_secs()),
                Bot {
                    pos,
                    yaw,
                    alive: true,
                    fall: 0.0,
                    pinged: false,
                },
                Replicate::to_clients(NetworkTarget::Only(members.clone())),
                InterpolationTarget::to_clients(NetworkTarget::Only(members.clone())),
            ));
        }
    }
}

/// Apply queued hits: mark dead, record the time, score the shooter.
fn apply_bot_hits(
    time: Res<Time>,
    mut events: EventReader<BotHit>,
    mut bots: Query<&mut Bot>,
    mut lobbies: Query<&mut Lobby>,
    mut commands: Commands,
) {
    let mut handled = bevy::platform::collections::HashSet::new();
    for ev in events.read() {
        if !handled.insert(ev.bot) {
            continue;
        }
        let Ok(mut bot) = bots.get_mut(ev.bot) else {
            continue;
        };
        if !bot.alive {
            continue;
        }
        bot.alive = false;
        commands.entity(ev.bot).insert(BotDeath {
            at: time.elapsed_secs(),
        });
        if let Some(mut lobby) = lobbies.iter_mut().find(|l| l.has(ev.by)) {
            if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == ev.by) {
                m.score += ev.points;
            }
        }
        info!("bot {:?} shot by {:?} for {} pts", ev.bot, ev.by, ev.points);
    }
}

/// Topple dead bots, then remove them once their time is up.
fn tick_bots(
    time: Res<Time>,
    lobbies: Query<&Lobby>,
    mut bots: Query<(Entity, &mut Bot, &mut BotDeath, &LobbyBot)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let step = time.delta_secs() / BOT_FALL_SECS;
    for (entity, mut bot, mut death, lb) in &mut bots {
        // Paused: hold the fall, and push the removal back by the pause.
        if lobbies.get(lb.lobby).is_ok_and(|l| l.paused) {
            death.at += time.delta_secs();
            continue;
        }
        if bot.fall < 1.0 {
            bot.fall = (bot.fall + step).min(1.0);
        }
        if now - death.at >= BOT_DEAD_SECS {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Drop bots whose lobby has ended or gone away.
fn cull_orphan_bots(
    bots: Query<(Entity, &LobbyBot)>,
    lobbies: Query<&Lobby>,
    mut commands: Commands,
) {
    for (entity, lb) in &bots {
        let ended = lobbies.get(lb.lobby).map(|l| !l.started).unwrap_or(true);
        if ended {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Walk every living `Freestyle` bot around its map: pick a random spot (from
/// the same spread as [`respawn_pose`]), follow a `nav` route to it at walking
/// pace — never sprinting — stand around a moment, repeat. Movement goes
/// through `ai::move_bot`, so walls stop it and slopes / steps carry it up
/// exactly as they do a `FreeForAll` bot.
fn wander_bots(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    navs: Res<NavGraphs>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<&Lobby>,
    mut bots: Query<(&mut Bot, &mut BotWander, &LobbyBot), Without<BotDeath>>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut bot, mut wander, lb) in &mut bots {
        let Some(lobby) = lobbies.get(lb.lobby).ok().filter(|l| l.started) else {
            continue;
        };
        if !bot.alive || endings.is_frozen(lb.lobby) || lobby.paused {
            continue;
        }
        let world = colliders.world(lobby.map);
        let nav = navs.graph(lobby.map);
        // Plain `&mut`s so fields can be borrowed separately below.
        let (bot, w) = (&mut *bot, &mut *wander);

        // Done standing around: set off for somewhere new.
        if w.path.is_empty() && now >= w.idle_until {
            let seed = (w.roll().to_bits() as u64) << 32 | w.roll().to_bits() as u64;
            let goal = respawn_pose(seed, lobby.map).0;
            match nav.find_path(world, bot.pos, goal) {
                Some(path) if path.len() > 1 => {
                    w.path = path;
                    w.path_index = 0;
                    w.walk_started = now;
                }
                // Unreachable from here — try another spot shortly.
                _ => w.idle_until = now + 0.25,
            }
        }

        let mut wish = Vec3::ZERO;
        if !w.path.is_empty() {
            if w.detour_until > now {
                wish = w.detour_dir;
            } else {
                let mut index = w.path_index;
                let next = NavGraph::next_waypoint(world, &w.path, &mut index, bot.pos);
                w.path_index = index;
                match next {
                    Some(wp) => {
                        wish = Vec3::new(wp.x - bot.pos.x, 0.0, wp.z - bot.pos.z).normalize_or_zero();
                    }
                    None => w.rest(now),
                }
            }
            if now - w.walk_started > MAX_WALK_SECS {
                w.rest(now);
                wish = Vec3::ZERO;
            }
        }

        let moved = move_bot(world, &mut bot.pos, &mut w.vertical_velocity, wish, WALK_SPEED, dt);

        // Face the way it's walking.
        if wish != Vec3::ZERO {
            let want = look_angles(wish).0;
            let max_turn = TURN_DEG_PER_SEC.to_radians() * dt;
            bot.yaw += shortest_angle(bot.yaw, want).clamp(-max_turn, max_turn);
            bot.yaw = shortest_angle(0.0, bot.yaw);
        }

        // Stuck against something → a random detour, then a fresh destination.
        if wish != Vec3::ZERO && moved.blocked {
            w.blocked_for += dt;
        } else {
            w.blocked_for = 0.0;
        }
        if w.blocked_for > STUCK_SECS {
            w.blocked_for = 0.0;
            let turn = (w.roll() - 0.5) * core::f32::consts::PI * 1.5;
            w.detour_dir = Quat::from_rotation_y(turn) * wish;
            w.detour_until = now + DETOUR_SECS;
            // Re-plan from wherever the detour leaves it.
            w.path.truncate(1);
            w.path_index = 1;
        }
        if bot.pos.y < VOID_Y {
            let seed = w.roll().to_bits() as u64;
            bot.pos = respawn_pose(seed, lobby.map).0;
            w.vertical_velocity = 0.0;
            w.rest(now);
        }
    }
}

/// A lobby member pinged a bot: mark it for [`PING_SECS`] (restarting the
/// clock if it's already marked). Only a living bot in the sender's own
/// started `Freestyle` game counts — anything else is ignored.
fn on_ping_bot(
    trigger: Trigger<RemoteTrigger<PingBot>>,
    time: Res<Time>,
    lobbies: Query<(Entity, &Lobby)>,
    mut bots: Query<(&mut Bot, &LobbyBot)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let target = trigger.trigger.bot;
    let Some((lobby_e, _)) = lobbies
        .iter()
        .find(|(_, l)| l.started && l.mode == GameMode::Freestyle && l.has(peer))
    else {
        return;
    };
    let Ok((mut bot, lb)) = bots.get_mut(target) else {
        return;
    };
    if lb.lobby != lobby_e || !bot.alive {
        return;
    }
    if !bot.pinged {
        bot.pinged = true;
    }
    commands.entity(target).insert(BotPing {
        until: time.elapsed_secs() + PING_SECS,
    });
}

/// Clear a ping once its time is up, or as soon as the bot dies.
fn expire_pings(
    time: Res<Time>,
    mut bots: Query<(Entity, &mut Bot, &BotPing)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (entity, mut bot, ping) in &mut bots {
        if now >= ping.until || !bot.alive {
            bot.pinged = false;
            commands.entity(entity).remove::<BotPing>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{EndCam, MapId};

    /// A started `Freestyle` lobby on `map` with one living bot at `feet`.
    fn world(map: MapId, feet: Vec3) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            core::time::Duration::from_secs_f64(1.0 / 64.0),
        ));
        app.insert_resource(MapColliders::load());
        app.insert_resource(NavGraphs::build(&MapColliders::load()));
        app.init_resource::<crate::killcam::EndingLobbies>();
        app.add_systems(Update, wander_bots);
        let lobby = app
            .world_mut()
            .spawn(Lobby {
                name: "test".into(),
                leader: PeerId::Local(1),
                mode: GameMode::Freestyle,
                map,
                started: true,
                time_limit_secs: 300,
                time_left_secs: 300,
                kill_limit: 30,
                end_cam: EndCam::default(),
                round: 0,
                enemies_left: 0,
                paused: false,
                members: Vec::new(),
            })
            .id();
        let bot = app
            .world_mut()
            .spawn((
                LobbyBot { lobby },
                // Already done idling.
                BotWander::new(11, -10.0),
                Bot {
                    pos: feet,
                    yaw: 0.0,
                    alive: true,
                    fall: 0.0,
                    pinged: false,
                },
            ))
            .id();
        (app, bot)
    }

    #[test]
    fn a_freestyle_bot_wanders_at_walking_pace_and_stays_on_the_map() {
        for map in [MapId::BasicMap, MapId::Shipment, MapId::BreakPoint] {
            let (mut app, bot) = world(map, shared::bots::BOT_AREA_CENTER);
            app.update(); // (the first update has no time delta)
            let start = app.world().get::<Bot>(bot).unwrap().pos;
            let mut prev = start;
            let mut farthest: f32 = 0.0;
            for _ in 0..(20 * 64) {
                app.update();
                let pos = app.world().get::<Bot>(bot).unwrap().pos;
                let step = Vec3::new(pos.x - prev.x, 0.0, pos.z - prev.z).length();
                // Never faster than a walk (a little slack for float error).
                assert!(step <= WALK_SPEED / 64.0 + 1e-3, "{map:?}: moved {step} m in a tick");
                assert!(pos.y > VOID_Y, "{map:?}: fell off the map to {pos:?}");
                farthest = farthest.max(pos.distance(start));
                prev = pos;
            }
            assert!(farthest > 2.0, "{map:?}: never went anywhere (max {farthest} m)");
        }
    }

    /// A started `Freestyle` lobby with member `PeerId::Netcode(1)` and one
    /// living bot, running just the ping systems.
    fn ping_world() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            core::time::Duration::from_secs_f64(1.0 / 64.0),
        ));
        app.add_observer(on_ping_bot);
        app.add_systems(Update, expire_pings);
        let lobby = app
            .world_mut()
            .spawn(Lobby {
                name: "test".into(),
                leader: PeerId::Netcode(1),
                mode: GameMode::Freestyle,
                map: MapId::BreakPoint,
                started: true,
                time_limit_secs: 300,
                time_left_secs: 300,
                kill_limit: 30,
                end_cam: EndCam::default(),
                round: 0,
                enemies_left: 0,
                paused: false,
                members: vec![shared::LobbyMember {
                    peer: PeerId::Netcode(1),
                    name: "me".into(),
                    score: 0,
                    loaded: true,
                    bot: None,
                    kills: 0,
                    perks: Vec::new(),
                }],
            })
            .id();
        let bot = app
            .world_mut()
            .spawn((
                LobbyBot { lobby },
                Bot {
                    pos: Vec3::ZERO,
                    yaw: 0.0,
                    alive: true,
                    fall: 0.0,
                    pinged: false,
                },
            ))
            .id();
        app.update(); // (the first update has no time delta)
        (app, bot)
    }

    fn ping(app: &mut App, bot: Entity, from: PeerId) {
        app.world_mut().trigger(RemoteTrigger {
            trigger: PingBot { bot },
            from,
        });
        app.world_mut().flush();
    }

    fn pinged(app: &App, bot: Entity) -> bool {
        app.world().get::<Bot>(bot).unwrap().pinged
    }

    #[test]
    fn a_members_ping_marks_the_bot_for_everyone_then_expires() {
        let (mut app, bot) = ping_world();
        ping(&mut app, bot, PeerId::Netcode(1));
        assert!(pinged(&app, bot));
        // Still marked just short of `PING_SECS`...
        for _ in 0..((PING_SECS - 0.5) * 64.0) as usize {
            app.update();
        }
        assert!(pinged(&app, bot));
        // ...and cleared after it.
        for _ in 0..64 {
            app.update();
        }
        assert!(!pinged(&app, bot));
    }

    #[test]
    fn a_ping_from_outside_the_lobby_is_ignored() {
        let (mut app, bot) = ping_world();
        ping(&mut app, bot, PeerId::Netcode(2));
        assert!(!pinged(&app, bot));
    }

    #[test]
    fn a_pinged_bot_that_dies_loses_its_ping() {
        let (mut app, bot) = ping_world();
        ping(&mut app, bot, PeerId::Netcode(1));
        app.world_mut().get_mut::<Bot>(bot).unwrap().alive = false;
        app.update();
        assert!(!pinged(&app, bot));
        // And a dead bot can't be pinged.
        ping(&mut app, bot, PeerId::Netcode(1));
        assert!(!pinged(&app, bot));
    }

    #[test]
    fn a_dead_bot_does_not_wander() {
        let (mut app, bot) = world(MapId::BreakPoint, shared::bots::BOT_AREA_CENTER);
        app.world_mut().get_mut::<Bot>(bot).unwrap().alive = false;
        for _ in 0..(3 * 64) {
            app.update();
        }
        let pos = app.world().get::<Bot>(bot).unwrap().pos;
        assert_eq!(pos, shared::bots::BOT_AREA_CENTER);
    }
}
