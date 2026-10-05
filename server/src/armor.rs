//! `Zombies` armor (`shared::armor`): buying and refilling it at the armor
//! station, and [`soak`] — every hit on a player (`pvp`) comes off their
//! armor first.
//!
//! A member's armor lives on their [`shared::LobbyMember::armor`], so it's
//! replicated with the lobby; it's cleared with their perks whenever a game
//! starts (`lobby`) and when they bleed out (`revive`, `pvp`).

use bevy::prelude::*;
use lightyear::prelude::*;
use shared::armor::{cost_to, Armor, MAX_LEVEL, REFILL_COST};
use shared::{BuyArmor, GameMode, Lobby, PlayerId, PlayerPose, RefillArmor};

use crate::killcam::EndingLobbies;
use crate::pvp::PlayerCombat;
use crate::wall_buys::standing_in_game;

pub struct ArmorPlugin;

impl Plugin for ArmorPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_buy_armor).add_observer(on_refill_armor);
    }
}

/// `damage` about to be taken by `victim`: what their armor soaks up (in a
/// `Zombies` game) comes off it, and what gets through to their health is
/// returned.
pub(crate) fn soak(lobbies: &mut Query<(Entity, &mut Lobby)>, victim: PeerId, damage: f32) -> f32 {
    let Some((_, mut lobby)) = lobbies.iter_mut().find(|(_, l)| {
        l.started
            && l.mode == GameMode::Zombies
            && l.members.iter().any(|m| m.peer == victim && m.armor.points > 0.0)
    }) else {
        return damage;
    };
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == victim) else {
        return damage;
    };
    let through = member.armor.absorb(damage);
    if member.armor.points <= 0.0 {
        info!("{victim:?}'s armor broke");
    }
    through
}

/// A member at the station (in a running `Zombies` game, up) with the
/// points — their lobby entity, if so.
fn at_station(
    peer: PeerId,
    endings: &EndingLobbies,
    lobbies: &Query<(Entity, &mut Lobby)>,
    players: &Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) -> Option<Entity> {
    let (lobby_e, feet, _) = standing_in_game(peer, endings, lobbies, players)?;
    let (_, lobby) = lobbies.get(lobby_e).ok()?;
    shared::armor::in_range(lobby.map, feet, 0.75).then_some(lobby_e)
}

/// Buy armor up to `level`, paying for every level on the way, and fill it.
fn on_buy_armor(
    trigger: Trigger<RemoteTrigger<BuyArmor>>,
    endings: Res<EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let level = trigger.trigger.level;
    let Some(lobby_e) = at_station(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) else {
        return;
    };
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    let current = member.armor.level;
    let cost = cost_to(current, level);
    if level > MAX_LEVEL || level <= current || member.score < cost {
        return;
    }
    member.score -= cost;
    member.armor = Armor::full(level);
    info!("{peer:?} bought armor level {level}");
}

/// Fill every plate a member owns back up, for the flat refill cost.
fn on_refill_armor(
    trigger: Trigger<RemoteTrigger<RefillArmor>>,
    endings: Res<EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let Some(lobby_e) = at_station(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) else {
        return;
    };
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    if member.armor.level == 0 || member.armor.is_full() || member.score < REFILL_COST {
        return;
    }
    member.score -= REFILL_COST;
    member.armor = Armor::full(member.armor.level);
    info!("{peer:?} refilled their armor");
}
