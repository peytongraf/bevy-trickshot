//! HUD: the dedicated UI camera and visibility gate, the crosshair, the
//! ammo/FPS readouts, and the score-popup stack.

mod ammo_text;
mod crosshair;
mod fps_text;
mod score_popup;
mod setup;

pub(crate) use ammo_text::*;
pub(crate) use crosshair::*;
pub(crate) use fps_text::*;
pub(crate) use score_popup::*;
pub(crate) use setup::*;
