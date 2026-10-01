//! `Zombies` zombie avatars (`models/zombie.glb`): which clip each one plays
//! — from the state the server publishes on its pose
//! ([`shared::PlayerPose::zombie`]) and how fast it's really moving — and
//! how fast, so the feet stay planted whatever its speed. Spawned in place of
//! the soldier by `net::spawn_remote_avatars`; no gun, so no sniper glint,
//! flashlight or bot tint.

use std::time::Duration;

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;
use shared::{PlayerId, PlayerPose, ZombieAnim};

use crate::net::RemoteAvatar;
use crate::player::Player;
use crate::AppState;

pub(crate) const ZOMBIE_MODEL: &str = "models/zombie.glb";

/// Tags a zombie avatar's `SceneRoot` (see [`start_zombie_animation`]).
#[derive(Component)]
pub(crate) struct ZombieVisual;

/// The `AnimationPlayer` inside a zombie avatar's scene, once it's spawned.
#[derive(Component)]
pub(crate) struct ZombieAnimationPlayer(pub(crate) Entity);

/// Which of `models/zombie.glb`'s clips a zombie avatar is playing
/// (`client/notes/zombie.md` — the last three, `Walk` / `Run` / `Walk1`, move
/// the model off its spot, so only the in-place ones are used).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ZombieAnimState {
    /// `Idle`: standing, or climbing out of the ground.
    #[default]
    Idle,
    /// `Walk1_InPlace`: arms down.
    Walk,
    /// `Walk_InPlace`: arms up (a walker near who it's after).
    WalkArmsUp,
    /// `Run_InPlace`.
    Run,
    /// `Attack`: swinging at someone.
    Attack,
    /// `FallingBack` / `FallingForward`: dead, held on the last frame.
    FallBack,
    FallForward,
}

impl ZombieAnimState {
    fn label(self) -> &'static str {
        match self {
            ZombieAnimState::Idle => "idle",
            ZombieAnimState::Walk => "walk (arms down)",
            ZombieAnimState::WalkArmsUp => "walk (arms up)",
            ZombieAnimState::Run => "run",
            ZombieAnimState::Attack => "attack",
            ZombieAnimState::FallBack => "falling back",
            ZombieAnimState::FallForward => "falling forward",
        }
    }

    fn moving(self) -> bool {
        matches!(
            self,
            ZombieAnimState::Walk | ZombieAnimState::WalkArmsUp | ZombieAnimState::Run
        )
    }
}

/// Graph + node for each clip used, built once at startup.
#[derive(Resource, Clone)]
pub(crate) struct ZombieAnimations {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    walk_arms_up: AnimationNodeIndex,
    run: AnimationNodeIndex,
    attack: AnimationNodeIndex,
    fall_back: AnimationNodeIndex,
    fall_forward: AnimationNodeIndex,
}

impl ZombieAnimations {
    fn node_for(&self, state: ZombieAnimState) -> AnimationNodeIndex {
        match state {
            ZombieAnimState::Idle => self.idle,
            ZombieAnimState::Walk => self.walk,
            ZombieAnimState::WalkArmsUp => self.walk_arms_up,
            ZombieAnimState::Run => self.run,
            ZombieAnimState::Attack => self.attack,
            ZombieAnimState::FallBack => self.fall_back,
            ZombieAnimState::FallForward => self.fall_forward,
        }
    }
}

/// Panel-tunable zombie model + animation ("Zombies" debug-panel section).
/// The three moving clips play at `speed (m/s) × *_per_mps`, so a slow
/// zombie's legs slow down with it and the feet stay put on the ground —
/// dial each in until the feet stop sliding.
#[derive(Resource, Clone, Copy)]
pub(crate) struct ZombieAvatarSettings {
    pub(crate) scale: f32,
    /// Extra turn (degrees) so the model faces the way it walks.
    pub(crate) yaw_offset_deg: f32,
    /// `Walk1_InPlace` (arms down) playback speed per m/s of movement.
    pub(crate) walk_per_mps: f32,
    /// `Walk_InPlace` (arms up) playback speed per m/s.
    pub(crate) walk_arms_up_per_mps: f32,
    /// `Run_InPlace` playback speed per m/s.
    pub(crate) run_per_mps: f32,
    /// Flat playback speeds for the rest.
    pub(crate) idle_speed: f32,
    pub(crate) attack_speed: f32,
    pub(crate) death_speed: f32,
    /// Seconds each clip blends into the next.
    pub(crate) blend_secs: f32,
    /// Below this speed (m/s) a zombie that should be walking stands idle
    /// instead of treading in place (stuck on something, say).
    pub(crate) min_move_speed: f32,
}

impl Default for ZombieAvatarSettings {
    fn default() -> Self {
        Self {
            scale: 0.95,
            yaw_offset_deg: 180.0,
            walk_per_mps: 2.5,
            walk_arms_up_per_mps: 2.5,
            run_per_mps: 0.35,
            idle_speed: 1.0,
            // `Attack` is 1 s long, the same as a swing on the server
            // (`shared::zombies::ZOMBIE_ATTACK_SECS`), so at 1× a looping
            // attack lines up with back-to-back swings.
            attack_speed: 1.0,
            death_speed: 1.0,
            blend_secs: 0.25,
            min_move_speed: 0.2,
        }
    }
}

/// The nearest zombie to us, for the debug panel's live readout (to match
/// the feet against).
#[derive(Resource, Default)]
pub(crate) struct ZombieAnimReadout {
    pub(crate) nearest: Option<(f32, f32, &'static str)>,
}

/// A zombie avatar's animation bookkeeping.
#[derive(Component, Default)]
pub(crate) struct ZombieMotion {
    prev_translation: Option<Vec3>,
    /// Horizontal speed (m/s), smoothed — interpolated poses arrive a little
    /// unevenly.
    speed: f32,
    state: ZombieAnimState,
}

impl ZombieMotion {
    /// How fast it's moving right now (m/s, horizontally, smoothed).
    pub(crate) fn speed(&self) -> f32 {
        self.speed
    }

    /// Dead and falling (or fallen): `Some(true)` face-first, `Some(false)`
    /// onto its back. `None` while it's still up.
    pub(crate) fn fell_forward(&self) -> Option<bool> {
        match self.state {
            ZombieAnimState::FallForward => Some(true),
            ZombieAnimState::FallBack => Some(false),
            _ => None,
        }
    }
}

pub(crate) struct ZombieAvatarPlugin;

impl Plugin for ZombieAvatarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZombieAvatarSettings>()
            .init_resource::<ZombieAnimReadout>()
            .add_systems(Startup, setup_zombie_assets)
            .add_systems(
                Update,
                animate_zombie_avatars.run_if(in_state(AppState::InGame)),
            );
    }
}

fn setup_zombie_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // `models/zombie.glb`'s clips: 0 Idle, 1 Walk1_InPlace, 2 Walk_InPlace,
    // 3 Run_InPlace, 4 Attack, 5 FallingBack, 6 FallingForward (7–9 unused).
    let clip = |i: usize| -> Handle<AnimationClip> {
        asset_server.load(GltfAssetLabel::Animation(i).from_asset(ZOMBIE_MODEL))
    };
    let (graph, nodes) = AnimationGraph::from_clips((0..7).map(clip));
    commands.insert_resource(ZombieAnimations {
        graph: graphs.add(graph),
        idle: nodes[0],
        walk: nodes[1],
        walk_arms_up: nodes[2],
        run: nodes[3],
        attack: nodes[4],
        fall_back: nodes[5],
        fall_forward: nodes[6],
    });
}

/// Spawn a zombie avatar following the `PlayerPose` on `src`.
pub(crate) fn spawn_zombie_avatar(
    commands: &mut Commands,
    asset_server: &AssetServer,
    settings: &ZombieAvatarSettings,
    src: Entity,
) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RemoteAvatar { src },
            ZombieMotion::default(),
            ZombieVisual,
            Transform::from_scale(Vec3::splat(settings.scale)),
            Visibility::default(),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(ZOMBIE_MODEL))),
        ))
        .observe(start_zombie_animation);
}

/// Once a zombie's scene is in: loop `Idle` (it's climbing out of the
/// ground), and remember its `AnimationPlayer`.
fn start_zombie_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    anims: Res<ZombieAnimations>,
) {
    let root = trigger.target();
    for entity in children.iter_descendants(root) {
        // A skinned mesh is culled against its rest pose, not the animated one.
        commands.entity(entity).insert(NoFrustumCulling);
        if let Ok(mut player) = players.get_mut(entity) {
            let mut transitions = AnimationTransitions::new();
            transitions
                .play(&mut player, anims.idle, Duration::ZERO)
                .set_repeat(RepeatAnimation::Forever);
            commands
                .entity(entity)
                .insert((AnimationGraphHandle(anims.graph.clone()), transitions));
            commands.entity(root).insert(ZombieAnimationPlayer(entity));
        }
    }
}

/// Pick each zombie's clip from what the server says it's doing, blend into
/// it, and keep the moving clips' speed in step with how fast it's really
/// going. Dead: a random fall (the same one on every client), held.
#[allow(clippy::type_complexity)]
fn animate_zombie_avatars(
    time: Res<Time>,
    anims: Option<Res<ZombieAnimations>>,
    settings: Res<ZombieAvatarSettings>,
    poses: Query<(&PlayerPose, Option<&PlayerId>)>,
    me: Query<&Transform, With<Player>>,
    mut avatars: Query<(&RemoteAvatar, &mut ZombieMotion, &ZombieAnimationPlayer)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut readout: ResMut<ZombieAnimReadout>,
) {
    let Some(anims) = anims else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let blend = Duration::from_secs_f32(settings.blend_secs.max(0.0));
    let me = me.single().ok().map(|t| t.translation);
    let mut nearest: Option<(f32, f32, f32, &'static str)> = None;

    for (avatar, mut motion, anim_player) in &mut avatars {
        let Ok((pose, id)) = poses.get(avatar.src) else {
            continue;
        };

        // Horizontal only — a zombie climbing out of the ground isn't walking.
        let raw = match motion.prev_translation {
            Some(prev) => {
                let d = pose.translation - prev;
                Vec2::new(d.x, d.z).length() / dt
            }
            None => 0.0,
        };
        motion.prev_translation = Some(pose.translation);
        let k = 1.0 - (-dt / 0.12).exp();
        motion.speed += (raw - motion.speed) * k;

        let state = if !pose.alive {
            if matches!(motion.state, ZombieAnimState::FallBack | ZombieAnimState::FallForward) {
                motion.state
            } else {
                // The same pick on every client: off its (shared) peer id.
                let seed = id.map_or(avatar.src.to_bits(), |id| id.0.to_bits());
                if crate::util::rand01((seed ^ (seed >> 32)) as u32) < 0.5 {
                    ZombieAnimState::FallBack
                } else {
                    ZombieAnimState::FallForward
                }
            }
        } else {
            let wanted = match pose.zombie {
                ZombieAnim::Walk => ZombieAnimState::Walk,
                ZombieAnim::WalkArmsUp => ZombieAnimState::WalkArmsUp,
                ZombieAnim::Run => ZombieAnimState::Run,
                ZombieAnim::Attack => ZombieAnimState::Attack,
                ZombieAnim::Idle | ZombieAnim::Rising | ZombieAnim::None => ZombieAnimState::Idle,
            };
            // Only tread along while actually getting somewhere (a little
            // hysteresis so it doesn't flicker at the threshold).
            let threshold = if motion.state.moving() {
                settings.min_move_speed * 0.5
            } else {
                settings.min_move_speed
            };
            if wanted.moving() && motion.speed < threshold {
                ZombieAnimState::Idle
            } else {
                wanted
            }
        };

        let Ok((mut player, mut transitions)) = players.get_mut(anim_player.0) else {
            continue;
        };
        if state != motion.state {
            motion.state = state;
            let repeat = match state {
                ZombieAnimState::FallBack | ZombieAnimState::FallForward => RepeatAnimation::Never,
                _ => RepeatAnimation::Forever,
            };
            transitions
                .play(&mut player, anims.node_for(state), blend)
                .set_repeat(repeat);
        }
        // Every frame, so the panel's sliders and speed changes apply live.
        let speed = motion.speed;
        let rate = match motion.state {
            ZombieAnimState::Idle => settings.idle_speed,
            ZombieAnimState::Walk => speed * settings.walk_per_mps,
            ZombieAnimState::WalkArmsUp => speed * settings.walk_arms_up_per_mps,
            ZombieAnimState::Run => speed * settings.run_per_mps,
            ZombieAnimState::Attack => settings.attack_speed,
            ZombieAnimState::FallBack | ZombieAnimState::FallForward => settings.death_speed,
        };
        if let Some(active) = player.animation_mut(anims.node_for(motion.state)) {
            active.set_speed(rate.max(0.01));
        }

        if let Some(me) = me {
            let d = me.distance(pose.translation);
            if nearest.is_none_or(|(n, ..)| d < n) {
                nearest = Some((d, speed, rate, motion.state.label()));
            }
        }
    }
    readout.nearest = nearest.map(|(_, speed, rate, label)| (speed, rate, label));
}
