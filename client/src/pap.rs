//! The `Zombies` Pack-a-Punch machine: for now just its model
//! (`models/pap_machine.glb`), placed from the debug panel ("Zombies perks" →
//! "Pack-a-Punch machine") on the map that has the power switch (Break Point
//! Night). It does nothing yet, and it's on this client only — no collision,
//! nothing on the server.
//!
//! `StateScoped(InGame)`, and taken away whenever we're not in such a game —
//! nothing carries into the next one.

use bevy::prelude::*;
use lightyear::prelude::LocalId;
use shared::Lobby;

use crate::net::GameClient;
use crate::zombies_hud::zombies_game;
use crate::AppState;

const PAP_MODEL: &str = "models/pap_machine.glb";
/// Half the model's height (m) at scale 1 — its origin is at its middle.
const PAP_HALF_HEIGHT: f32 = 2.0;

pub(crate) struct PapPlugin;

impl Plugin for PapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PapSettings>()
            .add_systems(Update, sync_pap_machine.run_if(in_state(AppState::InGame)));
    }
}

/// Panel-tunable Pack-a-Punch placement.
#[derive(Resource, Clone)]
pub(crate) struct PapSettings {
    /// World position (m) of the ground under its middle — it's lifted by
    /// its own half height (times `scale`) to stand there.
    pub(crate) pos: Vec3,
    /// Degrees about x (pitch), y (turn) and z (roll).
    pub(crate) rotation_deg: Vec3,
    /// 1 = the model as it comes: 2.5 m wide, 4 m tall, 1.2 m deep.
    pub(crate) scale: f32,
}

impl Default for PapSettings {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            rotation_deg: Vec3::ZERO,
            // A 2 m tall machine.
            scale: 0.5,
        }
    }
}

impl PapSettings {
    fn transform(&self) -> Transform {
        let r = self.rotation_deg;
        let scale = self.scale.max(1e-4);
        Transform::from_translation(self.pos + Vec3::Y * PAP_HALF_HEIGHT * scale)
            .with_rotation(Quat::from_euler(
                EulerRot::YXZ,
                r.y.to_radians(),
                r.x.to_radians(),
                r.z.to_radians(),
            ))
            .with_scale(Vec3::splat(scale))
    }
}

#[derive(Component)]
struct PapMachine;

/// Put the machine on the map while we're in a `Zombies` game on a map with
/// the power switch (and take it away otherwise), where the panel says.
fn sync_pap_machine(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    settings: Res<PapSettings>,
    asset_server: Res<AssetServer>,
    mut machines: Query<(Entity, &mut Transform), With<PapMachine>>,
    mut commands: Commands,
) {
    let here = zombies_game(&local, &lobbies).is_some_and(|l| shared::power::switch_pos(l.map).is_some());
    if !here {
        for (e, _) in &machines {
            commands.entity(e).despawn();
        }
        return;
    }
    if machines.is_empty() {
        commands.spawn((
            StateScoped(AppState::InGame),
            PapMachine,
            // (The root's at the model's middle already.)
            crate::power::PoweredHum(Vec3::ZERO),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(PAP_MODEL))),
            settings.transform(),
        ));
        return;
    }
    for (_, mut t) in &mut machines {
        t.set_if_neq(settings.transform());
    }
}
