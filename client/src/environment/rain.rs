//! Live-tunable rain for `shipment.glb` — a fixed pool of world-space streak
//! entities recycled around the player, rather than a camera-locked overlay.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;

use crate::util::{rand01, rand_roll, srgb_parts};
use crate::Player;

use super::map::CurrentMap;

/// Marker + pool index for one rain streak. A fixed pool of
/// [`RAIN_MAX_DROPS`] entities is spawned once ([`setup_rain`]); only the
/// first `RainSettings::count` of them are simulated/visible at a time
/// ([`update_rain`]) — raising the count in the debug panel just reveals
/// more of an already-spawned pool rather than spawning anything new, so
/// there's no per-frame spawn/despawn churn even while live-tuning it.
#[derive(Component)]
pub(crate) struct RainDrop(usize);

/// How far below the player (m) a [`RainDrop`] falls before [`update_rain`]
/// recycles it back up to `RainSettings::spawn_height` — generous enough
/// that it's always well under the player's feet regardless of what they're
/// standing on (ground level, a container roof, ...), since this isn't
/// checked against actual map geometry, just the player's own height.
pub(crate) const RAIN_RESET_DEPTH: f32 = 6.0;

/// Size of [`update_rain`]'s fixed drop pool — an upper bound on
/// `RainSettings::count`'s debug-panel slider, not a number you're meant to
/// reach for "light" rain.
pub(crate) const RAIN_MAX_DROPS: usize = 1200;

/// Live-tunable rain for `shipment.glb` — debug panel's "Rain" section.
/// Real world-space particles (thin falling streaks with their own
/// downward + wind velocity — [`update_rain`]), not a camera-locked 2D
/// overlay: running through them naturally looks and feels right for the
/// same reason anything else in the world does when you move past it, and
/// standing still naturally looks like standing still, with no special
/// "is the player moving" logic needed either way. Starts as light rain —
/// a fairly sparse, slow, short-streaked look; see field docs for what to
/// turn up for a heavier storm.
#[derive(Resource)]
pub(crate) struct RainSettings {
    pub(crate) enabled: bool,
    /// How many of the `RAIN_MAX_DROPS` pooled streaks are active. Density
    /// also depends on `radius` — the same count spread over a smaller
    /// radius reads as heavier rain.
    pub(crate) count: usize,
    /// Radius (m), centred on the player's XZ position, streaks are kept
    /// within — one falls straight through and recycles once it drifts
    /// outside it (`update_rain`).
    pub(crate) radius: f32,
    /// Height (m) above the player a recycled streak starts falling from.
    pub(crate) spawn_height: f32,
    /// Downward fall speed (m/s) — real light rain is roughly this order of
    /// magnitude; a storm would be noticeably faster.
    pub(crate) fall_speed: f32,
    /// Horizontal drift (m/s, world X/Z) — gives the rain a wind-blown tilt
    /// instead of falling perfectly straight down. Small by default.
    pub(crate) wind: Vec2,
    pub(crate) streak_length: f32,
    pub(crate) streak_radius: f32,
    pub(crate) color: [f32; 3],
    pub(crate) opacity: f32,
}

impl Default for RainSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            count: 500,
            radius: 21.0,
            spawn_height: 16.0,
            fall_speed: 15.0,
            wind: Vec2::new(1.5, 2.0),
            streak_length: 0.15,
            streak_radius: 0.005,
            // r119 g125 b131 (0-255) — a pale grey-blue.
            color: srgb_parts(Color::srgb(119.0 / 255.0, 125.0 / 255.0, 131.0 / 255.0)),
            opacity: 0.45,
        }
    }
}

/// Shared mesh + material for every [`RainDrop`] — all identical (a thin
/// cylinder), so unlike `Smoke` (which fades independently per particle and
/// so needs its own material each) there's nothing to gain from separate
/// assets per streak, only draw calls to lose.
#[derive(Resource)]
pub(crate) struct RainAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// Spawns [`RAIN_MAX_DROPS`] pooled streak entities at the world origin —
/// where exactly doesn't matter, since [`update_rain`]'s "fallen below the
/// player" check immediately recycles every one of them to a proper
/// position around the player on its very first run.
pub(crate) fn setup_rain(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    rain: Res<RainSettings>,
) {
    let [r, g, b] = rain.color;
    let assets = RainAssets {
        mesh: meshes.add(Cylinder::new(rain.streak_radius, rain.streak_length)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgba(r, g, b, rain.opacity),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        }),
    };
    for i in 0..RAIN_MAX_DROPS {
        commands.spawn((
            RainDrop(i),
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.material.clone()),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
            // A thousand-odd streaks constantly swept by the two shadow-
            // casting shipment floodlights would be a lot of flickering
            // shadow noise for no visual benefit — they're thin enough that
            // "rain casts a shadow" was never going to read as anything but
            // noise anyway.
            NotShadowCaster,
        ));
    }
    commands.insert_resource(assets);
}

/// Pushes `streak_length` / `streak_radius` / `color` / `opacity` onto
/// [`RainAssets`]'s shared mesh + material whenever [`RainSettings`]
/// changes — every [`RainDrop`] shares the same `Handle<Mesh>` and
/// `Handle<StandardMaterial>` (see `setup_rain`), so replacing the asset
/// each of those two handles points at updates every streak in the pool at
/// once, the same live-tuning behaviour every other panel section has.
pub(crate) fn apply_rain_assets(
    rain: Res<RainSettings>,
    assets: Res<RainAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !rain.is_changed() {
        return;
    }
    if let Some(mesh) = meshes.get_mut(&assets.mesh) {
        *mesh = Cylinder::new(rain.streak_radius, rain.streak_length).into();
    }
    if let Some(material) = materials.get_mut(&assets.material) {
        let [r, g, b] = rain.color;
        material.base_color = Color::srgba(r, g, b, rain.opacity);
    }
}

/// Falls every active [`RainDrop`] (`RainSettings::count` of the
/// [`RAIN_MAX_DROPS`]-entity pool) at `fall_speed` + `wind` each frame,
/// recycling one back up to `spawn_height` once it's `RAIN_RESET_DEPTH`
/// below the player or has drifted past `radius` away from them
/// horizontally — and keeps every streak mesh oriented along its own fall
/// direction, so it reads as a real 3D streak from any angle rather than a
/// flat billboard. Hides the whole pool (and skips simulating it) unless
/// `Shipment` is selected and `RainSettings::enabled` is on. Runs alongside
/// `apply_shipment_lights` and friends, so it only ever executes during
/// `AppState::InGame` (this tuple's own `run_if`) — the `Player` this reads
/// always exists by then.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_rain(
    time: Res<Time>,
    current: Res<CurrentMap>,
    settings: Res<RainSettings>,
    player: Single<&Transform, (With<Player>, Without<RainDrop>)>,
    mut drops: Query<(&RainDrop, &mut Transform, &mut Visibility), Without<Player>>,
    mut seed: Local<u32>,
) {
    let active = settings.enabled && current.0 == shared::MapId::Shipment;
    let velocity = Vec3::new(settings.wind.x, -settings.fall_speed, settings.wind.y);
    let rotation = Quat::from_rotation_arc(Vec3::Y, velocity.normalize_or_zero());
    let dt = time.delta_secs();
    let player_pos = player.translation;

    for (drop, mut transform, mut vis) in &mut drops {
        if !active || drop.0 >= settings.count {
            *vis = Visibility::Hidden;
            continue;
        }
        *vis = Visibility::Inherited;
        transform.rotation = rotation;
        transform.translation += velocity * dt;

        let dx = transform.translation.x - player_pos.x;
        let dz = transform.translation.z - player_pos.z;
        let fell_too_far = transform.translation.y < player_pos.y - RAIN_RESET_DEPTH;
        let drifted_too_far = (dx * dx + dz * dz).sqrt() > settings.radius;
        if fell_too_far || drifted_too_far {
            *seed = seed.wrapping_add(1);
            let s = seed.wrapping_add((drop.0 as u32).wrapping_mul(2_654_435_761));
            let angle = rand_roll(s);
            // sqrt so drops land uniformly over the disc's *area*, not
            // bunched up near its centre.
            let r = rain_radius_sample(s, settings.radius);
            transform.translation = Vec3::new(
                player_pos.x + angle.cos() * r,
                player_pos.y + settings.spawn_height,
                player_pos.z + angle.sin() * r,
            );
        }
    }
}

/// A uniform-over-the-disc-area radius sample in `[0, radius]` — see the
/// `sqrt` note at its call site.
pub(crate) fn rain_radius_sample(seed: u32, radius: f32) -> f32 {
    rand01(seed.wrapping_add(1)).sqrt() * radius
}
