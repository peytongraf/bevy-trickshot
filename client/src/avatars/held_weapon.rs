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
//!   a stab, dying — [`ik_wanted`]).
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
            muzzle: [-0.5, 0.0, 0.0],
            scope: None,
        }
    }
}

impl Grip {
    pub(crate) fn rotation(&self) -> Quat {
        euler_deg(self.turn_deg)
    }

    /// The right hand's turn ([`Grip::hand_turn_deg`]) in the model's frame.
    fn hand_turn(&self) -> Quat {
        let [pitch, yaw, roll] = self.hand_turn_deg.map(f32::to_radians);
        Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(MODEL_RIGHT, pitch) * Quat::from_axis_angle(MODEL_FORWARD, roll)
    }

    /// Whether the right hand's moved from the animation's at all.
    fn moves_hand(&self) -> bool {
        self.hand_shift != [0.0; 3] || self.hand_turn_deg != [0.0; 3]
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

    pub(crate) fn get_mut(&mut self, key: GripKey) -> &mut Grip {
        match key {
            GripKey::Sniper => &mut self.sniper,
            GripKey::Ak74 => &mut self.ak74,
            GripKey::RayGun => &mut self.raygun,
            GripKey::Knife => &mut self.knife,
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
            muzzle: m(g.muzzle),
            scope: g.scope.map(m),
        };
        Self {
            sniper: grip(&self.sniper),
            ak74: grip(&self.ak74),
            raygun: grip(&self.raygun),
            knife: grip(&self.knife),
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

/// The arm bones IK moves, and the rifle the model comes with — found by
/// `start_soldier_animation` once the scene's in.
#[derive(Component, Clone, Copy)]
pub(crate) struct SoldierRig {
    /// Upper arm, forearm, hand.
    pub(crate) right: [Entity; 3],
    pub(crate) left: [Entity; 3],
    /// `soldier.glb`'s own rifle mesh — hidden on a soldier that holds a
    /// [`HeldWeapon`] (shown again by the WEAPON EDITOR, to line one up
    /// against).
    pub(crate) baked_gun: Option<Entity>,
}

/// `soldier.glb`'s arm bones, by glTF name: upper arm, forearm, hand.
pub(crate) const RIGHT_ARM_BONES: [&str; 3] = ["Bind_RightArm_035_036", "Bind_RightForeArm_036_037", "Bind_RightHand_037_038"];
pub(crate) const LEFT_ARM_BONES: [&str; 3] = ["Bind_LeftArm_016_017", "Bind_LeftForeArm_017_018", "Bind_LeftHand_018_019"];

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
            animated: Vec::new(),
        }
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
    for (root, mut held, rig) in &mut soldiers {
        held.ik += (held.ik_wanted.clamp(0.0, 1.0) - held.ik) * ease;
        let (Some(item), Some(holder)) = (held.item, held.holder) else {
            held.anim_left = None;
            continue;
        };
        let grip = *grips.get(GripKey::of(item));
        // Metres per unit of the model's own frame (the avatar's scale).
        let per_m = 1.0 / transforms.get(root).map_or(1.0, |t| t.scale.x).max(1e-4);
        let w = held.ik;
        let mut animated = Vec::with_capacity(6);
        for e in rig.right.into_iter().chain(rig.left) {
            if let Ok(t) = transforms.get(e) {
                animated.push((e, *t));
            }
        }

        // The right hand, moved from the animation's.
        let hand = from_root(rig.right[2], root, &parents, &transforms);
        if w > 1e-3 && grip.moves_hand() {
            let (_, rot, pos) = hand.to_scale_rotation_translation();
            let [x, y, z] = grip.hand_shift;
            let shift = (MODEL_RIGHT * x + Vec3::Y * y + MODEL_FORWARD * z) * per_m;
            let to = pos + shift * w;
            let to_rot = rot.slerp(grip.hand_turn() * rot, w);
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
        if let Some(target) = grip.left_hand.filter(|_| w > 1e-3) {
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
    fn the_built_in_grips_read() {
        let grips = WeaponGrips::from_ron(include_str!("../../data/weapon_grips.ron")).unwrap();
        let again = WeaponGrips::from_ron(&grips.to_ron()).unwrap();
        assert_eq!(again, grips.rounded());
    }
}
