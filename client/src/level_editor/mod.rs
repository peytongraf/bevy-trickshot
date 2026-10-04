//! The `Zombies` level editor — main menu → LEVEL EDITOR
//! ([`AppState::LevelEditor`]). It places everything a `Zombies` game puts on
//! a map (the perk machines, Der Wunderfizz, the Pack-a-Punch, the ammo crate,
//! the power switch, the wall buys) and saves each map's layout to its file in
//! `shared/levels/` (`shared::level`), which the client and the server both
//! build in — rebuild them to play the new layout.
//!
//! Offline and game-free: no lobby, no server, no rounds, nothing to win or
//! lose. The camera orbits like Blender's viewport, never walks:
//!
//! * middle mouse drag orbits, Shift + it pans, Ctrl + it (or the wheel)
//!   zooms — or Alt + left mouse for all three, without a middle button;
//! * numpad 1 / 3 / 7 (or the top row's) look from the front / right / top
//!   (Ctrl for the opposite side), 2 4 6 8 step around, `.` or `F` frames the
//!   selection, Home frames everything;
//! * left click selects (Shift adds), dragging on empty space box-selects,
//!   dragging a thing moves it, A / Alt+A select all / none;
//! * G moves (along the surface under the cursor — X / Y / Z lock it to an
//!   axis), R turns, type a number for an exact amount, Ctrl snaps, left
//!   click / Enter confirms, right click / Esc cancels;
//! * End drops onto the ground below, X / Delete removes, Ctrl+Z / Ctrl+Shift+Z
//!   undo / redo, Ctrl+S saves; F1 lists all of it.
//!
//! Esc (with nothing to cancel) and the mouse back button do what EXIT does:
//! leave — asking first if there's anything unsaved.
//!
//! Every bit of it lives in [`Editor`], rebuilt on the way in and dropped on
//! the way out; the player rig the camera borrows, the view model camera and
//! the lighting are put back as they were ([`exit`]); the scene is
//! `StateScoped`. Nothing carries into a game.
//!
//! **Anything new a `Zombies` game puts on a map needs adding here** (an
//! [`ObjectId`], its model and box) as well as to `shared::level`.

mod input;
mod ui;

use bevy::pbr::FogFalloff;
use bevy::prelude::*;
use bevy::render::view::RenderLayers;
use bevy_egui::EguiPrimaryContextPass;
use shared::level::{Placement, ZombiesLayout};
use shared::perks::{Perk, PerkSet};
use shared::weapon::WeaponId;
use shared::MapId;

use crate::ammo_crate::{AmmoCrateSettings, AMMO_CRATE_MODEL};
use crate::pap::{PapSettings, PAP_MODEL};
use crate::player::{PlayerBodyCapsule, PlayerHead, ViewModelCamera, WorldModelCamera};
use crate::power::{PowerLeverSettings, LEVER_MODEL};
use crate::weapons::{CameraRecoil, CameraShake};
use crate::zombies_hud::{machine_model, perk_color, PerkMachineSettings, WUNDERFIZZ_MODEL};
use crate::{AppState, CurrentMap, Player};


pub(crate) struct LevelEditorPlugin;

impl Plugin for LevelEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editor>()
            .init_resource::<RigBackup>()
            .init_resource::<LightingBackup>()
            .init_gizmo_group::<EditorGizmos>()
            .add_systems(Startup, configure_gizmos)
            .add_systems(OnEnter(AppState::LevelEditor), enter)
            .add_systems(OnExit(AppState::LevelEditor), exit)
            .add_systems(
                Update,
                (
                    // The world's look, which only `InGame` keeps up otherwise.
                    (
                        crate::apply_map_transform,
                        crate::apply_shipment_transform,
                        (crate::apply_scene_tuning, apply_editor_lighting).chain(),
                        crate::sky_follow_camera,
                    ),
                    (
                        input::camera_input,
                        input::edit_input,
                        input::drive_camera,
                        sync_objects,
                        input::draw_gizmos,
                    )
                        .chain(),
                )
                    .run_if(in_state(AppState::LevelEditor)),
            )
            .add_systems(EguiPrimaryContextPass, ui::editor_ui.run_if(in_state(AppState::LevelEditor)));
    }
}

/// The selection boxes, facing arrows, ranges and axis lines — drawn over
/// the map (so a machine behind a wall still shows where it is).
#[derive(Default, Reflect, GizmoConfigGroup)]
struct EditorGizmos;

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<EditorGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 2.0;
    // (Only the world camera — not the HUD one.)
    config.render_layers = RenderLayers::layer(0);
}

// --- what's edited ------------------------------------------------------------

/// One thing the editor places.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum ObjectId {
    Perk(Perk),
    Wunderfizz,
    PackAPunch,
    AmmoCrate,
    PowerSwitch,
    /// The wall buy selling this gun.
    WallBuy(WeaponId),
    /// A stand-in model, just to judge sizes against — never saved.
    Reference(RefKind),
}

/// The stand-ins ([`ObjectId::Reference`]): a player (another player's
/// soldier, as they look in game) and a zombie.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum RefKind {
    Player,
    Zombie,
}

impl RefKind {
    fn index(self) -> usize {
        self as usize
    }
}

impl ObjectId {
    /// The optional ones — a map can do without them.
    pub(crate) const OPTIONAL: [ObjectId; 5] = [
        ObjectId::PackAPunch,
        ObjectId::AmmoCrate,
        ObjectId::PowerSwitch,
        ObjectId::WallBuy(WeaponId::Sniper),
        ObjectId::WallBuy(WeaponId::Ak74),
    ];

    /// The stand-ins.
    pub(crate) const REFERENCES: [ObjectId; 2] =
        [ObjectId::Reference(RefKind::Player), ObjectId::Reference(RefKind::Zombie)];

    pub(crate) fn label(self) -> &'static str {
        match self {
            ObjectId::Perk(p) => p.label(),
            ObjectId::Wunderfizz => "Der Wunderfizz",
            ObjectId::PackAPunch => "Pack-a-Punch",
            ObjectId::AmmoCrate => "Ammo crate",
            ObjectId::PowerSwitch => "Power switch",
            ObjectId::WallBuy(WeaponId::Ak74) => "AK-74 wall buy",
            ObjectId::WallBuy(_) => "Sniper wall buy",
            ObjectId::Reference(RefKind::Player) => "Player (reference)",
            ObjectId::Reference(RefKind::Zombie) => "Zombie (reference)",
        }
    }

    /// Whether its size can be changed — not the stand-ins (the yardstick)
    /// or the wall buys (every sign's one size, `shared::wall_buy::SIGN_SCALE`).
    pub(crate) fn scalable(self) -> bool {
        !matches!(self, ObjectId::Reference(_) | ObjectId::WallBuy(_))
    }

    /// A stand-in, not part of the layout.
    pub(crate) fn is_reference(self) -> bool {
        matches!(self, ObjectId::Reference(_))
    }

    /// Whether it can be added and removed (everything but the machines).
    pub(crate) fn is_optional(self) -> bool {
        Self::OPTIONAL.contains(&self) || self.is_reference()
    }

    fn slot(self, layout: &mut ZombiesLayout) -> Option<&mut Option<Placement>> {
        match self {
            ObjectId::PackAPunch => Some(&mut layout.pack_a_punch),
            ObjectId::AmmoCrate => Some(&mut layout.ammo_crate),
            ObjectId::PowerSwitch => Some(&mut layout.power_switch),
            _ => None,
        }
    }

    /// Where it stands in `doc` (the layout, or the stand-ins).
    pub(crate) fn get(self, doc: &Doc) -> Option<Placement> {
        match self {
            ObjectId::Reference(kind) => doc.refs[kind.index()],
            other => other.get_in(&doc.layout),
        }
    }

    /// Move it in `doc` (or, for an optional one that isn't there, add it).
    pub(crate) fn set(self, doc: &mut Doc, at: Placement) {
        match self {
            ObjectId::Reference(kind) => doc.refs[kind.index()] = Some(at),
            other => other.set_in(&mut doc.layout, at),
        }
    }

    /// Take an optional one out of `doc`; `false` for one that can't go.
    fn remove(self, doc: &mut Doc) -> bool {
        match self {
            ObjectId::Reference(kind) => doc.refs[kind.index()].take().is_some(),
            other => other.remove_in(&mut doc.layout),
        }
    }

    fn get_in(self, layout: &ZombiesLayout) -> Option<Placement> {
        match self {
            ObjectId::Reference(_) => None,
            ObjectId::Perk(p) => layout.perks.get(&p).copied(),
            ObjectId::Wunderfizz => Some(layout.wunderfizz),
            ObjectId::PackAPunch => layout.pack_a_punch,
            ObjectId::AmmoCrate => layout.ammo_crate,
            ObjectId::PowerSwitch => layout.power_switch,
            ObjectId::WallBuy(gun) => layout.wall_buy(gun),
        }
    }

    fn set_in(self, layout: &mut ZombiesLayout, at: Placement) {
        match self {
            ObjectId::Perk(p) => {
                layout.perks.insert(p, at);
            }
            ObjectId::Wunderfizz => layout.wunderfizz = at,
            ObjectId::WallBuy(gun) => layout.set_wall_buy(gun, at),
            other => {
                if let Some(slot) = other.slot(layout) {
                    *slot = Some(at);
                }
            }
        }
    }

    fn remove_in(self, layout: &mut ZombiesLayout) -> bool {
        if let ObjectId::WallBuy(gun) = self {
            let before = layout.wall_buys.len();
            layout.wall_buys.retain(|w| w.weapon != gun);
            return layout.wall_buys.len() != before;
        }
        match self.slot(layout) {
            Some(slot) => slot.take().is_some(),
            None => false,
        }
    }

    /// Its colour in the viewport.
    fn color(self) -> Color {
        match self {
            ObjectId::Perk(p) => perk_color(p),
            ObjectId::Wunderfizz => Color::srgb(0.62, 0.22, 1.0),
            ObjectId::PackAPunch => crate::pap::PAP_BLUE,
            ObjectId::AmmoCrate => Color::srgb(0.55, 0.8, 0.35),
            ObjectId::PowerSwitch => Color::srgb(1.0, 0.85, 0.2),
            ObjectId::WallBuy(_) => Color::srgb(0.6, 1.0, 0.45),
            ObjectId::Reference(_) => Color::srgb(0.8, 0.85, 0.9),
        }
    }

    /// How close (m, across the ground) a player has to be to use it.
    pub(crate) fn use_radius(self) -> f32 {
        match self {
            ObjectId::PackAPunch => shared::perks::PERK_USE_RADIUS + 0.5,
            ObjectId::WallBuy(_) => shared::wall_buy::USE_RADIUS,
            _ => shared::perks::PERK_USE_RADIUS,
        }
    }

    /// The middle of where it's used from, standing at `at`.
    pub(crate) fn use_center(self, at: Placement) -> Vec3 {
        match self {
            ObjectId::WallBuy(_) => shared::wall_buy::use_spot(at),
            _ => at.pos,
        }
    }

    /// Its box standing at `at`, scale and all: `(centre, rotation, half
    /// extents)` — what's clicked and outlined.
    pub(crate) fn world_box(self, at: Placement, lever: &PowerLeverSettings) -> (Vec3, Quat, Vec3) {
        let (center, half) = self.bounds(lever);
        let rot = at.rotation();
        (at.pos + rot * center * at.scale, rot, half * at.scale)
    }

    /// Its box in its own frame (from the ground under its middle), as made:
    /// `(centre, half extents)`.
    fn bounds(self, lever: &PowerLeverSettings) -> (Vec3, Vec3) {
        let standing = |half: Vec3| (Vec3::Y * half.y, half);
        match self {
            ObjectId::Perk(_) | ObjectId::Wunderfizz => standing(shared::perks::MACHINE_HALF_EXTENTS),
            ObjectId::PackAPunch => standing(shared::pap::MACHINE_HALF_EXTENTS),
            ObjectId::AmmoCrate => standing(shared::ammo::CRATE_HALF_EXTENTS),
            // The lever, up on the wall, and down to the ground where it's
            // used from.
            // The sign: post and board.
            ObjectId::WallBuy(_) => (Vec3::new(-0.02, 1.3, 0.0), Vec3::new(1.0, 1.5, 0.15)),
            // A person: the in-game body's height.
            ObjectId::Reference(_) => standing(Vec3::new(0.35, crate::EYE_HEIGHT * 0.5 + 0.1, 0.25)),
            ObjectId::PowerSwitch => {
                let top = lever.offset.y + 0.4;
                (Vec3::new(lever.offset.x, top * 0.5, lever.offset.z), Vec3::new(0.35, top * 0.5, 0.25))
            }
        }
    }
}

/// What's on the map in `set`'s game, then the stand-ins, in outliner order.
pub(crate) fn objects(doc: &Doc, set: PerkSet) -> Vec<ObjectId> {
    let mut out: Vec<ObjectId> = set.machine_perks().map(ObjectId::Perk).collect();
    if set == PerkSet::Classic {
        out.push(ObjectId::Wunderfizz);
    }
    out.extend(
        ObjectId::OPTIONAL
            .into_iter()
            .chain(ObjectId::REFERENCES)
            .filter(|o| o.get(doc).is_some()),
    );
    out
}

/// Things standing close enough to be bought from the same spot — the same
/// check as `shared`'s tests (which fail on any of these), for both sets.
pub(crate) fn overlaps(layout: &ZombiesLayout) -> Vec<(ObjectId, ObjectId)> {
    let mut out = Vec::new();
    for set in PerkSet::ALL {
        let mut things: Vec<ObjectId> = set.machine_perks().map(ObjectId::Perk).collect();
        if set == PerkSet::Classic {
            things.push(ObjectId::Wunderfizz);
        }
        for (i, &a) in things.iter().enumerate() {
            for &b in &things[i + 1..] {
                let (Some(pa), Some(pb)) = (a.get_in(layout), b.get_in(layout)) else {
                    continue;
                };
                let close = shared::perks::in_range_of(pa.pos, pb.pos, 0.75)
                    || shared::perks::in_range_of(pb.pos, pa.pos, 0.75);
                if close && !out.contains(&(a, b)) {
                    out.push((a, b));
                }
            }
        }
    }
    out
}

// --- the editor's state ---------------------------------------------------------

/// One place's layout being edited, with its history.
pub(crate) struct Doc {
    pub(crate) layout: ZombiesLayout,
    /// As last saved (or loaded) — anything else is unsaved.
    pub(crate) saved: ZombiesLayout,
    /// As this build has it — a saved layout that differs needs a rebuild
    /// to play.
    pub(crate) built_in: ZombiesLayout,
    undo: Vec<ZombiesLayout>,
    redo: Vec<ZombiesLayout>,
    /// The stand-ins ([`RefKind`]) — the editor's alone: never saved, and
    /// gone once it's left.
    pub(crate) refs: [Option<Placement>; 2],
}

/// Most undo steps kept per map.
const UNDO_LIMIT: usize = 200;

impl Doc {
    /// `map`'s layout: the repo's file when there is one (it may be newer
    /// than this build), else the built-in one.
    fn load(map: MapId) -> Self {
        let built_in = shared::level::layout(map).clone();
        let mut layout = std::fs::read_to_string(shared::level::source_path(map))
            .ok()
            .and_then(|text| match ZombiesLayout::from_ron(&text) {
                Ok(l) => Some(l),
                Err(e) => {
                    warn!("level editor: couldn't read {}: {e}", shared::level::file_name(map));
                    None
                }
            })
            .unwrap_or_else(|| built_in.clone());
        // Every machine there, whatever the file says.
        for perk in Perk::ALL.into_iter().filter(|p| p.has_machine()) {
            if !layout.perks.contains_key(&perk) {
                layout.perks.insert(perk, built_in.perk(perk));
            }
        }
        Self {
            saved: layout.clone(),
            layout,
            built_in,
            undo: Vec::new(),
            redo: Vec::new(),
            refs: [None; 2],
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.layout != self.saved
    }

    /// Remember `before` as an undo step (the layout's already moved on).
    pub(crate) fn checkpoint(&mut self, before: ZombiesLayout) {
        if before == self.layout {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.layout, prev));
        true
    }

    fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.layout, next));
        true
    }
}

/// Viewport toggles (View menu).
pub(crate) struct ViewOptions {
    pub(crate) grid: bool,
    pub(crate) labels: bool,
    /// Every thing's use range, not just the selection's.
    pub(crate) ranges: bool,
    /// Lift the ambient light and the fog, so a night map's workable.
    pub(crate) bright: bool,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            grid: true,
            labels: true,
            ranges: false,
            bright: true,
        }
    }
}

/// Everything the editor knows. Reset on the way in ([`enter`]) and out.
#[derive(Resource, Default)]
pub(crate) struct Editor {
    /// The map shown (its day or night version; the layout's the place's).
    pub(crate) map: MapId,
    /// Which perk set's machines are shown.
    pub(crate) perk_set: PerkSet,
    /// One per place, in `MapId::PLACES` order.
    pub(crate) docs: Vec<Doc>,
    /// The selection; the last is the active one (the properties panel's).
    pub(crate) selected: Vec<ObjectId>,
    /// A move / turn in progress.
    pub(crate) op: Option<input::Op>,
    /// A box selection being dragged out: where it started.
    pub(crate) box_from: Option<Vec2>,
    pub(crate) cam: input::OrbitCam,
    /// Where a view change (frame, front / top / ...) is easing the camera.
    pub(crate) cam_goto: Option<input::OrbitCam>,
    pub(crate) view: ViewOptions,
    /// The "leave with unsaved changes?" window is up.
    pub(crate) exit_prompt: bool,
    pub(crate) help_open: bool,
    /// A message for the status bar, and when it goes (elapsed seconds).
    pub(crate) toast: Option<(String, f32)>,
    /// A properties-panel field is mid-edit (one undo step for the lot).
    pub(crate) inspector_editing: bool,
}

impl Editor {
    fn fresh() -> Self {
        let map = MapId::BreakPoint;
        Self {
            map,
            perk_set: PerkSet::Classic,
            docs: MapId::PLACES.iter().map(|&p| Doc::load(p)).collect(),
            ..default()
        }
    }

    fn doc_index(map: MapId) -> usize {
        let place = shared::level::place(map);
        MapId::PLACES.iter().position(|&p| p == place).unwrap_or(0)
    }

    pub(crate) fn doc(&self) -> &Doc {
        &self.docs[Self::doc_index(self.map)]
    }

    pub(crate) fn doc_mut(&mut self) -> &mut Doc {
        let i = Self::doc_index(self.map);
        &mut self.docs[i]
    }

    pub(crate) fn doc_for(&self, place: MapId) -> &Doc {
        &self.docs[Self::doc_index(place)]
    }

    pub(crate) fn any_dirty(&self) -> bool {
        self.docs.iter().any(Doc::dirty)
    }

    /// What's on the map, as shown.
    pub(crate) fn objects(&self) -> Vec<ObjectId> {
        objects(self.doc(), self.perk_set)
    }

    pub(crate) fn active(&self) -> Option<ObjectId> {
        self.selected.last().copied()
    }

    pub(crate) fn toast(&mut self, message: impl Into<String>, now: f32) {
        let message = message.into();
        info!("level editor: {message}");
        self.toast = Some((message, now + 6.0));
    }

    /// Drop whatever's selected that isn't on the map any more.
    pub(crate) fn prune_selection(&mut self) {
        let shown = self.objects();
        self.selected.retain(|o| shown.contains(o));
    }

    /// Switch map (or its time of day), or perk set: anything in progress
    /// is dropped, and the camera frames the new lot.
    pub(crate) fn show(&mut self, map: MapId, perk_set: PerkSet, lever: &PowerLeverSettings) {
        let new_place = shared::level::place(map) != shared::level::place(self.map);
        if self.op.is_some() {
            input::cancel_op(self);
        }
        self.map = map;
        self.perk_set = perk_set;
        self.box_from = None;
        self.prune_selection();
        if new_place {
            self.selected.clear();
            self.cam_goto = Some(input::frame(self, &self.objects(), lever));
        }
    }

    pub(crate) fn undo(&mut self, now: f32) {
        if self.op.is_some() {
            return;
        }
        if self.doc_mut().undo() {
            self.prune_selection();
        } else {
            self.toast("Nothing to undo", now);
        }
    }

    pub(crate) fn redo(&mut self, now: f32) {
        if self.op.is_some() {
            return;
        }
        if self.doc_mut().redo() {
            self.prune_selection();
        } else {
            self.toast("Nothing to redo", now);
        }
    }

    /// Save `place`'s layout: over its file in the repo when this is a dev
    /// build run from one, and to the console either way.
    pub(crate) fn save(&mut self, place: MapId, now: f32) {
        let i = Self::doc_index(place);
        let text = self.docs[i].layout.to_ron();
        let name = shared::level::file_name(place);
        info!("level editor: {name}:\n{text}");
        let path = shared::level::source_path(place);
        let in_repo = std::path::Path::new(shared::level::LEVELS_DIR).is_dir();
        let message = if !in_repo {
            format!("No repo here — {name} printed to the console")
        } else {
            match std::fs::write(&path, &text) {
                Ok(()) => format!("Saved shared/levels/{name} — rebuild the client and server to play it"),
                Err(e) => {
                    self.toast(format!("Couldn't write {}: {e} (printed to the console)", path.display()), now);
                    return;
                }
            }
        };
        self.docs[i].saved = self.docs[i].layout.clone();
        self.toast(message, now);
    }

    pub(crate) fn save_all(&mut self, now: f32) {
        let dirty: Vec<MapId> = MapId::PLACES
            .into_iter()
            .filter(|&p| self.doc_for(p).dirty())
            .collect();
        if dirty.is_empty() {
            self.toast("Nothing to save", now);
        }
        for place in dirty {
            self.save(place, now);
        }
    }

    /// Put the shown map's layout back as last saved (undoable).
    pub(crate) fn revert(&mut self, now: f32) {
        if self.op.is_some() {
            input::cancel_op(self);
        }
        let doc = self.doc_mut();
        let before = std::mem::replace(&mut doc.layout, doc.saved.clone());
        doc.checkpoint(before);
        self.prune_selection();
        self.toast("Reverted to the last save", now);
    }

    /// EXIT / Esc / the mouse back button: leave, or ask first if anything's
    /// unsaved. `true` to leave now.
    pub(crate) fn request_exit(&mut self) -> bool {
        if self.any_dirty() {
            self.exit_prompt = true;
            false
        } else {
            true
        }
    }
}

// --- in and out -------------------------------------------------------------------

/// How the player rig the camera borrows was before the editor took it
/// (put back on the way out, so a game starts from where it would have).
#[derive(Resource, Default)]
struct RigBackup {
    transforms: Vec<(Entity, Transform)>,
    capsule: Option<(Entity, Visibility)>,
}

#[allow(clippy::type_complexity)]
fn enter(
    mut editor: ResMut<Editor>,
    mut current: ResMut<CurrentMap>,
    mut backup: ResMut<RigBackup>,
    mut rig: Query<
        (Entity, &mut Transform, Has<Player>, Has<PlayerHead>),
        Or<(With<Player>, With<PlayerHead>, With<CameraShake>, With<CameraRecoil>, With<WorldModelCamera>)>,
    >,
    mut capsule: Query<(Entity, &mut Visibility), With<PlayerBodyCapsule>>,
    mut view_model: Query<&mut Camera, With<ViewModelCamera>>,
    lever: Res<PowerLeverSettings>,
    window: Single<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    *editor = Editor::fresh();
    let all = editor.objects();
    editor.cam = input::frame(&editor, &all, &lever);
    current.set_if_neq(CurrentMap(editor.map));
    crate::set_cursor_grabbed(&mut window.into_inner(), false);

    // The camera rig: the player (yaw) and head (pitch) carry the view; the
    // shake / recoil / sway nodes under them sit still.
    backup.transforms.clear();
    for (e, mut t, player, head) in &mut rig {
        backup.transforms.push((e, *t));
        if !player && !head {
            *t = Transform::IDENTITY;
        }
    }
    // (Its shadow would hang under the camera.)
    backup.capsule = capsule.iter_mut().next().map(|(e, mut v)| {
        let was = *v;
        *v = Visibility::Hidden;
        (e, was)
    });
    // No gun in the view.
    for mut cam in &mut view_model {
        cam.is_active = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn exit(
    mut editor: ResMut<Editor>,
    mut backup: ResMut<RigBackup>,
    mut lighting: ResMut<LightingBackup>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
    mut view_model: Query<&mut Camera, With<ViewModelCamera>>,
    mut ambient: ResMut<AmbientLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
) {
    *editor = Editor::default();
    for (e, t) in backup.transforms.drain(..) {
        if let Ok(mut now) = transforms.get_mut(e) {
            *now = t;
        }
    }
    if let Some((e, v)) = backup.capsule.take() {
        if let Ok(mut now) = visibility.get_mut(e) {
            *now = v;
        }
    }
    for mut cam in &mut view_model {
        cam.is_active = true;
    }
    if let Some((brightness, falloff)) = lighting.0.take() {
        ambient.brightness = brightness;
        fog.falloff = falloff;
    }
}

// --- lighting -----------------------------------------------------------------------

/// The map's own ambient brightness and fog while [`ViewOptions::bright`]
/// has them lifted — put back when it's turned off, and on the way out.
#[derive(Resource, Default)]
struct LightingBackup(Option<(f32, FogFalloff)>);

/// Ambient brightness (and no fog to speak of) with "bright" on.
const BRIGHT_AMBIENT: f32 = 750.0;

/// Lift (or put back) the light, after `apply_scene_tuning` sets the map's
/// own — which it does again whenever the map changes, so then the old
/// backup's stale and the new look's the one to keep.
fn apply_editor_lighting(
    editor: Res<Editor>,
    current: Res<CurrentMap>,
    mut backup: ResMut<LightingBackup>,
    mut ambient: ResMut<AmbientLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
) {
    if current.is_changed() {
        backup.0 = None;
    }
    match (editor.view.bright, backup.0.is_some()) {
        (true, false) => {
            backup.0 = Some((ambient.brightness, fog.falloff.clone()));
            ambient.brightness = ambient.brightness.max(BRIGHT_AMBIENT);
            fog.falloff = FogFalloff::Linear {
                start: 2_000.0,
                end: 4_000.0,
            };
        }
        (false, true) => {
            if let Some((brightness, falloff)) = backup.0.take() {
                ambient.brightness = brightness;
                fog.falloff = falloff;
            }
        }
        _ => {}
    }
}

// --- the scene --------------------------------------------------------------------

/// A placed thing's stand-in in the editor: its root is its placement, its
/// model a child, fitted the way the game fits it.
#[derive(Component)]
struct EditorObject(ObjectId);

/// The model `id` is drawn with, and where it sits from its root.
fn model_of(
    id: ObjectId,
    machines: &PerkMachineSettings,
    pap: &PapSettings,
    ammo: &AmmoCrateSettings,
    lever: &PowerLeverSettings,
    avatars: &(Res<crate::RemoteAvatarSettings>, Res<crate::ZombieAvatarSettings>),
) -> (&'static str, Transform) {
    match id {
        ObjectId::Perk(p) => (machine_model(p).0, machines.model_transform(p)),
        ObjectId::Wunderfizz => (WUNDERFIZZ_MODEL, machines.wunderfizz_model_transform()),
        ObjectId::PackAPunch => (PAP_MODEL, pap.model_transform()),
        ObjectId::AmmoCrate => (AMMO_CRATE_MODEL, ammo.model_transform()),
        ObjectId::PowerSwitch => (LEVER_MODEL, lever.model_transform()),
        ObjectId::WallBuy(_) => (crate::wall_buys::SIGN_MODEL, Transform::IDENTITY),
        // At their in-game sizes; both models face +Z as made (the zombie's
        // panel turn is from the game's -Z facing).
        ObjectId::Reference(RefKind::Player) => (
            "models/characters/soldier.glb",
            Transform::from_scale(Vec3::splat(avatars.0.scale)),
        ),
        ObjectId::Reference(RefKind::Zombie) => (
            crate::ZOMBIE_MODEL,
            Transform::from_scale(Vec3::splat(avatars.1.scale.max(0.001)))
                .with_rotation(Quat::from_rotation_y((avatars.1.yaw_offset_deg - 180.0).to_radians())),
        ),
    }
}

fn root_transform(at: Placement) -> Transform {
    crate::util::placed(at)
}

/// Keep a model standing where each shown thing is, and none for anything
/// that isn't.
#[allow(clippy::too_many_arguments)]
fn sync_objects(
    editor: Res<Editor>,
    machines: Res<PerkMachineSettings>,
    pap: Res<PapSettings>,
    ammo: Res<AmmoCrateSettings>,
    lever: Res<PowerLeverSettings>,
    wall_buys: Option<Res<crate::wall_buys::WallBuyAssets>>,
    avatars: (Res<crate::RemoteAvatarSettings>, Res<crate::ZombieAvatarSettings>),
    asset_server: Res<AssetServer>,
    mut spawned: Query<(Entity, &EditorObject, &mut Transform)>,
    mut commands: Commands,
) {
    if editor.docs.is_empty() {
        return;
    }
    let doc = editor.doc();
    let wanted = editor.objects();
    for (e, obj, mut t) in &mut spawned {
        match obj.0.get(doc).filter(|_| wanted.contains(&obj.0)) {
            Some(at) => {
                t.set_if_neq(root_transform(at));
            }
            None => commands.entity(e).despawn(),
        }
    }
    for id in wanted {
        if spawned.iter().any(|(_, o, _)| o.0 == id) {
            continue;
        }
        let Some(at) = id.get(doc) else { continue };
        let (model, offset) = model_of(id, &machines, &pap, &ammo, &lever, &avatars);
        let mut object = commands.spawn((
            StateScoped(AppState::LevelEditor),
            EditorObject(id),
            root_transform(at),
            Visibility::default(),
        ));
        let scene = SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(model)));
        // The stand-ins stand idling, as in game.
        match id {
            ObjectId::Reference(RefKind::Player) => {
                object.with_children(|o| {
                    o.spawn((crate::SoldierVisual, scene, offset)).observe(crate::start_soldier_animation);
                });
            }
            ObjectId::Reference(RefKind::Zombie) => {
                object.with_children(|o| {
                    o.spawn((scene, offset)).observe(crate::start_zombie_animation);
                });
            }
            _ => {
                object.with_child((scene, offset));
            }
        }
        // (A sign's gun outline, as the game has it.)
        if let (ObjectId::WallBuy(gun), Some(assets)) = (id, wall_buys.as_deref()) {
            object.with_child(crate::wall_buys::decal(gun, assets));
        }
    }
}
