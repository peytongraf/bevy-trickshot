//! The local player: rig transform + physics, crouch/slide/dive, ledge
//! mantling, footsteps, mouse look, the debug teleport key, and trickshot
//! spin tracking.

mod camera;
mod footsteps;
mod mantle;
mod movement;
mod slide;
mod teleport;
mod trick;

pub(crate) use camera::*;
pub(crate) use footsteps::*;
pub(crate) use mantle::*;
pub(crate) use movement::*;
pub(crate) use slide::*;
pub(crate) use teleport::*;
pub(crate) use trick::*;
