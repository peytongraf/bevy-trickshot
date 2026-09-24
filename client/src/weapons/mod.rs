//! Weapons: ammo/fire/reload state, ADS, weapon sway, the render-to-texture
//! scope, the knife's, throwing arms' and drinking arms' view models, camera recoil, and the
//! view-model animation rig shared by all of them.

mod ads;
mod drink_arms;
mod knife_view_model;
mod recoil;
mod scope;
mod sway;
mod throw_arms;
mod view_model;
mod weapon;

pub(crate) use ads::*;
pub(crate) use drink_arms::*;
pub(crate) use knife_view_model::*;
pub(crate) use recoil::*;
pub(crate) use scope::*;
pub(crate) use sway::*;
pub(crate) use throw_arms::*;
pub(crate) use view_model::*;
pub(crate) use weapon::*;
