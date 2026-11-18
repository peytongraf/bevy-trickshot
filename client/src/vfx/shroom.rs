//! Shroom screen effect: a subtle, slow "breathing" wavy distortion of the
//! whole first-person view plus a saturation / hue shimmer — the look of the
//! future mushroom perk. For now it's just toggled and tuned from the debug
//! panel (`ads_tuning_ui`'s "Shroom effect" section).
//!
//! One fullscreen post-process pass (`assets/shaders/shroom.wesl`) run after
//! tonemapping on the [`ViewModelCamera`] — the last 3D camera to draw, and
//! it shares its render target with the world camera, so the pass sees world
//! and gun together. The HUD (its own 2D camera) and egui aren't distorted.
//! While the effect is off (or fully faded out) the pass doesn't run at all.

use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{DynamicUniformIndex, ExtractComponent};
use bevy::render::sync_component::SyncComponent;
use bevy::render::RenderApp;

use super::perk_kick::{KickSettings, KickState};
use crate::player::ViewModelCamera;

const SHADER_ASSET_PATH: &str = "shaders/shroom.wesl";

/// Panel-adjustable shroom look (`ads_tuning_ui`'s "Shroom effect" section).
#[derive(Resource, Clone, Debug)]
pub(crate) struct ShroomSettings {
    pub(crate) enabled: bool,
    /// Seconds to fade fully in / out when toggled (0 = instant).
    pub(crate) fade_secs: f32,
    /// Warp displacement, as a fraction of screen height.
    pub(crate) wave_amplitude: f32,
    /// Spatial frequency of the waves (higher = smaller, busier ripples).
    pub(crate) wave_frequency: f32,
    /// How fast the waves flow.
    pub(crate) wave_speed: f32,
    /// Peak extra zoom of the slow "breathing" pulse (0.02 = 2%).
    pub(crate) breathe_amplitude: f32,
    /// Breathing pulse rate (radians / s).
    pub(crate) breathe_speed: f32,
    /// 0 = warp evenly everywhere, 1 = keep the screen centre (the aim) still.
    pub(crate) center_clear: f32,
    /// Colour fringing toward the screen edges.
    pub(crate) chromatic: f32,
    /// Saturation multiplier (1 = unchanged).
    pub(crate) saturation: f32,
    /// Peak hue rotation of the colour shimmer (radians).
    pub(crate) hue_drift: f32,
    /// How fast the hue shimmer moves.
    pub(crate) hue_speed: f32,
    /// See-through-walls enemy ghosts (`shroom_xray`): sRGB colour...
    pub(crate) xray_color: [f32; 3],
    /// ...glow multiplier (above 1 blooms)...
    pub(crate) xray_brightness: f32,
    /// ...peak opacity...
    pub(crate) xray_opacity: f32,
    /// ...how far (m) the haze puffs out past the body...
    pub(crate) xray_inflate: f32,
    /// ...base opacity right out at the edge (0 = edges fully fade)...
    pub(crate) xray_fill: f32,
    /// ...how quickly it fades toward the outline (higher = thinner, softer
    /// edge)...
    pub(crate) xray_edge_softness: f32,
    /// ...smoke wisp size (higher = finer), drift speed, and how much it
    /// breaks the haze up...
    pub(crate) xray_smoke_scale: f32,
    pub(crate) xray_smoke_speed: f32,
    pub(crate) xray_smoke_amount: f32,
    /// ...and the psychedelic hue wobble (radians).
    pub(crate) xray_shimmer: f32,
    /// ...and what counts as hiding an enemy: something in front of its
    /// whole body — a sphere this far (m) round its middle, this high (m)
    /// above its feet — so none of its own parts (an arm behind its back,
    /// its gun) ever light it up while it's in plain sight...
    pub(crate) xray_body_radius: f32,
    pub(crate) xray_body_height: f32,
    /// ...and at least this far (m) in front of the part it hides.
    pub(crate) xray_min_gap: f32,
    /// ...and, before any of that, whether the enemy's hidden at all: rays
    /// from the camera to points down its body, blocked only by the map and
    /// what's placed on it (other enemies and its own body never count). It
    /// glows from this share of them blocked (fading up to full when all
    /// are)...
    pub(crate) xray_hidden_min: f32,
    /// ...easing in and out over this long (s).
    pub(crate) xray_cover_fade_secs: f32,
    /// Aim assist (`player::shroom_aim_assist`) while aimed down sight: on /
    /// off...
    pub(crate) assist_enabled: bool,
    /// ...how far off the crosshair (degrees) a bot still pulls...
    pub(crate) assist_cone_deg: f32,
    /// ...how hard it pulls (per second — higher snaps on quicker)...
    pub(crate) assist_strength: f32,
    /// ...the fastest it may turn the aim (degrees / s)...
    pub(crate) assist_max_speed_deg: f32,
    /// ...how far (m) away a bot can be...
    pub(crate) assist_range: f32,
    /// ...and how scoped in (`Ads::t`, 0..=1) the player must be.
    pub(crate) assist_min_ads: f32,
    /// The strong spell of the screen effect right after it switches on (the
    /// x-ray and aim assist aren't affected).
    pub(crate) kick: KickSettings,
}

impl Default for ShroomSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            fade_secs: 3.0,
            wave_amplitude: 0.01,
            wave_frequency: 4.0,
            wave_speed: 0.5,
            breathe_amplitude: 0.07,
            breathe_speed: 0.7,
            center_clear: 1.0,
            chromatic: 0.023,
            saturation: 1.65,
            hue_drift: 0.15,
            hue_speed: 1.25,
            // White-hot yellow.
            xray_color: [1.0, 0.95, 0.62],
            xray_brightness: 3.0,
            xray_opacity: 0.85,
            xray_inflate: 0.06,
            xray_fill: 0.1,
            xray_edge_softness: 1.5,
            xray_smoke_scale: 3.0,
            xray_smoke_speed: 0.8,
            xray_smoke_amount: 0.6,
            xray_shimmer: 0.35,
            xray_body_radius: 0.9,
            xray_body_height: 0.9,
            xray_min_gap: 0.5,
            xray_hidden_min: 0.34,
            xray_cover_fade_secs: 0.15,
            assist_enabled: true,
            assist_cone_deg: 4.0,
            assist_strength: 5.0,
            assist_max_speed_deg: 25.0,
            assist_range: 150.0,
            assist_min_ads: 0.8,
            kick: KickSettings::default(),
        }
    }
}

/// Whether the local player has the `Zombies` Shroom Tea perk right now (set
/// by `zombies_hud::sync_shroom_perk`) — turns the effect on alongside the
/// debug panel's own toggle.
#[derive(Resource, Default)]
pub(crate) struct ShroomPerk(pub(crate) bool);

/// How far the effect is faded in right now, 0..=1 — eased toward
/// `ShroomSettings::enabled` over `fade_secs` by [`sync_shroom`].
#[derive(Resource, Default)]
pub(crate) struct ShroomLevel(pub(crate) f32);

/// The screen effect's kick-in (`pub(crate)` for the debug panel's replay).
#[derive(Resource, Default)]
pub(crate) struct ShroomKick(pub(crate) KickState);

/// Per-camera uniform the shader reads. Lives on the [`ViewModelCamera`];
/// only extracted to the render world while `strength > 0`, so the pass is
/// skipped entirely when the effect is off.
pub(crate) use uniform::ShroomUniform;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct.
#[allow(dead_code)]
mod uniform {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(Component, Default, Clone, Copy, ShaderType)]
    pub(crate) struct ShroomUniform {
        pub(crate) strength: f32,
        pub(crate) time: f32,
        pub(crate) wave_amplitude: f32,
        pub(crate) wave_frequency: f32,
        pub(crate) wave_speed: f32,
        pub(crate) breathe_amplitude: f32,
        pub(crate) breathe_speed: f32,
        pub(crate) center_clear: f32,
        pub(crate) chromatic: f32,
        pub(crate) saturation: f32,
        pub(crate) hue_drift: f32,
        pub(crate) hue_speed: f32,
    }
}

impl SyncComponent<RenderApp> for ShroomUniform {
    // (Its uniform index and pipeline go with it once it stops being
    // extracted.)
    type Target = (Self, DynamicUniformIndex<Self>, super::fullscreen_pass::ViewPassPipeline<Self>);
}

impl ExtractComponent<RenderApp> for ShroomUniform {
    type QueryData = &'static ShroomUniform;
    type QueryFilter = ();
    type Out = ShroomUniform;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        (item.strength > 0.0).then_some(*item)
    }
}

impl super::fullscreen_pass::FullscreenPass for ShroomUniform {
    const LABEL: &'static str = "shroom";
    const SHADER: &'static str = SHADER_ASSET_PATH;
}

pub(crate) struct ShroomPlugin;

impl Plugin for ShroomPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShroomSettings>()
            .init_resource::<ShroomLevel>()
            .init_resource::<ShroomKick>()
            .init_resource::<ShroomPerk>()
            .add_systems(Update, sync_shroom);

        super::fullscreen_pass::add::<ShroomUniform, _>(app, bevy::core_pipeline::tonemapping::tonemapping);
    }
}

/// Fades [`ShroomLevel`] toward the toggle and copies the settings onto the
/// view-model camera's [`ShroomUniform`] (adding it the first time a camera
/// shows up — the cameras are respawned each game).
fn sync_shroom(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<ShroomSettings>,
    perk: Res<ShroomPerk>,
    mut level: ResMut<ShroomLevel>,
    mut kick: ResMut<ShroomKick>,
    new_cams: Query<Entity, (With<ViewModelCamera>, Without<ShroomUniform>)>,
    mut uniforms: Query<&mut ShroomUniform, With<ViewModelCamera>>,
) {
    for cam in &new_cams {
        commands.entity(cam).insert(ShroomUniform::default());
    }

    let target = if settings.enabled || perk.0 { 1.0 } else { 0.0 };
    level.0 = if settings.fade_secs <= 0.0 {
        target
    } else {
        let step = time.delta_secs() / settings.fade_secs;
        level.0 + (target - level.0).clamp(-step, step)
    };
    // Smoothstep so the onset / wear-off eases in and out, not linearly —
    // times the kick-in's boost (1 once it's settled).
    let boost = kick.0.update(target > 0.0, time.delta_secs(), &settings.kick);
    let strength = level.0 * level.0 * (3.0 - 2.0 * level.0) * settings.kick.multiplier(boost);

    let s = &*settings;
    for mut u in &mut uniforms {
        *u = ShroomUniform {
            strength,
            time: time.elapsed_secs_wrapped(),
            wave_amplitude: s.wave_amplitude,
            wave_frequency: s.wave_frequency,
            wave_speed: s.wave_speed,
            breathe_amplitude: s.breathe_amplitude,
            breathe_speed: s.breathe_speed,
            center_clear: s.center_clear,
            chromatic: s.chromatic,
            saturation: s.saturation,
            hue_drift: s.hue_drift,
            hue_speed: s.hue_speed,
        };
    }
}
