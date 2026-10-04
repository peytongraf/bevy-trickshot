//! Der Wunderfizz — the classic perk set's (`perks::PerkSet::Classic`) one
//! machine that sells every classic perk, including the two with no machine
//! of their own (Death Perception and PhD Flopper, [`crate::perks::Perk::has_machine`]).
//! It wakes up on round [`ACTIVE_ROUND`]; the server owns the purchases
//! (`server::zombies::on_buy_perk`, [`crate::BuyPerk::wunderfizz`]).

use bevy::math::{Quat, Vec3};

use crate::perks::{in_range_of, PerkSet, MACHINE_HALF_EXTENTS};
use crate::{Lobby, MapId};

/// The round Der Wunderfizz switches on.
pub const ACTIVE_ROUND: u32 = 17;

/// Whether `lobby` has a Wunderfizz at all (it plays the classic perks).
pub fn present(lobby: &Lobby) -> bool {
    lobby.perk_set == PerkSet::Classic
}

/// Whether `lobby`'s Wunderfizz is selling (round [`ACTIVE_ROUND`] on).
pub fn active(lobby: &Lobby) -> bool {
    present(lobby) && lobby.round >= ACTIVE_ROUND
}

/// Where it stands on `map` (the ground under its middle), from the map's
/// layout ([`crate::level`]).
pub fn machine_pos(map: MapId) -> Vec3 {
    crate::level::layout(map).wunderfizz.pos
}

/// Which way it faces on `map` (degrees about y).
pub fn machine_yaw_deg(map: MapId) -> f32 {
    crate::level::layout(map).wunderfizz.yaw_deg
}

/// Its solid box on `map` — the same as a perk machine's.
pub fn machine_box(map: MapId) -> (Vec3, Quat, Vec3) {
    (
        machine_pos(map) + Vec3::Y * MACHINE_HALF_EXTENTS.y,
        Quat::from_rotation_y(machine_yaw_deg(map).to_radians()),
        MACHINE_HALF_EXTENTS,
    )
}

/// Whether feet position `feet` is close enough to use it on `map` (`slack`
/// as in [`crate::perks::in_range`]).
pub fn in_range(map: MapId, feet: Vec3, slack: f32) -> bool {
    in_range_of(machine_pos(map), feet, slack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wunderfizz_is_clear_of_every_classic_machine() {
        // (The level editor warns about the same thing.)
        for map in MapId::PLACES {
            for &perk in PerkSet::Classic.perks().iter().filter(|p| p.has_machine()) {
                assert!(!in_range(map, perk.machine_pos(map), 0.75), "{perk:?} on {map:?}");
            }
        }
    }
}
