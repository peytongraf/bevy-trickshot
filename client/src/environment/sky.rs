//! The HDR sky sphere backdrop, recentred on the camera every frame so its
//! edge is never reached, and swapped per map.
//!
//! The map's distance fog never touches the sky (its material has fog
//! switched off) — each map's [`SkyLook`] (debug panel: "Fog & Sky" → the
//! map → "Sky") sets the sky's own brightness, tint, turn and haze instead.

use std::f32::consts::FRAC_PI_2;

use bevy::color::Mix;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::util::color_from_parts;
use crate::WorldModelCamera;

use super::atmosphere::MapLooks;
use super::dog_round::{DogRoundFade, DogRoundLook};
use super::map::CurrentMap;

/// A map's sky look — separate from its fog, which doesn't reach the sky.
/// Part of each map's [`super::SceneTuning`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct SkyLook {
    /// The HDR's brightness (1 = as shot).
    pub(crate) brightness: f32,
    /// Its tint (sRGB multiplier).
    pub(crate) tint: [f32; 3],
    /// How far (degrees) it's turned about the vertical.
    pub(crate) yaw_deg: f32,
    /// A see-through haze over it ([`SkyHazeVeil`]), 0 = clear, 1 = solid...
    pub(crate) haze: f32,
    /// ...in this colour.
    pub(crate) haze_color: [f32; 3],
}

impl SkyLook {
    /// An untouched sky, with `haze_color` ready for when haze is dialled in.
    pub(crate) fn clear(haze_color: [f32; 3]) -> Self {
        Self {
            brightness: 1.0,
            tint: [1.0; 3],
            yaw_deg: 0.0,
            haze: 0.0,
            haze_color,
        }
    }
}

/// The debug panel's sky sliders (inside each map's "Fog & Sky" section).
pub(crate) fn sky_look_sliders(ui: &mut egui::Ui, sky: &mut SkyLook) {
    ui.add(egui::Slider::new(&mut sky.brightness, 0.0f32..=4.0).text("sky brightness"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut sky.tint);
        ui.label("sky tint");
    });
    ui.add(egui::Slider::new(&mut sky.yaw_deg, -180.0f32..=180.0).text("sky turn (°)"));
    ui.add(egui::Slider::new(&mut sky.haze, 0.0f32..=1.0).text("sky haze"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut sky.haze_color);
        ui.label("sky haze colour");
    });
}

/// Radius of the sky sphere. Kept inside the camera far plane; the sphere
/// follows the camera so the player never reaches its edge.
pub(crate) const SKY_RADIUS: f32 = 900.0;

/// Radius of [`SkyHazeVeil`] — just inside the sky.
pub(crate) const SKY_HAZE_RADIUS: f32 = SKY_RADIUS - 5.0;

/// The sky's haze ([`SkyLook::haze`], thickened toward
/// [`DogRoundLook::sky_fog`] during a dog round), and its material: a
/// see-through shell just inside the sky. Hidden while there's none.
#[derive(Component)]
pub(crate) struct SkyHazeVeil(pub(crate) Handle<StandardMaterial>);

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
/// night one (qwantani) for `BreakPointNight` — which the other night maps
/// (`BasicMapNight`, `DerTempel`) use too, and `DerTempelDay` borrows the
/// sunflowers one.
pub(crate) fn sky_texture_path(map: shared::MapId) -> &'static str {
    match map {
        shared::MapId::BasicMap => "skybox/citrus_orchard_puresky_8k.hdr",
        shared::MapId::Shipment => "skybox/overcast_soil_puresky_8k.hdr",
        shared::MapId::ShipmentDay => "skybox/citrus_orchard_puresky_8k.hdr",
        shared::MapId::BreakPoint | shared::MapId::DerTempelDay => "skybox/sunflowers_puresky_8k.hdr",
        shared::MapId::BreakPointNight | shared::MapId::DerTempel | shared::MapId::BasicMapNight => {
            "skybox/qwantani_night_puresky_8k.hdr"
        }
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

/// Recentre the sky sphere (and its haze) on the camera so its edge is never
/// reached (it uses last frame's camera `GlobalTransform`, which is
/// imperceptible at this scale).
pub(crate) fn sky_follow_camera(
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut sky: Query<&mut Transform, Or<(With<SkySphere>, With<SkyHazeVeil>)>>,
) {
    for mut t in &mut sky {
        t.translation = camera.translation();
    }
}

/// Push the map's [`SkyLook`] onto the sky sphere and its haze shell,
/// blended toward the dog-round sky by [`DogRoundFade`] (`dog_round.rs`).
pub(crate) fn apply_sky_look(
    fade: Res<DogRoundFade>,
    dog_look: Res<DogRoundLook>,
    current: Res<CurrentMap>,
    looks: Res<MapLooks>,
    mut sky: Query<(&SkyMaterial, &mut Transform), With<SkySphere>>,
    mut veil: Query<(&SkyHazeVeil, &mut Visibility)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !fade.is_changed() && !dog_look.is_changed() && !current.is_changed() && !looks.is_changed() {
        return;
    }
    let look = looks.get(current.0).sky;
    let t = fade.0;
    if let Ok((sky, mut transform)) = sky.single_mut() {
        // Bevy's UV sphere has its poles on +Z/-Z; tip it so the equirect
        // map's zenith points up, then turn it.
        transform.rotation =
            Quat::from_rotation_y(look.yaw_deg.to_radians()) * Quat::from_rotation_x(-FRAC_PI_2);
        if let Some(material) = materials.get_mut(&sky.0) {
            let own = LinearRgba::from(color_from_parts(look.tint)) * look.brightness;
            let dark = LinearRgba::from(color_from_parts(dog_look.sky_tint)) * dog_look.sky_brightness;
            material.base_color = Color::LinearRgba(own * (1.0 - t) + dark * t);
        }
    }
    let Ok((veil, mut vis)) = veil.single_mut() else { return };
    let alpha = look.haze + (dog_look.sky_fog - look.haze) * t;
    vis.set_if_neq(if alpha > 0.0 { Visibility::Inherited } else { Visibility::Hidden });
    let Some(material) = materials.get_mut(&veil.0) else { return };
    let color = color_from_parts(look.haze_color).mix(&color_from_parts(dog_look.fog_color), t);
    material.base_color = color.with_alpha(alpha);
}
