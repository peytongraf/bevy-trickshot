//! HUD: the dedicated UI camera and visibility gate, the crosshair, the
//! ammo/FPS/ping readouts, the score-popup stack, and players' name tags.

mod ammo_text;
mod crosshair;
mod debug_readout;
mod fps_text;
mod name_tags;
mod score_popup;
mod setup;

pub(crate) use ammo_text::*;
pub(crate) use crosshair::*;
pub(crate) use debug_readout::*;
pub(crate) use fps_text::*;
pub(crate) use name_tags::*;
pub(crate) use score_popup::*;
pub(crate) use setup::*;
