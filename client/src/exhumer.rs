//! The Exhumer on this client — Der Tempel's wonder weapon (`shared::exhumer`;
//! the server's `exhumer` runs it). Two triggers:
//!
//! * **Aim (held) — its beam** ([`beam_input`]: the aim button with it in
//!   hand — no aim-down-sight for it, `weapons::ads`; sent with our input as
//!   `PlayerInput::beaming`). Every player's beam is drawn ([`draw_beams`]:
//!   ours from the gun, theirs from their `PlayerPose::beaming`) out to its
//!   reach or the first wall (`shaders/exhumer_beam.wgsl`), a plume of green
//!   soul-smoke drifting back along it into the gun, its hum looping. The zombies it has hold of
//!   ([`shared::exhumer::ExhumerDrain`]) are drained where they stand,
//!   shuddering and tipping back, a soul-light growing in each chest
//!   ([`Drained`]). One drained is exhumed
//!   ([`shared::exhumer::ExhumerExhume`], [`Exhumed`]): lifted into the
//!   air, turning and tipping back, until its soul is torn out at the top
//!   ([`shared::exhumer::ExhumerSouls`]): a cloud of green smoke streaks from its chest
//!   back into the gun and the empty body drops limp out of the air (a
//!   ragdoll, `ragdoll`).
//! * **Fire — let them out** (`weapons::weapon` asks; sent here as
//!   [`shared::exhumer::ReleaseSouls`]): a wisp after each zombie the
//!   server sent one for ([`shared::exhumer::ExhumerSwarm`]), following it.
//!
//! Its souls are its only ammo: the server's count (`Weapon::souls`), shown
//! as its mag while it's in hand ([`sync_soul_mag`]) — they brighten its
//! camo (`pap`).
//!
//! Its sounds are placeholders at their final paths
//! (`audio/zombies/exhumer/`). Everything here is `StateScoped(InGame)` and
//! gone once played out (the souls and the beam are `Weapon`'s, reset with
//! it every game). Tuned in the debug panel's "Easter egg (Der Tempel)" →
//! "The Exhumer".

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster};
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::render::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError};
use bevy::render::view::NoFrustumCulling;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use bevy_egui::egui;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::{Interpolated, LocalId, MessageReceiver, PeerId, TriggerSender};
use shared::exhumer::{
    beam_dir, ExhumerDrain, ExhumerExhume, ExhumerSouls, ExhumerSwarm, ReleaseSouls, BEAM_RANGE, BEAM_TICK_SECS,
    DRAIN_SECS, EXHUME_SECS, LIFT_HEIGHT, SWARM_SPEED,
};
use shared::{PlayerId, PlayerPose};

use crate::audio_engine::prelude::*;
use crate::keybinds::KeyBindings;
use crate::net::{GameClient, RemoteAvatar};
use crate::vfx::{smoke_billboard_rotation, ExplosionAssets};
use crate::weapons::Weapon;
use crate::{killcam, menu, AppState, WorldModelCamera, EYE_HEIGHT};

pub(crate) struct ExhumerPlugin;

impl Plugin for ExhumerPlugin {
    fn build(&self, app: &mut App) {
        let can_beam = || menu::weapon_active.and(killcam::no_killcam);
        app.add_plugins(MaterialPlugin::<ExhumerBeamMaterial> {
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .add_plugins(MaterialPlugin::<SoulSmokeMaterial> {
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .init_resource::<ExhumerFxSettings>()
            .init_resource::<OwnBeam>()
            .init_resource::<ExhumerDebug>()
            .add_systems(Startup, load_sounds)
            .add_systems(
                Update,
                (
                    (beam_input.run_if(can_beam()), beam_off.run_if(not(can_beam()))),
                    sync_soul_mag,
                    send_release,
                    debug_give,
                    soul_smoke,
                    (receive_drains, receive_exhumes, receive_souls, receive_swarms),
                    lift_avatars.after(crate::net::follow_remote_avatars),
                    run_glows,
                    fly_wisps,
                    altar_glows,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            // Once the view model's final for the frame (its sway's written in
            // `Update`, its animation just now), and in time for this frame's
            // transforms: a beam drawn off last frame's camera and gun
            // trails them by a frame, and judders as the aim moves.
            .add_systems(
                PostUpdate,
                (locate_own_beam, draw_beams, fly_motes)
                    .chain()
                    .after(bevy::app::Animation)
                    .before(bevy::transform::TransformSystem::TransformPropagate)
                    .run_if(in_state(AppState::InGame)),
            )
            // (Not left humming once out of a game.)
            .add_systems(
                OnExit(AppState::InGame),
                |mut weapon: ResMut<Weapon>, mut own: ResMut<OwnBeam>| {
                    weapon.beaming = false;
                    *own = default();
                },
            );
    }
}

/// Panel-tunable look ("The Exhumer").
#[derive(Resource, Clone, Debug)]
pub(crate) struct ExhumerFxSettings {
    /// The souls' colour (linear); a soul's smoke — each puff's size (m),
    /// how bright, how many puffs trail behind its head and how far apart
    /// (s), how far (m) they swirl about its path, how strong the smoke is
    /// (0..1), and how fast (rad/s) each puff turns (motes and the beam's
    /// end too).
    pub(crate) color: [f32; 3],
    pub(crate) wisp_size: f32,
    pub(crate) wisp_glow: f32,
    pub(crate) trail: u32,
    pub(crate) trail_lag: f32,
    pub(crate) wisp_spread: f32,
    pub(crate) smoke_opacity: f32,
    pub(crate) puff_spin: f32,
    /// How far (m) in front of a wall or the floor the smoke fades out.
    pub(crate) smoke_soft: f32,
    /// How long (s) a torn-out soul takes back to the gun, and how high (m)
    /// its arc bows.
    pub(crate) return_secs: f32,
    pub(crate) arc: f32,
    /// The beam: its width (m) at the gun and at its far end, how bright,
    /// how much it flickers (0..1), how fast (m/s) its smoke drifts back to
    /// the gun, how smoky it is (0..1), the size (m) of the smoke where it
    /// ends, and its light (lumens); the puffs of smoke drawn back along it —
    /// how many a second, how fast (m/s), how big (m).
    pub(crate) beam_width_near: f32,
    /// How long (s) it takes to shoot out from the gun to its full reach.
    pub(crate) beam_extend_secs: f32,
    /// Our own beam's start, nudged (m, as the view-model camera sees it: x
    /// right, y up, z back) from the end of the barrel ([`BARREL_TIP`]).
    pub(crate) beam_start_nudge: Vec3,
    /// How quickly (1/s) its length follows what it's hitting, so aiming
    /// across a nearer or further surface draws it in or out, not in one go.
    pub(crate) beam_reach_smooth: f32,
    /// How its far end trails behind as it swings, like the end of a rope
    /// ([`BeamVisual::swing`]) — side to side and up and down, never along
    /// it: how much (0 = dead straight); how quickly it catches up (Hz); how
    /// little it overshoots (1 = not at all); the most it's off (°); and how
    /// much of the bend's out at the end (1 = even along it, higher = less
    /// near the gun).
    pub(crate) beam_lag: f32,
    pub(crate) beam_lag_hz: f32,
    pub(crate) beam_lag_damping: f32,
    pub(crate) beam_lag_max_deg: f32,
    pub(crate) beam_lag_curve: f32,
    pub(crate) beam_width_far: f32,
    pub(crate) beam_glow: f32,
    pub(crate) beam_flicker: f32,
    pub(crate) beam_flow: f32,
    /// How much its edges billow and wander, how far (m) from the gun it
    /// burns pale, and how much its smoke dims what's behind it (0 = none) —
    /// what keeps it green over a bright scene.
    pub(crate) beam_turbulence: f32,
    pub(crate) beam_heat: f32,
    pub(crate) beam_body: f32,
    /// The glow where it leaves the barrel (m), and how bright the puffs of
    /// smoke along it and where it ends are.
    pub(crate) beam_muzzle_glow: f32,
    pub(crate) beam_puff_glow: f32,
    pub(crate) beam_smoke: f32,
    pub(crate) flare_size: f32,
    pub(crate) beam_light: f32,
    pub(crate) motes_per_sec: f32,
    pub(crate) mote_speed: f32,
    pub(crate) mote_size: f32,
    /// A body being drained, on the ground: how far (°) it tips back, and
    /// how hard (°) it shudders, once it's all but drained...
    pub(crate) drain_tilt_deg: f32,
    pub(crate) drain_shake_deg: f32,
    /// ...and exhumed, lifted: how fast (rad/s) it turns, how far (°) it
    /// tips back; the glow in its chest (lumens, at its brightest).
    pub(crate) lift_spin: f32,
    pub(crate) lift_tilt_deg: f32,
    pub(crate) chest_light: f32,
    /// Each Exhumer waiting on the pyramid's altar: its light (lumens), and
    /// the swirl of soul-smoke turning round it — its streaks' length (m),
    /// how far out (m) and how fast (rad/s) they circle.
    pub(crate) altar_light: f32,
    pub(crate) altar_swirl_len: f32,
    pub(crate) altar_swirl_radius: f32,
    pub(crate) altar_swirl_speed: f32,
    /// The sounds' volume (linear, before the mix), and the beam's hum's.
    pub(crate) volume: f32,
    pub(crate) beam_volume: f32,
}

impl Default for ExhumerFxSettings {
    fn default() -> Self {
        Self {
            color: [0.55, 1.0, 0.65],
            wisp_size: 1.1,
            wisp_glow: 3.0,
            trail: 9,
            trail_lag: 0.045,
            wisp_spread: 0.1,
            smoke_opacity: 0.6,
            puff_spin: 0.8,
            smoke_soft: 0.6,
            return_secs: 0.75,
            arc: 1.6,
            beam_width_near: 0.14,
            beam_extend_secs: 0.22,
            beam_start_nudge: Vec3::ZERO,
            beam_reach_smooth: 14.0,
            beam_lag: 1.0,
            beam_lag_hz: 2.5,
            beam_lag_damping: 0.7,
            beam_lag_max_deg: 6.0,
            beam_lag_curve: 2.0,
            beam_width_far: 1.8,
            beam_glow: 1.3,
            beam_flicker: 0.35,
            beam_flow: 5.0,
            beam_turbulence: 0.8,
            beam_heat: 3.0,
            beam_body: 0.3,
            beam_muzzle_glow: 0.55,
            beam_puff_glow: 0.55,
            beam_smoke: 0.85,
            flare_size: 2.4,
            beam_light: 80_000.0,
            motes_per_sec: 30.0,
            mote_speed: 6.0,
            mote_size: 1.5,
            drain_tilt_deg: 10.0,
            drain_shake_deg: 4.0,
            lift_spin: 0.6,
            lift_tilt_deg: 22.0,
            chest_light: 120_000.0,
            altar_light: 60_000.0,
            altar_swirl_len: 0.55,
            altar_swirl_radius: 0.3,
            altar_swirl_speed: 0.9,
            volume: 1.4,
            beam_volume: 0.9,
        }
    }
}

#[derive(Resource)]
struct ExhumerSounds {
    grab: Handle<AudioSource>,
    tear: Handle<AudioSource>,
    release: Handle<AudioSource>,
    beam: Handle<AudioSource>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(ExhumerSounds {
        grab: asset_server.load("audio/zombies/exhumer/lift.mp3"),
        tear: asset_server.load("audio/zombies/exhumer/soul_tear.mp3"),
        release: asset_server.load("audio/zombies/exhumer/release.mp3"),
        beam: asset_server.load("audio/zombies/exhumer/beam.mp3"),
    });
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn sound_at(commands: &mut Commands, clip: &Handle<AudioSource>, at: Vec3, volume: f32) {
    commands.spawn((
        StateScoped(AppState::InGame),
        AudioPlayer::new(clip.clone()),
        Transform::from_translation(at),
        crate::positional_playback(Volume::Linear(volume)),
    ));
}

// --- our triggers -------------------------------------------------------------------------

/// The aim button held with the Exhumer in hand (and our hands free, the
/// window ours): its beam's on.
#[allow(clippy::too_many_arguments)]
fn beam_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    hands: crate::weapons::Hands,
    mut weapon: ResMut<Weapon>,
) {
    let on = weapon.exhumer_in_hand()
        && weapon.busy.is_none()
        && hands.free()
        && window.cursor_options.grab_mode != CursorGrabMode::None
        && binds.aim.pressed(&keys, &mouse);
    if weapon.beaming != on {
        weapon.beaming = on;
    }
}

/// No beam while the weapon's not ours to use (a menu, dead, down, a kill
/// cam, a soul up in the pyramid...).
fn beam_off(mut weapon: ResMut<Weapon>) {
    if weapon.beaming {
        weapon.beaming = false;
    }
}

/// While the Exhumer's in hand its mag shows its souls — its only ammo —
/// with nothing in reserve.
fn sync_soul_mag(mut weapon: ResMut<Weapon>) {
    if weapon.primary != shared::weapon::WeaponId::Exhumer {
        return;
    }
    if weapon.mag != weapon.souls || weapon.reserve != 0 {
        weapon.mag = weapon.souls;
        weapon.reserve = 0;
    }
}

/// Our ask to let the souls out (`weapons::weapon`): to the server.
fn send_release(mut weapon: ResMut<Weapon>, mut tx: Query<&mut TriggerSender<ReleaseSouls>, With<GameClient>>) {
    if !std::mem::take(&mut weapon.release_request) {
        return;
    }
    if let Ok(mut tx) = tx.single_mut() {
        tx.trigger::<shared::LobbyChannel>(ReleaseSouls);
    }
}

// --- drained ------------------------------------------------------------------------------

/// On a zombie's avatar a beam has hold of, on the ground: how far it's
/// drained (0..=1, the server's), how far it's shown drained, and when
/// (`Time::elapsed_secs`) the server last said — let go, it eases back.
/// The server only says every `BEAM_TICK_SECS`, so in between it's carried
/// on at the rate it drains ([`lift_avatars`]): steady, not a jump each
/// time it hears.
#[derive(Component)]
struct Drained {
    target: f32,
    shown: f32,
    seen: f32,
}

/// On a zombie's avatar that's been drained and is being exhumed: how long
/// (s) it's been rising (to [`LIFT_HEIGHT`] over [`EXHUME_SECS`]), and how
/// far (rad) it's turned. Its death drops it (a ragdoll); should that never
/// come (the game ended meanwhile), it sinks back down.
#[derive(Component)]
struct Exhumed {
    t: f32,
    turn: f32,
}

/// The soul-light in a drained zombie's chest.
#[derive(Component)]
struct ChestGlow {
    avatar: Entity,
}

/// The avatar of the bot `peer`, if it's here — and whether a beam's
/// already got it (or it's being exhumed).
fn avatar_of(
    peer: PeerId,
    avatars: &Query<(Entity, &RemoteAvatar, Has<Drained>, Has<Exhumed>)>,
    ids: &Query<&PlayerId>,
    interp: &Query<&Interpolated>,
) -> Option<(Entity, bool, bool)> {
    avatars
        .iter()
        .find(|(_, a, ..)| crate::boss::peer_of(a.src, ids, interp) == Some(peer))
        .map(|(e, _, drained, exhumed)| (e, drained, exhumed))
}

/// The soul-light in `avatar`'s chest.
fn spawn_chest_glow(commands: &mut Commands, settings: &ExhumerFxSettings, avatar: Entity) {
    let c = settings.color;
    commands.spawn((
        StateScoped(AppState::InGame),
        ChestGlow { avatar },
        PointLight {
            color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
            intensity: 0.0,
            range: 4.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::default(),
    ));
}

#[allow(clippy::too_many_arguments)]
fn receive_drains(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    mut receivers: Query<&mut MessageReceiver<ExhumerDrain>>,
    avatars: Query<(Entity, &RemoteAvatar, Has<Drained>, Has<Exhumed>)>,
    mut drained: Query<&mut Drained>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            for (zombie, share) in msg.zombies {
                let Some((avatar, already, exhumed)) = avatar_of(zombie, &avatars, &ids, &interp) else { continue };
                if exhumed {
                    continue;
                }
                if already {
                    if let Ok(mut d) = drained.get_mut(avatar) {
                        d.target = d.target.max(share);
                        d.seen = now;
                    }
                    continue;
                }
                // Taken hold of: held where it stands, a glow in its chest.
                commands.entity(avatar).insert(Drained { target: share, shown: 0.0, seen: now });
                spawn_chest_glow(&mut commands, &settings, avatar);
            }
        }
    }
}

/// Zombies drained: each lifted into the air, its soul torn out at the top.
#[allow(clippy::too_many_arguments)]
fn receive_exhumes(
    settings: Res<ExhumerFxSettings>,
    sounds: Res<ExhumerSounds>,
    mut receivers: Query<&mut MessageReceiver<ExhumerExhume>>,
    avatars: Query<(Entity, &RemoteAvatar, Has<Drained>, Has<Exhumed>)>,
    transforms: Query<&Transform, With<RemoteAvatar>>,
    (ids, interp): (Query<&PlayerId>, Query<&Interpolated>),
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            for zombie in msg.zombies {
                let Some((avatar, drained, exhumed)) = avatar_of(zombie, &avatars, &ids, &interp) else { continue };
                if exhumed {
                    continue;
                }
                // (Its chest's glow carries on from the drain — or, never
                // seen drained, one's lit now.)
                if !drained {
                    spawn_chest_glow(&mut commands, &settings, avatar);
                }
                commands.entity(avatar).remove::<Drained>().insert(Exhumed { t: 0.0, turn: 0.0 });
                if let Ok(tf) = transforms.get(avatar) {
                    sound_at(&mut commands, &sounds.grab, tf.translation + Vec3::Y, settings.volume);
                }
            }
        }
    }
}

/// Every body a beam has hold of, over where its pose puts it, on the
/// ground: tipping back and shuddering harder the more it's drained —
/// easing back once let go. And every one exhumed: rising, turning and
/// tipping back, until its death drops it (a ragdoll goes its own way).
#[allow(clippy::type_complexity)]
fn lift_avatars(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    mut drained: Query<
        (Entity, &mut Drained, &mut Transform),
        (With<RemoteAvatar>, Without<crate::ragdoll::Ragdoll>, Without<Exhumed>),
    >,
    mut exhumed: Query<(Entity, &mut Exhumed, &mut Transform), (With<RemoteAvatar>, Without<crate::ragdoll::Ragdoll>)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    for (e, mut d, mut tf) in &mut drained {
        // (Let go a moment: it eases back.)
        if now - d.seen > 0.45 {
            d.target = 0.0;
        }
        // Where it should be by now: what the server last said, carried on
        // at the rate an ordinary one drains (no more than a tick and a half
        // past it, so a beam let go doesn't carry it on); eased toward that
        // at no more than a little over that rate — steady rather than in
        // jumps — and back more quickly.
        let rate = 1.0 / DRAIN_SECS;
        let ahead = if d.target > 0.0 { ((now - d.seen) * rate).min(BEAM_TICK_SECS * 1.5 * rate) } else { 0.0 };
        let goal = (d.target + ahead).min(1.0);
        let (up_step, down_step) = (dt * rate * 1.5, dt * 2.5);
        d.shown += (goal - d.shown).clamp(-down_step, up_step);
        if d.target == 0.0 && d.shown <= 0.0 {
            commands.entity(e).remove::<Drained>();
            continue;
        }
        let k = ease(d.shown);
        let shake = settings.drain_shake_deg.to_radians() * k;
        let seed = e.index() as f32;
        let (yaw, roll) = ((now * 31.0 + seed).sin() * shake, (now * 23.0 + seed * 1.7).sin() * shake * 0.6);
        let tilt = settings.drain_tilt_deg.to_radians() * k;
        tf.rotation = Quat::from_rotation_y(yaw) * tf.rotation * Quat::from_rotation_x(-tilt) * Quat::from_rotation_z(roll);
    }
    for (e, mut x, mut tf) in &mut exhumed {
        x.t += dt;
        // Up over `EXHUME_SECS`, held at the top a moment for its death —
        // and, should that not come, back down.
        let hold = 1.0;
        let p = if x.t <= EXHUME_SECS + hold {
            x.t / EXHUME_SECS
        } else {
            1.0 - (x.t - EXHUME_SECS - hold) / 0.6
        };
        if p <= 0.0 {
            commands.entity(e).remove::<Exhumed>();
            continue;
        }
        let up = ease(p);
        tf.translation.y += LIFT_HEIGHT * up + (now * 5.0).sin() * 0.05 * up;
        // (Turning at most `lift_spin`, as far as it's risen.)
        x.turn += settings.lift_spin * up * dt;
        let spin = x.turn;
        let tilt = (settings.drain_tilt_deg + (settings.lift_tilt_deg - settings.drain_tilt_deg) * up).to_radians();
        tf.rotation = Quat::from_rotation_y(spin) * tf.rotation * Quat::from_rotation_x(-tilt);
    }
}

/// The glow in each drained chest, as bright as it's drained — at its
/// brightest as it's exhumed.
#[allow(clippy::type_complexity)]
fn run_glows(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    avatars: Query<(&Transform, Option<&Drained>, Option<&Exhumed>), (With<RemoteAvatar>, Without<ChestGlow>)>,
    mut glows: Query<(Entity, &ChestGlow, &mut Transform, &mut PointLight)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (e, glow, mut tf, mut light) in &mut glows {
        let strength = match avatars.get(glow.avatar).ok() {
            Some((_, _, Some(x))) => 1.0 + 0.3 * (x.t / EXHUME_SECS).min(1.0),
            Some((_, Some(d), None)) => d.shown.powi(2),
            _ => {
                commands.entity(e).despawn();
                continue;
            }
        };
        let Ok((body, ..)) = avatars.get(glow.avatar) else { continue };
        tf.translation = body.translation + Vec3::Y * 1.25;
        let flicker = 0.8 + 0.2 * (now * 27.0 + e.index() as f32).sin();
        light.intensity = settings.chest_light * strength * flicker;
    }
}

// --- the beams ----------------------------------------------------------------------------

const BEAM_SHADER_ASSET_PATH: &str = "shaders/exhumer_beam.wgsl";
const SMOKE_SHADER_ASSET_PATH: &str = "shaders/soul_smoke.wgsl";
/// The green swirl of soul-smoke: a long, wispy streak (2:1, lying along
/// its width), see-through round it.
const SWIRL_TEXTURE: &str = "textures/vfx/soul_swirl.png";

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct (as in `power_ups`).
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct BeamParams {
        pub(crate) color: Vec4,
        pub(crate) near: f32,
        pub(crate) flow: f32,
        pub(crate) flicker: f32,
        pub(crate) turbulence: f32,
        pub(crate) heat: f32,
        pub(crate) smoke: f32,
        pub(crate) body: f32,
        pub(crate) _pad0: f32,
        /// How far (m, world) its far end's off its line; `w`: the bend's
        /// curve.
        pub(crate) bend: Vec4,
    }

    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct SmokeParams {
        pub(crate) color: Vec4,
        pub(crate) soft: f32,
        pub(crate) body: f32,
        pub(crate) _pad1: f32,
        pub(crate) _pad2: f32,
    }
}
use params::{BeamParams, SmokeParams};

/// Both faces, depth-tested but never writing depth.
fn additive_quad(descriptor: &mut RenderPipelineDescriptor) {
    descriptor.primitive.cull_mode = None;
    if let Some(depth) = descriptor.depth_stencil.as_mut() {
        depth.depth_write_enabled = false;
    }
}

/// The beam's look (`shaders/exhumer_beam.wgsl`): a billowing plume of
/// soul-smoke on a ribbon. Each beam has its own, for its bend ([`BeamVisual`];
/// its seed is its transform's).
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct ExhumerBeamMaterial {
    #[uniform(0)]
    params: BeamParams,
}

impl ExhumerBeamMaterial {
    fn params(s: &ExhumerFxSettings) -> BeamParams {
        let [r, g, b] = s.color;
        BeamParams {
            color: Vec4::new(r, g, b, s.beam_glow),
            near: (s.beam_width_near / s.beam_width_far.max(1.0e-3)).clamp(0.0, 1.0),
            flow: s.beam_flow,
            flicker: s.beam_flicker,
            turbulence: s.beam_turbulence,
            heat: s.beam_heat,
            smoke: s.beam_smoke,
            body: s.beam_body,
            bend: Vec3::ZERO.extend(s.beam_lag_curve.max(1.0)),
            ..default()
        }
    }
}

impl Material for ExhumerBeamMaterial {
    fn vertex_shader() -> ShaderRef {
        BEAM_SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        BEAM_SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        additive_quad(descriptor);
        Ok(())
    }
}

/// A sprite of soul-smoke (`shaders/soul_smoke.wgsl`): its texture tinted
/// and drawn additively, fading out where it meets anything behind it —
/// so it never cuts into the floor or a wall on a hard line.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct SoulSmokeMaterial {
    #[uniform(0)]
    params: SmokeParams,
    #[texture(1)]
    #[sampler(2)]
    texture: Handle<Image>,
}

impl Material for SoulSmokeMaterial {
    fn fragment_shader() -> ShaderRef {
        SMOKE_SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        additive_quad(descriptor);
        Ok(())
    }
}

/// A sprite at `pos` facing the camera with its width laid along `along`
/// (as it's seen), turned a further `roll` about the way it faces — for the
/// swirl, which is a streak, drawn along the way it's going.
pub(crate) fn along_rotation(pos: Vec3, along: Vec3, cam_pos: Vec3, roll: f32) -> Quat {
    let n = (cam_pos - pos).normalize_or(Vec3::Z);
    let x = (along - n * along.dot(n)).normalize_or(n.any_orthonormal_vector());
    let x = Quat::from_axis_angle(n, roll) * x;
    let y = n.cross(x);
    Quat::from_mat3(&Mat3::from_cols(x, y, n))
}

/// A beam being drawn: whose (`None`: ours), where it starts, the way it
/// goes and how far (m) it's drawn — this frame (shooting out from the gun
/// to its reach as it comes on, `reach` how far it's got) — motes owed
/// (fractional), its seed (1..2, so no two look alike), its material, and
/// how its far end's trailing ([`Self::swing`]).
#[derive(Component)]
struct BeamVisual {
    peer: Option<PeerId>,
    from: Vec3,
    dir: Vec3,
    len: f32,
    reach: f32,
    owed: f32,
    seed: f32,
    material: Handle<ExhumerBeamMaterial>,
    /// How far along the aim (m) it's reaching, easing toward what's there.
    hit: f32,
    /// Where its far end would be, dead straight, last frame.
    last_tip: Vec3,
    /// How far its far end's trailing (m), and how fast that's changing.
    lag: Vec3,
    lag_velocity: Vec3,
    /// What's drawn of `lag` this frame, and how it's spread along the beam.
    bend: Vec3,
    curve: f32,
}

impl BeamVisual {
    /// Let its far end trail behind wherever it's just gone, and spring back
    /// toward its line: held where it was in the world, then pulled in. Only
    /// across the beam, so it never stretches or shrinks.
    fn swing(&mut self, s: &ExhumerFxSettings, dt: f32) {
        let tip = self.from + self.dir * self.len;
        let moved = tip - self.last_tip;
        self.last_tip = tip;
        let across = |v: Vec3, dir: Vec3| v - dir * v.dot(dir);
        self.lag = across(self.lag - moved, self.dir);
        self.lag_velocity = across(self.lag_velocity, self.dir);
        let dt = dt.min(1.0 / 30.0);
        let w = std::f32::consts::TAU * s.beam_lag_hz.max(0.1);
        self.lag_velocity += (-w * w * self.lag - 2.0 * s.beam_lag_damping * w * self.lag_velocity) * dt;
        self.lag += self.lag_velocity * dt;
        // (Easing into its limit, not stopping dead at it.)
        let most = (self.len * s.beam_lag_max_deg.to_radians().tan()).max(1.0e-3);
        let wanted = self.lag * s.beam_lag;
        self.bend = wanted.normalize_or_zero() * most * (wanted.length() / most).tanh();
        self.curve = s.beam_lag_curve.max(1.0);
    }

    /// How far out along it (0..1) `along` metres is.
    fn share(&self, along: f32) -> f32 {
        (along / self.len.max(0.1)).clamp(1.0e-3, 1.0)
    }

    /// The point `along` metres out, relative to where it starts, with its
    /// bend (as `shaders/exhumer_beam.wgsl` draws it).
    fn point(&self, along: f32) -> Vec3 {
        self.dir * along + self.bend * self.share(along).powf(self.curve)
    }
}

/// A beam's ribbon, and its light.
#[derive(Component)]
struct BeamCore;

/// A puff of soul-smoke on a beam: where it ends, or the glow where it leaves
/// the barrel (`at_gun`). How fast (and which way) it turns, and its size.
/// Always square on to the camera, turning at its own pace whichever way the
/// beam's going.
#[derive(Component)]
struct BeamFlare {
    turn: f32,
    size: f32,
    at_gun: bool,
}

/// A puff of soul-smoke drawn back along a beam: how far along it (m from the
/// gun), its turn, how long it's been out (s), and whether it's mirrored.
#[derive(Component)]
struct Mote {
    beam: Entity,
    along: f32,
    spin: f32,
    seed: f32,
    age: f32,
    flip: Vec2,
}

#[derive(Resource)]
struct BeamAssets {
    ribbon: Handle<Mesh>,
}

/// How many pieces a beam's ribbon is along its length, to bend smoothly.
const RIBBON_SEGMENTS: u32 = 24;

/// A beam's ribbon: a quad like `Rectangle::new(1.0, 1.0)` (-0.5..0.5 across
/// X and along Y), cut along its length so the shader can bend it.
fn ribbon_mesh() -> Mesh {
    let rows = RIBBON_SEGMENTS + 1;
    let mut positions = Vec::with_capacity(rows as usize * 2);
    let mut uvs = Vec::with_capacity(rows as usize * 2);
    for row in 0..rows {
        let v = row as f32 / RIBBON_SEGMENTS as f32;
        for x in [-0.5f32, 0.5] {
            positions.push([x, v - 0.5, 0.0]);
            uvs.push([x + 0.5, 1.0 - v]);
        }
    }
    let mut indices = Vec::with_capacity(RIBBON_SEGMENTS as usize * 6);
    for row in 0..RIBBON_SEGMENTS {
        let (a, b, c, d) = (row * 2, row * 2 + 1, row * 2 + 2, row * 2 + 3);
        indices.extend([a, b, d, a, d, c]);
    }
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// The Exhumer's barrel in the Ray Gun model it's drawn with, and the very end
/// of it, in that part's own space.
const BARREL_NODE: &str = "barrel";
const BARREL_TIP: Vec3 = Vec3::new(-4.0231, -0.0552, -33.815);

/// Where our own beam comes from this frame: the camera ([`locate_own_beam`]),
/// and the end of the barrel as it shows on screen, if it's found.
#[derive(Resource, Default)]
struct OwnBeam {
    camera: Option<GlobalTransform>,
    start: Option<Vec3>,
    /// The view model's barrel, while it's the Exhumer's.
    barrel: Option<Entity>,
}

/// Find where the end of the barrel is on screen, through everything that
/// moves it (sway, bob, its animation). The gun's drawn by the view-model
/// camera, whose FOV isn't the world camera's the beam's drawn by, so it's
/// taken to the screen through the one and back out through the other at the
/// same depth (as `weapons::tracer_start`).
#[allow(clippy::too_many_arguments)]
fn locate_own_beam(
    settings: Res<ExhumerFxSettings>,
    weapon: Res<Weapon>,
    helper: bevy::transform::helper::TransformHelper,
    view_model: Query<(Entity, &crate::weapons::ViewModelAnimation), With<crate::weapons::ViewModel>>,
    children: Query<&Children>,
    names: Query<&Name>,
    vm_camera: Query<(Entity, &Projection), With<crate::player::ViewModelCamera>>,
    world_camera: Query<(Entity, &Projection), With<WorldModelCamera>>,
    mut own: ResMut<OwnBeam>,
) {
    let fov = |p: &Projection| match p {
        Projection::Perspective(p) => Some(p.fov),
        _ => None,
    };
    own.start = None;
    let Ok((world_cam, world_proj)) = world_camera.single() else {
        own.camera = None;
        return;
    };
    let Ok(cam) = helper.compute_global_transform(world_cam) else {
        own.camera = None;
        return;
    };
    own.camera = Some(cam);
    let holding = view_model.single().ok().filter(|(_, a)| a.weapon == shared::weapon::WeaponId::Exhumer);
    let Some((root, _)) = holding else {
        own.barrel = None;
        return;
    };
    if !weapon.beaming {
        return;
    }
    // (Found again whenever the model's been swapped from under us.)
    let tip = own
        .barrel
        .and_then(|barrel| helper.compute_global_transform(barrel).ok())
        .or_else(|| {
            let barrel = children
                .iter_descendants(root)
                .find(|e| names.get(*e).is_ok_and(|n| n.as_str() == BARREL_NODE))?;
            own.barrel = Some(barrel);
            helper.compute_global_transform(barrel).ok()
        })
        .map(|barrel| barrel.transform_point(BARREL_TIP));
    let (Some(tip), Ok((vm_cam, vm_proj))) = (tip, vm_camera.single()) else {
        own.barrel = None;
        return;
    };
    let (Ok(vm_at), Some(vm_fov), Some(world_fov)) =
        (helper.compute_global_transform(vm_cam), fov(vm_proj), fov(world_proj))
    else {
        return;
    };
    let p = vm_at.affine().inverse().transform_point3(tip) + settings.beam_start_nudge;
    let depth = -p.z;
    if depth < 0.05 {
        return;
    }
    let screen = p.truncate() / (depth * (vm_fov * 0.5).tan());
    let view = (screen * depth * (world_fov * 0.5).tan()).extend(-depth);
    own.start = Some(cam.transform_point(view));
}

/// Each beam that's on, drawn from its gun out to its reach (or the first
/// wall), with its light and hum — gone when it's let go.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw_beams(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    weapon: Res<Weapon>,
    sounds: Res<ExhumerSounds>,
    fx: Option<Res<ExplosionAssets>>,
    assets: Option<Res<BeamAssets>>,
    own: Res<OwnBeam>,
    (local, poses): (Query<&LocalId, With<GameClient>>, Query<(&PlayerId, &PlayerPose), With<Interpolated>>),
    rapier: ReadRapierContext,
    mut beams: Query<(Entity, &mut BeamVisual, &mut Transform, &Children), (Without<BeamCore>, Without<BeamFlare>)>,
    mut cores: Query<(&mut Transform, Option<&mut PointLight>), (With<BeamCore>, Without<BeamVisual>, Without<BeamFlare>)>,
    mut flares: Query<(&BeamFlare, &mut Transform), (Without<BeamCore>, Without<BeamVisual>)>,
    (mut meshes, mut beam_materials, smoke): (ResMut<Assets<Mesh>>, ResMut<Assets<ExhumerBeamMaterial>>, Option<Res<SoulSmoke>>),
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let s = &*settings;
    let (Some(fx), Some(smoke)) = (fx, smoke) else { return };
    let Some(assets) = assets else {
        commands.insert_resource(BeamAssets { ribbon: meshes.add(ribbon_mesh()) });
        return;
    };
    let me = local.iter().next().map(|l| l.0);
    let Some(cam) = own.camera else { return };
    let cam_pos = cam.translation();
    let cam_up = cam.up().as_vec3();
    // Every beam that's on now: where it comes from, then where it's aimed
    // from and which way. Ours from the end of the barrel to where we're
    // looking, theirs from where their gun would be.
    let mut on: Vec<(Option<PeerId>, Vec3, Vec3, Vec3)> = Vec::new();
    if weapon.beaming {
        let from = own
            .start
            .unwrap_or_else(|| cam.translation() + cam.right() * 0.14 - cam.up() * 0.13 + cam.forward() * 0.45);
        on.push((None, from, cam_pos, cam.forward().as_vec3()));
    }
    for (id, pose) in &poses {
        if pose.beaming && Some(id.0) != me {
            let dir = beam_dir(pose.yaw, pose.pitch);
            let right = Quat::from_rotation_y(pose.yaw) * Vec3::X;
            let from = pose.translation - Vec3::Y * 0.25 + right * 0.2 + dir * 0.5;
            on.push((Some(id.0), from, from, dir));
        }
    }
    for (e, beam, ..) in &beams {
        if !on.iter().any(|(p, ..)| *p == beam.peer) {
            commands.entity(e).despawn();
        }
    }
    let ctx = rapier.single().ok();
    let t = time.elapsed_secs();
    let flicker = 1.0 - s.beam_flicker * (0.5 + 0.5 * (t * 31.0).sin());
    for (peer, from, eye, forward) in on {
        // (Never nearer than the gun's end, up against a wall.)
        let hit = ctx
            .as_ref()
            .and_then(|r| r.cast_ray(eye, forward, BEAM_RANGE, true, QueryFilter::only_fixed()))
            .map_or(BEAM_RANGE, |(_, d)| d.max(0.7));
        let Some((_, mut beam, mut root, children)) = beams.iter_mut().find(|(_, b, ..)| b.peer == peer) else {
            // A new one: its ribbon (and light), the swirl where it ends,
            // its hum — ours not positional, theirs from where it starts.
            let mut hum = match peer {
                None => PlaybackSettings::LOOP,
                Some(_) => crate::positional_playback(Volume::Linear(s.beam_volume)),
            };
            hum.mode = crate::audio_engine::PlaybackMode::Loop;
            hum.volume = Volume::Linear(s.beam_volume);
            let c = s.color;
            *seq = seq.wrapping_add(1);
            let seed = 1.0 + crate::util::rand01(seq.wrapping_mul(2_654_435_761) ^ (t * 1000.0) as u32);
            // (Its own, for its bend — freed when it goes.)
            let material = beam_materials.add(ExhumerBeamMaterial { params: ExhumerBeamMaterial::params(s) });
            commands
                .spawn((
                    StateScoped(AppState::InGame),
                    BeamVisual {
                        peer,
                        from,
                        dir: forward,
                        len: 0.1,
                        reach: 0.0,
                        owed: 0.0,
                        seed,
                        material: material.clone(),
                        hit,
                        last_tip: from + forward * 0.1,
                        lag: Vec3::ZERO,
                        lag_velocity: Vec3::ZERO,
                        bend: Vec3::ZERO,
                        curve: 2.0,
                    },
                    Transform::from_translation(from),
                    Visibility::default(),
                ))
                .with_children(|b| {
                    b.spawn((
                        BeamCore,
                        Mesh3d(assets.ribbon.clone()),
                        MeshMaterial3d(material),
                        Transform::from_scale(Vec3::ZERO),
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                    b.spawn((
                        BeamCore,
                        PointLight {
                            color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
                            intensity: s.beam_light,
                            range: 6.0,
                            shadows_enabled: false,
                            ..default()
                        },
                        Transform::default(),
                    ));
                    // (Puffs turning over each other where it ends, each its
                    // own way and pace, and the glow at the barrel.)
                    for (turn, size, at_gun) in
                        [(0.9, 1.0, false), (-0.6, 1.35, false), (1.4, 0.7, false), (0.5, 1.0, true)]
                    {
                        b.spawn((
                            BeamFlare { turn, size, at_gun },
                            Mesh3d(fx.quad.clone()),
                            MeshMaterial3d(smoke.beam_puff.clone()),
                            Transform::from_scale(Vec3::ZERO),
                            NotShadowCaster,
                            NoFrustumCulling,
                        ));
                    }
                    b.spawn((AudioPlayer::new(sounds.beam.clone()), hum, Transform::default()));
                });
            continue;
        };
        // (Shooting out from the gun as it comes on — eased, quick off the
        // mark and slowing as it reaches — then held to wherever it hits.)
        beam.reach = (beam.reach + time.delta_secs() / s.beam_extend_secs.max(0.01)).min(1.0);
        let out = 1.0 - (1.0 - beam.reach).powi(2);
        // (Its length follows what it's hitting smoothly, however suddenly
        // that changes: the aim crossing the edge of something, say.)
        let ease = 1.0 - (-s.beam_reach_smooth.max(0.1) * time.delta_secs()).exp();
        beam.hit += (hit - beam.hit) * ease;
        let to = eye + forward * beam.hit - from;
        let dir = to.normalize_or(forward);
        let len = to.length().min(BEAM_RANGE * out).max(0.1);
        beam.from = from;
        beam.dir = dir;
        beam.len = len;
        beam.swing(s, time.delta_secs());
        beam.owed += s.motes_per_sec * time.delta_secs();
        root.translation = from;
        let seed = beam.seed;
        // (Its look follows the panel, its bend this beam's swing — only
        // touched when either's changed.)
        let mut params = ExhumerBeamMaterial::params(s);
        params.bend = beam.bend.extend(beam.curve);
        if beam_materials.get(&beam.material).is_some_and(|m| m.params != params) {
            if let Some(m) = beam_materials.get_mut(&beam.material) {
                m.params = params;
            }
        }
        for child in children.iter() {
            if let Ok((flare, mut tf)) = flares.get_mut(child) {
                // (Billowing where it takes hold, just short of what it hit,
                // breathing.)
                let breathe = 0.85 + 0.15 * (t * 3.0 + seed * 9.0 + flare.turn * 5.0).sin();
                let (along, size) = if flare.at_gun {
                    (0.02, s.beam_muzzle_glow * flare.size * breathe)
                } else {
                    ((len - 0.6).max(0.1), s.flare_size * flare.size * breathe)
                };
                let roll = (t * s.puff_spin + seed * 4.0) * flare.turn;
                let at = beam.point(along);
                *tf = Transform::from_translation(at)
                    .with_rotation(along_rotation(from + at, cam_up, cam_pos, roll))
                    .with_scale(Vec3::new(size, size, 1.0));
                continue;
            }
            let Ok((mut tf, light)) = cores.get_mut(child) else { continue };
            match light {
                Some(mut light) => {
                    tf.translation = beam.point(len * 0.5);
                    light.intensity = s.beam_light * flicker;
                }
                None => {
                    // (The ribbon's Y along the beam, its middle half way
                    // out; X its width at the far end; Z's scale its seed —
                    // the shader bends it and turns it to face us.)
                    *tf = Transform::from_translation(dir * len * 0.5)
                        .with_rotation(Quat::from_rotation_arc(Vec3::Y, dir))
                        .with_scale(Vec3::new(s.beam_width_far, len, seed));
                }
            }
        }
    }
    let mut spawn: Vec<(Entity, f32)> = Vec::new();
    for (e, mut beam, ..) in &mut beams {
        while beam.owed >= 1.0 {
            beam.owed -= 1.0;
            spawn.push((e, beam.len));
        }
    }
    for (i, (beam, len)) in spawn.into_iter().enumerate() {
        *seq = seq.wrapping_add(1);
        let r = |k: u32| crate::util::rand01((t * 1000.0) as u32 ^ (i as u32 + *seq).wrapping_mul(2_654_435_761) ^ k);
        let seed = r(1);
        let flip = Vec2::new(if r(2) < 0.5 { -1.0 } else { 1.0 }, if r(3) < 0.5 { -1.0 } else { 1.0 });
        commands.spawn((
            StateScoped(AppState::InGame),
            Mote { beam, along: len * (0.15 + 0.85 * seed), spin: r(4) * std::f32::consts::TAU, seed, age: 0.0, flip },
            Mesh3d(fx.quad.clone()),
            MeshMaterial3d(smoke.beam_puff.clone()),
            Transform::from_scale(Vec3::ZERO),
            NotShadowCaster,
            NoFrustumCulling,
        ));
    }
}

/// The puffs of soul-smoke on each beam, drawn back along it into the gun,
/// spread across it and slowly turning, growing in as they appear and
/// shrinking away as they get there — gone once there (or with their beam).
fn fly_motes(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    own: Res<OwnBeam>,
    beams: Query<&BeamVisual>,
    mut motes: Query<(Entity, &mut Mote, &mut Transform)>,
    mut commands: Commands,
) {
    let Some(cam) = own.camera else { return };
    let cam_pos = cam.translation();
    let (cam_up, cam_right) = (cam.up().as_vec3(), cam.right().as_vec3());
    let s = &*settings;
    let dt = time.delta_secs();
    for (e, mut mote, mut tf) in &mut motes {
        let Ok(beam) = beams.get(mote.beam) else {
            commands.entity(e).despawn();
            continue;
        };
        mote.along -= s.mote_speed * (0.7 + 0.6 * mote.seed) * dt;
        mote.spin += dt * s.puff_spin * (mote.seed - 0.5) * 2.0;
        mote.age += dt;
        if mote.along <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        // (Spread across it in the camera's plane, as wide as it is there.)
        let out = (mote.along / beam.len.max(0.1)).min(1.0);
        let along = mote.along.min(beam.len);
        let r = (s.beam_width_near + (s.beam_width_far - s.beam_width_near) * out) * 0.3;
        let round = mote.seed * std::f32::consts::TAU * 7.0 + mote.age * 0.5;
        let at = beam.from + beam.point(along) + (cam_right * round.cos() + cam_up * round.sin() * 0.6) * r;
        // (Bigger further out; growing in, not popping on.)
        let grow = (mote.age / 0.4).min(1.0);
        let size = s.mote_size * (0.6 + 0.8 * mote.seed) * (0.12 + 0.88 * out) * grow * grow * (3.0 - 2.0 * grow);
        *tf = Transform::from_translation(at)
            .with_rotation(along_rotation(at, cam_up, cam_pos, mote.spin))
            .with_scale(Vec3::new(size * mote.flip.x, size * mote.flip.y, 1.0));
    }
}

// --- souls in flight --------------------------------------------------------------------

/// The soul-smoke every soul, mote and beam's end is drawn in: the green
/// swirl (`textures/vfx/soul_swirl.png`), tinted the souls' colour, in three
/// strengths from a soul's bright head to its faint tail — and a soft puff
/// of smoke (`textures/vfx/smoke.png`) for the haze round a soul's head.
#[derive(Resource)]
pub(crate) struct SoulSmoke {
    swirls: [Handle<SoulSmokeMaterial>; 3],
    puff: Handle<SoulSmokeMaterial>,
    /// The soft puff the beam's own smoke is drawn in.
    beam_puff: Handle<SoulSmokeMaterial>,
}

/// How strong each of [`SoulSmoke`]'s swirls is, of `smoke_opacity` —
/// and its puff.
const SMOKE_TIERS: [f32; 3] = [1.0, 0.6, 0.32];
const PUFF_STRENGTH: f32 = 0.25;

impl SoulSmoke {
    /// The swirl at strength `tier` (0 brightest .. 2 faintest).
    pub(crate) fn swirl_material(&self, tier: usize) -> Handle<SoulSmokeMaterial> {
        self.swirls[tier.min(2)].clone()
    }

    /// The soft puff of haze.
    pub(crate) fn puff_material(&self) -> Handle<SoulSmokeMaterial> {
        self.puff.clone()
    }

    /// The swirl for the `i`th of a soul's `n` streaks (its head first).
    fn tier(&self, i: u32, n: u32) -> Handle<SoulSmokeMaterial> {
        let k = i as f32 / (n.max(1)) as f32;
        self.swirls[if k < 0.25 { 0 } else if k < 0.6 { 1 } else { 2 }].clone()
    }

    fn params(s: &ExhumerFxSettings, strength: f32) -> SmokeParams {
        let [r, g, b] = s.color;
        SmokeParams { color: Vec4::new(r, g, b, s.wisp_glow * s.smoke_opacity * strength), soft: s.smoke_soft.max(0.01), ..default() }
    }

    fn beam_params(s: &ExhumerFxSettings) -> SmokeParams {
        let [r, g, b] = s.color;
        SmokeParams {
            color: Vec4::new(r * 0.45, g * 0.9, b * 0.5, s.beam_puff_glow),
            soft: s.smoke_soft.max(0.01),
            body: s.beam_body * 0.5,
            ..default()
        }
    }
}

/// Make the soul-smoke (once the explosion's smoke is in), and keep it
/// following the panel.
fn soul_smoke(
    settings: Res<ExhumerFxSettings>,
    fx: Option<Res<ExplosionAssets>>,
    smoke: Option<Res<SoulSmoke>>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<SoulSmokeMaterial>>,
    mut commands: Commands,
) {
    let s = &*settings;
    let Some(smoke) = smoke else {
        let Some(fx) = fx else { return };
        let swirl: Handle<Image> = asset_server.load(SWIRL_TEXTURE);
        let swirls = [0, 1, 2].map(|tier| {
            materials.add(SoulSmokeMaterial { params: SoulSmoke::params(s, SMOKE_TIERS[tier]), texture: swirl.clone() })
        });
        let puff = materials.add(SoulSmokeMaterial { params: SoulSmoke::params(s, PUFF_STRENGTH), texture: fx.smoke.clone() });
        let beam_puff = materials.add(SoulSmokeMaterial { params: SoulSmoke::beam_params(s), texture: fx.smoke.clone() });
        commands.insert_resource(SoulSmoke { swirls, puff, beam_puff });
        return;
    };
    if !settings.is_changed() {
        return;
    }
    let all = smoke
        .swirls
        .iter()
        .zip(SMOKE_TIERS)
        .map(|(handle, strength)| (handle, SoulSmoke::params(s, strength)))
        .chain([(&smoke.puff, SoulSmoke::params(s, PUFF_STRENGTH)), (&smoke.beam_puff, SoulSmoke::beam_params(s))]);
    for (handle, params) in all {
        if materials.get(handle).is_some_and(|m| m.params != params) {
            if let Some(m) = materials.get_mut(handle) {
                m.params = params;
            }
        }
    }
}

/// Where a wisp's going.
#[derive(Clone, Copy)]
enum WispTo {
    /// Back into this player's Exhumer.
    Shooter(PeerId),
    /// Into this zombie (let out after it).
    Zombie(PeerId),
}

/// A soul in flight: from `from` to its target over `secs`, bowing to one
/// side (`side`); `last` is where its target was last seen.
#[derive(Component)]
struct Wisp {
    from: Vec3,
    to: WispTo,
    last: Vec3,
    side: Vec3,
    age: f32,
    secs: f32,
}

/// One part of a soul in flight: a streak of the swirl — its head (`i` 0)
/// or one trailing — laid along the way it's going, or (`puff`) the soft
/// haze round its head. Each with its own place in the swirl, size,
/// wavering and mirroring.
#[derive(Component)]
struct WispPart {
    i: u32,
    puff: bool,
    phase: f32,
    size: f32,
    turn: f32,
    flip: Vec2,
}

#[allow(clippy::too_many_arguments)]
fn spawn_wisp(
    commands: &mut Commands,
    fx: &ExplosionAssets,
    smoke: &SoulSmoke,
    s: &ExhumerFxSettings,
    from: Vec3,
    to: WispTo,
    last: Vec3,
    secs: f32,
    seed: u32,
) {
    let r = |k: u32| crate::util::rand01(seed.wrapping_mul(2_654_435_761).wrapping_add(k));
    let side = Vec3::new(r(1) * 2.0 - 1.0, (r(2) * 2.0 - 1.0) * 0.4, r(3) * 2.0 - 1.0).normalize_or(Vec3::X);
    let n = s.trail.max(1);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            Wisp { from, to, last, side, age: 0.0, secs: secs.max(0.05) },
            Transform::from_translation(from),
            Visibility::default(),
        ))
        .with_children(|w| {
            let part = |i: u32, puff: bool, k: u32| WispPart {
                i,
                puff,
                phase: r(k) * std::f32::consts::TAU,
                size: 0.75 + 0.5 * r(k + 1),
                turn: if r(k + 2) < 0.5 { -1.0 } else { 1.0 } * (0.5 + r(k + 3)),
                flip: Vec2::new(if r(k + 4) < 0.5 { -1.0 } else { 1.0 }, if r(k + 5) < 0.5 { -1.0 } else { 1.0 }),
            };
            for i in 0..=n {
                w.spawn((
                    part(i, false, 10 + i * 7),
                    Mesh3d(fx.quad.clone()),
                    MeshMaterial3d(smoke.tier(i, n)),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
            }
            for j in 0..2 {
                w.spawn((
                    part(j, true, 500 + j * 7),
                    Mesh3d(fx.quad.clone()),
                    MeshMaterial3d(smoke.puff.clone()),
                    Transform::from_scale(Vec3::ZERO),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
            }
        });
}

/// Souls torn out: each streaks back into its shooter's Exhumer — and ours
/// holds as many as the server says.
#[allow(clippy::too_many_arguments)]
fn receive_souls(
    settings: Res<ExhumerFxSettings>,
    sounds: Res<ExhumerSounds>,
    fx: Option<Res<ExplosionAssets>>,
    smoke: Option<Res<SoulSmoke>>,
    mut receivers: Query<&mut MessageReceiver<ExhumerSouls>>,
    local: Query<&LocalId, With<GameClient>>,
    mut weapon: ResMut<Weapon>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if Some(msg.shooter) == me && weapon.souls != msg.souls {
                weapon.souls = msg.souls;
            }
            if let Some(first) = msg.from.first() {
                sound_at(&mut commands, &sounds.tear, Vec3::from_array(*first), settings.volume);
            }
            let (Some(fx), Some(smoke)) = (fx.as_deref(), smoke.as_deref()) else { continue };
            for from in &msg.from {
                *seq = seq.wrapping_add(1);
                let from = Vec3::from_array(*from);
                let to = WispTo::Shooter(msg.shooter);
                spawn_wisp(&mut commands, fx, smoke, &settings, from, to, from, settings.return_secs, *seq);
            }
        }
    }
}

/// The souls let out: one after each zombie the server sent one for.
fn receive_swarms(
    settings: Res<ExhumerFxSettings>,
    sounds: Res<ExhumerSounds>,
    fx: Option<Res<ExplosionAssets>>,
    smoke: Option<Res<SoulSmoke>>,
    mut receivers: Query<&mut MessageReceiver<ExhumerSwarm>>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let origin = Vec3::from_array(msg.origin);
            sound_at(&mut commands, &sounds.release, origin, settings.volume);
            let (Some(fx), Some(smoke)) = (fx.as_deref(), smoke.as_deref()) else { continue };
            for (zombie, at) in &msg.targets {
                *seq = seq.wrapping_add(1);
                let at = Vec3::from_array(*at);
                let secs = origin.distance(at) / SWARM_SPEED + 0.2;
                spawn_wisp(&mut commands, fx, smoke, &settings, origin, WispTo::Zombie(*zombie), at, secs, *seq);
            }
        }
    }
}

/// Every soul on its way: a trail of the green swirl's streaks arcing from
/// where it left to where its target is now, each laid along the way it's
/// going and a little behind the one before — so together they read as one
/// long, writhing ribbon of smoke — wavering about its path, the tail
/// fainter and longer than the head; a soft haze round its head. Gone once
/// it's there.
#[allow(clippy::type_complexity)]
fn fly_wisps(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    local: Query<&LocalId, With<GameClient>>,
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    poses: Query<(&PlayerId, &PlayerPose), With<Interpolated>>,
    mut wisps: Query<(Entity, &mut Wisp, &mut Transform, &Children), Without<WispPart>>,
    mut parts: Query<(&WispPart, &mut Transform), Without<Wisp>>,
    mut commands: Commands,
) {
    let Some(cam) = camera.map(|c| c.into_inner()) else { return };
    let me = local.iter().next().map(|l| l.0);
    let (cam_pos, cam_up, cam_right) = (cam.translation(), cam.up().as_vec3(), cam.right().as_vec3());
    let s = &*settings;
    let dt = time.delta_secs();
    let n = s.trail.max(1) as f32;
    let pose_of = |peer: PeerId| poses.iter().find(|(id, _)| id.0 == peer).map(|(_, p)| p.translation);
    for (e, mut wisp, mut root, children) in &mut wisps {
        wisp.age += dt;
        // Where its target is now.
        let target = match wisp.to {
            WispTo::Shooter(p) if Some(p) == me => Some(cam_pos + cam.forward() * 0.6 - cam_up * 0.25),
            WispTo::Shooter(p) => pose_of(p).map(|eye| eye - Vec3::Y * 0.35),
            WispTo::Zombie(p) => pose_of(p).map(|eye| eye - Vec3::Y * (EYE_HEIGHT - 1.2)),
        };
        if let Some(t) = target {
            wisp.last = t;
        }
        if wisp.age >= wisp.secs {
            commands.entity(e).despawn();
            continue;
        }
        let (from, to, side, secs) = (wisp.from, wisp.last, wisp.side, wisp.secs);
        let at = |age: f32| {
            let u = ease(age / secs);
            let mid = from.lerp(to, 0.5) + Vec3::Y * s.arc + side * s.arc * 0.6;
            from.lerp(mid, u).lerp(mid.lerp(to, u), u)
        };
        let head = at(wisp.age);
        root.translation = head;
        // (Billowing out as it leaves, drawn in thin as it arrives.)
        let u = wisp.age / secs;
        let swell = smoothstep(0.0, 0.15, u) * (1.0 - 0.7 * smoothstep(0.7, 1.0, u));
        for child in children.iter() {
            let Ok((part, mut tf)) = parts.get_mut(child) else { continue };
            if part.puff {
                // The haze round its head.
                let size = (s.wisp_size * 0.6 * part.size * swell).max(1.0e-4);
                let roll = part.phase + wisp.age * s.puff_spin * part.turn;
                *tf = Transform::from_rotation(smoke_billboard_rotation(head, cam_pos, cam_up, cam_right, roll))
                    .with_scale(Vec3::new(size, size, 1.0));
                continue;
            }
            let age = wisp.age - part.i as f32 * s.trail_lag;
            if age < 0.0 {
                tf.scale = Vec3::ZERO;
                continue;
            }
            let k = part.i as f32 / n;
            let pos = at(age);
            // (The way it's going there — each streak laid along it.)
            let going = (at((age + 0.03).min(secs)) - at((age - 0.03).max(0.0))).normalize_or(cam_up);
            let a = going.any_orthonormal_vector();
            let b = going.cross(a);
            let swirl = part.phase + age * 3.0 * part.turn;
            let pos = pos + (a * swirl.cos() + b * swirl.sin()) * s.wisp_spread * (0.2 + 0.8 * k);
            // (Shrinking away at the very end of its tail.)
            let tail = 1.0 - smoothstep(0.75, 1.0, k) * 0.5;
            let size = (s.wisp_size * part.size * (0.75 + 0.6 * k) * swell * tail).max(1.0e-4);
            let roll = 0.3 * (age * 4.0 * part.turn + part.phase).sin();
            *tf = Transform::from_translation(pos - head)
                .with_rotation(along_rotation(pos, going, cam_pos, roll))
                .with_scale(Vec3::new(size * part.flip.x, size * 0.5 * part.flip.y, 1.0));
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    ease((x - e0) / (e1 - e0))
}


// --- on the altar -----------------------------------------------------------------------

/// The glow round an Exhumer waiting on the pyramid's altar (a
/// `WeaponDrop` of it with an owner, `server::easter_egg::ascension`) —
/// gone the moment it's taken.
#[derive(Component)]
struct AltarGlow {
    src: Entity,
    seed: f32,
}

/// One streak of an [`AltarGlow`]'s swirl (its `i`th of [`ALTAR_STREAKS`]).
#[derive(Component)]
struct AltarStreak(u32);

const ALTAR_STREAKS: u32 = 4;

/// A soft green light on each Exhumer waiting on the altar, pulsing slowly,
/// and the souls' smoke circling it, rising and falling — so they read as
/// the prize the moment the squad arrives.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn altar_glows(
    time: Res<Time>,
    settings: Res<ExhumerFxSettings>,
    (fx, smoke): (Option<Res<ExplosionAssets>>, Option<Res<SoulSmoke>>),
    camera: Option<Single<&GlobalTransform, With<WorldModelCamera>>>,
    drops: Query<(Entity, &shared::WeaponDrop)>,
    mut glows: Query<(Entity, &AltarGlow, &mut Transform, &Children), Without<AltarStreak>>,
    mut streaks: Query<(&AltarStreak, &mut Transform, Option<&mut PointLight>), Without<AltarGlow>>,
    mut commands: Commands,
) {
    let s = &*settings;
    let on_altar = |d: &shared::WeaponDrop| d.owner.is_some() && d.weapon == shared::weapon::SlotWeapon::Gun(shared::weapon::WeaponId::Exhumer);
    for (e, glow, ..) in &glows {
        if !drops.get(glow.src).is_ok_and(|(_, d)| on_altar(d)) {
            commands.entity(e).despawn();
        }
    }
    let (Some(fx), Some(smoke)) = (fx, smoke) else { return };
    for (src, drop) in &drops {
        if !on_altar(drop) || glows.iter().any(|(_, g, ..)| g.src == src) {
            continue;
        }
        let c = s.color;
        let seed = crate::util::rand01(src.index() ^ 0x5eed);
        commands
            .spawn((
                StateScoped(AppState::InGame),
                AltarGlow { src, seed },
                Transform::from_translation(drop.pos + Vec3::Y * 0.12),
                Visibility::default(),
            ))
            .with_children(|g| {
                g.spawn((
                    AltarStreak(u32::MAX),
                    PointLight {
                        color: Color::LinearRgba(LinearRgba::rgb(c[0], c[1], c[2])),
                        intensity: 0.0,
                        range: 3.5,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(Vec3::Y * 0.2),
                ));
                for i in 0..ALTAR_STREAKS {
                    g.spawn((
                        AltarStreak(i),
                        Mesh3d(fx.quad.clone()),
                        MeshMaterial3d(smoke.swirl_material(1 + (i % 2) as usize)),
                        Transform::from_scale(Vec3::ZERO),
                        NotShadowCaster,
                        NoFrustumCulling,
                    ));
                }
            });
    }
    let Some(cam) = camera.map(|c| c.into_inner()) else { return };
    let cam_pos = cam.translation();
    let t = time.elapsed_secs();
    for (_, glow, root, children) in &mut glows {
        let centre = root.translation;
        for child in children.iter() {
            let Ok((streak, mut tf, light)) = streaks.get_mut(child) else { continue };
            if let Some(mut light) = light {
                light.intensity = s.altar_light * (0.8 + 0.2 * (t * 1.7 + glow.seed * 9.0).sin());
                continue;
            }
            // (Each on its own tilted orbit, a quarter turn apart, rising
            // and falling.)
            let k = streak.0 as f32;
            let a = t * s.altar_swirl_speed + k * std::f32::consts::FRAC_PI_2 + glow.seed * 6.0;
            let bob = 0.12 * (t * 0.8 + k * 1.9).sin();
            let r = s.altar_swirl_radius * (0.8 + 0.2 * (t * 1.3 + k).sin());
            let p = Vec3::new(a.cos() * r, 0.1 + bob, a.sin() * r);
            let going = Vec3::new(-a.sin(), 0.15 * (t * 0.8 + k * 1.9).cos(), a.cos());
            let len = s.altar_swirl_len * (0.85 + 0.15 * (t * 2.1 + k * 3.0).sin());
            let flip = if streak.0 % 2 == 0 { 1.0 } else { -1.0 };
            *tf = Transform::from_translation(p)
                .with_rotation(along_rotation(centre + p, going, cam_pos, 0.0))
                .with_scale(Vec3::new(len, len * 0.5 * flip, 1.0));
        }
    }
}

// --- the debug panel --------------------------------------------------------------------

/// Between the debug panel's "The Exhumer" section and the game: what it
/// shows of ours, and its ask to be given one ([`debug_give`]). (The panel's
/// system already holds `Weapon` and a `GiveWeapon` sender elsewhere.)
#[derive(Resource, Default)]
pub(crate) struct ExhumerDebug {
    give: bool,
    souls: u32,
    beaming: bool,
    carried: bool,
}

/// The panel's ask to be given the Exhumer (in place of the gun in hand,
/// like a wall buy, free) — and what it shows, kept up to date.
fn debug_give(
    mut debug: ResMut<ExhumerDebug>,
    weapon: Res<Weapon>,
    mut tx: Query<&mut TriggerSender<shared::GiveWeapon>, With<GameClient>>,
) {
    let carried = weapon.carries(shared::weapon::SlotWeapon::Gun(shared::weapon::WeaponId::Exhumer));
    if (debug.souls, debug.beaming, debug.carried) != (weapon.souls, weapon.beaming, carried) {
        (debug.souls, debug.beaming, debug.carried) = (weapon.souls, weapon.beaming, carried);
    }
    if !std::mem::take(&mut debug.give) || carried {
        return;
    }
    let (mag, reserve) = weapon.held_ammo();
    if let Ok(mut tx) = tx.single_mut() {
        tx.trigger::<shared::LobbyChannel>(shared::GiveWeapon {
            weapon: shared::weapon::WeaponId::Exhumer,
            slot: weapon.held as u8,
            mag,
            reserve,
        });
    }
}

/// The debug panel's controls for it.
pub(crate) fn exhumer_section(ui: &mut egui::Ui, s: &mut ExhumerFxSettings, debug: &mut ExhumerDebug, in_zombies: bool) {
    ui.label(format!(
        "souls: {}/{}  ·  beam {}",
        debug.souls,
        shared::exhumer::MAX_SOULS,
        if debug.beaming { "on" } else { "off" }
    ));
    if ui
        .add_enabled(in_zombies && !debug.carried, egui::Button::new("Give me the Exhumer (in place of the gun in hand)"))
        .clicked()
    {
        debug.give = true;
    }
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut s.color);
        ui.label("souls' colour");
    });
    ui.add(egui::Slider::new(&mut s.beam_extend_secs, 0.01f32..=1.5).text("beam shoots out over (s)"));
    ui.add(egui::Slider::new(&mut s.beam_reach_smooth, 1.0f32..=60.0).text("beam's length follows the aim (1/s)"));
    ui.label("Our beam's start, from the end of the barrel (m)");
    ui.add(egui::Slider::new(&mut s.beam_start_nudge.x, -0.1f32..=0.1).text("right"));
    ui.add(egui::Slider::new(&mut s.beam_start_nudge.y, -0.1f32..=0.1).text("up"));
    ui.add(egui::Slider::new(&mut s.beam_start_nudge.z, -0.2f32..=0.2).text("back"));
    ui.add(egui::Slider::new(&mut s.beam_lag, 0.0f32..=3.0).text("beam's end trails behind (0 = straight)"));
    ui.add(egui::Slider::new(&mut s.beam_lag_hz, 0.2f32..=10.0).text("beam's end catches up (Hz, lower = looser)"));
    ui.add(egui::Slider::new(&mut s.beam_lag_damping, 0.1f32..=1.5).text("beam's end settles (lower = overshoots)"));
    ui.add(egui::Slider::new(&mut s.beam_lag_max_deg, 0.0f32..=30.0).text("beam's end trails at most (°)"));
    ui.add(egui::Slider::new(&mut s.beam_lag_curve, 1.0f32..=4.0).text("beam's bend toward its end (1 = even)"));
    ui.add(egui::Slider::new(&mut s.beam_width_near, 0.01f32..=1.0).text("beam width at the gun (m)"));
    ui.add(egui::Slider::new(&mut s.beam_width_far, 0.05f32..=6.0).text("beam width at its end (m)"));
    ui.add(egui::Slider::new(&mut s.beam_glow, 0.0f32..=12.0).text("beam brightness"));
    ui.add(egui::Slider::new(&mut s.beam_flicker, 0.0f32..=1.0).text("beam flicker"));
    ui.add(egui::Slider::new(&mut s.beam_flow, -30.0f32..=30.0).text("beam's smoke drifts in (m/s, negative = out)"));
    ui.add(egui::Slider::new(&mut s.beam_turbulence, 0.0f32..=2.0).text("beam's edges billow"));
    ui.add(egui::Slider::new(&mut s.beam_heat, 0.05f32..=15.0).text("beam burns pale out to (m)"));
    ui.add(egui::Slider::new(&mut s.beam_body, 0.0f32..=1.0).text("beam's smoke dims what's behind it"));
    ui.add(egui::Slider::new(&mut s.beam_muzzle_glow, 0.0f32..=3.0).text("glow at the barrel (m)"));
    ui.add(egui::Slider::new(&mut s.beam_puff_glow, 0.0f32..=4.0).text("beam's puffs of smoke: brightness"));
    ui.add(egui::Slider::new(&mut s.beam_smoke, 0.0f32..=1.0).text("beam smokiness"));
    ui.add(egui::Slider::new(&mut s.flare_size, 0.0f32..=8.0).text("smoke where the beam ends (m)"));
    ui.add(egui::Slider::new(&mut s.beam_light, 0.0f32..=1_000_000.0).text("beam light (lm)"));
    ui.add(egui::Slider::new(&mut s.motes_per_sec, 0.0f32..=200.0).text("puffs along the beam (/s)"));
    ui.add(egui::Slider::new(&mut s.mote_speed, 1.0f32..=60.0).text("puffs along the beam: speed (m/s)"));
    ui.add(egui::Slider::new(&mut s.mote_size, 0.02f32..=6.0).text("puffs along the beam: size (m)"));
    ui.add(egui::Slider::new(&mut s.wisp_size, 0.05f32..=2.0).text("soul streak length (m)"));
    ui.add(egui::Slider::new(&mut s.wisp_glow, 0.0f32..=20.0).text("soul smoke brightness"));
    ui.add(egui::Slider::new(&mut s.smoke_opacity, 0.0f32..=1.0).text("soul smoke strength"));
    ui.add(egui::Slider::new(&mut s.trail, 1..=30).text("soul streaks (next ones)"));
    ui.add(egui::Slider::new(&mut s.trail_lag, 0.005f32..=0.2).text("soul streak spacing (s)"));
    ui.add(egui::Slider::new(&mut s.wisp_spread, 0.0f32..=1.0).text("soul smoke swirl (m)"));
    ui.add(egui::Slider::new(&mut s.puff_spin, 0.0f32..=6.0).text("smoke at the beam's end turns (rad/s)"));
    ui.add(egui::Slider::new(&mut s.smoke_soft, 0.0f32..=3.0).text("smoke fades before walls (m)"));
    ui.add(egui::Slider::new(&mut s.return_secs, 0.1f32..=3.0).text("soul back to the gun (s)"));
    ui.add(egui::Slider::new(&mut s.arc, 0.0f32..=6.0).text("soul's arc (m)"));
    ui.add(egui::Slider::new(&mut s.drain_tilt_deg, 0.0f32..=45.0).text("body being drained tips back (°)"));
    ui.add(egui::Slider::new(&mut s.drain_shake_deg, 0.0f32..=20.0).text("body being drained shudders (°)"));
    ui.add(egui::Slider::new(&mut s.lift_spin, 0.0f32..=6.0).text("exhumed body turns (rad/s)"));
    ui.add(egui::Slider::new(&mut s.lift_tilt_deg, 0.0f32..=90.0).text("exhumed body tips back (°)"));
    ui.add(egui::Slider::new(&mut s.chest_light, 0.0f32..=1_000_000.0).text("glow in its chest (lm)"));
    ui.add(
        egui::Slider::new(&mut s.altar_light, 0.0f32..=1_000_000.0)
            .logarithmic(true)
            .text("on the altar: light (lm)"),
    );
    ui.add(egui::Slider::new(&mut s.altar_swirl_len, 0.05f32..=2.0).text("on the altar: swirl streak length (m)"));
    ui.add(egui::Slider::new(&mut s.altar_swirl_radius, 0.0f32..=1.5).text("on the altar: swirl out (m)"));
    ui.add(egui::Slider::new(&mut s.altar_swirl_speed, 0.0f32..=6.0).text("on the altar: swirl speed (rad/s)"));
    ui.add(egui::Slider::new(&mut s.volume, 0.0f32..=3.0).text("sounds' volume"));
    ui.add(egui::Slider::new(&mut s.beam_volume, 0.0f32..=3.0).text("beam hum volume (next time it's on)"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = ExhumerFxSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
