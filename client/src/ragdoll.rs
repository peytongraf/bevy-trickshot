//! `Zombies`: a zombie an explosion kills (a frag, PhD Flopper, a Bomb Shot,
//! a monkey bomb, a Ray Gun bolt's burst — [`shared::ZombieBlasted`]) is
//! thrown from it as a ragdoll instead of playing its death fall. Never a
//! hellhound or a boss (the server only sends it for a zombie).
//!
//! A crawler (`shared::zombies`' `CRAWLER_*`) has no death fall of its own:
//! whatever kills it, once the server says it's dead it goes limp where it
//! lies — a ragdoll with no throw, just a nudge so it slumps over. (Killed by
//! an explosion, it's thrown like any other: whichever comes first, the
//! blast's word or its death, it's one ragdoll, the blast kicking it if it's
//! already limp.)
//!
//! The ragdoll is eleven capsules — pelvis, chest, head, upper / lower arms
//! and legs — on the model's own bones, laid where they were the moment it
//! died, joined by limited joints (hinges for the elbows and knees) and
//! launched away from the blast. Each frame the bones are set to their
//! capsules (`drive_ragdolls`); everything below them (hands, feet, fingers,
//! jaw) keeps the pose it had, frozen with the paused animation.
//!
//! It all runs in a physics world of its own ([`RagdollContext`]) holding a
//! copy of every collider in the game world ([`ColliderMirror`]): the rest of
//! the client queries the default one — movement, shots, sound occlusion,
//! the shroom's x-ray — and must never see a body part. Purely cosmetic:
//! each client throws its own, so they needn't land the same everywhere.
//!
//! Nothing carries between games: the world, the mirrors and the parts are
//! all `StateScoped(InGame)`, and the parts go with their zombie's avatar
//! (gone when the server clears the corpse).

use bevy::ecs::system::SystemParam;
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_rapier3d::prelude::*;
use lightyear::prelude::{Interpolated, MessageReceiver, PeerId};
use shared::PlayerId;

use crate::net::RemoteAvatar;
use crate::util::rand01;
use crate::{AppState, ZombieAnimationPlayer, ZombieVisual};

/// Tunables — the debug panel's "Ragdolls (Zombies)" section. Read when a
/// zombie's thrown, so a change shows on the next one.
#[derive(Resource, Clone, Debug)]
pub(crate) struct RagdollSettings {
    /// Off: a blasted zombie just falls, like any other.
    pub(crate) enabled: bool,
    /// At most this many at once; any past it fall instead.
    pub(crate) max_ragdolls: u32,
    /// How fast (m/s) the whole body's thrown away from the blast, and up,
    /// right beside it...
    pub(crate) throw_speed: f32,
    pub(crate) lift_speed: f32,
    /// ...falling off to nothing `reach` (m) out — never less than
    /// `min_strength` of it, so anything a blast kills is thrown a little.
    pub(crate) reach: f32,
    pub(crate) min_strength: f32,
    /// On top of that, each part's flung straight out from the blast (m/s)
    /// — the nearer limbs harder, so it flails.
    pub(crate) limb_scatter: f32,
    /// Tumble (rad/s): head over heels away from the blast, plus a random
    /// spin either way.
    pub(crate) tumble: f32,
    pub(crate) spin_jitter: f32,
    /// How far (degrees) each joint can turn from where it was when it died:
    /// spine (pelvis → chest), neck, shoulders and hips each way; elbows and
    /// knees bend from straight to this.
    pub(crate) spine_limit_deg: f32,
    pub(crate) neck_limit_deg: f32,
    pub(crate) shoulder_limit_deg: f32,
    pub(crate) hip_limit_deg: f32,
    pub(crate) elbow_bend_deg: f32,
    pub(crate) knee_bend_deg: f32,
    /// A little muscle left in it: each joint's pull back to its death pose,
    /// and how much it resists turning at all.
    pub(crate) joint_stiffness: f32,
    pub(crate) joint_damping: f32,
    /// Slowing down through the air / ground (per s).
    pub(crate) linear_damping: f32,
    pub(crate) angular_damping: f32,
    pub(crate) friction: f32,
    pub(crate) bounce: f32,
    pub(crate) gravity_scale: f32,
    /// How thick (×) every capsule is, and how heavy (×) every part.
    pub(crate) thickness: f32,
    pub(crate) mass_scale: f32,
    /// A crawler going limp where it dies (off: it stops dead, its crawl
    /// frozen), and the push (m/s, a random way) it slumps over with.
    pub(crate) crawlers: bool,
    pub(crate) crawler_nudge: f32,
}

impl Default for RagdollSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_ragdolls: 12,
            throw_speed: 7.0,
            lift_speed: 5.0,
            reach: 6.0,
            min_strength: 0.35,
            limb_scatter: 2.0,
            tumble: 5.0,
            spin_jitter: 3.0,
            spine_limit_deg: 30.0,
            neck_limit_deg: 40.0,
            shoulder_limit_deg: 75.0,
            hip_limit_deg: 60.0,
            elbow_bend_deg: 140.0,
            knee_bend_deg: 140.0,
            joint_stiffness: 1.0,
            joint_damping: 0.6,
            linear_damping: 0.1,
            angular_damping: 1.0,
            friction: 0.8,
            bounce: 0.05,
            gravity_scale: 1.0,
            thickness: 1.0,
            mass_scale: 1.0,
            crawlers: true,
            crawler_nudge: 0.8,
        }
    }
}

/// The ragdolls' own physics world (no `DefaultRapierContext`, so nothing
/// else's `ReadRapierContext` ever finds it).
#[derive(Component)]
struct RagdollContext;

/// On a game-world collider: its copy in the [`RagdollContext`].
#[derive(Component)]
struct MirroredCollider(Entity);

/// A copy, in the [`RagdollContext`], of game-world collider `src`.
#[derive(Component)]
struct ColliderMirror {
    src: Entity,
}

/// One of a ragdoll's capsules, thrown for the zombie avatar `avatar`.
#[derive(Component)]
struct RagdollPart {
    avatar: Entity,
}

/// On a zombie avatar that's a ragdoll: which bone follows which part.
#[derive(Component)]
pub(crate) struct Ragdoll {
    /// Parents before children, so each bone's set after the one it hangs
    /// off.
    bones: Vec<DrivenBone>,
    /// The avatar's root follows the pelvis about (by how far it's gone
    /// from where it started), so its culling box goes with the body.
    pelvis: Entity,
    pelvis_start: Vec3,
    root_start: Vec3,
}

struct DrivenBone {
    bone: Entity,
    part: Entity,
    /// The bone's world scale (the model's), which its capsule hasn't got.
    scale: Vec3,
}

// `models/characters/zombie.glb`'s bones.
const PELVIS: &str = "Base HumanPelvis_01";
const SPINE2: &str = "Base HumanSpine2_00";
const NECK: &str = "Base HumanNeck_08";
const HEAD: &str = "Base HumanHead_09";
const R_UPPER_ARM: &str = "Base HumanRArmUpperarm_016";
const R_FOREARM: &str = "Base HumanRArmForearm_017";
const R_PALM: &str = "Base HumanRArmPalm_018";
const L_UPPER_ARM: &str = "Base HumanLArmUpperarm_034";
const L_FOREARM: &str = "Base HumanLArmForearm_035";
const L_PALM: &str = "Base HumanLArmPalm_036";
const R_THIGH: &str = "Base HumanRThigh_02";
const R_CALF: &str = "Base HumanRCalf_03";
const R_FOOT: &str = "Base HumanRFoot_04";
const L_THIGH: &str = "Base HumanLLegThigh_051";
const L_CALF: &str = "Base HumanLLegCalf_052";
const L_FOOT: &str = "Base HumanLLegFoot_053";

/// Where a part's capsule reaches to, from its bone.
#[derive(Clone, Copy)]
enum Tip {
    /// That bone (the next one down the limb).
    Bone(&'static str),
    /// That bone, then on past it by this much (m) — a forearm's hand.
    Past(&'static str, f32),
    /// This far (m) on from its own bone, straight away from that one — the
    /// head, away from the neck.
    AwayFrom(&'static str, f32),
}

#[derive(Clone, Copy)]
enum Joint {
    /// The pelvis: hung off nothing.
    None,
    /// Turns every way, as far as the setting says.
    Ball(fn(&RagdollSettings) -> f32),
    /// Bends one way only, from straight to the setting's angle: an elbow
    /// (the forearm comes forward) or a knee (the calf goes back).
    Hinge { max_bend: fn(&RagdollSettings) -> f32, forward: bool },
}

struct PartDef {
    bone: &'static str,
    tip: Tip,
    /// The part it hangs off (an index into [`PARTS`], always an earlier one).
    parent: Option<usize>,
    joint: Joint,
    /// Capsule radius (m) and mass (kg), before the thickness / mass scales.
    radius: f32,
    mass: f32,
}

const PARTS: [PartDef; 11] = [
    PartDef { bone: PELVIS, tip: Tip::Bone(SPINE2), parent: None, joint: Joint::None, radius: 0.15, mass: 14.0 },
    PartDef {
        bone: SPINE2,
        tip: Tip::Bone(NECK),
        parent: Some(0),
        joint: Joint::Ball(|s| s.spine_limit_deg),
        radius: 0.16,
        mass: 18.0,
    },
    PartDef {
        bone: HEAD,
        tip: Tip::AwayFrom(NECK, 0.2),
        parent: Some(1),
        joint: Joint::Ball(|s| s.neck_limit_deg),
        radius: 0.11,
        mass: 5.0,
    },
    PartDef {
        bone: R_UPPER_ARM,
        tip: Tip::Bone(R_FOREARM),
        parent: Some(1),
        joint: Joint::Ball(|s| s.shoulder_limit_deg),
        radius: 0.06,
        mass: 2.5,
    },
    PartDef {
        bone: R_FOREARM,
        tip: Tip::Past(R_PALM, 0.12),
        parent: Some(3),
        joint: Joint::Hinge { max_bend: |s| s.elbow_bend_deg, forward: true },
        radius: 0.05,
        mass: 1.8,
    },
    PartDef {
        bone: L_UPPER_ARM,
        tip: Tip::Bone(L_FOREARM),
        parent: Some(1),
        joint: Joint::Ball(|s| s.shoulder_limit_deg),
        radius: 0.06,
        mass: 2.5,
    },
    PartDef {
        bone: L_FOREARM,
        tip: Tip::Past(L_PALM, 0.12),
        parent: Some(5),
        joint: Joint::Hinge { max_bend: |s| s.elbow_bend_deg, forward: true },
        radius: 0.05,
        mass: 1.8,
    },
    PartDef {
        bone: R_THIGH,
        tip: Tip::Bone(R_CALF),
        parent: Some(0),
        joint: Joint::Ball(|s| s.hip_limit_deg),
        radius: 0.08,
        mass: 8.0,
    },
    PartDef {
        bone: R_CALF,
        tip: Tip::Bone(R_FOOT),
        parent: Some(7),
        joint: Joint::Hinge { max_bend: |s| s.knee_bend_deg, forward: false },
        radius: 0.06,
        mass: 4.0,
    },
    PartDef {
        bone: L_THIGH,
        tip: Tip::Bone(L_CALF),
        parent: Some(0),
        joint: Joint::Ball(|s| s.hip_limit_deg),
        radius: 0.08,
        mass: 8.0,
    },
    PartDef {
        bone: L_CALF,
        tip: Tip::Bone(L_FOOT),
        parent: Some(9),
        joint: Joint::Hinge { max_bend: |s| s.knee_bend_deg, forward: false },
        radius: 0.06,
        mass: 4.0,
    },
];

/// What a ragdoll's parts are: in the ragdoll world only, hitting the map's
/// copy (group 1, every collider's default) but never one another.
fn part_groups() -> CollisionGroups {
    CollisionGroups::new(Group::GROUP_2, Group::GROUP_1)
}

pub(crate) struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RagdollSettings>()
            .add_systems(
                Update,
                (start_ragdolls, mirror_colliders, clear_orphaned_parts).run_if(in_state(AppState::InGame)),
            )
            // After the physics has moved the parts and the (paused)
            // animation has posed the rest, before the bones' world
            // transforms are worked out.
            .add_systems(
                PostUpdate,
                drive_ragdolls
                    .after(PhysicsSet::Writeback)
                    .after(bevy::app::Animation)
                    .before(TransformSystem::TransformPropagate)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn peer_of(src: Entity, ids: &Query<&PlayerId>, interp: &Query<&Interpolated>) -> Option<PeerId> {
    ids.get(src)
        .ok()
        .or_else(|| interp.get(src).ok().and_then(|i| ids.get(i.confirmed_entity).ok()))
        .map(|id| id.0)
}

/// Server → us: an explosion killed a zombie — throw it. And a crawler the
/// server says is dead: let it go limp.
#[allow(clippy::too_many_arguments)]
fn start_ragdolls(
    mut commands: Commands,
    settings: Res<RagdollSettings>,
    mut receivers: Query<&mut MessageReceiver<shared::ZombieBlasted>>,
    avatars: Query<
        (Entity, &RemoteAvatar, &ZombieAnimationPlayer, &Transform, &crate::ZombieMotion),
        (With<ZombieVisual>, Without<Ragdoll>),
    >,
    (active, limp, mut velocities): (
        Query<(), With<Ragdoll>>,
        Query<(&RemoteAvatar, &Ragdoll)>,
        Query<&mut Velocity, With<RagdollPart>>,
    ),
    (ids, interp, poses): (Query<&PlayerId>, Query<&Interpolated>, Query<&shared::PlayerPose>),
    (children, names, globals): (Query<&Children>, Query<&Name>, Query<&GlobalTransform>),
    mut players: Query<&mut AnimationPlayer>,
    contexts: Query<Entity, With<RagdollContext>>,
) {
    let blasts: Vec<shared::ZombieBlasted> = receivers.iter_mut().flat_map(|mut rx| rx.receive().collect::<Vec<_>>()).collect();
    if !settings.enabled {
        return;
    }
    // What to throw: (avatar, the blast it's thrown from — `None` a dead
    // crawler, just going limp).
    let mut jobs: Vec<(Entity, Option<Vec3>, PeerId)> = Vec::new();
    for blast in &blasts {
        let from = Vec3::from_array(blast.from);
        if let Some((avatar, ..)) =
            avatars.iter().find(|(_, remote, ..)| peer_of(remote.src, &ids, &interp) == Some(blast.zombie))
        {
            jobs.push((avatar, Some(from), blast.zombie));
        } else if let Some((_, ragdoll)) =
            limp.iter().find(|(remote, _)| peer_of(remote.src, &ids, &interp) == Some(blast.zombie))
        {
            // Already gone limp (its death got here first): the blast kicks it.
            for bone in &ragdoll.bones {
                if let Ok(mut v) = velocities.get_mut(bone.part) {
                    let at = ragdoll.pelvis_start;
                    let away = Vec3::new(at.x - from.x, 0.0, at.z - from.z).normalize_or(Vec3::X);
                    let strength =
                        (1.0 - at.distance(from) / settings.reach.max(0.1)).clamp(settings.min_strength, 1.0);
                    v.linvel += (away * settings.throw_speed + Vec3::Y * settings.lift_speed) * strength;
                }
            }
        }
    }
    if settings.crawlers {
        for (avatar, remote, _, _, motion) in &avatars {
            // (Only on the server's word — a death foreseen here can still
            // be taken back, and a ragdoll can't get up again.)
            let dead = poses.get(remote.src).is_ok_and(|p| !p.alive);
            if dead && motion.crawler_dead() && !jobs.iter().any(|(a, ..)| *a == avatar) {
                let Some(peer) = peer_of(remote.src, &ids, &interp) else { continue };
                jobs.push((avatar, None, peer));
            }
        }
    }
    if jobs.is_empty() {
        return;
    }
    let mut count = active.iter().count() as u32;
    let mut context = contexts.iter().next();
    for (avatar, from, zombie) in jobs {
        if count >= settings.max_ragdolls {
            break;
        }
        let Ok((avatar, _, anim_player, root, _)) = avatars.get(avatar) else {
            continue;
        };
        // Its bones, by name (its scene's in: it has an animation player).
        let mut bones = std::collections::HashMap::new();
        for e in children.iter_descendants(avatar) {
            if let Ok(name) = names.get(e) {
                bones.insert(name.as_str(), e);
            }
        }
        let pose = |name: &str| -> Option<(Entity, Vec3, Quat, Vec3)> {
            let e = *bones.get(name)?;
            let (scale, rot, pos) = globals.get(e).ok()?.to_scale_rotation_translation();
            Some((e, pos, rot, scale))
        };
        let Some(posed) = PARTS.iter().map(|p| pose(p.bone)).collect::<Option<Vec<_>>>() else {
            warn!("ragdoll: zombie avatar {avatar:?} is missing a bone");
            continue;
        };
        let ctx = *context.get_or_insert_with(|| {
            commands
                .spawn((
                    Name::new("Ragdoll physics"),
                    RagdollContext,
                    RapierContextSimulation::default(),
                    RapierConfiguration::new(1.0),
                    StateScoped(AppState::InGame),
                ))
                .id()
        });
        if let Ok(mut player) = players.get_mut(anim_player.0) {
            player.pause_all();
        }

        let pelvis_pos = posed[0].1;
        // Its forward, from its hips (`R`/`L` are its own right and left).
        let right = posed[7].1 - posed[9].1;
        let forward = Vec3::Y.cross(right).normalize_or(root.rotation * Vec3::Z);
        let seed = zombie.to_bits();
        let seed = (seed ^ (seed >> 32)) as u32;
        let jitter = |i: u32| rand01(seed.wrapping_add(i.wrapping_mul(0x9e37_79b9))) * 2.0 - 1.0;
        // Thrown from the blast — or, a crawler going limp, only nudged over
        // (from a spot just beside it: the parts barely flung apart).
        let (from, strength, base_vel, base_spin) = match from {
            Some(from) => {
                let away = Vec3::new(pelvis_pos.x - from.x, 0.0, pelvis_pos.z - from.z).normalize_or(-forward);
                let strength =
                    (1.0 - pelvis_pos.distance(from) / settings.reach.max(0.1)).clamp(settings.min_strength, 1.0);
                (
                    from,
                    strength,
                    (away * settings.throw_speed + Vec3::Y * settings.lift_speed) * strength,
                    Vec3::Y.cross(away) * settings.tumble * strength
                        + Vec3::new(jitter(1), jitter(2), jitter(3)) * settings.spin_jitter * strength,
                )
            }
            None => {
                let away = Vec3::new(jitter(4), 0.0, jitter(5)).normalize_or(Vec3::X);
                (pelvis_pos - away * 50.0, 0.0, away * settings.crawler_nudge, Vec3::ZERO)
            }
        };

        let mut parts: Vec<Entity> = Vec::with_capacity(PARTS.len());
        let mut driven = Vec::with_capacity(PARTS.len());
        for (i, (def, &(bone, pos, rot, scale))) in PARTS.iter().zip(&posed).enumerate() {
            let tip = match def.tip {
                Tip::Bone(name) => pose(name).map(|p| p.1),
                Tip::Past(name, by) => pose(name).map(|p| p.1 + (p.1 - pos).normalize_or_zero() * by),
                Tip::AwayFrom(name, by) => pose(name).map(|p| pos + (pos - p.1).normalize_or(Vec3::Y) * by),
            }
            .unwrap_or(pos + Vec3::Y * 0.1);
            let radius = def.radius * settings.thickness.max(0.05);
            let out = (pos - from).normalize_or(Vec3::Y);
            let near = (1.0 - pos.distance(from) / settings.reach.max(0.1)).clamp(0.0, 1.0);
            let linvel = base_vel + out * settings.limb_scatter * (strength + near) * 0.5;
            let mut part = commands.spawn((
                RagdollPart { avatar },
                StateScoped(AppState::InGame),
                RapierContextEntityLink(ctx),
                RigidBody::Dynamic,
                Collider::capsule(Vec3::ZERO, rot.inverse() * (tip - pos), radius),
                ColliderMassProperties::Mass(def.mass * settings.mass_scale.max(0.01)),
                part_groups(),
                Friction::coefficient(settings.friction),
                Restitution::coefficient(settings.bounce),
                Damping {
                    linear_damping: settings.linear_damping,
                    angular_damping: settings.angular_damping,
                },
                GravityScale(settings.gravity_scale),
                Velocity { linvel, angvel: base_spin },
                Ccd::enabled(),
                (Transform::from_translation(pos).with_rotation(rot), GlobalTransform::from(Transform::from_translation(pos).with_rotation(rot))),
            ));
            if let Some(p) = def.parent {
                let (_, parent_pos, parent_rot, _) = posed[p];
                part.insert(ImpulseJoint::new(parts[p], joint(def, &settings, pos, rot, parent_pos, parent_rot, tip, forward)));
            }
            parts.push(part.id());
            driven.push(DrivenBone { bone, part: parts[i], scale });
        }
        commands.entity(avatar).insert(Ragdoll {
            bones: driven,
            pelvis: parts[0],
            pelvis_start: pelvis_pos,
            root_start: root.translation,
        });
        count += 1;
    }
}

/// The joint from a part (at `pos`, turned `rot`, reaching to `tip`) to its
/// parent — limited around the pose it died in, which is where both joint
/// frames line up.
#[allow(clippy::too_many_arguments)]
fn joint(
    def: &PartDef,
    settings: &RagdollSettings,
    pos: Vec3,
    rot: Quat,
    parent_pos: Vec3,
    parent_rot: Quat,
    tip: Vec3,
    forward: Vec3,
) -> TypedJoint {
    let (stiffness, damping) = (settings.joint_stiffness, settings.joint_damping);
    let (basis, locked, limits): (Quat, JointAxesMask, Vec<(JointAxis, [f32; 2])>) = match def.joint {
        Joint::None => unreachable!("the pelvis has no parent"),
        Joint::Ball(limit) => {
            let l = limit(settings).to_radians();
            (
                parent_rot,
                JointAxesMask::LIN_AXES,
                vec![(JointAxis::AngX, [-l, l]), (JointAxis::AngY, [-l, l]), (JointAxis::AngZ, [-l, l])],
            )
        }
        Joint::Hinge { max_bend, forward: bends_forward } => {
            let upper = (pos - parent_pos).normalize_or(Vec3::NEG_Y);
            let lower = (tip - pos).normalize_or(Vec3::NEG_Y);
            let bend = upper.angle_between(lower);
            // Turning the lower part the positive way about the axis bends
            // it further: about upper × lower if it's bent at all, or (dead
            // straight) toward the way that limb bends.
            let bent = upper.cross(lower);
            let axis = if bent.length() > 0.15 {
                bent.normalize()
            } else {
                let toward = if bends_forward { forward } else { -forward };
                lower.cross(toward).normalize_or(Vec3::X)
            };
            let max = max_bend(settings).to_radians();
            (
                Quat::from_rotation_arc(Vec3::X, axis),
                JointAxesMask::LIN_AXES | JointAxesMask::ANG_Y | JointAxesMask::ANG_Z,
                vec![(JointAxis::AngX, [-bend - 0.05, (max - bend).max(0.0)])],
            )
        }
    };
    let mut builder = GenericJointBuilder::new(locked)
        .local_anchor1(parent_rot.inverse() * (pos - parent_pos))
        .local_anchor2(Vec3::ZERO)
        .local_basis1(parent_rot.inverse() * basis)
        .local_basis2(rot.inverse() * basis);
    for (axis, [lo, hi]) in limits {
        builder = builder.limits(axis, [lo, hi]).motor_position(axis, 0.0, stiffness, damping);
    }
    TypedJoint::GenericJoint(builder.build())
}

/// Copy every game-world collider into the ragdoll world (once there is
/// one) — the map and whatever's placed on it — keep the copies where their
/// originals are, and drop a copy whose original's gone or switched off (a
/// door opened).
#[allow(clippy::type_complexity)]
fn mirror_colliders(
    mut commands: Commands,
    contexts: Query<Entity, With<RagdollContext>>,
    fresh: Query<
        (Entity, &Collider, &GlobalTransform, &RapierContextEntityLink),
        (
            Without<MirroredCollider>,
            Without<ColliderMirror>,
            Without<RagdollPart>,
            Without<Sensor>,
            Without<ColliderDisabled>,
            Without<RigidBody>,
        ),
    >,
    moved: Query<
        (&MirroredCollider, &Collider, &GlobalTransform),
        (Without<ColliderMirror>, Or<(Changed<GlobalTransform>, Changed<Collider>)>),
    >,
    mut mirrors: Query<(Entity, &ColliderMirror, &mut Collider, &mut Transform, &mut GlobalTransform), Without<MirroredCollider>>,
    gone: Query<(), (With<Collider>, Without<Sensor>, Without<ColliderDisabled>)>,
) {
    let Some(ctx) = contexts.iter().next() else {
        return;
    };
    for (src, collider, global, link) in &fresh {
        if link.0 == ctx {
            continue;
        }
        let mirror = commands
            .spawn((
                ColliderMirror { src },
                StateScoped(AppState::InGame),
                RapierContextEntityLink(ctx),
                collider.clone(),
                global.compute_transform(),
                *global,
            ))
            .id();
        commands.entity(src).insert(MirroredCollider(mirror));
    }
    for (link, collider, global) in &moved {
        if let Ok((_, _, mut c, mut t, mut g)) = mirrors.get_mut(link.0) {
            *c = collider.clone();
            *t = global.compute_transform();
            *g = *global;
        }
    }
    for (mirror, m, ..) in &mirrors {
        if gone.get(m.src).is_err() {
            commands.entity(mirror).try_despawn();
            if let Ok(mut src) = commands.get_entity(m.src) {
                src.remove::<MirroredCollider>();
            }
        }
    }
}

/// A ragdoll's parts go with its zombie (the corpse cleared away).
fn clear_orphaned_parts(mut commands: Commands, parts: Query<(Entity, &RagdollPart)>, ragdolls: Query<(), With<Ragdoll>>) {
    for (part, p) in &parts {
        if ragdolls.get(p.avatar).is_err() {
            commands.entity(part).try_despawn();
        }
    }
}

/// Set each ragdoll's bones to their parts (in the frame of the bone each
/// hangs off, as it stands this frame), and its root after its pelvis.
fn drive_ragdolls(ragdolls: Query<(Entity, &Ragdoll)>, parents: Query<&ChildOf>, mut transforms: Query<&mut Transform>) {
    for (avatar, ragdoll) in &ragdolls {
        let Ok(pelvis) = transforms.get(ragdoll.pelvis).map(|t| t.translation) else {
            continue;
        };
        if let Ok(mut root) = transforms.get_mut(avatar) {
            root.translation = ragdoll.root_start + (pelvis - ragdoll.pelvis_start);
        }
        for driven in &ragdoll.bones {
            let Ok(part) = transforms.get(driven.part).copied() else {
                continue;
            };
            let Some(parent) = parents.get(driven.bone).ok().map(|c| c.parent()) else {
                continue;
            };
            let Some(parent_world) = world_of(parent, avatar, &parents, &transforms) else {
                continue;
            };
            let want = Affine3A::from_scale_rotation_translation(driven.scale, part.rotation, part.translation);
            if let Ok(mut bone) = transforms.get_mut(driven.bone) {
                *bone = Transform::from_matrix(Mat4::from(parent_world.inverse() * want));
            }
        }
    }
}

/// `entity`'s world transform, from the local ones up to `root` (a
/// top-level avatar, whose own is its world one).
fn world_of(entity: Entity, root: Entity, parents: &Query<&ChildOf>, transforms: &Query<&mut Transform>) -> Option<Affine3A> {
    let mut world = Affine3A::IDENTITY;
    let mut at = entity;
    loop {
        world = transforms.get(at).ok()?.compute_affine() * world;
        if at == root {
            return Some(world);
        }
        at = parents.get(at).ok()?.parent();
    }
}

/// The debug panel's "Ragdolls (Zombies)" section.
#[derive(SystemParam)]
pub(crate) struct RagdollDebug<'w, 's> {
    settings: ResMut<'w, RagdollSettings>,
    active: Query<'w, 's, (), With<Ragdoll>>,
}

impl RagdollDebug<'_, '_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "A zombie an explosion kills is thrown as a ragdoll. Changes show on the next one \
             (the lobby's bomb test makes every kill a blast).",
        );
        ui.label(format!("Ragdolls now: {}", self.active.iter().count()));
        let s = &mut *self.settings;
        ui.checkbox(&mut s.enabled, "On");
        ui.add(egui::Slider::new(&mut s.max_ragdolls, 1u32..=40).text("at once, max"));
        ui.separator();
        ui.add(egui::Slider::new(&mut s.throw_speed, 0.0f32..=30.0).text("throw, away (m/s)"));
        ui.add(egui::Slider::new(&mut s.lift_speed, 0.0f32..=20.0).text("throw, up (m/s)"));
        ui.add(egui::Slider::new(&mut s.reach, 0.5f32..=20.0).text("full throw falls off by (m)"));
        ui.add(egui::Slider::new(&mut s.min_strength, 0.0f32..=1.0).text("weakest throw (× full)"));
        ui.add(egui::Slider::new(&mut s.limb_scatter, 0.0f32..=15.0).text("limbs flung out (m/s)"));
        ui.add(egui::Slider::new(&mut s.tumble, 0.0f32..=30.0).text("tumble (rad/s)"));
        ui.add(egui::Slider::new(&mut s.spin_jitter, 0.0f32..=30.0).text("random spin (rad/s)"));
        ui.separator();
        ui.add(egui::Slider::new(&mut s.spine_limit_deg, 0.0f32..=90.0).text("spine turns (°)"));
        ui.add(egui::Slider::new(&mut s.neck_limit_deg, 0.0f32..=90.0).text("neck turns (°)"));
        ui.add(egui::Slider::new(&mut s.shoulder_limit_deg, 0.0f32..=150.0).text("shoulders turn (°)"));
        ui.add(egui::Slider::new(&mut s.hip_limit_deg, 0.0f32..=150.0).text("hips turn (°)"));
        ui.add(egui::Slider::new(&mut s.elbow_bend_deg, 0.0f32..=170.0).text("elbows bend to (°)"));
        ui.add(egui::Slider::new(&mut s.knee_bend_deg, 0.0f32..=170.0).text("knees bend to (°)"));
        ui.add(egui::Slider::new(&mut s.joint_stiffness, 0.0f32..=50.0).text("joint pull to death pose"));
        ui.add(egui::Slider::new(&mut s.joint_damping, 0.0f32..=20.0).text("joint damping"));
        ui.separator();
        ui.add(egui::Slider::new(&mut s.linear_damping, 0.0f32..=5.0).text("damping, moving (/s)"));
        ui.add(egui::Slider::new(&mut s.angular_damping, 0.0f32..=10.0).text("damping, turning (/s)"));
        ui.add(egui::Slider::new(&mut s.friction, 0.0f32..=2.0).text("friction"));
        ui.add(egui::Slider::new(&mut s.bounce, 0.0f32..=1.0).text("bounce"));
        ui.add(egui::Slider::new(&mut s.gravity_scale, 0.0f32..=3.0).text("gravity (×)"));
        ui.add(egui::Slider::new(&mut s.thickness, 0.3f32..=2.0).text("limb thickness (×)"));
        ui.add(egui::Slider::new(&mut s.mass_scale, 0.1f32..=5.0).text("weight (×)"));
        ui.separator();
        ui.checkbox(&mut s.crawlers, "Dead crawlers go limp");
        ui.add(egui::Slider::new(&mut s.crawler_nudge, 0.0f32..=5.0).text("crawler slump push (m/s)"));
        ui.horizontal(|ui| {
            if ui.button("Reset ragdolls").clicked() {
                *s = default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("{s:#?}");
            }
        });
    }
}
