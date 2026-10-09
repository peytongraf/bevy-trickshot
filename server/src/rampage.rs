//! `Zombies`' Rampage Inducer, the server's side (`shared::rampage`): a
//! member who's held interact at it asks to turn it on or off
//! ([`shared::ToggleRampage`]); here that's checked and `Lobby::rampage`
//! flipped. What it does while it's on is in `crate::ai` (zombies faster —
//! those already up too) and `crate::zombies` (spawning quicker). It's
//! cleared whenever a game starts or ends (`crate::lobby`).

use bevy::prelude::*;
use lightyear::prelude::*;
use shared::{GameMode, Lobby, PlayerId, PlayerPose, ToggleRampage};

use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;

pub struct RampagePlugin;

impl Plugin for RampagePlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_toggle_rampage);
    }
}

/// A member wants the inducer on (or off): they must be in a running,
/// unpaused `Zombies` game on a map with one, alive and at it (a little
/// slack — their pose is a moment old), and it mustn't already be that way.
/// Anything else is ignored.
fn on_toggle_rampage(
    trigger: Trigger<RemoteTrigger<ToggleRampage>>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let on = trigger.trigger.on;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused || !shared::rampage::available(&lobby) || lobby.rampage == on {
        return;
    }
    let Some(at) = shared::rampage::layout(lobby.map) else {
        return;
    };
    let Some((_, pose, combat)) = players.iter().find(|(id, ..)| id.0 == peer) else {
        return;
    };
    let feet = pose.translation - Vec3::Y * EYE_HEIGHT;
    if !combat.alive || !shared::rampage::in_range_of(at, feet, 0.75) {
        return;
    }
    lobby.rampage = on;
    info!("lobby {lobby_e:?}: {peer:?} turned the Rampage Inducer {}", if on { "on" } else { "off" });
}
