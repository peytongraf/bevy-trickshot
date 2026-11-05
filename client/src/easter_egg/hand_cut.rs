//! The easter egg's ritual, in first person: as the man on the radio says
//! "cut your hands, bleed into it" (`RadioLine::EggRitualBegins`), we look
//! down at our own hands — the left held out palm up, our own combat knife
//! in the right (the Hierophant's dagger's already in the altar, and every
//! one of the squad has a knife of their own, so everyone cuts at once) —
//! draw the blade's edge across the palm, the cut opening red behind it,
//! and hold the hand out over the altar to bleed into it, drops running off
//! it, before both hands go back down and we look up for the vial.
//!
//! The arms are a copy of the knife's own (`models/weapons/knife.glb`,
//! arms and knife together), held on the clip's first frame (the idle grip)
//! and posed by two-bone IK every frame, after the animation and before
//! transforms propagate — as the mantle's are (`weapons::mantle_arms`):
//!
//! * the left hand's turned palm up and put where [`HandCutSettings`] says,
//!   in the view model's space;
//! * each arm reaches by its own two-bone IK ([`reach_arm`]): the elbow put
//!   where the forearm lines up with the hand (no wrist bent back on
//!   itself), and most of the hand's roll turned by the forearm about its
//!   length rather than the wrist (the rig has no twist bones — a wrist
//!   turned on its own pinches its skin like a sweet wrapper);
//! * the knife is placed first — its edge ([`EDGE_X`], along the blade from
//!   [`BLADE_FROM`] to [`BLADE_TO`], measured off its mesh) on the palm,
//!   lying across it toward the thumb, drawn back along its length as the
//!   cut opens — and the right hand's put wherever holding it there needs
//!   (the knife keeps the grip the clip gives it: in the model it's not the
//!   hand's child, so it's moved with it here).
//!
//! The cut scene hides the weapon's layer (`revive::apply_down_view`), so
//! these are on their own ([`crate::EGG_HANDS_RENDER_LAYER`]), lit like the
//! view model. The look down is the cut scene's ([`look_down`], from
//! `cinematic::aim_view`) — sent with our pose, so the others see us look
//! down too.
//!
//! All of it runs off the ritual's clock (`EggRitual::t`) — or, from the
//! debug panel's "Play the cut" ([`HandCutPreview`]), a clock of its own
//! running over the same seconds, outside the ritual: the weapon hidden
//! and the view tipped down the same way (on top of our own look, and given
//! back as it ends). Nothing's shown or kept outside either, and both clocks
//! are reset with the game. The blood's
//! pieces are `StateScoped(InGame)`; the drops are gone as the hands are.

use std::f32::consts::PI;

use bevy::math::Affine3A;
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy::scene::SceneInstanceReady;
use bevy::transform::TransformSystem;
use bevy_egui::egui;

use crate::audio_engine::prelude::*;
use crate::util::ease;
use crate::weapons::world_now;
use crate::{AppState, KnifeSounds, KnifeViewModelSettings, EGG_HANDS_RENDER_LAYER};

use super::EggRitual;

/// The knife arms' bones, by glTF name: upper arm, forearm, wrist...
const LEFT_ARM: [&str; 3] = ["L_arm_01", "L_elbow_02", "L_wrist_03"];
const RIGHT_ARM: [&str; 3] = ["R_arm_023", "R_elbow_024", "R_wrist_025"];
/// ...the cut hand's index, middle and little finger knuckles...
const LEFT_KNUCKLES: [&str; 3] = ["L_point1_08", "L_middle1_00", "L_pink1_019"];
/// ...and the knife (a child of the right hand's IK goal, not of the hand).
const KNIFE_NODE: &str = "knife";

/// The knife's mesh (its own units, its node's frame): the blade runs along
/// +Y from the guard ([`BLADE_FROM`]) to the tip ([`BLADE_TO`]), its sharp
/// edge along −X at [`EDGE_X`] (where the tip thins to nothing).
const BLADE_FROM: f32 = 10.0;
const BLADE_TO: f32 = 42.0;
const EDGE_X: f32 = -1.9;

pub(super) struct HandCutPlugin;

impl Plugin for HandCutPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HandCutSettings>()
            .init_resource::<HandCutFrame>()
            .init_resource::<HandCutPreview>()
            .add_systems(Startup, make_blood_assets)
            .add_systems(OnEnter(AppState::InGame), spawn_blood)
            .add_systems(
                OnExit(AppState::InGame),
                |mut frame: ResMut<HandCutFrame>, mut preview: ResMut<HandCutPreview>| {
                    *frame = default();
                    *preview = default();
                },
            )
            .add_systems(
                Update,
                run_preview
                    .after(super::track_ritual)
                    .after(crate::player::look_around)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                PostUpdate,
                (pose_hand_cut, bleed)
                    .chain()
                    .after(bevy::app::Animation)
                    .before(TransformSystem::TransformPropagate),
            );
    }
}

/// Panel-tunable cut (the debug panel's "Easter egg (Der Tempel)" → "The
/// hand cut"). Times are seconds into the ritual (to the radio's words);
/// places and directions are in the view model's space (x right, y up, −z
/// ahead, metres as the arms are drawn).
#[derive(Resource, Clone, Debug)]
pub(crate) struct HandCutSettings {
    /// How far (°) we look down at our hands, from then till then, and how
    /// long (s) the look takes to come in and go.
    pub(crate) look_down_deg: f32,
    pub(crate) look_from: f32,
    pub(crate) look_to: f32,
    pub(crate) look_ease: f32,
    /// The hands come up into view (the knife's a beat behind)...
    pub(crate) rise: (f32, f32),
    pub(crate) knife_lag: f32,
    /// ...the blade comes down onto the palm...
    pub(crate) approach: (f32, f32),
    /// ...is drawn across it...
    pub(crate) draw: (f32, f32),
    /// ...and goes, the knife hand dropping away...
    pub(crate) knife_away: (f32, f32),
    /// ...while the cut hand's tipped out over the altar to bleed...
    pub(crate) tilt: (f32, f32),
    /// ...before it goes too.
    pub(crate) hand_away: (f32, f32),
    /// Where the middle of the palm is, which way the fingers point and
    /// which way the palm faces.
    pub(crate) palm_at: Vec3,
    pub(crate) fingers: Vec3,
    pub(crate) palm_facing: Vec3,
    /// How far (m) the palm's skin is off the hand's bones.
    pub(crate) palm_surface: f32,
    /// How much (0..1) of each hand's roll the forearm takes (the rest the
    /// wrist), how far (share of the forearm's length) the cut hand's elbow
    /// drops below the line of the hand, and which way (from the knife hand,
    /// in forearm lengths) the knife arm's elbow sits.
    pub(crate) forearm_twist: f32,
    pub(crate) elbow_drop: f32,
    pub(crate) knife_elbow: Vec3,
    /// How far (m) out of view the hands are — down, not back toward the
    /// shoulders (which folds the arms up into view); the knife hand a
    /// little further out to its side.
    pub(crate) hidden: Vec3,
    /// The cut: how long (m) across the palm, turned (°) off straight
    /// across, which part of the blade (0 the guard, 1 the tip) touches as
    /// it starts and as it ends, how high (m) the blade comes down from, and
    /// how hard (m) it presses in.
    pub(crate) cut_len: f32,
    pub(crate) cut_turn_deg: f32,
    pub(crate) blade_from: f32,
    pub(crate) blade_to: f32,
    pub(crate) lift: f32,
    pub(crate) press: f32,
    /// Bleeding out over the altar: how far (°) the hand's tipped, and how
    /// far (m) it reaches out.
    pub(crate) tilt_deg: f32,
    pub(crate) reach_out: f32,
    /// The blood: how wide (m) the cut opens, how big (m) the pool in the
    /// palm gets, how many drops fall (per second) and how big (m), how fast
    /// they fall (m/s²) and how long (s) they last.
    pub(crate) cut_width: f32,
    pub(crate) pool_radius: f32,
    pub(crate) drops_per_sec: f32,
    pub(crate) drop_size: f32,
    pub(crate) drop_gravity: f32,
    pub(crate) drop_life: f32,
    /// How loud the cut is.
    pub(crate) volume: f32,
}

impl Default for HandCutSettings {
    fn default() -> Self {
        Self {
            look_down_deg: 18.0,
            look_from: 3.0,
            look_to: 5.6,
            look_ease: 0.45,
            rise: (3.05, 3.55),
            knife_lag: 0.12,
            approach: (3.6, 3.85),
            draw: (3.85, 4.5),
            knife_away: (4.6, 5.0),
            tilt: (4.55, 5.05),
            hand_away: (5.25, 5.65),
            palm_at: Vec3::new(-0.03, -0.15, -0.40),
            fingers: Vec3::new(0.25, 0.35, -1.0),
            palm_facing: Vec3::new(-0.1, 1.0, 0.75),
            palm_surface: 0.022,
            forearm_twist: 0.8,
            elbow_drop: 0.35,
            knife_elbow: Vec3::new(0.55, -1.0, 0.2),
            hidden: Vec3::new(0.0, -0.32, -0.02),
            cut_len: 0.055,
            cut_turn_deg: 12.0,
            blade_from: 0.25,
            blade_to: 0.8,
            lift: 0.04,
            press: 0.0008,
            tilt_deg: 35.0,
            reach_out: 0.04,
            cut_width: 0.004,
            pool_radius: 0.022,
            drops_per_sec: 18.0,
            drop_size: 0.0032,
            drop_gravity: 4.0,
            drop_life: 0.55,
            volume: 0.8,
        }
    }
}

/// `x` from 0 to 1 as `t` goes over `span` (held either side).
fn ramp(t: f32, span: (f32, f32)) -> f32 {
    ((t - span.0) / (span.1 - span.0).max(1e-3)).clamp(0.0, 1.0)
}

impl HandCutSettings {
    /// Whether the arms are out, `t` s into the ritual.
    fn shown(&self, t: f32) -> bool {
        t >= self.rise.0 && t < self.hand_away.1
    }

    /// How far (0..1) the cut's opened.
    fn cut(&self, t: f32) -> f32 {
        ease(ramp(t, self.draw))
    }
}

/// The debug panel's "Play the cut": its own clock (s, as into the ritual)
/// while it plays, and how far (rad) it's tipped our view down so far.
#[derive(Resource, Default)]
pub(crate) struct HandCutPreview {
    t: Option<f32>,
    asked: bool,
    looked: f32,
}

impl HandCutPreview {
    /// Whether it's playing (the weapon's hidden for it,
    /// `revive::apply_down_view`).
    pub(crate) fn playing(&self) -> bool {
        self.t.is_some()
    }
}

impl HandCutSettings {
    /// The seconds (into the ritual) a preview plays over: a moment before
    /// the look down or the hands, to a moment after both are done.
    fn preview_span(&self) -> (f32, f32) {
        (self.look_from.min(self.rise.0) - 0.3, self.look_to.max(self.hand_away.1) + 0.2)
    }
}

/// The cut's clock: the ritual's, or a preview's.
fn clock(ritual: &EggRitual, preview: &HandCutPreview) -> Option<f32> {
    ritual.t.or(preview.t)
}

/// Run a preview asked for (not during the ritual itself): its clock on,
/// held while the game's paused, and our view tipped down as the ritual's
/// cut scene would (by how much more or less than last frame, so our own
/// look stays ours) — all given back once it's done.
fn run_preview(
    time: Res<Time>,
    s: Res<HandCutSettings>,
    ritual: Res<EggRitual>,
    paused: Res<crate::pause::GamePaused>,
    mut preview: ResMut<HandCutPreview>,
    head: Option<Single<&mut Transform, With<crate::player::PlayerHead>>>,
) {
    let (from, to) = s.preview_span();
    if std::mem::take(&mut preview.asked) && ritual.t.is_none() {
        preview.t = Some(from);
    }
    if ritual.t.is_some() {
        preview.t = None;
    } else if let Some(t) = preview.t.as_mut() {
        if !paused.0 {
            *t += time.delta_secs();
        }
        if *t >= to {
            preview.t = None;
        }
    }
    let want = look_down(&s, preview.t);
    let turn = want - preview.looked;
    if turn != 0.0 {
        if let Some(mut head) = head {
            let (_, pitch, _) = head.rotation.to_euler(EulerRot::YXZ);
            head.rotation = Quat::from_rotation_x((pitch - turn).clamp(-1.5, 1.5));
        }
        preview.looked = want;
    }
}

/// How far (rad) the cut scene tips the view down, `t` s into the ritual
/// (none outside it) — `cinematic::aim_view`.
pub(crate) fn look_down(s: &HandCutSettings, t: Option<f32>) -> f32 {
    let Some(t) = t else { return 0.0 };
    let e = s.look_ease.max(0.01);
    let k = ease(ramp(t, (s.look_from, s.look_from + e))) * (1.0 - ease(ramp(t, (s.look_to - e, s.look_to))));
    s.look_down_deg.to_radians() * k
}

/// Where the cut is this frame, for [`bleed`] — set by [`pose_hand_cut`]
/// (none while the arms are away). In the world.
#[derive(Resource, Default)]
struct HandCutFrame {
    palm: Option<Palm>,
    /// Drops due (carried between frames), and how many have fallen.
    owed: f32,
    dropped: u32,
    /// Whether this ritual's cut has been heard.
    heard: bool,
}

#[derive(Clone, Copy)]
struct Palm {
    /// The middle of the palm, on its skin; which way it faces; and the
    /// cut's line across it (from end to end), as far as it's opened.
    at: Vec3,
    facing: Vec3,
    from: Vec3,
    to: Vec3,
    /// How far it's bled (the pool, the drops — 0 before the cut, 1 by the
    /// time the hand's tipped).
    bled: f32,
    /// Which way's down, in the view model's space (the drops' gravity).
    down: Vec3,
}

/// The arms' scene root, a child of the camera rig.
#[derive(Component)]
pub(crate) struct HandCutArms;

/// Handles + node index for the knife's clip, held on its first frame.
#[derive(Component)]
pub(crate) struct HandCutAnimation {
    pub(crate) graph: Handle<AnimationGraph>,
    pub(crate) index: AnimationNodeIndex,
}

/// The bones posed, found once the scene's in.
#[derive(Component)]
struct HandCutRig {
    /// Left then right: upper arm, forearm, wrist...
    arms: [[Entity; 3]; 2],
    /// ...the cut hand's index, middle and little finger knuckles...
    knuckles: [Entity; 3],
    /// ...and the knife.
    knife: Entity,
}

/// Once the arms scene has spawned: put it on the cut scene's layer, find
/// the bones and hold the clip on its first frame.
pub(crate) fn start_hand_cut_arms(
    trigger: Trigger<SceneInstanceReady>,
    arms: Query<&HandCutAnimation>,
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
            RenderLayers::layer(EGG_HANDS_RENDER_LAYER),
            // (Skinned meshes cull against their rest pose.)
            NoFrustumCulling,
            NotShadowCaster,
        ));
        if let Ok(name) = names.get(entity) {
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
        Some(HandCutRig {
            arms: [find(LEFT_ARM)?, find(RIGHT_ARM)?],
            knuckles: find(LEFT_KNUCKLES)?,
            knife: by_name.get(KNIFE_NODE).copied()?,
        })
    })();
    match rig {
        Some(rig) => {
            commands.entity(root).insert(rig);
        }
        None => warn!("hand cut: knife.glb is missing a bone or its knife"),
    }
}

/// A rotation taking `a` (and `b`, as far as it's square to `a`) to the
/// frame they make with `a × b`.
fn basis(a: Vec3, b: Vec3) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(a, b, a.cross(b)))
}

/// Show the arms while the ritual wants them, and pose them: the left palm
/// up, the knife's edge on it, drawn across, the right hand holding it —
/// then the cut hand tipped out to bleed, and both gone.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn pose_hand_cut(
    ritual: Res<EggRitual>,
    preview: Res<HandCutPreview>,
    s: Res<HandCutSettings>,
    knife_cfg: Res<KnifeViewModelSettings>,
    knife_sounds: Option<Res<KnifeSounds>>,
    mut frame: ResMut<HandCutFrame>,
    mut arms: Query<(Entity, &HandCutRig, &mut Visibility), With<HandCutArms>>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform>,
    mut commands: Commands,
) {
    let t = clock(&ritual, &preview);
    if t.is_none() {
        frame.heard = false;
    }
    let Ok((root, rig, mut vis)) = arms.single_mut() else { return };
    let t = t.filter(|t| s.shown(*t));
    vis.set_if_neq(if t.is_some() { Visibility::Inherited } else { Visibility::Hidden });
    let Some(t) = t else {
        frame.palm = None;
        return;
    };
    // (Held where the knife view model is.)
    if let Ok(mut tf) = transforms.get_mut(root) {
        *tf = Transform {
            translation: knife_cfg.translation,
            rotation: Quat::from_rotation_y(PI),
            scale: Vec3::splat(knife_cfg.scale),
        };
    }
    let Ok(rig_e) = parents.get(root).map(|p| p.parent()) else { return };
    // The view model's space, in the world.
    let space = world_now(rig_e, &parents, &transforms);
    let dir = |v: Vec3| space.transform_vector3(v).normalize_or(Vec3::NEG_Z);
    let down = dir(Vec3::NEG_Y);

    // The cut hand: up into view, palm up; tipped out over the altar to
    // bleed; then gone.
    let shown = ease(ramp(t, s.rise)) * (1.0 - ease(ramp(t, s.hand_away)));
    let tilt = ease(ramp(t, s.tilt));
    let fingers = dir(s.fingers);
    let facing = dir(s.palm_facing).reject_from(fingers).normalize_or(dir(Vec3::Y));
    // (Its index side, with its palm that way up — `thumb` × the fingers is
    // the palm's way, for this rig's left hand.)
    let thumb = facing.cross(fingers).normalize();
    let tip_out = Quat::from_axis_angle(fingers, -s.tilt_deg.to_radians() * tilt);
    let (facing, thumb) = (tip_out * facing, tip_out * thumb);
    let palm_at = space.transform_point3(
        s.palm_at + s.hidden * (1.0 - shown) + Vec3::NEG_Z * s.reach_out * tilt,
    );

    let [left, right] = rig.arms;
    let rot_pos = |e: Entity, transforms: &Query<&mut Transform>| {
        let (_, r, p) = world_now(e, &parents, transforms).to_scale_rotation_translation();
        (r, p)
    };
    let at = |e: Entity, transforms: &Query<&mut Transform>| -> Vec3 { world_now(e, &parents, transforms).translation.into() };

    // Left: the knuckles' frame turned onto the one wanted, the wrist put so
    // the middle of the palm lands on its spot.
    let (hand_rot, wrist) = rot_pos(left[2], &transforms);
    let [index, middle, pinky] = rig.knuckles.map(|e| at(e, &transforms));
    let Some(along) = (middle - wrist).try_normalize() else { return };
    let Some(across) = (index - pinky).reject_from(along).try_normalize() else { return };
    let palm_rot = basis(fingers, thumb) * basis(along, across).inverse() * hand_rot;
    let palm_side = along.cross(across).normalize();
    let palm_local = hand_rot.inverse() * ((middle - wrist) * 0.5 + palm_side * s.palm_surface);
    let left_to = palm_at - palm_rot * palm_local;
    let cut_arm = Reach { twist: s.forearm_twist, follow_hand: 1.0, elbow: down * s.elbow_drop };
    cut_arm.reach(left, left_to, palm_rot, &parents, &mut transforms);

    // The cut, across the palm toward the thumb (turned a little off
    // square), opening from the thumb's side as the blade's drawn back.
    let across_cut = Quat::from_axis_angle(facing, s.cut_turn_deg.to_radians()) * thumb;
    let cut = s.cut(t);
    let start = palm_at + across_cut * (s.cut_len * 0.5);
    let edge_at = start - across_cut * (s.cut_len * cut);

    // Right: the knife first — its edge on the cut (lifted off the palm
    // until it comes down, pressed in as it's drawn, and away once done) —
    // then the hand wherever holding it there needs.
    let (knife_scale, knife_rot, knife_pos) = world_now(rig.knife, &parents, &transforms).to_scale_rotation_translation();
    let (grip_rot, grip_wrist) = rot_pos(right[2], &transforms);
    let knife_in_hand = grip_rot.inverse() * knife_rot;
    let knife_off = grip_rot.inverse() * (knife_pos - grip_wrist);
    let knife_shown = ease(ramp(t, (s.rise.0 + s.knife_lag, s.rise.1 + s.knife_lag))) * (1.0 - ease(ramp(t, s.knife_away)));
    let lifted = s.lift * (1.0 - ease(ramp(t, s.approach))) + s.lift * ease(ramp(t, (s.draw.1, s.knife_away.0)));
    let pressed = s.press * (cut * PI).sin();
    // (Edge down onto the palm: the blade's −X toward it, its tip toward
    // the thumb.)
    let knife_rot_to = Quat::from_mat3(&Mat3::from_cols(facing, across_cut, facing.cross(across_cut)));
    let blade = s.blade_from + (s.blade_to - s.blade_from) * cut;
    let edge_local = Vec3::new(EDGE_X, BLADE_FROM + (BLADE_TO - BLADE_FROM) * blade, 0.0) * knife_scale;
    let hidden_world = space.transform_vector3(s.hidden + Vec3::X * 0.08) * (1.0 - knife_shown);
    let knife_to = edge_at + facing * (lifted - pressed) + hidden_world - knife_rot_to * edge_local;
    let grip_to_rot = knife_rot_to * knife_in_hand.inverse();
    let right_to = knife_to - grip_to_rot * knife_off;
    // (A fist round the knife points along the blade, not back down the
    // arm: its elbow sits where a held knife's would, down and out.)
    let knife_arm = Reach { twist: s.forearm_twist, follow_hand: 0.0, elbow: space.transform_vector3(s.knife_elbow) };
    knife_arm.reach(right, right_to, grip_to_rot, &parents, &mut transforms);
    // The knife where it's wanted, in its parent's frame (that's not moved
    // by any of the above).
    if let Ok(parent) = parents.get(rig.knife).map(|p| p.parent()) {
        let parent_world = world_now(parent, &parents, &transforms);
        let want = Affine3A::from_scale_rotation_translation(knife_scale, knife_rot_to, knife_to);
        let (scale, rotation, translation) = (parent_world.inverse() * want).to_scale_rotation_translation();
        if let Ok(mut tf) = transforms.get_mut(rig.knife) {
            *tf = Transform { translation, rotation, scale };
        }
    }

    // The blade bites as it's first drawn.
    if !frame.heard && t >= s.draw.0 {
        frame.heard = true;
        let variant = (crate::util::rand01(t.to_bits()) * 255.0) as u8;
        if let Some(clip) = knife_sounds.as_ref().and_then(|k| k.stab.pick(variant)) {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(clip.handle.clone()),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(s.volume)),
            ));
        }
    }
    let bled = ramp(t, (s.draw.0 + (s.draw.1 - s.draw.0) * 0.4, s.tilt.1));
    frame.palm = (cut > 0.0).then_some(Palm { at: palm_at, facing, from: start, to: edge_at, bled, down });
}

/// How an arm reaches ([`Reach::reach`]): how much of the hand's roll the
/// forearm takes; how much the elbow follows the line of the hand (back
/// along it from the wrist — right for an open hand, not a fist round a
/// knife); and which way (and how far, in forearm lengths) from the wrist
/// it goes besides.
struct Reach {
    twist: f32,
    follow_hand: f32,
    elbow: Vec3,
}

impl Reach {
    /// Two-bone IK in the world: turn `bones` (upper arm, forearm, hand) so
    /// the hand's at `to`, turned `to_rot` — or as near as the arm reaches.
    /// Unlike `weapons::reach` (which keeps the elbow bent the way the clip
    /// has it), the elbow goes where [`Reach`] says — for the cut hand,
    /// where the forearm would line up with it (back from the wrist along
    /// the hand's length, dropped a little), so the wrist only bends as
    /// much as it must; for the knife hand, down and out — and the
    /// forearm turns about its own length to take [`Reach::twist`] of the
    /// hand's roll, the wrist the rest.
    fn reach(
        &self,
        bones: [Entity; 3],
        to: Vec3,
        to_rot: Quat,
        parents: &Query<&ChildOf>,
        transforms: &mut Query<&mut Transform>,
    ) {
        let [upper, fore, hand] = bones;
        let Ok(above) = parents.get(upper).map(|p| p.parent()) else { return };
        let rot_pos = |e: Entity, transforms: &Query<&mut Transform>| {
            let (_, r, p) = world_now(e, parents, transforms).to_scale_rotation_translation();
            (r, p)
        };
        let (above, _) = rot_pos(above, transforms);
        let (upper_rot, a) = rot_pos(upper, transforms);
        let (fore_rot, b) = rot_pos(fore, transforms);
        let (_, c) = rot_pos(hand, transforms);
        // (Each bone runs along its child's offset from it — the forearm's
        // along the wrist's.)
        let Some(hand_axis) = transforms.get(hand).ok().and_then(|t| t.translation.try_normalize()) else { return };
        let (lab, lbc) = ((b - a).length(), (c - b).length());
        if lab < 1e-6 || lbc < 1e-6 {
            return;
        }
        let Some(dir) = (to - a).try_normalize() else { return };
        let dist = (to - a).length().clamp((lab - lbc).abs() + 1e-5, (lab + lbc) * 0.9999);
        // Where the elbow would like to be: back along the hand's own axis
        // (the hand bone runs along the same local axis as the forearm's
        // child offset) from the wrist, dropped a little.
        let hand_back = to_rot * -hand_axis;
        let wish = to + (hand_back * self.follow_hand + self.elbow) * lbc;
        let side = (wish - a)
            .reject_from(dir)
            .try_normalize()
            .or_else(|| (b - a).reject_from(dir).try_normalize())
            .unwrap_or_else(|| dir.any_orthonormal_vector());
        let along = (lab * lab - lbc * lbc + dist * dist) / (2.0 * dist);
        let elbow = a + dir * along + side * (lab * lab - along * along).max(0.0).sqrt();
        let wrist = a + dir * dist;
        let Some(turn_upper) = (b - a).try_normalize().zip((elbow - a).try_normalize()).map(|(f, t)| Quat::from_rotation_arc(f, t)) else {
            return;
        };
        let upper_rot = turn_upper * upper_rot;
        let fore_now = turn_upper * fore_rot;
        let Some(fore_dir) = (fore_now * hand_axis).try_normalize() else { return };
        let Some(fore_to) = (wrist - elbow).try_normalize() else { return };
        let fore_rot = Quat::from_rotation_arc(fore_dir, fore_to) * fore_now;
        // The hand's roll about the forearm's length, mostly the forearm's.
        let hand_local = fore_rot.inverse() * to_rot;
        let along_twist = hand_axis * hand_local.xyz().dot(hand_axis);
        let raw = Quat::from_xyzw(along_twist.x, along_twist.y, along_twist.z, hand_local.w);
        let twist = if raw.length_squared() > 1e-8 {
            let q = raw.normalize();
            if q.w < 0.0 { -q } else { q }
        } else {
            Quat::IDENTITY
        };
        let forearm_turn = Quat::IDENTITY.slerp(twist, self.twist.clamp(0.0, 1.0));
        let fore_rot = fore_rot * forearm_turn;
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
}

// --- the blood ------------------------------------------------------------------------------

#[derive(Resource)]
struct BloodAssets {
    rod: Handle<Mesh>,
    ball: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// The cut's line on the palm, and the blood pooling in it.
#[derive(Component)]
enum BloodPiece {
    Cut,
    Pool,
}

/// A drop of blood falling off the hand: how fast (m/s) it's going, and how
/// long (s) it has.
#[derive(Component)]
struct BloodDrop {
    velocity: Vec3,
    left: f32,
}

fn make_blood_assets(mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>, mut commands: Commands) {
    commands.insert_resource(BloodAssets {
        rod: meshes.add(Cylinder::new(0.5, 1.0)),
        ball: meshes.add(Sphere::new(1.0).mesh().uv(10, 6)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.32, 0.0, 0.015),
            perceptual_roughness: 0.15,
            reflectance: 0.6,
            emissive: LinearRgba::rgb(0.02, 0.0, 0.0),
            ..default()
        }),
    });
}

fn spawn_blood(assets: Res<BloodAssets>, mut commands: Commands) {
    for (piece, mesh) in [(BloodPiece::Cut, assets.rod.clone()), (BloodPiece::Pool, assets.ball.clone())] {
        commands.spawn((
            StateScoped(AppState::InGame),
            piece,
            Mesh3d(mesh),
            MeshMaterial3d(assets.material.clone()),
            Transform::default(),
            Visibility::Hidden,
            RenderLayers::layer(EGG_HANDS_RENDER_LAYER),
            NotShadowCaster,
            NoFrustumCulling,
        ));
    }
}

/// The cut opening on the palm and bleeding: its line, the pool in the
/// palm, and drops running off it — falling, and gone as they land (or as
/// the hand goes).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn bleed(
    time: Res<Time>,
    s: Res<HandCutSettings>,
    assets: Option<Res<BloodAssets>>,
    mut frame: ResMut<HandCutFrame>,
    mut pieces: Query<(&BloodPiece, &mut Transform, &mut Visibility), Without<BloodDrop>>,
    mut drops: Query<(Entity, &mut BloodDrop, &mut Transform), Without<BloodPiece>>,
    paused: Res<crate::pause::GamePaused>,
    mut commands: Commands,
) {
    let dt = if paused.0 { 0.0 } else { time.delta_secs() };
    let palm = frame.palm;
    for (piece, mut tf, mut vis) in &mut pieces {
        let Some(p) = palm else {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        vis.set_if_neq(Visibility::Inherited);
        match piece {
            BloodPiece::Cut => {
                let line = p.to - p.from;
                let len = line.length().max(1e-4);
                let width = s.cut_width * (0.6 + 0.4 * p.bled);
                *tf = Transform {
                    translation: (p.from + p.to) * 0.5 + p.facing * (width * 0.25),
                    rotation: Quat::from_rotation_arc(Vec3::Y, line / len),
                    scale: Vec3::new(width, len, width * 0.5),
                };
            }
            BloodPiece::Pool => {
                let r = s.pool_radius * p.bled.sqrt();
                *tf = Transform {
                    translation: p.at + p.facing * (r * 0.05),
                    rotation: Quat::from_rotation_arc(Vec3::Y, p.facing),
                    scale: Vec3::new(r, r * 0.12, r * 0.8),
                };
                vis.set_if_neq(if r > 0.0005 { Visibility::Inherited } else { Visibility::Hidden });
            }
        }
    }
    // Drops off the cut, as it bleeds.
    match (palm, assets) {
        (Some(p), Some(assets)) if p.bled > 0.0 => {
            frame.owed += s.drops_per_sec * dt;
            while frame.owed >= 1.0 {
                frame.owed -= 1.0;
                frame.dropped = frame.dropped.wrapping_add(1);
                let seed = frame.dropped;
                let along = crate::util::rand01(seed.wrapping_mul(7919));
                let size = s.drop_size * (0.6 + 0.8 * crate::util::rand01(seed.wrapping_mul(104_729)));
                let from = p.from.lerp(p.to, along) - p.facing * 0.002;
                let drift = Vec3::new(
                    crate::util::rand01(seed ^ 0x51) - 0.5,
                    0.0,
                    crate::util::rand01(seed ^ 0xa3) - 0.5,
                ) * 0.04;
                commands.spawn((
                    StateScoped(AppState::InGame),
                    BloodDrop { velocity: drift, left: s.drop_life },
                    Mesh3d(assets.ball.clone()),
                    MeshMaterial3d(assets.material.clone()),
                    Transform::from_translation(from).with_scale(Vec3::splat(size)),
                    RenderLayers::layer(EGG_HANDS_RENDER_LAYER),
                    NotShadowCaster,
                    NoFrustumCulling,
                ));
            }
        }
        _ => frame.owed = 0.0,
    }
    let gravity = palm.map_or(Vec3::NEG_Y, |p| p.down) * s.drop_gravity;
    for (e, mut drop, mut tf) in &mut drops {
        drop.left -= dt;
        if drop.left <= 0.0 || palm.is_none() {
            commands.entity(e).despawn();
            continue;
        }
        drop.velocity += gravity * dt;
        tf.translation += drop.velocity * dt;
        // (Stretched out along its fall.)
        let speed = drop.velocity.length();
        if let Some(way) = drop.velocity.try_normalize() {
            tf.rotation = Quat::from_rotation_arc(Vec3::Y, way);
        }
        let size = tf.scale.x.min(tf.scale.z);
        tf.scale = Vec3::new(size, size * (1.0 + speed * 1.5).min(3.0), size);
    }
}

// --- the debug panel ------------------------------------------------------------------------

fn span(ui: &mut egui::Ui, v: &mut (f32, f32), label: &str) {
    let max = shared::easter_egg::RITUAL_SECS;
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut v.0).speed(0.02).range(0.0..=max));
        ui.add(egui::DragValue::new(&mut v.1).speed(0.02).range(0.0..=max));
        ui.label(label);
    });
}

fn vec3(ui: &mut egui::Ui, v: &mut Vec3, label: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut v.x).speed(0.002).prefix("x "));
        ui.add(egui::DragValue::new(&mut v.y).speed(0.002).prefix("y "));
        ui.add(egui::DragValue::new(&mut v.z).speed(0.002).prefix("z "));
        ui.label(label);
    });
}

pub(crate) fn hand_cut_section(
    ui: &mut egui::Ui,
    s: &mut HandCutSettings,
    preview: &mut HandCutPreview,
    ritual: Option<f32>,
    in_game: bool,
) {
    ui.horizontal(|ui| {
        let can = in_game && ritual.is_none() && !preview.playing();
        if ui.add_enabled(can, egui::Button::new("Play the cut")).clicked() {
            preview.asked = true;
        }
        ui.label(match (ritual, preview.t) {
            (Some(t), _) => format!("ritual: {t:.2} s"),
            (None, Some(t)) => format!("playing: {t:.2} s"),
            (None, None) if in_game => "as in the ritual, look down and all".to_owned(),
            (None, None) => "(in a game)".to_owned(),
        });
    });
    ui.label("Times are seconds into the ritual (from — to).");
    ui.add(egui::Slider::new(&mut s.look_down_deg, 0.0f32..=45.0).text("look down at the hands (°)"));
    let mut look = (s.look_from, s.look_to);
    span(ui, &mut look, "looking down");
    (s.look_from, s.look_to) = look;
    ui.add(egui::Slider::new(&mut s.look_ease, 0.05f32..=1.5).text("look turns over (s)"));
    span(ui, &mut s.rise, "hands come up");
    ui.add(egui::Slider::new(&mut s.knife_lag, 0.0f32..=0.6).text("knife comes up after (s)"));
    span(ui, &mut s.approach, "blade comes down onto the palm");
    span(ui, &mut s.draw, "blade drawn across");
    span(ui, &mut s.knife_away, "knife goes");
    span(ui, &mut s.tilt, "hand tipped out to bleed");
    span(ui, &mut s.hand_away, "hand goes");
    vec3(ui, &mut s.palm_at, "palm's middle");
    vec3(ui, &mut s.fingers, "fingers point");
    vec3(ui, &mut s.palm_facing, "palm faces");
    vec3(ui, &mut s.hidden, "out of view by");
    ui.add(egui::Slider::new(&mut s.palm_surface, 0.0f32..=0.05).text("palm's skin off its bones (m)"));
    ui.add(egui::Slider::new(&mut s.forearm_twist, 0.0f32..=1.0).text("hand's roll taken by the forearm"));
    ui.add(egui::Slider::new(&mut s.elbow_drop, -1.0f32..=1.5).text("cut hand's elbow drops (forearm lengths)"));
    vec3(ui, &mut s.knife_elbow, "knife arm's elbow, from the hand (forearm lengths)");
    ui.add(egui::Slider::new(&mut s.cut_len, 0.0f32..=0.15).text("cut length (m)"));
    ui.add(egui::Slider::new(&mut s.cut_turn_deg, -60.0f32..=60.0).text("cut turned off square (°)"));
    ui.add(egui::Slider::new(&mut s.blade_from, 0.0f32..=1.0).text("blade touches from (0 guard, 1 tip)"));
    ui.add(egui::Slider::new(&mut s.blade_to, 0.0f32..=1.0).text("blade touches to"));
    ui.add(egui::Slider::new(&mut s.lift, 0.0f32..=0.2).text("blade comes down from (m)"));
    ui.add(egui::Slider::new(&mut s.press, 0.0f32..=0.02).text("blade presses in (m)"));
    ui.add(egui::Slider::new(&mut s.tilt_deg, -90.0f32..=90.0).text("hand tipped to bleed (°)"));
    ui.add(egui::Slider::new(&mut s.reach_out, 0.0f32..=0.2).text("hand reaches out to bleed (m)"));
    ui.add(egui::Slider::new(&mut s.cut_width, 0.0f32..=0.02).text("cut width (m)"));
    ui.add(egui::Slider::new(&mut s.pool_radius, 0.0f32..=0.06).text("blood pooling in the palm (m)"));
    ui.add(egui::Slider::new(&mut s.drops_per_sec, 0.0f32..=80.0).text("drops (/s)"));
    ui.add(egui::Slider::new(&mut s.drop_size, 0.0005f32..=0.015).text("drop size (m)"));
    ui.add(egui::Slider::new(&mut s.drop_gravity, 0.0f32..=15.0).text("drops fall (m/s²)"));
    ui.add(egui::Slider::new(&mut s.drop_life, 0.05f32..=2.0).text("drops last (s)"));
    ui.add(egui::Slider::new(&mut s.volume, 0.0f32..=3.0).text("cut volume"));
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *s = HandCutSettings::default();
        }
        if ui.button("Print settings to console").clicked() {
            info!("{s:#?}");
        }
    });
}
