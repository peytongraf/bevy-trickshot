//! What a soldier avatar has in its hands — the weapon its player really
//! holds (`PlayerPose::held`), not the rifle `models/characters/soldier.glb`
//! comes with (that one's hidden, [`SoldierRig::baked_gun`]).
//!
//! * **The weapon** is the same arm-less model a dropped one is
//!   (`weapon_drops::DropModel`, stood upright: barrel along -X, sights up,
//!   its middle at the origin, real-world size, Pack-a-Punch camo and all),
//!   on a holder parented to the right-hand bone — so it follows every
//!   animation.
//! * **Where it sits** is that weapon's [`Grip`], from `data/weapon_grips.ron`
//!   (the WEAPON EDITOR's, `weapon_editor`): its place and turn in the right
//!   hand; how far the right hand itself is moved from where the animation
//!   has it (a pistol held out in front rather than at the shoulder, as the
//!   rifle clips have it); where the left hand goes on it; and its muzzle
//!   (tracers start there) and scope (the sniper glint).
//! * **The hands** are put there by two-bone IK on each arm
//!   ([`pose_held_weapons`]), after the animation and before transforms
//!   propagate — keeping each elbow bent the way the animation bends it.
//!   It eases out while the animation needs the arms for itself (a reload,
//!   a stab, dying — [`ik_wanted`]). A one-handed weapon's left arm hangs
//!   down by the side instead ([`Grip::left_arm_down`], the pose
//!   [`WeaponGrips::arm_down`] — `idleWithoutGun`'s unless the WEAPON
//!   EDITOR's taken another).
//! * **A knife stab** is the right hand's own thrust ([`stab_curve`],
//!   [`Grip::stab_reach`]), the chest leaning and twisting into it
//!   ([`Grip::stab_lean_deg`]), over whatever the legs are doing —
//!   `soldier.glb` has no stab of its own (its `melee` is a rifle's).
//!
//! Everything here hangs off the avatar it's on, so it goes with it; the
//! grips are config, the same in every game.

use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;
use bevy::transform::TransformSystem;
use serde::{Deserialize, Serialize};
use shared::weapon::{SlotWeapon, WeaponId};

use super::SoldierAnimState;
use crate::weapon_drops::{model_path, DropModel};

pub(crate) struct HeldWeaponPlugin;

impl Plugin for HeldWeaponPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WeaponGrips::load())
            .add_observer(tag_held_weapon_parts)
            .add_systems(Update, attach_held_weapons)
            .add_systems(
                PostUpdate,
                (
                    // (Puts back what the IK changed last frame, for a bone
                    // the clip playing doesn't happen to key.)
                    unpose_held_weapons.before(bevy::app::Animation),
                    pose_held_weapons
                        .after(bevy::app::Animation)
                        .before(TransformSystem::TransformPropagate),
                ),
            );
    }
}

// --- the grips ------------------------------------------------------------------------

/// Where the grips are kept in the repo — the WEAPON EDITOR saves over it.
pub(crate) const GRIPS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data");
pub(crate) const GRIPS_FILE: &str = "weapon_grips.ron";

pub(crate) fn grips_source_path() -> std::path::PathBuf {
    std::path::Path::new(GRIPS_DIR).join(GRIPS_FILE)
}

/// Which grip a weapon uses — one per model (the Marksman is drawn as the
/// sniper, `weapon_drops::model_path`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum GripKey {
    Sniper,
    Ak74,
    RayGun,
    Knife,
}

impl GripKey {
    pub(crate) const ALL: [GripKey; 4] = [GripKey::Sniper, GripKey::Ak74, GripKey::RayGun, GripKey::Knife];

    pub(crate) fn of(weapon: SlotWeapon) -> GripKey {
        match weapon {
            SlotWeapon::Gun(WeaponId::Ak74) => GripKey::Ak74,
            SlotWeapon::Gun(WeaponId::RayGun) => GripKey::RayGun,
            SlotWeapon::Gun(_) => GripKey::Sniper,
            SlotWeapon::Knife => GripKey::Knife,
        }
    }

    pub(crate) fn weapon(self) -> SlotWeapon {
        match self {
            GripKey::Sniper => SlotWeapon::Gun(WeaponId::Sniper),
            GripKey::Ak74 => SlotWeapon::Gun(WeaponId::Ak74),
            GripKey::RayGun => SlotWeapon::Gun(WeaponId::RayGun),
            GripKey::Knife => SlotWeapon::Knife,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            GripKey::Sniper => "Sniper",
            GripKey::Ak74 => "AK-74",
            GripKey::RayGun => "Ray Gun",
            GripKey::Knife => "Knife",
        }
    }
}

/// A hand's place on a weapon: its wrist (the hand bone's origin), in the
/// weapon's own frame (m — barrel along -X, sights up +Y, its middle at the
/// origin), and the hand's turn there (degrees, X then Y then Z).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub(crate) struct HandTarget {
    pub(crate) pos: [f32; 3],
    pub(crate) turn_deg: [f32; 3],
}

impl HandTarget {
    pub(crate) fn rotation(&self) -> Quat {
        euler_deg(self.turn_deg)
    }
}

/// How one weapon is held ([`WeaponGrips`]).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Grip {
    /// The weapon's middle from the right-hand bone (m, in the hand bone's
    /// own axes)...
    pub(crate) offset: [f32; 3],
    /// ...and its turn there (degrees, X then Y then Z).
    pub(crate) turn_deg: [f32; 3],
    /// Its size, from its real-world one (`weapon_drops::real_length`).
    pub(crate) scale: f32,
    /// How far the right hand — and the weapon with it — is moved from
    /// where the animation has it (m: right, up, forward from the body)...
    pub(crate) hand_shift: [f32; 3],
    /// ...and turned (degrees: pitch up, yaw left, roll — about the body's
    /// right, up and forward).
    pub(crate) hand_turn_deg: [f32; 3],
    /// Where the left hand goes on it — `None` leaves the left hand where
    /// the animation has it.
    pub(crate) left_hand: Option<HandTarget>,
    /// The left arm hangs down by the side instead ([`WeaponGrips::arm_down`])
    /// — a one-handed weapon. Wins over `left_hand`.
    pub(crate) left_arm_down: bool,
    /// A stab with it in hand (the knife's): how far the right hand thrusts
    /// at its furthest (m: right, up, forward from the body)...
    pub(crate) stab_reach: [f32; 3],
    /// ...and turns there (degrees: pitch up, yaw left, roll)...
    pub(crate) stab_turn_deg: [f32; 3],
    /// ...and how far the chest leans and twists into it there (degrees:
    /// pitch up, yaw left — the right shoulder forward — roll).
    pub(crate) stab_lean_deg: [f32; 3],
    /// Its muzzle, in its own frame (m) — where a shot's tracer starts.
    pub(crate) muzzle: [f32; 3],
    /// Its scope's lens, in its own frame (m) — where the sniper glint
    /// shows. `None`: no scope, no glint.
    pub(crate) scope: Option<[f32; 3]>,
}

impl Default for Grip {
    fn default() -> Self {
        Self {
            offset: BAKED_GUN_CENTER,
            turn_deg: BAKED_GUN_TURN_DEG,
            scale: 1.0,
            hand_shift: [0.0; 3],
            hand_turn_deg: [0.0; 3],
            left_hand: None,
            left_arm_down: false,
            stab_reach: [0.0, 0.06, 0.42],
            stab_turn_deg: [-20.0, 0.0, 0.0],
            stab_lean_deg: [-10.0, 15.0, 0.0],
            muzzle: [-0.5, 0.0, 0.0],
            scope: None,
        }
    }
}

impl Grip {
    pub(crate) fn rotation(&self) -> Quat {
        euler_deg(self.turn_deg)
    }

    /// How far the right hand's moved from the animation's (m: right, up,
    /// forward) and turned (in the model's frame), `thrust` of the way into
    /// a stab ([`stab_curve`]) — `None` for not at all.
    fn hand_move(&self, thrust: f32) -> Option<(Vec3, Quat)> {
        let stab = |v: [f32; 3]| v.map(|x| x * thrust);
        let shift = Vec3::from_array(self.hand_shift) + Vec3::from_array(stab(self.stab_reach));
        let turn = body_turn(self.hand_turn_deg) * body_turn(stab(self.stab_turn_deg));
        (self.hand_shift != [0.0; 3] || self.hand_turn_deg != [0.0; 3] || thrust != 0.0).then_some((shift, turn))
    }
}

/// A turn in the model's frame (degrees: pitch up, yaw left, roll — about
/// the body's right, up and forward).
fn body_turn(deg: [f32; 3]) -> Quat {
    let [pitch, yaw, roll] = deg.map(f32::to_radians);
    Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(MODEL_RIGHT, pitch) * Quat::from_axis_angle(MODEL_FORWARD, roll)
}

/// How far into its thrust a knife stab `p` (`0..1`) of the way through is:
/// a little wind-up back, a quick lunge to full reach (`1`), a moment there,
/// then back. The stab lands as it reaches full stretch
/// (`shared::melee::KNIFE_HIT_SECS` in — keep the two together).
pub(crate) fn stab_curve(p: f32) -> f32 {
    const WIND_UP: f32 = 0.15;
    let smooth = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    match p {
        p if p < 0.15 => -WIND_UP * smooth(p / 0.15),
        p if p < 0.32 => -WIND_UP + (1.0 + WIND_UP) * smooth((p - 0.15) / 0.17),
        p if p < 0.5 => 1.0,
        p => 1.0 - smooth((p - 0.5) / 0.5),
    }
}

/// The left arm's turns (shoulder, upper arm, forearm, hand — each from its
/// parent, as quaternions) hanging down by the side —
/// [`WeaponGrips::arm_down`].
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub(crate) struct ArmPose {
    pub(crate) shoulder: [f32; 4],
    pub(crate) upper: [f32; 4],
    pub(crate) fore: [f32; 4],
    pub(crate) hand: [f32; 4],
}

impl ArmPose {
    /// `soldier.glb`'s `idleWithoutGun`, its first frame — the arm relaxed
    /// at the side (read off the file's keys).
    pub(crate) const IDLE_NO_GUN: ArmPose = ArmPose {
        shoulder: [0.61328, 0.33109, -0.60466, 0.38555],
        upper: [0.26526, 0.07946, -0.12997, 0.95207],
        fore: [0.09365, -0.02649, 0.36103, 0.92746],
        hand: [0.05759, 0.29399, -0.01775, 0.95391],
    };

    fn turns(&self) -> [[f32; 4]; 4] {
        [self.shoulder, self.upper, self.fore, self.hand]
    }
}

impl Default for ArmPose {
    fn default() -> Self {
        Self::IDLE_NO_GUN
    }
}

/// Where `soldier.glb`'s own rifle sits in the right hand — measured off its
/// mesh (rigidly skinned to that bone: every vertex through its inverse
/// bind matrix, then the hand's scale): its middle (m, hand axes)...
pub(crate) const BAKED_GUN_CENTER: [f32; 3] = [0.107, 0.158, 0.014];
/// ...and the turn that lines a weapon's own frame (barrel along -X, sights
/// up) up with it: barrel along the hand's +Y, sights toward its +X.
pub(crate) const BAKED_GUN_TURN_DEG: [f32; 3] = [3.33, 10.78, -92.55];

/// Every weapon's [`Grip`] — `data/weapon_grips.ron`, built in; the WEAPON
/// EDITOR changes it live and saves it back.
#[derive(Resource, Clone, PartialEq, Debug, Serialize, Deserialize)]
pub(crate) struct WeaponGrips {
    pub(crate) sniper: Grip,
    pub(crate) ak74: Grip,
    pub(crate) raygun: Grip,
    pub(crate) knife: Grip,
    /// The left arm down by the side, for every one-handed weapon
    /// ([`Grip::left_arm_down`]) — `idleWithoutGun`'s, unless the WEAPON
    /// EDITOR's taken another.
    #[serde(default)]
    pub(crate) arm_down: ArmPose,
}

impl WeaponGrips {
    pub(crate) fn get(&self, key: GripKey) -> &Grip {
        match key {
            GripKey::Sniper => &self.sniper,
            GripKey::Ak74 => &self.ak74,
            GripKey::RayGun => &self.raygun,
            GripKey::Knife => &self.knife,
        }
    }

    pub(crate) fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub(crate) fn to_ron(&self) -> String {
        let pretty = ron::ser::PrettyConfig::new().struct_names(false).depth_limit(3);
        ron::ser::to_string_pretty(&self.rounded(), pretty).unwrap_or_default()
    }

    /// The repo's when there is one (it may be newer than this build), else
    /// the built-in one.
    fn load() -> Self {
        let built_in = || {
            Self::from_ron(include_str!("../../data/weapon_grips.ron"))
                .unwrap_or_else(|e| panic!("client/data/{GRIPS_FILE}: {e}"))
        };
        match std::fs::read_to_string(grips_source_path()) {
            Ok(text) => Self::from_ron(&text).unwrap_or_else(|e| {
                warn!("couldn't read {GRIPS_FILE}: {e} — using the built-in grips");
                built_in()
            }),
            Err(_) => built_in(),
        }
    }

    /// Every number to a tidy few decimals, for the file.
    fn rounded(&self) -> Self {
        let r = |v: f32, per: f32| (v * per).round() / per;
        let m = |a: [f32; 3]| a.map(|v| r(v, 10_000.0));
        let d = |a: [f32; 3]| a.map(|v| r(v, 100.0));
        let grip = |g: &Grip| Grip {
            offset: m(g.offset),
            turn_deg: d(g.turn_deg),
            scale: r(g.scale, 1000.0),
            hand_shift: m(g.hand_shift),
            hand_turn_deg: d(g.hand_turn_deg),
            left_hand: g.left_hand.map(|h| HandTarget {
                pos: m(h.pos),
                turn_deg: d(h.turn_deg),
            }),
            left_arm_down: g.left_arm_down,
            stab_reach: m(g.stab_reach),
            stab_turn_deg: d(g.stab_turn_deg),
            stab_lean_deg: d(g.stab_lean_deg),
            muzzle: m(g.muzzle),
            scope: g.scope.map(m),
        };
        let q = |a: [f32; 4]| a.map(|v| r(v, 100_000.0));
        Self {
            sniper: grip(&self.sniper),
            ak74: grip(&self.ak74),
            raygun: grip(&self.raygun),
            knife: grip(&self.knife),
            arm_down: ArmPose {
                shoulder: q(self.arm_down.shoulder),
                upper: q(self.arm_down.upper),
                fore: q(self.arm_down.fore),
                hand: q(self.arm_down.hand),
            },
        }
    }
}

pub(crate) fn euler_deg(deg: [f32; 3]) -> Quat {
    let [x, y, z] = deg.map(f32::to_radians);
    Quat::from_euler(EulerRot::XYZ, x, y, z)
}

pub(crate) fn to_euler_deg(q: Quat) -> [f32; 3] {
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    [x, y, z].map(f32::to_degrees)
}

// --- on the avatar ------------------------------------------------------------------

/// `soldier.glb`'s model frame (its root's, under the avatar's own turn):
/// it faces +Z, so its right is -X.
const MODEL_RIGHT: Vec3 = Vec3::NEG_X;
const MODEL_FORWARD: Vec3 = Vec3::Z;

/// The bones moved here, and the rifle the model comes with — found by
/// `start_soldier_animation` once the scene's in.
#[derive(Component, Clone, Copy)]
pub(crate) struct SoldierRig {
    /// Upper arm, forearm, hand.
    pub(crate) right: [Entity; 3],
    pub(crate) left: [Entity; 3],
    /// The left collarbone — lowered with the arm, down by the side.
    pub(crate) left_shoulder: Entity,
    /// The chest — leaning into a knife stab.
    pub(crate) chest: Entity,
    /// `soldier.glb`'s own rifle mesh — hidden on a soldier that holds a
    /// [`HeldWeapon`] (shown again by the WEAPON EDITOR, to line one up
    /// against).
    pub(crate) baked_gun: Option<Entity>,
}

/// `soldier.glb`'s arm bones, by glTF name: upper arm, forearm, hand.
pub(crate) const RIGHT_ARM_BONES: [&str; 3] = ["Bind_RightArm_035_036", "Bind_RightForeArm_036_037", "Bind_RightHand_037_038"];
pub(crate) const LEFT_ARM_BONES: [&str; 3] = ["Bind_LeftArm_016_017", "Bind_LeftForeArm_017_018", "Bind_LeftHand_018_019"];
pub(crate) const LEFT_SHOULDER_BONE: &str = "Bind_LeftShoulder_015_016";
pub(crate) const CHEST_BONE: &str = "Bind_Spine2_012_013";

/// On a soldier avatar: what it's holding (set every frame by whoever
/// drives it — `net` from the player's pose, or the WEAPON EDITOR).
#[derive(Component)]
pub(crate) struct HeldWeapon {
    pub(crate) item: Option<SlotWeapon>,
    /// Its Pack-a-Punch level (its camo).
    pub(crate) pap: u8,
    /// How much the hands should be on it now (`0..=1`) — `0` hands the arms
    /// back to the animation ([`ik_wanted`]); eased toward.
    pub(crate) ik_wanted: f32,
    ik: f32,
    /// What's on the holder now.
    shown: Option<(SlotWeapon, u8)>,
    /// The weapon's holder, under the right-hand bone.
    pub(crate) holder: Option<Entity>,
    /// Where the animation had the left hand this frame (before IK), in the
    /// weapon's frame — what the WEAPON EDITOR snaps the left hand to.
    pub(crate) anim_left: Option<HandTarget>,
    /// Their player's stabbing (`PlayerPose::stabbing`) — with the knife in
    /// hand, its rising edge starts a thrust ([`stab_curve`]), which plays out
    /// in full.
    pub(crate) stabbing: bool,
    stab_was: bool,
    /// When (`Time::elapsed_secs`) the thrust playing started.
    stab_from: Option<f32>,
    /// The arm bones as the animation left them, before IK changed them
    /// ([`unpose_held_weapons`]).
    animated: Vec<(Entity, Transform)>,
}

impl HeldWeapon {
    pub(crate) fn new(item: Option<SlotWeapon>) -> Self {
        Self {
            item,
            pap: 0,
            ik_wanted: 1.0,
            ik: 1.0,
            shown: None,
            holder: None,
            anim_left: None,
            stabbing: false,
            stab_was: false,
            stab_from: None,
            animated: Vec::new(),
        }
    }

    /// A knife thrust is playing.
    pub(crate) fn stab_playing(&self) -> bool {
        self.stab_from.is_some()
    }

    /// The left arm as the animation had it last frame (before IK) — what
    /// the WEAPON EDITOR takes the "down by the side" pose from.
    pub(crate) fn animated_left_arm(&self, rig: &SoldierRig) -> Option<ArmPose> {
        let local = |e: Entity| self.animated.iter().find(|(b, _)| *b == e).map(|(_, t)| t.rotation.normalize().to_array());
        Some(ArmPose {
            shoulder: local(rig.left_shoulder)?,
            upper: local(rig.left[0])?,
            fore: local(rig.left[1])?,
            hand: local(rig.left[2])?,
        })
    }
}

impl Default for HeldWeapon {
    fn default() -> Self {
        Self::new(None)
    }
}

/// How much the hands should be on the weapon while `state` plays: not at
/// all while the clip needs the arms for itself — a reload's hands go to
/// the magazine, a stab swings, a death falls.
pub(crate) fn ik_wanted(state: SoldierAnimState) -> f32 {
    match state {
        SoldierAnimState::Reload | SoldierAnimState::Melee | SoldierAnimState::Dead => 0.0,
        _ => 1.0,
    }
}

/// The holder, under the right-hand bone: placed by [`pose_held_weapons`].
#[derive(Component)]
pub(crate) struct HeldWeaponHolder;

/// The weapon's scene, under its [`HeldWeaponHolder`].
#[derive(Component)]
struct HeldWeaponModel;

/// Every mesh of a held weapon — not part of the player, so a shot that
/// clips one doesn't hit them (`hit_detection`).
#[derive(Component)]
pub(crate) struct HeldWeaponPart;

/// Keep each soldier's holder holding what it's meant to.
fn attach_held_weapons(
    mut soldiers: Query<(&mut HeldWeapon, &SoldierRig)>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    for (mut held, rig) in &mut soldiers {
        let want = held.item.map(|item| (item, held.pap));
        if want == held.shown && (want.is_none() || held.holder.is_some()) {
            continue;
        }
        if let Some(old) = held.holder.take() {
            if let Ok(mut e) = commands.get_entity(old) {
                e.despawn();
            }
        }
        held.shown = want;
        let Some((weapon, pap)) = want else { continue };
        let holder = commands
            .spawn((HeldWeaponHolder, Transform::default(), Visibility::Inherited, ChildOf(rig.right[2])))
            .id();
        commands.spawn((
            ChildOf(holder),
            HeldWeaponModel,
            DropModel {
                weapon,
                pap,
                upright: true,
            },
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(model_path(weapon)))),
            // (Shown once it's fitted — `weapon_drops::fit_dropped_model`.)
            Visibility::Hidden,
        ));
        held.holder = Some(holder);
    }
}

/// A held weapon's scene is in: mark its meshes as not the player's.
fn tag_held_weapon_parts(
    trigger: Trigger<SceneInstanceReady>,
    models: Query<(), With<HeldWeaponModel>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    mut commands: Commands,
) {
    let root = trigger.target();
    if !models.contains(root) {
        return;
    }
    for e in children.iter_descendants(root) {
        if meshes.contains(e) {
            commands.entity(e).insert((HeldWeaponPart, NoFrustumCulling));
        }
    }
}

/// `e`'s transform in `root`'s frame — the product of the local transforms
/// on the way down (as `weapon_drops::root_from`, over a mutable query).
fn from_root(e: Entity, root: Entity, parents: &Query<&ChildOf>, transforms: &Query<&mut Transform>) -> Affine3A {
    let mut m = Affine3A::IDENTITY;
    let mut at = e;
    while at != root {
        if let Ok(t) = transforms.get(at) {
            m = t.compute_affine() * m;
        }
        let Ok(parent) = parents.get(at) else { break };
        at = parent.parent();
    }
    m
}

/// Put back the arm bones the IK moved, as the animation had them — so a
/// bone the clip playing doesn't key never drifts frame on frame.
fn unpose_held_weapons(mut soldiers: Query<&mut HeldWeapon>, mut transforms: Query<&mut Transform>) {
    for mut held in &mut soldiers {
        for (e, t) in held.animated.drain(..) {
            if let Ok(mut now) = transforms.get_mut(e) {
                *now = t;
            }
        }
    }
}

/// Each soldier's weapon and hands, after the animation and before
/// transforms propagate: the right hand moved by its grip's
/// [`Grip::hand_shift`] (IK), the weapon placed in it, and the left hand
/// put on the weapon (IK) — each as much as [`HeldWeapon`]'s IK says.
#[allow(clippy::type_complexity)]
fn pose_held_weapons(
    time: Res<Time>,
    grips: Res<WeaponGrips>,
    mut soldiers: Query<(Entity, &mut HeldWeapon, &SoldierRig)>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform>,
) {
    let ease = 1.0 - (-time.delta_secs() * 10.0).exp();
    let now = time.elapsed_secs();
    for (root, mut held, rig) in &mut soldiers {
        held.ik += (held.ik_wanted.clamp(0.0, 1.0) - held.ik) * ease;
        if held.stabbing && !held.stab_was {
            held.stab_from = Some(now);
        }
        held.stab_was = held.stabbing;
        let stab = held.stab_from.map(|from| (now - from) / crate::net::STAB_ANIM_SECS);
        if stab.is_some_and(|p| p >= 1.0) {
            held.stab_from = None;
        }
        let (Some(item), Some(holder)) = (held.item, held.holder) else {
            held.anim_left = None;
            continue;
        };
        let grip = *grips.get(GripKey::of(item));
        // Metres per unit of the model's own frame (the avatar's scale).
        let per_m = 1.0 / transforms.get(root).map_or(1.0, |t| t.scale.x).max(1e-4);
        let w = held.ik;
        let mut animated = Vec::with_capacity(8);
        for e in rig.right.into_iter().chain(rig.left).chain([rig.left_shoulder, rig.chest]) {
            if let Ok(t) = transforms.get(e) {
                animated.push((e, *t));
            }
        }

        // Mid-stab with the knife: the chest leans into it...
        let thrust = match (item, stab) {
            (SlotWeapon::Knife, Some(p)) if p < 1.0 => stab_curve(p),
            _ => 0.0,
        };
        if thrust != 0.0 && w > 1e-3 {
            if let Ok(above) = parents.get(rig.chest).map(|p| p.parent()) {
                let above = from_root(above, root, &parents, &transforms).to_scale_rotation_translation().1;
                let chest = from_root(rig.chest, root, &parents, &transforms).to_scale_rotation_translation().1;
                let lean = body_turn(grip.stab_lean_deg.map(|d| d * thrust * w));
                if let Ok(mut t) = transforms.get_mut(rig.chest) {
                    t.rotation = (above.inverse() * lean * chest).normalize();
                }
            }
        }

        // ...and the right hand's moved from the animation's — and thrust.
        let hand = from_root(rig.right[2], root, &parents, &transforms);
        if let Some((shift, turn)) = grip.hand_move(thrust).filter(|_| w > 1e-3) {
            let (_, rot, pos) = hand.to_scale_rotation_translation();
            let shift = (MODEL_RIGHT * shift.x + Vec3::Y * shift.y + MODEL_FORWARD * shift.z) * per_m;
            let to = pos + shift * w;
            let to_rot = rot.slerp(turn * rot, w);
            reach(rig.right, root, to, to_rot, &parents, &mut transforms);
        }

        // The weapon, in that hand.
        let hand = from_root(rig.right[2], root, &parents, &transforms);
        let (_, hand_rot, hand_pos) = hand.to_scale_rotation_translation();
        let weapon_rot = hand_rot * grip.rotation();
        let weapon = Affine3A::from_scale_rotation_translation(
            Vec3::splat(grip.scale.max(1e-3) * per_m),
            weapon_rot,
            hand_pos + hand_rot * (Vec3::from_array(grip.offset) * per_m),
        );
        if let Ok(mut t) = transforms.get_mut(holder) {
            *t = Transform::from_matrix((hand.inverse() * weapon).into());
        }

        // The left hand, on it.
        let left = from_root(rig.left[2], root, &parents, &transforms);
        let (_, left_rot, left_pos) = left.to_scale_rotation_translation();
        let on_weapon = weapon.inverse();
        held.anim_left = Some(HandTarget {
            pos: on_weapon.transform_point3(left_pos).to_array(),
            turn_deg: to_euler_deg(weapon_rot.inverse() * left_rot),
        });
        if grip.left_arm_down {
            if w > 1e-3 {
                let bones = [rig.left_shoulder, rig.left[0], rig.left[1], rig.left[2]];
                for (e, q) in bones.into_iter().zip(grips.arm_down.turns()) {
                    if let Ok(mut t) = transforms.get_mut(e) {
                        t.rotation = t.rotation.slerp(Quat::from_array(q).normalize(), w);
                    }
                }
            }
        } else if let Some(target) = grip.left_hand.filter(|_| w > 1e-3) {
            let to = left_pos.lerp(weapon.transform_point3(Vec3::from_array(target.pos)), w);
            let to_rot = left_rot.slerp(weapon_rot * target.rotation(), w);
            reach(rig.left, root, to, to_rot, &parents, &mut transforms);
        }
        held.animated = animated;
    }
}

/// Two-bone IK: turn `bones` (upper arm, forearm, hand — each the one
/// before's child) so the hand's at `to` turned `to_rot` (`root`'s frame) —
/// or as near as the arm reaches — keeping the elbow bent the way it is.
fn reach(
    bones: [Entity; 3],
    root: Entity,
    to: Vec3,
    to_rot: Quat,
    parents: &Query<&ChildOf>,
    transforms: &mut Query<&mut Transform>,
) {
    let [upper, fore, hand] = bones;
    let Ok(above) = parents.get(upper).map(|p| p.parent()) else { return };
    let above = from_root(above, root, parents, transforms).to_scale_rotation_translation().1;
    let (_, upper_rot, a) = from_root(upper, root, parents, transforms).to_scale_rotation_translation();
    let (_, fore_rot, b) = from_root(fore, root, parents, transforms).to_scale_rotation_translation();
    let (_, _, c) = from_root(hand, root, parents, transforms).to_scale_rotation_translation();
    let Some((turn_upper, turn_fore)) = two_bone(a, b, c, to, Vec3::NEG_Y) else { return };
    let upper_rot = turn_upper * upper_rot;
    let fore_rot = turn_fore * turn_upper * fore_rot;
    for (e, rot) in [(upper, above.inverse() * upper_rot), (fore, upper_rot.inverse() * fore_rot), (hand, fore_rot.inverse() * to_rot)] {
        if let Ok(mut t) = transforms.get_mut(e) {
            t.rotation = rot.normalize();
        }
    }
}

/// Two-bone IK in one frame: a shoulder at `a`, elbow at `b` and wrist at
/// `c`; the turns (about `a`, then about the elbow once that's moved) that
/// bring the wrist to `target` — or as close as the arm reaches. The elbow
/// stays on the side it's bent to now (toward `pole` if the arm's straight).
pub(crate) fn two_bone(a: Vec3, b: Vec3, c: Vec3, target: Vec3, pole: Vec3) -> Option<(Quat, Quat)> {
    let upper = b - a;
    let lower = c - b;
    let (lab, lbc) = (upper.length(), lower.length());
    if lab < 1e-6 || lbc < 1e-6 {
        return None;
    }
    let to_target = target - a;
    let dir = to_target.try_normalize().or_else(|| (c - a).try_normalize())?;
    let dist = to_target.length().clamp((lab - lbc).abs() + 1e-5, (lab + lbc) * 0.9999);
    let bend = |v: Vec3| (v - dir * v.dot(dir)).try_normalize();
    let side = bend(upper).or_else(|| bend(pole)).unwrap_or_else(|| dir.any_orthonormal_vector());
    let along = (lab * lab - lbc * lbc + dist * dist) / (2.0 * dist);
    let elbow = a + dir * along + side * (lab * lab - along * along).max(0.0).sqrt();
    let turn_upper = Quat::from_rotation_arc(upper / lab, (elbow - a).try_normalize()?);
    let wrist = elbow + turn_upper * lower;
    let turn_fore = Quat::from_rotation_arc((wrist - elbow).try_normalize()?, (a + dir * dist - elbow).try_normalize()?);
    Some((turn_upper, turn_fore))
}

/// A soldier's held gun as placed now (its holder) and its grip — for its
/// muzzle and scope. `None` with no gun in hand.
pub(crate) fn held_gun<'a>(
    held: Option<&HeldWeapon>,
    transforms: &'a Query<&GlobalTransform>,
    grips: &'a WeaponGrips,
) -> Option<(&'a GlobalTransform, &'a Grip)> {
    let held = held?;
    let item = held.item.filter(|i| i.gun().is_some())?;
    Some((transforms.get(held.holder?).ok()?, grips.get(GripKey::of(item))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_bone_ik_puts_the_wrist_on_the_target_and_keeps_the_bend() {
        let (a, b, c) = (Vec3::ZERO, Vec3::new(0.0, -0.3, 0.05), Vec3::new(0.0, -0.55, 0.25));
        for target in [Vec3::new(0.2, -0.2, 0.3), Vec3::new(-0.1, 0.1, 0.35), Vec3::new(0.0, -0.5, 0.1)] {
            let (tu, tf) = two_bone(a, b, c, target, Vec3::NEG_Y).unwrap();
            let elbow = a + tu * (b - a);
            let wrist = elbow + tf * (tu * (c - b));
            assert!(wrist.distance(target) < 1e-4, "{wrist} vs {target}");
            // Bones keep their lengths.
            assert!(((elbow - a).length() - (b - a).length()).abs() < 1e-5);
            assert!(((wrist - elbow).length() - (c - b).length()).abs() < 1e-5);
            // The elbow's out to the same side of the shoulder–target line
            // as it was.
            let dir = (target - a).normalize();
            let off_line = |v: Vec3| v - dir * v.dot(dir);
            assert!(off_line(elbow - a).dot(off_line(b - a)) > 0.0);
        }
    }

    #[test]
    fn two_bone_ik_reaches_as_far_as_it_can() {
        let (a, b, c) = (Vec3::ZERO, Vec3::new(0.0, -0.3, 0.0), Vec3::new(0.0, -0.3, 0.3));
        let (tu, tf) = two_bone(a, b, c, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Y).unwrap();
        let elbow = a + tu * (b - a);
        let wrist = elbow + tf * (tu * (c - b));
        assert!((wrist.length() - 0.6).abs() < 1e-3 && wrist.z > 0.59, "{wrist}");
    }

    #[test]
    fn a_stab_winds_up_lunges_and_comes_back_smoothly() {
        assert_eq!(stab_curve(0.0), 0.0);
        assert!(stab_curve(0.1) < 0.0);
        assert_eq!(stab_curve(0.4), 1.0);
        assert!(stab_curve(1.0).abs() < 1e-6);
        // No jumps between its pieces.
        let mut last = stab_curve(0.0);
        for i in 1..=1000 {
            let now = stab_curve(i as f32 / 1000.0);
            assert!((now - last).abs() < 0.03, "jump at {i}");
            last = now;
        }
    }

    #[test]
    fn a_stab_lands_as_the_thrust_reaches_full_stretch() {
        let p = shared::melee::KNIFE_HIT_SECS / crate::net::STAB_ANIM_SECS;
        assert!(stab_curve(p) > 0.99, "{}", stab_curve(p));
        assert!(stab_curve(p - 0.05) < 0.95, "lands no later than it reaches");
    }

    #[test]
    fn the_built_in_grips_read() {
        let grips = WeaponGrips::from_ron(include_str!("../../data/weapon_grips.ron")).unwrap();
        let again = WeaponGrips::from_ron(&grips.to_ron()).unwrap();
        assert_eq!(again, grips.rounded());
    }
}
