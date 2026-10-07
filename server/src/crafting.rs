//! The `Zombies` crafting table (`shared::crafting`): a member crafting one
//! piece of equipment ([`BuyEquipment`]) pays its price here, and what they
//! carried of another kind of the same sort is dropped around them. How many
//! they carry is the client's to keep (it only asks when it isn't full), as
//! with every lethal; the answer ([`EquipmentBought`]) tells it to add one.

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{BuyEquipment, EquipmentBought, GameChannel, GameMode, Lobby, PlayerId, PlayerPose};

use crate::collision::MapColliders;
use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;

pub struct CraftingPlugin;

impl Plugin for CraftingPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_buy_equipment);
    }
}

/// A member wants to craft one piece of equipment: they must be in a
/// running, unpaused `Zombies` game, alive, at the table (a little slack),
/// asking for something it sells, with the points. Anything else is
/// ignored.
#[allow(clippy::too_many_arguments)]
fn on_buy_equipment(
    trigger: Trigger<RemoteTrigger<BuyEquipment>>,
    endings: Res<crate::killcam::EndingLobbies>,
    colliders: Res<MapColliders>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let BuyEquipment { kind, dropping } = trigger.trigger;
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
    if !combat.alive || !shared::crafting::in_range(lobby.map, feet, 0.75) {
        return;
    }
    let Some(cost) = shared::crafting::cost(kind) else {
        return;
    };
    let Some(member) = lobby.members.iter_mut().find(|m| m.peer == peer) else {
        return;
    };
    if member.score < cost {
        return;
    }
    member.score -= cost;
    // (Only what's carried of the same sort — the client says which.)
    let dropping = dropping.filter(|(old, _)| old.is_tactical() == kind.is_tactical());
    crate::lethals::drop_carried(
        &mut commands,
        lobby_e,
        &lobby,
        peer,
        feet,
        dropping,
        kind,
        &colliders.for_lobby(&lobby),
    );
    if let Err(e) =
        sender.send::<_, GameChannel>(&EquipmentBought { kind }, server.into_inner(), &NetworkTarget::Single(peer))
    {
        error!("failed to send crafted equipment: {e:?}");
    }
    info!("{peer:?} crafted a {kind:?} for {cost}");
}
