//! `Zombies` wall buys: a wooden sign on the map with a gun's outline on
//! it — stand at it and pay ([`crate::weapon::wall_buy_cost`]) to swap the
//! weapon in your hands for that gun, dropping the old one
//! ([`crate::WeaponDrop`]). Where each stands is the map's layout's
//! ([`crate::level::WallBuy`], placed in the level editor); the server owns
//! the purchase (`server::wall_buys`).

use bevy::math::Vec3;

use crate::level::Placement;
use crate::perks::PERK_USE_HEIGHT;
use crate::weapon::WeaponId;
use crate::MapId;

/// How far (m, across the ground) from the spot just in front of the sign a
/// player can buy from it...
pub const USE_RADIUS: f32 = 1.8;
/// ...that spot being this far (m) out from the sign's post.
const USE_SPOT_AHEAD: f32 = 0.8;

/// Where a player stands to buy from a sign at `at`.
pub fn use_spot(at: Placement) -> Vec3 {
    at.pos + at.rotation() * Vec3::Z * USE_SPOT_AHEAD
}

/// Whether feet at `feet` can buy from a sign standing at `at` (`slack`
/// widens it — the server allows for the pose being a moment old).
pub fn in_range_of(at: Placement, feet: Vec3, slack: f32) -> bool {
    let spot = use_spot(at);
    Vec3::new(feet.x - spot.x, 0.0, feet.z - spot.z).length() <= USE_RADIUS + slack
        && (feet.y - at.pos.y).abs() <= PERK_USE_HEIGHT + slack
}

/// The wall buy for `gun` on `map` that feet at `feet` can buy from, if any.
pub fn in_range(map: MapId, gun: WeaponId, feet: Vec3, slack: f32) -> bool {
    crate::level::layout(map).wall_buy(gun).is_some_and(|at| in_range_of(at, feet, slack))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_bought_from_in_front_of_the_sign() {
        let at = Placement::new(Vec3::new(4.0, 2.0, -3.0), 90.0);
        // (Turned 90°: its front faces +X.)
        assert!(in_range_of(at, at.pos + Vec3::X * 1.2, 0.0));
        assert!(!in_range_of(at, at.pos - Vec3::X * 2.5, 0.0));
        assert!(!in_range_of(at, at.pos + Vec3::X * 1.2 + Vec3::Y * 3.0, 0.0));
    }
}
