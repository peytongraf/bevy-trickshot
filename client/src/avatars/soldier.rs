//! Remote-player ("soldier") avatar animation: the movement/aim clip graph
//! and the state each avatar's interpolated pose picks a clip from.

use std::time::Duration;

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;

/// Tags a remote player's `SceneRoot` entity (`models/soldier.glb`) so
/// `start_soldier_animation` can tell it apart from any other spawned scene.
#[derive(Component)]
pub(crate) struct SoldierVisual;

/// Which of `models/soldier.glb`'s movement/aim clips a remote avatar should
/// be playing, picked from its interpolated pose's frame-to-frame speed and
/// `ads_t` by `net::animate_remote_avatars`. `AimWalk`/`AimSprint` both play
/// `runAndShooting` — there's no separate walking-while-aiming clip — split
/// out only so each can get its own playback speed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum SoldierAnimState {
    #[default]
    Idle,
    Walk,
    Sprint,
    /// Aiming, not moving: `shooting`, looped.
    Aim,
    /// Aiming while walking: `runAndShooting`, looped at the walk speed.
    AimWalk,
    /// Aiming while sprinting: `runAndShooting`, looped at the sprint speed.
    AimSprint,
    /// Crouched, not moving: `crouch`, looped.
    Crouch,
    /// Crouch-walking: `crouchWalk`, looped.
    CrouchWalk,
    /// Reloading: `reload`, played once — takes priority over every other
    /// state above, and drops back to whichever of those applies once
    /// `pose.reloading` clears.
    Reload,
    /// Airborne from a jump: `jump`, played once at launch and held on its
    /// last frame for the rest of the jump arc (`pose.jumping` stays true
    /// until landing) — takes priority over everything, including `Reload`.
    Jump,
    /// Strafing right with no forward/back component: `strafeRight`, looped.
    StrafeRight,
    /// Strafing left with no forward/back component: `strafeLeft`, looped.
    StrafeLeft,
    /// Moving backward with no strafe component: `backpaddle`, looped.
    Backward,
    /// Dead (`pose.alive == false`): `death`, played once and held on its
    /// last frame (the model falling onto its back) until `pose.alive` goes
    /// back to `true` on respawn — outranks every other state, including
    /// `Jump`.
    Dead,
}

/// Graph + node indices for `models/soldier.glb`'s `idleWgun` / `walk` /
/// `run` / `shooting` / `runAndShooting` / `crouch` / `crouchWalk` / `reload`
/// / `jump` / `strafeRight` / `strafeLeft` / `backpaddle` / `death` clips —
/// 13 of its 18 animations wired up so far. Built once at startup.
#[derive(Resource, Clone)]
pub(crate) struct SoldierAnimations {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    sprint: AnimationNodeIndex,
    aim_idle: AnimationNodeIndex,
    aim_move: AnimationNodeIndex,
    crouch: AnimationNodeIndex,
    crouch_walk: AnimationNodeIndex,
    reload: AnimationNodeIndex,
    jump: AnimationNodeIndex,
    strafe_right: AnimationNodeIndex,
    strafe_left: AnimationNodeIndex,
    backward: AnimationNodeIndex,
    death: AnimationNodeIndex,
}

impl SoldierAnimations {
    pub(crate) fn node_for(&self, state: SoldierAnimState) -> AnimationNodeIndex {
        match state {
            SoldierAnimState::Idle => self.idle,
            SoldierAnimState::Walk => self.walk,
            SoldierAnimState::Sprint => self.sprint,
            SoldierAnimState::Aim => self.aim_idle,
            SoldierAnimState::AimWalk | SoldierAnimState::AimSprint => self.aim_move,
            SoldierAnimState::Crouch => self.crouch,
            SoldierAnimState::CrouchWalk => self.crouch_walk,
            SoldierAnimState::Reload => self.reload,
            SoldierAnimState::Jump => self.jump,
            SoldierAnimState::StrafeRight => self.strafe_right,
            SoldierAnimState::StrafeLeft => self.strafe_left,
            SoldierAnimState::Backward => self.backward,
            SoldierAnimState::Dead => self.death,
        }
    }
}

/// Playback-speed multipliers for the remote-player walk/sprint/aim clips,
/// calibrated for a remote player moving at exactly the default
/// `WALK_SPEED`/`SPRINT_SPEED` (hand-tuned so the feet don't slide). Exposed
/// on the "Remote players" debug-panel section for re-tuning if the clips
/// themselves change; `net::animate_remote_avatars` scales these by how far
/// `MovementSettings.walk_speed`/`sprint_speed` (the "Movement" panel's
/// player-speed controls) have been moved away from those defaults, so
/// dialing player speed up or down keeps the feet matching the ground
/// without needing separate, redundant animation-speed sliders.
///
/// `runAndShooting` has no walking-only counterpart, so `base_aim_walk_speed`
/// and `base_aim_sprint_speed` both drive that same clip — split into two
/// settings only because it needs a different speed depending on whether the
/// remote player is walking or sprinting while aiming.
///
/// `base_crouch_walk_speed` is calibrated against `SlideSettings.crouch_speed`
/// (the "Slide" panel's crouch-move-speed control) the same way the others
/// are calibrated against `WALK_SPEED`/`SPRINT_SPEED`.
///
/// `base_strafe_speed`/`base_backpaddle_speed` are calibrated against the
/// *effective* strafe/backward speed at the default settings — i.e.
/// `WALK_SPEED * STRAFE_SPEED_MULT` / `WALK_SPEED * BACKWARD_SPEED_MULT` —
/// since `strafeRight`/`strafeLeft`/`backpaddle` only ever play at walk pace
/// (no separate sprint-strafe clip exists), scaled by how far
/// `MovementSettings.strafe_speed_mult`/`backward_speed_mult` (the
/// "Movement" panel's controls) have moved away from their defaults.
///
/// `death_speed` isn't calibrated against anything — it's a flat playback
/// speed multiplier for the `death` clip (`1.0` = authored speed), since
/// there's no real-world reference like a movement speed to match it to.
#[derive(Resource, Clone, Copy)]
pub(crate) struct SoldierAnimSettings {
    pub(crate) base_walk_speed: f32,
    pub(crate) base_sprint_speed: f32,
    pub(crate) base_aim_walk_speed: f32,
    pub(crate) base_aim_sprint_speed: f32,
    pub(crate) base_crouch_walk_speed: f32,
    pub(crate) base_strafe_speed: f32,
    pub(crate) base_backpaddle_speed: f32,
    pub(crate) death_speed: f32,
}

impl Default for SoldierAnimSettings {
    fn default() -> Self {
        Self {
            base_walk_speed: 2.2,
            base_sprint_speed: 1.4,
            base_aim_walk_speed: 0.8,
            base_aim_sprint_speed: 1.5,
            base_crouch_walk_speed: 1.4,
            base_strafe_speed: 2.0,
            base_backpaddle_speed: 1.5,
            death_speed: 1.0,
        }
    }
}

/// Set by `start_soldier_animation` once it finds the `AnimationPlayer` inside
/// a remote-player avatar's spawned scene. Mirrors [`BotAnimationPlayer`],
/// kept as its own type since it points into a different graph. Read by
/// `net::animate_remote_avatars` to switch between idle/walk/sprint, the same
/// way `play_bot_death` uses `BotAnimationPlayer`.
#[derive(Component)]
pub(crate) struct SoldierAnimationPlayer(pub(crate) Entity);

/// Panel-adjustable uniform scale for the `models/soldier.glb` remote-player
/// avatar ("Remote players" debug-panel section), applied by
/// `net::follow_remote_avatars`. Live-tweakable rather than a baked constant
/// like `BOT_MODEL_SCALE` — the model's authored size relative to a real
/// player isn't known up front.
#[derive(Resource)]
pub(crate) struct RemoteAvatarSettings {
    pub(crate) scale: f32,
}

impl Default for RemoteAvatarSettings {
    fn default() -> Self {
        Self { scale: 21.0 }
    }
}

/// A remote player's (or bot's) sniper-glint sprite — `textures/sniper_glint.png`
/// on a quad, catching the eye off their scope while they're aiming down
/// sight, giving away a camping sniper the same way Call of Duty's own scope
/// glint does. Not parented to the `RemoteAvatar` model: its offset is meant
/// in real-world metres, and `RemoteAvatarSettings::scale` (~21×) would scale
/// a child's local translation right along with it. One spawned per
/// aiming-capable remote avatar by `net::spawn_sniper_glints`; followed,
/// billboarded to face the local camera, faded in/out with
/// `PlayerPose::ads_t` and despawned with its owner by
/// `net::update_sniper_glints`.
#[derive(Component)]
pub(crate) struct SniperGlint {
    /// The `PlayerPose` entity this glint follows — same as `RemoteAvatar::src`.
    pub(crate) src: Entity,
    /// The `RemoteAvatar` entity itself — looked up each frame for its
    /// [`SniperGlintBone`], if the scene has found one yet.
    pub(crate) avatar: Entity,
}

/// glTF node name of the joint that actually drives the gun mesh's on-screen
/// position in `models/soldier.glb`: **not** the gun mesh's own node
/// (Blender's `Object_10` / glTF node `gun_LOD0_055_056` — both dead weight,
/// with no animation channel anywhere in either one's ancestry, since the
/// mesh is *skinned* rather than rigidly parented — a skinned mesh's own
/// `Transform` never moves; the vertices are deformed by joint matrices in
/// the shader instead). Inspecting `Object_10`'s per-vertex skin weights
/// directly (every one of its 999 vertices) showed all of them at 100%
/// weight on this single joint — the right-hand bone — so it, not the gun
/// mesh's own node, is what actually carries the walk sway / recoil kick /
/// reload motion the gun visibly follows.
pub(crate) const SNIPER_GLINT_BONE_NAME: &str = "Bind_RightHand_037_038";

/// The animated joint the gun mesh is actually 100%-weighted to (see
/// [`SNIPER_GLINT_BONE_NAME`]'s doc comment) inside a `RemoteAvatar`'s
/// spawned scene — set by `start_soldier_animation` once it's found among
/// the scene's descendants. `SniperGlint` reads this entity's
/// `GlobalTransform` every frame instead of the replicated (unanimated)
/// `PlayerPose`, so it rides along with the actual animated gun mesh — walk
/// bob, aim raise, recoil — rather than floating in place while the avatar's
/// model moves under it during a movement / aim animation.
#[derive(Component)]
pub(crate) struct SniperGlintBone(pub(crate) Entity);

/// Shared quad mesh + the glint texture (`net::spawn_sniper_glints` clones a
/// fresh material per glint off `texture` so each one's alpha can fade on its
/// own — mirrors `vfx::impacts::ImpactAssets`). Built once by
/// `setup_sniper_glint_assets`, added to `PostStartup` (never `Startup`,
/// which blacks out the in-game 3D view).
#[derive(Resource)]
pub(crate) struct SniperGlintAssets {
    pub(crate) quad: Handle<Mesh>,
    pub(crate) texture: Handle<Image>,
}

/// Build [`SniperGlintAssets`].
pub(crate) fn setup_sniper_glint_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    commands.insert_resource(SniperGlintAssets {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        texture: asset_server.load("textures/sniper_glint.png"),
    });
}

/// Panel-adjustable sniper-glint placement/size ("Sniper glint" debug-panel
/// section), applied every frame by `net::update_sniper_glints`.
#[derive(Resource)]
pub(crate) struct SniperGlintSettings {
    /// Offset (world metres) from [`SniperGlintBone`]'s current animated
    /// position, in *its own* local orientation: `x` right(+)/left(-), `y`
    /// up(+)/down(-), `z` forward(+)/back(-) — whichever way those map onto
    /// the right-hand bone's own rest-pose axes in `models/soldier.glb` (the
    /// gun mesh is rigidly skinned to it — see [`SNIPER_GLINT_BONE_NAME`]).
    /// Falls back to the replicated pose (its `yaw` for `x`/`z`, world up for
    /// `y`) for the one frame or so before the scene's found the bone. Dial
    /// in against the raised-rifle aim pose from the debug panel — the
    /// sprite should sit still on the scope through every animation once
    /// this is right.
    pub(crate) offset: Vec3,
    /// Uniform size of the sprite quad (world metres).
    pub(crate) scale: f32,
    /// `PlayerPose::ads_t` the glint starts fading in at, reaching full
    /// opacity by `1.0` — defaults to the same cutoff
    /// `net::animate_remote_avatars` uses to switch into the aim pose, so it
    /// appears right as the avatar visibly raises its rifle.
    pub(crate) ads_threshold: f32,
}

impl Default for SniperGlintSettings {
    fn default() -> Self {
        // Dialed in by the user against the real `Bind_RightHand_037_038`
        // bone (see `SNIPER_GLINT_BONE_NAME`) in the debug panel — sits right
        // on the scope through the aim / shooting animations.
        Self {
            offset: Vec3::new(0.28, 0.4, 0.0),
            scale: 0.65,
            ads_threshold: 0.5,
        }
    }
}

/// Build the remote-player animation graph. Added to the same `Startup` tuple
/// as `setup_bot_assets` — see its comment for why a separate
/// `add_systems(Startup, ...)` (even from a plugin) can't be used instead.
pub(crate) fn setup_soldier_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Indices into `models/soldier.glb`'s 18 animations: 0 idleWgun, 1 walk,
    // 3 run, 4 shooting, 6 runAndShooting, 7 strafeRight, 8 strafeLeft,
    // 9 backpaddle, 10 jump, 11 crouch, 12 crouchWalk, 13 reload, 15 death.
    let idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("models/soldier.glb"));
    let walk_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(1).from_asset("models/soldier.glb"));
    let sprint_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(3).from_asset("models/soldier.glb"));
    let aim_idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(4).from_asset("models/soldier.glb"));
    let aim_move_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(6).from_asset("models/soldier.glb"));
    let crouch_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(11).from_asset("models/soldier.glb"));
    let crouch_walk_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(12).from_asset("models/soldier.glb"));
    let reload_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(13).from_asset("models/soldier.glb"));
    let jump_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(10).from_asset("models/soldier.glb"));
    let strafe_right_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(7).from_asset("models/soldier.glb"));
    let strafe_left_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(8).from_asset("models/soldier.glb"));
    let backward_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(9).from_asset("models/soldier.glb"));
    let death_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(15).from_asset("models/soldier.glb"));
    let (graph, indices) = AnimationGraph::from_clips([
        idle_clip,
        walk_clip,
        sprint_clip,
        aim_idle_clip,
        aim_move_clip,
        crouch_clip,
        crouch_walk_clip,
        reload_clip,
        jump_clip,
        strafe_right_clip,
        strafe_left_clip,
        backward_clip,
        death_clip,
    ]);
    let graph = graphs.add(graph);
    commands.insert_resource(SoldierAnimations {
        graph,
        idle: indices[0],
        walk: indices[1],
        sprint: indices[2],
        aim_idle: indices[3],
        aim_move: indices[4],
        crouch: indices[5],
        crouch_walk: indices[6],
        reload: indices[7],
        jump: indices[8],
        strafe_right: indices[9],
        strafe_left: indices[10],
        backward: indices[11],
        death: indices[12],
    });
}

/// Fires once a remote-player avatar's `SceneRoot` (tagged [`SoldierVisual`])
/// finishes spawning: starts the idle animation looping, remembers which
/// descendant holds the `AnimationPlayer` via [`SoldierAnimationPlayer`], and
/// remembers the right-hand bone the gun mesh is skinned to
/// ([`SNIPER_GLINT_BONE_NAME`]) via [`SniperGlintBone`] so the sniper glint
/// can ride along with it. Mirrors `start_bot_animation`.
pub(crate) fn start_soldier_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    soldiers: Query<(), With<SoldierVisual>>,
    mut players: Query<&mut AnimationPlayer>,
    names: Query<&Name>,
    anims: Res<SoldierAnimations>,
) {
    let root = trigger.target();
    if !soldiers.contains(root) {
        return;
    }
    for entity in children.iter_descendants(root) {
        // Same fix as the bot / sniper view model: a skinned mesh is
        // frustum-culled against its *rest-pose* AABB, not the animated one.
        commands.entity(entity).insert(NoFrustumCulling);

        if let Ok(mut player) = players.get_mut(entity) {
            let mut transitions = AnimationTransitions::new();
            transitions
                .play(&mut player, anims.idle, Duration::ZERO)
                .set_repeat(RepeatAnimation::Forever);
            commands
                .entity(entity)
                .insert((AnimationGraphHandle(anims.graph.clone()), transitions));
            commands.entity(root).insert(SoldierAnimationPlayer(entity));
        }

        if names.get(entity).map(Name::as_str) == Ok(SNIPER_GLINT_BONE_NAME) {
            commands.entity(root).insert(SniperGlintBone(entity));
        }
    }
}
