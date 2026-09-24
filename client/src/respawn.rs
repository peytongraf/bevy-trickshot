//! Respawning is a fresh start. When the local player's respawn is applied
//! (`net::LocalPlayerRespawned`, after the kill cam), everything transient that
//! belongs to the life that just ended is reset here, so none of it carries
//! over to the new one: camera shake and recoil, aiming, sprint / jump / slide /
//! mantle state, the in-progress trick (spin) tracking, and a queued-but-unfired
//! shot or stab. Health, the red tint / blood overlay and the heartbeat are
//! reset in `health` (which follows the server's health), the death and fall
//! effects clear themselves in `death_effect` / `fall_death`, ammo is refilled
//! where the teleport happens (`net::flush_pending_respawn`), and the server
//! restores its side — health, alive, damage timers — on `shared::RespawnReady`.

use bevy::prelude::*;

use crate::net::LocalPlayerRespawned;
use crate::{Ads, Jumping, Mantle, PendingMelee, PendingShot, Shake, Slide, Sprinting, TrickState};
use crate::AppState;

pub(crate) struct RespawnResetPlugin;

impl Plugin for RespawnResetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, reset_on_respawn.run_if(in_state(AppState::InGame)));
    }
}

/// Everything [`reset_on_respawn`] resets, bundled to stay under the system
/// parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
struct LifeState<'w> {
    shake: ResMut<'w, Shake>,
    ads: ResMut<'w, Ads>,
    sprinting: ResMut<'w, Sprinting>,
    jumping: ResMut<'w, Jumping>,
    slide: ResMut<'w, Slide>,
    mantle: ResMut<'w, Mantle>,
    trick: ResMut<'w, TrickState>,
    shot: ResMut<'w, PendingShot>,
    melee: ResMut<'w, PendingMelee>,
    drink: ResMut<'w, crate::PerkDrink>,
}

fn reset_on_respawn(mut respawned: EventReader<LocalPlayerRespawned>, mut life: LifeState) {
    if respawned.read().count() == 0 {
        return;
    }
    *life.shake = Shake::default();
    life.ads.t = 0.0;
    *life.sprinting = Sprinting::default();
    *life.jumping = Jumping::default();
    *life.slide = Slide::default();
    *life.mantle = Mantle::default();
    *life.trick = TrickState::default();
    *life.shot = PendingShot::default();
    *life.melee = PendingMelee::default();
    *life.drink = crate::PerkDrink::default();
}
