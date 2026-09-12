//! Kill-cam replay. On a bot kill the server ships the killer's last ~4.5 s
//! ([`shared::KillCam`]); in solo Practice the same window is cut from a local
//! ring buffer. Playback flies the *existing* player rig along the recorded
//! path — body position/yaw, head pitch, camera shake, recoil kick and weapon
//! sway are each reconstructed from their own recorded state (not one baked
//! camera transform), so shake / recoil / sway reproduce exactly, and the FOV
//! is the shooter's own — seek-drives the first-person weapon animation,
//! re-fires the recorded one-shot sounds + muzzle flash + barrel smoke +
//! ground bursts, and stands in ghost copies of the bots frozen at the kill so
//! the one that was hit topples on cue. A letterbox bar top and bottom names
//! "KILLCAM" / the killer; `F` (rebindable) skips.

use std::collections::VecDeque;
use std::f32::consts::PI;

use bevy::animation::RepeatAnimation;
use bevy::audio::Volume;
use bevy::prelude::*;

use shared::KillCamSample;

use crate::keybinds::KeyBindings;
use crate::settings::Settings;
use crate::{
    rand_roll, Ads, AimSwayState, AppState, BloodImpact, BotAnimationPlayer, BotAnimations,
    BotVisual, CameraRecoil, CameraShake, FireTracer, GameSounds, GroundImpact, MuzzleFlashState,
    Player, PlayerHead, ScopeCamera, SmokeEmission, SniperAnimationPlayer, TargetBotVisual,
    Tracer, ViewModel, ViewModelAnimation, Weapon, WeaponSlot, WorldModelCamera,
};

/// Read the first-person weapon animation's current playhead (seconds), for
/// recording. `0.0` if the player / clip isn't ready yet.
pub(crate) fn viewmodel_anim_time(
    players: &Query<&AnimationPlayer, With<SniperAnimationPlayer>>,
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
pub(crate) const SND_SHOT: u16 = 1 << 0;
pub(crate) const SND_RELOAD: u16 = 1 << 1;
pub(crate) const SND_RECHAMBER: u16 = 1 << 2;
pub(crate) const SND_SLIDE: u16 = 1 << 3;
pub(crate) const SND_DIVE: u16 = 1 << 4;
pub(crate) const SND_AIM_IN: u16 = 1 << 5;
pub(crate) const SND_AIM_OUT: u16 = 1 << 6;
pub(crate) const SND_FOOTSTEP: u16 = 1 << 7;
/// The jump-landing thump (`GameSounds::jump_land`) — there's no separate
/// jump-launch sound, only landing.
pub(crate) const SND_JUMP_LAND: u16 = 1 << 8;
/// All bits currently in use, for iterating a `sound_bits` mask.
pub(crate) const ALL_SND_BITS: [u16; 9] = [
    SND_SHOT,
    SND_RELOAD,
    SND_RECHAMBER,
    SND_SLIDE,
    SND_DIVE,
    SND_AIM_IN,
    SND_AIM_OUT,
    SND_FOOTSTEP,
    SND_JUMP_LAND,
];

/// Seconds of replay before / after the kill.
const PRE_SECS: f32 = 3.0;
pub(crate) const POST_SECS: f32 = 1.5;
/// A touch more than `PRE + POST`, so the local ring always covers the window.
const LOCAL_KEEP_SECS: f32 = 5.0;

/// Sounds the player triggered since the last input packet / replay frame.
/// `write_input` drains it in a networked game; `record_local_replay` drains it
/// in Practice.
#[derive(Resource, Default)]
pub(crate) struct ReplaySoundBits(pub(crate) u16);

impl ReplaySoundBits {
    pub(crate) fn note(&mut self, bit: u16) {
        self.0 |= bit;
    }
}

/// The local player's most recent ground-impact point that hasn't yet been
/// stamped onto a [`PlayerInput`]. Set by `resolve_local_shot`, drained by
/// `write_input` so the networked kill cam can replay ground bursts. (Practice
/// records its own bursts straight off the [`GroundImpact`] event stream.)
#[derive(Resource, Default)]
pub(crate) struct ReplayGroundImpact(pub(crate) Option<Vec3>);

/// As [`ReplayGroundImpact`], but for the most recent bot-hit blood squirt.
/// Set by `resolve_local_shot` / `net::follow_bot_avatars`, drained by
/// `write_input`. (Practice records straight off the `BloodImpact` stream.)
#[derive(Resource, Default)]
pub(crate) struct ReplayBloodImpact(pub(crate) Option<Vec3>);

/// As [`ReplayGroundImpact`], but the most recent shot tracer as
/// `(start, end)`. Set by `resolve_local_shot`, drained by `write_input`.
/// (Practice records straight off the `FireTracer` stream.)
#[derive(Resource, Default)]
pub(crate) struct ReplayTracer(pub(crate) Option<(Vec3, Vec3)>);

/// Rolling local recording for Practice (and the follow-through second even in a
/// networked game isn't needed here — the server owns that path).
#[derive(Resource, Default)]
struct LocalReplay {
    frames: VecDeque<(f32, KillCamSample)>,
    /// `(time, world point)` of ground-impact bursts, so Practice replays the
    /// rock / dust too.
    impacts: VecDeque<(f32, Vec3)>,
    /// `(time, hit point, squirt dir)` of bot-hit blood squirts.
    bloods: VecDeque<(f32, Vec3, Vec3)>,
    /// `(time, start, end)` of shot tracers, so the replay re-draws each along
    /// its true path at the right moment instead of leaving the live one hung
    /// in the world.
    tracers: VecDeque<(f32, Vec3, Vec3)>,
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
    /// `(seconds-from-start, world point)` ground bursts to re-emit.
    impacts: Vec<(f32, Vec3)>,
    /// `(seconds-from-start, hit point, squirt dir)` blood squirts to re-emit.
    bloods: Vec<(f32, Vec3, Vec3)>,
    /// `(seconds-from-start, start, end)` shot tracers to re-draw.
    tracers: Vec<(f32, Vec3, Vec3)>,
    /// Seconds-from-start the shot landed — when the hit ghost starts to topple.
    kill_time: f32,
    /// Bots frozen at the kill: `(pos, yaw, was_the_one_shot)`.
    bots: Vec<(Vec3, f32, bool)>,
    /// Ghost bot entities spawned for the replay (despawned on teardown).
    ghosts: Vec<Entity>,
    elapsed: f32,
    /// Index of the next frame whose sounds still need firing.
    sound_cursor: usize,
    /// Clip index the last replayed footstep used, so [`pick_footstep`] can
    /// avoid an instant repeat the same way live `play_footstep` does.
    footstep_last: usize,
    /// Index of the next ground burst to re-emit.
    impact_cursor: usize,
    /// Index of the next blood squirt to re-emit.
    blood_cursor: usize,
    /// Index of the next tracer to re-draw.
    tracer_cursor: usize,
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
    /// Whether the view model was shown, whether the throwing knife was held,
    /// and which slot was equipped, at the instant playback took over — all
    /// driven by the recorded samples for the replay's duration, then
    /// restored on teardown so live weapon state resumes exactly where it
    /// left off.
    weapon_visible: bool,
    knife_active: bool,
    slot: WeaponSlot,
}

#[derive(Component)]
struct KillCamBanner;

/// A stand-in bot shown during the replay, frozen where it stood at the kill.
#[derive(Component)]
struct KillCamGhost {
    pos: Vec3,
    yaw: f32,
    /// This is the bot that was shot — plays the death animation once
    /// `elapsed >= kill_time`.
    killed: bool,
    /// Set once the death animation has been kicked off, so it isn't
    /// restarted every frame the ghost lingers.
    die_played: bool,
}

/// Assets built at startup for the kill-cam banner (the ghost bots themselves
/// reuse the same `models/bot.glb` scene + [`BotAnimations`] as live bots).
#[derive(Resource)]
struct KillCamAssets {
    /// Bold condensed display face for the banner text.
    banner_font: Handle<Font>,
}

/// Filter for the four rig entities `start_killcam` / `drive_killcam` steer.
/// `Without<KillCamGhost>` / `Without<ViewModel>` keep the `&mut Transform`
/// access disjoint from the ghost and view-model queries in `drive_killcam`.
type RigFilter = (
    Or<(
        With<Player>,
        With<PlayerHead>,
        With<CameraShake>,
        With<CameraRecoil>,
    )>,
    Without<KillCamGhost>,
    Without<ViewModel>,
);

/// Which of the four rig markers an entity carries.
type RigTags = (
    Has<Player>,
    Has<PlayerHead>,
    Has<CameraShake>,
    Has<CameraRecoil>,
);

/// Disjointness proof for `world_cam` / `scope_cam` in [`start_killcam`]
/// against `rig`'s `RigFilter` — both touch `&mut Transform`, and Bevy can
/// only skip the runtime overlap check if every query rules out every marker
/// the others could match.
type AimCamFilter = (
    Without<Player>,
    Without<PlayerHead>,
    Without<CameraShake>,
    Without<CameraRecoil>,
);

pub struct KillCamPlugin;

impl Plugin for KillCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReplaySoundBits>()
            .init_resource::<ReplayGroundImpact>()
            .init_resource::<ReplayBloodImpact>()
            .init_resource::<ReplayTracer>()
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
                    // `apply_ads` still runs during a replay (it eases FOV /
                    // view-model pose off the replayed `ads.t`); `drive_killcam`
                    // overrides the FOV and multiplies the recorded shake /
                    // sway / recoil onto whatever `apply_ads` just wrote, the
                    // same order the live systems apply them, so it must run
                    // after it.
                    .after(crate::apply_ads)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Run condition: gameplay input is live (no replay playing).
pub(crate) fn no_killcam(active: Res<ActiveKillCam>) -> bool {
    active.0.is_none()
}

fn setup_killcam_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(KillCamAssets {
        banner_font: asset_server.load(crate::HUD_FONT),
    });
}

/// Maps a single `SND_*` bit to its clip — every bit except [`SND_FOOTSTEP`]
/// (footsteps pick randomly, see [`pick_footstep`]). Shared by kill-cam replay
/// and `net::receive_remote_sounds`.
pub(crate) fn sound_for(sounds: &GameSounds, bit: u16) -> Option<&Handle<AudioSource>> {
    Some(match bit {
        SND_SHOT => &sounds.shot,
        SND_RELOAD => &sounds.reload,
        SND_RECHAMBER => &sounds.rechamber,
        SND_SLIDE => &sounds.slide,
        SND_DIVE => &sounds.dive,
        SND_AIM_IN => &sounds.aim_in,
        SND_AIM_OUT => &sounds.aim_out,
        SND_JUMP_LAND => &sounds.jump_land,
        _ => return None,
    })
}

/// Pick a random footstep clip: a fresh pick that isn't an instant repeat of
/// `last`, mirroring live `play_footstep`'s anti-repeat rule (just seeded off
/// `seed` instead of a running RNG sequence). Shared by kill-cam replay and
/// `net::receive_remote_sounds`.
pub(crate) fn pick_footstep(
    clips: &[Handle<AudioSource>],
    last: &mut usize,
    seed: u32,
) -> Option<Handle<AudioSource>> {
    if clips.is_empty() {
        return None;
    }
    let t = (rand_roll(seed) + PI) / (2.0 * PI);
    let mut idx = ((t * clips.len() as f32) as usize).min(clips.len() - 1);
    if clips.len() > 1 && idx == *last {
        idx = (idx + 1) % clips.len();
    }
    *last = idx;
    Some(clips[idx].clone())
}

// --- recording (Practice) --------------------------------------------

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn record_local_replay(
    time: Res<Time>,
    ads: Res<Ads>,
    shake: Res<crate::Shake>,
    sway: Res<crate::WeaponSwayState>,
    settings: Res<Settings>,
    mut bits: ResMut<ReplaySoundBits>,
    mut replay: ResMut<LocalReplay>,
    mut impacts: EventReader<GroundImpact>,
    mut bloods: EventReader<BloodImpact>,
    mut tracers: EventReader<FireTracer>,
    player: Query<&Transform, With<Player>>,
    head: Query<&Transform, With<PlayerHead>>,
    anim_players: Query<&AnimationPlayer, With<SniperAnimationPlayer>>,
    view_models: Query<&ViewModelAnimation>,
    (view_model_vis, knife, weapon): (
        Query<&Visibility, With<ViewModel>>,
        Res<crate::ThrowingKnife>,
        Res<Weapon>,
    ),
) {
    let (Ok(pt), Ok(ht)) = (player.single(), head.single()) else {
        return;
    };
    let now = time.elapsed_secs();
    replay.frames.push_back((
        now,
        KillCamSample {
            translation: pt.translation.to_array(),
            yaw: pt.rotation.to_euler(EulerRot::YXZ).0,
            pitch: ht.rotation.to_euler(EulerRot::YXZ).1,
            shake_trauma: shake.trauma,
            shake_phase: shake.phase,
            shake_recoil: shake.recoil,
            sway_offset: sway.offset.to_array(),
            fov_deg: settings.fov,
            sound_bits: std::mem::take(&mut bits.0),
            anim_time: viewmodel_anim_time(&anim_players, &view_models),
            ads_t: ads.t,
            // Practice replays its ground bursts / blood / tracers from the
            // `LocalReplay` lists (filled just below), so these per-frame slots
            // — used only by the networked path — stay empty here.
            ground_pt: None,
            blood_pt: None,
            tracer: None,
            weapon_visible: view_model_vis
                .iter()
                .next()
                .is_none_or(|v| *v != Visibility::Hidden),
            knife_active: knife.active,
            sniper_active: weapon.slot == WeaponSlot::Primary,
        },
    ));
    for ev in impacts.read() {
        replay.impacts.push_back((now, ev.0));
    }
    for ev in bloods.read() {
        replay.bloods.push_back((now, ev.point, ev.dir));
    }
    for ev in tracers.read() {
        replay.tracers.push_back((now, ev.start, ev.end));
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
    while replay
        .bloods
        .front()
        .is_some_and(|(t, ..)| now - *t > LOCAL_KEEP_SECS)
    {
        replay.bloods.pop_front();
    }
    while replay
        .tracers
        .front()
        .is_some_and(|(t, ..)| now - *t > LOCAL_KEEP_SECS)
    {
        replay.tracers.pop_front();
    }
}

/// Practice: once `fire_at` passes, cut `[kill − 3 s, kill + 1.5 s]` from the
/// local ring and start the replay.
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
    let bloods: Vec<(f32, Vec3, Vec3)> = replay
        .bloods
        .iter()
        .filter(|(t, ..)| *t >= lo && *t <= hi)
        .map(|(t, p, d)| (*t - lo, *p, *d))
        .collect();
    let tracers: Vec<(f32, Vec3, Vec3)> = replay
        .tracers
        .iter()
        .filter(|(t, ..)| *t >= lo && *t <= hi)
        .map(|(t, s, e)| (*t - lo, *s, *e))
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
        bloods,
        tracers,
        kill_time,
        bots,
        ghosts: Vec::new(),
        elapsed: 0.0,
        sound_cursor: 0,
        footstep_last: 0,
        impact_cursor: 0,
        blood_cursor: 0,
        tracer_cursor: 0,
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
    // Ground bursts, blood squirts and tracers ride along in the samples
    // (networked path): one entry per frame that carried one, re-emitted as the
    // playhead reaches it. Networked bot kills carry no shot direction, so the
    // squirt replays straight up — the same fallback `follow_bot_avatars` uses.
    let impacts: Vec<(f32, Vec3)> = frames
        .iter()
        .filter_map(|(t, s)| s.ground_pt.map(|p| (*t, Vec3::from_array(p))))
        .collect();
    let bloods: Vec<(f32, Vec3, Vec3)> = frames
        .iter()
        .filter_map(|(t, s)| s.blood_pt.map(|p| (*t, Vec3::from_array(p), Vec3::Y)))
        .collect();
    let tracers: Vec<(f32, Vec3, Vec3)> = frames
        .iter()
        .filter_map(|(t, s)| {
            s.tracer
                .map(|[a, b]| (*t, Vec3::from_array(a), Vec3::from_array(b)))
        })
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
        bloods,
        tracers,
        kill_time,
        bots,
        ghosts: Vec::new(),
        elapsed: 0.0,
        sound_cursor: 0,
        footstep_last: 0,
        impact_cursor: 0,
        blood_cursor: 0,
        tracer_cursor: 0,
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
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn start_killcam(
    mut commands: Commands,
    assets: Res<KillCamAssets>,
    ads: Res<Ads>,
    mut active: ResMut<ActiveKillCam>,
    mut rig: Query<(&mut Transform, RigTags), RigFilter>,
    mut live_bots: Query<&mut Visibility, With<TargetBotVisual>>,
    stale_fx: Query<Entity, Or<(With<crate::Smoke>, With<crate::ImpactParticle>, With<Tracer>)>>,
    view_model_vis: Query<&Visibility, (With<ViewModel>, Without<TargetBotVisual>)>,
    knife: Res<crate::ThrowingKnife>,
    weapon: Res<Weapon>,
    asset_server: Res<AssetServer>,
    // `aim_idle_sway` (real camera rotation, gated off during a replay) is the
    // only thing that ever touches these — reset them here so a viewer's own
    // last live breathing doesn't leak into the replay for its whole duration.
    mut world_cam: Query<
        &mut Transform,
        (With<WorldModelCamera>, Without<ScopeCamera>, AimCamFilter),
    >,
    mut scope_cam: Query<
        &mut Transform,
        (With<ScopeCamera>, Without<WorldModelCamera>, AimCamFilter),
    >,
    mut aim_sway: ResMut<AimSwayState>,
) {
    let Some(run) = active.0.as_mut() else { return };
    if run.setup {
        return;
    }
    run.setup = true;

    if let Ok(mut tf) = world_cam.single_mut() {
        tf.rotation = Quat::IDENTITY;
    }
    if let Ok(mut tf) = scope_cam.single_mut() {
        tf.rotation = Quat::IDENTITY;
    }
    aim_sway.offset = Vec2::ZERO;

    let mut saved = SavedRig {
        player: Transform::IDENTITY,
        head: Transform::IDENTITY,
        ads_t: ads.t,
        weapon_visible: view_model_vis
            .iter()
            .next()
            .is_none_or(|v| *v != Visibility::Hidden),
        knife_active: knife.active,
        slot: weapon.slot,
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
        "kill cam: {} frames, {} bots, first pos {:?}",
        run.frames.len(),
        run.bots.len(),
        run.frames.first().map(|(_, s)| s.translation)
    );

    // Start from a clean slate: clear any live-play smoke, ground / blood debris
    // and shot tracers still in the world, so the only effects on screen are the
    // ones the replay re-emits at the recorded times.
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
                    die_played: false,
                },
                BotVisual,
                StateScoped(AppState::InGame),
                Transform::from_translation(pos)
                    .with_rotation(Quat::from_rotation_y(yaw))
                    .with_scale(Vec3::splat(crate::BOT_MODEL_SCALE)),
                Visibility::default(),
                SceneRoot(
                    asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/bot.glb")),
                ),
            ))
            .observe(crate::start_bot_animation)
            .id();
        run.ghosts.push(ghost);
    }

    // Cinematic letterbox: a translucent black bar top and bottom, each ~15%
    // of the screen, "KILLCAM" centred in the top one and the killer's name
    // centred in the bottom one.
    let bar_node = |top: bool| Node {
        position_type: PositionType::Absolute,
        top: if top { Val::Percent(0.0) } else { Val::Auto },
        bottom: if top { Val::Auto } else { Val::Percent(0.0) },
        left: Val::Percent(0.0),
        right: Val::Percent(0.0),
        height: Val::Percent(15.0),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..default()
    };
    let banner = commands
        .spawn((
            KillCamBanner,
            StateScoped(AppState::InGame),
            GlobalZIndex(20),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(0.0),
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                bottom: Val::Percent(0.0),
                ..default()
            },
        ))
        .with_children(|c| {
            c.spawn((bar_node(true), BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7))))
                .with_children(|bar| {
                    bar.spawn((
                        Text::new("KILLCAM"),
                        TextFont {
                            font: assets.banner_font.clone(),
                            font_size: 46.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.06, 0.06)),
                    ));
                });
            c.spawn((bar_node(false), BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7))))
                .with_children(|bar| {
                    bar.spawn((
                        Text::new(run.killer_name.clone()),
                        TextFont {
                            font: assets.banner_font.clone(),
                            font_size: 32.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));
                });
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
    // Bundled into tuples — a system function tops out at 16 top-level params.
    cfg: (
        Res<crate::ShakeSettings>,
        Res<crate::AdsTuning>,
        Res<crate::FootstepSettings>,
    ),
    mut fx: (ResMut<MuzzleFlashState>, ResMut<SmokeEmission>),
    mut commands: Commands,
    // Re-emitted as the playhead reaches each recorded time.
    mut fx_events: (
        EventWriter<GroundImpact>,
        EventWriter<BloodImpact>,
        EventWriter<FireTracer>,
    ),
    mut active: ResMut<ActiveKillCam>,
    (mut weapon, mut knife): (ResMut<Weapon>, ResMut<crate::ThrowingKnife>),
    mut ads: ResMut<Ads>,
    mut rig: Query<(&mut Transform, RigTags), RigFilter>,
    cams: (
        Single<&mut Projection, With<WorldModelCamera>>,
        Single<(&mut Transform, &mut Visibility), (With<ViewModel>, Without<TargetBotVisual>)>,
    ),
    bots: (
        // Explicit `With<KillCamGhost>` (redundant with the `&mut KillCamGhost`
        // fetch) plus `Without<ViewModel>` so Bevy can prove this is disjoint
        // from `rig` and `cams`'s view-model `Transform` access at the type
        // level, not just by knowing at runtime that the two never overlap.
        Query<(Entity, &mut KillCamGhost, &mut Transform), (With<KillCamGhost>, Without<ViewModel>)>,
        Query<&mut Visibility, With<TargetBotVisual>>,
        Query<&BotAnimationPlayer>,
        // `Without<SniperAnimationPlayer>`, disjoint from `anim.0` below the
        // same way the ghost query above is disjoint from `cams`.
        Query<&mut AnimationPlayer, Without<SniperAnimationPlayer>>,
        Res<BotAnimations>,
    ),
    anim: (
        Query<&mut AnimationPlayer, With<SniperAnimationPlayer>>,
        Query<&ViewModelAnimation>,
    ),
) {
    let Some(run) = active.0.as_mut() else { return };
    if !run.setup {
        return; // start_killcam hasn't run yet
    }
    let (shake_cfg, tuning, footstep_cfg) = cfg;
    let (ref mut muzzle, ref mut smoke) = fx;
    let (ref mut impacts, ref mut bloods, ref mut tracers) = fx_events;
    let (mut world_projection, view_model_single) = cams;
    let (mut view_model, mut view_model_vis) = view_model_single.into_inner();
    let (mut ghosts, mut live_bots, bot_roots, mut bot_players, bot_anims) = bots;
    let (mut anim_players, view_models) = anim;

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
            // Same for the weapon model / throwing-knife crosshair: whatever
            // was true live when the cam took over, not whatever the last
            // replayed frame happened to show.
            *view_model_vis = if saved.weapon_visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            knife.active = saved.knife_active;
            weapon.slot = saved.slot;
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

    // Fly the rig along the recorded path: body position/yaw, head pitch, and
    // the shake / recoil nodes each get their own recorded local pose (run
    // through the same pure formulas the live game uses), rather than
    // collapsing everything onto one rigid world transform — that's what lets
    // this reproduce camera shake, weapon recoil and weapon sway exactly
    // instead of just flying a pre-baked camera path.
    let t = run.elapsed.min(duration);
    let (a, b, frac) = bracket(&run.frames, t);
    let translation = Vec3::from_array(a.translation).lerp(Vec3::from_array(b.translation), frac);
    let yaw_rot = Quat::from_rotation_y(a.yaw).slerp(Quat::from_rotation_y(b.yaw), frac);
    let pitch_rot = Quat::from_rotation_x(a.pitch).slerp(Quat::from_rotation_x(b.pitch), frac);
    let trauma = a.shake_trauma.lerp(b.shake_trauma, frac);
    let phase = a.shake_phase.lerp(b.shake_phase, frac);
    let recoil = a.shake_recoil.lerp(b.shake_recoil, frac);
    let sway = Vec2::from_array(a.sway_offset).lerp(Vec2::from_array(b.sway_offset), frac);
    let fov_deg = a.fov_deg.lerp(b.fov_deg, frac);

    // Replay the aim-down-sight amount frame-for-frame. `update_ads` is frozen
    // while the cam runs, so `apply_ads` / `update_scope` (which keep running)
    // pick this up next frame and reproduce the exact scope-in / scope-out the
    // killer performed.
    ads.t = a.ads_t.lerp(b.ads_t, frac).clamp(0.0, 1.0);

    let shake_pose = crate::shake_camera_pose(&shake_cfg, ads.t, trauma, phase);
    for (mut tf, (is_player, is_head, is_shake, is_recoil)) in &mut rig {
        if is_player {
            *tf = Transform {
                translation,
                rotation: yaw_rot,
                scale: Vec3::ONE,
            };
        } else if is_head {
            *tf = Transform::from_rotation(pitch_rot);
        } else if is_shake {
            *tf = shake_pose;
        } else if is_recoil {
            *tf = Transform::from_xyz(0.0, 0.0, recoil);
        }
    }

    // FOV: override whatever `apply_ads` just wrote (the *viewer's* hip FOV)
    // with the FOV the shooter actually had, blended by the same replayed
    // `ads.t`.
    if let Projection::Perspective(perspective) = world_projection.as_mut() {
        perspective.fov = crate::ads_fov_rad(fov_deg, &tuning, ads.t);
    }

    // Weapon sway + recoil shudder: `apply_ads` already wrote the base hip/ads
    // pose onto the view model this frame (live, off the replayed `ads.t`);
    // multiply the recorded sway and kick on top, in the same order the live
    // `weapon_sway` / `weapon_recoil_shudder` systems apply them (both are
    // gated off while a kill cam is active, so there's no double-application).
    let sway_rot = Quat::from_euler(EulerRot::YXZ, sway.x, sway.y, 0.0);
    let kick = crate::weapon_kick_pose(&shake_cfg, ads.t, trauma, phase);
    *view_model = kick * Transform::from_rotation(sway_rot) * *view_model;

    // Play the death animation on the ghost that was shot once the playhead
    // reaches the kill moment — once, same guard `tick_practice_bots` /
    // `follow_bot_avatars` use for the live paths.
    for (entity, mut ghost, mut gtf) in &mut ghosts {
        if ghost.killed && !ghost.die_played && run.elapsed >= run.kill_time {
            ghost.die_played = true;
            if let Ok(target) = bot_roots.get(entity) {
                if let Ok(mut player) = bot_players.get_mut(target.0) {
                    let active = player.play(bot_anims.die);
                    active.set_repeat(RepeatAnimation::Never);
                    active.set_speed(crate::BOT_DIE_SPEED);
                    active.replay();
                }
            }
        }
        gtf.translation = ghost.pos;
        gtf.rotation = Quat::from_rotation_y(ghost.yaw);
    }

    // Weapon visibility / throwing-knife crosshair: booleans, so nearest
    // sample rather than a lerp — same rule the animation playhead uses below.
    let weapon_visible = if frac < 0.5 { a.weapon_visible } else { b.weapon_visible };
    *view_model_vis = if weapon_visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    knife.active = if frac < 0.5 { a.knife_active } else { b.knife_active };
    let sniper_active = if frac < 0.5 { a.sniper_active } else { b.sniper_active };
    weapon.slot = if sniper_active {
        WeaponSlot::Primary
    } else {
        WeaponSlot::Secondary
    };

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
            SND_JUMP_LAND,
        ] {
            if bits & bit != 0 {
                if let Some(clip) = sound_for(&sounds, bit) {
                    commands.spawn((AudioPlayer::new(clip.clone()), PlaybackSettings::DESPAWN));
                }
            }
        }
        // Footsteps aren't one fixed clip (live play picks randomly from
        // `sounds.footsteps`), so they don't go through `sound_for` — replay
        // the same random-pick-without-repeat + pitch-jitter live play does.
        if bits & SND_FOOTSTEP != 0 {
            let seed = run.sound_cursor as u32;
            if let Some(clip) = pick_footstep(&sounds.footsteps, &mut run.footstep_last, seed) {
                let pitch = 1.0 + (rand_roll(seed ^ 0x5bd1_e995) / PI) * footstep_cfg.pitch_jitter;
                commands.spawn((
                    AudioPlayer::new(clip),
                    PlaybackSettings::DESPAWN
                        .with_volume(Volume::Linear(
                            (footstep_cfg.volume * footstep_cfg.walk_volume).max(0.0),
                        ))
                        .with_speed(pitch.clamp(0.1, 4.0)),
                ));
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

    // Re-emit the recorded ground bursts, blood squirts and shot tracers as the
    // playhead reaches each one's time.
    while run
        .impacts
        .get(run.impact_cursor)
        .is_some_and(|(it, _)| *it <= t)
    {
        impacts.write(GroundImpact(run.impacts[run.impact_cursor].1));
        run.impact_cursor += 1;
    }
    while run
        .bloods
        .get(run.blood_cursor)
        .is_some_and(|(bt, ..)| *bt <= t)
    {
        let (_, point, dir) = run.bloods[run.blood_cursor];
        bloods.write(BloodImpact { point, dir });
        run.blood_cursor += 1;
    }
    while run
        .tracers
        .get(run.tracer_cursor)
        .is_some_and(|(tt, ..)| *tt <= t)
    {
        let (_, start, end) = run.tracers[run.tracer_cursor];
        tracers.write(FireTracer { start, end });
        run.tracer_cursor += 1;
    }
}

/// Tear a running replay down on the way out of the game — restore the rig,
/// drop the banner + ghosts, and hand the live bots back.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn stop_killcam(
    mut commands: Commands,
    mut active: ResMut<ActiveKillCam>,
    mut pending: ResMut<PendingLocalCam>,
    mut weapon: ResMut<Weapon>,
    mut ads: ResMut<Ads>,
    mut rig: Query<(&mut Transform, RigTags), RigFilter>,
    mut anim_players: Query<&mut AnimationPlayer, With<SniperAnimationPlayer>>,
    mut live_bots: Query<&mut Visibility, With<TargetBotVisual>>,
    view_models: Query<&ViewModelAnimation>,
    banner: Query<Entity, With<KillCamBanner>>,
    ghosts: Query<Entity, With<KillCamGhost>>,
    mut view_model_vis: Query<&mut Visibility, (With<ViewModel>, Without<TargetBotVisual>)>,
    mut knife: ResMut<crate::ThrowingKnife>,
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
            if let Ok(mut vis) = view_model_vis.single_mut() {
                *vis = if saved.weapon_visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
            knife.active = saved.knife_active;
            weapon.slot = saved.slot;
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
