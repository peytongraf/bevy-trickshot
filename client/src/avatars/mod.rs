//! Non-local-player avatar animation: remote players' (and bots') "soldier"
//! avatars — and what they hold — and `Zombies` zombies.

mod bounds;
pub(crate) mod held_weapon;
mod soldier;
mod zombie;

pub(crate) use bounds::CharacterBounds;
pub(crate) use held_weapon::{HeldWeapon, SoldierRig};
pub(crate) use soldier::*;
pub(crate) use zombie::*;
