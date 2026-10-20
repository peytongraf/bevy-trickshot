//! Keeps the big textures out of RAM once they're on the GPU.
//!
//! By default Bevy keeps a decoded copy of every image in the main world
//! (`RenderAssetUsages::MAIN_WORLD`) alongside the one it uploads — for a
//! texture nothing on the CPU ever reads again, that's pure waste. The
//! models' embedded textures (a 4K one is 64 MB decoded; ~4.4 GB over every
//! `.glb` in `assets/`) and the 8K HDR skies (512 MB each, as float RGBA)
//! are exactly that, and with two clients open side by side they were
//! enough to run the machine out of memory and get one killed.
//!
//! So each one is marked render-world-only the frame it's loaded, just
//! before it's extracted: Bevy then moves it to the GPU and frees the main
//! world's copy (`bevy_render::render_asset::extract_render_asset`). Its
//! handle keeps working for materials as before; only `Assets<Image>::get`
//! on it comes back empty, which nothing does for these.
//!
//! Images the game builds itself (the minimap bake, name tags, the ground)
//! are left alone — some of those are read back on the CPU.

use bevy::asset::AssetEvents;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;

pub(crate) struct AssetMemoryPlugin;

impl Plugin for AssetMemoryPlugin {
    fn build(&self, app: &mut App) {
        // (After this frame's asset events are out, before extraction reads
        // them — so it's uploaded once, already render-world-only.)
        app.add_systems(PostUpdate, gpu_only_textures.after(AssetEvents));
    }
}

/// Whether an image loaded from `path` only ever goes to the GPU: one
/// embedded in a model, or a sky.
fn gpu_only(path: &bevy::asset::AssetPath) -> bool {
    matches!(
        path.path().extension().and_then(|e| e.to_str()),
        Some("glb" | "gltf" | "hdr")
    )
}

fn gpu_only_textures(
    mut events: EventReader<AssetEvent<Image>>,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else { continue };
        if !asset_server.get_path(*id).is_some_and(|p| gpu_only(&p)) {
            continue;
        }
        if let Some(image) = images.get_mut(*id) {
            image.asset_usage = RenderAssetUsages::RENDER_WORLD;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_model_and_sky_textures_go_gpu_only() {
        assert!(gpu_only(&"models/weapons/m17.glb#Texture3".into()));
        assert!(gpu_only(&"skybox/sunflowers_puresky_8k.hdr".into()));
        assert!(!gpu_only(&"textures/environment/water_normal.png".into()));
        assert!(!gpu_only(&"ui/menu_art.png".into()));
    }
}
