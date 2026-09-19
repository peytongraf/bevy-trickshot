//! Fog/sun/ambient/bloom "look" per map, and the sun's shadow-map quality —
//! live-tunable from the debug panel, pushed onto the actual light/fog/bloom
//! components whenever the tuning or the selected map changes.

use bevy::core_pipeline::bloom::Bloom;
use bevy::pbr::{
    CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap, DistanceFog,
    FogFalloff,
};
use bevy::prelude::*;
use bevy_egui::egui;

use crate::player::WorldModelCamera;
use crate::settings::{Settings, ShadowQuality};
use crate::util::{color_from_parts, srgb_parts};

use super::map::CurrentMap;

// --- daylight look (bright, mostly-sunny midday) -----------------------------
// These are calibrated against Bevy's *default* camera exposure — a genuinely
// physical sun (~100k lux) would also need a `Exposure` retune on all three
// cameras, which is a separate pass. Tune these in-game.
/// Directional "sun" strength (lux).
pub(crate) const SUN_LUX: f32 = 28_000.0;
/// Sun colour — a faint warm (~6000 K).
pub(crate) const SUN_COLOR: Color = Color::srgb(1.0, 0.95, 0.88);
/// Ambient fill, tinted like a clear sky so shadows read blue rather than black.
pub(crate) const SKY_AMBIENT_COLOR: Color = Color::srgb(0.60, 0.73, 0.92);
pub(crate) const SKY_AMBIENT_LUX: f32 = 200.0;
/// Distance haze — pale sky blue; this is "sunny with air", not "foggy".
pub(crate) const FOG_COLOR: Color = Color::srgb(0.72, 0.80, 0.90);
/// Roughly the distance (m) at which geometry fades fully into the haze.
pub(crate) const FOG_VISIBILITY_M: f32 = 1300.0;

/// `basic_map.glb`'s daytime look, live-tweakable from the debug panel's
/// "Fog & Sky (Basic Map)" section and pushed onto the fog / sun / ambient /
/// bloom by `apply_scene_tuning` whenever that's the selected map — see
/// [`ShipmentSceneTuning`] for `Shipment`'s (separately tunable) look.
/// Defaults mirror the `SUN_*` / `SKY_*` / `FOG_*` consts.
#[derive(Resource)]
pub(crate) struct SceneTuning {
    fog_visibility_m: f32,
    fog_color: [f32; 3],
    fog_sun_exponent: f32,
    sun_lux: f32,
    sun_color: [f32; 3],
    ambient_color: [f32; 3],
    ambient_lux: f32,
    bloom_intensity: f32,
}

impl Default for SceneTuning {
    fn default() -> Self {
        Self {
            fog_visibility_m: FOG_VISIBILITY_M,
            fog_color: srgb_parts(FOG_COLOR),
            fog_sun_exponent: 100.0,
            sun_lux: SUN_LUX,
            sun_color: srgb_parts(SUN_COLOR),
            ambient_color: srgb_parts(SKY_AMBIENT_COLOR),
            ambient_lux: SKY_AMBIENT_LUX,
            bloom_intensity: 0.09,
        }
    }
}

/// `shipment.glb`'s look — same shape as [`SceneTuning`] (see that struct's
/// fields), just a separate resource so `Shipment` can be tuned to its own
/// MW3-style setting (dark, foggy, overcast — near dawn/dusk under heavy
/// cloud, out on open water) without touching `basic_map.glb`'s daytime one.
/// Live-tweakable from the debug panel's "Fog & Sky (Shipment)" section;
/// `apply_scene_tuning` pushes whichever of the two is currently selected
/// ([`CurrentMap`]) onto the shared fog / sun / ambient / bloom.
#[derive(Resource)]
pub(crate) struct ShipmentSceneTuning(pub(crate) SceneTuning);

impl Default for ShipmentSceneTuning {
    fn default() -> Self {
        Self(SceneTuning {
            fog_visibility_m: 200.0,
            // r24 g30 b37 (0-255) — a dark, cool overcast grey.
            fog_color: srgb_parts(Color::srgb(24.0 / 255.0, 30.0 / 255.0, 37.0 / 255.0)),
            fog_sun_exponent: 7.0,
            sun_lux: 1000.0,
            sun_color: srgb_parts(Color::srgb(0.75, 0.78, 0.85)),
            ambient_color: srgb_parts(Color::srgb(0.35, 0.38, 0.42)),
            ambient_lux: 50.0,
            bloom_intensity: 0.1,
        })
    }
}

/// `shipment.glb`'s daytime look ([`shared::MapId::ShipmentDay`]) — the same
/// yard as [`ShipmentSceneTuning`], but in bright midday sun instead of dark,
/// foggy, rainy dusk: barely any fog (just enough distance haze to soften the
/// horizon over the water), a strong warm sun, and a bright sky-blue ambient
/// fill so container shadows stay readable rather than going black. Paired
/// with `basic_map.glb`'s clear-sky HDR (see `sky_texture_path`) and no rain
/// (`update_rain` only runs for `Shipment`). Live-tweakable from the debug
/// panel's "Fog & Sky (Shipment Day)" section.
#[derive(Resource)]
pub(crate) struct ShipmentDaySceneTuning(pub(crate) SceneTuning);

impl Default for ShipmentDaySceneTuning {
    fn default() -> Self {
        Self(SceneTuning {
            fog_visibility_m: 1200.0,
            // r225 g240 b255 (0-255) — a pale, cool sky-blue haze.
            fog_color: srgb_parts(Color::srgb(225.0 / 255.0, 240.0 / 255.0, 255.0 / 255.0)),
            fog_sun_exponent: 3.0,
            sun_lux: 18_000.0,
            // r241 g233 b217 (0-255).
            sun_color: srgb_parts(Color::srgb(241.0 / 255.0, 233.0 / 255.0, 217.0 / 255.0)),
            // r208 g209 b221 (0-255).
            ambient_color: srgb_parts(Color::srgb(208.0 / 255.0, 209.0 / 255.0, 221.0 / 255.0)),
            ambient_lux: 200.0,
            bloom_intensity: 0.075,
        })
    }
}

/// Fog/sun/ambient/bloom sliders shared by "Fog & Sky (Basic Map)",
/// "Fog & Sky (Shipment)" and "Fog & Sky (Shipment Day)" — same [`SceneTuning`] shape, different resource
/// (and therefore different defaults) behind each.
pub(crate) fn scene_tuning_sliders(ui: &mut egui::Ui, s: &mut SceneTuning) {
    ui.add(
        egui::Slider::new(&mut s.fog_visibility_m, 20.0f32..=2000.0)
            .logarithmic(true)
            .text("fog visibility (m)"),
    );
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.fog_color);
        ui.label("fog colour");
    });
    ui.add(
        egui::Slider::new(&mut s.fog_sun_exponent, 1.0f32..=100.0).text("sun-scatter tightness"),
    );
    ui.separator();
    ui.add(egui::Slider::new(&mut s.sun_lux, 0.0f32..=120_000.0).text("sun (lux)"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.sun_color);
        ui.label("sun colour");
    });
    ui.add(egui::Slider::new(&mut s.ambient_lux, 0.0f32..=6000.0).text("sky ambient (lux)"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.ambient_color);
        ui.label("ambient colour");
    });
    ui.add(egui::Slider::new(&mut s.bloom_intensity, 0.0f32..=0.5).text("bloom"));
}

/// Push `SceneTuning` onto the live fog / sun / ambient / bloom whenever it
/// changes (also once at startup, which just re-applies the consts).
/// Picks whichever of [`SceneTuning`] (`BasicMap`), [`ShipmentSceneTuning`]
/// (`Shipment`) or [`ShipmentDaySceneTuning`] (`ShipmentDay`) is currently selected and pushes it onto the shared fog /
/// sun / ambient light / bloom — there's only one of each in the world, so
/// switching maps re-points them at a different look rather than swapping
/// entities.
pub(crate) fn apply_scene_tuning(
    current: Res<CurrentMap>,
    scene: Res<SceneTuning>,
    shipment_scene: Res<ShipmentSceneTuning>,
    shipment_day_scene: Res<ShipmentDaySceneTuning>,
    mut ambient: ResMut<AmbientLight>,
    mut sun: Single<&mut DirectionalLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
    mut bloom: Single<&mut Bloom, With<WorldModelCamera>>,
) {
    if !current.is_changed()
        && !scene.is_changed()
        && !shipment_scene.is_changed()
        && !shipment_day_scene.is_changed()
    {
        return;
    }
    let active = match current.0 {
        shared::MapId::BasicMap => &*scene,
        shared::MapId::Shipment => &shipment_scene.0,
        shared::MapId::ShipmentDay => &shipment_day_scene.0,
    };

    ambient.color = color_from_parts(active.ambient_color);
    ambient.brightness = active.ambient_lux;

    sun.illuminance = active.sun_lux;
    sun.color = color_from_parts(active.sun_color);

    fog.color = color_from_parts(active.fog_color);
    fog.directional_light_color = color_from_parts(active.sun_color);
    fog.directional_light_exponent = active.fog_sun_exponent;
    fog.falloff = FogFalloff::from_visibility(active.fog_visibility_m);

    bloom.intensity = active.bloom_intensity;
}

/// Pushes `Settings::shadow_quality` onto the sun's shadow map whenever it
/// changes. Mirrors Call of Duty's "Shadow Map" option: Disabled turns shadows
/// off outright, and each tier up trades performance for resolution / cascade
/// count / draw distance.
pub(crate) fn apply_shadow_quality(
    settings: Res<Settings>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut sun: Single<(&mut DirectionalLight, &mut CascadeShadowConfig)>,
    mut applied: Local<Option<ShadowQuality>>,
) {
    if applied.is_some_and(|q| q == settings.shadow_quality) {
        return;
    }
    *applied = Some(settings.shadow_quality);

    let (light, cascades) = &mut *sun;
    let (enabled, size, num_cascades, maximum_distance) = match settings.shadow_quality {
        ShadowQuality::Disabled => (false, shadow_map.size, 1, 40.0),
        ShadowQuality::Low => (true, 512, 1, 40.0),
        ShadowQuality::Normal => (true, 1024, 2, 80.0),
        ShadowQuality::High => (true, 2048, 4, 120.0),
        ShadowQuality::Extra => (true, 4096, 4, 160.0),
    };

    light.shadows_enabled = enabled;
    shadow_map.size = size;
    **cascades = CascadeShadowConfigBuilder {
        num_cascades,
        first_cascade_far_bound: 20.0,
        maximum_distance,
        ..default()
    }
    .build();
}
