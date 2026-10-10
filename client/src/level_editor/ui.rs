//! The level editor's panels (egui): the menu bar with the map and perk set,
//! the outliner, the selection's properties, the status bar, the help and
//! "unsaved changes" windows, and the names over things in the viewport.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts};
use bevy_rapier3d::prelude::ReadRapierContext;
use shared::perks::PerkSet;
use shared::MapId;

use super::input::{self, ACTIVE, SELECTED};
use super::{overlaps, Editor, Look, ObjectId};
use crate::player::WorldModelCamera;
use crate::power::PowerLeverSettings;
use crate::{map_ready, AppState, CurrentMap, MapLoadState};

fn color32(c: Color) -> egui::Color32 {
    let c = c.to_srgba();
    egui::Color32::from_rgba_unmultiplied(
        (c.red * 255.0) as u8,
        (c.green * 255.0) as u8,
        (c.blue * 255.0) as u8,
        (c.alpha * 255.0) as u8,
    )
}

const WARN: egui::Color32 = egui::Color32::from_rgb(255, 120, 90);
const DIM: egui::Color32 = egui::Color32::from_gray(140);

/// Every control, for the help window: `(keys, what they do)`.
const CONTROLS: &[(&str, &str)] = &[
    ("Middle mouse drag  /  Alt + left drag", "Orbit"),
    ("Shift + middle drag", "Pan"),
    ("Wheel  /  Ctrl + middle drag", "Zoom"),
    ("Ctrl / Shift + wheel", "Pan across / up"),
    ("Numpad (or top row) 1 / 3 / 7", "Front / right / top view (Ctrl: opposite)"),
    ("Numpad 2 4 6 8  /  9", "Step the view round  /  look from the other side"),
    ("F  /  Numpad .", "Frame the selection"),
    ("Home", "Frame everything"),
    ("Left click", "Select (Shift: add / remove)"),
    ("Left drag on empty space", "Box select (Shift: add, Ctrl: remove)"),
    ("Left drag on a thing", "Move it along the surface"),
    ("A  /  Alt + A", "Select all / none"),
    ("G", "Move along the surface under the cursor"),
    ("G then X / Y / Z", "Move along that axis only"),
    ("R", "Turn"),
    ("S", "Scale (not wall buys — all one size — or the reference player / zombie)"),
    ("Numpad 0  /  0", "View the selection as a player would (eye height, in front of it)"),
    ("While moving / turning: type a number", "Move that many metres / turn that many degrees"),
    ("While moving / turning: Ctrl", "Snap (0.25 m / 15°)"),
    ("Left click / Enter  ·  Right click / Esc", "Confirm  ·  cancel"),
    ("End", "Drop onto the ground below"),
    ("X  /  Delete", "Remove (Pack-a-Punch, ammo crate, power switch, wall buys, lights)"),
    ("Ctrl + Z  /  Ctrl + Shift + Z", "Undo / redo"),
    ("Ctrl + S", "Save this map"),
    ("Esc  /  mouse back", "Exit (asks first if anything's unsaved)"),
];

#[allow(clippy::too_many_arguments)]
pub(crate) fn editor_ui(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    time: Res<Time>,
    lever: Res<PowerLeverSettings>,
    rapier: ReadRapierContext,
    camera: Single<&Camera, With<WorldModelCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    map_state: Res<MapLoadState>,
    mut current: ResMut<CurrentMap>,
    mut next: ResMut<NextState<AppState>>,
) -> Result {
    if editor.docs.is_empty() {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let now = time.elapsed_secs();
    let ed = &mut *editor;

    menu_bar(ctx, ed, &lever, &rapier, now, &mut next);
    status_bar(ctx, ed, &map_state, now);
    outliner(ctx, ed, &lever, &rapier);
    properties(ctx, ed, &lever, &rapier, now);
    viewport_overlay(ctx, ed, &lever, &camera, &window);
    exit_prompt(ctx, ed, now, &mut next);
    help(ctx, ed);

    current.set_if_neq(CurrentMap(ed.map));
    Ok(())
}

fn menu_bar(
    ctx: &egui::Context,
    ed: &mut Editor,
    lever: &PowerLeverSettings,
    rapier: &ReadRapierContext,
    now: f32,
    next: &mut NextState<AppState>,
) {
    egui::TopBottomPanel::top("level_editor_menu").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.strong("ZOMBIES LEVEL EDITOR");
            ui.separator();
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("Save this map").shortcut_text("Ctrl+S")).clicked() {
                    ed.save(ed.map, now);
                }
                if ui.button("Save all maps").clicked() {
                    ed.save_all(now);
                }
                if ui.add_enabled(ed.doc().dirty(), egui::Button::new("Revert this map to its last save")).clicked() {
                    ed.revert(now);
                }
                ui.separator();
                if ui.add(egui::Button::new("Exit").shortcut_text("Esc")).clicked() && ed.request_exit() {
                    next.set(AppState::MainMenu);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add(egui::Button::new("Undo").shortcut_text("Ctrl+Z")).clicked() {
                    ed.undo(now);
                }
                if ui.add(egui::Button::new("Redo").shortcut_text("Ctrl+Shift+Z")).clicked() {
                    ed.redo(now);
                }
                ui.separator();
                if ui.add(egui::Button::new("Select all").shortcut_text("A")).clicked() {
                    ed.selected = ed.objects();
                }
                if ui.add(egui::Button::new("Select none").shortcut_text("Alt+A")).clicked() {
                    ed.selected.clear();
                }
                ui.separator();
                let any = !ed.selected.is_empty() && ed.op.is_none();
                if ui.add_enabled(any, egui::Button::new("Drop to ground").shortcut_text("End")).clicked() {
                    input::drop_to_ground(ed, rapier, now);
                }
                if ui.add_enabled(any, egui::Button::new("Remove").shortcut_text("X")).clicked() {
                    input::delete_selected(ed, now);
                }
            });
            ui.menu_button("Add", |ui| {
                let missing: Vec<ObjectId> = ObjectId::OPTIONAL
                    .into_iter()
                    .chain(ObjectId::REFERENCES)
                    .filter(|o| o.get(ed.doc()).is_none())
                    .collect();
                // (Start spots: as many as a lobby holds players.)
                let spawns = ed.doc().layout.player_spawns.len();
                let max = shared::level::MAX_PLAYER_SPAWNS;
                if ui
                    .add_enabled(spawns < max, egui::Button::new(format!("Player spawn  ({spawns}/{max})")))
                    .on_hover_text("Where players start a Zombies game — each is put on a different one, at random")
                    .clicked()
                {
                    input::add_at_view(ed, ObjectId::PlayerSpawn(spawns as u8), rapier);
                    ui.close();
                }
                // (Power lights: up to a map's worth.)
                let lights = ed.doc().layout.power_lights.len();
                let max = shared::level::MAX_POWER_LIGHTS;
                if ui
                    .add_enabled(lights < max, egui::Button::new(format!("Power light  ({lights}/{max})")))
                    .on_hover_text("A light the power turns on in a Zombies game (always on in the other modes) — night maps only")
                    .clicked()
                {
                    input::add_light_at_view(ed, rapier);
                    ui.close();
                }
                ui.separator();
                if missing.is_empty() {
                    ui.label("Everything else is on this map — perk machines can only be moved.");
                }
                for id in missing {
                    if ui.button(id.label()).clicked() {
                        input::add_at_view(ed, id, rapier);
                        ui.close();
                    }
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut ed.view.grid, "Floor grid");
                ui.checkbox(&mut ed.view.labels, "Names");
                ui.checkbox(&mut ed.view.ranges, "Every use range (not just the selection's)");
                ui.separator();
                ui.label("Look");
                for look in Look::ALL {
                    ui.radio_value(&mut ed.view.look, look, look.label());
                }
                ui.separator();
                if ui.add(egui::Button::new("Frame selection").shortcut_text("F")).clicked() {
                    let ids = if ed.selected.is_empty() { ed.objects() } else { ed.selected.clone() };
                    ed.cam_goto = Some(input::frame(ed, &ids, lever));
                }
                if ui.add(egui::Button::new("Frame everything").shortcut_text("Home")).clicked() {
                    ed.cam_goto = Some(input::frame(ed, &ed.objects(), lever));
                }
            });
            ui.separator();

            // The map, and its time of day.
            let mut place = shared::level::place(ed.map);
            ui.label("Map");
            egui::ComboBox::from_id_salt("level_editor_map")
                .selected_text(map_label(ed, place))
                .show_ui(ui, |ui| {
                    for p in MapId::PLACES {
                        ui.selectable_value(&mut place, shared::level::place(p), map_label(ed, p));
                    }
                });
            let mut map = ed.map;
            if place != shared::level::place(ed.map) {
                map = place.with_night(ed.map.is_night());
            }
            if map.has_time_of_day() {
                let mut night = map.is_night();
                ui.selectable_value(&mut night, false, "Day");
                ui.selectable_value(&mut night, true, "Night");
                map = map.with_night(night);
            }
            ui.separator();
            let mut set = ed.perk_set;
            ui.label("Perks");
            for s in PerkSet::ALL {
                ui.selectable_value(&mut set, s, s.label());
            }
            if map != ed.map || set != ed.perk_set {
                ed.show(map, set, lever);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("EXIT").clicked() && ed.request_exit() {
                    next.set(AppState::MainMenu);
                }
                if ui.button("Controls (F1)").clicked() {
                    ed.help_open = !ed.help_open;
                }
            });
        });
    });
}

/// A place's name in the map list, marked if it has unsaved changes.
fn map_label(ed: &Editor, place: MapId) -> String {
    let name = place.place_label();
    if ed.doc_for(place).dirty() {
        format!("{name}  ●")
    } else {
        name.to_string()
    }
}

fn status_bar(ctx: &egui::Context, ed: &mut Editor, map_state: &MapLoadState, now: f32) {
    if ed.toast.as_ref().is_some_and(|(_, until)| now > *until) {
        ed.toast = None;
    }
    egui::TopBottomPanel::bottom("level_editor_status").show(ctx, |ui| {
        ui.horizontal(|ui| {
            match &ed.op {
                Some(op) => {
                    ui.strong(op.describe(ed.doc()));
                    ui.label(match op.kind {
                        input::OpKind::Move => {
                            "X / Y / Z: axis  ·  type a number  ·  Ctrl: snap  ·  click / Enter: confirm  ·  right click / Esc: cancel"
                        }
                        input::OpKind::Turn => {
                            "type degrees  ·  Ctrl: snap  ·  click / Enter: confirm  ·  right click / Esc: cancel"
                        }
                        input::OpKind::Scale => {
                            "move away / toward it  ·  type a factor  ·  Ctrl: snap  ·  click / Enter: confirm  ·  \
                             right click / Esc: cancel"
                        }
                    });
                }
                None if ed.box_from.is_some() => {
                    ui.label("Box select  ·  Shift: add  ·  Ctrl: remove");
                }
                None => {
                    ui.label(
                        "Middle mouse: orbit  ·  Shift+middle: pan  ·  wheel: zoom  ·  click: select  ·  G: move  ·  \
                         R: turn  ·  F1: every control",
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let doc = ed.doc();
                if doc.dirty() {
                    ui.colored_label(WARN, "● Unsaved changes (Ctrl+S)");
                } else if doc.saved != doc.built_in {
                    ui.colored_label(DIM, "Saved — rebuild the client and server to play it");
                } else {
                    ui.colored_label(DIM, "Saved");
                }
                if !map_ready(map_state, ed.map) {
                    ui.separator();
                    ui.colored_label(DIM, "Loading the map…");
                }
                if let Some((message, _)) = &ed.toast {
                    ui.separator();
                    ui.label(message.as_str());
                }
            });
        });
    });
}

fn outliner(ctx: &egui::Context, ed: &mut Editor, lever: &PowerLeverSettings, rapier: &ReadRapierContext) {
    egui::SidePanel::left("level_editor_outliner")
        .default_width(220.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Outliner");
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                let shown = ed.objects();
                let shift = ui.input(|i| i.modifiers.shift);
                let row = |ui: &mut egui::Ui, ed: &mut Editor, id: ObjectId| {
                    let selected = ed.selected.contains(&id);
                    let mut text = egui::RichText::new(id.label());
                    if ed.active() == Some(id) {
                        text = text.color(color32(ACTIVE));
                    } else if selected {
                        text = text.color(color32(SELECTED));
                    }
                    let r = ui.selectable_label(selected, text);
                    if r.clicked() && ed.op.is_none() {
                        input::click_select(ed, Some(id), shift);
                    }
                    if r.double_clicked() {
                        ed.cam_goto = Some(input::frame(ed, &[id], lever));
                    }
                };
                ui.label(egui::RichText::new(format!("PERK MACHINES ({})", ed.perk_set.label())).small().color(DIM));
                for &id in shown.iter().filter(|o| matches!(o, ObjectId::Perk(_) | ObjectId::Wunderfizz)) {
                    row(ui, ed, id);
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new("MACHINES, SWITCHES & WALL BUYS").small().color(DIM));
                let listed = |ui: &mut egui::Ui, ed: &mut Editor, ids: &[ObjectId]| {
                    for &id in ids {
                        if shown.contains(&id) {
                            row(ui, ed, id);
                        } else {
                            ui.horizontal(|ui| {
                                ui.colored_label(DIM, format!("{} (none)", id.label()));
                                if ui.small_button("Add").clicked() && ed.op.is_none() {
                                    input::add_at_view(ed, id, rapier);
                                }
                            });
                        }
                    }
                };
                listed(ui, ed, &ObjectId::OPTIONAL);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("POWER LIGHTS").small().color(DIM));
                    let room = ed.doc().layout.power_lights.len() < shared::level::MAX_POWER_LIGHTS;
                    if ui.add_enabled(room, egui::Button::new("Add").small()).clicked() && ed.op.is_none() {
                        input::add_light_at_view(ed, rapier);
                    }
                });
                for &id in shown.iter().filter(|o| matches!(o, ObjectId::PowerLight(_))) {
                    row(ui, ed, id);
                }
                if ed.doc().layout.power_lights.is_empty() {
                    ui.colored_label(DIM, "None — the power turns nothing on here");
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new("REFERENCES (NOT SAVED)").small().color(DIM));
                listed(ui, ed, &ObjectId::REFERENCES);

                let clashes = overlaps(&ed.doc().layout);
                if !clashes.is_empty() {
                    ui.add_space(12.0);
                    ui.colored_label(WARN, "⚠ Too close together");
                    ui.label(
                        egui::RichText::new("Both can be bought from one spot (shared's tests fail on it):")
                            .small()
                            .color(DIM),
                    );
                    for (a, b) in clashes {
                        ui.colored_label(WARN, format!("{}  ·  {}", a.label(), b.label()));
                    }
                }
            });
        });
}

fn properties(
    ctx: &egui::Context,
    ed: &mut Editor,
    lever: &PowerLeverSettings,
    rapier: &ReadRapierContext,
    now: f32,
) {
    egui::SidePanel::right("level_editor_properties")
        .default_width(240.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Properties");
            ui.separator();
            let Some(id) = ed.active() else {
                ui.label("Click a machine to select it.");
                ui.add_space(6.0);
                ui.colored_label(DIM, "F1 lists every control.");
                ed.inspector_editing = false;
                return;
            };
            ui.strong(id.label());
            if ed.selected.len() > 1 {
                ui.colored_label(DIM, format!("+{} more selected — the fields edit this one", ed.selected.len() - 1));
            }
            ui.add_space(6.0);
            let Some(at) = id.get(ed.doc()) else { return };
            let busy = ed.op.is_some();
            let before = ed.doc().layout.clone();
            let mut edited = at;
            let mut editing = false;
            // The exfil area's size (its own, not a kind's).
            let area_size = (id == ObjectId::ExfilArea)
                .then(|| ed.doc().layout.exfil_area.map(|a| (a.width, a.depth)))
                .flatten();
            let mut size = area_size.unwrap_or_default();
            egui::Grid::new("level_editor_fields").num_columns(2).show(ui, |ui| {
                for (label, v) in [("X", &mut edited.pos.x), ("Y", &mut edited.pos.y), ("Z", &mut edited.pos.z)] {
                    ui.label(label);
                    let r = ui.add_enabled(!busy, egui::DragValue::new(v).speed(0.05).suffix(" m").max_decimals(3));
                    editing |= r.dragged() || r.has_focus();
                    ui.end_row();
                }
                // (A light doesn't face anywhere.)
                if !matches!(id, ObjectId::PowerLight(_)) {
                    ui.label("Turn");
                    let r = ui.add_enabled(
                        !busy,
                        egui::DragValue::new(&mut edited.yaw_deg)
                            .speed(0.5)
                            .range(-180.0..=180.0)
                            .suffix("°")
                            .max_decimals(1),
                    );
                    editing |= r.dragged() || r.has_focus();
                    ui.end_row();
                }
                // (Not the stand-ins — the yardstick, always as in game — or
                // the wall buys, all one size.)
                // (A kind's size is every map's.)
                if id.scalable() {
                    ui.label("Scale").on_hover_text("Its size on every map — the same kind is always the same size");
                    ui.horizontal(|ui| {
                        let r = ui.add_enabled(
                            !busy,
                            egui::DragValue::new(&mut edited.scale)
                                .speed(0.01)
                                .range(input::MIN_SCALE..=input::MAX_SCALE)
                                .prefix("×")
                                .max_decimals(3),
                        );
                        editing |= r.dragged() || r.has_focus();
                        if ui.add_enabled(!busy && edited.scale != 1.0, egui::Button::new("1")).on_hover_text("Back to as made").clicked() {
                            edited.scale = 1.0;
                        }
                    });
                    ui.end_row();
                }
                if area_size.is_some() {
                    for (label, v) in [("Width", &mut size.0), ("Depth", &mut size.1)] {
                        ui.label(label);
                        let r = ui.add_enabled(
                            !busy,
                            egui::DragValue::new(v).speed(0.1).range(1.0..=200.0).suffix(" m").max_decimals(2),
                        );
                        editing |= r.dragged() || r.has_focus();
                        ui.end_row();
                    }
                }
            });
            // A light's own settings.
            let light_index = match id {
                ObjectId::PowerLight(i) => Some(i as usize),
                _ => None,
            };
            let light_before = light_index.and_then(|i| ed.doc().layout.power_lights.get(i).copied());
            let mut light = light_before;
            if let Some(l) = light.as_mut() {
                ui.add_space(6.0);
                ui.strong("Light");
                egui::Grid::new("level_editor_light").num_columns(2).show(ui, |ui| {
                    ui.label("Colour");
                    let r = ui.add_enabled_ui(!busy, |ui| ui.color_edit_button_rgb(&mut l.color)).inner;
                    editing |= r.dragged() || r.has_focus();
                    ui.end_row();
                    ui.label("Brightness").on_hover_text("Lumens at full power");
                    // (Steps in proportion to how bright it is already.)
                    let step = l.intensity.max(1_000.0) * 0.01;
                    let r = ui.add_enabled(
                        !busy,
                        egui::DragValue::new(&mut l.intensity)
                            .speed(step)
                            .range(0.0..=200_000_000.0)
                            .suffix(" lm"),
                    );
                    editing |= r.dragged() || r.has_focus();
                    ui.end_row();
                    ui.label("Range").on_hover_text("How far it reaches");
                    let r = ui.add_enabled(
                        !busy,
                        egui::DragValue::new(&mut l.range).speed(0.25).range(0.5..=300.0).suffix(" m").max_decimals(2),
                    );
                    editing |= r.dragged() || r.has_focus();
                    ui.end_row();
                    ui.label("Softness").on_hover_text("The size of the glowing source — softens highlights and shadows");
                    let r = ui.add_enabled(
                        !busy,
                        egui::DragValue::new(&mut l.radius).speed(0.01).range(0.0..=10.0).suffix(" m").max_decimals(3),
                    );
                    editing |= r.dragged() || r.has_focus();
                    ui.end_row();
                    ui.label("Shadows").on_hover_text("Costly — keep it to the few lights that need them");
                    ui.add_enabled(!busy, egui::Checkbox::without_text(&mut l.shadows));
                    ui.end_row();
                    ui.label("Bulb glow").on_hover_text("A glowing bulb where it hangs, lighting just around itself (0 = none)");
                    let step = l.glow.max(1_000.0) * 0.01;
                    let r = ui.add_enabled(
                        !busy,
                        egui::DragValue::new(&mut l.glow).speed(step).range(0.0..=50_000_000.0).suffix(" lm"),
                    );
                    editing |= r.dragged() || r.has_focus();
                    ui.end_row();
                    ui.label("Spotlight").on_hover_text("Shine one way in a cone instead of all round");
                    let mut spot = l.spot.is_some();
                    if ui.add_enabled(!busy, egui::Checkbox::without_text(&mut spot)).changed() {
                        l.spot = spot.then(shared::level::SpotCone::default);
                    }
                    ui.end_row();
                    if let Some(cone) = l.spot.as_mut() {
                        for (label, v, range, hover) in [
                            ("Aim turn", &mut cone.yaw_deg, -180.0..=180.0, "Which way it shines, about the vertical"),
                            ("Aim tilt", &mut cone.pitch_deg, -90.0..=90.0, "How far it tips up (+) or down (−)"),
                            ("Cone core", &mut cone.inner_angle_deg, 0.0..=89.0, "Half-angle of its fully-bright middle"),
                            ("Cone edge", &mut cone.outer_angle_deg, 0.0..=89.0, "Half-angle to where it fades out"),
                        ] {
                            ui.label(label).on_hover_text(hover);
                            let r = ui.add_enabled(
                                !busy,
                                egui::DragValue::new(v).speed(0.5).range(range).suffix("°").max_decimals(1),
                            );
                            editing |= r.dragged() || r.has_focus();
                            ui.end_row();
                        }
                    }
                });
                if !ed.map.is_dark() {
                    ui.colored_label(DIM, "Never lit on a day map — switch to the night version to see it");
                } else if ed.view.look != Look::GamePowerOn {
                    ui.colored_label(DIM, "View → Look → In game — power on to see it lit");
                }
            }
            let light_changed = light != light_before;
            if let (true, Some(i), Some(l)) = (light_changed, light_index, light) {
                if let Some(slot) = ed.doc_mut().layout.power_lights.get_mut(i) {
                    // (Its position's the fields above.)
                    *slot = shared::level::PowerLight { pos: slot.pos, ..l };
                }
            }
            let resized = area_size.is_some_and(|s| s != size);
            if resized {
                if let Some(area) = ed.doc_mut().layout.exfil_area.as_mut() {
                    (area.width, area.depth) = size;
                }
            }
            if edited != at || resized || light_changed {
                id.set(ed.doc_mut(), edited);
                // One undo step for a whole drag / typed value.
                if !ed.inspector_editing {
                    ed.doc_mut().checkpoint(before);
                }
            }
            ed.inspector_editing = editing;

            ui.add_space(6.0);
            if id == ObjectId::ExfilArea {
                ui.colored_label(DIM, "During an exfil, only kills from inside it count");
            } else if let ObjectId::PowerLight(_) = id {
                ui.colored_label(DIM, "Night maps only: off until the power's on in a Zombies game; always on in the other modes");
            } else if let ObjectId::PlayerSpawn(_) = id {
                ui.colored_label(
                    DIM,
                    format!(
                        "A player starts here, facing its arrow ({} of {} placed — each player gets a different one, at random)",
                        ed.doc().layout.player_spawns.len(),
                        shared::level::MAX_PLAYER_SPAWNS
                    ),
                );
            } else {
                ui.colored_label(DIM, format!("Used from within {:.1} m", id.use_radius()));
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(!busy, |ui| {
                if ui.button("View as a player  (0)").clicked() {
                    match input::player_view(ed, id, lever, rapier) {
                        Some(view) => ed.cam_goto = Some(view),
                        None => ed.toast("Nothing to look at", now),
                    }
                }
                if ui.button("Drop to ground  (End)").clicked() {
                    input::drop_to_ground(ed, rapier, now);
                }
                if ui.button("Frame  (F)").clicked() {
                    ed.cam_goto = Some(input::frame(ed, &ed.selected.clone(), lever));
                }
                if id.is_optional() && ui.button("Remove  (X)").clicked() {
                    input::delete_selected(ed, now);
                }
            });
        });
}

/// Names over things, and the box being dragged out.
fn viewport_overlay(
    ctx: &egui::Context,
    ed: &Editor,
    lever: &PowerLeverSettings,
    camera: &Camera,
    window: &Window,
) {
    let cam = ed.cam.global();
    // (Only over the viewport, between the panels.)
    let viewport = ctx.available_rect();
    if ed.view.labels {
        let painter = ctx
            .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("level_editor_labels")))
            .with_clip_rect(viewport);
        let doc = ed.doc();
        for id in ed.objects() {
            let Some(at) = id.get(doc) else { continue };
            let (center, _, half) = id.world_box(at, lever);
            let top = center + Vec3::Y * (half.y + 0.25);
            let Ok(p) = camera.world_to_viewport(&cam, top) else { continue };
            let color = if ed.active() == Some(id) {
                color32(ACTIVE)
            } else if ed.selected.contains(&id) {
                color32(SELECTED)
            } else {
                egui::Color32::from_gray(225)
            };
            let font = egui::FontId::proportional(13.0);
            let pos = egui::pos2(p.x, p.y);
            painter.text(pos + egui::vec2(1.0, 1.0), egui::Align2::CENTER_BOTTOM, id.label(), font.clone(), egui::Color32::BLACK);
            painter.text(pos, egui::Align2::CENTER_BOTTOM, id.label(), font, color);
        }
    }
    if let (Some(from), Some(to)) = (ed.box_from, window.cursor_position()) {
        let painter = ctx
            .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("level_editor_box")))
            .with_clip_rect(viewport);
        let rect = egui::Rect::from_two_pos(egui::pos2(from.x, from.y), egui::pos2(to.x, to.y));
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(255, 150, 60, 25));
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, color32(SELECTED)),
            egui::StrokeKind::Inside,
        );
    }
}

fn exit_prompt(ctx: &egui::Context, ed: &mut Editor, now: f32, next: &mut NextState<AppState>) {
    if !ed.exit_prompt {
        return;
    }
    egui::Window::new("Unsaved changes")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            let dirty: Vec<&str> = MapId::PLACES
                .into_iter()
                .filter(|&p| ed.doc_for(p).dirty())
                .map(|p| p.place_label())
                .collect();
            ui.label(format!("Not saved: {}", dirty.join(", ")));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save all & exit").clicked() {
                    ed.save_all(now);
                    next.set(AppState::MainMenu);
                }
                if ui.button("Exit without saving").clicked() {
                    next.set(AppState::MainMenu);
                }
                if ui.button("Cancel").clicked() {
                    ed.exit_prompt = false;
                }
            });
        });
}

fn help(ctx: &egui::Context, ed: &mut Editor) {
    let mut open = ed.help_open;
    egui::Window::new("Controls")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Grid::new("level_editor_controls").striped(true).show(ui, |ui| {
                for (keys, what) in CONTROLS {
                    ui.strong(*keys);
                    ui.label(*what);
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.colored_label(
                DIM,
                "Saving writes shared/levels/<map>.ron (and prints it to the console). Rebuild the client and \
                 the server to play the new layout.",
            );
        });
    ed.help_open = open;
}
