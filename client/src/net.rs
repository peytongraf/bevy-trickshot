//! Client-side networking: connect to the authoritative server and expose the
//! connection state to the rest of the client.
//!
//! The address the client dials is the one seam a future matchmaker would touch
//! ([`server_addr`]); everything downstream is identical no matter where that
//! address came from.
//!
//! Responsibilities: connect + hold the [`ReplicationReceiver`]; once
//! `AppState::InGame`, ship the local player's pose in the input packet
//! (`write_input`) and render every other player as a `models/soldier.glb`
//! avatar (`spawn_remote_avatars` / `follow_remote_avatars`).

use core::net::{Ipv4Addr, Ipv6Addr};
use core::time::Duration;
use std::net::{SocketAddr, ToSocketAddrs};

use bevy::animation::{RepeatAnimation, prelude::AnimationTransitions};
use bevy::audio::{SpatialScale, Volume};
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use lightyear::prelude::client::{ClientPlugins, NetcodeClient, NetcodeConfig};
use lightyear::prelude::input::client::InputSet;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::*;

use shared::{
    Bot, KillCam, MatchOver, PlayerId, PlayerInput, PlayerPose, PlayerRespawn, ShotOutcome,
    ShotResolved, TrickScore,
};

use crate::fall_death;
use crate::killcam::{self, ActiveKillCam, ReplaySoundBits};
use crate::{
    Ads, AppState, BloodImpact,
    GroundImpact, MatchEndedEvent, PendingShot, Player, PlayerHead, PlayerPhysics, SniperGlint,
    SniperGlintAssets, SniperGlintBone, SniperGlintSettings, TrickScoredEvent, TrickState,
    WorldModelCamera, NOSCOPE_ADS_MAX,
};

/// Where a shipped build connects when `TRICKSHOT_SERVER` is unset and we're not
/// running under `cargo`. Mirrors `updater.rs`'s `DEFAULT_REPO` convention.
const DEFAULT_SERVER: &str = "bevy-trickshot-server.fly.dev:5000";

/// Resolve the server address and the netcode private key to authenticate
/// with:
/// * `TRICKSHOT_SERVER` env var wins (host:port, resolved via DNS) — the prod
///   key is used only if it's set to exactly [`DEFAULT_SERVER`], so pointing
///   it at a friend's self-hosted server (or `localhost`) still uses the dev
///   key that server almost certainly falls back to,
/// * else `TRICKSHOT_PROD` (any value) is a one-off escape hatch to point a
///   dev build at the real production server without typing out
///   [`DEFAULT_SERVER`] by hand — `TRICKSHOT_PROD=1 cargo run`,
/// * else `127.0.0.1:5000` under `cargo run` (dev), with the dev key — the
///   default, since that's what you want almost every time,
/// * else [`DEFAULT_SERVER`] (shipped build), with the prod key.
pub fn server_addr() -> (SocketAddr, [u8; 32]) {
    let fallback = || SocketAddr::from(([127, 0, 0, 1], shared::DEFAULT_PORT));
    // Prefer an IPv4 result over IPv6: fly.io's UDP routing needs a *dedicated*
    // IP, and plenty of players' networks (home ISPs, VPNs, campus/office
    // networks) have no outbound IPv6 route at all — sending to an IPv6
    // address there fails at the OS level (`ENETUNREACH`) before a single
    // packet leaves the machine, so the client just sits on "connecting"
    // forever and the server never logs an attempt. IPv4 connectivity is
    // close to universal, so it's the safer default when DNS offers both.
    let resolve = |s: &str| {
        let mut addrs: Vec<SocketAddr> = s.to_socket_addrs().ok()?.collect();
        addrs.sort_by_key(SocketAddr::is_ipv6);
        addrs.into_iter().next()
    };

    if let Ok(s) = std::env::var("TRICKSHOT_SERVER") {
        if let Some(addr) = resolve(&s) {
            let key = if s.trim() == DEFAULT_SERVER {
                shared::PROD_PRIVATE_KEY
            } else {
                shared::DEV_PRIVATE_KEY
            };
            return (addr, key);
        }
        warn!("TRICKSHOT_SERVER='{s}' did not resolve; using the default instead");
    }

    if std::env::var_os("TRICKSHOT_PROD").is_some() {
        let addr = resolve(DEFAULT_SERVER).unwrap_or_else(fallback);
        return (addr, shared::PROD_PRIVATE_KEY);
    }

    if std::env::var_os("CARGO").is_some() {
        let addr = resolve(&format!("127.0.0.1:{}", shared::DEFAULT_PORT)).unwrap_or_else(fallback);
        (addr, shared::DEV_PRIVATE_KEY)
    } else {
        let addr = resolve(DEFAULT_SERVER).unwrap_or_else(fallback);
        (addr, shared::PROD_PRIVATE_KEY)
    }
}

/// A `u64` that is distinct between two clients launched from the same machine
/// (netcode rejects a second connection with a `client_id` already in use).
fn dev_client_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    ((std::process::id() as u64) << 32) | nanos
}

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        // Order matters: client plugins, then the shared protocol, then (in a
        // `Startup` system below) the `Client` entity.
        app.add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / shared::TICK_HZ),
        });
        app.add_plugins(shared::SharedPlugin);

        app.init_resource::<PendingMatchEnd>();
        app.init_resource::<PendingRespawn>();
        app.add_event::<LocalPlayerRespawned>();
        app.add_systems(Startup, connect);
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);

        app.add_systems(
            FixedPreUpdate,
            write_input
                .in_set(InputSet::WriteClientInputs)
                // A kill-cam replay hijacks this client's own `Player`/
                // `PlayerHead` rig to fly the killer's recorded path (see
                // `killcam::drive_killcam`) — without this, `write_input`
                // would read that hijacked transform right off the rig and
                // broadcast it as this player's live position, so every
                // other client (the killer included) sees this player's
                // avatar warp onto and replay the killer's own recent
                // movement for the replay's duration. Mirrors
                // `record_local_replay`'s identical gating below. Also paused
                // for the same reason while a fall-death hold is up (see
                // `fall_death`) — the rig's rotation is being driven by that
                // effect's own look-at, not the player's live input.
                .run_if(
                    in_state(AppState::InGame)
                        .and(killcam::no_killcam)
                        .and(fall_death::no_fall_death),
                ),
        );
        app.add_systems(
            Update,
            (
                mark_local_input,
                spawn_remote_avatars,
                mark_bot_avatars,
                crate::tint_bot_avatars,
                follow_remote_avatars,
                animate_remote_avatars,
                hide_remote_avatars_during_killcam,
                spawn_sniper_glints,
                update_sniper_glints,
                spawn_bot_avatars,
                sync_bot_poses,
                receive_shots,
                receive_remote_sounds,
                receive_trick_scores,
                receive_match_over,
                receive_killcam,
                receive_respawn,
                // Must see this frame's `receive_killcam` (if the server's
                // "best play" `KillCam` and `MatchOver` land in the same
                // frame) before deciding whether a kill cam is holding the
                // results screen off — the tuple above isn't `.chain()`ed,
                // so this ordering needs to be explicit.
                flush_pending_match_end.after(receive_killcam),
                // Same reasoning: don't decide a `FreeForAll` kill cam isn't
                // coming until this frame's `receive_killcam` has had a
                // chance to start one.
                flush_pending_respawn.after(receive_killcam),
                dev_auto_fire.run_if(|| std::env::var_os("TRICKSHOT_AUTO_FIRE").is_some()),
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// Marker for the single client-connection entity.
#[derive(Component)]
pub struct GameClient;

fn connect(mut commands: Commands) {
    let (addr, private_key) = server_addr();
    let auth = Authentication::Manual {
        server_addr: addr,
        client_id: dev_client_id(),
        private_key,
        protocol_id: shared::PROTOCOL_ID,
    };
    let client = match NetcodeClient::new(auth, NetcodeConfig::default()) {
        Ok(c) => c,
        Err(e) => {
            error!("failed to build netcode client: {e:?}");
            return;
        }
    };

    // Bind the local socket in the same address family as the server, or the
    // first `sendto` fails with EAFNOSUPPORT (fly.io is IPv6-only).
    let local: SocketAddr = if addr.is_ipv6() {
        (Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    };

    let entity = commands
        .spawn((
            Name::from("GameClient"),
            GameClient,
            client,
            LocalAddr(local),
            UdpIo::default(),
            ReplicationReceiver::default(),
            PredictionManager::default(),
            InterpolationManager::default(),
        ))
        .id();
    commands.trigger_targets(Connect, entity);
    info!("connecting to server at {addr}");
}

fn on_connected(
    _t: Trigger<OnAdd, Connected>,
    q: Query<&LocalId, (With<GameClient>, With<Connected>)>,
) {
    if let Ok(local) = q.single() {
        info!("connected to server (local peer {:?})", local.0);
    } else {
        info!("connected to server");
    }
}

// --- local input --------------------------------------------------------

/// Attach the `InputMarker` to our own predicted player once it's replicated in,
/// so lightyear ships its `ActionState<PlayerInput>` to the server each tick.
#[allow(clippy::type_complexity)]
fn mark_local_input(
    local: Query<&LocalId, With<GameClient>>,
    predicted: Query<(Entity, &PlayerId), (With<Predicted>, Without<InputMarker<PlayerInput>>)>,
    mut commands: Commands,
) {
    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    for (entity, id) in &predicted {
        if id.0 == me {
            commands
                .entity(entity)
                .insert(InputMarker::<PlayerInput>::default());
            info!("local player {entity:?} ready — sending input");
        }
    }
}

/// Copy this frame's local pose (client-authoritative) into the input packet,
/// and — if `weapon_system` pulled the trigger — the fire request too.
#[allow(clippy::too_many_arguments)]
fn write_input(
    player: Query<&Transform, With<Player>>,
    head: Query<&Transform, With<PlayerHead>>,
    cam: Query<&GlobalTransform, With<WorldModelCamera>>,
    physics: Query<&PlayerPhysics, With<Player>>,
    anim_players: Query<&AnimationPlayer, With<crate::SniperAnimationPlayer>>,
    view_models: Query<&crate::ViewModelAnimation>,
    ads: Res<Ads>,
    slide: Res<crate::Slide>,
    shake: Res<crate::Shake>,
    sway: Res<crate::WeaponSwayState>,
    settings: Res<crate::settings::Settings>,
    mut trick: ResMut<TrickState>,
    // Kill-cam recording resources, bundled — a system tops out at 16 params.
    (mut snd, mut ground_hit, mut blood_hit, mut tracer_rec): (
        ResMut<ReplaySoundBits>,
        ResMut<killcam::ReplayGroundImpact>,
        ResMut<killcam::ReplayBloodImpact>,
        ResMut<killcam::ReplayTracer>,
    ),
    mut pending: ResMut<PendingShot>,
    mut q: Query<&mut ActionState<PlayerInput>, With<InputMarker<PlayerInput>>>,
    (
        view_model_vis,
        knife,
        weapon,
        jumping,
        mut pending_melee,
        knife_players,
        knife_anims,
        knife_view_vis,
        arms_players,
        arms_anims,
    ): (
        Query<&Visibility, With<crate::ViewModel>>,
        Res<crate::ThrowingKnife>,
        Res<crate::Weapon>,
        Res<crate::Jumping>,
        ResMut<crate::PendingMelee>,
        Query<&AnimationPlayer, With<crate::KnifeAnimationPlayer>>,
        Query<&crate::KnifeAnimation>,
        Query<&Visibility, With<crate::KnifeViewModel>>,
        Query<&AnimationPlayer, With<crate::ThrowArmsAnimationPlayer>>,
        Query<&crate::ThrowArmsAnimation>,
    ),
) {
    let (Ok(pt), Ok(ht), Ok(mut action)) = (player.single(), head.single(), q.single_mut()) else {
        return;
    };
    action.translation = pt.translation.to_array();
    action.yaw = pt.rotation.to_euler(EulerRot::YXZ).0;
    action.pitch = ht.rotation.to_euler(EulerRot::YXZ).1;
    action.weapon = shared::weapon::WeaponId::Sniper.as_u8();
    action.fire = false;
    // A knife stab this frame (`weapon_system` filed it) rides the same
    // origin/dir fields a shot uses — the two are never both pending, since
    // the knife and the sniper aren't drawn at once.
    action.melee = false;
    if let Some((origin, dir)) = pending_melee.0.take() {
        action.melee = true;
        action.fire_origin = origin.to_array();
        action.fire_dir = dir.to_array();
    }
    // Kill-cam recording: the camera-shake state, weapon-sway offset and hip
    // FOV, so a replay can reproduce shake / recoil / sway and render at this
    // player's FOV instead of re-deriving an absolute camera transform that
    // would just duplicate `translation` / `yaw` / `pitch` above.
    action.shake_trauma = shake.trauma;
    action.shake_phase = shake.phase;
    action.shake_recoil = shake.recoil;
    action.sway_offset = sway.offset.to_array();
    action.fov_deg = settings.fov;
    action.scope_zoom = settings.scope_zoom.magnification();
    action.sound_bits = std::mem::take(&mut snd.0);
    action.anim_time = crate::killcam::viewmodel_anim_time(&anim_players, &view_models);
    action.knife_anim_time = crate::killcam::knife_anim_time(&knife_players, &knife_anims);
    action.ads_t = ads.t;
    action.crouching = slide.stance == crate::Stance::Crouching;
    action.sliding = slide.stance == crate::Stance::Sliding;
    action.crouch_drop = slide.drop;
    action.ground_pt = ground_hit.0.take().map(|p| p.to_array());
    action.blood_pt = blood_hit.0.take().map(|p| p.to_array());
    action.tracer = tracer_rec
        .0
        .take()
        .map(|(s, e)| [s.to_array(), e.to_array()]);
    action.weapon_visible = view_model_vis
        .iter()
        .next()
        .is_none_or(|v| *v != Visibility::Hidden);
    action.knife_active = knife.active;
    action.sniper_active = weapon.slot == crate::WeaponSlot::Primary;
    // The throwing arms' state, so the kill cam can replay the hold / throw,
    // and the melee knife's own visibility, which can't be derived from the
    // sniper's any more (both are hidden while the arms are up).
    action.knife_visible = knife_view_vis
        .iter()
        .next()
        .is_none_or(|v| *v != Visibility::Hidden);
    action.arms_slide = knife.slide;
    action.arms_anim_time = crate::throw_arms_anim_time(&arms_players, &arms_anims);
    action.arms_knife_in_hand = knife.knife_in_hand();
    action.reloading = weapon
        .busy
        .as_ref()
        .is_some_and(|b| b.on_finish == crate::WeaponFinish::Reload);
    action.jumping = jumping.0;

    // Emit the shot from the world camera's viewpoint, along the direction
    // `weapon_system` computed (crosshair aim plus no-scope inaccuracy). Keep the
    // pending flag if the camera isn't ready yet, rather than dropping the shot.
    if let Some(dir) = pending.0 {
        if let Ok(cam) = cam.single() {
            pending.0 = None;
            action.fire = true;
            action.fire_origin = cam.translation().to_array();
            action.fire_dir = dir.to_array();
            // Trick metadata for server-side scoring, then reset for the next shot.
            let grounded = physics.single().map(|p| p.grounded).unwrap_or(true);
            action.spin_deg = trick.total_deg();
            action.airborne = trick.airborne || !grounded;
            action.noscope = ads.t <= NOSCOPE_ADS_MAX;
            trick.reset();
        }
    }
}

/// Server → everyone: a shot scored style points. Pop the yellow stack for our
/// own shooter id.
fn receive_trick_scores(
    local: Query<&LocalId, With<GameClient>>,
    mut receivers: Query<&mut MessageReceiver<TrickScore>>,
    mut scored: EventWriter<TrickScoredEvent>,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if Some(msg.shooter) != me {
                continue;
            }
            scored.write(TrickScoredEvent {
                total: msg.total,
                lines: msg.lines.into_iter().map(|l| (l.label, l.points)).collect(),
            });
        }
    }
}

/// Holds a received [`MatchOver`] until it's safe to show the results screen
/// — see `flush_pending_match_end`.
#[derive(Resource, Default)]
struct PendingMatchEnd {
    msg: Option<MatchOver>,
    /// Set from `msg.best_play_sent` once `msg` arrives: whether we should
    /// expect a best-play [`KillCam`] and hold off until it's actually
    /// played, rather than just "no kill cam is active *right now*" — the
    /// two messages ride the same reliable but *unordered* `GameChannel`, so
    /// `MatchOver` can arrive before the replay it's paired with.
    expect_best_play: bool,
    /// Set once that expected best-play replay has been observed to start,
    /// so `flush_pending_match_end` stops waiting for it to *arrive* and
    /// starts waiting for it (or whatever else is playing) to *finish*.
    best_play_seen: bool,
    /// Seconds spent waiting on `expect_best_play` so far. `receive_killcam`
    /// silently drops an incoming replay if one is already active
    /// (`killcam::begin_from_message`), so if the best play happens to lose
    /// that race — or is simply lost — this could otherwise wait forever;
    /// past `BEST_PLAY_TIMEOUT_SECS` we give up and show results anyway.
    waited_secs: f32,
}

/// However long the slow-mo ramp can stretch a best-play replay's ~4.5 s
/// window out to, plus real headroom for network delay — see
/// `PendingMatchEnd::waited_secs`.
const BEST_PLAY_TIMEOUT_SECS: f32 = 12.0;

/// Server → everyone in the lobby: the match clock ran out. Doesn't fire
/// [`MatchEndedEvent`] directly — see [`PendingMatchEnd`].
fn receive_match_over(
    mut receivers: Query<&mut MessageReceiver<MatchOver>>,
    mut pending: ResMut<PendingMatchEnd>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            pending.expect_best_play = msg.best_play_sent;
            pending.best_play_seen = false;
            pending.waited_secs = 0.0;
            pending.msg = Some(msg);
        }
    }
}

/// Fires [`MatchEndedEvent`] once it's safe to: immediately if the match had
/// no best play (nobody scored), otherwise only after that replay has both
/// started and finished — see [`PendingMatchEnd`].
fn flush_pending_match_end(
    time: Res<Time>,
    mut pending: ResMut<PendingMatchEnd>,
    active: Res<killcam::ActiveKillCam>,
    freeze: Res<crate::match_end::MatchEndFreeze>,
    mut ended: EventWriter<MatchEndedEvent>,
) {
    if pending.msg.is_none() {
        return;
    }
    // The timed VICTORY / DEFEAT screen always plays out first.
    if freeze.overlay_running() {
        return;
    }
    if pending.expect_best_play && !pending.best_play_seen {
        match &active.0 {
            // It's arrived and started — now just wait for it (below) like
            // any other in-flight cam.
            Some(run) if run.best_play => pending.best_play_seen = true,
            // Hasn't arrived yet (`GameChannel` is unordered) — keep waiting
            // rather than risk showing results before, or instead of, it —
            // unless it's been long enough that it's evidently not coming.
            _ => {
                pending.waited_secs += time.delta_secs();
                if pending.waited_secs < BEST_PLAY_TIMEOUT_SECS {
                    return;
                }
            }
        }
    }
    if active.0.is_some() {
        return;
    }
    let msg = pending.msg.take().unwrap();
    ended.write(MatchEndedEvent {
        winner: msg.winner_name,
        score: msg.winner_score,
    });
}

/// Holds a received [`PlayerRespawn`] until it's safe to apply — mirrors
/// [`PendingMatchEnd`]'s wait-for-the-paired-kill-cam shape: the respawn
/// message and the kill cam of our own death ride the same reliable but
/// *unordered* `GameChannel`, and the cam is buffered ~1.5 s server-side (see
/// `server::killcam::flush_killcams`), so it can easily arrive after this.
#[derive(Resource, Default)]
struct PendingRespawn {
    to: Option<(Vec3, f32)>,
    /// Set once a kill cam has been seen to start since `to` arrived, so we
    /// know to wait for it to *finish* rather than teleport out from under it.
    seen_killcam: bool,
    /// Seconds spent waiting for that kill cam to even start. If none shows up
    /// in time (the recorded window can be too short to send at all), stop
    /// waiting and just respawn.
    waited_secs: f32,
}

/// However long to wait for the paired kill cam to *start* before giving up
/// on it and respawning anyway.
const RESPAWN_KILLCAM_TIMEOUT_SECS: f32 = 3.0;

/// Server → the local player only: we're allowed to respawn, and here's
/// where — sent for a `FreeForAll` PvP kill or, in either game mode, falling
/// to death (see `fall_death`).
fn receive_respawn(
    mut receivers: Query<&mut MessageReceiver<PlayerRespawn>>,
    mut pending: ResMut<PendingRespawn>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            pending.to = Some((Vec3::from_array(msg.pos), msg.yaw));
            // A match-start spawn has no kill cam to wait for.
            pending.seen_killcam = msg.immediate;
            pending.waited_secs = 0.0;
        }
    }
}

/// Fired the moment the local player's rig is actually teleported back to
/// life by [`flush_pending_respawn`] — lets other systems (e.g.
/// `death_effect`) know to clear anything still tied to the death that just
/// ended, even on the fallback path where no paired kill cam ever arrived.
#[derive(Event)]
pub(crate) struct LocalPlayerRespawned;

/// Once it's safe — the paired kill cam (if one ever arrives) has both
/// started and finished, or enough time has passed that it evidently isn't
/// coming — teleport the local player rig to the stored respawn point.
fn flush_pending_respawn(
    time: Res<Time>,
    mut pending: ResMut<PendingRespawn>,
    active: Res<ActiveKillCam>,
    mut weapon: ResMut<crate::Weapon>,
    player: Single<(&mut Transform, &mut PlayerPhysics), (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
    mut respawned: EventWriter<LocalPlayerRespawned>,
    mut ready: Query<&mut TriggerSender<shared::RespawnReady>, With<GameClient>>,
) {
    let Some((pos, yaw)) = pending.to else {
        return;
    };
    if active.0.is_some() {
        pending.seen_killcam = true;
        return;
    }
    if !pending.seen_killcam {
        pending.waited_secs += time.delta_secs();
        if pending.waited_secs < RESPAWN_KILLCAM_TIMEOUT_SECS {
            return;
        }
    }
    let (mut transform, mut physics) = player.into_inner();
    // `pos` is the server's ground-level spawn point (`shared::spawns::spawn_point`
    // docs it as such) — the `Player` rig's own translation is eye height, not
    // feet (see `sim.rs`'s note on `PlayerPose::translation`, and the same
    // `+ EYE_HEIGHT` conversion in reverse at this file's line ~667), so
    // skipping this left the rig's feet a full `EYE_HEIGHT` below the real
    // floor — `apply_gravity`'s downward-only raycast can never find a floor
    // above where it starts, so the player fell forever until manually
    // teleported.
    transform.translation = pos + Vec3::Y * crate::EYE_HEIGHT;
    transform.rotation = Quat::from_rotation_y(yaw);
    // Always spawn looking level (pitch 0).
    head.rotation = Quat::IDENTITY;
    *physics = PlayerPhysics::default();
    weapon.refill_ammo();
    pending.to = None;
    respawned.write(LocalPlayerRespawned);
    // Tell the server: it brings us back to life now (full health, targetable)
    // rather than when its own timer runs out — which a skipped kill cam beats.
    if let Ok(mut sender) = ready.single_mut() {
        sender.trigger::<shared::LobbyChannel>(shared::RespawnReady);
    }
}

/// Server → everyone in the lobby: the killer's last ~3 s to replay.
fn receive_killcam(
    mut receivers: Query<&mut MessageReceiver<KillCam>>,
    mut active: ResMut<ActiveKillCam>,
    // An end-of-match replay that arrived while our own kill cam was still
    // playing (`begin_from_message` drops anything that arrives mid-replay):
    // held here and started as soon as that one finishes.
    mut queued_end_cam: Local<Option<KillCam>>,
    freeze: Res<crate::match_end::MatchEndFreeze>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            // (Also held while the VICTORY / DEFEAT screen is still up.)
            if msg.best_play && (active.0.is_some() || freeze.overlay_running()) {
                *queued_end_cam = Some(msg);
            } else {
                killcam::begin_from_message(&mut active, msg);
            }
        }
    }
    if active.0.is_none() && !freeze.overlay_running() {
        if let Some(msg) = queued_end_cam.take() {
            killcam::begin_from_message(&mut active, msg);
        }
    }
}

/// Drain the server's authoritative [`ShotResolved`] broadcasts: a `Ground`
/// outcome becomes a [`GroundImpact`] event so everyone sees a debris burst at
/// the same spot, and every outcome fires a [`crate::FireTracer`] so everyone
/// sees the shot's tracer along its true, server-resolved path. Our *own*
/// shots are skipped here — `resolve_local_shot` already spawned both locally,
/// instantly, the moment we fired.
///
/// While a kill cam is playing the messages are still drained (so they don't
/// dump in a batch when it ends) but their effects are dropped — the replay
/// paints only the killer's own recorded tracers / bursts.
fn receive_shots(
    local: Query<&LocalId, With<GameClient>>,
    active: Res<ActiveKillCam>,
    mut receivers: Query<&mut MessageReceiver<ShotResolved>>,
    mut impacts: EventWriter<GroundImpact>,
    mut holes: EventWriter<crate::BulletImpact>,
    mut tracers: EventWriter<crate::FireTracer>,
) {
    let me = local.iter().next().map(|l| l.0);
    let killcam_playing = active.0.is_some();
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if killcam_playing || Some(msg.shooter) == me {
                continue;
            }
            if let ShotOutcome::Ground { point, normal } = msg.outcome {
                impacts.write(GroundImpact(Vec3::from_array(point)));
                holes.write(crate::BulletImpact {
                    point: Vec3::from_array(point),
                    normal: Vec3::from_array(normal),
                });
            }
            tracers.write(crate::FireTracer {
                start: Vec3::from_array(msg.origin),
                end: Vec3::from_array(msg.tracer_end),
            });
        }
    }
}

/// Server → everyone else in the shooter's lobby: play another player's
/// one-shot sounds (footstep/jump/slide/reload/rechamber/shot/dive)
/// positionally at their reported position, falling off with distance. The
/// server already excludes the triggering player from the message's targets
/// (they hear their own local, non-spatial version instead), but the `me`
/// check here is a cheap defensive backstop.
#[allow(clippy::too_many_arguments)]
fn receive_remote_sounds(
    local: Query<&LocalId, With<GameClient>>,
    listener: Query<&GlobalTransform, With<WorldModelCamera>>,
    mut receivers: Query<&mut MessageReceiver<shared::RemoteSound>>,
    sounds: Res<crate::GameSounds>,
    sound_vol: Res<crate::SoundVolumes>,
    remote_sound: Res<crate::RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut footstep_seed: Local<u32>,
    mut footstep_last: Local<std::collections::HashMap<PeerId, usize>>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    let Ok(ear) = listener.single() else {
        return;
    };
    let ear = ear.translation();

    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if Some(msg.player) == me {
                continue;
            }
            let pos = Vec3::from_array(msg.position);
            let distance = ear.distance(pos);
            if distance >= remote_sound.max_distance {
                continue;
            }
            // Linear fade to silence at `max_distance`, squared for a
            // steeper near-field/far-field falloff than a straight ramp.
            let falloff = (1.0 - distance / remote_sound.max_distance).powi(2);

            for bit in killcam::ALL_SND_BITS {
                if msg.bits & bit == 0 {
                    continue;
                }
                let clip = if bit == killcam::SND_FOOTSTEP {
                    *footstep_seed = footstep_seed.wrapping_add(1);
                    let last = footstep_last.entry(msg.player).or_default();
                    killcam::pick_footstep(&sounds.footsteps, last, *footstep_seed)
                } else {
                    killcam::sound_for(&sounds, bit).cloned()
                };
                let Some(clip) = clip else { continue };
                let category = sound_vol.oneshot_for(&clip, &sounds).unwrap_or(1.0);
                let mult = (category * falloff * remote_sound.volume).max(0.0);
                let volume = Volume::Linear(mult) * global_volume.volume;
                commands.spawn((
                    crate::RemoteSoundEmitter,
                    AudioPlayer::new(clip),
                    Transform::from_translation(pos),
                    PlaybackSettings::DESPAWN
                        .with_spatial(true)
                        // rodio's spatial source attenuates each ear by
                        // `1 / distance²` in *raw* world units (see
                        // `rodio::source::spatial::Spatial::set_positions`),
                        // uncapped below 1 unit — at our meter-scale world
                        // that's already ~1% volume by 10m, drowning out the
                        // falloff we actually want to control below. Shrink
                        // the distance rodio sees so its own curve stays
                        // near-flat (panning only) across the whole hearing
                        // range, leaving `falloff` above as the sole thing
                        // that actually decides loudness by distance.
                        .with_spatial_scale(SpatialScale::new(0.01))
                        .with_volume(volume),
                ));
            }
        }
    }
}

// --- remote players (models/soldier.glb, idleWgun looping) ------------

/// A `models/soldier.glb` avatar standing in for another player; follows their
/// interpolated pose.
#[derive(Component)]
pub(crate) struct RemoteAvatar {
    pub(crate) src: Entity,
}

/// Tracks a remote avatar's previous position and currently-playing animation
/// so `animate_remote_avatars` can tell idle/walk/sprint apart from
/// frame-to-frame movement speed and only call `AnimationPlayer::play` on a
/// change (it starts on `idleWgun`, matching `start_soldier_animation`).
#[derive(Component, Default)]
pub(crate) struct RemoteAvatarMotion {
    prev_translation: Option<Vec3>,
    state: crate::SoldierAnimState,
}

/// Spawn a soldier avatar for every interpolated (i.e. *other*-player)
/// `PlayerPose` entity that doesn't have one yet. Polled rather than an
/// `OnAdd` observer so it doesn't matter whether `Interpolated` or
/// `PlayerPose` lands first.
fn spawn_remote_avatars(
    remotes: Query<Entity, (With<PlayerPose>, With<Interpolated>)>,
    avatars: Query<&RemoteAvatar>,
    remote_avatar_settings: Res<crate::RemoteAvatarSettings>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    let have: std::collections::HashSet<Entity> = avatars.iter().map(|a| a.src).collect();
    for src in &remotes {
        if have.contains(&src) {
            continue;
        }
        spawn_soldier_avatar(&mut commands, &asset_server, remote_avatar_settings.scale, src);
        info!("remote player {src:?} — spawned soldier avatar");
    }
}

/// A live `models/soldier.glb` [`RemoteAvatar`] following the `PlayerPose` on
/// `src` — another player's, or a `Freestyle` bot's [`BotPose`] stand-in.
fn spawn_soldier_avatar(
    commands: &mut Commands,
    asset_server: &AssetServer,
    scale: f32,
    src: Entity,
) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RemoteAvatar { src },
            RemoteAvatarMotion::default(),
            crate::SoldierVisual,
            Transform::from_scale(Vec3::splat(scale)),
            Visibility::default(),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/soldier.glb"))),
        ))
        .observe(crate::start_soldier_animation);
}

fn follow_remote_avatars(
    poses: Query<&PlayerPose>,
    remote_avatar_settings: Res<crate::RemoteAvatarSettings>,
    mut avatars: Query<(Entity, &RemoteAvatar, &mut Transform)>,
    mut commands: Commands,
) {
    for (entity, avatar, mut tf) in &mut avatars {
        match poses.get(avatar.src) {
            Ok(pose) => {
                // The replicated pose is at eye level; the model's origin is
                // at its feet (same convention `bot.pos` uses), so drop by
                // the same eye height the local rig's own ground-snap uses.
                tf.translation = pose.translation - Vec3::Y * crate::EYE_HEIGHT;
                // `soldier.glb`'s forward faces +Z, opposite the local rig's
                // -Z convention that `pose.yaw` is authored in, so flip it.
                tf.rotation = Quat::from_rotation_y(pose.yaw + std::f32::consts::PI);
                tf.scale = Vec3::splat(remote_avatar_settings.scale);
            }
            Err(_) => {
                commands.entity(entity).try_despawn();
            }
        }
    }
}

/// Give the bot look ([`crate::BotLook`]) to every live avatar that's a bot:
/// a `Freestyle` target's [`BotPose`] stand-in, or a `FreeForAll` bot player
/// (a bot peer id). Polled, since an avatar can exist a frame before its
/// source's `PlayerId` arrives. (Kill-cam stand-ins get it when spawned.)
fn mark_bot_avatars(
    avatars: Query<(Entity, &RemoteAvatar), Without<crate::BotLook>>,
    sources: Query<(Option<&PlayerId>, Has<BotPose>)>,
    mut commands: Commands,
) {
    for (entity, avatar) in &avatars {
        let Ok((id, stand_in)) = sources.get(avatar.src) else {
            continue;
        };
        if stand_in || id.is_some_and(|id| shared::bot_players::is_bot_peer(id.0)) {
            commands.entity(entity).insert(crate::BotLook);
        }
    }
}

/// Spawn a [`SniperGlint`] for every [`RemoteAvatar`] that doesn't have one
/// yet — mirrors `spawn_remote_avatars`'s polling, kept as its own entity
/// rather than a child of it (see [`SniperGlint`]'s doc comment for why).
/// Each gets its own material clone off [`SniperGlintAssets::texture`] so its
/// alpha can fade independently of every other glint's (mirrors
/// `vfx::impacts`).
fn spawn_sniper_glints(
    avatars: Query<(Entity, &RemoteAvatar)>,
    glints: Query<&SniperGlint>,
    assets: Res<SniperGlintAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let have: std::collections::HashSet<Entity> = glints.iter().map(|g| g.avatar).collect();
    for (avatar_entity, avatar) in &avatars {
        if have.contains(&avatar_entity) {
            continue;
        }
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(assets.texture.clone()),
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        commands.spawn((
            StateScoped(AppState::InGame),
            SniperGlint {
                src: avatar.src,
                avatar: avatar_entity,
            },
            Mesh3d(assets.quad.clone()),
            MeshMaterial3d(material),
            Transform::default(),
            Visibility::Hidden,
            NoFrustumCulling,
        ));
    }
}

/// Follows each [`SniperGlint`] to its anchor — [`SniperGlintBone`]'s current
/// animated `GlobalTransform` plus `SniperGlintSettings::offset` rotated into
/// its orientation, once the scene has found the gun mesh node (falls back to
/// the replicated pose, unanimated, for the frame or so before it has) — so
/// it rides along with the actual gun mesh through every animation (walk bob,
/// aim raise, recoil) instead of floating in place while the model moves
/// under it. Billboards it to face the local camera dead-on, and fades its
/// opacity with `PlayerPose::ads_t` past `ads_threshold`, so it appears right
/// as the remote avatar visibly raises its rifle. Forced off while the owner
/// is dead — same priority `animate_remote_avatars` gives `Dead` over `Aim` —
/// so it doesn't linger on a corpse for the moment `Ads::t` takes to decay
/// back to `0` after the killing blow. Also forced off during a kill cam —
/// the live avatar itself is hidden then too (see
/// `hide_remote_avatars_during_killcam`), replaced by the replay's own
/// stand-ins, which don't carry a glint of their own.
#[allow(clippy::too_many_arguments)]
fn update_sniper_glints(
    poses: Query<&PlayerPose>,
    bones: Query<Option<&SniperGlintBone>, With<RemoteAvatar>>,
    transforms: Query<&GlobalTransform>,
    settings: Res<SniperGlintSettings>,
    killcam: Res<killcam::ActiveKillCam>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    mut glints: Query<(
        Entity,
        &SniperGlint,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let cam_pos = camera.translation();

    for (entity, glint, mut tf, mut vis, material) in &mut glints {
        let Ok(pose) = poses.get(glint.src) else {
            commands.entity(entity).try_despawn();
            continue;
        };

        // The gun bone's `GlobalTransform` carries the huge
        // `RemoteAvatarSettings::scale` (~21×) baked into its own scale
        // component, so only its translation/rotation are used — the offset
        // stays in real-world metres regardless, same as the pose fallback.
        let anchor = bones
            .get(glint.avatar)
            .ok()
            .flatten()
            .and_then(|bone| transforms.get(bone.0).ok());
        let (origin, rotation) = match anchor {
            Some(gt) => (gt.translation(), gt.rotation()),
            None => (pose.translation, Quat::from_rotation_y(pose.yaw)),
        };
        let pos = origin
            + rotation * Vec3::X * settings.offset.x
            + rotation * Vec3::Y * settings.offset.y
            + rotation * Vec3::NEG_Z * settings.offset.z;

        // Always face the local player dead-on, regardless of the gun's own
        // orientation — `looking_at`'s `up` only matters when the look
        // direction is near-parallel to it, which a roughly-horizontal sight
        // line to another player's scope never is.
        tf.translation = pos;
        tf.rotation = Transform::from_translation(pos)
            .looking_at(cam_pos, Vec3::Y)
            .rotation;
        tf.scale = Vec3::splat(settings.scale.max(1.0e-4));

        let amount = if killcam.0.is_some() || !pose.alive {
            0.0
        } else {
            let span = (1.0 - settings.ads_threshold).max(1e-3);
            ((pose.ads_t - settings.ads_threshold) / span).clamp(0.0, 1.0)
        };

        if let Some(material) = materials.get_mut(&material.0) {
            material.base_color = Color::srgba(1.0, 1.0, 1.0, amount);
        }
        *vis = if amount > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Hides every live remote-player avatar for the duration of a kill cam, and
/// restores them once it ends. A kill cam replays the past (`start_killcam`
/// stands in `models/soldier.glb` ghosts that follow each other player's
/// *recorded* movement — see `killcam::drive_killcam_actors`) — without this,
/// these *live*, continuously updated avatars would wander through it and
/// could end up right on top of the replay camera, blocking the view entirely.
fn hide_remote_avatars_during_killcam(
    active: Res<killcam::ActiveKillCam>,
    // (Not the kill cam's own stand-ins — see `killcam::KillCamPlayerGhost`.)
    mut avatars: Query<&mut Visibility, (With<RemoteAvatar>, Without<killcam::KillCamPlayerGhost>)>,
) {
    let want = if active.0.is_some() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut vis in &mut avatars {
        if *vis != want {
            *vis = want;
        }
    }
}

/// Switches each remote avatar between `idleWgun`/`walk`/`run`/`shooting`/
/// `runAndShooting`/`crouch`/`crouchWalk`/`strafeRight`/`strafeLeft`/
/// `backpaddle` based on how fast — and in which direction relative to its
/// own facing — its interpolated pose is actually moving frame-to-frame, and
/// whether it's aiming (`pose.ads_t`) or crouching (`pose.crouching`). Keeps
/// each clip's playback speed tied to the current `MovementSettings`/
/// `SlideSettings` speeds (the "Movement"/"Slide" debug-panel sliders) so the
/// feet still match the ground after those are changed, without a separate
/// "animation speed" control to keep in sync by hand.
#[allow(clippy::too_many_arguments)]
fn animate_remote_avatars(
    poses: Query<&PlayerPose>,
    time: Res<Time>,
    anims: Res<crate::SoldierAnimations>,
    anim_settings: Res<crate::SoldierAnimSettings>,
    movement: Res<crate::MovementSettings>,
    slide_cfg: Res<crate::SlideSettings>,
    mut avatars: Query<(
        &RemoteAvatar,
        &mut RemoteAvatarMotion,
        &crate::SoldierAnimationPlayer,
        Has<killcam::KillCamPlayerGhost>,
    )>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    killcam: Res<killcam::ActiveKillCam>,
) {
    const BLEND_DURATION: Duration = Duration::from_millis(200);
    /// `ads_t` at or above this counts as "aiming" for animation purposes.
    const AIM_THRESHOLD: f32 = 0.5;

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let walk_enter = movement.walk_speed * 0.5;
    let sprint_enter = (movement.walk_speed + movement.sprint_speed) * 0.5;
    let crouch_walk_enter = slide_cfg.crouch_speed * 0.5;
    // `SoldierAnimSettings`'s multipliers are calibrated at the *default*
    // WALK_SPEED/SPRINT_SPEED/CROUCH_SPEED, so scale them by how far the live
    // movement/slide settings have drifted from those defaults.
    let walk_anim_speed = anim_settings.base_walk_speed * (movement.walk_speed / crate::WALK_SPEED);
    let sprint_anim_speed =
        anim_settings.base_sprint_speed * (movement.sprint_speed / crate::SPRINT_SPEED);
    let aim_walk_anim_speed =
        anim_settings.base_aim_walk_speed * (movement.walk_speed / crate::WALK_SPEED);
    let aim_sprint_anim_speed =
        anim_settings.base_aim_sprint_speed * (movement.sprint_speed / crate::SPRINT_SPEED);
    let crouch_walk_anim_speed =
        anim_settings.base_crouch_walk_speed * (slide_cfg.crouch_speed / crate::CROUCH_SPEED);
    // `strafeRight`/`strafeLeft`/`backpaddle` only ever play at walk pace, so
    // these are calibrated against the *effective* strafe/backward speed
    // (walk speed × the direction multiplier) rather than walk speed alone.
    let strafe_anim_speed = anim_settings.base_strafe_speed
        * ((movement.walk_speed * movement.strafe_speed_mult)
            / (crate::WALK_SPEED * crate::STRAFE_SPEED_MULT));
    let backpaddle_anim_speed = anim_settings.base_backpaddle_speed
        * ((movement.walk_speed * movement.backward_speed_mult)
            / (crate::WALK_SPEED * crate::BACKWARD_SPEED_MULT));

    // A kill-cam stand-in moves at the replay's pace (slower in a best play's
    // slow-mo dip), so judge its walking speed against replay time.
    let replay_speed = killcam.0.as_ref().map_or(1.0, |run| run.speed).max(0.05);

    for (avatar, mut motion, anim_player, in_replay) in &mut avatars {
        let Ok(pose) = poses.get(avatar.src) else {
            continue;
        };
        let dt = if in_replay { dt * replay_speed } else { dt };

        // Just respawned: `pose.translation` jumped straight from wherever
        // this avatar died to the new spawn point, which the frame-to-frame
        // speed calculation just below would otherwise read as a huge burst
        // of movement — a one-frame flash of `Sprint` (or worse) before it
        // settles on `Idle`. Priming `prev_translation` with *this* frame's
        // position first makes that delta (and so the computed speed) zero.
        if motion.state == crate::SoldierAnimState::Dead && pose.alive {
            motion.prev_translation = Some(pose.translation);
        }

        // Decompose frame-to-frame movement into the pose's own forward/right
        // basis (not just overall planar speed), so a pure sideways or
        // backward step can be told apart from a forward one.
        let (speed, forward_speed, right_speed) = match motion.prev_translation {
            Some(prev) => {
                let delta = pose.translation - prev;
                let facing = Quat::from_rotation_y(pose.yaw);
                let fwd = facing * Vec3::NEG_Z;
                let right = facing * Vec3::X;
                let f = delta.dot(fwd) / dt;
                let r = delta.dot(right) / dt;
                (f.hypot(r), f, r)
            }
            None => (0.0, 0.0, 0.0),
        };
        motion.prev_translation = Some(pose.translation);

        // Dead outranks everything, including jumping (a fatal shot mid-air
        // should still cut straight to the death pose) — held on its last
        // frame (`new_state != motion.state` below only calls `play()` on
        // the entry edge, and it's set to play once, not loop) until
        // `pose.alive` flips back to `true` on respawn. Jumping/reloading
        // take priority over everything below them. All three are sustained
        // flags, so each still plays its clip once — holding the last frame
        // — for as long as the flag stays true; once it clears, the state
        // below is picked again on the very next frame. `Jump` outranks
        // `Reload` since landing mid-reload should still show the jump/land
        // motion.
        let new_state = if !pose.alive {
            crate::SoldierAnimState::Dead
        } else if pose.jumping {
            crate::SoldierAnimState::Jump
        } else if pose.reloading {
            crate::SoldierAnimState::Reload
        } else if pose.sliding {
            // No dedicated slide clip — always the static crouch pose,
            // regardless of slide speed (never `crouchWalk`).
            crate::SoldierAnimState::Crouch
        } else if pose.crouching {
            // Crouching has no aiming/strafe variant of its own (`crouch`/
            // `crouchWalk` only), so it takes priority over everything below.
            if speed >= crouch_walk_enter {
                crate::SoldierAnimState::CrouchWalk
            } else {
                crate::SoldierAnimState::Crouch
            }
        } else if pose.ads_t >= AIM_THRESHOLD {
            // Aiming has no strafe/backpaddle variant either (only
            // `shooting`/`runAndShooting`), so direction doesn't matter here.
            if speed >= sprint_enter {
                crate::SoldierAnimState::AimSprint
            } else if speed >= walk_enter {
                crate::SoldierAnimState::AimWalk
            } else {
                crate::SoldierAnimState::Aim
            }
        } else if speed < walk_enter {
            crate::SoldierAnimState::Idle
        } else {
            let abs_forward = forward_speed.abs();
            let abs_right = right_speed.abs();
            // "Pure" meaning the other axis's component is negligible — the
            // model only has dedicated clips for straight forward/back/side
            // movement, not diagonals.
            if abs_right < walk_enter {
                if forward_speed < 0.0 {
                    crate::SoldierAnimState::Backward
                } else if speed >= sprint_enter {
                    crate::SoldierAnimState::Sprint
                } else {
                    crate::SoldierAnimState::Walk
                }
            } else if abs_forward < walk_enter {
                if right_speed > 0.0 {
                    crate::SoldierAnimState::StrafeRight
                } else {
                    crate::SoldierAnimState::StrafeLeft
                }
            } else if forward_speed >= 0.0 {
                // Diagonal forward+strafe: no dedicated clip, closest is the
                // plain forward walk/run.
                if speed >= sprint_enter {
                    crate::SoldierAnimState::Sprint
                } else {
                    crate::SoldierAnimState::Walk
                }
            } else {
                // Diagonal backward+strafe: closest is backpaddle.
                crate::SoldierAnimState::Backward
            }
        };

        let Ok((mut player, mut transitions)) = players.get_mut(anim_player.0) else {
            continue;
        };

        if new_state != motion.state {
            motion.state = new_state;
            let repeat = match new_state {
                crate::SoldierAnimState::Reload
                | crate::SoldierAnimState::Jump
                | crate::SoldierAnimState::Dead => RepeatAnimation::Never,
                _ => RepeatAnimation::Forever,
            };
            // `AnimationTransitions::play` fades the previous node's weight
            // down to 0 over `BLEND_DURATION` (and stops it once it reaches
            // 0) while the new node fades in, instead of a hard cut.
            transitions
                .play(&mut player, anims.node_for(new_state), BLEND_DURATION)
                .set_repeat(repeat);
        }

        // Re-applied every frame (not just on a state change) so a live edit
        // to the "Movement" walk/sprint sliders takes effect immediately.
        if let Some(active) = player.animation_mut(anims.node_for(motion.state)) {
            active.set_speed(match motion.state {
                crate::SoldierAnimState::Idle
                | crate::SoldierAnimState::Aim
                | crate::SoldierAnimState::Crouch
                | crate::SoldierAnimState::Reload
                | crate::SoldierAnimState::Jump => 1.0,
                crate::SoldierAnimState::Walk => walk_anim_speed,
                crate::SoldierAnimState::Sprint => sprint_anim_speed,
                crate::SoldierAnimState::AimWalk => aim_walk_anim_speed,
                crate::SoldierAnimState::AimSprint => aim_sprint_anim_speed,
                crate::SoldierAnimState::CrouchWalk => crouch_walk_anim_speed,
                crate::SoldierAnimState::StrafeRight | crate::SoldierAnimState::StrafeLeft => {
                    strafe_anim_speed
                }
                crate::SoldierAnimState::Backward => backpaddle_anim_speed,
                crate::SoldierAnimState::Dead => anim_settings.death_speed,
            });
        }
    }
}

// --- Freestyle target bots (drawn as remote-player soldiers) ----------

/// A client-only stand-in `PlayerPose` for a server-owned [`shared::Bot`] —
/// the same trick the kill cam's `KillCamPose` uses — so the bot is drawn by a
/// normal [`RemoteAvatar`] and walks / dies through exactly the same
/// animation logic as another player. Despawned with the bot, which takes the
/// avatar with it (`follow_remote_avatars`).
#[derive(Component)]
pub(crate) struct BotPose {
    pub(crate) bot: Entity,
    /// Set once the bot's death has been seen, so the blood burst fires once.
    died: bool,
}

const BOT_H: f32 = 1.8;

/// A bot's state as the `PlayerPose` its avatar animates from (eye height,
/// like a player's).
fn bot_player_pose(bot: &Bot) -> PlayerPose {
    PlayerPose {
        translation: bot.pos + Vec3::Y * crate::EYE_HEIGHT,
        yaw: bot.yaw,
        alive: bot.alive,
        ..default()
    }
}

fn spawn_bot_avatars(
    bots: Query<(Entity, &Bot), With<Interpolated>>,
    stand_ins: Query<&BotPose>,
    remote_avatar_settings: Res<crate::RemoteAvatarSettings>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    let have: std::collections::HashSet<Entity> = stand_ins.iter().map(|p| p.bot).collect();
    for (src, bot) in &bots {
        if have.contains(&src) {
            continue;
        }
        let pose = commands
            .spawn((
                StateScoped(AppState::InGame),
                // Already dead when first seen: no burst to play.
                BotPose { bot: src, died: !bot.alive },
                bot_player_pose(bot),
            ))
            .id();
        spawn_soldier_avatar(&mut commands, &asset_server, remote_avatar_settings.scale, pose);
        info!("bot {src:?} — spawned");
    }
}

/// Dev-only (`TRICKSHOT_AUTO_FIRE` env): aim the local rig at the nearest bot
/// and pull the trigger every ~1.2 s, so the shoot → score → respawn loop can
/// be exercised without a human clicking.
#[allow(clippy::type_complexity)]
fn dev_auto_fire(
    time: Res<Time>,
    mut cooldown: Local<f32>,
    bots: Query<&Bot>,
    mut pending: ResMut<PendingShot>,
    mut player: Query<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Query<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    *cooldown -= time.delta_secs();
    if *cooldown > 0.0 {
        return;
    }
    let (Ok(mut pt), Ok(mut ht)) = (player.single_mut(), head.single_mut()) else {
        return;
    };
    // Nearest alive bot.
    let Some(bot) = bots
        .iter()
        .filter(|b| b.alive)
        .min_by(|a, b| {
            a.pos
                .distance_squared(pt.translation)
                .total_cmp(&b.pos.distance_squared(pt.translation))
        })
        .copied()
    else {
        return;
    };
    let f = (bot.pos + Vec3::Y * (BOT_H * 0.5) - pt.translation).normalize_or_zero();
    if f == Vec3::ZERO {
        return;
    }
    pt.rotation = Quat::from_rotation_y(f.x.atan2(f.z) + core::f32::consts::PI);
    ht.rotation = Quat::from_rotation_x(f.y.clamp(-1.0, 1.0).asin());
    pending.0 = Some(f); // dev aimbot: straight at the bot, no inaccuracy
    *cooldown = 1.2;
}

/// Copy each bot's interpolated state onto its [`BotPose`] stand-in, with an
/// upward blood burst the moment it dies; drop the stand-in once the bot is
/// gone.
fn sync_bot_poses(
    bots: Query<&Bot>,
    mut stand_ins: Query<(Entity, &mut BotPose, &mut PlayerPose)>,
    mut blood: EventWriter<BloodImpact>,
    mut blood_rec: ResMut<killcam::ReplayBloodImpact>,
    mut commands: Commands,
) {
    for (entity, mut stand_in, mut pose) in &mut stand_ins {
        let Ok(bot) = bots.get(stand_in.bot) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        let want = bot_player_pose(bot);
        if *pose != want {
            *pose = want;
        }
        if !bot.alive && !stand_in.died {
            stand_in.died = true;
            // The server's `ShotResolved` reports a bot kill as a plain
            // miss with no hit point, so squirt an upward blood burst
            // from the bot's chest rather than a directed one.
            let point = bot.pos + Vec3::Y * (BOT_H * 0.55);
            blood.write(BloodImpact { point, dir: Vec3::Y });
            // Stamp it for this client's next input packet so a kill cam
            // built from our stream replays the squirt.
            blood_rec.0 = Some(point);
        }
    }
}

fn on_disconnected(t: Trigger<OnAdd, Disconnected>, q: Query<&Disconnected>) {
    // `NetcodeClient` requires `Disconnected`, so this also fires once at spawn
    // with no reason — that's just "not connected yet", not a real drop.
    match q.get(t.target()).ok().and_then(|d| d.reason.clone()) {
        Some(reason) => warn!("disconnected from server: {reason}"),
        None => debug!("client not connected yet"),
    }
}
