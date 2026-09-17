//! Weapons: ammo/fire/reload state, ADS, weapon sway, the render-to-texture
//! scope, the throwing knife's view model, camera recoil, and the view-model
//! animation rig shared by all of them.

mod ads;
mod knife_view_model;
mod recoil;
mod scope;
mod sway;
mod view_model;
mod weapon;

pub(crate) use ads::*;
pub(crate) use knife_view_model::*;
pub(crate) use recoil::*;
pub(crate) use scope::*;
pub(crate) use sway::*;
pub(crate) use view_model::*;
pub(crate) use weapon::*;
