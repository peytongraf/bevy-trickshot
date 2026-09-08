//! Kill-cam replay. On a bot kill the server ships the killer's last ~3 s
//! ([`shared::KillCam`]); in solo Practice the same window is cut from a local
//! ring buffer. Playback flies the *existing* player rig along the recorded
//! camera path (full world transform, so shake / recoil / crouch are baked in),
//! seek-drives the first-person weapon animation, re-fires the recorded one-shot
//! sounds + muzzle flash + barrel smoke + ground bursts, and stands in ghost
//! copies of the bots frozen at the kill so the one that was hit topples on cue.
//! A top banner names the killer; `F` (rebindable) skips.

use std::collections::VecDeque;

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;

use shared::bots::{BOT_FALL_SECS, BOT_HEIGHT, BOT_RADIUS};
use shared::KillCamSample;

use crate::keybinds::KeyBindings;
use crate::settings::Settings;
use crate::{
    rand_roll, Ads, AppState, CameraRecoil, CameraShake, GameSounds, GroundImpact,
    MuzzleFlashState, Player, PlayerHead, SmokeEmission, TargetBotVisual, ViewModelAnimation,
    Weapon, WorldModelCamera,
};

/// Read the first-person weapon animation's current playhead (seconds), for
/// recording. `0.0` if the player / clip isn't ready yet.
pub(crate) fn viewmodel_anim_time(
    players: &Query<&AnimationPlayer>,
    view_models: &Query<&ViewModelAnimation>,
) -> f32 {
    view_models
        .iter()
        .next()
        .zip(players.iter().next())
        .and_then(|(vm, ap)| ap.animation(vm.index).map(|a| a.seek_time()))
        .unwrap_or(0.0)
}

/// One-shot sound bits, packed into `PlayerInput::sound_bits` / recorded locally.
pub(crate) const SND_SHOT: u8 = 1 << 0;
pub(crate) const SND_RELOAD: u8 = 1 << 1;
pub(crate) const SND_RECHAMBER: u8 = 1 << 2;
pub(crate) const SND_SLIDE: u8 = 1 << 3;
pub(crate) const SND_DIVE: u8 = 1 << 4;
pub(crate) const SND_AIM_IN: u8 = 1 << 5;
pub(crate) const SND_AIM_OUT: u8 = 1 << 6;

/// Seconds of replay before / after the kill.
const PRE_SECS: f32 = 2.0;
const POST_SECS: f32 = 1.0;
/// A touch more than `PRE + POST`, so the local ring always covers the window.
const LOCAL_KEEP_SECS: f32 = 4.0;

/// Sounds the player triggered since the last input packet / replay frame.
/// `write_input` drains it in a networked game; `record_local_replay` drains it
/// in Practice.
#[derive(Resource, Default)]
pub(crate) struct ReplaySoundBits(pub(crate) u8);

impl ReplaySoundBits {
    pub(crate) fn note(&mut self, bit: u8) {
        self.0 |= bit;
    }
}

/// The local player's most recent ground-impact point that hasn't yet been
/// stamped onto a [`PlayerInput`]. Set by `resolve_local_shot`, drained by
/// `write_input` so the networked kill cam can replay ground bursts. (Practice
/// records its own bursts straight off the [`GroundImpact`] event stream.)
#[derive(Resource, Default)]
pub(crate) struct ReplayGroundImpact(pub(crate) Option<Vec3>);

/// Rolling local recording for Practice (and the follow-through second even in a
/// networked game isn't needed here — the server owns that path).
#[derive(Resource, Default)]
struct LocalReplay {
    frames: VecDeque<(f32, KillCamSample)>,
    /// `(time, world point)` of ground-impact bursts, so Practice replays the
    /// rock / dust too. (Networked kill cams don't carry these yet.)
    impacts: VecDeque<(f32, Vec3)>,
}

/// Practice: a kill landed at `kill_at`; assemble + start the cam at `fire_at`.
#[derive(Resource, Default)]
pub(crate) struct PendingLocalCam(pub(crate) Option<PendingLocal>);

pub(crate) struct PendingLocal {
    pub(crate) kill_at: f32,
    pub(crate) fire_at: f32,
    /// Every bot alive at the kill: `(pos, yaw, was_the_one_shot)`.
    pub(crate) bots: Vec<(Vec3, f32, bool)>,
}

/// The replay currently playing, if any.
#[derive(Resource, Default)]
pub(crate) struct ActiveKillCam(pub(crate) Option<KillCamRun>);

pub(crate) struct KillCamRun {
    pub(crate) killer_name: String,
    /// `(seconds-from-start, sample)`, oldest first.
    frames: Vec<(f32, KillCamSample)>,
    /// `(seconds-from-start, world point)` ground bursts to re-emit (Practice).
    impacts: Vec<(f32, Vec3)>,
    /// Seconds-from-start the shot landed — when the hit ghost starts to topple.
    kill_time: f32,
    /// Bots frozen at the kill: `(pos, yaw, was_the_one_shot)`.
    bots: Vec<(Vec3, f32, bool)>,
    /// Ghost bot entities spawned for the replay (despawned on teardown).
    ghosts: Vec<Entity>,
    elapsed: f32,
    /// Index of the next frame whose sounds still need firing.
    sound_cursor: usize,
    /// Index of the next ground burst to re-emit.
    impact_cursor: usize,
    /// Set once `start_killcam` has stashed the rig + built the banner.
    setup: bool,
    saved: Option<SavedRig>,
    banner: Option<Entity>,
}

/// The player rig's transforms, stashed so playback can drive them and then put
/// them back exactly.
struct SavedRig {
    player: Transform,
    head: Transform,
    /// `Ads::t` at the instant playback took over, restored on teardown so live
    /// aiming resumes exactly where it left off.
    ads_t: f32,
}

#[derive(Component)]
struct KillCamBanner;

/// A stand-in bot shown during the replay, frozen where it stood at the kill.
#[derive(Component)]
struct KillCamGhost {
    pos: Vec3,
    yaw: f32,
    /// This is the bot that was shot — topples once `elapsed >= kill_time`.
    killed: bool,
    /// `0.0` upright … `1.0` flat.
    fall: f32,
}

/// Shared capsule + material for the ghost bots (built at startup).
#[derive(Resource)]
struct KillCamAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// Filter for the four rig entities `start_killcam` / `drive_killcam` steer.
/// `Without<KillCamGhost>` keeps the `&mut Transform` access disjoint from the
/// ghost query in `drive_killcam`.
type RigFilter = (
    Or<(
        With<Player>,
        With<PlayerHead>,
        With<CameraShake>,
        With<CameraRecoil>,
    )>,
    Without<KillCamGhost>,
);

/// Which of the four rig markers an entity carries.
type RigTags = (
    Has<Player>,
    Has<PlayerHead>,
    Has<CameraShake>,
    Has<CameraRecoil>,
);

pub struct KillCamPlugin;

impl Plugin for KillCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReplaySoundBits>()
            .init_resource::<ReplayGroundImpact>()
            .init_resource::<LocalReplay>()
            .init_resource::<PendingLocalCam>()
            .init_resource::<ActiveKillCam>()
            // NOTE: build the ghost mesh / material in `PostStartup`, *not*
            // `Startup`. `main`'s `Startup` set spawns the 3D cameras, and adding
            // any extra system to that same schedule perturbs its execution order
            // enough that the primary window comes up with a black swapchain that
            // never recovers (the menu / lobby UI still draws on top, so only the
            // in-game world is affected). `PostStartup` runs after that set has
            // finished and its commands have flushed, so the camera setup is
            // undisturbed.
            .add_systems(PostStartup, setup_killcam_assets)
            .add_systems(OnExit(AppState::InGame), stop_killcam)
            .add_systems(
                Update,
                (
                    record_local_replay
                        .run_if(crate::practice::is_practice.and(no_killcam)),
                    start_local_killcam,
                    start_killcam,
                    drive_killcam,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Run condition: gameplay input is live (no replay playing).
pub(crate) fn no_killcam(active: Res<ActiveKillCam>) -> bool {
    active.0.is_none()
}

fn setup_killcam_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(KillCamAssets {
        mesh: meshes.add(Capsule3d::new(BOT_RADIUS, BOT_HEIGHT - 2.0 * BOT_RADIUS)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.95, 0.55, 0.15),
            perceptual_roughness: 0.8,
            ..default()
        }),
    });
}

fn sound_for<'a>(sounds: &'a GameSounds, bit: u8) -> Option<&'a Handle<AudioSource>> {
    Some(match bit {
        SND_SHOT => &sounds.shot,
        SND_RELOAD => &sounds.reload,
        SND_RECHAMBER => &sounds.rechamber,
        SND_SLIDE => &sounds.slide,
        SND_DIVE => &sounds.dive,
        SND_AIM_IN => &sounds.aim_in,
        SND_AIM_OUT => &sounds.aim_out,
        _ => return None,
    })
}

// --- recording (Practice) --------------------------------------------

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn record_local_replay(
    time: Res<Time>,
    ads: Res<Ads>,
    mut bits: ResMut<ReplaySoundBits>,
    mut replay: ResMut<LocalReplay>,
    mut impacts: EventReader<GroundImpact>,
    cam: Query<&GlobalTransform, With<WorldModelCamera>>,
    anim_players: Query<&AnimationPlayer>,
    view_models: Query<&ViewModelAnimation>,
) {
    let Ok(cam) = cam.single() else {
        return;
    };
    let now = time.elapsed_secs();
    let (_, rot, pos) = cam.to_scale_rotation_translation();
    replay.frames.push_back((
        now,
        KillCamSample {
            cam_pos: pos.to_array(),
            cam_rot: rot.to_array(),
            sound_bits: std::mem::take(&mut bits.0),
            anim_time: viewmodel_anim_time(&anim_players, &view_models),
            ads_t: ads.t,
            // Practice replays its ground bursts from `LocalReplay::impacts`
            // (filled just below), so the per-frame slot stays empty here.
            ground_pt: None,
        },
    ));
    for ev in impacts.read() {
        replay.impacts.push_back((now, ev.0));
    }
    while replay
        .frames
        .front()
        .is_some_and(|(t, _)| now - *t > LOCAL_KEEP_SECS)
    {
        replay.frames.pop_front();
    }
    while replay
        .impacts
        .front()
        .is_some_and(|(t, _)| now - *t > LOCAL_KEEP_SECS)
    {
        replay.impacts.pop_front();
    }
}

/// Practice: once `fire_at` passes, cut `[kill − 2 s, kill + 1 s]` from the local
/// ring and start the replay.
fn start_local_killcam(
    time: Res<Time>,
    settings: Res<Settings>,
    replay: Res<LocalReplay>,
    mut pending: ResMut<PendingLocalCam>,
    mut active: ResMut<ActiveKillCam>,
) {
    let Some(p) = pending.0.as_ref() else { return };
    if active.0.is_some() || time.elapsed_secs() < p.fire_at {
        return;
    }
    let (lo, hi) = (p.kill_at - PRE_SECS, p.kill_at + POST_SECS);
    let frames: Vec<(f32, KillCamSample)> = replay
        .frames
        .iter()
        .filter(|(t, _)| *t >= lo && *t <= hi)
        .map(|(t, s)| (*t - lo, *s))
        .collect();
    let impacts: Vec<(f32, Vec3)> = replay
        .impacts
        .iter()
        .filter(|(t, _)| *t >= lo && *t <= hi)
        .map(|(t, p)| (*t - lo, *p))
        .collect();
    let bots = p.bots.clone();
    let kill_time = (p.kill_at - lo).max(0.0);
    pending.0 = None;
    if frames.len() < 4 {
        return;
    }
    let name = settings
        .username
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "You".to_string());
    active.0 = Some(KillCamRun {
        killer_name: name,
        frames,
        impacts,
        kill_time,
        bots,
        ghosts: Vec::new(),
        elapsed: 0.0,
        sound_cursor: 0,
        impact_cursor: 0,
        setup: false,
        saved: None,
        banner: None,
    });
}

/// Networked entry point: build a run from a [`shared::KillCam`] message.
pub(crate) fn begin_from_message(active: &mut ActiveKillCam, msg: shared::KillCam) {
    if active.0.is_some() || msg.samples.len() < 4 {
        return;
    }
    let hz = shared::TICK_HZ as f32;
    let frames: Vec<(f32, KillCamSample)> = msg
        .samples
        .into_iter()
        .enumerate()
        .map(|(i, s)| (i as f32 / hz, s))
        .collect();
    // Ground bursts ride along in the samples (networked path): one entry per
    // frame whose shot struck the ground, re-emitted as the playhead reaches it.
    let impacts: Vec<(f32, Vec3)> = frames
        .iter()
        .filter_map(|(t, s)| s.ground_pt.map(|p| (*t, Vec3::from_array(p))))
        .collect();
    let kill_time = frames
        .get(msg.kill_index as usize)
        .map(|(t, _)| *t)
        .unwrap_or(PRE_SECS);
    let bots = msg
        .bots
        .iter()
        .map(|b| (Vec3::from_array(b.pos), b.yaw, b.killed))
        .collect();
    active.0 = Some(KillCamRun {
        killer_name: msg.killer_name,
        frames,
        impacts,
        kill_time,
        bots,
        ghosts: Vec::new(),
        elapsed: 0.0,
        sound_cursor: 0,
        impact_cursor: 0,
        setup: false,
        saved: None,
        banner: None,
    });
}

// --- playback -------------------------------------------------------

/// Take the game camera over for the replay: stash the rig transforms, zero the
/// shake / recoil nodes, and put up the banner. We fly the *existing* rig along
/// the recorded eye path, so the world camera keeps the game's exact fog /
/// bloom / tonemapping and the first-person gun rides along in view.
#[allow(clippy::type_complexity)]
fn start_killcam(
    mut commands: Commands,
    assets: Res<KillCamAssets>,
    ads: Res<Ads>,
    mut active: ResMut<ActiveKillCam>,
    mut rig: Query<(&mut Transform, RigTags), RigFilter>,
    mut live_bots: Query<&mut Visibility, With<TargetBotVisual>>,
    stale_fx: Query<Entity, Or<(With<crate::Smoke>, With<crate::ImpactParticle>)>>,
) {
    let Some(run) = active.0.as_mut() else { return };
    if run.setup {
        return;
    }
    run.setup = true;

    let mut saved = SavedRig {
        player: Transform::IDENTITY,
        head: Transform::IDENTITY,
        ads_t: ads.t,
    };
    for (mut tf, (is_player, is_head, is_shake, is_recoil)) in &mut rig {
        if is_player {
            saved.player = *tf;
        } else if is_head {
            saved.head = *tf;
        } else if is_shake || is_recoil {
            *tf = Transform::IDENTITY;
        }
    }
    run.saved = Some(saved);
    info!(
        "kill cam: {} frames, {} bots, first cam {:?}",
        run.frames.len(),
        run.bots.len(),
        run.frames.first().map(|(_, s)| s.cam_pos)
    );

    // Start from a clean slate: clear any live-play smoke / ground debris still
    // drifting, so the only bursts on screen are the ones the replay re-emits.
    for e in &stale_fx {
        commands.entity(e).try_despawn();
    }

    // Hide the live bots and stand in frozen ghosts, so the replay shows the
    // arena exactly as it was at the kill (the one that was shot still upright,
    // then toppling on cue) rather than its current state.
    for mut vis in &mut live_bots {
        *vis = Visibility::Hidden;
    }
    for &(pos, yaw, killed) in &run.bots {
        let ghost = commands
            .spawn((
                KillCamGhost {
                    pos,
                    yaw,
                    killed,
                    fall: 0.0,
                },
                StateScoped(AppState::InGame),
                Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)),
                Visibility::default(),
            ))
            .with_child((
                Mesh3d(assets.mesh.clone()),
                MeshMaterial3d(assets.material.clone()),
                Transform::from_xyz(0.0, BOT_HEIGHT * 0.5, 0.0),
            ))
            .id();
        run.ghosts.push(ghost);
    }

    let banner = commands
        .spawn((
            KillCamBanner,
            StateScoped(AppState::InGame),
            GlobalZIndex(20),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(28.0),
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
        ))
        .with_children(|c| {
            c.spawn((
                Text::new("KILLCAM"),
                TextFont { font_size: 34.0, ..default() },
                TextColor(Color::srgb(1.0, 0.82, 0.1)),
            ));
            c.spawn((
                Text::new(format!("{}  \u{25B8}  BOT", run.killer_name)),
                TextFont { font_size: 20.0, ..default() },
                TextColor(Color::WHITE),
            ));
            c.spawn((
                Text::new("[F] SKIP"),
                TextFont { font_size: 15.0, ..default() },
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6)),
            ));
        })
        .id();
    run.banner = Some(banner);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive_killcam(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    sounds: Res<GameSounds>,
    mut fx: (ResMut<MuzzleFlashState>, ResMut<SmokeEmission>),
    mut commands: Commands,
    mut impacts: EventWriter<GroundImpact>,
    mut active: ResMut<ActiveKillCam>,
    mut weapon: ResMut<Weapon>,
    mut ads: ResMut<Ads>,
    mut rig: Query<(&mut Transform, RigTags), RigFilter>,
    mut ghosts: Query<(&mut KillCamGhost, &mut Transform)>,
    mut live_bots: Query<&mut Visibility, With<TargetBotVisual>>,
    mut anim_players: Query<&mut AnimationPlayer>,
    view_models: Query<&ViewModelAnimation>,
) {
    let Some(run) = active.0.as_mut() else { return };
    if !run.setup {
        return; // start_killcam hasn't run yet
    }
    let (ref mut muzzle, ref mut smoke) = fx;

    let duration = run.frames.last().map(|(t, _)| *t).unwrap_or(0.0);
    run.elapsed += time.delta_secs();
    let skipped = binds.killcam_skip.just_pressed(&keys, &mouse);

    let anim_node = view_models.iter().next().map(|vm| vm.index);

    if skipped || run.elapsed >= duration + 0.05 {
        // Put the rig back exactly, park the gun at rest, drop the banner and
        // ghosts, and hand the live bots back.
        if let Some(saved) = run.saved.take() {
            for (mut tf, (is_player, is_head, ..)) in &mut rig {
                if is_player {
                    *tf = saved.player;
                } else if is_head {
                    *tf = saved.head;
                }
            }
            // Hand aiming back exactly where playback found it; `update_ads`
            // (re-enabled the moment the cam clears) eases on from here.
            ads.t = saved.ads_t;
        }
        if let (Some(node), Some(mut ap)) = (anim_node, anim_players.iter_mut().next()) {
            if let Some(a) = ap.animation_mut(node) {
                a.seek_to(0.0);
                a.pause();
            }
        }
        weapon.busy = None;
        if let Some(e) = run.banner.take() {
            commands.entity(e).try_despawn();
        }
        for e in run.ghosts.drain(..) {
            commands.entity(e).try_despawn();
        }
        for mut vis in &mut live_bots {
            *vis = Visibility::Inherited;
        }
        active.0 = None;
        return;
    }

    // Fly the rig along the recorded camera path. The recorded transform is the
    // world camera's full global transform (shake / recoil / crouch already
    // baked in), so we drop it straight onto the rig root and hold every node
    // below it at identity.
    let t = run.elapsed.min(duration);
    let (a, b, frac) = bracket(&run.frames, t);
    let cam_pos = Vec3::from_array(a.cam_pos).lerp(Vec3::from_array(b.cam_pos), frac);
    let cam_rot = Quat::from_array(a.cam_rot).slerp(Quat::from_array(b.cam_rot), frac);

    // Replay the aim-down-sight amount frame-for-frame. `update_ads` is frozen
    // while the cam runs, so `apply_ads` / `update_scope` / `weapon_sway` (which
    // keep running) pick this up next frame and reproduce the exact scope-in /
    // scope-out the killer performed.
    ads.t = a.ads_t.lerp(b.ads_t, frac).clamp(0.0, 1.0);
    for (mut tf, (is_player, is_head, is_shake, is_recoil)) in &mut rig {
        if is_player {
            *tf = Transform {
                translation: cam_pos,
                rotation: cam_rot,
                scale: Vec3::ONE,
            };
        } else if is_head || is_shake || is_recoil {
            *tf = Transform::IDENTITY;
        }
    }

    // Topple the ghost that was shot once the playhead reaches the kill moment.
    let fall_step = time.delta_secs() / BOT_FALL_SECS;
    for (mut ghost, mut gtf) in &mut ghosts {
        if ghost.killed && run.elapsed >= run.kill_time && ghost.fall < 1.0 {
            ghost.fall = (ghost.fall + fall_step).min(1.0);
        }
        gtf.translation = ghost.pos;
        gtf.rotation = Quat::from_rotation_y(ghost.yaw)
            * Quat::from_rotation_x(ghost.fall.clamp(0.0, 1.0) * core::f32::consts::FRAC_PI_2);
    }

    // Pose the first-person weapon exactly where it was: seek its baked clip to
    // the recorded playhead (nearest sample, so we don't lerp across the jumps
    // between animation segments) and hold it paused there.
    let anim_t = if frac < 0.5 { a.anim_time } else { b.anim_time };
    if let (Some(node), Some(mut ap)) = (anim_node, anim_players.iter_mut().next()) {
        if ap.animation(node).is_none() {
            ap.play(node);
        }
        if let Some(active_anim) = ap.animation_mut(node) {
            active_anim.set_repeat(RepeatAnimation::Never);
            active_anim.set_speed(1.0);
            active_anim.seek_to(anim_t);
            active_anim.pause();
        }
    }

    // Fire any recorded sounds we've now reached.
    while run
        .frames
        .get(run.sound_cursor)
        .is_some_and(|(ft, _)| *ft <= t)
    {
        let bits = run.frames[run.sound_cursor].1.sound_bits;
        for bit in [
            SND_SHOT,
            SND_RELOAD,
            SND_RECHAMBER,
            SND_SLIDE,
            SND_DIVE,
            SND_AIM_IN,
            SND_AIM_OUT,
        ] {
            if bits & bit != 0 {
                if let Some(clip) = sound_for(&sounds, bit) {
                    commands.spawn((AudioPlayer::new(clip.clone()), PlaybackSettings::DESPAWN));
                }
            }
        }
        // The shot frame also drives the muzzle flash + barrel smoke, the same
        // pokes `weapon_system` does live (their update systems keep running
        // through the replay and position off the flown rig).
        if bits & SND_SHOT != 0 {
            muzzle.shots = muzzle.shots.wrapping_add(1);
            muzzle.roll = rand_roll(muzzle.shots);
            muzzle.intensity = 1.0;
            smoke.0 = Some(0.0);
        }
        run.sound_cursor += 1;
    }

    // Re-emit ground bursts (Practice) as the playhead reaches them.
    while run
        .impacts
        .get(run.impact_cursor)
        .is_some_and(|(it, _)| *it <= t)
    {
        impacts.write(GroundImpact(run.impacts[run.impact_cursor].1));
        run.impact_cursor += 1;
    }
}

/// Tear a running replay down on the way out of the game — restore the rig,
/// drop the banner + ghosts, and hand the live bots back.
#[allow(clippy::type_complexity)]
fn stop_killcam(
    mut commands: Commands,
    mut active: ResMut<ActiveKillCam>,
    mut pending: ResMut<PendingLocalCam>,
    mut weapon: ResMut<Weapon>,
    mut ads: ResMut<Ads>,
    mut rig: Query<(&mut Transform, RigTags), RigFilter>,
    mut anim_players: Query<&mut AnimationPlayer>,
    mut live_bots: Query<&mut Visibility, With<TargetBotVisual>>,
    view_models: Query<&ViewModelAnimation>,
    banner: Query<Entity, With<KillCamBanner>>,
    ghosts: Query<Entity, With<KillCamGhost>>,
) {
    pending.0 = None;
    for mut vis in &mut live_bots {
        *vis = Visibility::Inherited;
    }
    for e in &ghosts {
        commands.entity(e).try_despawn();
    }
    if let Some(run) = active.0.take() {
        if let Some(saved) = run.saved {
            for (mut tf, (is_player, is_head, ..)) in &mut rig {
                if is_player {
                    *tf = saved.player;
                } else if is_head {
                    *tf = saved.head;
                }
            }
            ads.t = saved.ads_t;
        }
        if let (Some(node), Some(mut ap)) = (
            view_models.iter().next().map(|vm| vm.index),
            anim_players.iter_mut().next(),
        ) {
            if let Some(a) = ap.animation_mut(node) {
                a.seek_to(0.0);
                a.pause();
            }
        }
        weapon.busy = None;
    }
    for e in &banner {
        commands.entity(e).try_despawn();
    }
}

// --- helpers -------------------------------------------------------

/// The two frames bracketing time `t`, plus the 0..1 fraction between them.
fn bracket(frames: &[(f32, KillCamSample)], t: f32) -> (KillCamSample, KillCamSample, f32) {
    if frames.len() < 2 {
        let s = frames[0].1;
        return (s, s, 0.0);
    }
    let i = frames.partition_point(|(ft, _)| *ft <= t).max(1).min(frames.len() - 1);
    let (t0, a) = frames[i - 1];
    let (t1, b) = frames[i];
    let frac = if t1 > t0 { ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 0.0 };
    (a, b, frac)
}
