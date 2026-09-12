//! Server-owned practice bots.
//!
//! Bots are stationary capsules replicated to the members of one lobby's game.
//! A hit (resolved in [`crate::sim`]) sends a [`BotHit`] event; here we mark the
//! bot dead, tip it over, score the shooter, and after a couple of seconds
//! despawn it. [`ensure_bots`] then refills the lobby's bot count at a fresh
//! spot near the map centre — so respawn is just "despawn + top up".

use bevy::prelude::*;

use lightyear::prelude::*;

use shared::bots::{respawn_pose, BOTS_ALIVE, BOT_DEAD_SECS, BOT_FALL_SECS};
use shared::{Bot, GameMode, Lobby};

/// A hit on a bot, written by [`crate::sim::resolve_shots`].
#[derive(Event)]
pub struct BotHit {
    pub bot: Entity,
    pub by: PeerId,
    /// Style points the kill is worth (`shared::scoring::score_kill`).
    pub points: u32,
}

/// Which lobby's game a bot belongs to.
#[derive(Component)]
pub struct LobbyBot {
    pub lobby: Entity,
}

/// Wall-clock time (seconds) a bot was shot. Absent while alive.
#[derive(Component)]
struct BotDeath {
    at: f32,
}

pub struct BotsPlugin;

impl Plugin for BotsPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<BotHit>().add_systems(
            FixedUpdate,
            (apply_bot_hits, tick_bots, cull_orphan_bots, ensure_bots),
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
        let members: Vec<PeerId> = lobby.members.iter().map(|m| m.peer).collect();
        for _ in have..BOTS_ALIVE {
            *seq = seq.wrapping_add(1);
            let seed = (time.elapsed().as_nanos() as u64)
                ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ lobby_e.to_bits();
            let (pos, yaw) = respawn_pose(seed, lobby.map);
            commands.spawn((
                Name::from("Bot"),
                LobbyBot { lobby: lobby_e },
                Bot {
                    pos,
                    yaw,
                    alive: true,
                    fall: 0.0,
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
    mut bots: Query<(Entity, &mut Bot, &BotDeath)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let step = time.delta_secs() / BOT_FALL_SECS;
    for (entity, mut bot, death) in &mut bots {
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
