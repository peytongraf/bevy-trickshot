//! Remote-player ("soldier") avatar animation: the movement/aim clip graph
//! and the state each avatar's interpolated pose picks a clip from.

use std::time::Duration;

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::AnimationTarget;
use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;

use super::held_weapon::{
    Grip, HeldWeapon, SoldierRig, LEFT_ARM_BONES, LEFT_SHOULDER_BONE, RIGHT_ARM_BONES, RIGHT_THIGH_BONE, SPINE_BONES,
};

/// Tags a remote player's `SceneRoot` entity (`models/characters/soldier.glb`) so
/// `start_soldier_animation` can tell it apart from any other spawned scene.
#[derive(Component)]
pub(crate) struct SoldierVisual;

/// Which of `models/characters/soldier.glb`'s movement/aim clips a remote avatar should
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
    /// Aiming, not moving: `shooting` — held on its last frame (the gun up,
    /// still), and played once through for each shot their player fires
    /// ([`SHOT_CLIP_SECS`]).
    Aim,
    /// Aiming while walking: `runAndShooting`'s legs, looped at the walk
    /// speed, under the standing aim's upper body — held, and kicked with
    /// each shot, as `Aim` is ([`SoldierAnimations::shot_upper`]).
    AimWalk,
    /// Aiming while sprinting: as `AimWalk`, at the sprint speed.
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
    /// Mid knife stab (`pose.stabbing`): `melee`, played once — outranks
    /// everything but `Dead`.
    Melee,
    /// Dead (`pose.alive == false`): `death`, played once and held on its
    /// last frame (the model falling onto its back) until `pose.alive` goes
    /// back to `true` on respawn — outranks every other state, including
    /// `Jump`.
    Dead,
    /// Lying down (`pose.prone`), still: `prone`, looped — propped on the
    /// elbows, rifle forward (one of the clips `client/tools/
    /// soldier_prone_clips.py` adds to `soldier.glb`).
    Prone,
    /// Lying down, crawling forward: `proneCrawl`, looped at the speed
    /// they're really going...
    ProneCrawl,
    /// ...or back: `proneCrawlBack`.
    ProneCrawlBack,
    /// Standing with no gun: `idleWithoutGun`, looped. Never picked in a
    /// game — it's where the WEAPON EDITOR takes the left arm's "down by the
    /// side" pose from (`held_weapon::ArmPose`).
    IdleNoGun,
}

/// Graph + node indices for `models/characters/soldier.glb`'s `idleWgun` / `walk` /
/// `run` / `shooting` / `runAndShooting` / `crouch` / `crouchWalk` / `reload`
/// / `jump` / `strafeRight` / `strafeLeft` / `backpaddle` / `melee` / `death`
/// / `idleWithoutGun` / `prone` / `proneCrawl` / `proneCrawlBack` clips. Built
/// once at startup.
#[derive(Resource, Clone)]
pub(crate) struct SoldierAnimations {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    sprint: AnimationNodeIndex,
    aim_idle: AnimationNodeIndex,
    /// `runAndShooting`, the legs only (its own upper body fires a shot
    /// every step)...
    aim_move_legs: AnimationNodeIndex,
    /// ...under `shooting`, the upper body only.
    shot_upper: AnimationNodeIndex,
    /// `throw`, the upper body only — over whatever the legs are doing
    /// ([`drive_throw_layer`])...
    throw_upper: AnimationNodeIndex,
    /// ...and `drink` ([`drive_drink_layer`]).
    drink_upper: AnimationNodeIndex,
    crouch: AnimationNodeIndex,
    crouch_walk: AnimationNodeIndex,
    reload: AnimationNodeIndex,
    jump: AnimationNodeIndex,
    strafe_right: AnimationNodeIndex,
    strafe_left: AnimationNodeIndex,
    backward: AnimationNodeIndex,
    melee: AnimationNodeIndex,
    death: AnimationNodeIndex,
    idle_no_gun: AnimationNodeIndex,
    prone: AnimationNodeIndex,
    prone_crawl: AnimationNodeIndex,
    prone_crawl_back: AnimationNodeIndex,
}

impl SoldierAnimations {
    pub(crate) fn graph(&self) -> &Handle<AnimationGraph> {
        &self.graph
    }

    /// `shooting` on the upper body alone, over the aiming-while-moving
    /// legs (`AimWalk` / `AimSprint`) — played alongside them, its own
    /// weight eased in and out (`net::animate_remote_avatars`).
    pub(crate) fn shot_upper(&self) -> AnimationNodeIndex {
        self.shot_upper
    }

    pub(crate) fn node_for(&self, state: SoldierAnimState) -> AnimationNodeIndex {
        match state {
            SoldierAnimState::Idle => self.idle,
            SoldierAnimState::Walk => self.walk,
            SoldierAnimState::Sprint => self.sprint,
            SoldierAnimState::Aim => self.aim_idle,
            SoldierAnimState::AimWalk | SoldierAnimState::AimSprint => self.aim_move_legs,
            SoldierAnimState::Crouch => self.crouch,
            SoldierAnimState::CrouchWalk => self.crouch_walk,
            SoldierAnimState::Reload => self.reload,
            SoldierAnimState::Jump => self.jump,
            SoldierAnimState::StrafeRight => self.strafe_right,
            SoldierAnimState::StrafeLeft => self.strafe_left,
            SoldierAnimState::Backward => self.backward,
            SoldierAnimState::Melee => self.melee,
            SoldierAnimState::Dead => self.death,
            SoldierAnimState::IdleNoGun => self.idle_no_gun,
            SoldierAnimState::Prone => self.prone,
            SoldierAnimState::ProneCrawl => self.prone_crawl,
            SoldierAnimState::ProneCrawlBack => self.prone_crawl_back,
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
/// a remote-player (or bot) avatar's spawned scene. Read by
/// `net::animate_remote_avatars` to switch between idle/walk/sprint.
#[derive(Component)]
pub(crate) struct SoldierAnimationPlayer(pub(crate) Entity);

/// Panel-adjustable uniform scale for the `models/characters/soldier.glb` remote-player
/// avatar ("Remote players" debug-panel section), applied by
/// `net::follow_remote_avatars`. Live-tweakable rather than a baked constant
/// — the model's authored size relative to a real
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

/// On a soldier avatar that's a bot (a `Freestyle` target, a `FreeForAll` bot
/// player, or either's kill-cam stand-in): [`tint_bot_avatars`] swaps its body
/// material for a tinted one, so bots can be told apart from real players at
/// a glance while still using the same model.
#[derive(Component)]
pub(crate) struct BotLook;

/// Set once [`tint_bot_avatars`] has re-skinned a [`BotLook`] avatar.
#[derive(Component)]
pub(crate) struct BotLookApplied;

/// Panel-adjustable bot body tint ("Remote players" debug-panel section).
#[derive(Resource)]
pub(crate) struct BotLookSettings {
    /// sRGB multiplier on the body texture — `models/characters/soldier.glb`'s armour is
    /// mostly white, so this comes out as the armour's colour, with the
    /// texture's detail and shading intact.
    pub(crate) tint: [f32; 3],
}

impl Default for BotLookSettings {
    fn default() -> Self {
        Self {
            tint: [0.12, 0.2, 0.55],
        }
    }
}

/// `models/characters/soldier.glb`'s body material (index 0, "soldier"); index 1, the
/// rifle, is left as-is.
const SOLDIER_BODY_MATERIAL: usize = 0;

/// Re-skin every [`BotLook`] avatar whose scene has finished spawning: each
/// mesh using the body material gets one shared tinted copy of it instead
/// (built on first use, re-tinted live when [`BotLookSettings`] changes).
#[allow(clippy::type_complexity)]
pub(crate) fn tint_bot_avatars(
    settings: Res<BotLookSettings>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tinted: Local<Option<Handle<StandardMaterial>>>,
    avatars: Query<
        Entity,
        (
            With<BotLook>,
            With<SoldierAnimationPlayer>,
            Without<BotLookApplied>,
        ),
    >,
    children: Query<&Children>,
    mut meshes: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut commands: Commands,
) {
    let tint = Color::srgb(settings.tint[0], settings.tint[1], settings.tint[2]);
    if let Some(handle) = tinted.as_ref() {
        // (`get` first: `get_mut` alone would re-upload the material every
        // frame the debug panel's picker marks the settings changed.)
        if settings.is_changed() && materials.get(handle).is_some_and(|m| m.base_color != tint) {
            if let Some(m) = materials.get_mut(handle) {
                m.base_color = tint;
            }
        }
    }
    if avatars.is_empty() {
        return;
    }
    let body: Handle<StandardMaterial> = asset_server.load(
        GltfAssetLabel::Material {
            index: SOLDIER_BODY_MATERIAL,
            is_scale_inverted: false,
        }
        .from_asset("models/characters/soldier.glb"),
    );
    if tinted.is_none() {
        // (The scene is spawned, so its materials are loaded.)
        let Some(original) = materials.get(&body) else {
            return;
        };
        let mut copy = original.clone();
        copy.base_color = tint;
        *tinted = Some(materials.add(copy));
    }
    let Some(tinted) = tinted.as_ref() else { return };
    for root in &avatars {
        for entity in children.iter_descendants(root) {
            if let Ok(mut mat) = meshes.get_mut(entity) {
                if mat.0 == body {
                    mat.0 = tinted.clone();
                }
            }
        }
        commands.entity(root).insert(BotLookApplied);
    }
}

/// A remote player's (or bot's) sniper-glint sprite — `textures/vfx/sniper_glint.png`
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
/// position in `models/characters/soldier.glb`: **not** the gun mesh's own node
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
        texture: asset_server.load("textures/vfx/sniper_glint.png"),
    });
}

/// Panel-adjustable sniper-glint placement/size ("Sniper glint" debug-panel
/// section), applied every frame by `net::update_sniper_glints`.
#[derive(Resource)]
pub(crate) struct SniperGlintSettings {
    /// Offset (world metres) from [`SniperGlintBone`]'s current animated
    /// position, in *its own* local orientation: `x` right(+)/left(-), `y`
    /// up(+)/down(-), `z` forward(+)/back(-) — whichever way those map onto
    /// the right-hand bone's own rest-pose axes in `models/characters/soldier.glb` (the
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

/// The scope on a remote soldier's gun right now — where the sniper glint
/// sits: the scope of the gun in their hands (`held`, its holder and grip —
/// `held_weapon`), else `glint.offset` off the gun bone's animated pose
/// (`bone`), or off the replicated pose for the frame or so before the
/// scene's found the bone. (The bone's `GlobalTransform` carries the
/// avatar's big scale, so only its translation / rotation are used — the
/// offset stays in metres.)
pub(crate) fn soldier_scope_point(
    pose: &shared::PlayerPose,
    bone: Option<&GlobalTransform>,
    held: Option<(&GlobalTransform, &Grip)>,
    glint: &SniperGlintSettings,
) -> Vec3 {
    if let Some((at, scope)) = held.and_then(|(at, grip)| Some((at, grip.scope?))) {
        return at.transform_point(Vec3::from_array(scope));
    }
    let (origin, rotation) = match bone {
        Some(gt) => (gt.translation(), gt.rotation()),
        None => (pose.translation, Quat::from_rotation_y(pose.yaw)),
    };
    origin
        + rotation * Vec3::X * glint.offset.x
        + rotation * Vec3::Y * glint.offset.y
        + rotation * Vec3::NEG_Z * glint.offset.z
}

/// Where a remote soldier's barrel ends — their shot tracers start here
/// (`net::receive_shots`) — when there's no gun in their hands to take it
/// from (`held_weapon::Grip::muzzle`).
#[derive(Resource)]
pub(crate) struct RemoteMuzzleSettings {
    /// Offset (m) from the scope ([`soldier_scope_point`]) along where
    /// they're aiming: `x` right, `y` up, `z` forward — so it follows the
    /// gun through every animation, aim raise included.
    pub(crate) offset: Vec3,
    /// Debug: mark every remote soldier's muzzle point, to tune `offset`.
    pub(crate) show_marker: bool,
}

impl Default for RemoteMuzzleSettings {
    fn default() -> Self {
        Self {
            offset: Vec3::new(0.0, -0.06, 0.6),
            show_marker: false,
        }
    }
}

/// The muzzle of the gun in a soldier's hands (`held`) — or, without one,
/// [`RemoteMuzzleSettings`]' point for a soldier at `pose`.
pub(crate) fn soldier_muzzle_point(
    pose: &shared::PlayerPose,
    bone: Option<&GlobalTransform>,
    held: Option<(&GlobalTransform, &Grip)>,
    glint: &SniperGlintSettings,
    muzzle: &RemoteMuzzleSettings,
) -> Vec3 {
    if let Some((at, grip)) = held {
        return at.transform_point(Vec3::from_array(grip.muzzle));
    }
    let aim = Quat::from_euler(EulerRot::YXZ, pose.yaw, pose.pitch, 0.0);
    soldier_scope_point(pose, bone, None, glint) + aim * Vec3::new(muzzle.offset.x, muzzle.offset.y, -muzzle.offset.z)
}

/// Build the remote-player animation graph. Part of `main.rs`'s one
/// `Startup` tuple rather than its own `add_systems(Startup, ...)`.
pub(crate) fn setup_soldier_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Indices into `models/characters/soldier.glb`'s 18 animations: 0 idleWgun, 1 walk,
    // 3 run, 4 shooting, 6 runAndShooting, 7 strafeRight, 8 strafeLeft,
    // 9 backpaddle, 10 jump, 11 crouch, 12 crouchWalk, 13 reload, 14 melee,
    // 15 death, 17 idleWithoutGun — and the three `client/tools/
    // soldier_prone_clips.py` adds after them: 18 prone, 19 proneCrawl,
    // 20 proneCrawlBack, 21 throw, 22 drink.
    let idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(0).from_asset("models/characters/soldier.glb"));
    let walk_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(1).from_asset("models/characters/soldier.glb"));
    let sprint_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(3).from_asset("models/characters/soldier.glb"));
    let aim_idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(4).from_asset("models/characters/soldier.glb"));
    let aim_move_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(6).from_asset("models/characters/soldier.glb"));
    let crouch_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(11).from_asset("models/characters/soldier.glb"));
    let crouch_walk_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(12).from_asset("models/characters/soldier.glb"));
    let reload_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(13).from_asset("models/characters/soldier.glb"));
    let jump_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(10).from_asset("models/characters/soldier.glb"));
    let strafe_right_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(7).from_asset("models/characters/soldier.glb"));
    let strafe_left_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(8).from_asset("models/characters/soldier.glb"));
    let backward_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(9).from_asset("models/characters/soldier.glb"));
    let melee_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(14).from_asset("models/characters/soldier.glb"));
    let death_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(15).from_asset("models/characters/soldier.glb"));
    let idle_no_gun_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(17).from_asset("models/characters/soldier.glb"));
    let prone_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(18).from_asset("models/characters/soldier.glb"));
    let prone_crawl_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(19).from_asset("models/characters/soldier.glb"));
    let prone_crawl_back_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(20).from_asset("models/characters/soldier.glb"));
    let throw_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(21).from_asset("models/characters/soldier.glb"));
    let drink_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(22).from_asset("models/characters/soldier.glb"));
    let (mut graph, indices) = AnimationGraph::from_clips([
        idle_clip,
        walk_clip,
        sprint_clip,
        aim_idle_clip.clone(),
        crouch_clip,
        crouch_walk_clip,
        reload_clip,
        jump_clip,
        strafe_right_clip,
        strafe_left_clip,
        backward_clip,
        death_clip,
        melee_clip,
        idle_no_gun_clip,
        prone_clip,
        prone_crawl_clip,
        prone_crawl_back_clip,
    ]);
    // (The two halves of aiming on the move — see `soldier_mask_groups`.)
    let root = graph.root;
    let aim_move_legs = graph.add_clip_with_mask(aim_move_clip, 1 << UPPER_BODY, 1.0, root);
    let shot_upper = graph.add_clip_with_mask(aim_idle_clip, 1 << LOWER_BODY, 1.0, root);
    let throw_upper = graph.add_clip_with_mask(throw_clip, 1 << LOWER_BODY, 1.0, root);
    let drink_upper = graph.add_clip_with_mask(drink_clip, 1 << LOWER_BODY, 1.0, root);
    let graph = graphs.add(graph);
    commands.insert_resource(SoldierAnimations {
        graph,
        idle: indices[0],
        walk: indices[1],
        sprint: indices[2],
        aim_idle: indices[3],
        aim_move_legs,
        shot_upper,
        throw_upper,
        drink_upper,
        crouch: indices[4],
        crouch_walk: indices[5],
        reload: indices[6],
        jump: indices[7],
        strafe_right: indices[8],
        strafe_left: indices[9],
        backward: indices[10],
        death: indices[11],
        melee: indices[12],
        idle_no_gun: indices[13],
        prone: indices[14],
        prone_crawl: indices[15],
        prone_crawl_back: indices[16],
    });
}

/// `throw`'s length (s) — the first-person throw's (`THROW_CLIP_SECS`), its
/// release on the same moment of it.
pub(crate) const THROW_SECS: f32 = 0.667;
/// When (s into `throw`) it leaves the hand — the first-person throw's
/// default (`ThrowArmsSettings::throw_release_secs`).
pub(crate) const THROW_RELEASE_SECS: f32 = 0.25;

/// `drink`'s length (s) — the first-person drink's (its clip's frames
/// 160..410 at 30 fps, played 5×), so a soldier's ends just as theirs does.
pub(crate) const DRINK_SECS: f32 = 1.667;

/// A clip over a soldier's upper body (a throw, a drink): how much of it the
/// clip has, and where in it it is — kept on whoever drives a soldier
/// ([`drive_throw_layer`], [`drive_drink_layer`]).
#[derive(Clone, Copy, Default)]
pub(crate) struct UpperLayer {
    share: f32,
    at: f32,
}

/// Play the throw over a soldier's upper body as `stage` says (a player's
/// `PlayerPose::throw`): the arm coming up ready as theirs does — and held
/// there while they cook it — then the throw in step with theirs, easing off
/// over `fade_secs` once it's done.
pub(crate) fn drive_throw_layer(
    player: &mut AnimationPlayer,
    anims: &SoldierAnimations,
    layer: &mut UpperLayer,
    stage: Option<shared::ThrowStage>,
    dt: f32,
    fade_secs: f32,
) {
    let want = stage.map(|stage| match stage {
        shared::ThrowStage::Raising(s) => (s.clamp(0.0, 1.0), 0.0),
        shared::ThrowStage::Throwing { t, .. } => (1.0, t.clamp(0.0, 1.0) * THROW_SECS),
    });
    drive_upper_layer(player, anims.throw_upper, layer, want, dt, fade_secs);
}

/// Play the drink over a soldier's upper body, `t` (`0..1`, a player's
/// `PlayerPose::drink`) of the way through: taking over as the bottle comes
/// off the hip, in step with theirs, easing off over `fade_secs` after.
pub(crate) fn drive_drink_layer(
    player: &mut AnimationPlayer,
    anims: &SoldierAnimations,
    layer: &mut UpperLayer,
    t: Option<f32>,
    dt: f32,
    fade_secs: f32,
) {
    let want = t.map(|t| {
        let t = t.clamp(0.0, 1.0);
        ((t / 0.08).min(1.0), t * DRINK_SECS)
    });
    drive_upper_layer(player, anims.drink_upper, layer, want, dt, fade_secs);
}

/// Play `node` (masked to the upper body) at `want`'s `(share, seconds into
/// it)`, or ease it off over `fade_secs` — rising with `share`, never
/// snapping off. It takes over the upper body from whatever else is playing
/// there: Bevy blends clips by their share of the total weight, so it gets
/// `share / (1 - share)` against the rest's ~1.
fn drive_upper_layer(
    player: &mut AnimationPlayer,
    node: AnimationNodeIndex,
    layer: &mut UpperLayer,
    want: Option<(f32, f32)>,
    dt: f32,
    fade_secs: f32,
) {
    let (target, at) = want.unwrap_or((0.0, layer.at));
    layer.at = at;
    layer.share = if target >= layer.share {
        target
    } else {
        (layer.share - dt / fade_secs.max(1e-3)).max(target)
    };
    if layer.share <= 0.0 {
        player.stop(node);
        return;
    }
    let active = player.play(node);
    active.pause().seek_to(layer.at);
    let share = layer.share.min(0.995);
    active.set_weight(share / (1.0 - share));
}

/// `shooting`'s length (s): one shot — the kick at 0.07 s, back on aim by
/// the end (its last frame and its first are the same pose).
pub(crate) const SHOT_CLIP_SECS: f32 = 0.667;

/// The animation graph's mask groups: the spine and everything on it (arms,
/// head)...
const UPPER_BODY: u32 = 0;
/// ...and the hips and legs.
const LOWER_BODY: u32 = 1;

/// Sort every animated bone into [`UPPER_BODY`] or [`LOWER_BODY`], once —
/// the graph's shared by every soldier, so the first one in does it.
fn soldier_mask_groups(
    graph: &mut AnimationGraph,
    root: Entity,
    spine: Entity,
    children: &Query<&Children>,
    targets: &Query<&AnimationTarget>,
    parents: &Query<&ChildOf>,
) {
    if !graph.mask_groups.is_empty() {
        return;
    }
    for entity in children.iter_descendants(root) {
        let Ok(target) = targets.get(entity) else { continue };
        let mut at = entity;
        let upper = loop {
            if at == spine {
                break true;
            }
            match parents.get(at) {
                Ok(p) if at != root => at = p.parent(),
                _ => break false,
            }
        };
        graph.add_target_to_mask_group(target.id, if upper { UPPER_BODY } else { LOWER_BODY });
    }
}

/// `soldier.glb`'s own rifle's mesh (by its asset label): skinned to the
/// right hand like the body, but no part of the player. Hidden on a soldier
/// that holds a real weapon instead (`held_weapon`).
pub(crate) const SOLDIER_GUN_MESH: &str = "Mesh1/Primitive0";

/// Fires once a remote-player avatar's `SceneRoot` (tagged [`SoldierVisual`])
/// finishes spawning: starts the idle animation looping, remembers which
/// descendant holds the `AnimationPlayer` via [`SoldierAnimationPlayer`],
/// remembers the right-hand bone the gun mesh is skinned to
/// ([`SNIPER_GLINT_BONE_NAME`]) via [`SniperGlintBone`] so the sniper glint
/// can ride along with it, and finds the arm bones and the model's own rifle
/// ([`SoldierRig`]) — hiding the rifle on one that holds a [`HeldWeapon`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_soldier_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    soldiers: Query<Has<HeldWeapon>, With<SoldierVisual>>,
    mut players: Query<&mut AnimationPlayer>,
    names: Query<&Name>,
    meshes: Query<&Mesh3d>,
    anims: Res<SoldierAnimations>,
    (mut graphs, targets, parents): (ResMut<Assets<AnimationGraph>>, Query<&AnimationTarget>, Query<&ChildOf>),
) {
    let root = trigger.target();
    let Ok(holds) = soldiers.get(root) else {
        return;
    };
    let mut right = [None; 3];
    let mut left = [None; 3];
    let (mut left_shoulder, mut right_thigh) = (None, None);
    let mut spine = [None; 3];
    let mut baked_gun = None;
    for entity in children.iter_descendants(root) {
        if let Ok(name) = names.get(entity) {
            let name = name.as_str();
            if let Some(i) = RIGHT_ARM_BONES.iter().position(|&b| b == name) {
                right[i] = Some(entity);
            }
            if let Some(i) = LEFT_ARM_BONES.iter().position(|&b| b == name) {
                left[i] = Some(entity);
            }
            if name == LEFT_SHOULDER_BONE {
                left_shoulder = Some(entity);
            }
            if name == RIGHT_THIGH_BONE {
                right_thigh = Some(entity);
            }
            if let Some(i) = SPINE_BONES.iter().position(|&b| b == name) {
                spine[i] = Some(entity);
            }
        }
        if meshes.get(entity).is_ok_and(|m| m.0.path().and_then(|p| p.label()) == Some(SOLDIER_GUN_MESH)) {
            baked_gun = Some(entity);
            if holds {
                commands.entity(entity).insert(Visibility::Hidden);
            }
        }

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
    if let (Some(spine), Some(graph)) = (spine[0], graphs.get_mut(anims.graph())) {
        soldier_mask_groups(graph, root, spine, &children, &targets, &parents);
    }
    if let (Some(right), Some(left), Some(left_shoulder), Some(spine), Some(right_thigh)) =
        (bones(right), bones(left), left_shoulder, bones(spine), right_thigh)
    {
        commands.entity(root).insert(SoldierRig {
            right,
            left,
            left_shoulder,
            spine,
            right_thigh,
            baked_gun,
        });
    } else {
        warn!("soldier.glb: arm bones not found — its weapon won't show");
    }
}

fn bones(found: [Option<Entity>; 3]) -> Option<[Entity; 3]> {
    Some([found[0]?, found[1]?, found[2]?])
}
