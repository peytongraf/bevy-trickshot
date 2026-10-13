//! Fog/sun/ambient/bloom "look" per map, and the sun's shadow-map quality —
//! live-tunable from the debug panel, pushed onto the actual light/fog/bloom
//! components whenever the tuning or the selected map changes.

use bevy::core_pipeline::bloom::Bloom;
use bevy::pbr::{
    CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap, DistanceFog,
    FogFalloff,
};
use bevy::color::Mix;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::player::WorldModelCamera;
use crate::settings::{Settings, ShadowQuality};
use crate::util::{color_from_parts, srgb_parts};

use super::map::CurrentMap;
use super::ground_fog::{ground_fog_sliders, GroundFogLook};
use super::sky::{sky_look_sliders, SkyLook};

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

/// One map's look — its fog, sun, ambient light, bloom and (separately, since
/// the fog never reaches it) its sky. Every map has its own, day and night
/// versions alike ([`MapLooks`]), live-tweakable from the debug panel's
/// "Maps & lighting" → "Fog & Sky" → the map.
#[derive(Clone, Debug)]
pub(crate) struct SceneTuning {
    pub(crate) fog_visibility_m: f32,
    pub(crate) fog_color: [f32; 3],
    fog_sun_exponent: f32,
    sun_lux: f32,
    sun_color: [f32; 3],
    ambient_color: [f32; 3],
    ambient_lux: f32,
    bloom_intensity: f32,
    /// The low, uneven fog over the floor, on top of the even fog above.
    pub(crate) ground_fog: GroundFogLook,
    /// The sky's own look — the fog above never reaches it.
    pub(crate) sky: SkyLook,
}

impl SceneTuning {
    /// Print it to the console (the debug panel's "Copy to console"), in one
    /// line ready to paste back into [`Self::default_for`].
    pub(crate) fn log(&self, map: shared::MapId) {
        info!("fog & sky — {} ({map:?}): {self:?}", map.label());
    }

    /// `map`'s look before any tweaking.
    pub(crate) fn default_for(map: shared::MapId) -> Self {
        use shared::MapId;
        match map {
            // Bright, mostly-sunny midday — the `SUN_*` / `SKY_*` / `FOG_*` consts.
            MapId::BasicMap => Self {
                fog_visibility_m: FOG_VISIBILITY_M,
                fog_color: srgb_parts(FOG_COLOR),
                fog_sun_exponent: 100.0,
                sun_lux: SUN_LUX,
                sun_color: srgb_parts(SUN_COLOR),
                ambient_color: srgb_parts(SKY_AMBIENT_COLOR),
                ambient_lux: SKY_AMBIENT_LUX,
                bloom_intensity: 0.09,
                ground_fog: GroundFogLook::off(),
                sky: SkyLook::clear(srgb_parts(FOG_COLOR)),
            },
            // MW3-style: dark, foggy, overcast — near dawn/dusk under heavy
            // cloud, out on open water.
            MapId::Shipment => Self {
                fog_visibility_m: 200.0,
                // r24 g30 b37 (0-255) — a dark, cool overcast grey.
                fog_color: srgb_parts(Color::srgb(24.0 / 255.0, 30.0 / 255.0, 37.0 / 255.0)),
                fog_sun_exponent: 7.0,
                sun_lux: 1000.0,
                sun_color: srgb_parts(Color::srgb(0.75, 0.78, 0.85)),
                ambient_color: srgb_parts(Color::srgb(0.35, 0.38, 0.42)),
                ambient_lux: 50.0,
                bloom_intensity: 0.1,
                ground_fog: GroundFogLook::off(),
                sky: SkyLook::clear(srgb_parts(Color::srgb(24.0 / 255.0, 30.0 / 255.0, 37.0 / 255.0))),
            },
            // The same yard in bright midday sun: barely any fog (just enough
            // haze to soften the horizon over the water), a strong warm sun,
            // and a bright ambient fill so container shadows stay readable.
            MapId::ShipmentDay => Self {
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
                ground_fog: GroundFogLook::off(),
                sky: SkyLook::clear(srgb_parts(Color::srgb(225.0 / 255.0, 240.0 / 255.0, 255.0 / 255.0))),
            },
            // A clear mid-day with great visibility — the fog's pushed out to
            // tens of kilometres.
            MapId::BreakPoint => Self {
                fog_visibility_m: 40_000.0,
                // r225 g240 b255 (0-255) — a pale, cool sky-blue (barely visible).
                fog_color: srgb_parts(Color::srgb(225.0 / 255.0, 240.0 / 255.0, 255.0 / 255.0)),
                fog_sun_exponent: 3.0,
                sun_lux: 20_000.0,
                // r250 g240 b222 (0-255) — bright, slightly warm noon light.
                sun_color: srgb_parts(Color::srgb(250.0 / 255.0, 240.0 / 255.0, 222.0 / 255.0)),
                // r208 g215 b232 (0-255) — clear-sky fill.
                ambient_color: srgb_parts(Color::srgb(208.0 / 255.0, 215.0 / 255.0, 232.0 / 255.0)),
                ambient_lux: 240.0,
                bloom_intensity: 0.075,
                ground_fog: GroundFogLook::off(),
                sky: SkyLook::clear(srgb_parts(Color::srgb(225.0 / 255.0, 240.0 / 255.0, 255.0 / 255.0))),
            },
            // A soft, hazy day: a warm dusty fog a few kilometres out, a gentle
            // sun and the faintest mist over the temple floor.
            MapId::DerTempelDay => Self {
                fog_visibility_m: 5000.0,
                fog_color: [0.7991695, 0.7551609, 0.70117474],
                fog_sun_exponent: 2.0,
                sun_lux: 5000.0,
                sun_color: [0.98039216, 0.9411765, 0.87058824],
                ambient_color: [0.8156863, 0.84313726, 0.9098039],
                ambient_lux: 100.0,
                bloom_intensity: 0.14,
                ground_fog: GroundFogLook {
                    density: 0.001,
                    base_height: 0.0,
                    falloff_height: 5.0,
                    color: [0.6, 0.62, 0.66],
                    brightness: 1.0,
                    max_distance: 150.0,
                    max_opacity: 0.03,
                    splotch_size: 9.0,
                    splotchiness: 0.82,
                    contrast: 2.0,
                    seed: 480,
                    drift_speed: 0.0,
                    drift_dir_deg: 0.0,
                },
                sky: SkyLook {
                    brightness: 0.7,
                    tint: [1.0, 1.0, 1.0],
                    yaw_deg: 60.0,
                    haze: 0.03,
                    haze_color: [0.88235295, 0.94117635, 1.0],
                },
            },
            // Pitch dark: a thick near-black fog close in, a dim grey ambient
            // fill and a barely-there sky — the gun flashlights do the lighting.
            MapId::DerTempel => Self {
                fog_visibility_m: 120.0,
                fog_color: [0.040293436, 0.040293436, 0.040293436],
                fog_sun_exponent: 100.0,
                // No sun (colour kept only for the fog's sun-scatter tint).
                sun_lux: 0.0,
                sun_color: [0.5882353, 0.6666666, 0.8627451],
                ambient_color: [0.53333336, 0.53333336, 0.53333336],
                ambient_lux: 20.0,
                bloom_intensity: 0.095,
                // A cold mist pooled over the temple floor.
                ground_fog: GroundFogLook {
                    density: 0.05,
                    base_height: 0.0,
                    falloff_height: 2.0,
                    color: [0.12, 0.13, 0.15],
                    brightness: 1.0,
                    max_distance: 30.0,
                    max_opacity: 0.43,
                    splotch_size: 1.4,
                    splotchiness: 0.33,
                    contrast: 3.1,
                    seed: 600,
                    drift_speed: 1.25,
                    drift_dir_deg: 70.0,
                },
                sky: SkyLook {
                    brightness: 0.05,
                    tint: [0.26091984, 0.26091984, 0.26091984],
                    yaw_deg: -130.0,
                    haze: 0.0,
                    haze_color: [0.023529414, 0.03137255, 0.05490196],
                },
            },
            // Full night: no sun at all, a near-black blue fog closing in the
            // distance, and a flat grey ambient fill so the map reads as dim
            // shapes — the gun flashlights (`player::flashlight`) do the real
            // lighting. (Basic Map by night starts out the same.)
            MapId::BreakPointNight | MapId::BasicMapNight => Self {
                fog_visibility_m: 500.0,
                // r6 g8 b14 (0-255) — near-black night blue.
                fog_color: srgb_parts(Color::srgb(6.0 / 255.0, 8.0 / 255.0, 14.0 / 255.0)),
                fog_sun_exponent: 100.0,
                // No sun (colour kept only for the fog's sun-scatter tint).
                sun_lux: 0.0,
                sun_color: srgb_parts(Color::srgb(150.0 / 255.0, 170.0 / 255.0, 220.0 / 255.0)),
                // r136 g136 b136 (0-255) — flat grey.
                ambient_color: srgb_parts(Color::srgb(136.0 / 255.0, 136.0 / 255.0, 136.0 / 255.0)),
                ambient_lux: 200.0,
                bloom_intensity: 0.095,
                ground_fog: GroundFogLook::off(),
                sky: SkyLook::clear(srgb_parts(Color::srgb(6.0 / 255.0, 8.0 / 255.0, 14.0 / 255.0))),
            },
        }
    }
}

/// One map's fog/sun/ambient/bloom/sky sliders (the debug panel's
/// "Fog & Sky" section has one set per map).
pub(crate) fn scene_tuning_sliders(ui: &mut egui::Ui, s: &mut SceneTuning) {
    ui.add(
        egui::Slider::new(&mut s.fog_visibility_m, 20.0f32..=100_000.0)
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
    ui.separator();
    ui.collapsing("Ground fog", |ui| ground_fog_sliders(ui, &mut s.ground_fog));
    ui.separator();
    ui.label("Sky (the fog doesn't reach it)");
    sky_look_sliders(ui, &mut s.sky);
}

/// Every map's look ([`SceneTuning`]), one per [`shared::MapId::ALL`] —
/// read by `apply_scene_tuning` and the sky (`sky::apply_sky_look`).
#[derive(Resource)]
pub(crate) struct MapLooks([SceneTuning; shared::MapId::ALL.len()]);

impl Default for MapLooks {
    fn default() -> Self {
        Self(shared::MapId::ALL.map(SceneTuning::default_for))
    }
}

impl MapLooks {
    fn index(map: shared::MapId) -> usize {
        shared::MapId::ALL.iter().position(|&m| m == map).expect("every map is in `MapId::ALL`")
    }

    /// The look `map` uses.
    pub(crate) fn get(&self, map: shared::MapId) -> &SceneTuning {
        &self.0[Self::index(map)]
    }

    pub(crate) fn get_mut(&mut self, map: shared::MapId) -> &mut SceneTuning {
        &mut self.0[Self::index(map)]
    }
}

/// Push the current map's [`SceneTuning`] onto the live fog / sun / ambient /
/// bloom whenever it or the map changes (also once at startup) — there's only
/// one of each in the world, so switching maps re-points them at a different
/// look rather than swapping entities.
pub(crate) fn apply_scene_tuning(
    current: Res<CurrentMap>,
    looks: Res<MapLooks>,
    dog_fade: Res<super::DogRoundFade>,
    dog_look: Res<super::DogRoundLook>,
    mut ambient: ResMut<AmbientLight>,
    mut sun: Single<&mut DirectionalLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
    mut bloom: Single<&mut Bloom, With<WorldModelCamera>>,
) {
    if !current.is_changed() && !looks.is_changed() && !dog_fade.is_changed() && !dog_look.is_changed() {
        return;
    }
    let active = looks.get(current.0);

    // A dog round darkens all of it, `t` of the way (`dog_round.rs`).
    let t = dog_fade.0;
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    let mix = |a: [f32; 3], b: [f32; 3]| color_from_parts(a).mix(&color_from_parts(b), t);
    let sun_mult = lerp(1.0, dog_look.sun_mult);

    ambient.color = mix(active.ambient_color, dog_look.ambient_color);
    ambient.brightness = active.ambient_lux * lerp(1.0, dog_look.ambient_mult);

    sun.illuminance = active.sun_lux * sun_mult;
    sun.color = color_from_parts(active.sun_color);

    fog.color = mix(active.fog_color, dog_look.fog_color);
    fog.directional_light_color = Color::LinearRgba(LinearRgba::from(color_from_parts(active.sun_color)) * sun_mult);
    fog.directional_light_exponent = active.fog_sun_exponent;
    fog.falloff = FogFalloff::from_visibility(lerp(
        active.fog_visibility_m,
        dog_look.fog_visibility_m.min(active.fog_visibility_m),
    ));

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
