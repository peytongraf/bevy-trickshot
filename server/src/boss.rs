//! `Zombies` bosses (`shared::boss`), on top of `crate::zombies`' rounds:
//! that queues each boss of a boss round ([`PendingBoss`]); this strikes
//! its orange lightning where it'll appear, brings it in once the strike's
//! done, and flies the energy blasts it hurls (`ai::drive_bots` decides when,
//! [`ai::BossBlastFired`]) — each one straight on until it touches a player
//! or the map (or runs out of time), then blowing up, hurting every player
//! in reach by how close they were ([`shared::boss::blast_damage`]).
//!
//! A boss is a zombie (`Zombie`) to everything else: scored, counted and
//! culled like one (its corpse lies there for its death animation, then
//! `zombies::clear_dead_zombies` removes it). Blasts in flight belong to
//! their lobby's game and are dropped the moment it isn't running.

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::Server;
use lightyear::prelude::*;

use shared::boss::{blast_damage, BLAST_HIT_RADIUS, BLAST_LIFETIME_SECS, BLAST_SPEED};
use shared::bot_players::{bot_peer, is_bot_peer, BotDifficulty};
use shared::hitbox::{ray_capsule, Capsule};
use shared::map::CollisionWorld;
use shared::{GameChannel, GameMode, Lobby, PlayerId, PlayerInput, PlayerName, PlayerPose, SpawnBoss};

use crate::ai::{BossBlastFired, BotBrain, NextBotId};
use crate::collision::MapColliders;
use crate::lobby::LobbyPlayer;
use crate::nav::NavGraphs;
use crate::pvp::{PlayerCombat, PlayerHit};
use crate::sim::{EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_RADIUS};
use crate::zombies::{Zombie, ZombieRounds};

/// On a boss's player entity (along with `Zombie`).
#[derive(Component)]
pub struct Boss;

/// A boss round's boss, struck in by lightning but not here yet: it appears
/// at `feet`, facing `yaw`, at `spawn_at` (`Time::elapsed_secs`).
pub(crate) struct PendingBoss {
    pub(crate) feet: Vec3,
    pub(crate) yaw: f32,
    pub(crate) spawn_at: f32,
    /// Its lightning's been sent to the lobby.
    pub(crate) announced: bool,
}

impl PendingBoss {
    /// A boss to strike in at `feet`, facing `toward`, from `now`.
    pub(crate) fn new(feet: Vec3, toward: Vec3, now: f32) -> Self {
        let to = toward - feet;
        Self {
            feet,
            yaw: f32::atan2(-to.x, -to.z),
            spawn_at: now + shared::boss::BOSS_PRE_SPAWN_SECS,
            announced: false,
        }
    }
}

/// One blast in flight.
struct Blast {
    id: u32,
    lobby: Entity,
    /// The boss that hurled it (who a kill is credited to).
    boss: PeerId,
    pos: Vec3,
    dir: Vec3,
    /// Seconds it's been flying (not counting pauses).
    age: f32,
}

/// Every blast in flight, and the next blast's id.
#[derive(Resource, Default)]
struct Blasts {
    flying: Vec<Blast>,
    next_id: u32,
}

pub struct BossPlugin;

impl Plugin for BossPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Blasts>()
            .add_observer(on_spawn_boss)
            .add_systems(
                FixedUpdate,
                (
                    bring_in_bosses
                        .after(crate::zombies::run_rounds)
                        .before(crate::ai::drive_bots),
                    (launch_blasts, run_blasts).chain().after(crate::ai::drive_bots),
                    send_knockbacks.after(crate::ai::drive_bots),
                ),
            );
    }
}

fn send_to_lobby<M: lightyear::prelude::Message>(
    sender: &mut ServerMultiMessageSender,
    server: &Server,
    lobby: &Lobby,
    msg: &M,
) {
    if let Err(e) = sender.send::<_, GameChannel>(msg, server, &NetworkTarget::Only(lobby.real_peers())) {
        error!("failed to send a boss message: {e:?}");
    }
}

/// Strike each queued boss's lightning, and bring it in once it's due.
fn bring_in_bosses(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut next_id: ResMut<NextBotId>,
    mut lobbies: Query<(Entity, &Lobby, &mut ZombieRounds)>,
    mut quotes: EventWriter<crate::quotes::SayQuote>,
    mut commands: Commands,
) {
    let server = server.into_inner();
    let now = time.elapsed_secs();
    for (lobby_e, lobby, mut rounds) in &mut lobbies {
        if lobby.paused {
            continue;
        }
        let round = rounds.round.max(1);
        let mut i = 0;
        while i < rounds.pending_bosses.len() {
            let boss = &mut rounds.pending_bosses[i];
            if !boss.announced {
                boss.announced = true;
                send_to_lobby(&mut sender, server, lobby, &shared::BossLightning { at: boss.feet.to_array() });
            }
            if now < boss.spawn_at {
                i += 1;
                continue;
            }
            let boss = rounds.pending_bosses.swap_remove(i);
            // The game's first boss round's, and its second's: someone says
            // so (not an exfil's — that brings its own).
            if rounds.exfil.is_none() && rounds.last_boss_round != rounds.round {
                rounds.last_boss_round = rounds.round;
                rounds.boss_rounds_seen += 1;
                let quote = match rounds.boss_rounds_seen {
                    1 => Some(shared::quotes::Quote::FirstBoss),
                    2 => Some(shared::quotes::Quote::BossRepeat),
                    _ => None,
                };
                if let Some(quote) = quote {
                    quotes.write(crate::quotes::SayQuote::anyone(lobby_e, quote));
                }
            }

            let peer = bot_peer(next_id.0);
            next_id.0 += 1;
            let seed = (now.to_bits() as u64) << 20 ^ next_id.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ lobby_e.to_bits();
            // Quick to notice you and to turn on you.
            let mut skill = crate::zombies::zombie_skill(round);
            skill.reaction_secs = skill.reaction_secs.min(0.4);
            skill.turn_deg_per_sec = skill.turn_deg_per_sec.max(160.0);
            skill.sight_range = skill.sight_range.max(shared::boss::BLAST_MAX_RANGE + 10.0);
            let health = shared::boss::boss_health(round, lobby.real_count());
            let real = lobby.real_peers();
            commands.spawn((
                Name::from("Boss"),
                Zombie,
                Boss,
                LobbyPlayer { lobby: lobby_e },
                PlayerId(peer),
                PlayerName("Boss".into()),
                PlayerPose {
                    translation: boss.feet + Vec3::Y * EYE_HEIGHT,
                    yaw: boss.yaw,
                    zombie: shared::ZombieAnim::BossIdle,
                    ..default()
                },
                ActionState::<PlayerInput>::default(),
                BotBrain::new(BotDifficulty::Recruit, boss.feet, seed)
                    .facing(boss.yaw)
                    .with_skill(skill)
                    .boss(now),
                PlayerCombat::zombie(health),
                shared::PlayerHealth(health),
                Replicate::to_clients(NetworkTarget::Only(real.clone())),
                InterpolationTarget::to_clients(NetworkTarget::Only(real)),
            ));
            send_to_lobby(&mut sender, server, lobby, &shared::BossSpawned { at: boss.feet.to_array() });
            info!("lobby {lobby_e:?}: a boss appeared ({health:.0} health)");
        }
    }
}

/// A boss let go of a blast: start it flying, and tell the lobby.
fn launch_blasts(
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut fired: EventReader<BossBlastFired>,
    lobbies: Query<&Lobby>,
    mut blasts: ResMut<Blasts>,
) {
    let server = server.into_inner();
    for ev in fired.read() {
        let Ok(lobby) = lobbies.get(ev.lobby) else { continue };
        blasts.next_id = blasts.next_id.wrapping_add(1);
        let id = blasts.next_id;
        blasts.flying.push(Blast {
            id,
            lobby: ev.lobby,
            boss: ev.boss,
            pos: ev.from,
            dir: ev.dir,
            age: 0.0,
        });
        send_to_lobby(
            &mut sender,
            server,
            lobby,
            &shared::BossBlastLaunched {
                id,
                boss: ev.boss,
                from: ev.from.to_array(),
                dir: ev.dir.to_array(),
                speed: BLAST_SPEED,
            },
        );
    }
}

/// Fly every blast on, and blow it up on the first player or wall it
/// touches (or once it's flown its lifetime).
#[allow(clippy::too_many_arguments)]
fn run_blasts(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    colliders: Res<MapColliders>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<&Lobby>,
    players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &PlayerCombat), Without<Zombie>>,
    mut blasts: ResMut<Blasts>,
    mut hits: EventWriter<PlayerHit>,
) {
    let server = server.into_inner();
    let dt = time.delta_secs();
    let mut i = 0;
    while i < blasts.flying.len() {
        let blast = &mut blasts.flying[i];
        // Its game's over (or gone): it just goes.
        let Some(lobby) = lobbies
            .get(blast.lobby)
            .ok()
            .filter(|l| l.started && l.mode == GameMode::Zombies && !endings.is_frozen(blast.lobby))
        else {
            blasts.flying.swap_remove(i);
            continue;
        };
        if lobby.paused {
            i += 1;
            continue;
        }
        let step = BLAST_SPEED * dt;
        let world = colliders.for_lobby(lobby);
        // The nearest thing it touches this tick, as a distance along its
        // path: the map...
        let mut hit_at: Option<f32> = world
            .sweep_sphere(blast.pos, blast.pos + blast.dir * step, BLAST_HIT_RADIUS * 0.5)
            .map(|h| h.fraction * step);
        // ...or a player (their body, fattened by the ball's size).
        for (id, pose, lp, combat) in &players {
            if lp.lobby != blast.lobby || is_bot_peer(id.0) || !combat.alive {
                continue;
            }
            let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
            let body = Capsule::standing(feet, PLAYER_HEIGHT, PLAYER_RADIUS + BLAST_HIT_RADIUS);
            if let Some(t) = ray_capsule(blast.pos, blast.dir, &body).filter(|t| *t <= step) {
                hit_at = Some(hit_at.map_or(t, |h| h.min(t)));
            }
        }
        blast.age += dt;
        let at = match hit_at {
            Some(t) => blast.pos + blast.dir * t,
            None if blast.age >= BLAST_LIFETIME_SECS => blast.pos,
            None => {
                blast.pos += blast.dir * step;
                i += 1;
                continue;
            }
        };

        // Boom: everyone in reach takes it by how close they were (to the
        // nearest part of them — a blast at their feet is a direct hit).
        for (id, pose, lp, combat) in &players {
            if lp.lobby != blast.lobby || is_bot_peer(id.0) || !combat.alive {
                continue;
            }
            let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
            let d = shared::boss::segment_point_distance(feet, pose.translation, at);
            let damage = blast_damage(d);
            if damage > 0.0 {
                hits.write(PlayerHit {
                    cause: crate::pvp::HitCause::Enemy,
                    victim: id.0,
                    killer: blast.boss,
                    damage,
                    bomb_shot: false,
                    blast: false,
                    critical: false,
                    point: None,
                });
            }
        }
        send_to_lobby(
            &mut sender,
            server,
            lobby,
            &shared::BossBlastExploded {
                id: blast.id,
                at: at.to_array(),
            },
        );
        blasts.flying.swap_remove(i);
    }
}

/// Debug: the party leader strikes a boss in, 15 m in front of them (on the
/// nearest walkable spot) — only in a running, unpaused `Zombies` game.
fn on_spawn_boss(
    trigger: Trigger<RemoteTrigger<SpawnBoss>>,
    time: Res<Time>,
    navs: Res<NavGraphs>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<(Entity, &Lobby)>,
    mut rounds: Query<&mut ZombieRounds>,
    players: Query<(&PlayerId, &PlayerPose)>,
) {
    let peer = trigger.from;
    let Some((lobby_e, lobby)) = lobbies
        .iter()
        .find(|(_, l)| l.leader == peer && l.started && l.mode == GameMode::Zombies && !l.paused)
    else {
        return;
    };
    if endings.is_ending(lobby_e) {
        return;
    }
    let (Ok(mut rounds), Some((_, pose))) = (rounds.get_mut(lobby_e), players.iter().find(|(id, _)| id.0 == peer))
    else {
        return;
    };
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    let forward = Vec3::new(-pose.yaw.sin(), 0.0, -pose.yaw.cos());
    let nav = navs.graph(lobby.map);
    let Some(spot) = nav.nearest_spot(feet + forward * 15.0).or_else(|| nav.nearest_spot(feet)) else {
        return;
    };
    rounds.pending_bosses.push(PendingBoss::new(spot, feet, time.elapsed_secs()));
    info!("lobby {lobby_e:?}: {peer:?} struck a boss in (debug)");
}

/// A smash landed: throw its victim back (their client moves them —
/// movement's theirs — and keeps them out of the walls).
fn send_knockbacks(
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut smashes: EventReader<crate::ai::BossSmashed>,
) {
    let server = server.into_inner();
    for smash in smashes.read() {
        if shared::bot_players::is_bot_peer(smash.victim) {
            continue;
        }
        let msg = shared::KnockedBack {
            velocity: (smash.dir * shared::boss::SMASH_KNOCKBACK_SPEED).to_array(),
        };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(smash.victim)) {
            error!("failed to send knockback to {:?}: {e:?}", smash.victim);
        }
    }
}
