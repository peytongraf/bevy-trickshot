//! The squad's souls go up into the pyramid ([`EggStep::Ascension`] — each
//! client plays its own trip, `client::easter_egg::ascension`): as it
//! starts, every player still in the game (not bled out, not a bot) has
//! their body left where they stand, recorded on the egg
//! (`EasterEgg::bodies`, in the order their souls take the pyramid's spots)
//! so every client can show the bodies slumped there. Anyone down gets back
//! up — there's nothing left to bleed out to, and their soul goes with the
//! rest. Nothing can hurt the bodies: the round's held and the zombies hold
//! their fire throughout (`shared::easter_egg::holds_spawns`,
//! `Lobby::bots_hold_fire`).
//!
//! On the chamber's altar lies an Exhumer for each of them (the wonder
//! weapon, `crate::exhumer`) — a [`WeaponDrop`] only its soul can take
//! ([`WeaponDrop::owner`]; what it's swapped for drops by their body below,
//! `wall_buys`). Once every soul up there carries theirs, they all go back
//! down together ([`EggStep::Complete`]: a white flash on every client, back
//! in their bodies) and the round goes on.
//!
//! A moment after they're back ([`AFTER_PYRAMID_RADIO_SECS`]), the man on
//! the radio has his closing word (`RadioLine::EggAfterPyramid`) — and
//! that's the egg done: it stays [`EggStep::Complete`] for the rest of the
//! game (nothing in it can start again — every step before keys off its
//! own), and a new game starts it from the beginning
//! (`lobby::on_start`'s `EasterEgg::default`).
//!
//! The bodies, and any Exhumer still on the altar, are forgotten as soon as
//! the egg's anywhere else (its souls back down, a debug jump, a reset).
//!
//! [`AFTER_PYRAMID_RADIO_SECS`]: shared::easter_egg::AFTER_PYRAMID_RADIO_SECS

use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use lightyear::prelude::server::Server;
use lightyear::prelude::*;
use shared::bot_players::is_bot_peer;
use shared::easter_egg::{EggStep, SoulBody};
use shared::revive::PlayerRevived;
use shared::weapon::{SlotWeapon, WeaponId};
use shared::{GameChannel, Lobby, PlayerId, PlayerPose, WeaponDrop};

use crate::lobby::LobbyPlayer;
use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;
use crate::zombies::Zombie;

/// The lobbies whose souls have left their bodies, this trip.
#[derive(Resource, Default)]
pub(super) struct Ascents(HashSet<Entity>);

/// The lobbies whose souls came back down with their Exhumers: how long
/// (s, not counting pauses) since — until the radio's had its word.
#[derive(Resource, Default)]
pub(super) struct Returned(bevy::platform::collections::HashMap<Entity, f32>);

/// On an Exhumer lying on the pyramid's altar, in `0`'s game.
#[derive(Component)]
pub(super) struct AltarExhumer(Entity);

/// The Exhumer as it's taken: no souls in it yet (they're its only ammo,
/// kept by `crate::exhumer`).
fn altar_exhumer(peer: PeerId, pos: Vec3, yaw: f32) -> WeaponDrop {
    WeaponDrop {
        weapon: SlotWeapon::Gun(WeaponId::Exhumer),
        pap: 0,
        mag: 0,
        reserve: 0,
        pos,
        yaw,
        owner: Some(peer),
    }
}

/// Leave each player's body as the trip starts (getting anyone down back
/// up); forget them once it's over.
#[allow(clippy::type_complexity)]
pub(super) fn run_ascension(
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut ascents: ResMut<Ascents>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &mut PlayerCombat), Without<Zombie>>,
    on_altar: Query<(Entity, &AltarExhumer)>,
    (time, mut returned, mut radio): (Res<Time>, ResMut<Returned>, EventWriter<crate::radio::SayRadio>),
    mut commands: Commands,
) {
    let server = server.into_inner();
    ascents.0.retain(|e| lobbies.contains(*e));
    // Back in their bodies: a moment, then the man on the radio's last word.
    returned.0.retain(|e, since| {
        let Ok((_, lobby)) = lobbies.get(*e) else { return false };
        if !shared::easter_egg::available(lobby) || lobby.easter_egg.step != EggStep::Complete {
            return false;
        }
        if !lobby.paused {
            *since += time.delta_secs();
        }
        if *since < shared::easter_egg::AFTER_PYRAMID_RADIO_SECS {
            return true;
        }
        radio.write(crate::radio::SayRadio { lobby: *e, line: shared::radio::RadioLine::EggAfterPyramid });
        info!("lobby {e:?}: the easter egg's done");
        false
    });
    for (e, altar) in &on_altar {
        if !lobbies.contains(altar.0) {
            commands.entity(e).try_despawn();
        }
    }
    for (lobby_e, mut lobby) in &mut lobbies {
        let up = shared::easter_egg::available(&lobby) && lobby.easter_egg.step == EggStep::Ascension;
        if !up {
            ascents.0.remove(&lobby_e);
            if !lobby.easter_egg.bodies.is_empty() {
                lobby.easter_egg.bodies.clear();
            }
            for (e, altar) in &on_altar {
                if altar.0 == lobby_e {
                    commands.entity(e).try_despawn();
                }
            }
            continue;
        }
        if !ascents.0.insert(lobby_e) {
            // Every soul up there has theirs: back down, together.
            let bodies = &lobby.easter_egg.bodies;
            let done = !bodies.is_empty()
                && bodies.iter().all(|b| {
                    lobby
                        .members
                        .iter()
                        .find(|m| m.peer == b.peer)
                        .is_none_or(|m| m.weapons.contains(&SlotWeapon::Gun(WeaponId::Exhumer)))
                });
            if done {
                lobby.easter_egg.step = EggStep::Complete;
                returned.0.insert(lobby_e, 0.0);
                info!("lobby {lobby_e:?}: every soul has its Exhumer — back down");
            }
            continue;
        }
        let mut bodies = Vec::new();
        for (id, pose, lp, mut combat) in &mut players {
            if lp.lobby != lobby_e || is_bot_peer(id.0) || combat.bled_out.is_some() {
                continue;
            }
            if combat.down.is_some() {
                combat.revive();
                let msg = PlayerRevived { peer: id.0, reviver: id.0 };
                if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(vec![id.0])) {
                    error!("failed to get {:?} up for the pyramid: {e:?}", id.0);
                }
            }
            if !combat.alive {
                continue;
            }
            bodies.push(SoulBody { peer: id.0, feet: pose.translation - Vec3::Y * EYE_HEIGHT, yaw: pose.yaw });
        }
        // (The same order on every client: who joined first, first.)
        bodies.sort_by_key(|b| lobby.members.iter().position(|m| m.peer == b.peer).unwrap_or(usize::MAX));
        info!("lobby {lobby_e:?}: {} souls go up into the pyramid", bodies.len());
        // An Exhumer on the altar for each of them (by their spot, should
        // the map have no altar placed).
        let altar = shared::level::layout(lobby.map).pyramid_altar;
        let n = bodies.len();
        for (i, b) in bodies.iter().enumerate() {
            let (pos, yaw) = match altar {
                Some(altar) => shared::exhumer::on_altar(altar, i, n),
                None => match shared::easter_egg::pyramid_spot(lobby.map, i) {
                    Some((spot, yaw)) => (spot + Quat::from_rotation_y(yaw) * Vec3::NEG_Z, yaw),
                    None => continue,
                },
            };
            crate::wall_buys::spawn_drop(&mut commands, lobby_e, &lobby, altar_exhumer(b.peer, pos, yaw))
                .insert(AltarExhumer(lobby_e));
        }
        lobby.easter_egg.bodies = bodies;
    }
}
