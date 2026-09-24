//! The authoritative tick: accept client-authoritative movement, then resolve
//! every fire request server-side.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use std::collections::HashMap;

use shared::ballistics::{ground_impact, resolve_shot_pierce, Target};
use shared::map::CollisionWorld;
use shared::hitbox::Capsule;
use shared::weapon::WeaponId;
use shared::{
    Bot, GameChannel, GameMode, HitMarker, KnifeAttackSound, Lobby, PlayerId, PlayerInput, PlayerPose,
    RemoteSound, ShotOutcome, ShotResolved, TrickScore,
};

use crate::bots::{BotHealth, BotHit, LobbyBot};
use crate::collision::MapColliders;
use crate::pvp::{PlayerCombat, PlayerHit};
use shared::bots::{BOT_HEAD_RADIUS, BOT_HEIGHT, BOT_RADIUS};

/// Nominal player dimensions for hitbox construction. Replace with per-character
/// values once real models exist.
pub(crate) const PLAYER_HEIGHT: f32 = 1.8;
pub(crate) const PLAYER_RADIUS: f32 = 0.35;
const HEAD_RADIUS: f32 = 0.12;

/// `PlayerPose::translation` is the owner's eye/camera position, not their
/// feet (see `client::net::follow_remote_avatars`, which subtracts this same
/// height to place the remote avatar model) — must track `client::EYE_HEIGHT`.
pub(crate) const EYE_HEIGHT: f32 = 1.7;

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
pub(crate) fn apply_client_pose(
    mut players: Query<(&mut PlayerPose, &ActionState<PlayerInput>, Option<&PlayerCombat>)>,
) {
    for (mut pose, input, combat) in &mut players {
        let i = &input.0;
        pose.translation = Vec3::from_array(i.translation);
        pose.yaw = i.yaw;
        pose.pitch = i.pitch;
        pose.ads_t = i.ads_t;
        pose.crouching = i.crouching;
        pose.reloading = i.reloading;
        pose.jumping = i.jumping;
        pose.sliding = i.sliding;
        // Not from `input` like everything else above — a dead client isn't
        // sending fresh input at all (`client::net::write_input` stops for
        // the duration of their kill cam), so this has to come from our own
        // authoritative combat state instead (`PlayerCombat` is on every
        // player; the `is_none_or` is just a backstop).
        pose.alive = combat.is_none_or(|c| c.alive);
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
    players: Query<(&PlayerId, &PlayerPose, &ActionState<PlayerInput>, &crate::lobby::LobbyPlayer)>,
    lobbies: Query<&Lobby>,
) {
    let server = server.into_inner();
    for (id, pose, input, lp) in &players {
        let bits = input.0.sound_bits;
        if bits == 0 {
            continue;
        }
        // (By the player entity's lobby, not membership — a `Zombies` zombie
        // isn't a member, but its shots and footsteps should still be heard.)
        let Ok(lobby) = lobbies.get(lp.lobby) else {
            continue;
        };
        // Real clients only — a bot player (`ai`) has nobody to send to — and
        // never back to whoever made the sound.
        let targets: Vec<PeerId> = lobby
            .real_peers()
            .into_iter()
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
    colliders: Res<MapColliders>,
    mut sender: ServerMultiMessageSender,
    shooters: Query<(&PlayerId, &ActionState<PlayerInput>, &crate::lobby::LobbyPlayer)>,
    poses: Query<(&PlayerId, &PlayerPose, &crate::lobby::LobbyPlayer)>,
    mut bots: Query<(Entity, &Bot, &LobbyBot, &mut BotHealth)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    lobbies: Query<(Entity, &Lobby)>,
    mut bot_hits: EventWriter<BotHit>,
    mut player_hits: EventWriter<PlayerHit>,
) {
    let tick = timeline.tick().0;
    let server = server.into_inner();

    // A player who's dead and waiting to respawn (`PlayerCombat::alive ==
    // false` — after a `FreeForAll` kill or a fatal fall in either mode) can
    // neither shoot nor be shot.
    let is_alive = |peer: PeerId| {
        combats
            .iter()
            .find(|(id, _)| id.0 == peer)
            .map(|(_, c)| c.alive)
            .unwrap_or(true)
    };

    for (shooter, input, shooter_lp) in &shooters {
        let i = &input.0;
        if !(i.fire || i.melee) || !is_alive(shooter.0) {
            continue;
        }
        // A shot only touches the shooter's own lobby's game. (By the shooter's
        // player entity, not lobby membership: a `Zombies` zombie shoots too.)
        let Ok((lobby_e, lobby)) = lobbies.get(shooter_lp.lobby) else {
            continue;
        };
        if !lobby.started {
            continue;
        }
        let zombies = lobby.mode == GameMode::Zombies;

        let mut kind: HashMap<u64, HitKind> = HashMap::new();
        let mut targets: Vec<Target> = Vec::new();

        for (id, pose, lp) in &poses {
            if id.0 == shooter.0 || lp.lobby != lobby_e || !is_alive(id.0) {
                continue;
            }
            // `Zombies`: players and zombies only hit each other — no friendly
            // fire on either side.
            if zombies
                && shared::bot_players::is_bot_peer(id.0)
                    == shared::bot_players::is_bot_peer(shooter.0)
            {
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
            for (entity, bot, lb, _) in &bots {
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

        // A knife stab: no ballistics, no tracer — the nearest target that's
        // close and roughly under the crosshair dies. Bots (`Freestyle`) score
        // flat knife-kill points; players (`FreeForAll`) take lethal damage
        // through the same `PlayerHit` a bullet uses, so death, respawn, kill
        // credit and the victim's kill cam all follow for free.
        if i.melee {
            // Where the stab landed, if it hit a valid target — for the lobby's
            // stab sound. Anything else (a whiff, or a player in `Freestyle`,
            // where they aren't a target) is a swing.
            let mut stabbed_at: Option<Vec3> = None;
            let hit = shared::melee::resolve_melee(origin, dir, &targets);
            match hit.and_then(|h| kind.get(&h.target).map(|k| (h, k))) {
                Some((hit, HitKind::Bot(bot))) => {
                    stabbed_at = Some(hit.point);
                    let (points, lines) = shared::scoring::score_knife_kill();
                    bot_hits.write(BotHit {
                        bot: *bot,
                        by: shooter.0,
                        points,
                    });
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
                    info!("tick {tick}: {:?} knifed a bot for {points} pts", shooter.0);
                }
                Some((hit, HitKind::Player(victim))) if lobby.mode != GameMode::Freestyle => {
                    stabbed_at = Some(hit.point);
                    player_hits.write(PlayerHit {
                        victim: *victim,
                        killer: shooter.0,
                        damage: shared::melee::KNIFE_DAMAGE,
                    });
                    info!("tick {tick}: {:?} knifed player {:?}", shooter.0, victim);
                }
                _ => {}
            }

            // Everyone in the lobby hears it: a stab from where it landed, a
            // swing from the attacker.
            let swing_at = poses
                .iter()
                .find(|(id, ..)| id.0 == shooter.0)
                .map_or(origin, |(_, pose, _)| pose.translation);
            let members: Vec<PeerId> = lobby.real_peers();
            let variant = ((tick as u64) ^ shooter.0.to_bits())
                .wrapping_mul(0x2545_F491_4F6C_DD1D)
                >> 56;
            let msg = KnifeAttackSound {
                point: stabbed_at.unwrap_or(swing_at).to_array(),
                stab: stabbed_at.is_some(),
                variant: variant as u8,
            };
            if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(members)) {
                error!("failed to send knife attack sound: {e:?}");
            }
            continue;
        }

        let Some(weapon) = WeaponId::from_u8(i.weapon) else {
            continue;
        };

        // The nearest solid surface along the shot (wall, container, crate,
        // ground mesh). A bullet stops there: nothing beyond it can be hit —
        // no wallbangs (yet) — and the tracer ends on it.
        let aim = dir.normalize_or_zero();
        let wall = colliders
            .world(lobby.map)
            .raycast(origin, aim, weapon.spec().max_range);
        let wall_dist = wall.map(|h| h.distance);

        // The tracer's true endpoint — captured up front so it's correct even
        // for a bot kill, which `outcome` below reports as `Miss` (bots aren't
        // a valid `ShotOutcome::Hit` target).
        let mut tracer_end = origin + aim * weapon.spec().max_range;

        // Collateral: the bullet keeps going after a bot (Call-of-Duty style),
        // so a single shot can pierce through several — `hits` is every one it
        // reached, nearest first.
        let hits = resolve_shot_pierce(
            weapon,
            origin,
            dir,
            &targets,
            // Occlusion: a target whose hit point lies farther along the shot
            // than the first solid surface is behind it, so it can't be hit.
            |_from, to| wall_dist.is_some_and(|d| origin.distance(to) > d),
        );

        let outcome = match hits.last() {
            Some(last) => {
                tracer_end = last.point;

                // Every bot the shot pierced takes that hit's damage off its
                // health (a leg shot, say, leaves it alive); the ones that
                // reach zero die. Each dead bot becomes its own `BotHit` (each
                // needs to topple independently), but the shot's score is
                // worked out once for the whole chain and multiplied by how
                // many it *killed* — crediting it again per bot would double
                // it, so only the first `BotHit` carries the points. The
                // distance multiplier goes off the nearest (first) bot killed
                // — "how far the shot was", regardless of how much farther it
                // happened to pierce. A bot that survives just gives the
                // shooter a hit marker.
                let mut bots_hit: Vec<(Entity, f32)> = Vec::new();
                let mut survivor = false;
                for hit in &hits {
                    let Some(HitKind::Bot(bot)) = kind.get(&hit.target) else {
                        continue;
                    };
                    let Ok((_, _, _, mut health)) = bots.get_mut(*bot) else {
                        continue;
                    };
                    health.0 -= hit.damage;
                    if health.0 <= 0.0 {
                        bots_hit.push((*bot, hit.distance));
                    } else {
                        survivor = true;
                    }
                }
                if survivor && !shared::bot_players::is_bot_peer(shooter.0) {
                    if let Err(e) = sender.send::<_, GameChannel>(
                        &HitMarker,
                        server,
                        &NetworkTarget::Single(shooter.0),
                    ) {
                        error!("failed to send hit marker: {e:?}");
                    }
                }
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
                        if lobby.mode != GameMode::Freestyle {
                            // A zombie's hit is capped — see `ZOMBIE_HIT_DAMAGE`.
                            let damage = if zombies && shared::bot_players::is_bot_peer(shooter.0) {
                                hit.damage.min(shared::ZOMBIE_HIT_DAMAGE)
                            } else {
                                hit.damage
                            };
                            player_hits.write(PlayerHit {
                                victim: *p,
                                killer: shooter.0,
                                damage,
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
            None => {
                // Nothing hit: the shot ends on whichever comes first, the
                // map's own surface or the flat ground plane fallback.
                // Each candidate carries its surface normal (the mesh's own, or
                // straight up for the flat plane) for the clients' bullet hole.
                let mesh_hit = wall.map(|h| (origin + aim * h.distance, h.normal));
                let plane_hit = ground_impact(origin, dir).map(|p| (p, Vec3::Y));
                let surface = match (mesh_hit, plane_hit) {
                    (Some(w), Some(g)) => Some(if origin.distance(w.0) <= origin.distance(g.0) {
                        w
                    } else {
                        g
                    }),
                    (w, g) => w.or(g),
                };
                match surface {
                    Some((p, normal)) => {
                        tracer_end = p;
                        ShotOutcome::Ground {
                            point: p.to_array(),
                            normal: normal.to_array(),
                        }
                    }
                    None => ShotOutcome::Miss,
                }
            }
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
