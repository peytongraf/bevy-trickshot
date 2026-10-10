//! The map's lights and the `Zombies` power switch that turns them on.
//!
//! A map's power lights are its layout's (`shared::level`, placed and tuned
//! in the level editor) — lit only on a night map (`MapId::is_dark`): the
//! day version of a place shares its layout but has none on. In `Zombies`
//! they start off: a player pays at the
//! switch — the power lever model, where `shared::power` puts it — and the
//! server sets `Lobby::power_on`, which fades them in for everyone. Every
//! client sees that flip, so for everyone the lever throws (its one
//! animation, once) and the power-on sound plays from it. In the other modes
//! there's no switch, so they're simply on.
//!
//! The lights, the lever and its sound are `StateScoped(InGame)` (the lever
//! remembers whether it's thrown on itself), and the fade level drops back
//! to 0 whenever there are no lights — nothing carries into the next game.

use bevy::audio::SpatialAudioSink;
use bevy::prelude::*;
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
            .add_systems(OnEnter(AppState::InGame), spawn_power_card)
            .add_systems(
                Update,
                (
                    sync_power_lever,
                    fade_power_sound,
                    (sync_machine_hums, fade_machine_hums).chain(),
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
    let wanted: &[PowerLight] = if current.0.is_dark() {
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
                    // Volume is ours to set (`fade_power_sound`), not the
                    // one-shot volume pass's.
                    crate::RemoteSoundEmitter,
                    AudioPlayer::new(sounds.power_on.clone()),
                    // From the lever itself.
                    Transform::from_translation(t.translation),
                    // Starts silent; the real volume is set from the next frame.
                    crate::positional_playback(bevy::audio::Volume::Linear(0.0)),
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

/// Keep the power-on sound's loudness matched to how far the listener is from
/// the lever — the same fade as other players' sounds ("Remote sounds"
/// range) — times the "power on" volume.
fn fade_power_sound(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    sound_vol: Res<crate::SoundVolumes>,
    remote: Res<crate::RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut sounds: Query<(&GlobalTransform, &mut SpatialAudioSink), With<PowerOnSound>>,
) {
    let Ok(ear) = listener.single() else {
        return;
    };
    let ear = ear.translation();
    for (gt, mut sink) in &mut sounds {
        let loudness = sound_vol.power_on * crate::distance_falloff(ear.distance(gt.translation()), &remote);
        sink.set_volume(bevy::audio::Volume::Linear(loudness.max(0.0)) * global_volume.volume);
    }
}

// --- the machines' hum ----------------------------------------------------

/// On a machine (every perk machine, the Pack-a-Punch): once the power's on,
/// it hums — `GameSounds::machine_hum` looped from this offset in its own
/// space. The hum is a child, so it goes when the machine does.
#[derive(Component)]
pub(crate) struct PoweredHum(pub(crate) Vec3);

/// On a [`PoweredHum`] machine once its hum is going.
#[derive(Component)]
struct Humming;

/// A machine's looping hum, and when it started (it swells in).
#[derive(Component)]
struct MachineHum {
    started: f32,
}

/// Panel-tunable machine hum ("Machine hum (Zombies)"): its own short range,
/// so it's only heard near a machine — not the "Remote sounds" range meant
/// for gunfire across the map.
#[derive(Resource, Clone)]
pub(crate) struct MachineHumSettings {
    pub(crate) volume: f32,
    /// Full volume within this far (m) of a machine...
    pub(crate) full_distance: f32,
    /// ...fading to silence here.
    pub(crate) max_distance: f32,
    /// The fade's curve between them (1 = linear; higher drops off sooner).
    pub(crate) falloff: f32,
    /// Seconds a hum takes to swell to full once the power comes on.
    pub(crate) fade_in_secs: f32,
}

impl Default for MachineHumSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            full_distance: 2.0,
            max_distance: 10.0,
            falloff: 2.0,
            fade_in_secs: 1.5,
        }
    }
}

impl MachineHumSettings {
    /// Loudness (before the swell and global volume) `distance` m away.
    fn loudness(&self, distance: f32) -> f32 {
        let span = (self.max_distance - self.full_distance).max(1e-3);
        let t = ((distance - self.full_distance) / span).clamp(0.0, 1.0);
        self.volume * (1.0 - t).powf(self.falloff.max(0.01))
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
    let powered = zombies_game(&local, &lobbies)
        .is_some_and(|l| shared::power::has_power(l.map, l.power_on) || light_settings.force_on);
    if !powered {
        return;
    }
    for (machine, hum) in &machines {
        commands.entity(machine).insert(Humming);
        commands.spawn((
            MachineHum {
                started: time.elapsed_secs(),
            },
            // Volume is ours to set (`fade_machine_hums`), not the one-shot
            // volume pass's.
            crate::RemoteSoundEmitter,
            AudioPlayer::new(sounds.machine_hum.clone()),
            Transform::from_translation(hum.0),
            // Looped; starts silent, the real volume set from the next frame.
            PlaybackSettings::LOOP
                .with_spatial(true)
                .with_spatial_scale(bevy::audio::SpatialScale::new(0.01))
                .with_volume(bevy::audio::Volume::Linear(0.0)),
            ChildOf(machine),
        ));
    }
}

/// Keep each hum's loudness matched to how far the listener is from its
/// machine ([`MachineHumSettings`]), swelling in as it starts.
fn fade_machine_hums(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    settings: Res<MachineHumSettings>,
    global_volume: Res<GlobalVolume>,
    time: Res<Time>,
    mut hums: Query<(&GlobalTransform, &MachineHum, &mut SpatialAudioSink)>,
) {
    let Ok(ear) = listener.single() else {
        return;
    };
    let ear = ear.translation();
    for (gt, hum, mut sink) in &mut hums {
        let swell = if settings.fade_in_secs > 0.0 {
            ((time.elapsed_secs() - hum.started) / settings.fade_in_secs).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let loudness = swell * settings.loudness(ear.distance(gt.translation()));
        sink.set_volume(bevy::audio::Volume::Linear(loudness.max(0.0)) * global_volume.volume);
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
