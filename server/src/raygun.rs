//! Ray Gun bolts, flown for real. A Ray Gun shot isn't resolved the tick
//! it's fired (`sim::resolve_shots` hands it to [`fire`]): its bolt flies
//! at [`RAYGUN_BOLT_SPEED`], and only when it reaches the first zombie in
//! its way — or the surface it was headed for — does it burst, Call of
//! Duty's way: the zombie it hit takes the bolt, every other zombie close
//! by takes the burst ([`raygun_splash_damage`]), and whatever survives is
//! stunned (as PhD Flopper's blast stuns). That's when everyone hears of
//! it ([`ShotResolved`]); the rest of the lobby saw the bolt leave the
//! barrel ([`RayGunFired`]).
//!
//! A bolt dies with its game: one whose lobby is gone or no longer started
//! is dropped, and a paused one waits in the air.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::bot_players::is_bot_peer;
use shared::hitbox::{ray_capsule, Capsule};
use shared::weapon::{raygun_splash_damage, WeaponId, RAYGUN_BOLT_SPEED, RAYGUN_STUN_SECS};
use shared::{GameChannel, Lobby, PlayerId, PlayerPose, RayGunFired, ShotOutcome, ShotResolved};

use crate::ai::BotBrain;
use crate::lobby::LobbyPlayer;
use crate::pvp::{PlayerCombat, PlayerHit};
use crate::sim::{pap_mult, EYE_HEIGHT, HEAD_RADIUS, PLAYER_HEIGHT, PLAYER_RADIUS};

/// A bolt in flight.
struct Bolt {
    lobby: Entity,
    shooter: PeerId,
    /// Where it was fired from, and where it is now.
    origin: Vec3,
    pos: Vec3,
    dir: Vec3,
    /// How much further (m) it can go: to `surface`, or its max range.
    left: f32,
    /// The surface (point, normal) it bursts on if nothing gets in the way
    /// first — `None`: off into the sky, where it fizzles out.
    surface: Option<(Vec3, Vec3)>,
}

/// Every Ray Gun bolt still in the air, across all lobbies.
#[derive(Resource, Default)]
pub(crate) struct RayGunBolts(Vec<Bolt>);

/// Launch `shooter`'s bolt from `origin` along `aim` (unit length), headed
/// for `surface` (from `sim::first_surface`), and show the rest of the
/// lobby it leaving the barrel.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire(
    bolts: &mut RayGunBolts,
    sender: &mut ServerMultiMessageSender,
    server: &Server,
    lobby_e: Entity,
    lobby: &Lobby,
    shooter: PeerId,
    origin: Vec3,
    aim: Vec3,
    surface: Option<(Vec3, Vec3)>,
) {
    let max_range = WeaponId::RayGun.spec().max_range;
    let left = surface.map_or(max_range, |(p, _)| origin.distance(p).min(max_range));
    bolts.0.push(Bolt {
        lobby: lobby_e,
        shooter,
        origin,
        pos: origin,
        dir: aim,
        left,
        surface,
    });
    let others: Vec<PeerId> = lobby.real_peers().into_iter().filter(|&p| p != shooter).collect();
    if others.is_empty() {
        return;
    }
    let msg = RayGunFired {
        shooter,
        origin: origin.to_array(),
        end: (origin + aim * left).to_array(),
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(others)) {
        error!("failed to send ray gun bolt: {e:?}");
    }
}

/// Fly every bolt one tick, bursting the ones that reach a zombie or their
/// surface.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fly_bolts(
    time: Res<Time>,
    timeline: Single<&LocalTimeline, With<Server>>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut bolts: ResMut<RayGunBolts>,
    lobbies: Query<&Lobby>,
    mut zombies: Query<(&PlayerId, &PlayerPose, &PlayerCombat, &LobbyPlayer, Option<&mut BotBrain>)>,
    mut player_hits: EventWriter<PlayerHit>,
) {
    let server = server.into_inner();
    let tick = timeline.tick().0;
    let now = time.elapsed_secs();
    let step = RAYGUN_BOLT_SPEED * time.delta_secs();
    let mut i = 0;
    while i < bolts.0.len() {
        let bolt = &mut bolts.0[i];
        let Some(lobby) = lobbies.get(bolt.lobby).ok().filter(|l| l.started) else {
            bolts.0.swap_remove(i);
            continue;
        };
        if lobby.paused {
            i += 1;
            continue;
        }
        let reach = step.min(bolt.left);

        // The nearest living zombie it touches this tick: (distance along,
        // who, headshot).
        let mut nearest: Option<(f32, PeerId, bool)> = None;
        for (id, pose, combat, lp, _) in &zombies {
            if lp.lobby != bolt.lobby || !combat.alive || !is_bot_peer(id.0) {
                continue;
            }
            let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
            let body = ray_capsule(bolt.pos, bolt.dir, &Capsule::standing(feet, PLAYER_HEIGHT, PLAYER_RADIUS));
            let head = ray_capsule(bolt.pos, bolt.dir, &Capsule::head(feet, PLAYER_HEIGHT, HEAD_RADIUS));
            let (t, headshot) = match (body, head) {
                (Some(b), Some(h)) => (b.min(h), h <= b),
                (Some(b), None) => (b, false),
                (None, Some(h)) => (h, true),
                (None, None) => continue,
            };
            if t <= reach && nearest.is_none_or(|(n, ..)| t < n) {
                nearest = Some((t, id.0, headshot));
            }
        }

        let (at, outcome, direct) = match nearest {
            Some((t, victim, headshot)) => {
                let point = bolt.pos + bolt.dir * t;
                let damage = WeaponId::RayGun.spec().base_damage;
                (
                    point,
                    ShotOutcome::Hit {
                        target: victim.to_bits(),
                        headshot,
                        point: point.to_array(),
                        damage,
                    },
                    Some((victim, headshot, damage)),
                )
            }
            None if reach < bolt.left => {
                bolt.pos += bolt.dir * reach;
                bolt.left -= reach;
                i += 1;
                continue;
            }
            None => match bolt.surface {
                Some((point, normal)) => (
                    point,
                    ShotOutcome::Ground {
                        point: point.to_array(),
                        normal: normal.to_array(),
                    },
                    None,
                ),
                None => (bolt.pos + bolt.dir * reach, ShotOutcome::Miss, None),
            },
        };
        let bolt = bolts.0.swap_remove(i);

        // It burst on something (not fizzled out in the sky): the zombie it
        // hit takes the bolt, the rest close by the burst — less the
        // further out, more packed — and any of them it doesn't kill are
        // stunned (it makes no odds to one it does).
        if !matches!(outcome, ShotOutcome::Miss) {
            let mult = pap_mult(lobby, bolt.shooter, shared::pap::PapWeapon::RayGun);
            for (id, pose, combat, lp, brain) in &mut zombies {
                if lp.lobby != bolt.lobby || !combat.alive || !is_bot_peer(id.0) {
                    continue;
                }
                let hit = match direct {
                    Some((victim, headshot, damage)) if victim == id.0 => Some((damage, headshot, Some(at))),
                    _ => {
                        let middle = pose.translation - Vec3::Y * (EYE_HEIGHT - PLAYER_HEIGHT * 0.5);
                        let damage = raygun_splash_damage(middle.distance(at));
                        (damage > 0.0).then_some((damage, false, None))
                    }
                };
                let Some((damage, critical, point)) = hit else {
                    continue;
                };
                if let Some(mut brain) = brain {
                    brain.stun(now + RAYGUN_STUN_SECS);
                }
                player_hits.write(PlayerHit {
                    victim: id.0,
                    killer: bolt.shooter,
                    damage: damage * mult,
                    bomb_shot: false,
                    blast: point.is_none(),
                    critical,
                    point,
                });
            }
        }

        let msg = ShotResolved {
            shooter: bolt.shooter,
            tick,
            outcome,
            origin: bolt.origin.to_array(),
            tracer_end: at.to_array(),
            weapon: WeaponId::RayGun.as_u8(),
        };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::All) {
            error!("failed to broadcast ray gun burst: {e:?}");
        }
    }
}
