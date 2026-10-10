//! World geometry and weather for the current map: the map/shipment model,
//! its water plane, its rain, its sky, and the dog-round darkness (its fixture lights are its
//! layout's power lights — `power.rs`). Mostly systems `main.rs`'s `setup_world` spawns once
//! and that then just react to `CurrentMap` changes or live debug-panel
//! tweaks.

mod atmosphere;
mod dog_round;
mod map;
mod rain;
mod sky;
mod water;

pub(crate) use atmosphere::*;
pub(crate) use dog_round::*;
pub(crate) use map::*;
pub(crate) use rain::*;
pub(crate) use sky::*;
pub(crate) use water::*;
