//! The easter egg's boss fight on this client ([`EggStep::BossFight`] — the
//! server's `easter_egg::fight` runs him):
//!
//! * **Him** — for now the boss's model and animations (`crate::boss`, his
//!   pose's `Boss*` states), until his own are made; his aura's a sickly
//!   spirit-green rather than a boss's orange ([`crate::boss::BossTint`]),
//!   and he's no boss's growls or death roar ([`crate::boss::BossVoiced`]):
//!   his own voice instead.
//! * **His voice** — [`shared::HierophantSays`]: the line, from where he
//!   stands (following him), heard far off (`mix.ron`'s "Hierophant
//!   voice"). A phase or death line cuts off whatever he was saying.
//! * **His teleport** (phase 2) — [`shared::HierophantTeleport`]: green
//!   smoke boils up where he stands and he sinks through the floor
//!   (`teleport_out.mp3`); a moment later it boils up near a player and he
//!   rises out of the ground there (`teleport_in.mp3`). His avatar's sunk by
//!   [`Sinking`], after it's put at his pose.
//! * **Raising the dead** (phase 2) — [`shared::HierophantRaise`]: a wide
//!   green burst round him and `raise_dead.mp3`; the zombies rise as any do.
//! * **His health bar** — top centre, under the compass: his name over a
//!   long bar of what's left of the health he came in with
//!   (`EasterEgg::hierophant`), a paler chunk trailing behind each hit.
//!   Up while he's alive in the fight, hidden behind menus and during a
//!   kill cam like the rest of the HUD.
//!
//! The bar's `StateScoped(InGame)`, hidden (and its trail reset) whenever
//! there's no fight; his lines go with his avatar, and the bursts and their
//! sounds are `StateScoped(InGame)` and gone once they've played out.
//! Tuned in the debug panel's "Easter egg (Der Tempel)" → "The fight".

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy_egui::egui;
use lightyear::prelude::{Interpolated, LocalId, MessageReceiver, TriggerSender};
use shared::boss::BOSS_HEIGHT;
use shared::easter_egg::{
    EggStep, Hierophant, HierophantDebugAction, TELEPORT_RISE_SECS, TELEPORT_SINK_SECS, TELEPORT_UNDER_SECS,
};
use shared::{Lobby, PlayerHealth, PlayerId};

use crate::audio_engine::prelude::*;
use crate::boss::{BossTint, BossVisual, BossVoiced};
use crate::net::{GameClient, RemoteAvatar};
use crate::vfx::{smoke_billboard_rotation, ExplosionAssets};
use crate::{killcam, menu, AppState, WorldModelCamera, HUD_FONT};

use super::egg_game;

pub(super) struct FightPlugin;

impl Plugin for FightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggFightSettings>()
            .add_systems(OnEnter(AppState::InGame), spawn_bar)
            .add_systems(
                Update,
                (
                    mark_him,
                    (his_lines, receive_teleports, receive_raises),
                    sink_him.after(crate::net::follow_remote_avatars),
                    run_bursts,
                    update_bar,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Panel-tunable fight ("The fight").
#[derive(Resource, Clone, Debug)]
pub(crate) struct EggFightSettings {
    /// His lines' volume (linear, before the mix).
    pub(crate) voice_volume: f32,
    /// How high (m) over his feet they come from.
    pub(crate) voice_height: f32,
    /// The teleport's and raising's sounds' volume.
    pub(crate) fx_volume: f32,
    /// His aura's colour (linear) — a boss's is orange — and his bursts'.
    pub(crate) aura_color: [f32; 3],
    /// A teleport's burst: how wide (m) its glow on the ground, how many
    /// puffs of smoke, how bright its light (lumens) and how bright the
    /// glow.
    pub(crate) burst_size: f32,
    pub(crate) burst_puffs: u32,
    pub(crate) burst_light: f32,
    pub(crate) burst_glow: f32,
    /// Raising the dead's burst, this much wider.
    pub(crate) raise_scale: f32,
    /// The health bar's width (px).
    pub(crate) bar_width: f32,
    /// How fast (of the bar a second) the paler chunk behind a hit drains,
    /// after holding this long (s).
    pub(crate) trail_speed: f32,
    pub(crate) trail_hold_secs: f32,
}

impl Default for EggFightSettings {
    fn default() -> Self {
        Self {
            voice_volume: 2.0,
            voice_height: 2.3,
            fx_volume: 1.5,
            aura_color: [0.35, 1.0, 0.32],
            burst_size: 2.6,
            burst_puffs: 14,
            burst_light: 400_000.0,
            burst_glow: 2.5,
            raise_scale: 2.4,
            bar_width: 560.0,
            trail_speed: 0.35,
            trail_hold_secs: 0.5,
        }
    }
}

const DIR: &str = "audio/zombies/easter_egg/hierophant";
/// His spoken lines (made from the clean recordings in
/// `client/voice_src/hierophant/` by `client/tools/voice_layers.sh`).
const VOICE_DIR: &str = "audio/zombies/easter_egg/hierophant/voice";

/// Him, while he's in the fight: who he is.
fn in_fight(lobby: Option<&Lobby>) -> Option<Hierophant> {
    lobby
        .filter(|l| l.easter_egg.step == EggStep::BossFight)
        .and_then(|l| l.easter_egg.hierophant)
}

/// His avatar, if it's here.
fn his_avatar(
    him: Hierophant,
    avatars: &Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    ids: &Query<&PlayerId>,
    interp: &Query<&Interpolated>,
) -> Option<Entity> {
    avatars
        .iter()
        .find(|(_, a)| crate::boss::peer_of(a.src, ids, interp) == Some(him.peer))
        .map(|(e, _)| e)
}

/// His avatar gets his colour and his own voice (whoever he was — a corpse
/// keeps them).
#[allow(clippy::too_many_arguments)]
fn mark_him(
    settings: Res<EggFightSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    mut tints: Query<&mut BossTint>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut commands: Commands,
) {
    let Some(him) = egg_game(&local, &lobbies).and_then(|l| l.easter_egg.hierophant) else {
        return;
    };
    let Some(avatar) = his_avatar(him, &avatars, &ids, &interp) else {
        return;
    };
    match tints.get_mut(avatar) {
        Ok(mut tint) => {
            if tint.0 != settings.aura_color {
                tint.0 = settings.aura_color;
            }
        }
        Err(_) => {
            commands.entity(avatar).insert((BossTint(settings.aura_color), BossVoiced));
        }
    }
}

/// One of his lines playing.
#[derive(Component)]
struct HisLine;

/// His lines, from where he stands.
#[allow(clippy::too_many_arguments)]
fn his_lines(
    settings: Res<EggFightSettings>,
    asset_server: Res<AssetServer>,
    mut receivers: Query<&mut MessageReceiver<shared::HierophantSays>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    scales: Query<&Transform, With<BossVisual>>,
    playing: Query<Entity, With<HisLine>>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut commands: Commands,
) {
    // (His death comes as he's no longer in the fight: whoever he was.)
    let him = egg_game(&local, &lobbies).and_then(|l| l.easter_egg.hierophant);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let Some(him) = him else { continue };
            let Some(avatar) = his_avatar(him, &avatars, &ids, &interp) else {
                continue;
            };
            if msg.line.interrupts() {
                for e in &playing {
                    commands.entity(e).try_despawn();
                }
            }
            let clip: Handle<AudioSource> = asset_server.load(format!("{VOICE_DIR}/{}", msg.line.file()));
            // (Under his avatar, which is scaled.)
            let scale = scales.get(avatar).map_or(1.0, |t| t.scale.y.max(0.001));
            commands.entity(avatar).with_child((
                HisLine,
                AudioPlayer::new(clip),
                Transform::from_xyz(0.0, settings.voice_height / scale, 0.0),
                crate::positional_playback(Volume::Linear(settings.voice_volume)),
            ));
        }
    }
}

// --- his teleport and raising the dead ---------------------------------------------------

/// On his avatar mid-teleport: when it began (`Time::elapsed_secs`), and
/// where he sinks and rises.
#[derive(Component)]
struct Sinking {
    start: f32,
    from: Vec3,
    to: Vec3,
}

/// How far (m) down he goes — all of him under the floor.
const UNDER_DEPTH: f32 = BOSS_HEIGHT * 1.1;

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A burst of green smoke and light on the ground at its root: from
/// `start` (`Time::elapsed_secs`, hidden till then — its sound playing
/// then), for `life` s, `size` m across.
#[derive(Component)]
struct Burst {
    start: f32,
    life: f32,
    size: f32,
    sound: Option<Handle<AudioSource>>,
    glow: Handle<StandardMaterial>,
    smoke: Handle<StandardMaterial>,
}

#[derive(Component, Clone, Copy)]
enum BurstPart {
    Glow,
    Light,
    /// A puff: where it starts (m, round the middle) and how fast it climbs.
    Puff { at: Vec3, rise: f32, spin: f32 },
}

#[allow(clippy::too_many_arguments)]
fn spawn_burst(
    commands: &mut Commands,
    fx: &ExplosionAssets,
    materials: &mut Assets<StandardMaterial>,
    s: &EggFightSettings,
    at: Vec3,
    start: f32,
    life: f32,
    size: f32,
    sound: Option<Handle<AudioSource>>,
    seed: u32,
) {
    let c = s.aura_color;
    let glow = materials.add(StandardMaterial {
        base_color: LinearRgba::rgb(0.0, 0.0, 0.0).into(),
        base_color_texture: Some(fx.glow.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let smoke = materials.add(StandardMaterial {
        base_color: Color::srgba(c[0] * 0.25, c[1] * 0.35, c[2] * 0.25, 0.0),
        base_color_texture: Some(fx.smoke.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let roll = |k: u32| crate::util::rand01(seed.wrapping_mul(2_654_435_761).wrapping_add(k));
    commands
        .spawn((
            StateScoped(AppState::InGame),
            Burst { start, life, size, sound, glow: glow.clone(), smoke: smoke.clone() },
            Transform::from_translation(at),
            Visibility::default(),
        ))
        .with_children(|b| {
            b.spawn((
                BurstPart::Glow,
                Mesh3d(fx.quad.clone()),
                MeshMaterial3d(glow),
                Transform::from_scale(Vec3::ZERO),
                NotShadowCaster,
                NoFrustumCulling,
            ));
            b.spawn((
                BurstPart::Light,
                PointLight {
                    color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
                    intensity: 0.0,
                    range: size * 5.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, 0.6, 0.0),
            ));
            for i in 0..s.burst_puffs {
                let a = roll(i * 4) * std::f32::consts::TAU;
                let r = roll(i * 4 + 1).sqrt() * size * 0.4;
                b.spawn((
                    BurstPart::Puff {
                        at: Vec3::new(a.cos() * r, 0.1 + roll(i * 4 + 2) * 0.4, a.sin() * r),
                        rise: 0.6 + roll(i * 4 + 3) * 1.4,
                        spin: (roll(i * 4 + 5) - 0.5) * 2.0,
                    },
                    Mesh3d(fx.quad.clone()),
                    MeshMaterial3d(smoke.clone()),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
            }
        });
}

/// His teleport begun: green smoke where he sinks, then where he rises,
/// each with its sound — and his avatar sunk through the floor meanwhile.
#[allow(clippy::too_many_arguments)]
fn receive_teleports(
    time: Res<Time>,
    settings: Res<EggFightSettings>,
    asset_server: Res<AssetServer>,
    fx: Option<Res<ExplosionAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut receivers: Query<&mut MessageReceiver<shared::HierophantTeleport>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<(Entity, &RemoteAvatar), With<BossVisual>>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let him = in_fight(egg_game(&local, &lobbies));
    let now = time.elapsed_secs();
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let Some(fx) = fx.as_deref() else { continue };
            let (from, to) = (Vec3::from_array(msg.from), Vec3::from_array(msg.to));
            let s = &*settings;
            let out = asset_server.load(format!("{DIR}/teleport_out.mp3"));
            let inn = asset_server.load(format!("{DIR}/teleport_in.mp3"));
            *seq = seq.wrapping_add(2);
            spawn_burst(&mut commands, fx, &mut materials, s, from, now, TELEPORT_SINK_SECS + 1.2, s.burst_size, Some(out), *seq);
            let rise_at = now + TELEPORT_SINK_SECS + TELEPORT_UNDER_SECS;
            spawn_burst(
                &mut commands,
                fx,
                &mut materials,
                s,
                to,
                rise_at - 0.25,
                TELEPORT_RISE_SECS + 1.4,
                s.burst_size,
                Some(inn),
                seq.wrapping_add(1),
            );
            if let Some(avatar) = him.and_then(|h| his_avatar(h, &avatars, &ids, &interp)) {
                commands.entity(avatar).insert(Sinking { start: now, from, to });
            }
        }
    }
}

/// He raises the dead: a wide green burst round him, and its sound.
#[allow(clippy::too_many_arguments)]
fn receive_raises(
    time: Res<Time>,
    settings: Res<EggFightSettings>,
    asset_server: Res<AssetServer>,
    fx: Option<Res<ExplosionAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut receivers: Query<&mut MessageReceiver<shared::HierophantRaise>>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let Some(fx) = fx.as_deref() else { continue };
            let s = &*settings;
            *seq = seq.wrapping_add(1);
            let sound = asset_server.load(format!("{DIR}/raise_dead.mp3"));
            let at = Vec3::from_array(msg.at);
            spawn_burst(&mut commands, fx, &mut materials, s, at, now, 2.8, s.burst_size * s.raise_scale, Some(sound), *seq ^ 0x5eed);
        }
    }
}

/// His avatar mid-teleport: sinking through the floor where he stood, under
/// it, then rising out of it where he comes up (put there outright — his
/// pose catches up while he's under).
fn sink_him(time: Res<Time>, mut avatars: Query<(Entity, &Sinking, &mut Transform)>, mut commands: Commands) {
    let now = time.elapsed_secs();
    for (e, sink, mut tf) in &mut avatars {
        let t = now - sink.start;
        let under_to = TELEPORT_SINK_SECS + TELEPORT_UNDER_SECS;
        if t < TELEPORT_SINK_SECS {
            tf.translation.y -= UNDER_DEPTH * ease(t / TELEPORT_SINK_SECS);
        } else if t < under_to {
            let at = if t < TELEPORT_SINK_SECS + TELEPORT_UNDER_SECS * 0.75 { sink.from } else { sink.to };
            tf.translation = at - Vec3::Y * UNDER_DEPTH;
        } else if t < under_to + TELEPORT_RISE_SECS {
            let up = ease((t - under_to) / TELEPORT_RISE_SECS);
            tf.translation = sink.to - Vec3::Y * UNDER_DEPTH * (1.0 - up);
        } else {
            commands.entity(e).remove::<Sinking>();
        }
    }
}

/// Each burst: its glow swelling on the ground and fading, its light
/// flaring, its smoke boiling up — then gone.
#[allow(clippy::type_complexity)]
fn run_bursts(
    time: Res<Time>,
    settings: Res<EggFightSettings>,
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    mut bursts: Query<(Entity, &mut Burst, &Transform, &Children)>,
    mut parts: Query<(&BurstPart, &mut Transform, Option<&mut PointLight>), Without<Burst>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let s = &*settings;
    let Some(cam) = camera.map(|c| c.into_inner()) else { return };
    let (cam_pos, cam_up, cam_right) = (cam.translation(), cam.up().as_vec3(), cam.right().as_vec3());
    let c = s.aura_color;
    for (e, mut burst, root, children) in &mut bursts {
        let t = now - burst.start;
        if t >= burst.life {
            commands.entity(e).try_despawn();
            continue;
        }
        let p = (t / burst.life).max(0.0);
        if t >= 0.0 {
            if let Some(sound) = burst.sound.take() {
                commands.spawn((
                    StateScoped(AppState::InGame),
                    AudioPlayer::new(sound),
                    Transform::from_translation(root.translation + Vec3::Y),
                    crate::positional_playback(Volume::Linear(s.fx_volume)),
                ));
            }
        }
        // In fast, out slow.
        let level = if t < 0.0 { 0.0 } else { (t / 0.25).min(1.0) * (1.0 - p).powf(1.5) };
        if let Some(m) = materials.get_mut(&burst.glow) {
            let b = s.burst_glow * level;
            m.base_color = LinearRgba::rgb(c[0] * b, c[1] * b, c[2] * b).into();
        }
        if let Some(m) = materials.get_mut(&burst.smoke) {
            m.base_color = Color::srgba(c[0] * 0.2, c[1] * 0.32, c[2] * 0.2, 0.75 * level);
        }
        for child in children.iter() {
            let Ok((part, mut tf, light)) = parts.get_mut(child) else { continue };
            match *part {
                BurstPart::Glow => {
                    let size = burst.size * (0.6 + 0.4 * ease(t / 0.4));
                    *tf = Transform::from_xyz(0.0, 0.04, 0.0)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                        .with_scale(Vec3::new(size, size, 1.0) * if t < 0.0 { 0.0 } else { 1.0 });
                }
                BurstPart::Light => {
                    if let Some(mut light) = light {
                        light.intensity = s.burst_light * level * (1.0 + 0.25 * (now * 23.0).sin());
                    }
                }
                BurstPart::Puff { at, rise, spin } => {
                    if t < 0.0 {
                        tf.scale = Vec3::ZERO;
                        continue;
                    }
                    let local = at + Vec3::Y * rise * t;
                    let world = root.translation + local;
                    let size = burst.size * (0.35 + 0.5 * p);
                    *tf = Transform::from_translation(local)
                        .with_rotation(smoke_billboard_rotation(world, cam_pos, cam_up, cam_right, spin * t))
                        .with_scale(Vec3::new(size, size, 1.0));
                }
            }
        }
    }
}

// --- his health bar ---------------------------------------------------------------------

#[derive(Component)]
struct HealthBar;

#[derive(Component)]
struct BarFill;

#[derive(Component)]
struct BarTrail;

#[derive(Component)]
struct BarFrame;

/// The paler chunk behind a hit: where it's at (0..1), and when it starts
/// draining (`Time::elapsed_secs`).
#[derive(Component, Default)]
struct Trail {
    at: Option<f32>,
    drain_from: f32,
}

const BAR_HEIGHT: f32 = 14.0;
const FILL: Color = Color::srgb(0.62, 0.05, 0.08);
const TRAIL: Color = Color::srgb(0.95, 0.78, 0.55);
const NAME: Color = Color::srgb(0.85, 1.0, 0.82);

fn spawn_bar(mut commands: Commands, asset_server: Res<AssetServer>, settings: Res<EggFightSettings>) {
    let font = asset_server.load(HUD_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            HealthBar,
            Trail::default(),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                // (Under the compass.)
                top: Val::Px(crate::hud::COMPASS_HEIGHT + 10.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|col| {
            col.spawn((
                Text::new("THE HIEROPHANT"),
                TextFont {
                    font,
                    font_size: 26.0,
                    ..default()
                },
                TextColor(NAME),
                TextShadow {
                    offset: Vec2::splat(2.0),
                    color: Color::BLACK.with_alpha(0.85),
                },
            ));
            col.spawn((
                BarFrame,
                Node {
                    width: Val::Px(settings.bar_width),
                    height: Val::Px(BAR_HEIGHT),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
                BorderColor(Color::srgba(0.45, 0.9, 0.42, 0.75)),
            ))
            .with_children(|frame| {
                frame.spawn((
                    BarTrail,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(TRAIL),
                ));
                frame.spawn((
                    BarFill,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(FILL),
                ));
            });
        });
}

/// Up while he's alive in the fight (not behind a menu or during a kill
/// cam): what's left of his health, the chunk behind a hit trailing after.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_bar(
    time: Res<Time>,
    settings: Res<EggFightSettings>,
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    healths: Query<(&PlayerId, &PlayerHealth)>,
    bar: Single<(&mut Visibility, &mut Trail), With<HealthBar>>,
    mut frame: Single<&mut Node, (With<BarFrame>, Without<BarFill>, Without<BarTrail>)>,
    mut fill: Single<&mut Node, (With<BarFill>, Without<BarFrame>, Without<BarTrail>)>,
    mut trail_node: Single<&mut Node, (With<BarTrail>, Without<BarFrame>, Without<BarFill>)>,
) {
    let (mut visibility, mut trail) = bar.into_inner();
    let him = in_fight(egg_game(&local, &lobbies));
    let left = him.and_then(|h| {
        let health = healths.iter().find(|(id, _)| id.0 == h.peer).map(|(_, hp)| hp.0)?;
        Some((health / h.max_health.max(1.0)).clamp(0.0, 1.0))
    });
    let Some(left) = left.filter(|l| *l > 0.0) else {
        visibility.set_if_neq(Visibility::Hidden);
        trail.at = None;
        return;
    };
    let shown = !menu.is_open() && active_killcam.0.is_none();
    visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    let now = time.elapsed_secs();
    let draining = now >= trail.drain_from;
    let at = trail.at.get_or_insert(left);
    if left > *at {
        // (Healed, or a fresh one: no trail.)
        *at = left;
    } else if left < *at && draining {
        *at = (*at - settings.trail_speed * time.delta_secs()).max(left);
    }
    let at = *at;
    // A hit just landed: the trail holds a moment before draining.
    if fill.width != Val::Percent(left * 100.0) {
        if let Val::Percent(was) = fill.width {
            if left * 100.0 < was - 0.01 {
                trail.drain_from = now + settings.trail_hold_secs;
            }
        }
        fill.width = Val::Percent(left * 100.0);
    }
    if trail_node.width != Val::Percent(at * 100.0) {
        trail_node.width = Val::Percent(at * 100.0);
    }
    if frame.width != Val::Px(settings.bar_width) {
        frame.width = Val::Px(settings.bar_width);
    }
}

// --- the debug panel --------------------------------------------------------------------

/// The debug panel's controls for it — and a poke at him to send, if one
/// was clicked (the leader's, `is_leader`).
pub(crate) fn fight_section(
    ui: &mut egui::Ui,
    s: &mut EggFightSettings,
    lobby: Option<&Lobby>,
    is_leader: bool,
) -> Option<HierophantDebugAction> {
    let mut poke = None;
    let fight = in_fight(lobby);
    match fight {
        Some(h) => ui.label(format!(
            "now: he's in, phase {} ({:.0} health to start) — the boss model for now",
            h.phase, h.max_health
        )),
        None => ui.label("now: no fight — jump to \"Boss fight\" to bring him in"),
    };
    ui.add_enabled_ui(fight.is_some() && is_leader, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (label, share) in [("Health 100%", 1.0), ("Health 67% (phase 2)", 0.66), ("Health 34%", 0.34), ("Health 5%", 0.05)] {
                if ui.button(label).clicked() {
                    poke = Some(HierophantDebugAction::SetHealth(share));
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Raise a ring now").clicked() {
                poke = Some(HierophantDebugAction::RaiseRing);
            }
            if ui.button("Teleport now").clicked() {
                poke = Some(HierophantDebugAction::Teleport);
            }
        });
    });
    ui.separator();
    ui.add(egui::Slider::new(&mut s.voice_volume, 0.0f32..=3.0).text("voice volume"));
    ui.add(egui::Slider::new(&mut s.voice_height, 0.0f32..=4.0).text("voice from (m up)"));
    ui.add(egui::Slider::new(&mut s.fx_volume, 0.0f32..=3.0).text("teleport / raise sounds' volume"));
    ui.horizontal(|ui| {
        ui.label("aura & burst colour");
        ui.color_edit_button_rgb(&mut s.aura_color);
    });
    ui.add(egui::Slider::new(&mut s.burst_size, 0.2f32..=10.0).text("teleport burst size (m)"));
    ui.add(egui::Slider::new(&mut s.burst_puffs, 0..=40).text("smoke puffs"));
    ui.add(egui::Slider::new(&mut s.burst_light, 0.0f32..=3_000_000.0).text("burst light (lm)"));
    ui.add(egui::Slider::new(&mut s.burst_glow, 0.0f32..=10.0).text("burst glow"));
    ui.add(egui::Slider::new(&mut s.raise_scale, 0.5f32..=6.0).text("raising's burst (× size)"));
    ui.add(egui::Slider::new(&mut s.bar_width, 120.0f32..=1200.0).text("health bar width (px)"));
    ui.add(egui::Slider::new(&mut s.trail_hold_secs, 0.0f32..=3.0).text("hit trail holds (s)"));
    ui.add(egui::Slider::new(&mut s.trail_speed, 0.01f32..=3.0).text("hit trail drains (of the bar / s)"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = EggFightSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
    poke
}

/// Send the debug panel's poke at him.
pub(crate) fn send_poke(tx: &mut TriggerSender<shared::HierophantDebug>, action: HierophantDebugAction) {
    tx.trigger::<shared::LobbyChannel>(shared::HierophantDebug { action });
}
