//! `shipment.glb`'s fixture lighting: the mast/crane floodlights, one
//! container's fluorescent tube, and another's pair of bare bulbs.

use bevy::prelude::*;

use crate::settings::Settings;
use crate::util::{color_from_parts, srgb_parts};

/// Marker for one of `shipment.glb`'s debug-controllable mast/crane-mounted
/// floodlights, lighting the yard the way the real MW3 Shipment's deck
/// lights do (their light spreads out from the fixture in the cone/
/// "triangle" shape any spotlight does, especially visible against this
/// map's heavy fog — see [`ShipmentSceneTuning`]). `.0` indexes into
/// [`ShipmentLightSettings::lights`] — `0` for the first light, `1` for the
/// second, and so on if a third ever gets added (one more array entry, one
/// more `commands.spawn` in `setup_world` with the next index — no new
/// marker type needed). Shown only for `Shipment` — folded into
/// [`sync_shipment_only_visibility`] alongside [`ProceduralGround`] and
/// [`WaterPlane`]'s toggles.
#[derive(Component)]
pub(crate) struct ShipmentSpotLight(pub(crate) usize);

/// One floodlight's position/aim/cone — see [`ShipmentLightSettings`] (which
/// holds one of these per [`ShipmentSpotLight`]) for how it's used.
/// `yaw_deg` / `pitch_deg` use this file's usual
/// `Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)` convention (see e.g.
/// `look_around`) — a `SpotLight` shines along its transform's local -Z
/// (forward), so together with `position` this fully determines where it
/// points. `inner_angle_deg` / `outer_angle_deg` are the cone's half-angles
/// (`SpotLight`'s own convention, matching real fixtures' "beam angle"):
/// equal values give a hard-edged cone, a gap between them a soft penumbra.
/// `glow_intensity` is separate from `intensity` — see [`ShipmentLightGlow`]
/// — since the two read very differently: the flood beam only needs to light
/// the yard, the glow only needs to look bright standing right under it.
#[derive(Clone)]
pub(crate) struct ShipmentLight {
    pub(crate) position: Vec3,
    pub(crate) yaw_deg: f32,
    pub(crate) pitch_deg: f32,
    pub(crate) color: [f32; 3],
    pub(crate) intensity: f32,
    pub(crate) range: f32,
    pub(crate) inner_angle_deg: f32,
    pub(crate) outer_angle_deg: f32,
    pub(crate) shadows_enabled: bool,
    pub(crate) glow_intensity: f32,
}

/// Live-tunable floodlights for `shipment.glb` — debug panel's "Shipment
/// Lights" section, applied every frame to the matching [`ShipmentSpotLight`]
/// by [`apply_shipment_lights`]. `markers_visible` gates
/// [`ShipmentLightMarker`] (the position/aim gizmos) on top of the debug
/// panel itself being open — off by default, since the gizmos are purely a
/// placement aid, not something to leave on once a light's dialled in.
#[derive(Resource)]
pub(crate) struct ShipmentLightSettings {
    pub(crate) lights: [ShipmentLight; 2],
    pub(crate) markers_visible: bool,
}

impl Default for ShipmentLightSettings {
    fn default() -> Self {
        Self {
            lights: [
                ShipmentLight {
                    position: Vec3::new(16.0, 18.5, 0.08),
                    yaw_deg: 85.0,
                    pitch_deg: -30.0,
                    color: srgb_parts(Color::srgb(1.0, 1.0, 1.0)),
                    // Maxed against the "intensity (lumens)" slider's own
                    // 100,000,000 ceiling (`shipment_light_sliders`) — the
                    // flood beam, not `glow_intensity` (the bulb).
                    intensity: 100_000_000.0,
                    range: 62.0,
                    inner_angle_deg: 89.0,
                    outer_angle_deg: 89.0,
                    shadows_enabled: true,
                    glow_intensity: 5_000_000.0,
                },
                ShipmentLight {
                    position: Vec3::new(-16.0, 18.5, 0.0),
                    yaw_deg: -75.0,
                    pitch_deg: -35.0,
                    color: srgb_parts(Color::srgb(1.0, 1.0, 1.0)),
                    intensity: 100_000_000.0,
                    range: 62.0,
                    inner_angle_deg: 89.0,
                    outer_angle_deg: 89.0,
                    shadows_enabled: true,
                    glow_intensity: 5_000_000.0,
                },
            ],
            markers_visible: false,
        }
    }
}

/// Push each [`ShipmentLight`] in [`ShipmentLightSettings`] onto its matching
/// [`ShipmentSpotLight`] (and that light's [`ShipmentLightGlow`] child) every
/// frame — mirrors `apply_shipment_transform`'s reasoning: cheap, and
/// unconditional so a value tweaked while the lights are hidden still takes
/// effect the instant [`sync_shipment_only_visibility`] reveals them.
pub(crate) fn apply_shipment_lights(
    settings: Res<ShipmentLightSettings>,
    mut lights: Query<(&ShipmentSpotLight, &mut Transform, &mut SpotLight)>,
    mut glows: Query<(
        &ShipmentLightGlow,
        &mut PointLight,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (index, mut transform, mut spot) in &mut lights {
        let Some(cfg) = settings.lights.get(index.0) else {
            continue;
        };
        transform.translation = cfg.position;
        transform.rotation = Quat::from_euler(
            EulerRot::YXZ,
            cfg.yaw_deg.to_radians(),
            cfg.pitch_deg.to_radians(),
            0.0,
        );
        spot.color = color_from_parts(cfg.color);
        spot.intensity = cfg.intensity;
        spot.range = cfg.range;
        spot.inner_angle = cfg.inner_angle_deg.to_radians();
        spot.outer_angle = cfg.outer_angle_deg.to_radians();
        spot.shadows_enabled = cfg.shadows_enabled;
    }
    for (index, mut point, material) in &mut glows {
        let Some(cfg) = settings.lights.get(index.0) else {
            continue;
        };
        let color = color_from_parts(cfg.color);
        point.color = color;
        point.intensity = cfg.glow_intensity;
        if let Some(material) = materials.get_mut(material) {
            // Scaled well past 1.0 (HDR) so `Bloom` actually catches it — a
            // merely `1.0`-bright emissive reads as a flat lit surface, not
            // a light source. `GLOW_EMISSIVE_PER_LUMEN` is picked by eye
            // against `ShipmentLight::glow_intensity`'s slider range, not
            // any physical unit conversion.
            material.emissive = LinearRgba::from(color) * cfg.glow_intensity
                / GLOW_EMISSIVE_PER_LUMEN;
        }
    }
}

/// How many lumens of [`ShipmentLight::glow_intensity`] correspond to one
/// unit of [`ShipmentLightGlow`]'s emissive brightness — see
/// `apply_shipment_lights`. Tuned by eye so the default `glow_intensity`
/// reads as a bright bulb without blowing out to a featureless white blob.
pub(crate) const GLOW_EMISSIVE_PER_LUMEN: f32 = 25_000.0;

/// An always-visible small emissive sphere at a [`ShipmentSpotLight`]'s
/// fixture, paired with a short-range `PointLight` on the same entity — so
/// looking up at one of MW3 Shipment's crane floodlights actually shows a
/// bright light source, not just an invisible cone with nothing visibly
/// casting it, and the platform/crane right around the fixture gets a soft
/// glow the narrow flood beam alone wouldn't reach. `.0` indexes into
/// [`ShipmentLightSettings::lights`], same as [`ShipmentSpotLight`]. Unlike
/// [`ShipmentLightMarker`]'s debug-only gizmo, this is a normal, always-shown
/// part of the scene — being a child of its `ShipmentSpotLight` means
/// `sync_shipment_only_visibility`'s map-based hide of the parent still
/// applies to it via `Inherited` visibility.
#[derive(Component)]
pub(crate) struct ShipmentLightGlow(pub(crate) usize);

/// Radius of a [`ShipmentLightGlow`]'s emissive bulb sphere — small enough to
/// read as a fixture, not a floating ball.
pub(crate) const LIGHT_GLOW_BULB_RADIUS: f32 = 0.35;

/// How far a [`ShipmentLightGlow`]'s companion `PointLight` reaches — short
/// on purpose, just enough to light the fixture's own crane/platform; the
/// yard-scale illumination is entirely the `SpotLight`'s job.
pub(crate) const LIGHT_GLOW_RANGE: f32 = 12.0;

/// Marker for a [`ShipmentSpotLight`]'s debug-only gizmo (a bulb + aim rod,
/// spawned as its children in `setup_world`) — see [`sync_light_marker_visibility`]
/// for why its `Visibility` needs its own active management beyond just
/// following its parent light around. Shared by every light's gizmo, not
/// just one — [`sync_light_marker_visibility`] doesn't need to know how many
/// lights there are, only which entities are "a marker."
#[derive(Component)]
pub(crate) struct ShipmentLightMarker;

/// Radius of a [`ShipmentLightMarker`]'s "bulb" sphere.
pub(crate) const LIGHT_MARKER_BULB_RADIUS: f32 = 0.5;
/// Length and thickness of a [`ShipmentLightMarker`]'s aim-direction rod —
/// sized to be obvious without being huge next to a container-scale map.
pub(crate) const LIGHT_MARKER_ROD_LENGTH: f32 = 4.0;
pub(crate) const LIGHT_MARKER_ROD_THICKNESS: f32 = 0.15;

/// Shows every [`ShipmentLightMarker`] only while the debug panel is open
/// *and* [`ShipmentLightSettings::markers_visible`] is on — a floating
/// bulb-and-rod gizmo has no business appearing in normal play, or even in
/// debug mode once a light's already positioned. Set unconditionally each
/// frame (not gated on a change flag) so toggling either back off actually
/// hides the gizmos again rather than freezing whatever state they were
/// last set to. Being children of their [`ShipmentSpotLight`] (an
/// `Inherited` visibility, which this only ever sets them to or away from)
/// means [`sync_shipment_only_visibility`]'s map-based hide of the parent
/// still applies on top of this — a gizmo needs the debug panel open, its
/// toggle on, *and* `Shipment` selected to actually show.
pub(crate) fn sync_light_marker_visibility(
    settings: Res<Settings>,
    lights: Res<ShipmentLightSettings>,
    mut marker: Query<&mut Visibility, With<ShipmentLightMarker>>,
) {
    let target = if settings.debug_mode && lights.markers_visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut vis in &mut marker {
        *vis = target;
    }
}

/// The container fluorescent-tube fixture's light — see [`FluoroLightSettings`].
#[derive(Component)]
pub(crate) struct ContainerFluoroLight;

/// Debug-only position marker for [`ContainerFluoroLight`] — mirrors
/// [`ShipmentLightMarker`], minus the aim rod (a `PointLight` has no
/// direction to show).
#[derive(Component)]
pub(crate) struct ContainerFluoroLightMarker;

/// Live-tunable `PointLight` standing in for the fluorescent-tube fixture
/// model in one of `shipment_visual.glb`'s containers — debug panel's
/// "Fluorescent Light" section, applied every frame by
/// [`apply_fluoro_light`]. Bevy has no tube/area light, so a single
/// `PointLight` is the stand-in; cool white and a modest range suit lighting
/// one container's interior rather than the yard.
#[derive(Resource)]
pub(crate) struct FluoroLightSettings {
    pub(crate) position: Vec3,
    pub(crate) color: [f32; 3],
    pub(crate) intensity: f32,
    pub(crate) range: f32,
    pub(crate) shadows_enabled: bool,
    /// Shows [`ContainerFluoroLightMarker`] — debug panel only, off by
    /// default (a floating marker sphere has no business in normal play).
    pub(crate) markers_visible: bool,
}

impl Default for FluoroLightSettings {
    fn default() -> Self {
        Self {
            // Dialed in against the container model (see the "Fluorescent
            // Light" debug-panel section).
            position: Vec3::new(-24.5, 2.0, 0.0),
            // Cool white — daylight-ish fluorescent tube colour.
            color: srgb_parts(Color::srgb(0.85, 0.93, 1.0)),
            intensity: 1_500_000.0,
            range: 8.0,
            shadows_enabled: true,
            markers_visible: false,
        }
    }
}

/// Push [`FluoroLightSettings`] onto [`ContainerFluoroLight`] every frame —
/// mirrors `apply_shipment_lights`'s reasoning (cheap, and unconditional so
/// a value tweaked while hidden still takes effect the instant
/// `sync_shipment_only_visibility` reveals it).
pub(crate) fn apply_fluoro_light(
    settings: Res<FluoroLightSettings>,
    mut light: Single<(&mut Transform, &mut PointLight), With<ContainerFluoroLight>>,
) {
    let (transform, point) = &mut *light;
    transform.translation = settings.position;
    point.color = color_from_parts(settings.color);
    point.intensity = settings.intensity;
    point.range = settings.range;
    point.shadows_enabled = settings.shadows_enabled;
}

/// Shows [`ContainerFluoroLightMarker`] only while the debug panel is open
/// *and* [`FluoroLightSettings::markers_visible`] is on — mirrors
/// [`sync_light_marker_visibility`].
pub(crate) fn sync_fluoro_marker_visibility(
    settings: Res<Settings>,
    fluoro: Res<FluoroLightSettings>,
    mut marker: Query<&mut Visibility, With<ContainerFluoroLightMarker>>,
) {
    let target = if settings.debug_mode && fluoro.markers_visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut vis in &mut marker {
        *vis = target;
    }
}

/// One of the two bare-bulb fixture lights — see [`BulbLightSettings`]. `.0`
/// indexes into `BulbLightSettings::positions`, mirroring
/// [`ShipmentSpotLight`].
#[derive(Component)]
pub(crate) struct ContainerBulbLight(pub(crate) usize);

/// Debug-only position marker for a [`ContainerBulbLight`] — mirrors
/// [`ContainerFluoroLightMarker`]; shared by both bulbs' gizmos, same as
/// [`ShipmentLightMarker`] is shared by both crane lights'.
#[derive(Component)]
pub(crate) struct ContainerBulbLightMarker;

/// Live-tunable `PointLight`s standing in for the two light-bulb fixture
/// models in another of `shipment_visual.glb`'s containers — debug panel's
/// "Bulb Lights" section, applied every frame by [`apply_bulb_lights`]. Both
/// bulbs share every setting but position (one set of controls moves both
/// identical bare-bulb fixtures at once), so only `positions` is per-light.
#[derive(Resource)]
pub(crate) struct BulbLightSettings {
    pub(crate) positions: [Vec3; 2],
    pub(crate) color: [f32; 3],
    pub(crate) intensity: f32,
    pub(crate) range: f32,
    pub(crate) shadows_enabled: bool,
    /// Shows both bulbs' [`ContainerBulbLightMarker`] — debug panel only,
    /// off by default.
    pub(crate) markers_visible: bool,
}

impl Default for BulbLightSettings {
    fn default() -> Self {
        Self {
            // Dialed in against the two bulb-fixture models (see the "Bulb
            // Lights" debug-panel section).
            positions: [Vec3::new(23.8, 2.0, -5.0), Vec3::new(26.1, 2.0, 3.1)],
            // r255 g202 b0 (0-255) — warm incandescent-bulb orange.
            color: srgb_parts(Color::srgb(255.0 / 255.0, 202.0 / 255.0, 0.0 / 255.0)),
            intensity: 2_000_000.0,
            range: 5.0,
            shadows_enabled: true,
            markers_visible: false,
        }
    }
}

/// Push [`BulbLightSettings`] onto both [`ContainerBulbLight`]s every frame —
/// same shared-settings-plus-per-index-position shape as
/// `apply_shipment_lights`/`ShipmentLightSettings::lights`.
pub(crate) fn apply_bulb_lights(
    settings: Res<BulbLightSettings>,
    mut lights: Query<(&ContainerBulbLight, &mut Transform, &mut PointLight)>,
) {
    for (index, mut transform, mut point) in &mut lights {
        let Some(&pos) = settings.positions.get(index.0) else {
            continue;
        };
        transform.translation = pos;
        point.color = color_from_parts(settings.color);
        point.intensity = settings.intensity;
        point.range = settings.range;
        point.shadows_enabled = settings.shadows_enabled;
    }
}

/// Shows every [`ContainerBulbLightMarker`] only while the debug panel is
/// open *and* [`BulbLightSettings::markers_visible`] is on — mirrors
/// [`sync_light_marker_visibility`].
pub(crate) fn sync_bulb_marker_visibility(
    settings: Res<Settings>,
    bulbs: Res<BulbLightSettings>,
    mut marker: Query<&mut Visibility, With<ContainerBulbLightMarker>>,
) {
    let target = if settings.debug_mode && bulbs.markers_visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut vis in &mut marker {
        *vis = target;
    }
}
