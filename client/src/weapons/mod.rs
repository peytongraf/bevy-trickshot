//! Weapons: ammo/fire/reload state, the AK-74 (a loadout primary), ADS, weapon sway, the render-to-texture
//! scope, the knife's, throwing arms' and drinking arms' view models, camera recoil, and the
//! view-model animation rig shared by all of them.

mod ads;
mod aim_recoil;
mod ak;
mod drink_arms;
mod raygun;
mod knife_view_model;
mod recoil;
mod scope;
mod sway;
mod throw_arms;
mod view_model;
mod weapon;

pub(crate) use ads::*;
pub(crate) use aim_recoil::*;
pub(crate) use ak::*;
pub(crate) use drink_arms::*;
pub(crate) use raygun::*;
pub(crate) use knife_view_model::*;
pub(crate) use recoil::*;
pub(crate) use scope::*;
pub(crate) use sway::*;
pub(crate) use throw_arms::*;
pub(crate) use view_model::*;
pub(crate) use weapon::*;
