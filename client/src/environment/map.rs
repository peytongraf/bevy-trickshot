//! The current map's collision blockout + optional visual overlay, and the
//! shared `CurrentMap` state everything else (lighting, water, rain, spawns)
//! reads to know which map is active.

use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::scene::SceneInstanceReady;
use bevy_rapier3d::prelude::*;

use super::lighting::{ContainerBulbLight, ContainerFluoroLight, ShipmentSpotLight};
use super::water::WaterPlane;

/// Placement of `basic_map.glb` (position / yaw / uniform scale), live-tweakable
/// from the debug panel's "Map" section and pushed onto the loaded scene by
/// `apply_map_transform`. Rotation is yaw-only (about Y) — the collider
/// `sync_map_model` generates from the mesh moves with this transform too, but
/// `apply_gravity`'s ground raycast assumes "up" stays world-up, which a
/// pitch/roll tilt would break.
#[derive(Resource)]
pub(crate) struct MapSettings {
    pub(crate) position: Vec3,
    pub(crate) rotation_deg: f32,
    pub(crate) scale: f32,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            position: Vec3::new(16.0, 0.0, 8.0),
            rotation_deg: 0.0,
            scale: 0.65,
        }
    }
}

/// Marker on every entity [`sync_map_model`] spawns for the current map
/// ([`MapModel`] and, if there is one, [`MapVisualModel`]) — lets
/// [`apply_map_transform`] / [`apply_shipment_transform`] move both with one
/// query instead of naming each marker (and instead of `Or<(With<A>,
/// With<B>)>`, which drags clippy's `type_complexity` lint in for no benefit
/// here).
#[derive(Component)]
pub(crate) struct MapGeometry;

/// Marker on the spawned collision-blockout scene root (`basic_map.glb`, or
/// `shipment.glb` for `Shipment`) — the mesh `sync_map_model`'s
/// `AsyncSceneCollider` actually generates `apply_gravity` /
/// `resolve_wall_collisions`'s colliders from. Stays visible until
/// [`reveal_map_visual`] hides it, once the matching [`MapVisualModel`] (if
/// any) has actually finished loading — not just because one was requested,
/// so a slow or failed visual load leaves the player looking at the
/// (correctly-colliding, if ugly) blockout instead of nothing at all. Its
/// colliders stay active regardless of `Visibility` either way.
#[derive(Component)]
pub(crate) struct MapModel;

/// Marker on the optional, nicer-looking scene spawned *in addition to* the
/// matching [`MapModel`] for maps that have one — so far just `Shipment`'s
/// `shipment_visual.glb`, sitting on top of `shipment.glb`'s collision
/// blockout (hidden once this finishes loading — see [`reveal_map_visual`]).
/// Purely visual: no collider, and its `Transform` is kept identical to
/// `MapModel`'s ([`apply_map_transform`] / [`apply_shipment_transform`] move
/// both, via [`MapGeometry`]), so it lines up with the blockout's geometry —
/// and therefore with collision — as long as whoever builds it keeps the two
/// aligned in whatever tool exported them.
#[derive(Component)]
pub(crate) struct MapVisualModel;

/// Marker on the always-spawned procedural asphalt ground plane — spawned
/// hidden with its collider disabled (see `setup_world`) since every current
/// map ships its own ground mesh now. Kept as a ready-made fallback floor for
/// a future map that doesn't.
#[derive(Component)]
pub(crate) struct ProceduralGround;

/// Whether the current [`CurrentMap`]'s scene(s) have finished loading —
/// `game_start`'s "waiting for party" screen watches this (alongside every
/// other lobby member's own report) before letting a match actually start,
/// so nobody drops into a still-loading map. Reset by [`sync_map_model`]
/// whenever the map changes; set by [`mark_model_ready`] /
/// [`mark_visual_ready`] once the matching scene's `SceneInstanceReady`
/// fires.
#[derive(Resource, Default)]
pub(crate) struct MapLoadState {
    model_ready: bool,
    visual_ready: bool,
}

/// Whether `state` reflects a fully-loaded `map` — `model_ready`, plus
/// `visual_ready` too if `map` actually has a [`MapVisualModel`] (see
/// [`map_visual_path`]).
pub(crate) fn map_ready(state: &MapLoadState, map: shared::MapId) -> bool {
    state.model_ready && (map_visual_path(map).is_none() || state.visual_ready)
}

/// Push `MapSettings` onto the loaded `basic_map.glb` scene every frame it's
/// the selected map — cheap (one `Transform` write), and unconditional so a
/// freshly re-spawned model (see `sync_map_model`, after switching away from
/// and back to this map) always picks the placement back up, rather than only
/// on the next tweak to `MapSettings` itself.
pub(crate) fn apply_map_transform(
    current: Res<CurrentMap>,
    map: Res<MapSettings>,
    mut models: Query<&mut Transform, With<MapGeometry>>,
) {
    if current.0 != shared::MapId::BasicMap {
        return;
    }
    for mut transform in &mut models {
        transform.translation = map.position;
        transform.rotation = Quat::from_rotation_y(map.rotation_deg.to_radians());
        transform.scale = Vec3::splat(map.scale);
    }
}

/// The map the current lobby (or, in solo Practice, the default) is playing
/// on — drives which `SceneRoot` [`sync_map_model`] keeps spawned under
/// [`MapModel`] (and its auto-generated `Collider`s, which is all the ground/
/// wall collision in [`apply_gravity`] / [`resolve_wall_collisions`] needs).
/// Set once on [`AppState::InGame`] entry by `lobby_ui::sync_current_map`;
/// defaults to `BasicMap` before that ever runs.
#[derive(Resource, Default, PartialEq)]
pub(crate) struct CurrentMap(pub shared::MapId);

/// Live-tunable uniform scale for `shipment.glb` — see
/// `shared::map::SHIPMENT_SCALE`, which this defaults to and which the
/// server's own spawn/respawn placement always uses. Adjustable from the
/// debug panel's "Shipment map" section so you can dial in a different number
/// by eye; [`apply_shipment_transform`] reads the live value here, and
/// because the model's own `Collider`s are children of the `Transform` this
/// writes, `apply_gravity` / `resolve_wall_collisions` automatically collide
/// against whatever scale is currently showing — nothing else to keep in
/// sync. Once you've settled on a number, update the `shared` constant to
/// match — otherwise a fresh session (or the server's own placement) falls
/// back to the old default.
#[derive(Resource)]
pub(crate) struct ShipmentSettings {
    pub scale: f32,
}

impl Default for ShipmentSettings {
    fn default() -> Self {
        Self {
            scale: shared::map::SHIPMENT_SCALE,
        }
    }
}

/// Push [`ShipmentSettings`] onto both the loaded `shipment.glb`
/// collision blockout and, if it's currently spawned, its
/// `shipment_visual.glb` overlay — every frame it's the selected map, so the
/// two stay at the same scale as each other (mirrors
/// [`apply_map_transform`]'s reasoning: unconditional, not gated on a change
/// flag, so freshly re-spawned models always pick the current scale back
/// up).
pub(crate) fn apply_shipment_transform(
    current: Res<CurrentMap>,
    settings: Res<ShipmentSettings>,
    mut models: Query<&mut Transform, With<MapGeometry>>,
) {
    if !current.0.is_shipment() {
        return;
    }
    for mut transform in &mut models {
        transform.scale = Vec3::splat(settings.scale);
    }
}

/// Collider shape used for every mesh node in a map's `.glb` — an exact
/// triangle mesh, so collision always matches the visual geometry exactly,
/// including concave/open shapes: a container built from a few wall meshes
/// and a ceiling joined together, missing a wall or two on purpose so a
/// player can walk in, stays walk-in-able, rather than getting filled solid.
/// (`ComputedColliderShape::ConvexHull` — the previous default for anything
/// other than a node named `"Ground"` — can only ever produce a *convex*
/// shape, so it silently filled in any such opening; there's no Blender-side
/// fix for that, it's a property of convex hulls in general.) Every map's
/// collision blockout is static — nothing here moves at runtime beyond
/// `MapSettings`/`ShipmentSettings`'s whole-scene placement — so trimesh's
/// usual downside (expensive to move) doesn't apply, and slopes/ramps built
/// into a mesh (a node named `"Ground"`, or any other) produce real walkable
/// geometry for free, the same way.
pub(crate) fn map_collider_shape() -> ComputedColliderShape {
    ComputedColliderShape::TriMesh(TriMeshFlags::MERGE_DUPLICATE_VERTICES)
}

/// The optional nicer-looking scene shown *instead of* `map`'s collision
/// blockout — `None` means the blockout is what's on screen (`basic_map.glb`
/// today; every map starts out this way before it has real art). Add an
/// entry here once a map gets a `MapVisualModel` of its own.
pub(crate) fn map_visual_path(map: shared::MapId) -> Option<&'static str> {
    match map {
        shared::MapId::BasicMap => None,
        shared::MapId::Shipment | shared::MapId::ShipmentDay => Some("models/shipment_visual.glb"),
    }
}

/// Keep exactly one `MapModel` (plus, if [`map_visual_path`] has one, one
/// `MapVisualModel`) scene spawned, matching [`CurrentMap`] — swaps them out
/// (despawn old, spawn new) whenever the selection changes. Runs
/// unconditionally (not gated on `AppState`) so the world behind the menu/
/// lobby UI is already showing the right map by the time a game starts, the
/// same "always loaded" behaviour `setup_world` used to provide for the one
/// map that used to exist.
///
/// The `AsyncSceneCollider` on `MapModel` is the whole reason a new map
/// needs zero hand-authored collision data: once the scene finishes loading,
/// rapier walks every mesh node and builds a real [`map_collider_shape`]
/// `Collider` from its actual geometry. Export a `.glb`, point a `MapId` at
/// it here, and `apply_gravity` / `resolve_wall_collisions` (neither of
/// which know or care which map is loaded) just work — a map plays
/// correctly on nothing but its blockout, so
/// a `MapVisualModel` is an optional, purely cosmetic layer on top: it never
/// gets a collider, and `MapModel` is only hidden (rather than despawned)
/// once [`reveal_map_visual`] confirms the replacement actually made it on
/// screen, so collision keeps coming from the same blockout either way.
pub(crate) fn sync_map_model(
    current: Res<CurrentMap>,
    asset_server: Res<AssetServer>,
    existing: Query<Entity, With<MapGeometry>>,
    mut load_state: ResMut<MapLoadState>,
    mut commands: Commands,
) {
    if !current.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }
    *load_state = MapLoadState::default();
    let path = match current.0 {
        shared::MapId::BasicMap => "models/basic_map.glb",
        shared::MapId::Shipment | shared::MapId::ShipmentDay => "models/shipment.glb",
    };
    commands
        .spawn((
            MapModel,
            MapGeometry,
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path))),
            AsyncSceneCollider {
                shape: Some(map_collider_shape()),
                named_shapes: default(),
            },
        ))
        .observe(mark_model_ready);
    if let Some(visual_path) = map_visual_path(current.0) {
        commands
            .spawn((
                MapVisualModel,
                MapGeometry,
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(visual_path))),
            ))
            .observe(reveal_map_visual)
            .observe(mark_visual_ready);
    }
}

/// Fires once [`MapModel`]'s `SceneRoot` finishes spawning — sets
/// [`MapLoadState::model_ready`].
fn mark_model_ready(
    trigger: Trigger<SceneInstanceReady>,
    models: Query<(), With<MapModel>>,
    mut state: ResMut<MapLoadState>,
) {
    if models.contains(trigger.target()) {
        state.model_ready = true;
    }
}

/// Fires once [`MapVisualModel`]'s `SceneRoot` finishes spawning — sets
/// [`MapLoadState::visual_ready`]. Separate from [`reveal_map_visual`] (which
/// also fires on this same event) since that one only cares about maps that
/// *have* a visual override; this one is read via [`map_ready`], which
/// already accounts for maps that don't.
fn mark_visual_ready(
    trigger: Trigger<SceneInstanceReady>,
    visuals: Query<(), With<MapVisualModel>>,
    mut state: ResMut<MapLoadState>,
) {
    if visuals.contains(trigger.target()) {
        state.visual_ready = true;
    }
}

/// Fires once a [`MapVisualModel`]'s `SceneRoot` finishes spawning — only
/// *then* hides the matching [`MapModel`] blockout, rather than hiding it
/// eagerly the moment a visual override is merely requested. `shipment_visual
/// .glb` is a much heavier asset than the blockout it replaces (a full
/// container pack's worth of meshes and textures vs. a handful of untextured
/// boxes), so if it's slow to load, still loading, or — for whatever
/// reason — never resolves, this keeps the player looking at the (correctly
/// colliding, if plain) blockout instead of bare sky.
pub(crate) fn reveal_map_visual(
    trigger: Trigger<SceneInstanceReady>,
    visuals: Query<(), With<MapVisualModel>>,
    mut blockout: Query<&mut Visibility, With<MapModel>>,
) {
    if !visuals.contains(trigger.target()) {
        return;
    }
    if let Ok(mut vis) = blockout.single_mut() {
        *vis = Visibility::Hidden;
    }
}

/// Toggles [`WaterPlane`] and [`ShipmentSpotLight`] (plus the container
/// fluoro/bulb lights) visible only for maps with a nautical setting to
/// speak of — `Shipment`'s MW3-style cargo ship; hidden for `basic_map.glb`.
///
/// [`ProceduralGround`] used to get the same map-dependent treatment here
/// (shown for whichever map didn't yet have its own ground mesh), but every
/// current map ships its own now, so `setup_world` just spawns it already
/// hidden with its collider disabled and nothing here needs to touch it —
/// see that spawn's doc comment.
///
/// [`WaterPlane`] and [`ShipmentSpotLight`] are spawned once in `setup_world`
/// and never respawned, unlike [`MapModel`], so there's no freshly-respawned
/// entity to recover state for on other frames.
pub(crate) fn sync_shipment_only_visibility(
    current: Res<CurrentMap>,
    water: Query<Entity, With<WaterPlane>>,
    light: Query<Entity, With<ShipmentSpotLight>>,
    fluoro: Query<Entity, With<ContainerFluoroLight>>,
    bulbs: Query<Entity, With<ContainerBulbLight>>,
    mut commands: Commands,
) {
    if !current.is_changed() {
        return;
    }
    let visible_if = |shown: bool| {
        if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    };
    // The ocean shows on both Shipment variants; the floodlights and
    // container fixtures are night-only — Shipment Day has full sun, and
    // their beams/glows would just read as stray bright spots in daylight.
    let water_visibility = visible_if(current.0.is_shipment());
    let night_visibility = visible_if(current.0 == shared::MapId::Shipment);
    if let Ok(entity) = water.single() {
        commands.entity(entity).insert(water_visibility);
    }
    for entity in &light {
        commands.entity(entity).insert(night_visibility);
    }
    if let Ok(entity) = fluoro.single() {
        commands.entity(entity).insert(night_visibility);
    }
    for entity in &bulbs {
        commands.entity(entity).insert(night_visibility);
    }
}

/// Build a seamless tiling ground texture: layered value noise ramped between a
/// dark and a light tarmac tone, with fine grain plus the odd lighter aggregate
/// fleck, so the ground reads as weathered asphalt instead of a flat grid.
pub(crate) fn build_ground_texture() -> Image {
    const N: u32 = 512;

    // Cheap integer-lattice hash → [0, 1).
    fn hash(x: i32, y: i32) -> f32 {
        let mut h = (x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263)) as u32;
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        h ^= h >> 16;
        h as f32 / u32::MAX as f32
    }

    fn smooth(t: f32) -> f32 {
        t * t * (3.0 - 2.0 * t)
    }

    // Value noise on a lattice that wraps every `period` cells, so a tile whose
    // width spans a whole number of periods is seamless.
    fn value_noise(x: f32, y: f32, period: i32) -> f32 {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let fx = smooth(x - x0 as f32);
        let fy = smooth(y - y0 as f32);
        let w = |v: i32| v.rem_euclid(period);
        let a = hash(w(x0), w(y0));
        let b = hash(w(x0 + 1), w(y0));
        let c = hash(w(x0), w(y0 + 1));
        let d = hash(w(x0 + 1), w(y0 + 1));
        let top = a + (b - a) * fx;
        let bot = c + (d - c) * fx;
        top + (bot - top) * fy
    }

    // fBm whose octave frequencies all divide the tile, so the sum tiles too.
    fn fbm(u: f32, v: f32) -> f32 {
        let (mut sum, mut amp, mut freq) = (0.0, 0.5, 4.0);
        for _ in 0..5 {
            sum += value_noise(u * freq, v * freq, freq as i32) * amp;
            freq *= 2.0;
            amp *= 0.5;
        }
        sum
    }

    let lo = [24.0f32, 25.0, 28.0]; // wet/shadowed tarmac
    let hi = [70.0f32, 71.0, 75.0]; // sun-bleached tarmac
    let fleck = [118.0f32, 116.0, 120.0]; // exposed aggregate

    let mut data = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let u = x as f32 / N as f32;
            let v = y as f32 / N as f32;

            let n = fbm(u, v).clamp(0.0, 1.0);
            let speck = value_noise(u * 96.0, v * 96.0, 96);
            let fleck_amt = if speck > 0.86 {
                (speck - 0.86) / 0.14
            } else {
                0.0
            };

            let mut rgb = [0u8; 3];
            for c in 0..3 {
                let base = lo[c] + (hi[c] - lo[c]) * n;
                rgb[c] = (base * (1.0 - fleck_amt) + fleck[c] * fleck_amt).round() as u8;
            }
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..default()
    });
    image
}
