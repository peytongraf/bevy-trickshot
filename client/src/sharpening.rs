//! AMD's contrast-adaptive sharpening on the picture — a player setting
//! (`Settings::sharpening`), as Call of Duty has. On the world and scope
//! cameras only: the gun's camera draws over a see-through background,
//! which sharpening would fringe.
//!
//! Nothing here carries between games: the cameras are set afresh whenever
//! they're spawned or the setting changes.

use bevy::core_pipeline::contrast_adaptive_sharpening::ContrastAdaptiveSharpening;
use bevy::prelude::*;

use crate::player::WorldModelCamera;
use crate::settings::Settings;
use crate::weapons::ScopeCamera;

pub(crate) struct SharpeningPlugin;

impl Plugin for SharpeningPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, apply_sharpening);
    }
}

/// Sharpen the world and scope cameras' pictures by `Settings::sharpening`
/// (`0` off) — whenever it changes, or a camera's new.
#[allow(clippy::type_complexity)]
fn apply_sharpening(
    settings: Res<Settings>,
    cameras: Query<(Entity, Ref<Camera>, Option<&ContrastAdaptiveSharpening>), Or<(With<WorldModelCamera>, With<ScopeCamera>)>>,
    mut commands: Commands,
) {
    let strength = settings.sharpening.clamp(0.0, 1.0);
    for (entity, camera, cas) in &cameras {
        let want_on = strength > 0.0;
        let same = cas.is_some_and(|c| c.enabled == want_on && c.sharpening_strength == strength);
        if same && !camera.is_added() {
            continue;
        }
        commands.entity(entity).insert(ContrastAdaptiveSharpening {
            enabled: want_on,
            sharpening_strength: strength,
            denoise: false,
        });
    }
}
