//! `Zombies` wall buys and dropped weapons (`shared::wall_buy`).
//!
//! A player carries two weapons (`LobbyMember::weapons`). Buying a gun at
//! its sign ([`BuyWallWeapon`]) or picking a dropped weapon up
//! ([`PickUpWeapon`]) puts it in the slot in their hands — the weapon that
//! was there is dropped where they stand ([`WeaponDrop`]), with its
//! Pack-a-Punch level and the rounds the client says it had (ammo is
//! client-side), for anyone to pick up. The levels go with the weapons:
//! the dropped kind's is the drop's now, and a picked-up weapon's is the
//! picker's.
//!
//! Drops stay until the game ends ([`cull_drops`]); the player entities and
//! lobby members are reset with every game, so there's nothing else to clear.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use shared::pap::PapWeapon;
use shared::throwing_knife::in_server_pickup_range;
use shared::weapon::SlotWeapon;
use shared::{
    BuyWallWeapon, GameChannel, GiveWeapon, GameMode, Lobby, PickUpWeapon, PlayerId, PlayerPose, WallWeaponBought,
    WeaponDrop, WeaponPickedUp,
};

use crate::collision::MapColliders;
use crate::killcam::EndingLobbies;
use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;

/// The server side of one [`WeaponDrop`]: whose game it's in.
#[derive(Component)]
struct DropSim {
    lobby: Entity,
}

pub struct WallBuysPlugin;

impl Plugin for WallBuysPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_buy_wall_weapon)
            .add_observer(on_give_weapon)
            .add_observer(on_pick_up_weapon)
            .add_systems(FixedUpdate, cull_drops);
    }
}

/// `peer`'s running, unpaused `Zombies` game (not ending), if they're up and
/// about in it: the lobby, and their feet and eye.
pub(crate) fn standing_in_game(
    peer: PeerId,
    endings: &EndingLobbies,
    lobbies: &Query<(Entity, &mut Lobby)>,
    players: &Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) -> Option<(Entity, Vec3, Vec3)> {
    let (lobby_e, lobby) = lobbies
        .iter()
        .find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer))?;
    if endings.is_ending(lobby_e) || lobby.paused {
        return None;
    }
    let (_, pose, combat) = players.iter().find(|(id, ..)| id.0 == peer)?;
    if !combat.alive || combat.down.is_some() {
        return None;
    }
    let eye = pose.translation;
    Some((lobby_e, eye - Vec3::Y * EYE_HEIGHT, eye))
}

/// Put `taken` in `peer`'s slot `slot` (packed to `pap`), dropping what was
/// there (with `mag` / `reserve` rounds) at `feet`. `None` if `peer` isn't a
/// member, `slot` is out of range or they carry `taken` already.
#[allow(clippy::too_many_arguments)]
pub(crate) fn swap_into_slot(
    commands: &mut Commands,
    colliders: &MapColliders,
    lobby_e: Entity,
    lobby: &mut Lobby,
    peer: PeerId,
    slot: u8,
    taken: SlotWeapon,
    pap: u8,
    (mag, reserve): (u32, u32),
    feet: Vec3,
) -> Option<()> {
    let slot = slot as usize;
    let member = lobby.members.iter().find(|m| m.peer == peer)?;
    if slot >= member.weapons.len() || member.weapons.contains(&taken) {
        return None;
    }
    let dropped = member.weapons[slot];
    let dropped_pap = member.pap.get(PapWeapon::of(dropped));
    let seed = (feet.x * 7.31 + feet.z * 3.17).fract().abs();
    let pos = shared::molotov::drop_spot(feet, seed, &colliders.for_lobby(lobby));
    let real = lobby.real_peers();
    commands.spawn((
        Name::from("WeaponDrop"),
        WeaponDrop {
            weapon: dropped,
            pap: dropped_pap,
            // (Only a gun's rounds mean anything.)
            mag: if dropped.gun().is_some() { mag } else { 0 },
            reserve: if dropped.gun().is_some() { reserve } else { 0 },
            pos,
            yaw: seed * std::f32::consts::TAU,
        },
        DropSim { lobby: lobby_e },
        Replicate::to_clients(NetworkTarget::Only(real)),
    ));
    let member = lobby.members.iter_mut().find(|m| m.peer == peer)?;
    member.pap.set(PapWeapon::of(dropped), 0);
    member.pap.set(PapWeapon::of(taken), pap);
    member.weapons[slot] = taken;
    if let Some(gun) = taken.gun() {
        member.primary = gun;
    }
    info!("{peer:?} dropped their {} for a {}", dropped.label(), taken.label());
    Some(())
}

/// A member wants a gun from its wall buy, in place of the weapon in their
/// hands: in a running `Zombies` game, up, at the sign (a little slack), not
/// carrying it already, with the points. Anything else is ignored.
#[allow(clippy::too_many_arguments)]
fn on_buy_wall_weapon(
    trigger: Trigger<RemoteTrigger<BuyWallWeapon>>,
    endings: Res<EndingLobbies>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
    mut commands: Commands,
    mut quotes: EventWriter<crate::quotes::SayQuote>,
) {
    let peer = trigger.from;
    let BuyWallWeapon { weapon, slot, mag, reserve } = trigger.trigger;
    if !shared::weapon::WALL_BUY_WEAPONS.contains(&weapon) {
        return;
    }
    let Some((lobby_e, feet, _)) = standing_in_game(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) else {
        return;
    };
    if !shared::wall_buy::in_range(lobby.map, weapon, feet, 0.75) {
        return;
    }
    let cost = shared::weapon::wall_buy_cost(weapon);
    if !lobby.members.iter().any(|m| m.peer == peer && m.score >= cost) {
        return;
    }
    let taken = SlotWeapon::Gun(weapon);
    if swap_into_slot(&mut commands, &colliders, lobby_e, &mut lobby, peer, slot, taken, 0, (mag, reserve), feet)
        .is_none()
    {
        return;
    }
    if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
        m.score -= cost;
    }
    let msg = WallWeaponBought { weapon, slot };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server.into_inner(), &NetworkTarget::Single(peer)) {
        error!("failed to send a wall buy to {peer:?}: {e:?}");
    }
    if weapon == shared::weapon::WeaponId::Ak74 {
        quotes.write(crate::quotes::SayQuote::by(lobby_e, peer, shared::quotes::Quote::BuyAk74));
    }
    info!("{peer:?} bought the {} off the wall", weapon.label());
}

/// Debug: put a gun in a member's hands for free, like a wall buy with no
/// sign or cost — for trying out guns there's no other way to get yet (the
/// Ray Gun).
#[allow(clippy::too_many_arguments)]
fn on_give_weapon(
    trigger: Trigger<RemoteTrigger<GiveWeapon>>,
    endings: Res<EndingLobbies>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let GiveWeapon { weapon, slot, mag, reserve } = trigger.trigger;
    let Some((lobby_e, feet, _)) = standing_in_game(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) else {
        return;
    };
    let taken = SlotWeapon::Gun(weapon);
    if swap_into_slot(&mut commands, &colliders, lobby_e, &mut lobby, peer, slot, taken, 0, (mag, reserve), feet)
        .is_none()
    {
        return;
    }
    let msg = WallWeaponBought { weapon, slot };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server.into_inner(), &NetworkTarget::Single(peer)) {
        error!("failed to send a debug weapon to {peer:?}: {e:?}");
    }
    info!("{peer:?} was given the {} (debug)", weapon.label());
}

/// A member wants the dropped weapon nearest them, in place of the weapon in
/// their hands — one they don't carry already.
#[allow(clippy::too_many_arguments)]
fn on_pick_up_weapon(
    trigger: Trigger<RemoteTrigger<PickUpWeapon>>,
    endings: Res<EndingLobbies>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
    drops: Query<(Entity, &DropSim, &WeaponDrop)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let PickUpWeapon { slot, mag, reserve } = trigger.trigger;
    let Some((lobby_e, feet, eye)) = standing_in_game(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) else {
        return;
    };
    let carried = lobby.members.iter().find(|m| m.peer == peer).map_or([SlotWeapon::Knife; 2], |m| m.weapons);
    let nearest = drops
        .iter()
        .filter(|(_, sim, d)| {
            sim.lobby == lobby_e && in_server_pickup_range(feet, eye, d.pos) && !carried.contains(&d.weapon)
        })
        .min_by(|a, b| a.2.pos.distance_squared(eye).total_cmp(&b.2.pos.distance_squared(eye)));
    let Some((entity, _, drop)) = nearest else {
        return;
    };
    let drop = *drop;
    if swap_into_slot(
        &mut commands,
        &colliders,
        lobby_e,
        &mut lobby,
        peer,
        slot,
        drop.weapon,
        drop.pap,
        (mag, reserve),
        feet,
    )
    .is_none()
    {
        return;
    }
    commands.entity(entity).try_despawn();
    let msg = WeaponPickedUp {
        weapon: drop.weapon,
        slot,
        pap: drop.pap,
        mag: drop.mag,
        reserve: drop.reserve,
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server.into_inner(), &NetworkTarget::Single(peer)) {
        error!("failed to send a weapon pickup to {peer:?}: {e:?}");
    }
}

/// Remove the drops whose lobby's game has ended or gone.
fn cull_drops(lobbies: Query<&Lobby>, drops: Query<(Entity, &DropSim)>, mut commands: Commands) {
    for (e, sim) in &drops {
        if lobbies.get(sim.lobby).map(|l| !l.started).unwrap_or(true) {
            commands.entity(e).try_despawn();
        }
    }
}
