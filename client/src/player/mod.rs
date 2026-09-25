//! The local player: rig transform + physics, crouch/slide/dive, ledge
//! mantling, footsteps, mouse look (plus the shroom aim assist), the debug
//! teleport key, trickshot spin tracking, and the gun flashlights (theirs
//! too) on night maps.

mod aim_assist;
mod camera;
mod flashlight;
mod footsteps;
mod mantle;
mod movement;
mod slide;
mod teleport;
mod trick;

pub(crate) use aim_assist::*;
pub(crate) use camera::*;
pub(crate) use flashlight::*;
pub(crate) use footsteps::*;
pub(crate) use mantle::*;
pub(crate) use movement::*;
pub(crate) use slide::*;
pub(crate) use teleport::*;
pub(crate) use trick::*;
