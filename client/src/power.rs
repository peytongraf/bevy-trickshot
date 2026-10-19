//! The map's lights and the `Zombies` power switch that turns them on.
//!
//! A map's power lights are its layout's (`shared::level`, placed and tuned
//! in the level editor) — lit only on a night map (`MapId::is_night`): the
//! day version of a place shares its layout but has none on. In `Zombies`
//! they start off: a player pays at the
//! switch — the power lever model, where `shared::power` puts it — and the
//! server sets `Lobby::power_on`, which fades them in for everyone. Every
//! client sees that flip, so for everyone the lever throws (its one
//! animation, once) and the power-on sound plays from it. In the other modes
//! there's no switch, so they're simply on.
//!
//! The power station (`shared::power_station`) is scenery the lever's put
//! on in the level editor: its model and solid box, in a `Zombies` game on
//! a map that has one ([`sync_power_station`]). Once the power's on, its
//! two Tesla coils crackle with electricity — fading in, then arcing
//! wildly off their tips for the rest of the game ([`spark_coils`]) — its
//! coils' light flashing with them, its blue light coming on in the middle
//! of its glass (day or night, turning with it as a perk machine's does)
//! and its hum looping from it.
//!
//! The lights, the lever, the station and the lever's sound are
//! `StateScoped(InGame)` (the lever
//! remembers whether it's thrown on itself), and the fade level drops back
//! to 0 whenever there are no lights — nothing carries into the next game.

use bevy::prelude::*;

use crate::audio_engine::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::level::{PowerLight, SpotCone};
use shared::{GameMode, Lobby};

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::zombies_hud::{my_lobby, zombies_game, CARD_RED, MONEY_YELLOW, PERK_CARD_WIDTH};
use crate::{killcam, menu, AppState, CurrentMap, Player, EYE_HEIGHT, HUD_FONT};

pub(crate) struct PowerPlugin;

impl Plugin for PowerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLightSettings>()
            .init_resource::<PowerLevel>()
            .init_resource::<PowerLeverSettings>()
            .init_resource::<MachineHumSettings>()
            .add_observer(glaze_power_station)
            .add_systems(Startup, setup_coil_assets)
            .add_systems(OnEnter(AppState::InGame), spawn_power_card)
            .add_systems(
                Update,
                (
                    sync_power_lever,
                    (sync_power_station, spark_coils).chain(),
                    (sync_machine_hums, swell_machine_hums).chain(),
                    update_power_card,
                    turn_on_power.run_if(menu::game_active.and(killcam::no_killcam)),
                )
                    .run_if(in_state(AppState::InGame)),
            )
            // Not gated on `InGame`: it's what takes the lights away (and
            // resets the fade) once the game is left.
            .add_systems(Update, sync_map_lights);
    }
}

// --- map lights ------------------------------------------------------------

/// The map lights' fade and the debug "power on" ("Map lights" in the debug
/// panel). The lights themselves are each map's own — placed and tuned in
/// the level editor (`shared::level::ZombiesLayout::power_lights`).
#[derive(Resource, Clone)]
pub(crate) struct MapLightSettings {
    /// Seconds for the lights to fade in once the power's on.
    pub(crate) fade_secs: f32,
    /// Debug: act as if the power's on (untick and tick again to replay the
    /// fade).
    pub(crate) force_on: bool,
}

impl Default for MapLightSettings {
    fn default() -> Self {
        Self {
            fade_secs: 5.0,
            force_on: false,
        }
    }
}

/// How far the map lights are faded in, 0..=1.
#[derive(Resource, Default)]
struct PowerLevel(f32);

/// On the game's power lights (the editor has its own).
#[derive(Component)]
struct GameLight;

/// A power light's entity — which of the layout's it is, and what it was
/// built as (a spotlight? with a bulb?), so a change there rebuilds it. Its
/// glowing bulb ([`PowerLightGlow`]), if it has one, is a child.
#[derive(Component)]
pub(crate) struct PowerLightSlot {
    index: usize,
    shape: (bool, bool),
}

/// A power light's glowing bulb (`shared::level::PowerLight::glow`): an
/// emissive ball where it hangs, with a short light of its own.
#[derive(Component)]
pub(crate) struct PowerLightGlow;

/// The glowing bulb's size — a fixture, not a floating ball.
const GLOW_BULB_RADIUS: f32 = 0.35;
/// How far the bulb's own light reaches — just its fixture.
const GLOW_RANGE: f32 = 12.0;
/// Lumens of glow per unit of the bulb's emissive brightness (by eye, so a
/// bright one blooms without washing out).
const GLOW_EMISSIVE_PER_LUMEN: f32 = 25_000.0;

fn shape(light: &PowerLight) -> (bool, bool) {
    (light.spot.is_some(), light.glow > 0.0)
}

fn light_color(light: &PowerLight) -> Color {
    Color::srgb(light.color[0], light.color[1], light.color[2])
}

/// Where a power light hangs, aimed if it's a spotlight.
fn light_transform(light: &PowerLight) -> Transform {
    Transform::from_translation(light.pos).with_rotation(light.spot.map_or(Quat::IDENTITY, |c| c.rotation()))
}

/// A point power light as a Bevy light, `fade` (0..1) of the way up.
fn point_light(light: &PowerLight, fade: f32) -> PointLight {
    PointLight {
        color: light_color(light),
        intensity: light.intensity * fade,
        range: light.range,
        radius: light.radius,
        shadows_enabled: light.shadows && fade > 0.0,
        ..default()
    }
}

/// A spotlight power light as a Bevy light, `fade` (0..1) of the way up.
fn spot_light(light: &PowerLight, cone: SpotCone, fade: f32) -> SpotLight {
    let outer = cone.outer_angle_deg.to_radians();
    SpotLight {
        color: light_color(light),
        intensity: light.intensity * fade,
        range: light.range,
        radius: light.radius,
        shadows_enabled: light.shadows && fade > 0.0,
        outer_angle: outer,
        inner_angle: cone.inner_angle_deg.to_radians().min(outer),
        ..default()
    }
}

/// The bulb's own short light.
fn glow_light(light: &PowerLight, fade: f32) -> PointLight {
    PointLight {
        color: light_color(light),
        intensity: light.glow * fade,
        range: GLOW_RANGE,
        shadows_enabled: false,
        ..default()
    }
}

/// The bulb's emissive brightness — well past 1 (HDR) so bloom catches it.
fn glow_emissive(light: &PowerLight, fade: f32) -> LinearRgba {
    LinearRgba::from(light_color(light)) * light.glow * fade / GLOW_EMISSIVE_PER_LUMEN
}

/// Keep a set of power-light entities (the ones `lights` finds) matching
/// `wanted` at `fade` (0..1): rebuilt — each spawned with `extra` — when the
/// set's changed shape, otherwise brought up to date when `changed`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_power_lights<F: bevy::ecs::query::QueryFilter, B: Bundle>(
    wanted: &[PowerLight],
    fade: f32,
    changed: bool,
    lights: &Query<(Entity, &PowerLightSlot), F>,
    glows: &Query<(Entity, &ChildOf, &MeshMaterial3d<StandardMaterial>), With<PowerLightGlow>>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    extra: impl Fn() -> B,
) {
    let stale = lights.iter().count() != wanted.len()
        || lights
            .iter()
            .any(|(_, slot)| wanted.get(slot.index).map(shape) != Some(slot.shape));
    if stale {
        for (e, _) in lights {
            commands.entity(e).despawn();
        }
        for (index, light) in wanted.iter().enumerate() {
            let mut e = commands.spawn((
                extra(),
                PowerLightSlot {
                    index,
                    shape: shape(light),
                },
                light_transform(light),
            ));
            match light.spot {
                Some(cone) => e.insert(spot_light(light, cone, fade)),
                None => e.insert(point_light(light, fade)),
            };
            if light.glow > 0.0 {
                e.with_child((
                    PowerLightGlow,
                    Mesh3d(meshes.add(Sphere::new(GLOW_BULB_RADIUS))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: light_color(light),
                        emissive: glow_emissive(light, fade),
                        ..default()
                    })),
                    bevy::pbr::NotShadowCaster,
                    glow_light(light, fade),
                ));
            }
        }
        return;
    }
    if !changed {
        return;
    }
    for (e, slot) in lights {
        let Some(light) = wanted.get(slot.index) else { continue };
        match light.spot {
            Some(cone) => commands.entity(e).insert((spot_light(light, cone, fade), light_transform(light))),
            None => commands.entity(e).insert((point_light(light, fade), light_transform(light))),
        };
        for (glow, parent, material) in glows {
            if parent.parent() != e {
                continue;
            }
            commands.entity(glow).insert(glow_light(light, fade));
            if let Some(m) = materials.get_mut(material) {
                m.base_color = light_color(light);
                m.emissive = glow_emissive(light, fade);
            }
        }
    }
}

/// Put the map's power lights up while in a game on a night map that has
/// them (a day map has none on), fade them toward on / off, and keep them
/// matching the panel.
#[allow(clippy::too_many_arguments)]
fn sync_map_lights(
    state: Res<State<AppState>>,
    current: Res<CurrentMap>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    settings: Res<MapLightSettings>,
    time: Res<Time>,
    mut level: ResMut<PowerLevel>,
    lights: Query<(Entity, &PowerLightSlot), With<GameLight>>,
    glows: Query<(Entity, &ChildOf, &MeshMaterial3d<StandardMaterial>), With<PowerLightGlow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let wanted: &[PowerLight] = if current.0.is_night() {
        &shared::level::layout(current.0).power_lights
    } else {
        &[]
    };
    if *state.get() != AppState::InGame || wanted.is_empty() {
        for (e, _) in &lights {
            commands.entity(e).despawn();
        }
        level.0 = 0.0;
        return;
    }

    // In `Zombies` the power has to be turned on (and fades in); anywhere
    // else there's no switch, so they're just on.
    let lobby = my_lobby(&local, &lobbies);
    let zombies = lobby.is_some_and(|l| l.mode == GameMode::Zombies);
    let on = settings.force_on || !zombies || lobby.is_some_and(|l| l.power_on);
    let before = level.0;
    let target = if on { 1.0 } else { 0.0 };
    level.0 = if !zombies || settings.fade_secs <= 0.0 {
        target
    } else {
        let step = time.delta_secs() / settings.fade_secs;
        level.0 + (target - level.0).clamp(-step, step)
    };
    let changed = level.0 != before || settings.is_changed() || current.is_changed();
    let fade = level.0 * level.0 * (3.0 - 2.0 * level.0);
    sync_power_lights(
        wanted,
        fade,
        changed,
        &lights,
        &glows,
        &mut commands,
        &mut meshes,
        &mut materials,
        || (StateScoped(AppState::InGame), GameLight),
    );
}

// --- the switch ------------------------------------------------------------

/// The power lever model: one clip, the lever going from up to down.
pub(crate) const LEVER_MODEL: &str = "models/props/power_lever.glb";
pub(crate) const POWER_STATION_MODEL: &str = "models/props/power_station.glb";

/// The power station (its model, solid box and coils' sparks are
/// children): how far its coils have sparked up, `0..=1`.
#[derive(Component, Default)]
struct PowerStation {
    fade: f32,
}

/// The top of each Tesla coil (the ball on it), in the station model's own
/// frame — `power_station.glb`'s `Sphere` / `Sphere.007`.
const COIL_TIPS: [Vec3; 2] = [Vec3::new(0.0, 3.86, -1.05), Vec3::new(0.0, 3.86, 1.05)];
/// Each coil's middle line (x, z) and how far down it runs (y) — arcs crawl
/// down its sides.
const COIL_BODY_Y: (f32, f32) = (2.5, 3.65);
const COIL_RADIUS: f32 = 0.17;
/// The middle of the station's ring (the dial) — some arcs reach for it.
const RING_CENTRE: Vec3 = Vec3::new(0.0, 2.75, 0.0);
const RING_RADIUS: f32 = 0.62;
/// Arcs off each coil at once, and the segments in each (its main jag,
/// then a fork off it).
const ARCS_PER_COIL: usize = 5;
const ARC_SEGMENTS: usize = 10;
const FORK_SEGMENTS: usize = 4;
/// How long (s) the sparking takes to come up once the power's on.
const COIL_FADE_SECS: f32 = 1.5;
/// Each arc's re-strike: a fresh path every this many seconds (random in
/// between) — fast, so it reads as an angry crackle.
const ARC_RESTRIKE: (f32, f32) = (0.025, 0.08);
/// The share of strikes an arc's dark for.
const ARC_DARK: f32 = 0.3;
/// The electricity's colour (linear, before brightness), its brightness
/// (blooms into a glow) and an arc's thickness (m, model frame).
const SPARK_COLOR: [f32; 3] = [0.45, 0.7, 1.0];
const SPARK_BRIGHTNESS: f32 = 60.0;
const ARC_WIDTH: f32 = 0.018;
/// Each coil's light: its brightness (lm) at a flash and how far it
/// reaches; the glow it settles to between flashes (a share of that); and
/// how soon (s, random in between) the next flash comes and how fast one
/// dies away (1/s).
const COIL_LIGHT_LM: f32 = 400_000.0;
const COIL_LIGHT_RANGE: f32 = 9.0;
const COIL_LIGHT_BASE: f32 = 0.25;
const COIL_FLASH_EVERY: (f32, f32) = (0.05, 0.3);
const COIL_FLASH_DECAY: f32 = 14.0;
/// The station's power light: blue, in the middle of its glass (model
/// frame — so it moves and turns with it, as a perk machine's does), on
/// with the power, day or night, flickering a little with the coils.
const POWER_LIGHT_AT: Vec3 = Vec3::new(0.0, 2.81, 0.0);
const POWER_LIGHT_COLOR: Color = Color::srgb(0.0, 0.231, 0.987);
const POWER_LIGHT_LM: f32 = 2_000_000.0;
const POWER_LIGHT_RANGE: f32 = 8.5;
const POWER_LIGHT_RADIUS: f32 = 0.77;
/// How much (a share) the power light dips with the coils' flicker.
const POWER_LIGHT_FLICKER: f32 = 0.12;
/// Its hum (looped from it once the power's on): the clip, and how loud.
const STATION_HUM: &str = "audio/ambient/power_station_hum.mp3";
const STATION_HUM_VOLUME: f32 = 1.0;

/// What every coil spark shares: a segment (a thin cylinder, stretched
/// between two points), a glow ball, and the glowing materials (brightened
/// with the fade).
#[derive(Resource)]
struct CoilAssets {
    segment: Handle<Mesh>,
    ball: Handle<Mesh>,
    arc: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
}

fn spark(brightness: f32) -> LinearRgba {
    LinearRgba::rgb(
        SPARK_COLOR[0] * brightness,
        SPARK_COLOR[1] * brightness,
        SPARK_COLOR[2] * brightness,
    )
}

fn setup_coil_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glowing = |materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: Color::BLACK,
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        })
    };
    commands.insert_resource(CoilAssets {
        segment: meshes.add(Cylinder::new(0.5, 1.0)),
        ball: meshes.add(Sphere::new(1.0)),
        arc: glowing(&mut materials),
        glow: glowing(&mut materials),
    });
}

/// One arc off a coil: which coil, and when it strikes anew.
#[derive(Component)]
struct CoilArc {
    coil: usize,
    next: f32,
    seq: u32,
}

/// One segment of a [`CoilArc`] (its main jag first, then its fork).
#[derive(Component)]
struct ArcSegment(usize);

/// A coil's crackling glow ball, and its light (its flash dying away, and
/// when the next comes).
#[derive(Component)]
struct CoilGlow(usize);
#[derive(Component)]
struct CoilLight {
    coil: usize,
    flash: f32,
    next: f32,
}

/// The station's blue power light, and its hum.
#[derive(Component)]
struct StationPowerLight;
#[derive(Component)]
struct StationHum;

/// The coils, once the power's on: fading up over [`COIL_FADE_SECS`] (with
/// a surge as they catch), then crackling for good — each arc re-struck
/// every few hundredths of a second along a fresh jagged path off its
/// coil's tip: out into the air, down the coil's own side, or across to
/// the ring; a fork off most; some dark each strike. The tips' glow and
/// light flicker with it. Off (and dark) until then.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn spark_coils(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    assets: Option<Res<CoilAssets>>,
    light_settings: Res<MapLightSettings>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut stations: Query<&mut PowerStation>,
    mut arcs: Query<(&mut CoilArc, &Children)>,
    mut segments: Query<(&ArcSegment, &mut Transform), (Without<CoilGlow>, Without<CoilLight>)>,
    mut glows: Query<(&CoilGlow, &mut Transform), (Without<ArcSegment>, Without<CoilLight>)>,
    mut lights: Query<(&mut CoilLight, &mut PointLight), Without<StationPowerLight>>,
    (mut power_light, mut hum): (
        Query<&mut PointLight, (With<StationPowerLight>, Without<CoilLight>)>,
        Query<&mut SpatialAudioSink, With<StationHum>>,
    ),
) {
    let Some(assets) = assets else { return };
    let Ok(mut station) = stations.single_mut() else { return };
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    // (The debug panel's "power on" too, as for the lights.)
    let on = zombies_game(&local, &lobbies).is_some_and(|l| l.power_on) || light_settings.force_on;
    let target = if on { 1.0 } else { 0.0 };
    let step = dt / COIL_FADE_SECS;
    let before = station.fade;
    station.fade += (target - station.fade).clamp(-step, step);
    let fade = station.fade * station.fade * (3.0 - 2.0 * station.fade);
    // (A surge as it catches: brighter for a moment partway up.)
    let surge = 1.0 + 1.5 * (std::f32::consts::PI * station.fade).sin().powi(2) * f32::from(on);
    if fade != before || fade > 0.0 {
        let flicker = 0.75 + 0.5 * crate::util::rand01((now * 60.0) as u32 ^ 0x51ED);
        if let Some(m) = materials.get_mut(&assets.arc) {
            m.base_color = Color::LinearRgba(spark(SPARK_BRIGHTNESS * fade * surge));
        }
        if let Some(m) = materials.get_mut(&assets.glow) {
            m.base_color = Color::LinearRgba(spark(SPARK_BRIGHTNESS * 0.25 * fade * flicker));
        }
    }
    // (Off and already hidden: nothing to do.)
    let idle = fade <= 0.0 && before <= 0.0;
    for (mut arc, children) in &mut arcs {
        if idle || (now < arc.next && fade > 0.0) {
            continue;
        }
        arc.seq = arc.seq.wrapping_add(1);
        let base = arc.seq.wrapping_mul(2_654_435_761) ^ (arc.coil as u32 * 0x85EB_CA6B);
        let roll = |k: u32| crate::util::rand01(base ^ k.wrapping_mul(0x9E37_79B9));
        arc.next = now + ARC_RESTRIKE.0 + (ARC_RESTRIKE.1 - ARC_RESTRIKE.0) * roll(1);
        // (Fewer as it fades in; some dark each strike.)
        let lit = fade > 0.0 && roll(2) < fade * (1.0 - ARC_DARK);
        let tip = COIL_TIPS[arc.coil];
        let side = tip.z.signum();
        let points = if lit {
            let kind = roll(3);
            let end = if kind < 0.5 {
                // Out into the air round the tip — mostly out and up.
                let yaw = roll(4) * std::f32::consts::TAU;
                let rise = -0.6 + 1.4 * roll(5);
                let out = Vec3::new(yaw.cos(), rise, yaw.sin() * 0.6 + side * 0.4).normalize();
                tip + out * (0.4 + 0.65 * roll(6))
            } else if kind < 0.8 {
                // Down the coil's own side.
                let around = roll(4) * std::f32::consts::TAU;
                let y = COIL_BODY_Y.0 + (COIL_BODY_Y.1 - COIL_BODY_Y.0) * roll(5);
                Vec3::new(around.cos() * COIL_RADIUS, y, tip.z + around.sin() * COIL_RADIUS)
            } else {
                // Across to the ring.
                let a = (roll(4) - 0.5) * 1.6;
                RING_CENTRE + Vec3::new(0.0, a.cos(), side * a.sin().abs().max(0.4)) * RING_RADIUS
            };
            let span = end - tip;
            let len = span.length().max(1e-3);
            let dir = span / len;
            let across = dir.any_orthonormal_vector();
            let up = dir.cross(across);
            let mut main = Vec::with_capacity(ARC_SEGMENTS + 1);
            for i in 0..=ARC_SEGMENTS {
                let t = i as f32 / ARC_SEGMENTS as f32;
                let bend = (std::f32::consts::PI * t).sin() * len * 0.22;
                let jag = across * (roll(10 + i as u32) - 0.5) + up * (roll(40 + i as u32) - 0.5);
                main.push(tip + span * t + if i == 0 || i == ARC_SEGMENTS { Vec3::ZERO } else { jag * bend });
            }
            // A fork off partway along, flicking off sideways.
            let from = 2 + (roll(70) * (ARC_SEGMENTS - 4) as f32) as usize;
            let off = (across * (roll(71) - 0.5) + up * (roll(72) - 0.5) + dir * 0.3).normalize_or(dir);
            let mut fork = vec![main[from]];
            for j in 1..=FORK_SEGMENTS {
                let jag = (across * (roll(80 + j as u32) - 0.5) + up * (roll(90 + j as u32) - 0.5)) * len * 0.08;
                fork.push(fork[j - 1] + off * len * 0.09 + jag);
            }
            Some((main, fork, roll(73) < 0.7))
        } else {
            None
        };
        for child in children.iter() {
            let Ok((seg, mut tf)) = segments.get_mut(child) else { continue };
            let i = seg.0;
            let piece = points.as_ref().and_then(|(main, fork, forked)| {
                if i < ARC_SEGMENTS {
                    Some((main[i], main[i + 1], 1.0))
                } else if *forked {
                    let j = i - ARC_SEGMENTS;
                    Some((fork[j], fork[j + 1], 0.6))
                } else {
                    None
                }
            });
            *tf = match piece {
                Some((a, b, w)) => {
                    let d = b - a;
                    let len = d.length().max(1e-4);
                    let width = ARC_WIDTH * w;
                    Transform {
                        translation: (a + b) * 0.5,
                        rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
                        scale: Vec3::new(width, len, width),
                    }
                }
                None => Transform::from_scale(Vec3::ZERO),
            };
        }
    }
    // The tips' glow and light, crackling.
    let pulse = |coil: usize| {
        let r = crate::util::rand01(((now * 45.0) as u32).wrapping_mul(31) ^ coil as u32);
        fade * (0.55 + 0.45 * r)
    };
    for (glow, mut tf) in &mut glows {
        let p = pulse(glow.0);
        tf.scale = Vec3::splat(if fade > 0.0 { 0.06 + 0.06 * p } else { 0.0 });
    }
    // Each coil's light: a glow, flashing bright now and then as an arc
    // strikes and dying away fast.
    let mut flicker = 0.0f32;
    for (mut light, mut point) in &mut lights {
        if now >= light.next {
            let coil = light.coil as u32;
            let r = |k: u32| crate::util::rand01(((now * 1000.0) as u32).wrapping_mul(2_654_435_761) ^ k ^ coil);
            light.flash = 0.6 + 0.4 * r(1);
            light.next = now + COIL_FLASH_EVERY.0 + (COIL_FLASH_EVERY.1 - COIL_FLASH_EVERY.0) * r(2);
        }
        light.flash *= (-COIL_FLASH_DECAY * dt).exp();
        flicker = flicker.max(light.flash);
        let level = COIL_LIGHT_BASE + (1.0 - COIL_LIGHT_BASE) * light.flash;
        point.intensity = COIL_LIGHT_LM * fade * level * surge;
    }
    // The power light, on with the power, dipping a little with the
    // flashes; the hum with it.
    for mut point in &mut power_light {
        point.intensity = POWER_LIGHT_LM * fade * (1.0 - POWER_LIGHT_FLICKER * (1.0 - flicker));
    }
    for mut sink in &mut hum {
        sink.set_volume(Volume::Linear(STATION_HUM_VOLUME * fade));
    }
}

/// The power station's scene — its glass is swapped once it's in
/// ([`glaze_power_station`]).
#[derive(Component)]
pub(crate) struct PowerStationModel;

/// The name of the station model's glass material (the dial's face).
const STATION_GLASS: &str = "glass";

/// The station's scene is in: its glass gets a plain see-through pane in
/// place of what glTF brings. That's Blender's Transmission on top of an
/// alpha blend — Bevy draws the first as its screen-space see-through (a
/// copy of what's behind on screen, bent by the angle it's seen at) and
/// the second over it again, so the face jumped between clear and a flat
/// bright sheen as you moved. Here it's a faint tint you see straight
/// through, with its highlights kept at full strength (premultiplied
/// alpha) — no metal, no transmission. (Shared by the editor's station.)
pub(crate) fn glaze_power_station(
    trigger: Trigger<bevy::scene::SceneInstanceReady>,
    models: Query<(), With<PowerStationModel>>,
    children: Query<&Children>,
    glass: Query<(&bevy::gltf::GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let root = trigger.target();
    if !models.contains(root) {
        return;
    }
    let mut pane: Option<Handle<StandardMaterial>> = None;
    for e in children.iter_descendants(root) {
        let Ok((name, own)) = glass.get(e) else { continue };
        if name.0 != STATION_GLASS {
            continue;
        }
        let pane = pane.get_or_insert_with(|| {
            // (Its own tint, from the model.)
            let tint = materials
                .get(&own.0)
                .map_or(LinearRgba::rgb(0.13, 0.31, 0.6), |m| m.base_color.to_linear());
            let alpha = 0.18;
            materials.add(StandardMaterial {
                // (Premultiplied: the tint's already scaled by how much of it
                // there is.)
                base_color: Color::LinearRgba(LinearRgba::new(tint.red * alpha, tint.green * alpha, tint.blue * alpha, alpha)),
                alpha_mode: AlphaMode::Premultiplied,
                metallic: 0.0,
                perceptual_roughness: 0.05,
                reflectance: 0.5,
                specular_transmission: 0.0,
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        });
        commands.entity(e).insert(MeshMaterial3d(pane.clone()));
    }
}

/// Keep the power station where the map's layout puts it in a `Zombies`
/// game — none anywhere else.
fn sync_power_station(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    coil_assets: Option<Res<CoilAssets>>,
    mut stations: Query<(Entity, &mut Transform), With<PowerStation>>,
    mut commands: Commands,
) {
    let Some(at) = zombies_game(&local, &lobbies).and_then(|l| shared::power_station::placement(l.map)) else {
        for (e, _) in &stations {
            commands.entity(e).despawn();
        }
        return;
    };
    if let Some((_, mut t)) = stations.iter_mut().next() {
        t.set_if_neq(crate::util::placed(at));
        return;
    }
    let half = shared::power_station::HALF_EXTENTS;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PowerStation::default(),
            crate::util::placed(at),
            Visibility::default(),
        ))
        .with_children(|s| {
            // The coils' sparks, dark until the power's on.
            if let Some(assets) = &coil_assets {
                for (coil, tip) in COIL_TIPS.into_iter().enumerate() {
                    for arc in 0..ARCS_PER_COIL {
                        s.spawn((
                            CoilArc {
                                coil,
                                next: 0.0,
                                seq: (coil * 97 + arc * 13) as u32,
                            },
                            Transform::default(),
                            Visibility::default(),
                        ))
                        .with_children(|a| {
                            for i in 0..ARC_SEGMENTS + FORK_SEGMENTS {
                                a.spawn((
                                    ArcSegment(i),
                                    Mesh3d(assets.segment.clone()),
                                    MeshMaterial3d(assets.arc.clone()),
                                    Transform::from_scale(Vec3::ZERO),
                                    bevy::pbr::NotShadowCaster,
                                    bevy::render::view::NoFrustumCulling,
                                ));
                            }
                        });
                    }
                    s.spawn((
                        CoilGlow(coil),
                        Mesh3d(assets.ball.clone()),
                        MeshMaterial3d(assets.glow.clone()),
                        Transform::from_translation(tip).with_scale(Vec3::ZERO),
                        bevy::pbr::NotShadowCaster,
                    ));
                    s.spawn((
                        CoilLight {
                            coil,
                            flash: 0.0,
                            next: 0.0,
                        },
                        PointLight {
                            color: Color::LinearRgba(spark(1.0)),
                            intensity: 0.0,
                            range: COIL_LIGHT_RANGE,
                            shadows_enabled: false,
                            ..default()
                        },
                        Transform::from_translation(tip + Vec3::Y * 0.15),
                    ));
                }
            }
            s.spawn((
                StationPowerLight,
                PointLight {
                    color: POWER_LIGHT_COLOR,
                    intensity: 0.0,
                    range: POWER_LIGHT_RANGE,
                    radius: POWER_LIGHT_RADIUS,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_translation(POWER_LIGHT_AT),
            ));
            // (Silent until the power's on — `spark_coils` turns it up.)
            s.spawn((
                StationHum,
                AudioPlayer::new(asset_server.load(STATION_HUM)),
                PlaybackSettings::LOOP.with_spatial(true).with_volume(Volume::Linear(0.0)),
                Transform::from_translation(POWER_LIGHT_AT),
            ));
            s.spawn((
                PowerStationModel,
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(POWER_STATION_MODEL))),
            ));
            // Solid — the server's `shared::power_station::solid_box`. (No
            // bullet holes: the box is looser than the model up top.)
            s.spawn((
                crate::vfx::NoBulletHoles,
                bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                Transform::from_translation(Vec3::Y * half.y),
            ));
        });
}
/// That clip's length (s) at normal speed.
const LEVER_CLIP_SECS: f32 = 0.833;

/// Panel-tunable power lever ("Power lever (Zombies)"): where it stands
/// relative to the switch spot in the map's layout (`shared::power`), in the
/// switch's own frame, its turn on top of the switch's, size, and how fast
/// it throws.
#[derive(Resource, Clone)]
pub(crate) struct PowerLeverSettings {
    /// Metres from `shared::power::switch_pos` (the ground under it) to the
    /// model's middle, turned with the switch.
    pub(crate) offset: Vec3,
    /// Degrees about x (pitch), y (turn) and z (roll).
    pub(crate) rotation_deg: Vec3,
    /// The model comes in hundreds of units tall: this makes it a
    /// ~0.6 m panel.
    pub(crate) scale: f32,
    /// Throw animation speed (1 = as authored, 0.83 s).
    pub(crate) anim_speed: f32,
    /// Debug: throw it again (it plays the sound too) — cleared once done.
    pub(crate) replay: bool,
}

impl Default for PowerLeverSettings {
    fn default() -> Self {
        Self {
            // On the wall, at chest height.
            offset: Vec3::new(0.0, 1.8, 0.0),
            rotation_deg: Vec3::ZERO,
            scale: 0.002,
            anim_speed: 1.0,
            replay: false,
        }
    }
}

impl PowerLeverSettings {
    /// The lever at a switch standing at `switch`.
    pub(crate) fn transform(&self, switch: shared::level::Placement) -> Transform {
        crate::util::placed(switch).mul_transform(self.model_transform())
    }

    /// The lever, in the switch's own frame (from the ground under it).
    pub(crate) fn model_transform(&self) -> Transform {
        let r = self.rotation_deg;
        Transform::from_translation(self.offset)
            .with_rotation(Quat::from_euler(EulerRot::YXZ, r.y.to_radians(), r.x.to_radians(), r.z.to_radians()))
            .with_scale(Vec3::splat(self.scale.max(1e-5)))
    }
}

/// The power lever: its throw clip, its `AnimationPlayer` once the scene has
/// spawned one, and whether it's thrown (so the power coming on — seen as
/// `Lobby::power_on` flipping — throws it just the once).
#[derive(Component)]
struct PowerLever {
    graph: Handle<AnimationGraph>,
    clip: AnimationNodeIndex,
    player: Option<Entity>,
    thrown: bool,
}

/// The power-on sound, playing from the lever.
#[derive(Component)]
struct PowerOnSound;

/// Put the lever on the map while we're in a `Zombies` game on a map that
/// has a switch (and take it away otherwise), keep it where the panel says,
/// and throw it — with its sound — when the power comes on.
#[allow(clippy::too_many_arguments)]
fn sync_power_lever(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut settings: ResMut<PowerLeverSettings>,
    asset_server: Res<AssetServer>,
    sounds: Option<Res<crate::GameSounds>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut levers: Query<(Entity, &mut PowerLever, &mut Transform)>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    mut commands: Commands,
) {
    let lobby = zombies_game(&local, &lobbies);
    let switch = lobby.and_then(|l| shared::level::layout(l.map).power_switch);
    let (Some(lobby), Some(switch)) = (lobby, switch) else {
        for (e, ..) in &levers {
            commands.entity(e).despawn();
        }
        return;
    };
    if levers.is_empty() {
        let (graph, clip) =
            AnimationGraph::from_clip(asset_server.load(GltfAssetLabel::Animation(0).from_asset(LEVER_MODEL)));
        commands.spawn((
            StateScoped(AppState::InGame),
            PowerLever {
                graph: graphs.add(graph),
                clip,
                player: None,
                // (Already on — joined late: it's just down, no throw.)
                thrown: lobby.power_on,
            },
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(LEVER_MODEL))),
            settings.transform(switch),
        ));
        return;
    }

    let replay = std::mem::take(&mut settings.bypass_change_detection().replay);
    for (entity, mut lever, mut t) in &mut levers {
        t.set_if_neq(settings.transform(switch));

        // The scene's `AnimationPlayer`, once it's there: hold it up (the
        // clip's first frame), or down if the power's already on.
        if lever.player.is_none() {
            let Some(found) = children.iter_descendants(entity).find(|&e| players.contains(e)) else {
                continue;
            };
            let mut player = players.get_mut(found).unwrap();
            let clip = player.start(lever.clip);
            if lever.thrown {
                clip.seek_to(LEVER_CLIP_SECS);
            } else {
                clip.pause();
            }
            commands.entity(found).insert(AnimationGraphHandle(lever.graph.clone()));
            lever.player = Some(found);
        }
        let Some(mut player) = lever.player.and_then(|p| players.get_mut(p).ok()) else {
            continue;
        };

        // The power just came on (or the panel's replay): throw it, once,
        // and the sound from it.
        if (lobby.power_on && !lever.thrown) || replay {
            lever.thrown = true;
            // (`start` rewinds but keeps it paused from holding it up.)
            player
                .start(lever.clip)
                .resume()
                .set_speed(settings.anim_speed.max(0.01));
            if let Some(sounds) = sounds.as_ref() {
                commands.spawn((
                    StateScoped(AppState::InGame),
                    PowerOnSound,
                    AudioPlayer::new(sounds.power_on.clone()),
                    // From the lever itself.
                    Transform::from_translation(t.translation),
                    crate::positional_playback(crate::audio_engine::Volume::Linear(1.0)),
                ));
            }
        }
        if settings.is_changed() {
            if let Some(clip) = player.animation_mut(lever.clip) {
                if !clip.is_paused() {
                    clip.set_speed(settings.anim_speed.max(0.01));
                }
            }
        }
    }
}

// --- the machines' hum ----------------------------------------------------

/// On a machine (every perk machine, the Pack-a-Punch): once it's on (the
/// power — or for a perk machine, from the start unless the lobby makes
/// perks wait for it, `shared::perks::machines_on`) it hums —
/// `GameSounds::machine_hum` looped from `offset` in its own space. The hum
/// is a child, so it goes when the machine does.
#[derive(Component)]
pub(crate) struct PoweredHum {
    pub(crate) offset: Vec3,
    /// A perk machine's, not the Pack-a-Punch's.
    pub(crate) perk: bool,
}

/// On a [`PoweredHum`] machine once its hum is going.
#[derive(Component)]
struct Humming;

/// A machine's looping hum, and when it started (it swells in).
#[derive(Component)]
struct MachineHum {
    started: f32,
}

/// Panel-tunable machine hum ("Machine hum (Zombies)"). How it fades with
/// distance is the "Machine hum" profile's, in the audio mixer.
#[derive(Resource, Clone)]
pub(crate) struct MachineHumSettings {
    /// Seconds a hum takes to swell to full once the power comes on.
    pub(crate) fade_in_secs: f32,
}

impl Default for MachineHumSettings {
    fn default() -> Self {
        Self { fade_in_secs: 1.5 }
    }
}

/// Start every machine's hum once the power's on (or the panel forces it).
fn sync_machine_hums(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    light_settings: Res<MapLightSettings>,
    sounds: Option<Res<crate::GameSounds>>,
    time: Res<Time>,
    machines: Query<(Entity, &PoweredHum), Without<Humming>>,
    mut commands: Commands,
) {
    let Some(sounds) = sounds else { return };
    let Some(lobby) = zombies_game(&local, &lobbies) else { return };
    let force = light_settings.force_on;
    let power = force || shared::power::has_power(lobby.map, lobby.power_on);
    let perks = force || shared::perks::machines_on(lobby);
    for (machine, hum) in &machines {
        if !(if hum.perk { perks } else { power }) {
            continue;
        }
        commands.entity(machine).insert(Humming);
        commands.spawn((
            MachineHum {
                started: time.elapsed_secs(),
            },
            AudioPlayer::new(sounds.machine_hum.clone()),
            Transform::from_translation(hum.offset),
            // Looped; starts silent, swelling in (`swell_machine_hums`).
            PlaybackSettings::LOOP
                .with_spatial(true)
                .with_volume(crate::audio_engine::Volume::Linear(0.0)),
            ChildOf(machine),
        ));
    }
}

/// Swell each hum in as it starts ([`MachineHumSettings::fade_in_secs`]).
fn swell_machine_hums(
    settings: Res<MachineHumSettings>,
    time: Res<Time>,
    mut hums: Query<(&MachineHum, &mut SpatialAudioSink)>,
) {
    for (hum, mut sink) in &mut hums {
        let swell = if settings.fade_in_secs > 0.0 {
            ((time.elapsed_secs() - hum.started) / settings.fade_in_secs).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if sink.volume().to_linear() != swell {
            sink.set_volume(crate::audio_engine::Volume::Linear(swell));
        }
    }
}

/// Whether we're at the switch with the power still off, and our points.
fn at_switch(lobby: &Lobby, me: lightyear::prelude::PeerId, feet: Vec3) -> Option<u32> {
    if lobby.power_on || !shared::power::in_range(lobby.map, feet, 0.0) {
        return None;
    }
    lobby.members.iter().find(|m| m.peer == me).map(|m| m.score)
}

/// The interact key at the switch, if we can afford it: ask the server to
/// turn the power on (it checks again, and flips `Lobby::power_on`).
fn turn_on_power(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut sender: Query<&mut TriggerSender<shared::TurnOnPower>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    let (Some(me), Some(lobby)) = (local.iter().next().map(|l| l.0), zombies_game(&local, &lobbies)) else {
        return;
    };
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !at_switch(lobby, me, feet).is_some_and(|points| points >= shared::power::POWER_COST) {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::TurnOnPower);
    }
}

// --- the card (at the switch) ------------------------------------------------

/// The card shown at the switch while the power's off — the perk card's
/// style (`zombies_hud`), with the cost, our points and what the key does.
#[derive(Component)]
struct PowerCard;

/// The strip along the card's bottom saying what the interact key does.
#[derive(Component)]
struct PowerCardAction;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PowerCardText {
    Cost,
    Points,
    Action,
}

/// The power's colour on the card: a warm electric amber.
const POWER_AMBER: Color = Color::srgb(1.0, 0.72, 0.15);

fn spawn_power_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let body = asset_server.load(crate::BODY_FONT);
    let heading = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    let faint = TextColor(Color::srgba(1.0, 1.0, 1.0, 0.55));
    let divider = (
        Node {
            height: Val::Px(1.0),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.15)),
    );
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(57.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                PowerCard,
                Node {
                    width: Val::Px(PERK_CARD_WIDTH),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(POWER_AMBER),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((Text::new("POWER"), heading(38.0), TextColor(POWER_AMBER)));
                    col.spawn((
                        Text::new("The power is off. Turn it on to light up the map."),
                        TextFont {
                            font: body.clone(),
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                    ));
                });
                card.spawn(divider);
                // Cost on the left, our points on the right.
                card.spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::End,
                    ..default()
                })
                .with_children(|footer| {
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("COST"), heading(16.0), faint));
                            col.spawn((PowerCardText::Cost, Text::new(""), heading(30.0), TextColor(MONEY_YELLOW)));
                        });
                    footer
                        .spawn(Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::End,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((Text::new("YOUR POINTS"), heading(16.0), faint));
                            col.spawn((PowerCardText::Points, Text::new(""), heading(30.0), TextColor::WHITE));
                        });
                });
                card.spawn((
                    PowerCardAction,
                    Node {
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_child((PowerCardText::Action, Text::new(""), heading(24.0), TextColor::WHITE));
            });
        });
}

/// Show the card while we're at the switch and the power's off (hidden behind
/// menus and during a kill cam, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_power_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<PowerCard>>,
    mut action_bar: Single<&mut BackgroundColor, With<PowerCardAction>>,
    mut texts: Query<(&PowerCardText, &mut Text, &mut TextColor)>,
) {
    let (mut vis, mut border) = card.into_inner();
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let points = (!menu.is_open() && active_killcam.0.is_none())
        .then(|| at_switch(zombies_game(&local, &lobbies)?, local.iter().next()?.0, feet))
        .flatten();
    let Some(points) = points else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);

    let cost = shared::power::POWER_COST;
    let affordable = points >= cost;
    let (edge, bar, cost_color, points_color, action, action_color) = if affordable {
        (
            POWER_AMBER,
            POWER_AMBER.with_alpha(0.25),
            MONEY_YELLOW,
            Color::WHITE,
            format!("PRESS {} TO TURN ON THE POWER", binds.interact.label().to_uppercase()),
            Color::WHITE,
        )
    } else {
        (
            CARD_RED,
            CARD_RED.with_alpha(0.2),
            CARD_RED,
            CARD_RED,
            "NOT ENOUGH POINTS".to_string(),
            CARD_RED,
        )
    };
    if border.0 != edge {
        border.0 = edge;
    }
    if action_bar.0 != bar {
        action_bar.0 = bar;
    }
    for (which, mut text, mut color) in &mut texts {
        let (wanted, wanted_color) = match which {
            PowerCardText::Cost => (format!("${}", crate::util::grouped(cost)), cost_color),
            PowerCardText::Points => (format!("${}", crate::util::grouped(points)), points_color),
            PowerCardText::Action => (action.clone(), action_color),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        if color.0 != wanted_color {
            color.0 = wanted_color;
        }
    }
}
