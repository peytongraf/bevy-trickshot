//! Liquid Courage's screen effect: a subtle drunk look — the view sways and
//! rolls a little as if your head won't quite hold still, a faint double image
//! drifts apart and back together, the edges darken, and there's a warm
//! flush. It kicks in hard right after the perk's bought, then settles
//! ([`super::perk_kick`]). Tuned from the debug panel ("Zombies perks" → "Liquid
//! Courage").
//!
//! One fullscreen post-process pass (`assets/shaders/drunk.wesl`) on the
//! [`ViewModelCamera`], exactly like the shroom pass and chained right after
//! it (tonemapping → shroom → drunk), so with both perks the drunk pass works
//! on the already-shroomed frame and the two stack. Each pass only runs while
//! its own effect is faded in.

use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{DynamicUniformIndex, ExtractComponent};
use bevy::render::sync_component::SyncComponent;
use bevy::render::RenderApp;

use super::perk_kick::{KickSettings, KickState};
use crate::player::ViewModelCamera;

const SHADER_ASSET_PATH: &str = "shaders/drunk.wesl";

/// Panel-adjustable drunk look ("Liquid Courage" debug section).
#[derive(Resource, Clone)]
pub(crate) struct DrunkSettings {
    /// Debug: show it without owning the perk.
    pub(crate) enabled: bool,
    /// Seconds to fade fully in / out (0 = instant).
    pub(crate) fade_secs: f32,
    /// Peak roll of the head sway (degrees)...
    pub(crate) sway_roll_deg: f32,
    /// ...peak drift of it (fraction of screen height)...
    pub(crate) sway_drift: f32,
    /// ...and how fast it wanders.
    pub(crate) sway_speed: f32,
    /// Double vision: how far apart the two images get (fraction of screen
    /// height)...
    pub(crate) double_offset: f32,
    /// ...how strong the ghost image is (0..=1)...
    pub(crate) double_mix: f32,
    /// ...and how fast it drifts apart and back.
    pub(crate) double_speed: f32,
    /// Double vision at the top of the kick-in: separation and ghost strength
    /// it climbs to (instead of scaling the two above by the kick's peak),
    /// then eases back down to them.
    pub(crate) double_offset_kick: f32,
    pub(crate) double_mix_kick: f32,
    /// How dark the edges get (0..=1).
    pub(crate) vignette: f32,
    /// Softening blur radius (fraction of screen height), stronger toward the
    /// edges — only during the kick-in: it grows with the ramp up and is gone
    /// again by the end of the ramp down.
    pub(crate) blur: f32,
    /// How much the dark edges share in the kick-in, as a fraction of the
    /// kick's peak (0.5 = the edges peak at half what everything else does).
    pub(crate) vignette_kick_scale: f32,
    /// Warm, reddish flush (0..=1).
    pub(crate) flush: f32,
    /// The strong spell right after it switches on.
    pub(crate) kick: KickSettings,
}

impl Default for DrunkSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            fade_secs: 3.0,
            sway_roll_deg: 0.8,
            sway_drift: 0.0085,
            sway_speed: 0.6,
            double_offset: 0.001,
            double_mix: 0.35,
            double_speed: 0.5,
            double_offset_kick: 0.048,
            double_mix_kick: 0.74,
            vignette: 0.65,
            blur: 0.0067,
            vignette_kick_scale: 1.0,
            flush: 1.0,
            kick: KickSettings::default(),
        }
    }
}

/// Whether the local player has the `Zombies` Liquid Courage perk right now
/// (set by `zombies_hud::sync_owned_perks`).
#[derive(Resource, Default)]
pub(crate) struct LiquidCouragePerk(pub(crate) bool);

/// How far the effect is faded in, 0..=1.
#[derive(Resource, Default)]
struct DrunkLevel(f32);

/// The drunk effect's kick-in (`pub(crate)` for the debug panel's replay).
#[derive(Resource, Default)]
pub(crate) struct DrunkKick(pub(crate) KickState);

pub(crate) use uniform::DrunkUniform;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct.
#[allow(dead_code)]
mod uniform {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(Component, Default, Clone, Copy, ShaderType)]
    pub(crate) struct DrunkUniform {
        pub(crate) strength: f32,
        pub(crate) time: f32,
        pub(crate) sway_roll: f32,
        pub(crate) sway_drift: f32,
        pub(crate) sway_speed: f32,
        /// Final double-vision separation / ghost strength (fade and kick-in
        /// already applied).
        pub(crate) double_offset: f32,
        pub(crate) double_mix: f32,
        pub(crate) double_speed: f32,
        /// Final edge darkening (fade and its own kick share already in).
        pub(crate) vignette: f32,
        /// Final blur radius (only non-zero during the kick-in).
        pub(crate) blur: f32,
        pub(crate) flush: f32,
    }
}

impl SyncComponent<RenderApp> for DrunkUniform {
    // (Its uniform index and pipeline go with it once it stops being
    // extracted.)
    type Target = (Self, DynamicUniformIndex<Self>, super::fullscreen_pass::ViewPassPipeline<Self>);
}

impl ExtractComponent<RenderApp> for DrunkUniform {
    type QueryData = &'static DrunkUniform;
    type QueryFilter = ();
    type Out = DrunkUniform;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        (item.strength > 0.0).then_some(*item)
    }
}

impl super::fullscreen_pass::FullscreenPass for DrunkUniform {
    const LABEL: &'static str = "drunk";
    const SHADER: &'static str = SHADER_ASSET_PATH;
}

/// Must be added after `ShroomPlugin` (its node is chained after the shroom
/// one).
pub(crate) struct DrunkPlugin;

impl Plugin for DrunkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DrunkSettings>()
            .init_resource::<DrunkLevel>()
            .init_resource::<DrunkKick>()
            .init_resource::<LiquidCouragePerk>()
            .add_systems(Update, sync_drunk);

        super::fullscreen_pass::add::<DrunkUniform, _>(app, super::fullscreen_pass::run::<super::shroom::ShroomUniform>);
    }
}

/// Fades [`DrunkLevel`] toward on / off and copies the settings onto the
/// view-model camera's [`DrunkUniform`] (added the first time a camera shows
/// up — the cameras are respawned each game).
fn sync_drunk(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<DrunkSettings>,
    perk: Res<LiquidCouragePerk>,
    mut level: ResMut<DrunkLevel>,
    mut kick: ResMut<DrunkKick>,
    new_cams: Query<Entity, (With<ViewModelCamera>, Without<DrunkUniform>)>,
    mut uniforms: Query<&mut DrunkUniform, With<ViewModelCamera>>,
) {
    for cam in &new_cams {
        commands.entity(cam).insert(DrunkUniform::default());
    }

    let target = if settings.enabled || perk.0 { 1.0 } else { 0.0 };
    level.0 = if settings.fade_secs <= 0.0 {
        target
    } else {
        let step = time.delta_secs() / settings.fade_secs;
        level.0 + (target - level.0).clamp(-step, step)
    };
    // Eased fade, times the kick-in's boost (1 once it's settled).
    let boost = kick.0.update(target > 0.0, time.delta_secs(), &settings.kick);
    let fade = level.0 * level.0 * (3.0 - 2.0 * level.0);
    let strength = fade * settings.kick.multiplier(boost);
    // The dark edges only take a share of the kick, and the blur is there
    // for the kick alone.
    let edge_peak = (settings.kick.peak * settings.vignette_kick_scale).max(1.0);
    let vignette = settings.vignette * fade * (1.0 + (edge_peak - 1.0) * boost);
    let blur = settings.blur * fade * boost;
    // Double vision has its own kick-in peak: it climbs to the kick values
    // and back down to the normal ones.
    let double_offset = fade * settings.double_offset.lerp(settings.double_offset_kick, boost);
    let double_mix = fade * settings.double_mix.lerp(settings.double_mix_kick, boost);

    let s = &*settings;
    for mut u in &mut uniforms {
        *u = DrunkUniform {
            strength,
            time: time.elapsed_secs_wrapped(),
            sway_roll: s.sway_roll_deg.to_radians(),
            sway_drift: s.sway_drift,
            sway_speed: s.sway_speed,
            double_offset,
            double_mix,
            double_speed: s.double_speed,
            vignette,
            blur,
            flush: s.flush,
        };
    }
}
