//! The authoritative tick: accept client-authoritative movement, then resolve
//! every fire request server-side.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use std::collections::HashMap;

use shared::ballistics::{ground_impact, resolve_shot_pierce, Target};
use shared::hitbox::Capsule;
use shared::weapon::WeaponId;
use shared::{
    Bot, GameChannel, GameMode, Lobby, PlayerId, PlayerInput, PlayerPose, RemoteSound, ShotOutcome,
    ShotResolved, TrickScore,
};

use crate::bots::{BotHit, LobbyBot};
use crate::pvp::{PlayerCombat, PlayerHit};
use shared::bots::{BOT_HEAD_RADIUS, BOT_HEIGHT, BOT_RADIUS};

/// Nominal player dimensions for hitbox construction. Replace with per-character
/// values once real models exist.
const PLAYER_HEIGHT: f32 = 1.8;
const PLAYER_RADIUS: f32 = 0.35;
const HEAD_RADIUS: f32 = 0.12;

/// `PlayerPose::translation` is the owner's eye/camera position, not their
/// feet (see `client::net::follow_remote_avatars`, which subtracts this same
/// height to place the remote avatar model) — must track `client::EYE_HEIGHT`.
const EYE_HEIGHT: f32 = 1.7;

/// What a resolved hit landed on.
enum HitKind {
    Player(PeerId),
    Bot(Entity),
}

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (apply_client_pose, resolve_shots, broadcast_remote_sounds).chain(),
        );
    }
}

/// Client-authoritative movement: publish the owner's reported pose as-is.
fn apply_client_pose(mut players: Query<(&mut PlayerPose, &ActionState<PlayerInput>)>) {
    for (mut pose, input) in &mut players {
        let i = &input.0;
        pose.translation = Vec3::from_array(i.translation);
        pose.yaw = i.yaw;
        pose.pitch = i.pitch;
        pose.ads_t = i.ads_t;
        pose.crouching = i.crouching;
        pose.reloading = i.reloading;
        pose.jumping = i.jumping;
        pose.sliding = i.sliding;
    }
}

/// Relay each player's one-shot sound bits (already sent every tick for the
/// kill cam — see `KillCam`) live to the rest of their lobby, stamped with
/// their current position, so other clients can play them back positionally.
/// Never sent back to the player who triggered them; they already hear their
/// own local, non-spatial version of these sounds.
fn broadcast_remote_sounds(
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    players: Query<(&PlayerId, &PlayerPose, &ActionState<PlayerInput>)>,
    lobbies: Query<&Lobby>,
) {
    let server = server.into_inner();
    for (id, pose, input) in &players {
        let bits = input.0.sound_bits;
        if bits == 0 {
            continue;
        }
        let Some(lobby) = lobbies.iter().find(|l| l.has(id.0)) else {
            continue;
        };
        let targets: Vec<PeerId> = lobby
            .members
            .iter()
            .map(|m| m.peer)
            .filter(|&peer| peer != id.0)
            .collect();
        if targets.is_empty() {
            continue;
        }
        let msg = RemoteSound {
            player: id.0,
            bits,
            position: pose.translation.to_array(),
        };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(targets)) {
            error!("failed to broadcast remote sound: {e:?}");
        }
    }
}

/// Server-authoritative shots: for each client that fired this tick, ray-cast
/// against the other players **and the bots** in that client's lobby. A bot hit
/// becomes a [`BotHit`] event (scored + toppled in [`crate::bots`]); a player
/// hit is broadcast as [`ShotResolved`] as before.
#[allow(clippy::too_many_arguments)]
fn resolve_shots(
    timeline: Single<&LocalTimeline, With<Server>>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    shooters: Query<(&PlayerId, &ActionState<PlayerInput>)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    bots: Query<(Entity, &Bot, &LobbyBot)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    lobbies: Query<(Entity, &Lobby)>,
    mut bot_hits: EventWriter<BotHit>,
    mut player_hits: EventWriter<PlayerHit>,
) {
    let tick = timeline.tick().0;
    let server = server.into_inner();

    // A `FreeForAll` player mid-respawn (`PlayerCombat::alive == false`) can
    // neither shoot nor be shot; `Freestyle` players never get a
    // `PlayerCombat` at all, so they're always "alive" here.
    let is_alive = |peer: PeerId| {
        combats
            .iter()
            .find(|(id, _)| id.0 == peer)
            .map(|(_, c)| c.alive)
            .unwrap_or(true)
    };

    for (shooter, input) in &shooters {
        let i = &input.0;
        if !i.fire || !is_alive(shooter.0) {
            continue;
        }
        let Some(weapon) = WeaponId::from_u8(i.weapon) else {
            continue;
        };
        // A shot only touches the shooter's own lobby's game.
        let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| l.has(shooter.0)) else {
            continue;
        };

        let mut kind: HashMap<u64, HitKind> = HashMap::new();
        let mut targets: Vec<Target> = Vec::new();

        for (id, pose) in &poses {
            if id.0 == shooter.0 || !lobby.has(id.0) || !is_alive(id.0) {
                continue;
            }
            let key = id.0.to_bits();
            kind.insert(key, HitKind::Player(id.0));
            let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
            targets.push(Target {
                id: key,
                body: Capsule::standing(feet, PLAYER_HEIGHT, PLAYER_RADIUS),
                head: Capsule::head(feet, PLAYER_HEIGHT, HEAD_RADIUS),
            });
        }
        // No bots in `FreeForAll` — it's pure PvP.
        if lobby.mode == GameMode::Freestyle {
            for (entity, bot, lb) in &bots {
                if lb.lobby != lobby_e || !bot.alive {
                    continue;
                }
                let key = entity.to_bits();
                kind.insert(key, HitKind::Bot(entity));
                targets.push(Target {
                    id: key,
                    body: Capsule::standing(bot.pos, BOT_HEIGHT, BOT_RADIUS),
                    head: Capsule::head(bot.pos, BOT_HEIGHT, BOT_HEAD_RADIUS),
                });
            }
        }

        let origin = Vec3::from_array(i.fire_origin);
        let dir = Vec3::from_array(i.fire_dir);

        // The tracer's true endpoint — captured up front so it's correct even
        // for a bot kill, which `outcome` below reports as `Miss` (bots aren't
        // a valid `ShotOutcome::Hit` target).
        let mut tracer_end = origin + dir.normalize_or_zero() * weapon.spec().max_range;

        // Collateral: the bullet keeps going after a bot (Call-of-Duty style),
        // so a single shot can pierce through several — `hits` is every one it
        // reached, nearest first.
        let hits = resolve_shot_pierce(
            weapon,
            origin,
            dir,
            &targets,
            // TODO: swap for your map's occlusion test — shared::map::CollisionWorld.
            |_from, _to| false,
        );

        let outcome = match hits.last() {
            Some(last) => {
                tracer_end = last.point;

                // Every bot the shot pierced becomes its own `BotHit` (each
                // needs to die + topple independently), but the shot's score
                // is worked out once for the whole chain and multiplied by how
                // many it hit — crediting it again per bot would double it, so
                // only the first `BotHit` carries the points. The distance
                // multiplier goes off the nearest (first) bot the shot hit —
                // that's "how far the shot was", regardless of how much
                // farther it happened to pierce.
                let bots_hit: Vec<(Entity, f32)> = hits
                    .iter()
                    .filter_map(|hit| match kind.get(&hit.target) {
                        Some(HitKind::Bot(bot)) => Some((*bot, hit.distance)),
                        _ => None,
                    })
                    .collect();
                if let Some(&(_, distance)) = bots_hit.first() {
                    let (points, lines) = shared::scoring::score_multi_kill(
                        i.spin_deg,
                        i.airborne,
                        i.noscope,
                        bots_hit.len() as u32,
                        distance,
                        weapon.spec().max_range,
                    );
                    for (idx, (bot, _)) in bots_hit.iter().enumerate() {
                        bot_hits.write(BotHit {
                            bot: *bot,
                            by: shooter.0,
                            points: if idx == 0 { points } else { 0 },
                        });
                    }
                    let trick = TrickScore {
                        shooter: shooter.0,
                        total: points,
                        lines,
                    };
                    if let Err(e) =
                        sender.send::<_, GameChannel>(&trick, server, &NetworkTarget::All)
                    {
                        error!("failed to broadcast trick score: {e:?}");
                    }
                    if bots_hit.len() > 1 {
                        info!(
                            "tick {tick}: {:?} got a {}-bot COLLATERAL for {points} pts",
                            shooter.0,
                            bots_hit.len(),
                        );
                    } else {
                        info!("tick {tick}: {:?} killed a bot for {points} pts", shooter.0);
                    }
                }

                // A player hit (if the pierced chain reached one) is still
                // reported for hit-marker / damage feedback — the nearest one,
                // same as before piercing existed.
                match hits.iter().find_map(|hit| match kind.get(&hit.target) {
                    Some(HitKind::Player(p)) => Some((p, hit)),
                    _ => None,
                }) {
                    Some((p, hit)) => {
                        info!(
                            "tick {tick}: {:?} {} player {:?}",
                            shooter.0,
                            if hit.headshot { "HEADSHOT on" } else { "hit" },
                            p,
                        );
                        if lobby.mode == GameMode::FreeForAll {
                            player_hits.write(PlayerHit {
                                victim: *p,
                                killer: shooter.0,
                                damage: hit.damage,
                            });
                        }
                        ShotOutcome::Hit {
                            target: p.to_bits(),
                            headshot: hit.headshot,
                            point: hit.point.to_array(),
                            damage: hit.damage,
                        }
                    }
                    None => ShotOutcome::Miss,
                }
            }
            None => match ground_impact(origin, dir) {
                Some(p) => {
                    tracer_end = p;
                    ShotOutcome::Ground { point: p.to_array() }
                }
                None => ShotOutcome::Miss,
            },
        };

        let msg = ShotResolved {
            shooter: shooter.0,
            tick,
            outcome,
            origin: origin.to_array(),
            tracer_end: tracer_end.to_array(),
        };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::All) {
            error!("failed to broadcast shot result: {e:?}");
        }
    }
}
