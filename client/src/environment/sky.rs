//! The HDR sky sphere backdrop, recentred on the camera every frame so its
//! edge is never reached, and swapped per map.

use bevy::prelude::*;

use crate::WorldModelCamera;

use super::map::CurrentMap;

/// Radius of the sky sphere. Kept inside the camera far plane; the sphere
/// follows the camera so the player never reaches its edge.
pub(crate) const SKY_RADIUS: f32 = 900.0;

/// The HDR sky sphere; recentred on the camera every frame.
#[derive(Component)]
pub(crate) struct SkySphere;

/// [`SkySphere`]'s material handle, so [`sync_sky_texture`] can look it up in
/// `Assets<StandardMaterial>` and swap its `base_color_texture` per map — a
/// plain `Query` can't mutate a shared material asset, only the handle
/// pointing at one (mirrors [`WaterMaterial`]).
#[derive(Component)]
pub(crate) struct SkyMaterial(pub(crate) Handle<StandardMaterial>);

/// The equirectangular HDR [`SkySphere`] shows for `map` — a clear-sky
/// backdrop for `basic_map.glb`'s open field, an overcast one for
/// `shipment.glb`'s cargo-ship-at-sea setting (a bright sunny sky reads a
/// bit odd out over open, ostensibly rougher water). `ShipmentDay` is the
/// same ship in full sun, so it reuses `basic_map.glb`'s clear-sky HDR.
/// `break_point_map.glb` gets a clear mid-day sky (sunflowers), or a clear
/// night one (qwantani) for `BreakPointNight`.
pub(crate) fn sky_texture_path(map: shared::MapId) -> &'static str {
    match map {
        shared::MapId::BasicMap => "skybox/citrus_orchard_puresky_8k.hdr",
        shared::MapId::Shipment => "skybox/overcast_soil_puresky_8k.hdr",
        shared::MapId::ShipmentDay => "skybox/citrus_orchard_puresky_8k.hdr",
        shared::MapId::BreakPoint => "skybox/sunflowers_puresky_8k.hdr",
        shared::MapId::BreakPointNight => "skybox/qwantani_night_puresky_8k.hdr",
    }
}

/// Swaps [`SkySphere`]'s HDR to match [`CurrentMap`] (see
/// [`sky_texture_path`]) — only reacts to an actual map change, same
/// reasoning as [`sync_shipment_only_visibility`] (the sphere is spawned once in
/// `setup_world` and never respawned). `AssetServer::load` dedupes by path,
/// so switching back and forth between maps after the first load of each
/// doesn't re-read either file from disk.
pub(crate) fn sync_sky_texture(
    current: Res<CurrentMap>,
    asset_server: Res<AssetServer>,
    sky: Query<&SkyMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !current.is_changed() {
        return;
    }
    let Ok(sky) = sky.single() else {
        return;
    };
    let Some(material) = materials.get_mut(&sky.0) else {
        return;
    };
    material.base_color_texture = Some(asset_server.load(sky_texture_path(current.0)));
}

/// Recentre the sky sphere on the camera so its edge is never reached (it uses
/// last frame's camera `GlobalTransform`, which is imperceptible at this scale).
pub(crate) fn sky_follow_camera(
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut sky: Single<&mut Transform, With<SkySphere>>,
) {
    sky.translation = camera.translation();
}
