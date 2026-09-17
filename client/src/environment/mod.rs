//! World geometry, weather and fixture lighting for the current map: the
//! map/shipment model, its water plane, its container lighting fixtures, its
//! rain, and its sky. Mostly systems `main.rs`'s `setup_world` spawns once
//! and that then just react to `CurrentMap` changes or live debug-panel
//! tweaks.

mod lighting;
mod map;
mod rain;
mod sky;
mod water;

pub(crate) use lighting::*;
pub(crate) use map::*;
pub(crate) use rain::*;
pub(crate) use sky::*;
pub(crate) use water::*;
