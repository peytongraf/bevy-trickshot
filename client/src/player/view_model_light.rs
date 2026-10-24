//! Lighting the first-person gun and arms like the world around them.
//!
//! They're drawn by their own camera on [`crate::VIEW_MODEL_RENDER_LAYER`],
//! and a Bevy light only lights meshes on a layer it shares — one with no
//! `RenderLayers` is on layer 0 alone. So every point / spot light (map and
//! power lights, other players' flashlights, machines, fire…) that doesn't
//! pick its own layers is put on the world's and the view model's both, the
//! way the sun is (`setup_world`); without that, only the sun and ambient
//! light ever reached the gun, and on night maps it went near-black next to
//! a lamp-lit floor.
//!
//! The view model also casts no shadows: the sun lighting its layer would
//! otherwise drop a floating gun-and-arms shadow on the ground (there's no
//! body to go with it).

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::RenderLayers;

use crate::VIEW_MODEL_RENDER_LAYER;

pub(crate) struct ViewModelLightPlugin;

impl Plugin for ViewModelLightPlugin {
    fn build(&self, app: &mut App) {
        // (`PostUpdate`, so whatever spawned this frame is covered before
        // it's first drawn.)
        app.add_systems(PostUpdate, (light_view_model, no_view_model_shadows));
    }
}

#[allow(clippy::type_complexity)]
fn light_view_model(
    lights: Query<Entity, (Or<(Added<PointLight>, Added<SpotLight>)>, Without<RenderLayers>)>,
    mut commands: Commands,
) {
    for e in &lights {
        commands
            .entity(e)
            .insert(RenderLayers::from_layers(&[0, VIEW_MODEL_RENDER_LAYER]));
    }
}

#[allow(clippy::type_complexity)]
fn no_view_model_shadows(
    meshes: Query<(Entity, &RenderLayers), (With<Mesh3d>, Without<NotShadowCaster>, Changed<RenderLayers>)>,
    mut commands: Commands,
) {
    let view_model = RenderLayers::layer(VIEW_MODEL_RENDER_LAYER);
    for (e, layers) in &meshes {
        if layers.intersects(&view_model) {
            commands.entity(e).insert(NotShadowCaster);
        }
    }
}
