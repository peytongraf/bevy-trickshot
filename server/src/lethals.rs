//! Swapping lethals: a player carries one kind at a time, so picking up (or
//! taking from the Mystery Box) another kind drops what they carried of the
//! old one around them ([`drop_carried`]), for anyone to pick up again.

use bevy::prelude::*;
use lightyear::prelude::*;
use shared::lethal::{Carried, LethalKind};
use shared::Lobby;

/// Drop what `peer` is `carrying` (of a kind other than `taking`) around
/// `feet`, in their `Zombies` lobby `lobby_e`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drop_carried(
    commands: &mut Commands,
    lobby_e: Entity,
    lobby: &Lobby,
    peer: PeerId,
    feet: Vec3,
    carrying: Carried,
    taking: LethalKind,
    world: &dyn shared::map::CollisionWorld,
) {
    let Some((kind, count)) = carrying.filter(|(kind, count)| *kind != taking && *count > 0) else {
        return;
    };
    if lobby.mode != shared::GameMode::Zombies {
        return;
    }
    let count = count.min(kind.max_carried());
    match kind {
        LethalKind::ThrowingKnife => crate::knives::drop_knives_around(commands, lobby_e, lobby, peer, feet, count, world),
        LethalKind::Molotov => crate::molotovs::drop_around(commands, lobby_e, lobby, feet, count, world),
        LethalKind::MonkeyBomb => crate::monkey_bombs::drop_around(commands, lobby_e, lobby, feet, count, world),
        LethalKind::Frag => crate::frags::drop_around(commands, lobby_e, lobby, feet, count, world),
    }
}
