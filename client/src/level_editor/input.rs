//! The level editor's viewport: the Blender-style orbit camera, clicking /
//! box-selecting, moving and turning things (G / R), the shortcuts, and the
//! boxes, arrows and ranges drawn over it all.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use shared::level::{Placement, ZombiesLayout};

use super::{Doc, Editor, EditorGizmos, ObjectId};
use crate::player::{PlayerHead, WorldModelCamera};
use crate::power::PowerLeverSettings;
use crate::{AppState, Player};

/// Closest to straight up / down the camera looks.
const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.001;
/// Orbit speed (radians per pixel of mouse travel).
const ORBIT_PER_PX: f32 = 0.006;
/// One numpad step (2 4 6 8).
const VIEW_STEP: f32 = PI / 12.0;
/// How far a drag must go (px) before it's a drag rather than a click.
const DRAG_PX: f32 = 4.0;
/// Ctrl snapping: metres, degrees, and scale steps.
const SNAP_M: f32 = 0.25;
const SNAP_DEG: f32 = 15.0;
const SNAP_SCALE: f32 = 0.05;
/// The smallest and biggest anything's scaled.
pub(crate) const MIN_SCALE: f32 = 0.05;
pub(crate) const MAX_SCALE: f32 = 20.0;
/// Furthest (m) a click or a move looks for the map.
const RAY_M: f32 = 2_000.0;

// --- the camera -------------------------------------------------------------------

/// The orbit camera: it looks at `focus` from `distance` away, turned `yaw`
/// about the vertical (0 looks along -Z) and tipped `pitch` (negative looks
/// down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OrbitCam {
    pub(crate) focus: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
}

impl Default for OrbitCam {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            yaw: PI / 4.0,
            pitch: -0.6,
            distance: 40.0,
        }
    }
}

impl OrbitCam {
    pub(crate) fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    pub(crate) fn eye(&self) -> Vec3 {
        self.focus + self.rotation() * Vec3::Z * self.distance
    }

    pub(crate) fn global(&self) -> GlobalTransform {
        GlobalTransform::from(Transform::from_translation(self.eye()).with_rotation(self.rotation()))
    }

    fn clamped(mut self) -> Self {
        self.pitch = self.pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.distance = self.distance.clamp(0.5, 3_000.0);
        self.yaw = wrap_angle(self.yaw);
        self
    }

    /// `t` of the way to `to` (turning the short way round).
    fn lerp(&self, to: &OrbitCam, t: f32) -> OrbitCam {
        OrbitCam {
            focus: self.focus.lerp(to.focus, t),
            yaw: self.yaw + wrap_angle(to.yaw - self.yaw) * t,
            pitch: self.pitch + (to.pitch - self.pitch) * t,
            distance: self.distance + (to.distance - self.distance) * t,
        }
    }
}

fn wrap_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

fn wrap_deg(a: f32) -> f32 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// A view that takes in all of `ids`, from the current angle.
pub(crate) fn frame(editor: &Editor, ids: &[ObjectId], lever: &PowerLeverSettings) -> OrbitCam {
    let doc = editor.doc();
    let points: Vec<Vec3> = ids
        .iter()
        .filter_map(|&id| {
            let (center, _, half) = id.world_box(id.get(doc)?, lever);
            Some([center - half.length(), center + half.length()])
        })
        .flatten()
        .collect();
    let mut cam = editor.cam_goto.unwrap_or(editor.cam);
    if points.is_empty() {
        return cam;
    }
    let min = points.iter().copied().reduce(Vec3::min).unwrap_or_default();
    let max = points.iter().copied().reduce(Vec3::max).unwrap_or_default();
    cam.focus = (min + max) * 0.5;
    // Far enough back to fit it (at a 90° view, with a margin).
    cam.distance = ((max - min).length() * 0.5 * 1.4).max(4.0);
    cam.clamped()
}

/// Mouse and keys that move the camera: orbit / pan / zoom (middle mouse, or
/// Alt + left), the wheel, the numpad's views and framing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn camera_input(
    mut editor: ResMut<Editor>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    window: Single<&Window, With<PrimaryWindow>>,
    projection: Single<&Projection, With<WorldModelCamera>>,
    lever: Res<PowerLeverSettings>,
    mut dragging: Local<bool>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let fov = match *projection {
        Projection::Perspective(ref p) => p.fov,
        _ => FRAC_PI_2,
    };

    // A drag that started over the viewport keeps going over a panel.
    let middle = mouse.pressed(MouseButton::Middle) || (alt && mouse.pressed(MouseButton::Left));
    let started = mouse.just_pressed(MouseButton::Middle) || (alt && mouse.just_pressed(MouseButton::Left));
    if started && !egui.is_pointer_over_area() {
        *dragging = true;
    }
    if !middle {
        *dragging = false;
    }
    let mut cam = editor.cam;
    let mut moved = false;
    let per_px = 2.0 * cam.distance * (fov * 0.5).tan() / window.height().max(1.0);
    if *dragging && motion.delta != Vec2::ZERO {
        let d = motion.delta;
        if shift {
            let rot = cam.rotation();
            cam.focus += (rot * Vec3::NEG_X * d.x + rot * Vec3::Y * d.y) * per_px;
        } else if ctrl {
            cam.distance *= (d.y * 0.01).exp();
        } else {
            cam.yaw -= d.x * ORBIT_PER_PX;
            cam.pitch -= d.y * ORBIT_PER_PX;
        }
        moved = true;
    }
    if scroll.delta != Vec2::ZERO && !egui.is_pointer_over_area() {
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        let rot = cam.rotation();
        if ctrl {
            cam.focus += rot * Vec3::NEG_X * lines * 40.0 * per_px;
        } else if shift {
            cam.focus += rot * Vec3::Y * lines * 40.0 * per_px;
        } else {
            cam.distance *= 0.85f32.powf(lines);
        }
        moved = true;
    }
    if moved {
        editor.cam_goto = None;
        editor.cam = cam.clamped();
    }

    // Views — not while typing in a panel or a number for a move.
    if egui.wants_any_keyboard_input() || editor.op.is_some() {
        return;
    }
    let pressed = |numpad: KeyCode, top_row: KeyCode| keys.just_pressed(numpad) || keys.just_pressed(top_row);
    let mut goto = editor.cam_goto.unwrap_or(editor.cam);
    let mut changed = true;
    if pressed(KeyCode::Numpad1, KeyCode::Digit1) {
        (goto.yaw, goto.pitch) = (if ctrl { PI } else { 0.0 }, 0.0);
    } else if pressed(KeyCode::Numpad3, KeyCode::Digit3) {
        (goto.yaw, goto.pitch) = (if ctrl { -FRAC_PI_2 } else { FRAC_PI_2 }, 0.0);
    } else if pressed(KeyCode::Numpad7, KeyCode::Digit7) {
        (goto.yaw, goto.pitch) = (0.0, if ctrl { PITCH_LIMIT } else { -PITCH_LIMIT });
    } else if pressed(KeyCode::Numpad9, KeyCode::Digit9) {
        (goto.yaw, goto.pitch) = (goto.yaw + PI, -goto.pitch);
    } else if pressed(KeyCode::Numpad4, KeyCode::Digit4) {
        goto.yaw -= VIEW_STEP;
    } else if pressed(KeyCode::Numpad6, KeyCode::Digit6) {
        goto.yaw += VIEW_STEP;
    } else if pressed(KeyCode::Numpad8, KeyCode::Digit8) {
        goto.pitch += VIEW_STEP;
    } else if pressed(KeyCode::Numpad2, KeyCode::Digit2) {
        goto.pitch -= VIEW_STEP;
    } else if keys.just_pressed(KeyCode::NumpadDecimal) || keys.just_pressed(KeyCode::KeyF) {
        let ids = if editor.selected.is_empty() { editor.objects() } else { editor.selected.clone() };
        goto = frame(&editor, &ids, &lever);
    } else if keys.just_pressed(KeyCode::Home) {
        goto = frame(&editor, &editor.objects(), &lever);
    } else {
        changed = false;
    }
    if changed {
        editor.cam_goto = Some(goto.clamped());
    }
}

/// Ease the camera toward a view change, and stand the player rig where the
/// camera is: the player turned to its yaw, the head tipped to its pitch.
pub(crate) fn drive_camera(
    time: Res<Time>,
    mut editor: ResMut<Editor>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    if let Some(goto) = editor.cam_goto {
        let t = 1.0 - (-time.delta_secs() * 14.0).exp();
        let next = editor.cam.lerp(&goto, t);
        let done = next.focus.distance(goto.focus) < 0.01
            && wrap_angle(next.yaw - goto.yaw).abs() < 0.001
            && (next.pitch - goto.pitch).abs() < 0.001
            && (next.distance - goto.distance).abs() < 0.01;
        editor.cam = if done { goto } else { next };
        if done {
            editor.cam_goto = None;
        }
    }
    let cam = editor.cam;
    player.translation = cam.eye();
    player.rotation = Quat::from_rotation_y(cam.yaw);
    head.rotation = Quat::from_rotation_x(cam.pitch);
}

// --- picking ------------------------------------------------------------------------

/// Where along `ray` (m) it first enters a box at `center`, turned `rot`,
/// `half` its size.
fn ray_box(ray: Ray3d, center: Vec3, rot: Quat, half: Vec3) -> Option<f32> {
    let inv = rot.inverse();
    let o = inv * (ray.origin - center);
    let d = inv * *ray.direction;
    let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
    for i in 0..3 {
        if d[i].abs() < 1e-8 {
            if o[i].abs() > half[i] {
                return None;
            }
            continue;
        }
        let a = (-half[i] - o[i]) / d[i];
        let b = (half[i] - o[i]) / d[i];
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far && far >= 0.0).then_some(near.max(0.0))
}

/// The shown thing nearest along `ray`.
fn pick(editor: &Editor, ray: Ray3d, lever: &PowerLeverSettings) -> Option<ObjectId> {
    let doc = editor.doc();
    editor
        .objects()
        .into_iter()
        .filter_map(|id| {
            let (center, rot, half) = id.world_box(id.get(doc)?, lever);
            ray_box(ray, center, rot, half).map(|t| (id, t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The map's surface along `ray`, if it meets it.
fn hit_map(rapier: &ReadRapierContext, origin: Vec3, dir: Vec3, max: f32) -> Option<Vec3> {
    let ctx = rapier.single().ok()?;
    ctx.cast_ray(origin, dir, max, true, QueryFilter::default())
        .map(|(_, t)| origin + dir * t)
}

/// The ground under `pos` (looking down from a little above it, so a thing
/// sunk into the floor still finds it).
fn ground_below(rapier: &ReadRapierContext, pos: Vec3) -> Option<Vec3> {
    hit_map(rapier, pos + Vec3::Y * 1.0, Vec3::NEG_Y, 500.0)
}

fn to_screen(cam: &GlobalTransform, camera: &Camera, pos: Vec3) -> Option<Vec2> {
    camera.world_to_viewport(cam, pos).ok()
}

// --- moving and turning --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OpKind {
    Move,
    Turn,
    /// Bigger / smaller — not the stand-ins, which are the yardstick.
    Scale,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub(crate) fn vec(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            Axis::X => Color::srgb(1.0, 0.25, 0.3),
            Axis::Y => Color::srgb(0.3, 0.55, 1.0),
            Axis::Z => Color::srgb(0.45, 0.9, 0.2),
        }
    }
}

/// A move, turn or scale in progress (G / R / S, or dragging a selected
/// thing).
pub(crate) struct Op {
    pub(crate) kind: OpKind,
    pub(crate) axis: Option<Axis>,
    /// The layout before it — put back on cancel, an undo step on confirm.
    /// (The stand-ins go back to `starts`.)
    before: ZombiesLayout,
    /// Each selected thing's placement at the start; the last is the
    /// active one, which the cursor steers.
    starts: Vec<(ObjectId, Placement)>,
    cursor_start: Vec2,
    /// From the cursor to the active thing's base on screen at the start —
    /// kept, so it doesn't jump to the cursor.
    grab_offset: Vec2,
    /// A turn's angle so far (degrees, unwrapped) and the cursor's angle
    /// last frame.
    turned: f32,
    last_angle: Option<f32>,
    /// A typed amount (metres along the axis, degrees, or a scale factor).
    pub(crate) typed: String,
    /// Started by dragging: it ends when the button's let go.
    from_drag: bool,
}

impl Op {
    pub(crate) fn active_start(&self) -> Placement {
        self.starts.last().map(|s| s.1).unwrap_or_default()
    }

    /// The typed amount, if it's a number yet.
    fn typed_value(&self) -> Option<f32> {
        self.typed.parse::<f32>().ok()
    }

    /// What the status bar says about it.
    pub(crate) fn describe(&self, doc: &Doc) -> String {
        let axis = self.axis.map_or(String::new(), |a| format!(" along {a:?}"));
        let typed = if self.typed.is_empty() { String::new() } else { format!("  [{}]", self.typed) };
        let Some(&(id, start)) = self.starts.last() else { return String::new() };
        let now = id.get(doc).unwrap_or(start);
        match self.kind {
            OpKind::Move => {
                let d = now.pos - start.pos;
                format!("Move{axis}  Δ ({:.2}, {:.2}, {:.2}) m{typed}", d.x, d.y, d.z)
            }
            OpKind::Turn => format!("Turn  {:.1}°{typed}", wrap_deg(now.yaw_deg - start.yaw_deg)),
            OpKind::Scale => format!("Scale  ×{:.3}  (now {:.3}){typed}", now.scale / start.scale.max(1e-4), now.scale),
        }
    }
}

/// Start moving / turning the selection.
fn start_op(editor: &mut Editor, kind: OpKind, cursor: Vec2, camera: &Camera, from_drag: bool) {
    let cam = editor.cam.global();
    let doc = editor.doc();
    let layout = &doc.layout;
    let starts: Vec<(ObjectId, Placement)> = editor
        .selected
        .iter()
        // (The stand-ins and wall buys keep their size.)
        .filter(|id| kind != OpKind::Scale || id.scalable())
        .filter_map(|&id| id.get(doc).map(|at| (id, at)))
        .collect();
    let Some(&(_, active)) = starts.last() else { return };
    let grab_offset = to_screen(&cam, camera, active.pos).map_or(Vec2::ZERO, |s| s - cursor);
    editor.op = Some(Op {
        kind,
        axis: None,
        before: layout.clone(),
        starts,
        cursor_start: cursor,
        grab_offset,
        turned: 0.0,
        last_angle: None,
        typed: String::new(),
        from_drag,
    });
}

/// Drop the move / turn in progress, putting everything back.
pub(crate) fn cancel_op(editor: &mut Editor) {
    if let Some(op) = editor.op.take() {
        let doc = editor.doc_mut();
        doc.layout = op.before;
        for (id, start) in op.starts.into_iter().filter(|(id, _)| id.is_reference()) {
            id.set(doc, start);
        }
    }
}

fn confirm_op(editor: &mut Editor) {
    if let Some(op) = editor.op.take() {
        editor.doc_mut().checkpoint(op.before);
    }
}

/// The digit, `.` or `-` just typed, if any.
fn typed_char(keys: &ButtonInput<KeyCode>) -> Option<char> {
    const DIGITS: [(KeyCode, KeyCode, char); 10] = [
        (KeyCode::Digit0, KeyCode::Numpad0, '0'),
        (KeyCode::Digit1, KeyCode::Numpad1, '1'),
        (KeyCode::Digit2, KeyCode::Numpad2, '2'),
        (KeyCode::Digit3, KeyCode::Numpad3, '3'),
        (KeyCode::Digit4, KeyCode::Numpad4, '4'),
        (KeyCode::Digit5, KeyCode::Numpad5, '5'),
        (KeyCode::Digit6, KeyCode::Numpad6, '6'),
        (KeyCode::Digit7, KeyCode::Numpad7, '7'),
        (KeyCode::Digit8, KeyCode::Numpad8, '8'),
        (KeyCode::Digit9, KeyCode::Numpad9, '9'),
    ];
    for (a, b, c) in DIGITS {
        if keys.just_pressed(a) || keys.just_pressed(b) {
            return Some(c);
        }
    }
    if keys.just_pressed(KeyCode::Period) || keys.just_pressed(KeyCode::NumpadDecimal) {
        return Some('.');
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        return Some('-');
    }
    None
}

/// Run the move / turn on by a frame: axis keys, typing, the cursor.
fn update_op(
    editor: &mut Editor,
    keys: &ButtonInput<KeyCode>,
    cursor: Vec2,
    camera: &Camera,
    rapier: &ReadRapierContext,
) {
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let cam = editor.cam.global();
    let Some(op) = editor.op.as_mut() else { return };

    // Axis locks (the same one again lets go), and typing an amount.
    if op.kind == OpKind::Move {
        for (key, axis) in [(KeyCode::KeyX, Axis::X), (KeyCode::KeyY, Axis::Y), (KeyCode::KeyZ, Axis::Z)] {
            if keys.just_pressed(key) {
                op.axis = (op.axis != Some(axis)).then_some(axis);
            }
        }
    }
    if let Some(c) = typed_char(keys) {
        if c == '-' {
            op.typed = match op.typed.strip_prefix('-') {
                Some(rest) => rest.to_string(),
                None => format!("-{}", op.typed),
            };
        } else {
            op.typed.push(c);
        }
    }
    if keys.just_pressed(KeyCode::Backspace) {
        op.typed.pop();
    }

    let active = op.active_start();
    let mut delta_pos = Vec3::ZERO;
    let mut delta_yaw = 0.0;
    let mut factor = 1.0;
    match op.kind {
        OpKind::Move => match (op.axis, op.typed_value()) {
            (Some(axis), Some(v)) => delta_pos = axis.vec() * v,
            (Some(axis), None) => {
                // Along the axis as it lies on screen.
                let a = to_screen(&cam, camera, active.pos);
                let b = to_screen(&cam, camera, active.pos + axis.vec());
                if let (Some(a), Some(b)) = (a, b) {
                    let along = b - a;
                    let px_per_m = along.length().max(1.0);
                    let mut m = (cursor - op.cursor_start).dot(along / px_per_m) / px_per_m;
                    if ctrl {
                        m = (m / SNAP_M).round() * SNAP_M;
                    }
                    delta_pos = axis.vec() * m;
                }
            }
            (None, _) => {
                // Onto whatever's under the cursor (kept where it was on
                // screen); missing the map, across the height it started at.
                if let Ok(ray) = camera.viewport_to_world(&cam, cursor + op.grab_offset) {
                    let dir = *ray.direction;
                    let target = hit_map(rapier, ray.origin, dir, RAY_M).or_else(|| {
                        (dir.y.abs() > 1e-4)
                            .then(|| (active.pos.y - ray.origin.y) / dir.y)
                            .filter(|t| *t > 0.0)
                            .map(|t| ray.origin + dir * t)
                    });
                    if let Some(mut target) = target {
                        if ctrl {
                            target.x = (target.x / SNAP_M).round() * SNAP_M;
                            target.z = (target.z / SNAP_M).round() * SNAP_M;
                        }
                        delta_pos = target - active.pos;
                    }
                }
            }
        },
        OpKind::Turn => {
            if let Some(v) = op.typed_value() {
                delta_yaw = v;
            } else {
                let pivot = to_screen(&cam, camera, active.pos + Vec3::Y).unwrap_or(op.cursor_start);
                let v = cursor - pivot;
                if v.length() > 2.0 {
                    // (Screen y runs down: counter-clockwise on screen is a
                    // left turn seen from above.)
                    let angle = (-v.y).atan2(v.x).to_degrees();
                    if let Some(last) = op.last_angle {
                        op.turned += wrap_deg(angle - last);
                    }
                    op.last_angle = Some(angle);
                }
                delta_yaw = op.turned;
                if ctrl {
                    delta_yaw = (delta_yaw / SNAP_DEG).round() * SNAP_DEG;
                }
            }
        }
        OpKind::Scale => {
            factor = match op.typed_value() {
                Some(v) => v,
                None => {
                    // How far the cursor is from the active thing's middle
                    // on screen, against how far it started.
                    let pivot = to_screen(&cam, camera, active.pos + Vec3::Y).unwrap_or(op.cursor_start);
                    let from = (op.cursor_start - pivot).length().max(8.0);
                    let mut f = (cursor - pivot).length() / from;
                    if ctrl {
                        f = (f / SNAP_SCALE).round() * SNAP_SCALE;
                    }
                    f
                }
            };
        }
    }
    let starts = op.starts.clone();
    let doc = editor.doc_mut();
    for (id, start) in starts {
        let scale = (start.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        id.set(
            doc,
            Placement::new(start.pos + delta_pos, wrap_deg(start.yaw_deg + delta_yaw)).with_scale(scale),
        );
    }
}

/// The view of `id` a player gets using it: from where they'd stand — just
/// in front of it, or a wall buy's use spot — at the game's eye height on
/// the ground there, looking at it (a wall buy's outline). The player
/// stand-in instead looks out of its own eyes, the way it faces. The camera
/// still orbits from there, round what it's looking at.
pub(crate) fn player_view(
    editor: &Editor,
    id: ObjectId,
    lever: &PowerLeverSettings,
    rapier: &ReadRapierContext,
) -> Option<OrbitCam> {
    let at = id.get(editor.doc())?;
    let (center, rot, half) = id.world_box(at, lever);
    let (eye, target) = match id {
        ObjectId::Reference(super::RefKind::Player) | ObjectId::PlayerSpawn(_) => {
            let eye = at.pos + Vec3::Y * crate::EYE_HEIGHT;
            (eye, eye + rot * Vec3::Z * 5.0)
        }
        _ => {
            let (stand, target) = match id {
                ObjectId::WallBuy(_) => (
                    shared::wall_buy::use_spot(at),
                    at.pos + rot * crate::wall_buys::DECAL_CENTER * at.scale,
                ),
                _ => (at.pos + rot * Vec3::Z * (half.z + 1.2), center),
            };
            let feet = ground_below(rapier, stand).unwrap_or(Vec3::new(stand.x, at.pos.y, stand.z));
            (feet + Vec3::Y * crate::EYE_HEIGHT, target)
        }
    };
    let to = target - eye;
    let distance = to.length().max(0.1);
    let d = to / distance;
    Some(OrbitCam {
        focus: target,
        yaw: f32::atan2(-d.x, -d.z),
        pitch: d.y.clamp(-1.0, 1.0).asin(),
        distance,
    })
}

// --- clicks and keys -----------------------------------------------------------------

/// Drop each selected thing onto the ground under it (undoable).
pub(crate) fn drop_to_ground(editor: &mut Editor, rapier: &ReadRapierContext, now: f32) {
    let before = editor.doc().layout.clone();
    let mut missed = 0;
    for id in editor.selected.clone() {
        let Some(mut at) = id.get(editor.doc()) else { continue };
        match ground_below(rapier, at.pos) {
            Some(ground) => {
                at.pos.y = ground.y;
                id.set(editor.doc_mut(), at);
            }
            None => missed += 1,
        }
    }
    editor.doc_mut().checkpoint(before);
    if missed > 0 {
        editor.toast(format!("No ground under {missed} of them"), now);
    }
}

/// Take the selected optional things off the map (undoable).
pub(crate) fn delete_selected(editor: &mut Editor, now: f32) {
    let before = editor.doc().layout.clone();
    let mut kept = false;
    // (Start spots last-first, so taking one out doesn't shift the rest
    // before they go.)
    let mut ids = editor.selected.clone();
    ids.sort_by_key(|id| std::cmp::Reverse(if let ObjectId::PlayerSpawn(i) = id { *i as i32 } else { -1 }));
    for id in ids {
        if !id.remove(editor.doc_mut()) {
            kept = true;
        }
    }
    editor.doc_mut().checkpoint(before);
    editor.prune_selection();
    if kept {
        editor.toast("Perk machines and Der Wunderfizz can't be removed — only moved", now);
    }
}

/// Put `id` (an optional thing that isn't there) on the map, on the ground
/// at the middle of the view, facing the camera, and select it.
pub(crate) fn add_at_view(editor: &mut Editor, id: ObjectId, rapier: &ReadRapierContext) {
    let focus = editor.cam.focus;
    let ground = hit_map(rapier, focus + Vec3::Y * 50.0, Vec3::NEG_Y, 500.0).unwrap_or(focus);
    let to_camera = editor.cam.eye() - ground;
    let yaw_deg = f32::atan2(to_camera.x, to_camera.z).to_degrees();
    let before = editor.doc().layout.clone();
    // (At its kind's size — every map's.)
    let size = id.size_key().map_or(1.0, |key| editor.sizes.get(&key));
    id.set(editor.doc_mut(), Placement::new(ground, yaw_deg).with_scale(size));
    editor.doc_mut().checkpoint(before);
    editor.selected = vec![id];
}

/// Select `id` — added to (or, if it's there, taken out of) the selection
/// with `extend`, otherwise on its own.
pub(crate) fn click_select(editor: &mut Editor, id: Option<ObjectId>, extend: bool) {
    match (id, extend) {
        (Some(id), true) => {
            // Already the active one: deselect it; otherwise make it active.
            let active = editor.active() == Some(id);
            editor.selected.retain(|&s| s != id);
            if !active {
                editor.selected.push(id);
            }
        }
        (Some(id), false) => editor.selected = vec![id],
        (None, true) => {}
        (None, false) => editor.selected.clear(),
    }
}

/// Everything the viewport does with the left / right mouse and the keys:
/// selecting, moving and turning, deleting, undo, saving, leaving.
#[allow(clippy::too_many_arguments)]
pub(crate) fn edit_input(
    mut editor: ResMut<Editor>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<&Camera, With<WorldModelCamera>>,
    lever: Res<PowerLeverSettings>,
    rapier: ReadRapierContext,
    mut next: ResMut<NextState<AppState>>,
    // Where the left button went down over the viewport, and on what.
    mut press: Local<Option<(Vec2, Option<ObjectId>)>>,
) {
    let now = time.elapsed_secs();
    let cursor = window.cursor_position();
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let typing = egui.wants_any_keyboard_input();
    let over_ui = egui.is_pointer_over_area();
    let cam = editor.cam.global();

    // Leaving: Esc (with nothing to cancel) and the mouse back button do
    // what EXIT does.
    let back = mouse.just_pressed(MouseButton::Back);
    let esc = keys.just_pressed(KeyCode::Escape) && !typing;
    if back || esc {
        if editor.op.is_some() {
            cancel_op(&mut editor);
        } else if editor.box_from.take().is_some() {
            *press = None;
        } else if editor.exit_prompt {
            editor.exit_prompt = false;
        } else if editor.help_open && esc {
            editor.help_open = false;
        } else if editor.request_exit() {
            next.set(AppState::MainMenu);
        }
        return;
    }
    if editor.exit_prompt {
        return;
    }

    // A move / turn in progress has the mouse and keys to itself.
    if editor.op.is_some() {
        let Some(cursor) = cursor else { return };
        update_op(&mut editor, &keys, cursor, &camera, &rapier);
        let from_drag = editor.op.as_ref().is_some_and(|o| o.from_drag);
        if mouse.just_pressed(MouseButton::Right) {
            cancel_op(&mut editor);
        } else if keys.just_pressed(KeyCode::Enter)
            || keys.just_pressed(KeyCode::NumpadEnter)
            || (!from_drag && mouse.just_pressed(MouseButton::Left))
            || (from_drag && !mouse.pressed(MouseButton::Left))
        {
            confirm_op(&mut editor);
        }
        return;
    }

    // Left mouse: click to select, drag a selected thing to move it, drag
    // on anything else to box-select. (Alt + left is the camera's.)
    if let Some(cursor) = cursor {
        if mouse.just_pressed(MouseButton::Left) && !over_ui && !alt {
            let hit = camera
                .viewport_to_world(&cam, cursor)
                .ok()
                .and_then(|ray| pick(&editor, ray, &lever));
            *press = Some((cursor, hit));
        }
        if let Some((from, hit)) = *press {
            if mouse.pressed(MouseButton::Left) {
                if editor.box_from.is_none() && cursor.distance(from) > DRAG_PX {
                    match hit {
                        Some(id) => {
                            if !editor.selected.contains(&id) {
                                editor.selected = vec![id];
                            } else {
                                // (The one grabbed leads.)
                                editor.selected.retain(|&s| s != id);
                                editor.selected.push(id);
                            }
                            start_op(&mut editor, OpKind::Move, from, &camera, true);
                            *press = None;
                        }
                        None => editor.box_from = Some(from),
                    }
                }
            } else {
                *press = None;
                match editor.box_from.take() {
                    Some(start) => {
                        // Everything whose middle is inside the box.
                        let (lo, hi) = (start.min(cursor), start.max(cursor));
                        let doc = editor.doc();
                        let inside: Vec<ObjectId> = editor
                            .objects()
                            .into_iter()
                            .filter(|id| {
                                id.get(doc).is_some_and(|at| {
                                    let (center, ..) = id.world_box(at, &lever);
                                    to_screen(&cam, &camera, center)
                                        .is_some_and(|p| p.cmpge(lo).all() && p.cmple(hi).all())
                                })
                            })
                            .collect();
                        if ctrl {
                            editor.selected.retain(|s| !inside.contains(s));
                        } else {
                            if !shift {
                                editor.selected.clear();
                            }
                            for id in inside {
                                if !editor.selected.contains(&id) {
                                    editor.selected.push(id);
                                }
                            }
                        }
                    }
                    None => click_select(&mut editor, hit, shift),
                }
            }
        }
    }

    if typing {
        return;
    }
    let cursor_or_middle = cursor.unwrap_or(Vec2::new(window.width(), window.height()) * 0.5);
    if ctrl && keys.just_pressed(KeyCode::KeyZ) {
        if shift {
            editor.redo(now);
        } else {
            editor.undo(now);
        }
    } else if ctrl && keys.just_pressed(KeyCode::KeyY) {
        editor.redo(now);
    } else if ctrl && keys.just_pressed(KeyCode::KeyS) {
        let map = editor.map;
        editor.save(map, now);
    } else if ctrl {
        // (Nothing else takes Ctrl.)
    } else if keys.just_pressed(KeyCode::F1) {
        editor.help_open = !editor.help_open;
    } else if keys.just_pressed(KeyCode::KeyA) {
        if alt {
            editor.selected.clear();
        } else {
            editor.selected = editor.objects();
        }
    } else if keys.just_pressed(KeyCode::Numpad0) || keys.just_pressed(KeyCode::Digit0) {
        match editor.active().and_then(|id| player_view(&editor, id, &lever, &rapier)) {
            Some(view) => editor.cam_goto = Some(view),
            None => editor.toast("Select something to see it as a player would", now),
        }
    } else if editor.selected.is_empty() {
        // (The rest work on the selection.)
    } else if keys.just_pressed(KeyCode::KeyG) {
        start_op(&mut editor, OpKind::Move, cursor_or_middle, &camera, false);
    } else if keys.just_pressed(KeyCode::KeyR) {
        start_op(&mut editor, OpKind::Turn, cursor_or_middle, &camera, false);
    } else if keys.just_pressed(KeyCode::KeyS) {
        start_op(&mut editor, OpKind::Scale, cursor_or_middle, &camera, false);
    } else if keys.just_pressed(KeyCode::KeyX) || keys.just_pressed(KeyCode::Delete) {
        delete_selected(&mut editor, now);
    } else if keys.just_pressed(KeyCode::End) {
        drop_to_ground(&mut editor, &rapier, now);
    }
}

// --- drawing ---------------------------------------------------------------------------

/// The active selection's colour (Blender's), the rest of the selection's,
/// and everything else's.
pub(crate) const ACTIVE: Color = Color::srgb(1.0, 0.75, 0.25);
pub(crate) const SELECTED: Color = Color::srgb(0.95, 0.45, 0.1);

/// The floor grid, each thing's box and facing, the selection's use ranges,
/// and an axis lock's line.
pub(crate) fn draw_gizmos(
    editor: Res<Editor>,
    lever: Res<PowerLeverSettings>,
    mut grid: Gizmos,
    mut gizmos: Gizmos<EditorGizmos>,
) {
    if editor.docs.is_empty() {
        return;
    }
    let flat = Quat::from_rotation_x(-FRAC_PI_2);
    if editor.view.grid {
        grid.grid(
            Isometry3d::new(Vec3::ZERO, flat),
            UVec2::splat(200),
            Vec2::splat(1.0),
            Color::srgba(1.0, 1.0, 1.0, 0.07),
        );
        grid.line(Vec3::new(-100.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 0.0), Axis::X.color().with_alpha(0.6));
        grid.line(Vec3::new(0.0, 0.0, -100.0), Vec3::new(0.0, 0.0, 100.0), Axis::Z.color().with_alpha(0.6));
    }
    let doc = editor.doc();
    let active = editor.active();
    for id in editor.objects() {
        let Some(at) = id.get(doc) else { continue };
        let selected = editor.selected.contains(&id);
        let color = if active == Some(id) {
            ACTIVE
        } else if selected {
            SELECTED
        } else {
            id.color().with_alpha(0.45)
        };
        let (center, rot, half) = id.world_box(at, &lever);
        gizmos.cuboid(
            Transform::from_translation(center).with_rotation(rot).with_scale(half * 2.0),
            color,
        );
        // Which way it faces (its front, +Z), along the ground.
        let base = at.pos + Vec3::Y * 0.05;
        gizmos.arrow(base, base + rot * Vec3::Z * (half.z + 1.0), color);
        // The exfil area: its rectangle, always.
        if id == ObjectId::ExfilArea {
            if let Some(area) = doc.layout.exfil_area {
                let c = area.corners().map(|p| p + Vec3::Y * 0.05);
                let line = if selected { color } else { id.color() };
                for i in 0..4 {
                    gizmos.line(c[i], c[(i + 1) % 4], line);
                }
            }
            continue;
        }
        if selected || editor.view.ranges {
            gizmos.circle(
                Isometry3d::new(id.use_center(at) + Vec3::Y * 0.05, flat),
                id.use_radius(),
                id.color().with_alpha(if selected { 0.9 } else { 0.4 }),
            );
        }
    }
    if let Some(op) = &editor.op {
        if let Some(axis) = op.axis {
            let from = op.active_start().pos;
            gizmos.line(from - axis.vec() * 500.0, from + axis.vec() * 500.0, axis.color());
        }
    }
}
