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
use bevy::prelude::*;
use lightyear::prelude::client::{ClientPlugins, NetcodeClient, NetcodeConfig};
use lightyear::prelude::input::client::InputSet;
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::*;

use shared::{
    Bot, KillCam, MatchOver, PlayerId, PlayerInput, PlayerPose, ShotOutcome, ShotResolved,
    TrickScore,
};

use crate::killcam::{self, ActiveKillCam, ReplaySoundBits};
use crate::{
    play_bot_death, Ads, AppState, BloodImpact, BotAnimationPlayer, BotAnimations, BotVisual,
    GroundImpact, MatchEndedEvent, PendingShot, Player, PlayerHead, PlayerPhysics, TrickScoredEvent,
    TrickState,
    WorldModelCamera, NOSCOPE_ADS_MAX,
};

/// Where a shipped build connects when `TRICKSHOT_SERVER` is unset and we're not
/// running under `cargo`. Mirrors `updater.rs`'s `DEFAULT_REPO` convention.
const DEFAULT_SERVER: &str = "bevy-trickshot-server.fly.dev:5000";

/// Resolve the server address:
/// * `TRICKSHOT_SERVER` env var wins (host:port, resolved via DNS),
/// * else `127.0.0.1:5000` under `cargo run` (dev),
/// * else [`DEFAULT_SERVER`] (shipped build).
pub fn server_addr() -> SocketAddr {
    let fallback = || SocketAddr::from(([127, 0, 0, 1], shared::DEFAULT_PORT));
    let resolve = |s: &str| s.to_socket_addrs().ok().and_then(|mut it| it.next());

    if let Ok(s) = std::env::var("TRICKSHOT_SERVER") {
        if let Some(addr) = resolve(&s) {
            return addr;
        }
        warn!("TRICKSHOT_SERVER='{s}' did not resolve; using the default instead");
    }

    let default = if std::env::var_os("CARGO").is_some() {
        format!("127.0.0.1:{}", shared::DEFAULT_PORT)
    } else {
        DEFAULT_SERVER.to_string()
    };
    resolve(&default).unwrap_or_else(fallback)
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

        app.add_systems(Startup, connect);
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);

        app.add_systems(
            FixedPreUpdate,
            write_input
                .in_set(InputSet::WriteClientInputs)
                .run_if(in_state(AppState::InGame)),
        );
        app.add_systems(
            Update,
            (
                mark_local_input,
                spawn_remote_avatars,
                follow_remote_avatars,
                animate_remote_avatars,
                spawn_bot_avatars,
                follow_bot_avatars,
                receive_shots,
                receive_trick_scores,
                receive_match_over,
                receive_killcam,
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
    let addr = server_addr();
    let auth = Authentication::Manual {
        server_addr: addr,
        client_id: dev_client_id(),
        private_key: shared::DEV_PRIVATE_KEY,
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
    (view_model_vis, knife, weapon, jumping): (
        Query<&Visibility, With<crate::ViewModel>>,
        Res<crate::ThrowingKnife>,
        Res<crate::Weapon>,
        Res<crate::Jumping>,
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
    // Kill-cam recording: the camera-shake state, weapon-sway offset and hip
    // FOV, so a replay can reproduce shake / recoil / sway and render at this
    // player's FOV instead of re-deriving an absolute camera transform that
    // would just duplicate `translation` / `yaw` / `pitch` above.
    action.shake_trauma = shake.trauma;
    action.shake_phase = shake.phase;
    action.shake_recoil = shake.recoil;
    action.sway_offset = sway.offset.to_array();
    action.fov_deg = settings.fov;
    action.sound_bits = std::mem::take(&mut snd.0);
    action.anim_time = crate::killcam::viewmodel_anim_time(&anim_players, &view_models);
    action.ads_t = ads.t;
    action.crouching = slide.stance == crate::Stance::Crouching;
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

/// Server → everyone in the lobby: the match clock ran out.
fn receive_match_over(
    mut receivers: Query<&mut MessageReceiver<MatchOver>>,
    mut ended: EventWriter<MatchEndedEvent>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            ended.write(MatchEndedEvent {
                winner: msg.winner_name,
                score: msg.winner_score,
            });
        }
    }
}

/// Server → everyone in the lobby: the killer's last ~3 s to replay.
fn receive_killcam(
    mut receivers: Query<&mut MessageReceiver<KillCam>>,
    mut active: ResMut<ActiveKillCam>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
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
    mut tracers: EventWriter<crate::FireTracer>,
) {
    let me = local.iter().next().map(|l| l.0);
    let killcam_playing = active.0.is_some();
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if killcam_playing || Some(msg.shooter) == me {
                continue;
            }
            if let ShotOutcome::Ground { point } = msg.outcome {
                impacts.write(GroundImpact(Vec3::from_array(point)));
            }
            tracers.write(crate::FireTracer {
                start: Vec3::from_array(msg.origin),
                end: Vec3::from_array(msg.tracer_end),
            });
        }
    }
}

// --- remote players (models/soldier.glb, idleWgun looping) ------------

/// A `models/soldier.glb` avatar standing in for another player; follows their
/// interpolated pose.
#[derive(Component)]
struct RemoteAvatar {
    src: Entity,
}

/// Tracks a remote avatar's previous position and currently-playing animation
/// so `animate_remote_avatars` can tell idle/walk/sprint apart from
/// frame-to-frame movement speed and only call `AnimationPlayer::play` on a
/// change (it starts on `idleWgun`, matching `start_soldier_animation`).
#[derive(Component, Default)]
struct RemoteAvatarMotion {
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
        commands
            .spawn((
                StateScoped(AppState::InGame),
                RemoteAvatar { src },
                RemoteAvatarMotion::default(),
                crate::SoldierVisual,
                Transform::from_scale(Vec3::splat(remote_avatar_settings.scale)),
                Visibility::default(),
                SceneRoot(
                    asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/soldier.glb")),
                ),
            ))
            .observe(crate::start_soldier_animation);
        info!("remote player {src:?} — spawned soldier avatar");
    }
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

/// Switches each remote avatar between `idleWgun`/`walk`/`run`/`shooting`/
/// `runAndShooting`/`crouch`/`crouchWalk` based on how fast its interpolated
/// pose is actually moving frame-to-frame and whether it's aiming
/// (`pose.ads_t`) or crouching (`pose.crouching`), and keeps each clip's
/// playback speed tied to the current `MovementSettings`/`SlideSettings`
/// speeds (the "Movement"/"Slide" debug-panel sliders) so the feet still
/// match the ground after those are changed, without a separate "animation
/// speed" control to keep in sync by hand.
#[allow(clippy::too_many_arguments)]
fn animate_remote_avatars(
    poses: Query<&PlayerPose>,
    time: Res<Time>,
    anims: Res<crate::SoldierAnimations>,
    anim_settings: Res<crate::SoldierAnimSettings>,
    movement: Res<crate::MovementSettings>,
    slide_cfg: Res<crate::SlideSettings>,
    mut avatars: Query<(&RemoteAvatar, &mut RemoteAvatarMotion, &crate::SoldierAnimationPlayer)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
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

    for (avatar, mut motion, anim_player) in &mut avatars {
        let Ok(pose) = poses.get(avatar.src) else {
            continue;
        };

        let speed = match motion.prev_translation {
            Some(prev) => (pose.translation.xz() - prev.xz()).length() / dt,
            None => 0.0,
        };
        motion.prev_translation = Some(pose.translation);

        // Jumping/reloading take priority over everything else. Both are
        // sustained flags (true for the whole jump arc / whole reload), but
        // the `new_state != motion.state` guard below only calls `play()` on
        // the entry edge, so each still plays its clip once — holding the
        // last frame — for as long as the flag stays true. Once it clears,
        // the state below is picked again on the very next frame. `Jump`
        // outranks even `Reload` since landing mid-reload should still show
        // the jump/land motion.
        let new_state = if pose.jumping {
            crate::SoldierAnimState::Jump
        } else if pose.reloading {
            crate::SoldierAnimState::Reload
        } else if pose.crouching {
            // Crouching has no aiming variant of its own (`crouch`/
            // `crouchWalk` only), so it takes priority over the aim states.
            if speed >= crouch_walk_enter {
                crate::SoldierAnimState::CrouchWalk
            } else {
                crate::SoldierAnimState::Crouch
            }
        } else {
            let move_state = if speed >= sprint_enter {
                crate::SoldierAnimState::Sprint
            } else if speed >= walk_enter {
                crate::SoldierAnimState::Walk
            } else {
                crate::SoldierAnimState::Idle
            };
            let aiming = pose.ads_t >= AIM_THRESHOLD;
            match (aiming, move_state) {
                (true, crate::SoldierAnimState::Idle) => crate::SoldierAnimState::Aim,
                (true, crate::SoldierAnimState::Walk) => crate::SoldierAnimState::AimWalk,
                (true, crate::SoldierAnimState::Sprint) => crate::SoldierAnimState::AimSprint,
                (false, state) => state,
                // `move_state` is only ever Idle/Walk/Sprint.
                (true, _) => unreachable!(),
            }
        };

        let Ok((mut player, mut transitions)) = players.get_mut(anim_player.0) else {
            continue;
        };

        if new_state != motion.state {
            motion.state = new_state;
            let repeat = match new_state {
                crate::SoldierAnimState::Reload | crate::SoldierAnimState::Jump => {
                    RepeatAnimation::Never
                }
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
            });
        }
    }
}

// --- bots (models that play a death animation when shot) --------------

/// Stands in for a server-owned [`shared::Bot`], as `models/bot.glb`.
#[derive(Component)]
struct BotAvatar {
    src: Entity,
    /// Set once the death animation has been kicked off, so it isn't
    /// restarted every replicated update after the bot dies.
    died: bool,
}

const BOT_H: f32 = 1.8;

fn spawn_bot_avatars(
    bots: Query<Entity, (With<Bot>, With<Interpolated>)>,
    avatars: Query<&BotAvatar>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    let have: std::collections::HashSet<Entity> = avatars.iter().map(|a| a.src).collect();
    for src in &bots {
        if have.contains(&src) {
            continue;
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                BotAvatar { src, died: false },
                BotVisual,
                crate::TargetBotVisual,
                Transform::from_scale(Vec3::splat(crate::BOT_MODEL_SCALE)),
                Visibility::default(),
                SceneRoot(
                    asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/bot.glb")),
                ),
            ))
            .observe(crate::start_bot_animation);
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

#[allow(clippy::too_many_arguments)]
fn follow_bot_avatars(
    bots: Query<&Bot>,
    mut avatars: Query<(Entity, &mut BotAvatar, &mut Transform)>,
    roots: Query<&BotAnimationPlayer>,
    mut players: Query<&mut AnimationPlayer>,
    anims: Res<BotAnimations>,
    mut blood: EventWriter<BloodImpact>,
    mut blood_rec: ResMut<killcam::ReplayBloodImpact>,
    mut commands: Commands,
) {
    for (entity, mut avatar, mut tf) in &mut avatars {
        match bots.get(avatar.src) {
            Ok(bot) => {
                tf.translation = bot.pos;
                tf.rotation = Quat::from_rotation_y(bot.yaw);
                if !bot.alive && !avatar.died {
                    avatar.died = true;
                    play_bot_death(entity, &roots, &mut players, &anims);
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
            Err(_) => {
                commands.entity(entity).try_despawn();
            }
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
