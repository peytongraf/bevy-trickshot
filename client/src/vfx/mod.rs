//! Shot/weapon visual effects: muzzle flash, barrel smoke, fire tracers, and
//! bullet-impact particles (ground rock/dust, bot blood).

mod impacts;
mod muzzle_flash;
mod smoke;
mod tracers;

pub(crate) use impacts::*;
pub(crate) use muzzle_flash::*;
pub(crate) use smoke::*;
pub(crate) use tracers::*;
