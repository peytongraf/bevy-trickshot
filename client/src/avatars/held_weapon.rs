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
//! * **Where they're looking**, up or down (`PlayerPose::pitch`), is shown
//!   whatever the animation, as Call of Duty does: the spine bends by it
//!   ([`SPINE_PITCH_SHARE`]), so the chest — and the arms, the weapon and
//!   the head on it — point where they aim.
//! * **Everything else they carry** shows where it's carried — a gun on the
//!   back, the knife on the right thigh ([`Grip::stow_on`]) — and changing
//!   weapons swaps it over as Call of Duty does ([`Swap`], [`swap_motion`]):
//!   the right hand takes the weapon there and lets it go, then reaches for
//!   the next and brings it round, the left hand off it meanwhile.
//! * **A perk being drunk** is in the right hand as the drink plays (`drink`,
//!   `avatars::soldier`): the first-person bottle (`arms_drinking.glb`'s,
//!   fitted by its shape — [`BottleModel`]) in the perk's colour, coming off
//!   the hip as it grows in and going back to it as it shrinks away
//!   ([`bottle_size`]) — never just appearing in a raised hand.
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
use bevy::render::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use shared::lethal::LethalKind;
use shared::perks::Perk;
use shared::weapon::{SlotWeapon, WeaponId};

use super::SoldierAnimState;
use crate::weapon_drops::{model_path, DropModel};

pub(crate) struct HeldWeaponPlugin;

impl Plugin for HeldWeaponPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WeaponGrips::load())
            .add_observer(tag_held_weapon_parts)
            .add_observer(fit_bottle_model)
            .add_systems(Update, (attach_held_weapons, tint_remote_bottles))
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
    /// ...and how far the whole body lunges forward into it there (m) — what
    /// gets the tip out to [`shared::melee::KNIFE_REACH_M`].
    pub(crate) stab_lunge: f32,
    /// Its muzzle, in its own frame (m) — where a shot's tracer starts.
    pub(crate) muzzle: [f32; 3],
    /// Its scope's lens, in its own frame (m) — where the sniper glint
    /// shows. `None`: no scope, no glint.
    pub(crate) scope: Option<[f32; 3]>,
    /// Where it's carried while not in hand...
    pub(crate) stow_on: StowOn,
    /// ...its middle from that bone (m, in the bone's own axes)...
    pub(crate) stow_offset: [f32; 3],
    /// ...and its turn there (degrees, X then Y then Z).
    pub(crate) stow_turn_deg: [f32; 3],
}

/// Where a weapon's carried while it isn't in hand ([`Grip::stow_on`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum StowOn {
    /// Across the back (the chest bone).
    Back,
    /// On the outside of the right thigh.
    RightThigh,
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
            stab_reach: [0.0, 0.08, 0.7],
            stab_turn_deg: [0.0, -40.0, 0.0],
            stab_lean_deg: [-28.0, 32.0, 0.0],
            stab_lunge: 0.65,
            muzzle: [-0.5, 0.0, 0.0],
            scope: None,
            stow_on: StowOn::Back,
            stow_offset: [-0.0867, -0.0101, -0.1473],
            stow_turn_deg: [-139.31, 45.82, 47.02],
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
    /// Where a lethal or tactical sits in the right hand as it's thrown
    /// (its middle from the hand bone, m, in its axes — and its turn), for
    /// every kind.
    #[serde(default = "default_throw_hold")]
    pub(crate) throw_hold: HandTarget,
    /// Where a perk bottle sits in the right hand (its middle from the hand
    /// bone, m, in its axes — and its turn; neck up its +Y) — the drink clip's
    /// made with it there (`client/tools/soldier_prone_clips.py`, which
    /// prints it).
    #[serde(default = "default_drink_hold")]
    pub(crate) drink_hold: HandTarget,
    /// Which of each weapon's objects wear the Pack-a-Punch camo once it's
    /// packed (`pap`) — the WEAPON EDITOR's toggles.
    #[serde(default)]
    pub(crate) camo: CamoObjects,
}

/// The objects (glTF node names — [`crate::pap::object_name`]) of each
/// weapon's model that wear the Pack-a-Punch camo, everywhere it's drawn: in
/// our hands, other players' and on the ground. `None` keeps the built-in
/// ones (`pap::camo_parts`).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CamoObjects {
    pub(crate) sniper: Option<Vec<String>>,
    pub(crate) ak74: Option<Vec<String>>,
    pub(crate) raygun: Option<Vec<String>>,
    pub(crate) knife: Option<Vec<String>>,
}

impl CamoObjects {
    pub(crate) fn get(&self, key: GripKey) -> Option<&[String]> {
        match key {
            GripKey::Sniper => &self.sniper,
            GripKey::Ak74 => &self.ak74,
            GripKey::RayGun => &self.raygun,
            GripKey::Knife => &self.knife,
        }
        .as_deref()
    }

    pub(crate) fn get_mut(&mut self, key: GripKey) -> &mut Option<Vec<String>> {
        match key {
            GripKey::Sniper => &mut self.sniper,
            GripKey::Ak74 => &mut self.ak74,
            GripKey::RayGun => &mut self.raygun,
            GripKey::Knife => &mut self.knife,
        }
    }

    /// Whether `key`'s object `name` wears the camo — `built_in` if it's
    /// never been picked.
    pub(crate) fn wears(&self, key: GripKey, name: &str, built_in: bool) -> bool {
        self.get(key).map_or(built_in, |picked| picked.iter().any(|n| n == name))
    }
}

fn default_drink_hold() -> HandTarget {
    HandTarget {
        pos: [0.0, 0.08, 0.035],
        turn_deg: [14.35, -66.36, 177.25],
    }
}

fn default_throw_hold() -> HandTarget {
    // (In the palm: the fingers run along the hand bone's +Y.)
    HandTarget {
        pos: [0.0, 0.08, 0.035],
        turn_deg: [0.0; 3],
    }
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
            stab_lunge: r(g.stab_lunge, 1000.0),
            muzzle: m(g.muzzle),
            scope: g.scope.map(m),
            stow_on: g.stow_on,
            stow_offset: m(g.stow_offset),
            stow_turn_deg: d(g.stow_turn_deg),
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
            throw_hold: HandTarget {
                pos: m(self.throw_hold.pos),
                turn_deg: d(self.throw_hold.turn_deg),
            },
            drink_hold: HandTarget {
                pos: m(self.drink_hold.pos),
                turn_deg: d(self.drink_hold.turn_deg),
            },
            camo: self.camo.clone(),
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
    /// The skeleton's root — moved forward by a knife stab's lunge.
    pub(crate) root: Entity,
    /// Upper arm, forearm, hand.
    pub(crate) right: [Entity; 3],
    pub(crate) left: [Entity; 3],
    /// The left collarbone — lowered with the arm, down by the side.
    pub(crate) left_shoulder: Entity,
    /// The spine, bottom to top — bent by where they're looking up or down;
    /// the last (the chest) leans into a knife stab too.
    pub(crate) spine: [Entity; 3],
    /// The right thigh — where the knife's carried.
    pub(crate) right_thigh: Entity,
    /// Each hand's index, middle and little finger knuckles, left then right
    /// — which way the hand points, and which way it's turned (a mantle's
    /// hands, flat on the ledge).
    pub(crate) knuckles: [[Entity; 3]; 2],
    /// `soldier.glb`'s own rifle mesh — hidden on a soldier that holds a
    /// [`HeldWeapon`] (shown again by the WEAPON EDITOR, to line one up
    /// against).
    pub(crate) baked_gun: Option<Entity>,
}

/// `soldier.glb`'s arm bones, by glTF name: upper arm, forearm, hand.
pub(crate) const RIGHT_ARM_BONES: [&str; 3] = ["Bind_RightArm_035_036", "Bind_RightForeArm_036_037", "Bind_RightHand_037_038"];
pub(crate) const LEFT_ARM_BONES: [&str; 3] = ["Bind_LeftArm_016_017", "Bind_LeftForeArm_017_018", "Bind_LeftHand_018_019"];
/// ...and each hand's index, middle and little finger knuckles.
pub(crate) const LEFT_KNUCKLE_BONES: [&str; 3] =
    ["Bind_LeftHandIndex1_022_023", "Bind_LeftHandMiddle1_025_026", "Bind_LeftHandPinky1_031_032"];
pub(crate) const RIGHT_KNUCKLE_BONES: [&str; 3] =
    ["Bind_RightHandIndex1_041_042", "Bind_RightHandMiddle1_044_045", "Bind_RightHandPinky1_050_051"];
pub(crate) const LEFT_SHOULDER_BONE: &str = "Bind_LeftShoulder_015_016";
pub(crate) const SPINE_BONES: [&str; 3] = ["Bind_Spine_010_011", "Bind_Spine1_011_012", "Bind_Spine2_012_013"];
pub(crate) const RIGHT_THIGH_BONE: &str = "Bind_RightUpLeg_01_07";
pub(crate) const ROOT_BONE: &str = "Bind_Root_00_02";

/// How much of the look up or down each of [`SPINE_BONES`] takes — all of it
/// between them, so the chest (and the weapon) end up at the full angle.
pub(crate) const SPINE_PITCH_SHARE: [f32; 3] = [0.3, 0.35, 0.35];

/// How long (s) putting a weapon away takes, and getting one out.
pub(crate) const STOW_SECS: f32 = 0.4;
pub(crate) const DRAW_SECS: f32 = 0.4;

/// How high (m) the hand arcs on its way between the grip and where a weapon's
/// carried — up over the shoulder, round the hip.
const SWAP_LIFT_M: f32 = 0.12;

/// Where the weapon in hand is in being swapped ([`HeldWeapon`]).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum Swap {
    #[default]
    Holding,
    /// Putting it away, this far through (`0..1`)...
    Stowing(f32),
    /// ...or getting it out.
    Drawing(f32),
}

/// Mid-swap: how far the right hand's gone toward where the weapon in hand
/// is carried (`0` at its grip, `1` there), and how far the weapon's settled
/// there (`0` in hand, `1` carried). Putting it away, the hand reaches back,
/// lets it go as it gets there and comes back empty; getting it out, it
/// reaches, takes it and brings it round.
pub(crate) fn swap_motion(swap: Swap) -> (f32, f32) {
    let s = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    match swap {
        Swap::Holding => (0.0, 0.0),
        Swap::Stowing(t) => (
            if t < 0.7 { s(t / 0.55) } else { 1.0 - s((t - 0.7) / 0.3) },
            s((t - 0.45) / 0.25),
        ),
        Swap::Drawing(t) => (
            if t < 0.45 { s(t / 0.35) } else { 1.0 - s((t - 0.45) / 0.55) },
            1.0 - s((t - 0.3) / 0.25),
        ),
    }
}

/// One weapon a soldier carries, and its holder (under the right-hand bone,
/// placed in hand or where it's carried by [`pose_held_weapons`]).
struct Shown {
    item: SlotWeapon,
    pap: u8,
    holder: Entity,
}

/// On a soldier avatar: what it's holding and carrying (set every frame by
/// whoever drives it — `net` from the player's pose and lobby, or the WEAPON
/// EDITOR).
#[derive(Component)]
pub(crate) struct HeldWeapon {
    /// What should be in hand — a change swaps it in ([`Swap`]).
    pub(crate) item: Option<SlotWeapon>,
    /// Its Pack-a-Punch level (its camo).
    pub(crate) pap: u8,
    /// Everything they carry, at its Pack-a-Punch level — what isn't in hand
    /// shows where it's carried.
    pub(crate) carried: Vec<(SlotWeapon, u8)>,
    /// How far through putting the weapon in hand away, or getting `item`
    /// out, their own first-person view is (`PlayerPose::swap`) — the swap
    /// follows it exactly, so it ends as theirs does. `None` (no swap on
    /// their screen, or no word of it — a kill-cam ghost) plays the swap at
    /// its own pace ([`STOW_SECS`], [`DRAW_SECS`]).
    pub(crate) swap_progress: Option<shared::SwapProgress>,
    /// What's really in hand — `item`, once a swap's played out.
    in_hand: Option<SlotWeapon>,
    /// A lethal or tactical being thrown, and whether it's still in the hand
    /// (not yet let go) — `PlayerPose::throw`.
    pub(crate) throwing: Option<(LethalKind, bool)>,
    /// Its model in the right hand, while there's one.
    throwable: Option<(LethalKind, Entity)>,
    /// A perk being drunk, and how far through (`0..1`) — `PlayerPose::drink`.
    pub(crate) drinking: Option<(Perk, f32)>,
    /// The bottle's holder, once there's been one.
    bottle: Option<Entity>,
    swap: Swap,
    /// `in_hand` has been set going in (a new soldier starts with it in
    /// hand — no swap).
    started: bool,
    /// How much the hands should be on it now (`0..=1`) — `0` hands the arms
    /// back to the animation ([`ik_wanted`]); eased toward.
    pub(crate) ik_wanted: f32,
    ik: f32,
    /// Every weapon on show, in hand or carried.
    shown: Vec<Shown>,
    /// The holder of the weapon in hand, while it's held (not mid-swap) —
    /// where its muzzle and scope are.
    pub(crate) holder: Option<Entity>,
    /// Where the animation had the left hand this frame (before IK), in the
    /// weapon's frame — what the WEAPON EDITOR snaps the left hand to.
    pub(crate) anim_left: Option<HandTarget>,
    /// Where they're looking up (+) or down (radians, `PlayerPose::pitch`)...
    pub(crate) pitch: f32,
    /// ...and the look shown, eased toward it (so dying, which drops it to
    /// `0`, straightens up rather than snapping).
    shown_pitch: f32,
    /// Their player's stabbing (`PlayerPose::stabbing`) — with the knife in
    /// hand, its rising edge starts a thrust ([`stab_curve`]), which plays out
    /// in full.
    pub(crate) stabbing: bool,
    stab_was: bool,
    /// When (`Time::elapsed_secs`) the thrust playing started.
    stab_from: Option<f32>,
    /// Their player climbing onto a ledge (`PlayerPose::mantle`), and what
    /// that looks like now ([`mantle_look`]) — the hands on it.
    pub(crate) mantle: Option<(shared::MantleProgress, MantleLook)>,
    /// The arm bones as the animation left them, before IK changed them
    /// ([`unpose_held_weapons`]).
    animated: Vec<(Entity, Transform)>,
}

impl HeldWeapon {
    pub(crate) fn new(item: Option<SlotWeapon>) -> Self {
        Self {
            item,
            pap: 0,
            carried: Vec::new(),
            swap_progress: None,
            in_hand: None,
            throwing: None,
            throwable: None,
            drinking: None,
            bottle: None,
            swap: Swap::Holding,
            started: false,
            ik_wanted: 1.0,
            ik: 1.0,
            shown: Vec::new(),
            holder: None,
            anim_left: None,
            pitch: 0.0,
            shown_pitch: 0.0,
            stabbing: false,
            stab_was: false,
            stab_from: None,
            mantle: None,
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

/// A soldier mantling, `clock` seconds into it ([`HeldWeapon::mantle`]) —
/// in step with its player's own first-person climb
/// (`weapons::mantle_arms`): the weapon put away (onto the back, as for a
/// swap), the left hand reaching for the ledge meanwhile and the right once
/// it's let the weapon go, both on it as the body rises past them, letting
/// go as theirs do — and the weapon got out again as theirs comes back up.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct MantleLook {
    /// Putting the weapon away, this far through (`0..=1`)...
    pub(crate) stow: f32,
    /// ...or getting it out again, once they're up.
    pub(crate) draw: Option<f32>,
    /// How far (`0..1`) each hand (left, right) is on the ledge.
    pub(crate) grip: [f32; 2],
}

pub(crate) fn mantle_look(m: &shared::MantleProgress, look: &crate::weapons::MantleArmsLook) -> MantleLook {
    let s = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let (c, d) = (m.clock, m.duration.max(0.01));
    let stow_end = d * MANTLE_STOW_FRAC;
    let let_go = d * look.let_go_frac;
    let release = look.release_secs.max(1e-3);
    let raise_from = let_go + release * 0.5;
    let on = |from: f32, to: f32| {
        if c < let_go {
            s((c - from) / (to - from).max(1e-3))
        } else {
            1.0 - s((c - let_go) / release)
        }
    };
    MantleLook {
        stow: (c / stow_end).clamp(0.0, 1.0),
        draw: (c >= raise_from).then(|| ((c - raise_from) / look.raise_secs.max(1e-3)).clamp(0.0, 1.0)),
        grip: [on(0.0, d * look.reach_frac), on(stow_end, stow_end + d * look.reach_frac)],
    }
}

/// How far into a climb (of its duration) a soldier's weapon is away.
const MANTLE_STOW_FRAC: f32 = 0.3;

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

/// Keep a holder for every weapon each soldier carries — and for what's in
/// hand, or on its way there — and none for one they've let go of.
fn attach_held_weapons(
    mut soldiers: Query<(&mut HeldWeapon, &SoldierRig)>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    for (mut held, rig) in &mut soldiers {
        let held = &mut *held;
        let mut want = held.carried.clone();
        let in_hand_pap = |item: SlotWeapon| held.shown.iter().find(|s| s.item == item).map_or(0, |s| s.pap);
        for (item, pap) in [held.item.map(|i| (i, held.pap)), held.in_hand.map(|i| (i, in_hand_pap(i)))]
            .into_iter()
            .flatten()
        {
            if !want.iter().any(|(w, _)| *w == item) {
                want.push((item, pap));
            }
        }
        held.shown.retain(|s| {
            let keep = want.contains(&(s.item, s.pap));
            if !keep {
                if let Ok(mut e) = commands.get_entity(s.holder) {
                    e.despawn();
                }
            }
            keep
        });
        for (weapon, pap) in want {
            if held.shown.iter().any(|s| s.item == weapon) {
                continue;
            }
            held.shown.push(Shown {
                item: weapon,
                pap,
                holder: spawn_holder(weapon, pap, rig, &asset_server, &mut commands),
            });
        }
        // A perk bottle, once one's drunk (kept for the next, hidden).
        if held.drinking.is_some() && held.bottle.is_none() {
            held.bottle = Some(spawn_bottle(rig, &asset_server, &mut commands));
        }
        // What's being thrown, in the hand — kept until the next throw is
        // something else.
        if let Some((kind, _)) = held.throwing {
            if held.throwable.is_none_or(|(k, _)| k != kind) {
                if let Some((_, old)) = held.throwable.take() {
                    if let Ok(mut e) = commands.get_entity(old) {
                        e.despawn();
                    }
                }
                held.throwable = Some((kind, spawn_throwable(kind, rig, &asset_server, &mut commands)));
            }
        }
    }
}

/// A holder for a lethal or tactical in the right hand, and its model on it
/// — at its real size, its middle at the holder.
fn spawn_throwable(kind: LethalKind, rig: &SoldierRig, asset_server: &AssetServer, commands: &mut Commands) -> Entity {
    let holder = commands
        .spawn((HeldWeaponHolder, Transform::default(), Visibility::Hidden, ChildOf(rig.right[2])))
        .id();
    if kind == LethalKind::MonkeyBomb {
        commands.spawn((
            ChildOf(holder),
            HeldWeaponModel,
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(crate::monkey_bomb::MONKEY_MODEL))),
            crate::monkey_bomb::centered_transform(MONKEY_IN_HAND_HEIGHT),
        ));
    } else {
        let (path, length) = crate::mystery_box::lethal_model(kind);
        commands.spawn((
            ChildOf(holder),
            HeldWeaponModel,
            crate::mystery_box::LethalModel { length },
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path))),
            // (Shown once it's fitted — `mystery_box::fit_lethal_model`.)
            Visibility::Hidden,
        ));
    }
    holder
}

/// How tall (m) a monkey bomb is in a thrower's hand.
const MONKEY_IN_HAND_HEIGHT: f32 = 0.22;

/// How tall (m) a perk bottle is.
const BOTTLE_HEIGHT: f32 = 0.24;

/// A perk bottle's model ([`spawn_bottle`]): the drinking arms' scene, all
/// but the bottle hidden, fitted by its shape once it's in
/// ([`fit_bottle_model`]).
#[derive(Component)]
struct BottleModel;

/// A [`BottleModel`]'s glass — its own copies of the material, to tint the
/// colour of `perk` ([`tint_remote_bottles`]).
#[derive(Component)]
struct BottleGlass {
    materials: Vec<Handle<StandardMaterial>>,
    perk: Option<Perk>,
}

/// A holder for a perk bottle in the right hand, and the bottle on it.
fn spawn_bottle(rig: &SoldierRig, asset_server: &AssetServer, commands: &mut Commands) -> Entity {
    let holder = commands
        .spawn((HeldWeaponHolder, Transform::default(), Visibility::Hidden, ChildOf(rig.right[2])))
        .id();
    commands.spawn((
        ChildOf(holder),
        HeldWeaponModel,
        BottleModel,
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(crate::weapons::DRINK_ARMS_MODEL))),
        // (Shown once it's fitted.)
        Visibility::Hidden,
    ));
    holder
}

/// How big a perk bottle is (`0..1`) `t` of the way through its drink: grown
/// in over the first moments, as the hand takes it off the hip, and shrunk
/// away over the last, as it's put back there.
pub(crate) fn bottle_size(t: f32) -> f32 {
    let s = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    s(t / 0.07) * s((1.0 - t) / 0.07)
}

/// A perk bottle's scene is in: hide everything but the bottle and its
/// label (the arms), give its glass its own material, and stand it up by its
/// shape — neck (its narrower end) up its +Y, its middle at the origin,
/// [`BOTTLE_HEIGHT`] tall.
#[allow(clippy::too_many_arguments)]
fn fit_bottle_model(
    trigger: Trigger<SceneInstanceReady>,
    models: Query<(), With<BottleModel>>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    mesh_entities: Query<(&Mesh3d, Option<&SkinnedMesh>, &MeshMaterial3d<StandardMaterial>)>,
    meshes: Res<Assets<Mesh>>,
    bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let root = trigger.target();
    if !models.contains(root) {
        return;
    }
    // (A mesh is its node's child — one per primitive.)
    let named = |e: Entity, name: &str| {
        names.get(e).is_ok_and(|n| n.as_str() == name) || parents.get(e).is_ok_and(|p| names.get(p.parent()).is_ok_and(|n| n.as_str() == name))
    };
    let mut points = Vec::new();
    let mut glass = Vec::new();
    for entity in children.iter_descendants(root) {
        let Ok((mesh, skinned, material)) = mesh_entities.get(entity) else { continue };
        let is_glass = named(entity, crate::weapons::BOTTLE_NODE);
        if !is_glass && !named(entity, crate::weapons::BOTTLE_LABEL_NODE) {
            commands.entity(entity).insert(Visibility::Hidden);
            continue;
        }
        crate::weapon_drops::posed_points(root, entity, &mesh.0, skinned, &meshes, &bindposes, &parents, &transforms, &mut points);
        if is_glass {
            if let Some(own) = materials.get(&material.0).cloned() {
                let own = materials.add(own);
                commands.entity(entity).insert(MeshMaterial3d(own.clone()));
                glass.push(own);
            }
        }
    }
    if points.len() < 3 {
        warn!("perk bottle: no bottle meshes to fit");
        return;
    }
    let (center, [(along, lo, hi), (across, ..), _]) = crate::weapon_drops::principal_axes(&points);
    let length = hi - lo;
    if length <= 1e-6 {
        return;
    }
    // Its neck's the narrower end.
    let girth = |from: f32, to: f32| {
        let (sum, n) = points
            .iter()
            .map(|p| *p - center)
            .filter(|d| (from..to).contains(&d.dot(along)))
            .fold((0.0, 0), |(sum, n), d| (sum + (d - along * d.dot(along)).length(), n + 1));
        sum / n.max(1) as f32
    };
    let up = if girth(hi - 0.25 * length, hi + 1e-3) > girth(lo, lo + 0.25 * length) { -along } else { along };
    let right = (across - up * across.dot(up)).normalize_or(up.any_orthonormal_vector());
    let turn = Quat::from_mat3(&Mat3::from_cols(right, up, right.cross(up)).transpose());
    let scale = BOTTLE_HEIGHT / length;
    commands.entity(root).insert((
        Transform {
            translation: -(turn * (center * scale)),
            rotation: turn,
            scale: Vec3::splat(scale),
        },
        Visibility::Inherited,
        BottleGlass {
            materials: glass,
            perk: None,
        },
    ));
}

/// Keep each soldier's bottle the colour of the perk they're drinking — the
/// same glass as our own (`drink_arms::tint_bottle_glass`).
fn tint_remote_bottles(
    settings: Res<crate::weapons::DrinkArmsSettings>,
    soldiers: Query<&HeldWeapon>,
    children: Query<&Children>,
    mut glasses: Query<&mut BottleGlass>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for held in &soldiers {
        let (Some(holder), Some((perk, _))) = (held.bottle, held.drinking) else { continue };
        for model in children.iter_descendants(holder) {
            let Ok(mut glass) = glasses.get_mut(model) else { continue };
            if glass.perk == Some(perk) && !settings.is_changed() {
                continue;
            }
            glass.perk = Some(perk);
            for handle in &glass.materials {
                if let Some(mat) = materials.get_mut(handle) {
                    crate::weapons::tint_bottle_glass(mat, perk, &settings);
                }
            }
        }
    }
}

/// A holder for `weapon` under the right-hand bone, and its model on it.
fn spawn_holder(weapon: SlotWeapon, pap: u8, rig: &SoldierRig, asset_server: &AssetServer, commands: &mut Commands) -> Entity {
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
    holder
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
    cfg: Res<crate::MantleSettings>,
    mut soldiers: Query<(Entity, &mut HeldWeapon, &SoldierRig)>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility, With<HeldWeaponHolder>>,
) {
    let ease = 1.0 - (-time.delta_secs() * 10.0).exp();
    let pitch_ease = 1.0 - (-time.delta_secs() * 25.0).exp();
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
        let mut animated = Vec::with_capacity(11);
        for e in rig.right.into_iter().chain(rig.left).chain([rig.left_shoulder, rig.root]).chain(rig.spine) {
            if let Ok(t) = transforms.get(e) {
                animated.push((e, *t));
            }
        }

        // Where they're looking, up or down: the spine bends by it, about
        // the body's right — square to where they look, however the clip
        // twists the chest — each bone its share, about its own joint.
        held.shown_pitch += (held.pitch - held.shown_pitch) * pitch_ease;
        if held.shown_pitch.abs() > 1e-4 {
            for (bone, share) in rig.spine.into_iter().zip(SPINE_PITCH_SHARE) {
                let Ok(above) = parents.get(bone).map(|p| p.parent()) else { continue };
                let above = from_root(above, root, &parents, &transforms).to_scale_rotation_translation().1;
                let own = from_root(bone, root, &parents, &transforms).to_scale_rotation_translation().1;
                let turn = Quat::from_axis_angle(MODEL_RIGHT, held.shown_pitch * share);
                if let Ok(mut t) = transforms.get_mut(bone) {
                    t.rotation = (above.inverse() * turn * own).normalize();
                }
            }
        }

        // Metres per unit of the model's own frame (the avatar's scale).
        let per_m = 1.0 / transforms.get(root).map_or(1.0, |t| t.scale.x).max(1e-4);
        let w = held.ik;

        // What's in hand: swapped over as Call of Duty does when it changes.
        let dt = time.delta_secs();
        if !held.started {
            held.in_hand = held.item;
            held.started = true;
        }
        held.swap = match (held.swap_progress, held.swap) {
            // As their own screen has it: what's in hand going away...
            (Some(shared::SwapProgress::Stowing(t)), _) if held.in_hand.is_some() => Swap::Stowing(t.clamp(0.0, 1.0)),
            // ...and what's coming out.
            (Some(shared::SwapProgress::Drawing(t)), _) if held.item.is_some() => {
                held.in_hand = held.item;
                Swap::Drawing(t.clamp(0.0, 1.0))
            }
            // Otherwise at its own pace — or finishing the last sliver of one
            // their screen's just done.
            (_, swap) => match swap {
            Swap::Holding if held.in_hand != held.item => {
                if held.in_hand.is_some() {
                    Swap::Stowing(0.0)
                } else {
                    held.in_hand = held.item;
                    Swap::Drawing(0.0)
                }
            }
            Swap::Stowing(t) if t + dt / STOW_SECS >= 1.0 => {
                held.in_hand = held.item;
                if held.item.is_some() { Swap::Drawing(0.0) } else { Swap::Holding }
            }
            Swap::Stowing(t) => Swap::Stowing(t + dt / STOW_SECS),
            Swap::Drawing(t) if t + dt / DRAW_SECS >= 1.0 => Swap::Holding,
            Swap::Drawing(t) => Swap::Drawing(t + dt / DRAW_SECS),
            holding => holding,
            },
        };
        let (fetch, let_go) = swap_motion(held.swap);
        let in_hand = held
            .in_hand
            .and_then(|item| held.shown.iter().find(|s| s.item == item).map(|s| (item, s.holder)));
        held.holder = in_hand.filter(|_| held.swap == Swap::Holding).map(|(_, h)| h);

        if let Some((item, holder)) = in_hand {
            let grip = *grips.get(GripKey::of(item));

            // Mid-stab with the knife: the chest leans into it...
            let thrust = match (item, stab, held.swap) {
                (SlotWeapon::Knife, Some(p), Swap::Holding) if p < 1.0 => stab_curve(p),
                _ => 0.0,
            };
            if thrust != 0.0 && w > 1e-3 {
                // ...the whole body lunging forward into it...
                if let Ok(above) = parents.get(rig.root).map(|p| p.parent()) {
                    let above = from_root(above, root, &parents, &transforms);
                    let lunge = above.inverse().transform_vector3(MODEL_FORWARD * (grip.stab_lunge * thrust * w * per_m));
                    if let Ok(mut t) = transforms.get_mut(rig.root) {
                        t.translation += lunge;
                    }
                }
                let chest_bone = rig.spine[2];
                if let Ok(above) = parents.get(chest_bone).map(|p| p.parent()) {
                    let above = from_root(above, root, &parents, &transforms).to_scale_rotation_translation().1;
                    let chest = from_root(chest_bone, root, &parents, &transforms).to_scale_rotation_translation().1;
                    let lean = body_turn(grip.stab_lean_deg.map(|d| d * thrust * w));
                    if let Ok(mut t) = transforms.get_mut(chest_bone) {
                        t.rotation = (above.inverse() * lean * chest).normalize();
                    }
                }
            }

            // ...and the right hand's moved from the animation's — and
            // thrust; and mid-swap, off to where the weapon's carried, over
            // the shoulder or round the hip.
            let hand = from_root(rig.right[2], root, &parents, &transforms);
            let (_, rot, pos) = hand.to_scale_rotation_translation();
            let (mut to, mut to_rot) = (pos, rot);
            if let Some((shift, turn)) = grip.hand_move(thrust).filter(|_| w > 1e-3) {
                let shift = (MODEL_RIGHT * shift.x + Vec3::Y * shift.y + MODEL_FORWARD * shift.z) * per_m;
                to = pos + shift * w;
                to_rot = rot.slerp(turn * rot, w);
            }
            if fetch > 0.0 {
                // (The hand that would hold it right where it's carried.)
                let (_, carried_rot, carried_pos) = stowed(&grip, rig, root, per_m, &parents, &transforms)
                    .to_scale_rotation_translation();
                let hand_rot = carried_rot * grip.rotation().inverse();
                let hand_pos = carried_pos - hand_rot * (Vec3::from_array(grip.offset) * per_m);
                to = to.lerp(hand_pos, fetch) + Vec3::Y * (SWAP_LIFT_M * per_m * (fetch * std::f32::consts::PI).sin());
                to_rot = to_rot.slerp(hand_rot, fetch);
            }
            if to != pos || to_rot != rot {
                reach(rig.right, root, to, to_rot, &parents, &mut transforms);
            }

            // The weapon, in that hand — or settling where it's carried.
            let hand = from_root(rig.right[2], root, &parents, &transforms);
            let (_, hand_rot, hand_pos) = hand.to_scale_rotation_translation();
            let weapon_rot = hand_rot * grip.rotation();
            let in_grip = Affine3A::from_scale_rotation_translation(
                Vec3::splat(grip.scale.max(1e-3) * per_m),
                weapon_rot,
                hand_pos + hand_rot * (Vec3::from_array(grip.offset) * per_m),
            );
            let weapon = if let_go > 0.0 {
                blend(in_grip, stowed(&grip, rig, root, per_m, &parents, &transforms), let_go)
            } else {
                in_grip
            };
            if let Ok(mut t) = transforms.get_mut(holder) {
                *t = Transform::from_matrix((hand.inverse() * weapon).into());
            }

            // The left hand, on it — off it mid-swap.
            let left = from_root(rig.left[2], root, &parents, &transforms);
            let (_, left_rot, left_pos) = left.to_scale_rotation_translation();
            held.anim_left = Some(HandTarget {
                pos: in_grip.inverse().transform_point3(left_pos).to_array(),
                turn_deg: to_euler_deg(weapon_rot.inverse() * left_rot),
            });
            let on = w * (1.0 - fetch.max(let_go));
            if grip.left_arm_down {
                if w > 1e-3 {
                    let bones = [rig.left_shoulder, rig.left[0], rig.left[1], rig.left[2]];
                    for (e, q) in bones.into_iter().zip(grips.arm_down.turns()) {
                        if let Ok(mut t) = transforms.get_mut(e) {
                            t.rotation = t.rotation.slerp(Quat::from_array(q).normalize(), w);
                        }
                    }
                }
            } else if let Some(target) = grip.left_hand.filter(|_| on > 1e-3) {
                let to = left_pos.lerp(in_grip.transform_point3(Vec3::from_array(target.pos)), on);
                let to_rot = left_rot.slerp(weapon_rot * target.rotation(), on);
                reach(rig.left, root, to, to_rot, &parents, &mut transforms);
            }
        } else {
            held.anim_left = None;
        }

        // Everything else they carry, where it's carried.
        let hand = from_root(rig.right[2], root, &parents, &transforms).inverse();
        for shown in &held.shown {
            if Some(shown.item) == held.in_hand {
                continue;
            }
            let at = stowed(grips.get(GripKey::of(shown.item)), rig, root, per_m, &parents, &transforms);
            if let Ok(mut t) = transforms.get_mut(shown.holder) {
                *t = Transform::from_matrix((hand * at).into());
            }
        }
        // A perk being drunk: its bottle in the hand, grown in and shrunk
        // away at the hip.
        if let Some(holder) = held.bottle {
            let size = held.drinking.map_or(0.0, |(_, t)| bottle_size(t));
            if let Ok(mut vis) = visibility.get_mut(holder) {
                vis.set_if_neq(if size > 0.0 { Visibility::Inherited } else { Visibility::Hidden });
            }
            if size > 0.0 {
                let hand = from_root(rig.right[2], root, &parents, &transforms);
                let (_, rot, pos) = hand.to_scale_rotation_translation();
                let hold = grips.drink_hold;
                let at = Affine3A::from_scale_rotation_translation(
                    Vec3::splat(per_m * size),
                    rot * hold.rotation(),
                    pos + rot * (Vec3::from_array(hold.pos) * per_m),
                );
                if let Ok(mut t) = transforms.get_mut(holder) {
                    *t = Transform::from_matrix((hand.inverse() * at).into());
                }
            }
        }
        // What's being thrown, in the hand until it's let go.
        if let Some((_, holder)) = held.throwable {
            let in_hand = held.throwing.is_some_and(|(_, in_hand)| in_hand);
            if let Ok(mut vis) = visibility.get_mut(holder) {
                vis.set_if_neq(if in_hand { Visibility::Inherited } else { Visibility::Hidden });
            }
            if in_hand {
                let hand = from_root(rig.right[2], root, &parents, &transforms);
                let (_, rot, pos) = hand.to_scale_rotation_translation();
                let hold = grips.throw_hold;
                let at = Affine3A::from_scale_rotation_translation(
                    Vec3::splat(per_m),
                    rot * hold.rotation(),
                    pos + rot * (Vec3::from_array(hold.pos) * per_m),
                );
                if let Ok(mut t) = transforms.get_mut(holder) {
                    *t = Transform::from_matrix((hand.inverse() * at).into());
                }
            }
        }
        // Mantling: the hands on the ledge.
        if let Some((m, look)) = held.mantle {
            mantle_hands(&m, &look, &cfg.arms, rig, root, &parents, &mut transforms);
        }
        held.animated = animated;
    }
}

/// `e`'s transform in the world as of this frame — every local transform up
/// the hierarchy, before they've propagated.
fn world_now(e: Entity, parents: &Query<&ChildOf>, transforms: &Query<&mut Transform>) -> Affine3A {
    let mut m = Affine3A::IDENTITY;
    let mut at = e;
    loop {
        if let Ok(t) = transforms.get(at) {
            m = t.compute_affine() * m;
        }
        let Ok(parent) = parents.get(at) else { break };
        at = parent.parent();
    }
    m
}

/// A mantling soldier's hands on the ledge, as much as [`MantleLook::grip`]
/// says: each knuckle on its spot along the edge, the palm flat on top and
/// the fingers over it — as its player's own first-person hands are
/// (`weapons::mantle_arms`).
fn mantle_hands(
    m: &shared::MantleProgress,
    look: &MantleLook,
    arms: &crate::weapons::MantleArmsLook,
    rig: &SoldierRig,
    root: Entity,
    parents: &Query<&ChildOf>,
    transforms: &mut Query<&mut Transform>,
) {
    // (The ledge, in the soldier's own frame.)
    let to_root = world_now(root, parents, transforms).inverse();
    let facing = Quat::from_rotation_y(m.facing_yaw) * Vec3::NEG_Z;
    let right = facing.cross(Vec3::Y).normalize_or(Vec3::X);
    let tip = arms.fingers_down_deg.to_radians().clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
    let fingers = to_root.transform_vector3(facing * tip.cos() - Vec3::Y * tip.sin()).normalize_or(MODEL_FORWARD);
    let grab = Vec3::from_array(m.grab);
    for (side, (bones, knuckles)) in [rig.left, rig.right].into_iter().zip(rig.knuckles).enumerate() {
        let grip = look.grip[side];
        if grip <= 0.0 {
            continue;
        }
        // (Left hand: its thumb side points right, palm down; the right
        // hand's, left.)
        let outward = if side == 0 { -1.0 } else { 1.0 };
        let spot = grab + right * (outward * arms.hand_spread * 0.5) + facing * arms.grab_inset;
        let spot = to_root.transform_point3(spot);
        let want_across = to_root.transform_vector3(right * -outward).reject_from(fingers).normalize_or(MODEL_RIGHT);

        let (_, hand_rot, wrist) = from_root(bones[2], root, parents, transforms).to_scale_rotation_translation();
        let at = |e: Entity| from_root(e, root, parents, transforms).translation.into();
        let [index, middle, pinky]: [Vec3; 3] = knuckles.map(at);
        let Some(along) = (middle - wrist).try_normalize() else { continue };
        let Some(across) = (index - pinky).reject_from(along).try_normalize() else { continue };
        let basis = |a: Vec3, b: Vec3| Quat::from_mat3(&Mat3::from_cols(a, b, a.cross(b)));
        let flat = basis(fingers, want_across) * basis(along, across).inverse() * hand_rot;
        // (The wrist, so that the knuckles land on the spot.)
        let knuckle = hand_rot.inverse() * (middle - wrist);
        let to = wrist.lerp(spot - flat * knuckle, grip);
        reach(bones, root, to, hand_rot.slerp(flat, grip), parents, transforms);
    }
}

/// Where a weapon held by `grip` sits while it's carried, in `root`'s frame.
fn stowed(
    grip: &Grip,
    rig: &SoldierRig,
    root: Entity,
    per_m: f32,
    parents: &Query<&ChildOf>,
    transforms: &Query<&mut Transform>,
) -> Affine3A {
    let bone = match grip.stow_on {
        StowOn::Back => rig.spine[2],
        StowOn::RightThigh => rig.right_thigh,
    };
    let (_, rot, pos) = from_root(bone, root, parents, transforms).to_scale_rotation_translation();
    Affine3A::from_scale_rotation_translation(
        Vec3::splat(grip.scale.max(1e-3) * per_m),
        rot * euler_deg(grip.stow_turn_deg),
        pos + rot * (Vec3::from_array(grip.stow_offset) * per_m),
    )
}

/// `t` of the way from `a` to `b` (moved, turned and sized between them).
fn blend(a: Affine3A, b: Affine3A, t: f32) -> Affine3A {
    let (sa, ra, pa) = a.to_scale_rotation_translation();
    let (sb, rb, pb) = b.to_scale_rotation_translation();
    Affine3A::from_scale_rotation_translation(sa.lerp(sb, t), ra.slerp(rb, t), pa.lerp(pb, t))
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
    let item = held.in_hand.filter(|i| i.gun().is_some())?;
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
    fn a_swap_hands_the_weapon_over_where_its_carried_without_jumps() {
        assert_eq!(swap_motion(Swap::Holding), (0.0, 0.0));
        for phase in [Swap::Stowing as fn(f32) -> Swap, Swap::Drawing] {
            let mut last = swap_motion(phase(0.0));
            for i in 1..=1000 {
                let now = swap_motion(phase(i as f32 / 1000.0));
                assert!((now.0 - last.0).abs() < 0.02 && (now.1 - last.1).abs() < 0.02, "jump at {i}");
                // The weapon only moves between hand and carried with the
                // hand (nearly) there.
                if now.1 > 0.0 && now.1 < 1.0 {
                    assert!(now.0 > 0.85, "handed over away from the hand at {i}: {now:?}");
                }
                last = now;
            }
        }
        // Put away: starts in hand, ends carried, the hand back.
        assert_eq!(swap_motion(Swap::Stowing(0.0)), (0.0, 0.0));
        let end = swap_motion(Swap::Stowing(1.0));
        assert!(end.0.abs() < 1e-6 && (end.1 - 1.0).abs() < 1e-6);
        // Got out: starts carried, ends in hand.
        assert_eq!(swap_motion(Swap::Drawing(0.0)), (0.0, 1.0));
        let end = swap_motion(Swap::Drawing(1.0));
        assert!(end.0.abs() < 1e-6 && end.1.abs() < 1e-6);
    }

    #[test]
    fn a_bottle_grows_in_and_shrinks_away_only_at_the_hip() {
        assert_eq!(bottle_size(0.0), 0.0);
        assert!(bottle_size(1.0).abs() < 1e-6);
        // Whole the whole time it's up off the hip...
        for i in 0..=100 {
            let t = 0.07 + 0.86 * i as f32 / 100.0;
            assert!((bottle_size(t) - 1.0).abs() < 1e-6, "{t}");
        }
        // ...and never jumping.
        let mut last = 0.0;
        for i in 1..=1000 {
            let now = bottle_size(i as f32 / 1000.0);
            assert!((now - last).abs() < 0.05, "jump at {i}");
            last = now;
        }
    }

    #[test]
    fn a_mantle_puts_the_weapon_away_grabs_the_ledge_and_gets_it_back_as_ours_ends() {
        let arms = crate::weapons::MantleArmsLook::default();
        let at = |clock: f32| {
            mantle_look(&shared::MantleProgress { grab: [0.0; 3], facing_yaw: 0.0, clock, duration: 0.45 }, &arms)
        };
        let start = at(0.0);
        assert_eq!((start.stow, start.draw, start.grip), (0.0, None, [0.0, 0.0]));
        // The weapon's away before the right hand goes for the ledge...
        let stowed = at(0.45 * MANTLE_STOW_FRAC);
        assert!(stowed.stow >= 1.0 && stowed.grip[1] < 1e-3 && stowed.grip[0] > 0.5);
        // ...both hands on it mid-climb...
        assert!(at(0.3).grip.iter().all(|g| *g > 0.999));
        // ...and the weapon all the way out just as ours is back up.
        let end = 0.45 * arms.let_go_frac + arms.release_secs * 0.5 + arms.raise_secs;
        assert!(at(end).draw.is_some_and(|d| d > 0.999) && at(end).grip.iter().all(|g| *g < 1e-3));
        let mut last = 0.0;
        for i in 0..=100 {
            let look = at(end * i as f32 / 100.0);
            assert!(look.stow >= last);
            last = look.stow;
        }
    }

    #[test]
    fn the_built_in_grips_read() {
        let grips = WeaponGrips::from_ron(include_str!("../../data/weapon_grips.ron")).unwrap();
        let again = WeaponGrips::from_ron(&grips.to_ron()).unwrap();
        assert_eq!(again, grips.rounded());
    }
}
