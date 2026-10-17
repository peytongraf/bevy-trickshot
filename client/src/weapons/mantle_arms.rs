//! What a mantle (`player::mantle`) looks like in first person, as Call of
//! Duty shows it: the weapon dips away, both hands come up and plant on the
//! ledge, stay put on it while we climb past them — the arms bending, then
//! pushing down — let go and drop out of view, and the weapon comes back up.
//!
//! The arms are a second, weaponless copy of the perk-drinking arms
//! (`models/characters/arms_drinking.glb`, its bottle hidden), held on the
//! clip's first frame — relaxed, half-curled hands — and posed by two-bone
//! IK ([`held_weapon::two_bone`]) every frame, after the animation and before
//! transforms propagate. Not a baked clip: every ledge is a different height
//! and distance, and the hands have to land on *this* one and stay on it.
//!
//! The hands' targets are the ledge's real edge (`MantleRun::grab`), brought
//! into the view model's space: a view model is drawn small and close to
//! the eye, by its own camera with its own field of view, so a point in the
//! world maps to the one the view-model camera draws in the same place on
//! screen, at the arms' scale ([`MantleArmsLook::world_scale`]).
//!
//! The weapon's state is left alone — `weapon_system` is paused for the
//! climb (`not_mantling`), and for the moment the weapon takes to come back
//! up ([`weapon_ready`]); only the view models' transforms are dipped, on
//! top of everything else that frame ([`dip_weapon_for_mantle`]).
//!
//! [`MantleView`] is cleared on entering a game, on respawn and by a kill
//! cam, death or fall; the arms hide whenever it's empty.

use std::f32::consts::PI;

use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy::scene::SceneInstanceReady;
use bevy::transform::TransformSystem;

use crate::avatars::held_weapon::two_bone;
use crate::death_effect::DeathEffect;
use crate::fall_death::FallDeathState;
use crate::killcam::ActiveKillCam;
use crate::player::{Mantle, MantleSettings, ViewModelCamera, WorldModelCamera};
use crate::util::ease;
use crate::{AppState, VIEW_MODEL_RENDER_LAYER};

use super::drink_arms::{DrinkArmsSettings, DrinkPhase, PerkDrink, BOTTLE_LABEL_NODE, BOTTLE_NODE};
use super::knife_view_model::KnifeViewModel;
use super::view_model::ViewModel;
use super::ThrowingKnife;

/// The arms' bones, by glTF name: upper arm, forearm, wrist...
const LEFT_ARM: [&str; 3] = ["l_upperarm_05", "l_forearm_03", "l_wrist_01"];
const RIGHT_ARM: [&str; 3] = ["r_upperarm_025", "r_forearm_026", "r_wrist_027"];
/// ...and each hand's index, middle and little finger knuckles — which way
/// the hand points, and which way it's turned.
const LEFT_KNUCKLES: [&str; 3] = ["l_index_low_06", "l_middle_low_055", "l_pinky_low_09"];
const RIGHT_KNUCKLES: [&str; 3] = ["r_index_low_035", "r_middle_low_028", "r_pinky_low_038"];

pub(crate) struct MantleArmsPlugin;

impl Plugin for MantleArmsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MantleView>()
            .add_systems(OnEnter(AppState::InGame), reset_mantle_view)
            .add_systems(OnExit(AppState::InGame), reset_mantle_view)
            .add_systems(
                Update,
                (
                    clear_mantle_view,
                    // (Held still whenever the climb is — `drive_mantle`'s
                    // gating.)
                    follow_mantle.run_if(
                        crate::menu::game_active
                            .and(crate::killcam::no_killcam)
                            .and(crate::fall_death::no_fall_death),
                    ),
                )
                    .chain()
                    .after(crate::player::drive_mantle)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                PostUpdate,
                pose_mantle_arms
                    .after(bevy::app::Animation)
                    .before(TransformSystem::TransformPropagate),
            );
    }
}

/// Panel-tunable first-person climb (part of `MantleSettings`, the debug
/// panel's "Mantle" section). Fractions are of the climb's duration.
#[derive(Clone, Copy)]
pub(crate) struct MantleArmsLook {
    /// View-model metres per world metre: how big the world is to the arms
    /// (theirs are drawn about this much smaller than life).
    pub(crate) world_scale: f32,
    /// How far apart (m) the hands land on the ledge...
    pub(crate) hand_spread: f32,
    /// ...how far (m) onto it past its edge the knuckles go...
    pub(crate) grab_inset: f32,
    /// ...and how far (degrees) the fingers tip down from level, over it.
    pub(crate) fingers_down_deg: f32,
    /// The hands are on the ledge this far into the climb...
    pub(crate) reach_frac: f32,
    /// ...and let go this far in...
    pub(crate) let_go_frac: f32,
    /// ...taking this long (s) to drop out of view.
    pub(crate) release_secs: f32,
    /// How long (s) the weapon takes to dip away...
    pub(crate) stow_secs: f32,
    /// ...and to come back up, once the hands are half gone.
    pub(crate) raise_secs: f32,
    /// How far (m) the weapon drops, and tips forward (degrees), while away.
    pub(crate) weapon_drop: f32,
    pub(crate) weapon_tip_deg: f32,
}

impl Default for MantleArmsLook {
    fn default() -> Self {
        Self {
            world_scale: 0.4,
            hand_spread: 0.45,
            grab_inset: 0.04,
            fingers_down_deg: 12.0,
            reach_frac: 0.3,
            let_go_frac: 0.75,
            release_secs: 0.15,
            stow_secs: 0.12,
            raise_secs: 0.22,
            weapon_drop: 0.3,
            weapon_tip_deg: 35.0,
        }
    }
}

/// The climb as we see it — it runs on past the climb itself while the
/// hands drop away and the weapon comes back.
#[derive(Resource, Default)]
pub(crate) struct MantleView {
    climb: Option<ClimbView>,
    /// The last climb followed (`Mantle::count`).
    seen: u32,
}

struct ClimbView {
    grab: Vec3,
    facing: Vec3,
    /// Seconds since the climb started...
    clock: f32,
    /// ...and how long the climb itself takes.
    duration: f32,
}

impl MantleView {
    /// A climb's showing — the weapon's away, or on its way back.
    pub(crate) fn busy(&self) -> bool {
        self.climb.is_some()
    }

    /// The climb for everyone else (`PlayerPose::mantle`) — their view of
    /// our soldier follows it (`avatars::held_weapon::mantle_look`).
    pub(crate) fn remote(&self) -> Option<shared::MantleProgress> {
        let c = self.climb.as_ref()?;
        Some(shared::MantleProgress {
            grab: c.grab.to_array(),
            facing_yaw: (-c.facing.x).atan2(-c.facing.z),
            clock: c.clock,
            duration: c.duration,
        })
    }
}

impl ClimbView {
    /// How far (`0..1`) the hands are on the ledge — off it, `0`, is where
    /// the drink clip's first frame hangs them, out of view.
    fn grip(&self, look: &MantleArmsLook) -> f32 {
        let reach = (self.duration * look.reach_frac).max(1e-3);
        let let_go = self.duration * look.let_go_frac;
        if self.clock < let_go {
            ease(self.clock / reach)
        } else {
            1.0 - ease((self.clock - let_go) / look.release_secs.max(1e-3))
        }
    }

    fn arms_shown(&self, look: &MantleArmsLook) -> bool {
        self.clock < self.duration * look.let_go_frac + look.release_secs
    }

    /// When the weapon starts back up — the hands half gone.
    fn raise_from(&self, look: &MantleArmsLook) -> f32 {
        self.duration * look.let_go_frac + look.release_secs * 0.5
    }

    /// How far (`0..1`) the weapon's dipped away.
    fn weapon_away(&self, look: &MantleArmsLook) -> f32 {
        let away = ease(self.clock / look.stow_secs.max(1e-3));
        let back = ease((self.clock - self.raise_from(look)) / look.raise_secs.max(1e-3));
        away * (1.0 - back)
    }

    fn over(&self, look: &MantleArmsLook) -> bool {
        self.clock >= self.raise_from(look) + look.raise_secs
    }
}

/// Run condition: the weapon's up — no climb showing. `weapon_system` waits
/// for it, so nothing fires, reloads or swaps with the weapon away.
pub(crate) fn weapon_ready(view: Res<MantleView>) -> bool {
    !view.busy()
}

fn reset_mantle_view(mut view: ResMut<MantleView>) {
    *view = MantleView::default();
}

/// The arms scene root, a child of the camera rig.
#[derive(Component)]
pub(crate) struct MantleArmsViewModel;

/// Handles + node index for the arms' clip (the drink's, held on its first
/// frame).
#[derive(Component)]
pub(crate) struct MantleArmsAnimation {
    pub(crate) graph: Handle<AnimationGraph>,
    pub(crate) index: AnimationNodeIndex,
}

/// The bones posed, found once the scene's in.
#[derive(Component)]
struct MantleArmsRig {
    /// Left then right: upper arm, forearm, wrist...
    arms: [[Entity; 3]; 2],
    /// ...and the index, middle and little finger knuckles.
    knuckles: [[Entity; 3]; 2],
}

/// Once the arms scene has spawned: put it on the view-model layer, hide
/// the bottle, find the bones and hold the clip on its first frame.
pub(crate) fn start_mantle_arms(
    trigger: Trigger<SceneInstanceReady>,
    arms: Query<&MantleArmsAnimation>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut players: Query<&mut AnimationPlayer>,
    mut commands: Commands,
) {
    let root = trigger.target();
    let Ok(anim) = arms.get(root) else { return };
    let mut by_name = std::collections::HashMap::new();
    for entity in children.iter_descendants(root) {
        commands.entity(entity).insert((
            RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
            // (Skinned meshes cull against their rest pose.)
            NoFrustumCulling,
        ));
        if let Ok(name) = names.get(entity) {
            if matches!(name.as_str(), BOTTLE_NODE | BOTTLE_LABEL_NODE) {
                commands.entity(entity).insert(Visibility::Hidden);
            }
            by_name.insert(name.as_str().to_owned(), entity);
        }
        if let Ok(mut player) = players.get_mut(entity) {
            player.play(anim.index).seek_to(0.0).pause();
            commands.entity(entity).insert(AnimationGraphHandle(anim.graph.clone()));
        }
    }
    let find = |list: [&str; 3]| -> Option<[Entity; 3]> {
        Some([by_name.get(list[0]).copied()?, by_name.get(list[1]).copied()?, by_name.get(list[2]).copied()?])
    };
    let rig = (|| {
        Some(MantleArmsRig {
            arms: [find(LEFT_ARM)?, find(RIGHT_ARM)?],
            knuckles: [find(LEFT_KNUCKLES)?, find(RIGHT_KNUCKLES)?],
        })
    })();
    match rig {
        Some(rig) => {
            commands.entity(root).insert(rig);
        }
        None => warn!("mantle arms: arms_drinking.glb is missing an arm bone"),
    }
}

/// A kill cam, death or fall ends whatever climb is showing.
fn clear_mantle_view(
    mantle: Res<Mantle>,
    killcam: Res<ActiveKillCam>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    mut view: ResMut<MantleView>,
) {
    if killcam.0.is_some() || death.is_active() || crate::fall_death::effect_active(fall) {
        if view.climb.is_some() {
            view.climb = None;
        }
        view.seen = mantle.count;
    }
}

/// Start following each new climb — unless the drinking or throwing arms
/// are out (the weapon's away already; the climb just moves us) — and run
/// the clock.
fn follow_mantle(
    time: Res<Time>,
    mantle: Res<Mantle>,
    cfg: Res<MantleSettings>,
    drink: Res<PerkDrink>,
    lethal: Res<ThrowingKnife>,
    mut view: ResMut<MantleView>,
) {
    if let Some(run) = mantle.active.as_ref().filter(|_| mantle.count != view.seen) {
        view.seen = mantle.count;
        if drink.phase == DrinkPhase::Idle && !lethal.in_use() {
            view.climb = Some(ClimbView {
                grab: run.grab,
                facing: run.facing,
                clock: 0.0,
                duration: cfg.duration.max(0.01),
            });
            return;
        }
    }
    let look = cfg.arms;
    if let Some(climb) = view.climb.as_mut() {
        climb.clock += time.delta_secs();
        if climb.over(&look) {
            view.climb = None;
        }
    }
}

/// Dip the sniper / AK-74 / Ray Gun and the knife down and forward, out of
/// view, for the climb — on top of everything else that's posed them this
/// frame (`apply_ads`, the sways, the recoil shudder all rewrite them first).
pub(crate) fn dip_weapon_for_mantle(
    view: Res<MantleView>,
    cfg: Res<MantleSettings>,
    mut models: Query<&mut Transform, Or<(With<ViewModel>, With<KnifeViewModel>)>>,
) {
    let Some(climb) = view.climb.as_ref() else { return };
    let look = cfg.arms;
    let away = climb.weapon_away(&look);
    if away <= 0.0 {
        return;
    }
    let dip = Transform::from_rotation(Quat::from_rotation_x(-look.weapon_tip_deg.to_radians() * away))
        * Transform::from_translation(Vec3::NEG_Y * look.weapon_drop * away);
    for mut t in &mut models {
        *t = dip * *t;
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

fn fov(projection: &Projection) -> f32 {
    match projection {
        Projection::Perspective(p) => p.fov,
        _ => 70f32.to_radians(),
    }
}

/// Show the arms while a climb wants them, and put the hands on the ledge:
/// each knuckle on its spot along the edge, the palm flat on top and the
/// fingers over it, as much as [`ClimbView::grip`] says — the rest of the
/// way, where the clip's first frame hangs them.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn pose_mantle_arms(
    view: Res<MantleView>,
    cfg: Res<MantleSettings>,
    drink_cfg: Res<DrinkArmsSettings>,
    mut arms: Query<(Entity, &MantleArmsRig, &mut Visibility), With<MantleArmsViewModel>>,
    world_cam: Query<(Entity, &Projection), (With<WorldModelCamera>, Without<ViewModelCamera>)>,
    vm_cam: Query<(Entity, &Projection), (With<ViewModelCamera>, Without<WorldModelCamera>)>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform>,
) {
    let Ok((root, rig, mut vis)) = arms.single_mut() else { return };
    let look = cfg.arms;
    let climb = view.climb.as_ref().filter(|c| c.arms_shown(&look));
    vis.set_if_neq(if climb.is_some() { Visibility::Inherited } else { Visibility::Hidden });
    let Some(climb) = climb else { return };
    let (Ok((world_cam, world_proj)), Ok((vm_cam, vm_proj))) = (world_cam.single(), vm_cam.single()) else {
        return;
    };
    if let Ok(mut t) = transforms.get_mut(root) {
        *t = drink_cfg.transform();
    }
    let grip = climb.grip(&look);
    if grip <= 0.0 {
        return;
    }

    // A point in the world to the one the view-model camera draws in the same
    // place on screen, at the arms' scale — never behind the eye.
    let world_eye = world_now(world_cam, &parents, &transforms).inverse();
    let vm_eye = world_now(vm_cam, &parents, &transforms);
    let widen = (fov(vm_proj) * 0.5).tan() / (fov(world_proj) * 0.5).tan().max(1e-3);
    let scale = look.world_scale.max(1e-3);
    let to_view_model = |p: Vec3| {
        let local = world_eye.transform_point3(p);
        let local = Vec3::new(local.x * widen, local.y * widen, local.z.min(-0.05 / scale)) * scale;
        vm_eye.transform_point3(local)
    };

    let facing = climb.facing;
    let right = facing.cross(Vec3::Y).normalize_or(Vec3::X);
    let tip = look.fingers_down_deg.to_radians().clamp(-PI * 0.5, PI * 0.5);
    let fingers = (facing * tip.cos() - Vec3::Y * tip.sin()).normalize();
    for (side, (bones, knuckles)) in rig.arms.iter().zip(&rig.knuckles).enumerate() {
        // (Left hand: its thumb side points right, palm down; the right
        // hand's, left.)
        let outward = if side == 0 { -1.0 } else { 1.0 };
        let spot = climb.grab + right * (outward * look.hand_spread * 0.5) + facing * look.grab_inset;
        let spot = to_view_model(spot);

        let (_, hand_rot, wrist) = world_now(bones[2], &parents, &transforms).to_scale_rotation_translation();
        let at = |e: Entity| world_now(e, &parents, &transforms).translation.into();
        let [index, middle, pinky]: [Vec3; 3] = knuckles.map(at);
        let Some(along) = (middle - wrist).try_normalize() else { continue };
        let Some(across) = (index - pinky).reject_from(along).try_normalize() else { continue };
        let want_across = (right * -outward).reject_from(fingers).normalize_or(right);
        let basis = |a: Vec3, b: Vec3| Quat::from_mat3(&Mat3::from_cols(a, b, a.cross(b)));
        let flat = basis(fingers, want_across) * basis(along, across).inverse() * hand_rot;
        let to_rot = hand_rot.slerp(flat, grip);
        // (The wrist, so that the knuckles land on the spot.)
        let knuckle = hand_rot.inverse() * (middle - wrist);
        let to = wrist.lerp(spot - flat * knuckle, grip);
        reach(*bones, to, to_rot, vm_eye.transform_vector3(Vec3::NEG_Y), &parents, &mut transforms);
    }
}

/// Two-bone IK in the world: turn `bones` (upper arm, forearm, hand) so the
/// hand's at `to`, turned `to_rot` — or as near as the arm reaches —
/// keeping the elbow bent the way it is.
fn reach(
    bones: [Entity; 3],
    to: Vec3,
    to_rot: Quat,
    pole: Vec3,
    parents: &Query<&ChildOf>,
    transforms: &mut Query<&mut Transform>,
) {
    let [upper, fore, hand] = bones;
    let Ok(above) = parents.get(upper).map(|p| p.parent()) else { return };
    let rot_pos = |e: Entity| {
        let (_, r, p) = world_now(e, parents, transforms).to_scale_rotation_translation();
        (r, p)
    };
    let (above, _) = rot_pos(above);
    let (upper_rot, a) = rot_pos(upper);
    let (fore_rot, b) = rot_pos(fore);
    let (_, c) = rot_pos(hand);
    let Some((turn_upper, turn_fore)) = two_bone(a, b, c, to, pole) else { return };
    let upper_rot = turn_upper * upper_rot;
    let fore_rot = turn_fore * turn_upper * fore_rot;
    for (e, rot) in [
        (upper, above.inverse() * upper_rot),
        (fore, upper_rot.inverse() * fore_rot),
        (hand, fore_rot.inverse() * to_rot),
    ] {
        if let Ok(mut t) = transforms.get_mut(e) {
            t.rotation = rot.normalize();
        }
    }
}
