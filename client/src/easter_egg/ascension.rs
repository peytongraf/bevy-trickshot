//! The squad's souls go up into the pyramid ([`EggStep::Ascension`] — its
//! timings are `shared::easter_egg`'s `ASCEND_*`; the server leaves each
//! body where it stood, `EasterEgg::bodies`):
//!
//! 1. we slump where we stand — the view sinking toward the floor, tipping
//!    over, darkening ([`EggSoulView`], layered in by `weapons::camera_shake`);
//! 2. our soul leaves the body — rising out of it (`float_up.mp3`), the
//!    world washed out and pale, the gun gone (`revive::apply_down_view`) —
//!    while the body crumples below, as every player's does on every client
//!    ([`SoulBodyVisual`]: `soldier.glb`'s `death`, played once, held);
//! 3. it glides up over the pyramid's apex and sinks down through it into
//!    the chamber, onto our spot (`shared::easter_egg::pyramid_spot`) — his
//!    spirit making its last scream from the altar as we go
//!    (`pyramid_fade.mp3`, from `PYRAMID_FADE_AT_SECS`), its glow dying with
//!    it, so he's gone just as we arrive;
//! 4. there we're free to walk about (no weapon) in the chamber's hum
//!    (`chamber_ambience.mp3`).
//!
//! Until we've arrived we're locked (`menu::game_active`), the weapon the
//! whole trip (`menu::weapon_active`). The souls come back down when the
//! egg moves on (taking the wonder weapon — still to come; the debug panel's
//! "Complete" jump does it now): a white flash and we're back in our body,
//! where it lay. A player who's bled out and watching stays as they are.
//!
//! Everything's per game: the clock ([`EggSoul`]), the view's offset and
//! the overlay are reset on leaving it; the bodies, sounds and the altar's
//! glow are `StateScoped(InGame)` and gone once the egg's moved on. The
//! sounds are placeholders at their final paths
//! (`audio/zombies/easter_egg/pyramid/`). Tuned in the debug panel's "Easter
//! egg (Der Tempel)" → "Into the pyramid".

use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::easter_egg::{
    ascend_arrive_secs, EggStep, ASCEND_DESCEND_SECS, ASCEND_GLIDE_SECS, ASCEND_RISE_SECS, ASCEND_SLUMP_SECS,
    PYRAMID_FADE_AT_SECS, PYRAMID_FADE_SECS,
};
use shared::Lobby;

use crate::audio_engine::prelude::*;
use crate::net::GameClient;
use crate::player::{PlayerHead, PlayerPhysics};
use crate::{AppState, Player, EYE_HEIGHT};

use super::egg_game;

pub(super) struct AscensionPlugin;

impl Plugin for AscensionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggSoul>()
            .init_resource::<EggSoulView>()
            .init_resource::<EggSoulSettings>()
            .add_systems(Startup, load_sounds)
            .add_systems(OnEnter(AppState::InGame), spawn_overlay)
            .add_systems(
                OnExit(AppState::InGame),
                |mut soul: ResMut<EggSoul>, mut view: ResMut<EggSoulView>| {
                    *soul = default();
                    *view = default();
                },
            )
            .add_systems(
                Update,
                (run_soul, sync_bodies, soul_sounds, update_overlay)
                    .chain()
                    .run_if(in_state(AppState::InGame))
                    .before(crate::camera_shake),
            );
    }
}

/// Our soul's trip, this time up.
#[derive(Resource, Default)]
pub(crate) struct EggSoul {
    /// How far (s) into it we are — `None` unless the squad's up there.
    /// Held while the game's paused.
    t: Option<f32>,
    /// Whether ours is going (not if we'd bled out, watching).
    going: bool,
    /// Where our body was left (feet, yaw), where our eye was as we slumped,
    /// and the way we faced.
    body: Option<(Vec3, f32)>,
    from_eye: Vec3,
    from_yaw: f32,
    /// Our spot in the chamber (feet, yaw), and where the souls cross the
    /// apex, once known.
    path: Option<(Vec3, f32, Vec3)>,
    /// We're in the chamber, free.
    arrived: bool,
    /// The sounds played this time.
    floated: bool,
    faded: bool,
    /// Back in our body: how long (s) since — the white flash fading.
    back_for: Option<f32>,
}

impl EggSoul {
    /// Locked in place: slumping, or our soul on its way up.
    pub(crate) fn locked(&self) -> bool {
        self.going && self.t.is_some() && !self.arrived
    }

    /// Our soul's out of our body — no weapon, the world washed out.
    pub(crate) fn out(&self) -> bool {
        self.going && self.t.is_some()
    }
}

/// The camera's offset as we slump — `weapons::camera_shake` layers it in.
#[derive(Resource, Default)]
pub(crate) struct EggSoulView(pub(crate) Transform);

/// Panel-tunable trip ("Into the pyramid").
#[derive(Resource, Clone, Debug)]
pub(crate) struct EggSoulSettings {
    /// How far (m) the view sinks as we slump, and how far (°) it tips.
    pub(crate) slump_drop: f32,
    pub(crate) slump_roll_deg: f32,
    /// How far (m) the soul rises out of the body, how high (m) over the
    /// chamber's spots it crosses the apex, and how high (m) its glide bows.
    pub(crate) rise_height: f32,
    pub(crate) apex_height: f32,
    pub(crate) glide_arc: f32,
    /// How the world looks while we're out: its colour left (0 grey … 1 as
    /// it is), and a pale tint over it (sRGB, and how strong).
    pub(crate) saturation: f32,
    pub(crate) tint: [f32; 3],
    pub(crate) tint_alpha: f32,
    /// How dark the view goes as we slump (0..1), and how long (s) the
    /// white flash back in our body takes to fade.
    pub(crate) slump_dark: f32,
    pub(crate) flash_secs: f32,
    /// The sounds' volumes (linear, before the mix).
    pub(crate) float_volume: f32,
    pub(crate) ambience_volume: f32,
    pub(crate) scream_volume: f32,
    /// His spirit's glow over the altar as it fades (lumens), and its colour
    /// (linear).
    pub(crate) fade_light: f32,
    pub(crate) fade_color: [f32; 3],
}

impl Default for EggSoulSettings {
    fn default() -> Self {
        Self {
            slump_drop: 1.2,
            slump_roll_deg: 28.0,
            rise_height: 2.5,
            apex_height: 14.0,
            glide_arc: 6.0,
            saturation: 0.35,
            tint: [0.75, 0.9, 1.0],
            tint_alpha: 0.12,
            slump_dark: 0.75,
            flash_secs: 1.4,
            float_volume: 1.0,
            ambience_volume: 0.6,
            scream_volume: 2.0,
            fade_light: 400_000.0,
            fade_color: [0.4, 1.0, 0.45],
        }
    }
}

#[derive(Resource)]
struct SoulSounds {
    float_up: Handle<AudioSource>,
    ambience: Handle<AudioSource>,
    scream: Handle<AudioSource>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(SoulSounds {
        float_up: asset_server.load("audio/zombies/easter_egg/pyramid/float_up.mp3"),
        ambience: asset_server.load("audio/zombies/easter_egg/pyramid/chamber_ambience.mp3"),
        scream: asset_server.load("audio/zombies/easter_egg/hierophant/voice/pyramid_fade.mp3"),
    });
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Where our eye is `t` s in, once our soul's out — up out of the body,
/// gliding over the apex, sinking through it onto our spot — and the
/// (yaw, pitch) it wants to look.
fn soul_at(t: f32, soul: &EggSoul, s: &EggSoulSettings) -> Option<(Vec3, f32, f32)> {
    let (spot, spot_yaw, apex) = soul.path?;
    let start = soul.from_eye - Vec3::Y * s.slump_drop;
    let risen = start + Vec3::Y * s.rise_height;
    let spot_eye = spot + Vec3::Y * EYE_HEIGHT;
    let glide_from = ASCEND_SLUMP_SECS + ASCEND_RISE_SECS;
    let descend_from = glide_from + ASCEND_GLIDE_SECS;
    let toward = |to: Vec3, from: Vec3| {
        let d = to - from;
        if d.with_y(0.0).length() < 0.1 { soul.from_yaw } else { f32::atan2(-d.x, -d.z) }
    };
    if t < glide_from {
        let x = ((t - ASCEND_SLUMP_SECS) / ASCEND_RISE_SECS).clamp(0.0, 1.0);
        let up = 1.0 - (1.0 - x).powi(2);
        return Some((start.lerp(risen, up), soul.from_yaw, 15f32.to_radians()));
    }
    if t < descend_from {
        let u = ease((t - glide_from) / ASCEND_GLIDE_SECS);
        let mid = risen.lerp(apex, 0.5) + Vec3::Y * s.glide_arc;
        let at = risen.lerp(mid, u).lerp(mid.lerp(apex, u), u);
        return Some((at, toward(apex, risen), (-10f32).to_radians()));
    }
    let x = ((t - descend_from) / ASCEND_DESCEND_SECS).clamp(0.0, 1.0);
    // (Looking down into it, then up as we land.)
    let pitch = -55f32.to_radians() * (1.0 - ease((x - 0.55) / 0.45));
    Some((apex.lerp(spot_eye, ease(x)), spot_yaw, pitch))
}

/// Our soul's trip: slumping, out of the body and up into the chamber —
/// and back down into the body once it's over.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_soul(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    settings: Res<EggSoulSettings>,
    spectate: Res<crate::revive::Spectate>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut soul: ResMut<EggSoul>,
    mut view: ResMut<EggSoulView>,
    player: Single<(&mut Transform, &mut PlayerPhysics), (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    let s = &*settings;
    let dt = time.delta_secs();
    let (mut tf, mut physics) = player.into_inner();
    let lobby = egg_game(&local, &lobbies);
    let up = lobby.is_some_and(|l| l.easter_egg.step == EggStep::Ascension);
    if let Some(back) = soul.back_for.as_mut() {
        *back += dt;
        if *back >= s.flash_secs {
            soul.back_for = None;
        }
    }
    if !up {
        if soul.t.is_some() {
            // Back down: into our body, where it lay.
            if let Some((feet, yaw)) = soul.body.filter(|_| soul.going) {
                tf.translation = feet + Vec3::Y * EYE_HEIGHT;
                tf.rotation = Quat::from_rotation_y(yaw);
                head.rotation = Quat::IDENTITY;
                *physics = PlayerPhysics::default();
                soul.back_for = Some(0.0);
            }
            *soul = EggSoul { back_for: soul.back_for, ..default() };
        }
        if view.0 != Transform::IDENTITY {
            view.0 = Transform::IDENTITY;
        }
        return;
    }
    let Some(lobby) = lobby else { return };
    if soul.t.is_none() {
        let yaw = tf.rotation.to_euler(EulerRot::YXZ).0;
        *soul = EggSoul {
            t: Some(0.0),
            going: !spectate.active(),
            body: Some((tf.translation - Vec3::Y * EYE_HEIGHT, yaw)),
            from_eye: tf.translation,
            from_yaw: yaw,
            ..default()
        };
    }
    let t = soul.t.unwrap_or(0.0) + if paused.0 { 0.0 } else { dt };
    soul.t = Some(t);
    if !soul.going {
        return;
    }
    // Our spot, once the server's said where the bodies are (who's where).
    if soul.path.is_none() {
        let me = local.iter().next().map(|l| l.0);
        let bodies = &lobby.easter_egg.bodies;
        if let Some(i) = bodies.iter().position(|b| Some(b.peer) == me) {
            let b = bodies[i];
            soul.body = Some((b.feet, b.yaw));
            let spots = &shared::level::layout(lobby.map).pyramid_spots;
            if let Some((spot, yaw)) = shared::easter_egg::pyramid_spot(lobby.map, i) {
                let middle = spots.iter().map(|p| p.pos).sum::<Vec3>() / spots.len().max(1) as f32;
                let top = spots.iter().map(|p| p.pos.y).fold(f32::NEG_INFINITY, f32::max);
                soul.path = Some((spot, yaw, middle.with_y(top + s.apex_height)));
            }
        }
    }
    if soul.arrived {
        return;
    }
    if t < ASCEND_SLUMP_SECS {
        // Slumping where we stand: sinking, tipping over, looking level.
        let k = ease(t / ASCEND_SLUMP_SECS);
        view.0 = Transform::from_translation(Vec3::NEG_Y * s.slump_drop * k)
            .with_rotation(Quat::from_rotation_z(s.slump_roll_deg.to_radians() * k));
        head.rotation = head.rotation.slerp(Quat::IDENTITY, (dt * 5.0).min(1.0));
        return;
    }
    if soul.path.is_none() {
        // (Nowhere to go — no spots on this map, or the bodies not in yet:
        // a moment longer, then just back up where we are.)
        if t >= ascend_arrive_secs() {
            soul.arrived = true;
            view.0 = Transform::IDENTITY;
        }
        return;
    }
    if t >= ascend_arrive_secs() {
        // In the chamber, on our spot: free.
        if let Some((spot, yaw, _)) = soul.path {
            tf.translation = spot + Vec3::Y * EYE_HEIGHT;
            tf.rotation = Quat::from_rotation_y(yaw);
        }
        head.rotation = Quat::IDENTITY;
        *physics = PlayerPhysics::default();
        view.0 = Transform::IDENTITY;
        soul.arrived = true;
        return;
    }
    let Some((at, yaw, pitch)) = soul_at(t, &soul, s) else { return };
    // (The slump's tip easing off as we rise.)
    let tip = 1.0 - ease((t - ASCEND_SLUMP_SECS) / 0.7);
    view.0 = Transform::from_rotation(Quat::from_rotation_z(s.slump_roll_deg.to_radians() * tip));
    tf.translation = at;
    let turn = (dt * 2.5).min(1.0);
    tf.rotation = tf.rotation.slerp(Quat::from_rotation_y(yaw), turn);
    head.rotation = head.rotation.slerp(Quat::from_rotation_x(pitch), turn);
    *physics = PlayerPhysics::default();
}

// --- the bodies left below -------------------------------------------------------------

/// A player's body, slumped where they left it (`peer`'s).
#[derive(Component)]
struct SoulBodyVisual {
    peer: lightyear::prelude::PeerId,
    /// Its `AnimationPlayer`, once its scene's in, and whether it's fallen.
    rig: Option<Entity>,
    fallen: bool,
}

/// Every body left below, there while the souls are up — seen once they've
/// left them, crumpling (`death`, once, held).
#[allow(clippy::too_many_arguments)]
fn sync_bodies(
    soul: Res<EggSoul>,
    asset_server: Res<AssetServer>,
    avatar: Res<crate::RemoteAvatarSettings>,
    anims: Res<crate::SoldierAnimations>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut bodies: Query<(Entity, &mut SoulBodyVisual, &mut Visibility)>,
    mut players: Query<&mut AnimationPlayer>,
    mut commands: Commands,
) {
    let wanted = egg_game(&local, &lobbies)
        .filter(|l| l.easter_egg.step == EggStep::Ascension)
        .map(|l| l.easter_egg.bodies.as_slice())
        .unwrap_or(&[]);
    for (e, body, _) in &bodies {
        if !wanted.iter().any(|b| b.peer == body.peer) {
            commands.entity(e).despawn();
        }
    }
    for b in wanted {
        if bodies.iter().any(|(_, v, _)| v.peer == b.peer) {
            continue;
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                SoulBodyVisual { peer: b.peer, rig: None, fallen: false },
                // (`soldier.glb` faces +Z — the rig's yaw is for -Z.)
                Transform::from_translation(b.feet)
                    .with_rotation(Quat::from_rotation_y(b.yaw + std::f32::consts::PI))
                    .with_scale(Vec3::splat(avatar.scale)),
                Visibility::Hidden,
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/characters/soldier.glb"))),
            ))
            .observe(body_ready);
    }
    let shown = soul.t.is_some_and(|t| t >= ASCEND_SLUMP_SECS);
    for (_, mut body, mut vis) in &mut bodies {
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        let Some(rig) = body.rig.filter(|_| shown && !body.fallen) else { continue };
        let Ok(mut player) = players.get_mut(rig) else { continue };
        player.stop_all();
        player.play(anims.death());
        body.fallen = true;
    }
}

/// A body's scene is in: animate it with the soldier's clips.
fn body_ready(
    trigger: Trigger<SceneInstanceReady>,
    anims: Res<crate::SoldierAnimations>,
    children: Query<&Children>,
    players: Query<(), With<AnimationPlayer>>,
    mut bodies: Query<&mut SoulBodyVisual>,
    mut bounds: crate::CharacterBounds,
    mut commands: Commands,
) {
    let root = trigger.target();
    for e in children.iter_descendants(root) {
        bounds.fit(&mut commands, root, e);
        if players.contains(e) {
            commands.entity(e).insert(AnimationGraphHandle(anims.graph().clone()));
            if let Ok(mut body) = bodies.get_mut(root) {
                body.rig = Some(e);
            }
        }
    }
}

// --- its sounds, and his last scream ------------------------------------------------------

#[derive(Component)]
struct ChamberAmbience;

/// His spirit's glow over the chamber's altar as he screams his last.
#[derive(Component)]
struct FadingSpirit {
    start: f32,
}

/// The float up as we leave our body, the chamber's hum while we're in it,
/// and his last scream from its altar.
#[allow(clippy::too_many_arguments)]
fn soul_sounds(
    mut soul: ResMut<EggSoul>,
    settings: Res<EggSoulSettings>,
    sounds: Res<SoulSounds>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    ambience: Query<Entity, With<ChamberAmbience>>,
    mut spirit: Query<(Entity, &FadingSpirit, &mut PointLight)>,
    mut commands: Commands,
) {
    let s = &*settings;
    let Some(t) = soul.t.filter(|_| soul.going) else {
        for e in &ambience {
            commands.entity(e).despawn();
        }
        for (e, ..) in &spirit {
            commands.entity(e).despawn();
        }
        return;
    };
    if t >= ASCEND_SLUMP_SECS && !soul.floated {
        soul.floated = true;
        commands.spawn((
            StateScoped(AppState::InGame),
            AudioPlayer::new(sounds.float_up.clone()),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(s.float_volume)),
        ));
    }
    if soul.arrived && ambience.is_empty() {
        commands.spawn((
            StateScoped(AppState::InGame),
            ChamberAmbience,
            AudioPlayer::new(sounds.ambience.clone()),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(s.ambience_volume)),
        ));
    }
    let scream_at = PYRAMID_FADE_AT_SECS;
    if t >= scream_at && !soul.faded {
        soul.faded = true;
        let map = egg_game(&local, &lobbies).map(|l| l.map);
        let altar = map.and_then(|m| shared::level::layout(m).pyramid_altar).map(|a| a.pos);
        let at = altar.or(soul.path.map(|p| p.0)).map(|p| p + Vec3::Y * 1.5);
        if let Some(at) = at {
            let c = s.fade_color;
            commands
                .spawn((
                    StateScoped(AppState::InGame),
                    FadingSpirit { start: t },
                    PointLight {
                        color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
                        intensity: s.fade_light,
                        range: 14.0,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(at),
                ))
                .with_child((
                    AudioPlayer::new(sounds.scream.clone()),
                    Transform::default(),
                    crate::positional_playback(Volume::Linear(s.scream_volume)),
                ));
        }
    }
    // His glow flickering out with his scream.
    for (e, fade, mut light) in &mut spirit {
        let x = (t - fade.start) / PYRAMID_FADE_SECS;
        if x >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        let flicker = 0.75 + 0.25 * (t * 31.0).sin() * (t * 7.0).cos();
        light.intensity = s.fade_light * (1.0 - x).powi(2) * flicker;
    }
}

// --- the overlay ------------------------------------------------------------------------

#[derive(Component)]
struct SoulOverlay;

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        StateScoped(AppState::InGame),
        SoulOverlay,
        GlobalZIndex(3),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
        Pickable::IGNORE,
    ));
}

/// Dark as we slump, a pale wash while we're out, a white flash back in our
/// body — clear otherwise.
fn update_overlay(
    soul: Res<EggSoul>,
    settings: Res<EggSoulSettings>,
    mut overlay: Single<&mut BackgroundColor, With<SoulOverlay>>,
) {
    let s = &*settings;
    let [r, g, b] = s.tint;
    let want = if let Some(back) = soul.back_for {
        Color::srgba(1.0, 1.0, 1.0, (1.0 - back / s.flash_secs.max(0.01)).clamp(0.0, 1.0).powf(1.5))
    } else if let Some(t) = soul.t.filter(|_| soul.going) {
        if t < ASCEND_SLUMP_SECS {
            Color::srgba(0.0, 0.0, 0.0, s.slump_dark * ease(t / ASCEND_SLUMP_SECS))
        } else {
            // (The dark lifting as we rise, into the pale.)
            let k = ease((t - ASCEND_SLUMP_SECS) / 0.8);
            let dark = s.slump_dark * (1.0 - k);
            let pale = s.tint_alpha * k;
            let a = dark + pale;
            let w = if a > 0.0 { pale / a } else { 1.0 };
            Color::srgba(r * w, g * w, b * w, a)
        }
    } else {
        Color::NONE
    };
    if overlay.0 != want {
        overlay.0 = want;
    }
}

// --- the debug panel --------------------------------------------------------------------

/// The debug panel's controls for it.
pub(crate) fn ascension_section(ui: &mut egui::Ui, s: &mut EggSoulSettings, soul: &EggSoul) {
    match soul.t {
        Some(t) if soul.arrived => ui.label(format!("now: in the chamber ({t:.1} s in)")),
        Some(t) => ui.label(format!("now: {t:.1} s in (there at {:.1} s)", ascend_arrive_secs())),
        None => ui.label("now: not up there — jump to \"Into the pyramid\" (\"Complete\" brings you back down)"),
    };
    ui.add(egui::Slider::new(&mut s.slump_drop, 0.0f32..=1.6).text("slump: view sinks (m)"));
    ui.add(egui::Slider::new(&mut s.slump_roll_deg, 0.0f32..=90.0).text("slump: tips (°)"));
    ui.add(egui::Slider::new(&mut s.slump_dark, 0.0f32..=1.0).text("slump: darkens"));
    ui.add(egui::Slider::new(&mut s.rise_height, 0.0f32..=8.0).text("rises out of the body (m)"));
    ui.add(egui::Slider::new(&mut s.apex_height, 1.0f32..=40.0).text("crosses the apex this high over the spots (m, next time)"));
    ui.add(egui::Slider::new(&mut s.glide_arc, 0.0f32..=30.0).text("glide's arc (m)"));
    ui.add(egui::Slider::new(&mut s.saturation, 0.0f32..=1.0).text("colour left while out"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.tint);
        ui.label("pale tint");
    });
    ui.add(egui::Slider::new(&mut s.tint_alpha, 0.0f32..=0.6).text("tint strength"));
    ui.add(egui::Slider::new(&mut s.flash_secs, 0.1f32..=5.0).text("white flash back (s)"));
    ui.add(egui::Slider::new(&mut s.float_volume, 0.0f32..=3.0).text("float-up volume"));
    ui.add(egui::Slider::new(&mut s.ambience_volume, 0.0f32..=3.0).text("chamber ambience volume"));
    ui.add(egui::Slider::new(&mut s.scream_volume, 0.0f32..=3.0).text("his last scream's volume"));
    ui.add(egui::Slider::new(&mut s.fade_light, 0.0f32..=3_000_000.0).text("his fading glow (lm)"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.fade_color);
        ui.label("his glow's colour");
    });
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = EggSoulSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
