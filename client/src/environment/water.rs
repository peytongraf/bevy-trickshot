//! The ocean plane ringing `shipment.glb`'s yard.

use bevy::math::Affine2;
use bevy::prelude::*;

use crate::util::srgb_parts;

/// Marker on the always-spawned ocean plane ringing `shipment.glb`'s yard
/// just below its ground level — the MW3 Shipment cargo-ship setting this
/// map's remaking. Shown only for `Shipment` ([`sync_shipment_only_visibility`]);
/// `basic_map.glb` has no nautical setting to speak of. No collider — it
/// sits outside the yard's walls, so a player can never actually reach it.
#[derive(Component)]
pub(crate) struct WaterPlane;

/// How far below Shipment's ground level (`y = 0`) [`WaterPlane`] sits by
/// default — just enough to read as "the deck's edge drops off into the
/// sea" without a visible gap between the two. Live-tunable via
/// [`WaterSettings`]'s debug-panel "Water" section.
pub(crate) const WATER_LEVEL_DROP: f32 = 3.0;

/// [`WaterPlane`]'s material handle, so [`apply_water_settings`] /
/// [`scroll_water_normal`] can look it up in `Assets<StandardMaterial>` and
/// mutate it — a plain `Query` can't mutate a shared material asset, only
/// the handle pointing at one.
#[derive(Component, Clone)]
pub(crate) struct WaterMaterial(pub(crate) Handle<StandardMaterial>);

/// How many `water_normal.png` tiles span [`WaterPlane`]'s full width by
/// default — picked by eye so ripples read as roughly wave-sized rather
/// than either a single stretched blur or distractingly tiny repetition.
pub(crate) const WATER_NORMAL_TILING: f32 = 60.0;

/// Metres/second-equivalent [`WaterPlane`]'s normal map drifts by default,
/// in UV space (a fraction of one tile per second along each axis) — enough
/// to read as gently moving water without any actual wave simulation.
pub(crate) const WATER_SCROLL_SPEED: Vec2 = Vec2::new(-0.022, 0.007);

/// Live-tunable "Water" debug-panel section — see [`apply_water_settings`]
/// (level/tint/roughness/reflectance) and [`scroll_water_normal`] (ripple
/// tiling/scroll speed) for where each field actually takes effect.
#[derive(Resource)]
pub(crate) struct WaterSettings {
    pub(crate) level_drop: f32,
    pub(crate) tint: [f32; 3],
    pub(crate) alpha: f32,
    pub(crate) roughness: f32,
    pub(crate) reflectance: f32,
    pub(crate) normal_tiling: f32,
    pub(crate) scroll_speed: Vec2,
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            level_drop: WATER_LEVEL_DROP,
            // Placeholder dark blue — "Copy water settings to console" (the
            // debug panel's "Water" section) prints the exact tint you've
            // actually dialled in, once you have it, to replace this.
            tint: srgb_parts(Color::srgb(0.01, 0.03, 0.09)),
            alpha: 0.98,
            roughness: 0.13,
            reflectance: 0.53,
            normal_tiling: WATER_NORMAL_TILING,
            scroll_speed: WATER_SCROLL_SPEED,
        }
    }
}

/// Push [`WaterSettings`] onto [`WaterPlane`]'s `Transform` and material
/// every frame — mirrors `apply_shipment_transform`'s reasoning: cheap, and
/// unconditional so a value tweaked while the plane is hidden still takes
/// effect the instant `sync_shipment_only_visibility` reveals it.
pub(crate) fn apply_water_settings(
    settings: Res<WaterSettings>,
    mut plane: Query<(&mut Transform, &WaterMaterial), With<WaterPlane>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok((mut transform, water)) = plane.single_mut() else {
        return;
    };
    transform.translation.y = -settings.level_drop;
    let Some(material) = materials.get_mut(&water.0) else {
        return;
    };
    let [r, g, b] = settings.tint;
    material.base_color = Color::srgba(r, g, b, settings.alpha);
    material.perceptual_roughness = settings.roughness;
    material.reflectance = settings.reflectance;
}

/// Animates [`WaterPlane`]'s ripples by scrolling its normal map over time —
/// cosmetic only, so it runs unconditionally rather than being gated on the
/// plane's current visibility (the cost of updating one `Affine2` is trivial
/// either way, and this keeps the ripples already in motion the instant the
/// plane becomes visible instead of starting from a frozen frame).
pub(crate) fn scroll_water_normal(
    time: Res<Time>,
    settings: Res<WaterSettings>,
    water: Query<&WaterMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(water) = water.single() else {
        return;
    };
    let Some(material) = materials.get_mut(&water.0) else {
        return;
    };
    let scroll = settings.scroll_speed * time.elapsed_secs();
    material.uv_transform =
        Affine2::from_scale_angle_translation(Vec2::splat(settings.normal_tiling), 0.0, scroll);
}
