//! The Hierophant's death ([`EggStep::HisDeath`] — its timings are
//! `shared::easter_egg`'s `DEATH_*`, the server moving on once it's done):
//!
//! 1. his body falls (the boss's own death, `crate::boss`: its `death` clip,
//!    its aura dying away; his eyes go out, `fight`), and he screams
//!    (`death.mp3`, [`DeathCry`] — it goes with his spirit once it's out);
//! 2. [`DEATH_SPIRIT_AT`] in, his spirit tears up out of the body — a
//!    churning mass of the souls' green smoke (the Exhumer's,
//!    `exhumer::SoulSmoke`: streaks swirling round a dim glow, a haze round
//!    them and a long smoky tail along its path) — the lights stuttering,
//!    and hangs over him, writhing;
//! 3. [`DEATH_FLY_AT`] in, it flies, screaming (`spirit_flee.mp3`, a
//!    placeholder to record over), in an arc up into the pyramid — at its
//!    altar, inside it (`ZombiesLayout::pyramid_altar`; straight up without
//!    one) — gone with a flash and another stutter;
//! 4. once his scream's died away, it's over: the server moves on to
//!    `Ascension`, and the map, the ambience and the round come back as
//!    they were before the ritual (`environment::egg_mood`, `music`).
//!
//! The map stays gone wrong through it (`holds_spawns` counts it); the
//! fight's music, his health bar and phase 3's red go as he falls.
//!
//! The clock ([`EggDeath`]) is reset on leaving the game and whenever the
//! egg's anywhere else, so a debug jump back to "His death" plays it again.
//! The spirit's parts are `StateScoped(InGame)` and gone as soon as it's
//! over. Tuned in the debug panel's "Easter egg (Der Tempel)" → "His death".

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::easter_egg::{
    death_spirit_in_secs, EggStep, DEATH_FLY_AT, DEATH_FLY_SECS, DEATH_SPIRIT_AT, DEATH_SPIRIT_RISE_SECS,
};
use shared::Lobby;

use crate::audio_engine::prelude::*;
use crate::net::GameClient;
use crate::exhumer::{along_rotation, SoulSmoke};
use crate::vfx::{smoke_billboard_rotation, ExplosionAssets};
use crate::{AppState, WorldModelCamera};

use super::{egg_game, ramp};

pub(super) struct DeathPlugin;

impl Plugin for DeathPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggDeath>()
            .init_resource::<EggDeathSettings>()
            .add_systems(OnExit(AppState::InGame), |mut death: ResMut<EggDeath>| *death = default())
            .add_systems(
                Update,
                (track_death, sync_spirit, run_spirit, carry_cry).chain().run_if(in_state(AppState::InGame)),
            );
    }
}

/// How far (s) into his death we are — `None` unless it's playing. Held
/// while the game's paused.
#[derive(Resource, Default)]
pub(crate) struct EggDeath {
    t: Option<f32>,
    /// The lights' stutters and the flee sound, once each this time.
    torn_out: bool,
    fled: bool,
    arrived: bool,
}

/// Panel-tunable death ("His death").
#[derive(Resource, Clone, Debug)]
pub(crate) struct EggDeathSettings {
    /// The spirit's colour (linear), how big (m) its glow, and how bright.
    pub(crate) color: [f32; 3],
    pub(crate) size: f32,
    pub(crate) glow: f32,
    /// How big its smoke is (of `size`) — the streaks swirling round it and
    /// its haze.
    pub(crate) smoke_size: f32,
    /// Its tail: how many streaks, how far behind (s) each, and how big (of
    /// the spirit's) the first.
    pub(crate) trail: u32,
    pub(crate) trail_lag: f32,
    pub(crate) trail_size: f32,
    /// Its light (lumens) and how far (m) it reaches.
    pub(crate) light: f32,
    pub(crate) light_range: f32,
    /// It tears out this high (m) over his feet and hangs this much higher
    /// (m), writhing this much (m).
    pub(crate) chest_height: f32,
    pub(crate) hover_height: f32,
    pub(crate) writhe: f32,
    /// Its flight: how high (m) its arc bows over the straight line, and
    /// how far over the pyramid's altar (m) it aims.
    pub(crate) arc_height: f32,
    pub(crate) target_up: f32,
    /// The flash (lumens) as it goes into the pyramid.
    pub(crate) flash_light: f32,
    /// The flee sound's volume (linear, before the mix).
    pub(crate) flee_volume: f32,
}

impl Default for EggDeathSettings {
    fn default() -> Self {
        Self {
            color: [0.4, 1.0, 0.45],
            size: 1.4,
            glow: 1.2,
            smoke_size: 1.0,
            trail: 16,
            trail_lag: 0.04,
            trail_size: 0.7,
            light: 500_000.0,
            light_range: 16.0,
            chest_height: 1.4,
            hover_height: 2.6,
            writhe: 0.18,
            arc_height: 8.0,
            target_up: 1.5,
            flash_light: 2_000_000.0,
            flee_volume: 1.5,
        }
    }
}

/// His death cry (`death.mp3`): from his body, then from his spirit once
/// it's out (`fight::his_lines` spawns it).
#[derive(Component)]
pub(super) struct DeathCry;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum SpiritPart {
    /// The dim glow at its heart, with its light.
    Core,
    /// A streak of smoke swirling round it.
    Head(u32),
    /// The haze round it.
    Puff(u32),
    /// Its tail, `0` nearest it.
    Trail(u32),
    /// The flash where it goes into the pyramid.
    Flash,
}

/// How many streaks swirl round it, and how many puffs of haze.
const HEAD_STREAKS: u32 = 6;
const PUFFS: u32 = 2;

/// A smoke part's own look: where it starts in its swirl, which way and
/// how fast it turns, its size (of its kind's) and its mirroring — so no
/// two look alike.
#[derive(Component, Clone, Copy)]
struct SmokeBits {
    phase: f32,
    turn: f32,
    size: f32,
    flip: Vec2,
}

impl SmokeBits {
    fn new(seed: u32) -> Self {
        let r = |k: u32| crate::util::rand01(seed.wrapping_mul(2_654_435_761).wrapping_add(k));
        let sign = |x: f32| if x < 0.5 { -1.0 } else { 1.0 };
        Self {
            phase: r(1) * std::f32::consts::TAU,
            turn: sign(r(2)) * (0.6 + r(3)),
            size: 0.75 + 0.5 * r(4),
            flip: Vec2::new(sign(r(5)), sign(r(6))),
        }
    }
}

/// Where its parts are: where it tears out (his chest), where it hangs,
/// and where it goes (into the pyramid).
#[derive(Resource, Clone, Copy)]
struct SpiritPath {
    chest: Vec3,
    hover: Vec3,
    target: Vec3,
}

/// Its flee sound, loaded once.
#[derive(Resource)]
struct FleeSound(Handle<AudioSource>);

fn track_death(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut death: ResMut<EggDeath>,
) {
    let dying = egg_game(&local, &lobbies).is_some_and(|l| l.easter_egg.step == EggStep::HisDeath);
    if !dying {
        if death.t.is_some() {
            *death = default();
        }
        return;
    }
    let t = death.t.get_or_insert(0.0);
    if !paused.0 {
        *t += time.delta_secs();
    }
}

/// Its parts are there while his death plays (on a map with somewhere for
/// it to start), and gone otherwise.
#[allow(clippy::too_many_arguments)]
fn sync_spirit(
    death: Res<EggDeath>,
    settings: Res<EggDeathSettings>,
    asset_server: Res<AssetServer>,
    fx: Option<Res<ExplosionAssets>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    parts: Query<Entity, With<SpiritPart>>,
    path: Option<Res<SpiritPath>>,
    smoke: Option<Res<SoulSmoke>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let lobby = egg_game(&local, &lobbies);
    let from = lobby.filter(|_| death.t.is_some()).and_then(|l| {
        let fell = l.easter_egg.hierophant.and_then(|h| h.fell_at);
        // (A debug jump straight here: from where he'd have stood up.)
        fell.or_else(|| shared::easter_egg::hierophant_start(l.map).map(|(feet, _)| feet))
    });
    let Some((from, lobby)) = from.zip(lobby) else {
        for e in &parts {
            commands.entity(e).despawn();
        }
        if path.is_some() {
            commands.remove_resource::<SpiritPath>();
        }
        return;
    };
    if !parts.is_empty() {
        return;
    }
    let (Some(fx), Some(smoke)) = (fx, smoke) else { return };
    let s = &*settings;
    let chest = from + Vec3::Y * s.chest_height;
    let hover = chest + Vec3::Y * s.hover_height;
    let target = shared::level::layout(lobby.map)
        .pyramid_altar
        .map_or(hover + Vec3::Y * 40.0, |at| at.pos + Vec3::Y * s.target_up);
    commands.insert_resource(SpiritPath { chest, hover, target });
    commands.insert_resource(FleeSound(asset_server.load("audio/zombies/easter_egg/hierophant/spirit_flee.mp3")));
    let glow = |materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: LinearRgba::rgb(0.0, 0.0, 0.0).into(),
            base_color_texture: Some(fx.glow.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            double_sided: true,
            cull_mode: None,
            ..default()
        })
    };
    let sprite = |part: SpiritPart, material: Handle<StandardMaterial>| {
        (
            StateScoped(AppState::InGame),
            part,
            Mesh3d(fx.quad.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(chest).with_scale(Vec3::ZERO),
            Visibility::Hidden,
            NotShadowCaster,
            NoFrustumCulling,
        )
    };
    let c = s.color;
    commands.spawn(sprite(SpiritPart::Core, glow(&mut materials))).with_child((
        PointLight {
            color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
            intensity: 0.0,
            range: s.light_range,
            shadows_enabled: false,
            ..default()
        },
        Transform::default(),
    ));
    let smoky = |part: SpiritPart, material: Handle<crate::exhumer::SoulSmokeMaterial>, seed: u32| {
        (
            StateScoped(AppState::InGame),
            part,
            SmokeBits::new(seed),
            Mesh3d(fx.quad.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(chest).with_scale(Vec3::ZERO),
            Visibility::Hidden,
            NotShadowCaster,
            NoFrustumCulling,
        )
    };
    for j in 0..PUFFS {
        commands.spawn(smoky(SpiritPart::Puff(j), smoke.puff_material(), 900 + j));
    }
    for j in 0..HEAD_STREAKS {
        commands.spawn(smoky(SpiritPart::Head(j), smoke.swirl_material(0), 100 + j));
    }
    let n = s.trail.max(1);
    for i in 0..s.trail {
        // (Fainter further back.)
        let k = i as f32 / n as f32;
        let tier = if k < 0.3 { 0 } else if k < 0.65 { 1 } else { 2 };
        commands.spawn(smoky(SpiritPart::Trail(i), smoke.swirl_material(tier), 300 + i));
    }
    commands.spawn((
        StateScoped(AppState::InGame),
        SpiritPart::Flash,
        PointLight {
            color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
            intensity: 0.0,
            range: 30.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(target),
    ));
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Where the spirit is `t` s into his death, and how big (0..=1) — `None`
/// before it's out and once it's in the pyramid.
fn spirit_at(t: f32, path: &SpiritPath, s: &EggDeathSettings) -> Option<(Vec3, f32)> {
    if t < DEATH_SPIRIT_AT || t >= death_spirit_in_secs() {
        return None;
    }
    // Writhing as it hangs there (less as it picks up speed).
    let writhe = |k: f32| {
        Vec3::new((t * 7.3).sin() + 0.5 * (t * 13.1).sin(), 0.6 * (t * 9.7).sin(), (t * 6.1).cos() + 0.5 * (t * 11.9).cos())
            * s.writhe
            * k
    };
    if t < DEATH_FLY_AT {
        // Tearing up out of him, then hanging there.
        let up = ramp(t, DEATH_SPIRIT_AT, DEATH_SPIRIT_AT + DEATH_SPIRIT_RISE_SECS);
        let out = 1.0 - (1.0 - up).powi(3);
        return Some((path.chest.lerp(path.hover, out) + writhe(up), 0.3 + 0.7 * ease(up / 0.4)));
    }
    // Flying: slow off the mark, faster and faster, in an arc — a bow over
    // the straight line — spiralling as it goes.
    let x = ramp(t, DEATH_FLY_AT, DEATH_FLY_AT + DEATH_FLY_SECS);
    let u = x.powf(1.7);
    let mid = path.hover.lerp(path.target, 0.5) + Vec3::Y * s.arc_height;
    let along = path.hover.lerp(mid, u).lerp(mid.lerp(path.target, u), u);
    let spiral = Vec3::new((x * 18.0).cos(), 0.0, (x * 18.0).sin()) * 0.6 * (1.0 - x);
    Some((along + spiral + writhe(1.0 - x), 1.0 - 0.45 * x))
}

/// Play it out on the spirit, its tail and the flash by the clock — the
/// lights stuttering as it tears out and as it goes in, its sound as it
/// flies.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_spirit(
    time: Res<Time>,
    mut death: ResMut<EggDeath>,
    settings: Res<EggDeathSettings>,
    path: Option<Res<SpiritPath>>,
    flee: Option<Res<FleeSound>>,
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    mut parts: Query<
        (
            Entity,
            &SpiritPart,
            &mut Transform,
            &mut Visibility,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&mut PointLight>,
            Option<&SmokeBits>,
        ),
        Without<WorldModelCamera>,
    >,
    mut lights: Query<&mut PointLight, Without<SpiritPart>>,
    children: Query<&Children>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut light_cue: ResMut<crate::environment::EggLightCue>,
    mut commands: Commands,
) {
    let (Some(t), Some(path), Some(cam)) = (death.t, path, camera) else { return };
    let s = &*settings;
    let cam = cam.into_inner();
    let (cam_pos, cam_up, cam_right) = (cam.translation(), cam.up().as_vec3(), cam.right().as_vec3());
    let c = s.color;
    let flicker = 0.85 + 0.15 * (time.elapsed_secs() * 21.0).sin();

    if t >= DEATH_SPIRIT_AT && !death.torn_out {
        death.torn_out = true;
        light_cue.jolt = true;
    }
    if t >= death_spirit_in_secs() && !death.arrived {
        death.arrived = true;
        light_cue.jolt = true;
    }
    let now_at = spirit_at(t, &path, s);
    // The way it's going at `t` (rising, while it hangs there) — its smoke
    // laid along it.
    let going = |t: f32| {
        let (Some((a, _)), Some((b, _))) = (spirit_at(t - 0.04, &path, s), spirit_at(t + 0.04, &path, s)) else {
            return Vec3::Y;
        };
        ((b - a) / 0.08 + Vec3::Y * 1.5).normalize_or(Vec3::Y)
    };
    let smoke_size = s.size * s.smoke_size;
    for (e, part, mut tf, mut vis, material, light, bits) in &mut parts {
        let bits = bits.copied().unwrap_or(SmokeBits::new(0));
        match *part {
            SpiritPart::Head(_) => {
                vis.set_if_neq(if now_at.is_some() { Visibility::Inherited } else { Visibility::Hidden });
                let Some((head, size)) = now_at else { continue };
                let dir = going(t);
                let (a, b) = (dir.any_orthonormal_vector(), dir.cross(dir.any_orthonormal_vector()));
                let swirl = bits.phase + t * 3.2 * bits.turn;
                let pos = head + (a * swirl.cos() + b * swirl.sin()) * smoke_size * 0.16;
                let w = (smoke_size * 0.8 * bits.size * size).max(1.0e-4);
                let roll = 0.35 * (t * 4.0 * bits.turn + bits.phase).sin();
                *tf = Transform::from_translation(pos)
                    .with_rotation(along_rotation(pos, dir, cam_pos, roll))
                    .with_scale(Vec3::new(w * bits.flip.x, w * 0.5 * bits.flip.y, 1.0));
            }
            SpiritPart::Puff(_) => {
                vis.set_if_neq(if now_at.is_some() { Visibility::Inherited } else { Visibility::Hidden });
                let Some((head, size)) = now_at else { continue };
                let w = (smoke_size * 1.1 * bits.size * size).max(1.0e-4);
                *tf = Transform::from_translation(head)
                    .with_rotation(smoke_billboard_rotation(head, cam_pos, cam_up, cam_right, bits.phase + t * 0.7 * bits.turn))
                    .with_scale(Vec3::new(w, w, 1.0));
            }
            SpiritPart::Core => {
                let at = now_at;
                vis.set_if_neq(if at.is_some() { Visibility::Inherited } else { Visibility::Hidden });
                let (pos, size) = at.unwrap_or((tf.translation, 0.0));
                // (A dim heart inside the smoke.)
                let size = (s.size * 0.45 * size).max(1.0e-4);
                *tf = Transform::from_translation(pos)
                    .with_rotation(smoke_billboard_rotation(pos, cam_pos, cam_up, cam_right, t * 2.0))
                    .with_scale(Vec3::new(size, size, 1.0));
                if let Some(m) = material.and_then(|m| materials.get_mut(&m.0)) {
                    let b = s.glow * flicker;
                    m.base_color = LinearRgba::rgb(c[0] * b, c[1] * b, c[2] * b).into();
                }
                for child in children.get(e).into_iter().flatten() {
                    if let Ok(mut light) = lights.get_mut(*child) {
                        light.intensity = if at.is_some() { s.light * flicker } else { 0.0 };
                        light.range = s.light_range;
                    }
                }
                // Its scream as it flies, going with it.
                if t >= DEATH_FLY_AT && !death.fled && at.is_some() {
                    death.fled = true;
                    if let Some(flee) = &flee {
                        commands.entity(e).with_child((
                            AudioPlayer::new(flee.0.clone()),
                            Transform::default(),
                            crate::positional_playback(Volume::Linear(s.flee_volume)),
                        ));
                    }
                }
            }
            // Streaks of smoke left behind along its path, each laid along
            // the way it went there, spiralling out wider and growing
            // further back — dying away at the very end.
            SpiritPart::Trail(i) => {
                let back = (i + 1) as f32 * s.trail_lag;
                let at = spirit_at(t - back, &path, s).filter(|_| now_at.is_some());
                vis.set_if_neq(if at.is_some() { Visibility::Inherited } else { Visibility::Hidden });
                let Some((pos, size)) = at else { continue };
                let k = (i + 1) as f32 / (s.trail.max(1) as f32 + 1.0);
                let dir = going(t - back);
                let (a, b) = (dir.any_orthonormal_vector(), dir.cross(dir.any_orthonormal_vector()));
                let swirl = bits.phase + (t - back) * 3.0 * bits.turn;
                let pos = pos + (a * swirl.cos() + b * swirl.sin()) * smoke_size * 0.25 * (0.2 + 0.8 * k);
                let tail = 1.0 - 0.6 * ((k - 0.7) / 0.3).clamp(0.0, 1.0);
                let w = (smoke_size * s.trail_size * bits.size * size * (0.75 + 0.6 * k) * tail).max(1.0e-4);
                let roll = 0.3 * ((t - back) * 4.0 * bits.turn + bits.phase).sin();
                *tf = Transform::from_translation(pos)
                    .with_rotation(along_rotation(pos, dir, cam_pos, roll))
                    .with_scale(Vec3::new(w * bits.flip.x, w * 0.5 * bits.flip.y, 1.0));
            }
            SpiritPart::Flash => {
                if let Some(mut light) = light {
                    let since = t - death_spirit_in_secs();
                    light.intensity = if since >= 0.0 { s.flash_light * (-since / 0.35).exp() } else { 0.0 };
                    tf.translation = path.target;
                }
            }
        }
    }
}

/// His death cry goes with his spirit once it's out.
fn carry_cry(
    death: Res<EggDeath>,
    settings: Res<EggDeathSettings>,
    path: Option<Res<SpiritPath>>,
    mut cries: Query<&mut Transform, With<DeathCry>>,
) {
    let (Some(t), Some(path)) = (death.t, path) else { return };
    let Some((pos, _)) = spirit_at(t, &path, &settings) else { return };
    for mut tf in &mut cries {
        tf.translation = pos;
    }
}

// --- the debug panel --------------------------------------------------------------------

/// The debug panel's controls for it.
pub(crate) fn death_section(ui: &mut egui::Ui, s: &mut EggDeathSettings, death: &EggDeath) {
    match death.t {
        Some(t) => ui.label(format!("now: {t:.1} s into it (over at {:.1} s)", shared::easter_egg::DEATH_SECS)),
        None => ui.label("now: not playing — kill him (\"The fight\" → Kill him) or jump to \"His death\""),
    };
    let color = |ui: &mut egui::Ui, c: &mut [f32; 3], label: &str| {
        ui.horizontal(|ui| {
            ui.color_edit_button_rgb(c);
            ui.label(label);
        });
    };
    color(ui, &mut s.color, "spirit colour");
    ui.add(egui::Slider::new(&mut s.size, 0.1f32..=5.0).text("spirit size (m)"));
    ui.add(egui::Slider::new(&mut s.glow, 0.0f32..=20.0).text("glow at its heart"));
    ui.add(egui::Slider::new(&mut s.smoke_size, 0.1f32..=3.0).text("smoke size (× spirit)"));
    ui.add(egui::Slider::new(&mut s.trail, 0..=40).text("tail streaks (next time)"));
    ui.add(egui::Slider::new(&mut s.trail_lag, 0.005f32..=0.2).text("tail spacing (s)"));
    ui.add(egui::Slider::new(&mut s.trail_size, 0.05f32..=2.0).text("tail size (× spirit)"));
    ui.add(egui::Slider::new(&mut s.light, 0.0f32..=3_000_000.0).text("spirit light (lm)"));
    ui.add(egui::Slider::new(&mut s.light_range, 1.0f32..=60.0).text("spirit light reach (m)"));
    ui.add(egui::Slider::new(&mut s.chest_height, 0.0f32..=4.0).text("tears out at (m up, next time)"));
    ui.add(egui::Slider::new(&mut s.hover_height, 0.0f32..=8.0).text("hangs higher by (m, next time)"));
    ui.add(egui::Slider::new(&mut s.writhe, 0.0f32..=1.0).text("writhing (m)"));
    ui.add(egui::Slider::new(&mut s.arc_height, 0.0f32..=40.0).text("flight's arc (m)"));
    ui.add(egui::Slider::new(&mut s.target_up, -5.0f32..=10.0).text("aims over the pyramid altar by (m, next time)"));
    ui.add(egui::Slider::new(&mut s.flash_light, 0.0f32..=10_000_000.0).text("flash going in (lm)"));
    ui.add(egui::Slider::new(&mut s.flee_volume, 0.0f32..=3.0).text("flee sound volume"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = EggDeathSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
