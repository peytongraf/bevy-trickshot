//! The `Zombies` level editor — main menu → LEVEL EDITOR
//! ([`AppState::LevelEditor`]). It places everything a `Zombies` game puts on
//! a map (the perk machines, Der Wunderfizz, the Pack-a-Punch, the ammo crate,
//! the power switch, the wall buys, the exfil radio and area, the Rampage
//! Inducer, the lights the power turns on, the fires, doors, windows, water
//! and waterfalls, the players' start spots) and
//! saves each map's layout to its file in
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
//! The map shows brightly lit and fog-free to work on; View → Look shows it
//! as a game does instead — its own light and fog — with the power off, or
//! on (its power lights lit, [`sync_editor_lights`]).
//!
//! H (or View → Clean view) hides everything that's the editor's own — the
//! panels, names, grid, boxes, arrows and ranges, and the reference / start
//! spot people — leaving just the map as it renders, for a screenshot; the
//! camera still moves. H, Esc or the mouse back button bring it all back.
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
use shared::level::{ExfilArea, ObjectSizes, Placement, ZombiesLayout};
use shared::perks::{Perk, PerkSet};
use shared::weapon::WeaponId;
use shared::MapId;

use crate::ammo_crate::{AmmoCrateSettings, AMMO_CRATE_MODEL};
use crate::pap::{PapSettings, PAP_MODEL};
use crate::player::{PlayerHead, ViewModelCamera, WorldModelCamera};
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
                        (crate::apply_scene_tuning, apply_editor_lighting).chain(),
                        crate::sky_follow_camera,
                    ),
                    (
                        input::camera_input,
                        input::edit_input,
                        share_sizes,
                        input::drive_camera,
                        sync_objects,
                        sync_editor_lights,
                        sync_editor_flames,
                        sync_editor_water,
                        sync_editor_windows,
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
    /// One of the ammo crates (`ZombiesLayout::ammo_crates`, by index) — as
    /// many as the map wants, one per zone say.
    AmmoCrate(u8),
    PowerSwitch,
    MysteryBox,
    ArmorStation,
    /// The crafting table.
    CraftingTable,
    /// The exfil's radio...
    ExfilRadio,
    /// ...and the area the party has to hold (a rectangle: its middle and
    /// turn are its placement, its width and depth the properties panel's).
    ExfilArea,
    /// The Rampage Inducer.
    RampageInducer,
    /// The power station — scenery the power switch goes on.
    PowerStation,
    /// The wall buy selling this gun.
    WallBuy(WeaponId),
    /// A players' start spot (`ZombiesLayout::player_spawns`, by index) —
    /// as many as players a lobby holds, each player dealt a different one.
    PlayerSpawn(u8),
    /// One of the lights the power turns on (`ZombiesLayout::power_lights`,
    /// by index) — hanging where it's put, no model; its colour, brightness
    /// and reach are the properties panel's.
    PowerLight(u8),
    /// One of the fires burning on the map (`ZombiesLayout::flames`, by
    /// index) — where its flames start, no model; its size and light are
    /// the properties panel's.
    Flame(u8),
    /// One of the doors in the map's doorways (`ZombiesLayout::doors`, by
    /// index) — its own width, height and depth, cost, whether it needs the
    /// power and which way it slides are the properties panel's.
    Door(u8),
    /// One of the windows zombies climb in through (`ZombiesLayout::windows`,
    /// by index) — its spot is the floor under its opening, halfway through
    /// the wall, its arrow pointing inside; its opening's size, sill and the
    /// wall's thickness are the properties panel's.
    Window(u8),
    /// One of the pools of water (`ZombiesLayout::waters`, by index) — its
    /// spot is the middle of its surface; its size and look are the
    /// properties panel's.
    Water(u8),
    /// One of the waterfalls (`ZombiesLayout::waterfalls`, by index) — its
    /// spot is the middle of its lip, its arrow the way it pours; its
    /// width, drop and look are the properties panel's.
    Waterfall(u8),
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
    pub(crate) const OPTIONAL: [ObjectId; 11] = [
        ObjectId::PackAPunch,
        ObjectId::PowerSwitch,
        ObjectId::MysteryBox,
        ObjectId::ArmorStation,
        ObjectId::CraftingTable,
        ObjectId::ExfilRadio,
        ObjectId::ExfilArea,
        ObjectId::RampageInducer,
        ObjectId::PowerStation,
        ObjectId::WallBuy(WeaponId::Sniper),
        ObjectId::WallBuy(WeaponId::Ak74),
    ];

    /// Names for the ammo crates ([`ObjectId::AmmoCrate`]).
    const CRATE_LABELS: [&'static str; shared::level::MAX_AMMO_CRATES] = [
        "Ammo crate 1",
        "Ammo crate 2",
        "Ammo crate 3",
        "Ammo crate 4",
        "Ammo crate 5",
        "Ammo crate 6",
        "Ammo crate 7",
        "Ammo crate 8",
    ];

    /// Names for the start spots ([`ObjectId::PlayerSpawn`]).
    const SPAWN_LABELS: [&'static str; shared::level::MAX_PLAYER_SPAWNS] = [
        "Player spawn 1",
        "Player spawn 2",
        "Player spawn 3",
        "Player spawn 4",
        "Player spawn 5",
        "Player spawn 6",
        "Player spawn 7",
        "Player spawn 8",
    ];

    /// Names for the power lights ([`ObjectId::PowerLight`]).
    const LIGHT_LABELS: [&'static str; shared::level::MAX_POWER_LIGHTS] = [
        "Power light 1",
        "Power light 2",
        "Power light 3",
        "Power light 4",
        "Power light 5",
        "Power light 6",
        "Power light 7",
        "Power light 8",
        "Power light 9",
        "Power light 10",
        "Power light 11",
        "Power light 12",
        "Power light 13",
        "Power light 14",
        "Power light 15",
        "Power light 16",
    ];

    /// Names for the fires ([`ObjectId::Flame`]).
    const FLAME_LABELS: [&'static str; shared::level::MAX_FLAMES] = [
        "Fire 1",
        "Fire 2",
        "Fire 3",
        "Fire 4",
        "Fire 5",
        "Fire 6",
        "Fire 7",
        "Fire 8",
        "Fire 9",
        "Fire 10",
        "Fire 11",
        "Fire 12",
        "Fire 13",
        "Fire 14",
        "Fire 15",
        "Fire 16",
        "Fire 17",
        "Fire 18",
        "Fire 19",
        "Fire 20",
        "Fire 21",
        "Fire 22",
        "Fire 23",
        "Fire 24",
        "Fire 25",
        "Fire 26",
        "Fire 27",
        "Fire 28",
        "Fire 29",
        "Fire 30",
        "Fire 31",
        "Fire 32",
    ];

    /// Names for the doors ([`ObjectId::Door`]).
    const DOOR_LABELS: [&'static str; shared::doors::MAX_DOORS] = [
        "Door 1",
        "Door 2",
        "Door 3",
        "Door 4",
        "Door 5",
        "Door 6",
        "Door 7",
        "Door 8",
        "Door 9",
        "Door 10",
        "Door 11",
        "Door 12",
        "Door 13",
        "Door 14",
        "Door 15",
        "Door 16",
        "Door 17",
        "Door 18",
        "Door 19",
        "Door 20",
        "Door 21",
        "Door 22",
        "Door 23",
        "Door 24",
        "Door 25",
        "Door 26",
        "Door 27",
        "Door 28",
        "Door 29",
        "Door 30",
        "Door 31",
        "Door 32",
    ];

    /// Names for the windows ([`ObjectId::Window`]).
    const WINDOW_LABELS: [&'static str; shared::windows::MAX_WINDOWS] = [
        "Window 1",
        "Window 2",
        "Window 3",
        "Window 4",
        "Window 5",
        "Window 6",
        "Window 7",
        "Window 8",
        "Window 9",
        "Window 10",
        "Window 11",
        "Window 12",
        "Window 13",
        "Window 14",
        "Window 15",
        "Window 16",
        "Window 17",
        "Window 18",
        "Window 19",
        "Window 20",
        "Window 21",
        "Window 22",
        "Window 23",
        "Window 24",
        "Window 25",
        "Window 26",
        "Window 27",
        "Window 28",
        "Window 29",
        "Window 30",
        "Window 31",
        "Window 32",
    ];

    /// Names for the pools ([`ObjectId::Water`]).
    const WATER_LABELS: [&'static str; shared::level::MAX_WATERS] = [
        "Water 1",
        "Water 2",
        "Water 3",
        "Water 4",
        "Water 5",
        "Water 6",
        "Water 7",
        "Water 8",
        "Water 9",
        "Water 10",
        "Water 11",
        "Water 12",
        "Water 13",
        "Water 14",
        "Water 15",
        "Water 16",
    ];

    /// Names for the waterfalls ([`ObjectId::Waterfall`]).
    const WATERFALL_LABELS: [&'static str; shared::level::MAX_WATERFALLS] = [
        "Waterfall 1",
        "Waterfall 2",
        "Waterfall 3",
        "Waterfall 4",
        "Waterfall 5",
        "Waterfall 6",
        "Waterfall 7",
        "Waterfall 8",
        "Waterfall 9",
        "Waterfall 10",
        "Waterfall 11",
        "Waterfall 12",
        "Waterfall 13",
        "Waterfall 14",
        "Waterfall 15",
        "Waterfall 16",
    ];

    /// The stand-ins.
    pub(crate) const REFERENCES: [ObjectId; 2] =
        [ObjectId::Reference(RefKind::Player), ObjectId::Reference(RefKind::Zombie)];

    pub(crate) fn label(self) -> &'static str {
        match self {
            ObjectId::Perk(p) => p.label(),
            ObjectId::Wunderfizz => "Der Wunderfizz",
            ObjectId::PackAPunch => "Pack-a-Punch",
            ObjectId::AmmoCrate(i) => Self::CRATE_LABELS[(i as usize).min(Self::CRATE_LABELS.len() - 1)],
            ObjectId::PowerSwitch => "Power switch",
            ObjectId::MysteryBox => "Mystery Box",
            ObjectId::ArmorStation => "Armor station",
            ObjectId::CraftingTable => "Crafting table",
            ObjectId::ExfilRadio => "Exfil radio",
            ObjectId::ExfilArea => "Exfil area",
            ObjectId::RampageInducer => "Rampage Inducer",
            ObjectId::PowerStation => "Power station",
            ObjectId::WallBuy(WeaponId::Ak74) => "AK-74 wall buy",
            ObjectId::WallBuy(_) => "Sniper wall buy",
            ObjectId::PlayerSpawn(i) => Self::SPAWN_LABELS[(i as usize).min(Self::SPAWN_LABELS.len() - 1)],
            ObjectId::PowerLight(i) => Self::LIGHT_LABELS[(i as usize).min(Self::LIGHT_LABELS.len() - 1)],
            ObjectId::Flame(i) => Self::FLAME_LABELS[(i as usize).min(Self::FLAME_LABELS.len() - 1)],
            ObjectId::Door(i) => Self::DOOR_LABELS[(i as usize).min(Self::DOOR_LABELS.len() - 1)],
            ObjectId::Window(i) => Self::WINDOW_LABELS[(i as usize).min(Self::WINDOW_LABELS.len() - 1)],
            ObjectId::Water(i) => Self::WATER_LABELS[(i as usize).min(Self::WATER_LABELS.len() - 1)],
            ObjectId::Waterfall(i) => Self::WATERFALL_LABELS[(i as usize).min(Self::WATERFALL_LABELS.len() - 1)],
            ObjectId::Reference(RefKind::Player) => "Player (reference)",
            ObjectId::Reference(RefKind::Zombie) => "Zombie (reference)",
        }
    }

    /// Whether its size can be changed — not the stand-ins (the yardstick),
    /// the wall buys (every sign's one size, `shared::wall_buy::SIGN_SCALE`)
    /// or the exfil area (its width and depth are its own).
    pub(crate) fn scalable(self) -> bool {
        !matches!(
            self,
            ObjectId::Reference(_)
                | ObjectId::WallBuy(_)
                | ObjectId::ExfilArea
                | ObjectId::PlayerSpawn(_)
                | ObjectId::PowerLight(_)
                | ObjectId::Flame(_)
                | ObjectId::Door(_)
                | ObjectId::Window(_)
                | ObjectId::Water(_)
                | ObjectId::Waterfall(_)
        )
    }

    /// Its kind's key in the sizes every map shares
    /// ([`shared::level::ObjectSizes`]) — `None` for what isn't sized there
    /// (the wall buys, the stand-ins).
    pub(crate) fn size_key(self) -> Option<String> {
        match self {
            ObjectId::Perk(p) => Some(shared::level::perk_size_key(p)),
            ObjectId::Wunderfizz => Some("wunderfizz".into()),
            ObjectId::PackAPunch => Some("pack_a_punch".into()),
            ObjectId::AmmoCrate(_) => Some("ammo_crate".into()),
            ObjectId::PowerSwitch => Some("power_switch".into()),
            ObjectId::MysteryBox => Some("mystery_box".into()),
            ObjectId::ArmorStation => Some("armor_station".into()),
            ObjectId::CraftingTable => Some("crafting_table".into()),
            ObjectId::ExfilRadio => Some("exfil_radio".into()),
            ObjectId::RampageInducer => Some("rampage_inducer".into()),
            ObjectId::PowerStation => Some("power_station".into()),
            ObjectId::ExfilArea
            | ObjectId::WallBuy(_)
            | ObjectId::Reference(_)
            | ObjectId::PlayerSpawn(_)
            | ObjectId::PowerLight(_)
            | ObjectId::Flame(_)
            | ObjectId::Door(_)
            | ObjectId::Window(_)
            | ObjectId::Water(_)
            | ObjectId::Waterfall(_) => None,
        }
    }

    /// A stand-in, not part of the layout.
    pub(crate) fn is_reference(self) -> bool {
        matches!(self, ObjectId::Reference(_))
    }

    /// Whether it can be added and removed (everything but the machines).
    pub(crate) fn is_optional(self) -> bool {
        Self::OPTIONAL.contains(&self)
            || self.is_reference()
            || matches!(
                self,
                ObjectId::AmmoCrate(_)
                    | ObjectId::PlayerSpawn(_)
                    | ObjectId::PowerLight(_)
                    | ObjectId::Flame(_)
                    | ObjectId::Door(_)
                    | ObjectId::Window(_)
                    | ObjectId::Water(_)
                    | ObjectId::Waterfall(_)
            )
    }

    fn slot(self, layout: &mut ZombiesLayout) -> Option<&mut Option<Placement>> {
        match self {
            ObjectId::PackAPunch => Some(&mut layout.pack_a_punch),
            ObjectId::PowerSwitch => Some(&mut layout.power_switch),
            ObjectId::MysteryBox => Some(&mut layout.mystery_box),
            ObjectId::ArmorStation => Some(&mut layout.armor_station),
            ObjectId::CraftingTable => Some(&mut layout.crafting_table),
            ObjectId::ExfilRadio => Some(&mut layout.exfil_radio),
            ObjectId::RampageInducer => Some(&mut layout.rampage_inducer),
            ObjectId::PowerStation => Some(&mut layout.power_station),
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
            ObjectId::AmmoCrate(i) => layout.ammo_crates.get(i as usize).copied(),
            ObjectId::PowerSwitch => layout.power_switch,
            ObjectId::MysteryBox => layout.mystery_box,
            ObjectId::ArmorStation => layout.armor_station,
            ObjectId::CraftingTable => layout.crafting_table,
            ObjectId::ExfilRadio => layout.exfil_radio,
            ObjectId::ExfilArea => layout.exfil_area.map(|a| a.at),
            ObjectId::RampageInducer => layout.rampage_inducer,
            ObjectId::PowerStation => layout.power_station,
            ObjectId::WallBuy(gun) => layout.wall_buy(gun),
            ObjectId::PlayerSpawn(i) => layout.player_spawns.get(i as usize).copied(),
            ObjectId::PowerLight(i) => layout.power_lights.get(i as usize).map(|l| Placement::new(l.pos, 0.0)),
            ObjectId::Flame(i) => layout.flames.get(i as usize).map(|f| Placement::new(f.pos, 0.0)),
            ObjectId::Door(i) => layout.doors.get(i as usize).map(|d| d.at),
            ObjectId::Window(i) => layout.windows.get(i as usize).map(|w| w.at),
            ObjectId::Water(i) => layout.waters.get(i as usize).map(|w| w.at),
            ObjectId::Waterfall(i) => layout.waterfalls.get(i as usize).map(|f| f.at),
        }
    }

    fn set_in(self, layout: &mut ZombiesLayout, at: Placement) {
        match self {
            ObjectId::Perk(p) => {
                layout.perks.insert(p, at);
            }
            ObjectId::Wunderfizz => layout.wunderfizz = at,
            ObjectId::WallBuy(gun) => layout.set_wall_buy(gun, at),
            // (A new one goes on the end.)
            ObjectId::AmmoCrate(i) => {
                let crates = &mut layout.ammo_crates;
                if let Some(c) = crates.get_mut(i as usize) {
                    *c = at;
                } else if crates.len() < shared::level::MAX_AMMO_CRATES {
                    crates.push(at);
                }
            }
            // (Keeping its size; a new one's the default size.)
            // (A new one goes on the end.)
            ObjectId::PlayerSpawn(i) => {
                let at = at.with_scale(1.0);
                let spots = &mut layout.player_spawns;
                if let Some(spot) = spots.get_mut(i as usize) {
                    *spot = at;
                } else if spots.len() < shared::level::MAX_PLAYER_SPAWNS {
                    spots.push(at);
                }
            }
            // (Only its position — the rest is the properties panel's; a
            // new one goes on the end.)
            ObjectId::PowerLight(i) => {
                let lights = &mut layout.power_lights;
                if let Some(light) = lights.get_mut(i as usize) {
                    light.pos = at.pos;
                } else if lights.len() < shared::level::MAX_POWER_LIGHTS {
                    lights.push(shared::level::PowerLight::new(at.pos));
                }
            }
            // (Its spot and turn; the rest is the properties panel's.)
            ObjectId::Door(i) => {
                let doors = &mut layout.doors;
                if let Some(door) = doors.get_mut(i as usize) {
                    door.at = at.with_scale(1.0);
                } else if doors.len() < shared::doors::MAX_DOORS {
                    doors.push(shared::doors::Door::new(at));
                }
            }
            // (Its spot and turn; the rest is the properties panel's.)
            ObjectId::Window(i) => {
                let windows = &mut layout.windows;
                if let Some(window) = windows.get_mut(i as usize) {
                    window.at = at.with_scale(1.0);
                } else if windows.len() < shared::windows::MAX_WINDOWS {
                    windows.push(shared::windows::Window::new(at));
                }
            }
            // (Its spot and turn; the rest is the properties panel's.)
            ObjectId::Water(i) => {
                let waters = &mut layout.waters;
                if let Some(water) = waters.get_mut(i as usize) {
                    water.at = at.with_scale(1.0);
                } else if waters.len() < shared::level::MAX_WATERS {
                    waters.push(shared::level::Water::new(at));
                }
            }
            // (Its spot and turn; the rest is the properties panel's.)
            ObjectId::Waterfall(i) => {
                let falls = &mut layout.waterfalls;
                if let Some(fall) = falls.get_mut(i as usize) {
                    fall.at = at.with_scale(1.0);
                } else if falls.len() < shared::level::MAX_WATERFALLS {
                    falls.push(shared::level::Waterfall::new(at));
                }
            }
            // (The same: only its position here.)
            ObjectId::Flame(i) => {
                let flames = &mut layout.flames;
                if let Some(flame) = flames.get_mut(i as usize) {
                    flame.pos = at.pos;
                } else if flames.len() < shared::level::MAX_FLAMES {
                    flames.push(shared::level::Flame::new(at.pos));
                }
            }
            ObjectId::ExfilArea => match layout.exfil_area.as_mut() {
                Some(area) => area.at = at.with_scale(1.0),
                None => layout.exfil_area = Some(ExfilArea::new(at)),
            },
            other => {
                if let Some(slot) = other.slot(layout) {
                    *slot = Some(at);
                }
            }
        }
    }

    fn remove_in(self, layout: &mut ZombiesLayout) -> bool {
        if self == ObjectId::ExfilArea {
            return layout.exfil_area.take().is_some();
        }
        if let ObjectId::AmmoCrate(i) = self {
            let i = i as usize;
            return (i < layout.ammo_crates.len()).then(|| layout.ammo_crates.remove(i)).is_some();
        }
        if let ObjectId::PlayerSpawn(i) = self {
            let i = i as usize;
            return (i < layout.player_spawns.len()).then(|| layout.player_spawns.remove(i)).is_some();
        }
        if let ObjectId::PowerLight(i) = self {
            let i = i as usize;
            return (i < layout.power_lights.len()).then(|| layout.power_lights.remove(i)).is_some();
        }
        if let ObjectId::Flame(i) = self {
            let i = i as usize;
            return (i < layout.flames.len()).then(|| layout.flames.remove(i)).is_some();
        }
        if let ObjectId::Door(i) = self {
            let i = i as usize;
            return (i < layout.doors.len()).then(|| layout.doors.remove(i)).is_some();
        }
        if let ObjectId::Window(i) = self {
            let i = i as usize;
            return (i < layout.windows.len()).then(|| layout.windows.remove(i)).is_some();
        }
        if let ObjectId::Water(i) = self {
            let i = i as usize;
            return (i < layout.waters.len()).then(|| layout.waters.remove(i)).is_some();
        }
        if let ObjectId::Waterfall(i) = self {
            let i = i as usize;
            return (i < layout.waterfalls.len()).then(|| layout.waterfalls.remove(i)).is_some();
        }
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
            ObjectId::AmmoCrate(_) => Color::srgb(0.55, 0.8, 0.35),
            ObjectId::PowerSwitch => Color::srgb(1.0, 0.85, 0.2),
            ObjectId::MysteryBox => crate::mystery_box::AMBER,
            ObjectId::ArmorStation => crate::armor::ARMOR_BLUE,
            ObjectId::CraftingTable => Color::srgb(0.85, 0.6, 0.3),
            ObjectId::ExfilRadio => crate::exfil::EXFIL_BLUE,
            ObjectId::ExfilArea => crate::exfil::EXFIL_ORANGE,
            ObjectId::RampageInducer => crate::rampage::RAMPAGE_ORANGE,
            ObjectId::PowerStation => Color::srgb(0.95, 0.8, 0.3),
            ObjectId::WallBuy(_) => Color::srgb(0.6, 1.0, 0.45),
            ObjectId::Reference(_) => Color::srgb(0.8, 0.85, 0.9),
            ObjectId::PlayerSpawn(_) => Color::srgb(0.3, 0.9, 1.0),
            ObjectId::PowerLight(_) => Color::srgb(1.0, 0.92, 0.45),
            ObjectId::Flame(_) => Color::srgb(1.0, 0.5, 0.12),
            ObjectId::Door(_) => Color::srgb(0.85, 0.72, 0.5),
            ObjectId::Window(_) => Color::srgb(0.8, 0.62, 0.4),
            ObjectId::Water(_) => Color::srgb(0.2, 0.75, 0.85),
            ObjectId::Waterfall(_) => Color::srgb(0.55, 0.9, 1.0),
        }
    }

    /// How close (m, across the ground) a player has to be to use it.
    pub(crate) fn use_radius(self) -> f32 {
        match self {
            ObjectId::PackAPunch => shared::perks::PERK_USE_RADIUS + 0.5,
            ObjectId::WallBuy(_) => shared::wall_buy::USE_RADIUS,
            ObjectId::MysteryBox => shared::mystery_box::USE_RADIUS,
            ObjectId::ArmorStation => shared::armor::USE_RADIUS,
            ObjectId::CraftingTable => shared::crafting::USE_RADIUS,
            ObjectId::ExfilRadio => shared::exfil::USE_RADIUS,
            ObjectId::RampageInducer => shared::rampage::USE_RADIUS,
            // (Not used — stood in. See `draw_gizmos`' rectangle.)
            ObjectId::ExfilArea => 0.0,
            // (Not used — stood on. Its circle's about a body's width.)
            ObjectId::PlayerSpawn(_) => 0.4,
            // (Not used — its reach is drawn as a sphere instead.)
            ObjectId::PowerLight(_) | ObjectId::Flame(_) => 0.0,
            // (Bought from in front of or behind it — a rectangle, drawn by
            // `input::draw_gizmos`.)
            ObjectId::Door(_) => shared::doors::USE_REACH,
            // (Rebuilt from inside — a rectangle, drawn by
            // `input::draw_gizmos`.)
            ObjectId::Window(_) => shared::windows::REPAIR_REACH,
            // (Not used — just looked at; the switch on it is its own.)
            ObjectId::Water(_) | ObjectId::Waterfall(_) | ObjectId::PowerStation => 0.0,
            _ => shared::perks::PERK_USE_RADIUS,
        }
    }

    /// The middle of where it's used from, standing at `at`.
    pub(crate) fn use_center(self, at: Placement) -> Vec3 {
        match self {
            ObjectId::WallBuy(_) => shared::wall_buy::use_spot(at),
            ObjectId::MysteryBox => shared::mystery_box::use_spot(at),
            _ => at.pos,
        }
    }

    /// Its box standing at `at`, scale and all: `(centre, rotation, half
    /// extents)` — what's clicked and outlined.
    pub(crate) fn world_box(self, doc: &Doc, at: Placement, lever: &PowerLeverSettings) -> (Vec3, Quat, Vec3) {
        let (center, half) = self.bounds(doc, lever);
        let rot = at.rotation();
        (at.pos + rot * center * at.scale, rot, half * at.scale)
    }

    /// Its box in its own frame (from the ground under its middle), as made:
    /// `(centre, half extents)`.
    fn bounds(self, doc: &Doc, lever: &PowerLeverSettings) -> (Vec3, Vec3) {
        let standing = |half: Vec3| (Vec3::Y * half.y, half);
        match self {
            // A door: its own size.
            ObjectId::Door(i) => standing(
                doc.layout.doors.get(i as usize).map_or(shared::doors::MODEL_SIZE, |d| d.size) * 0.5,
            ),
            // A window: its opening.
            ObjectId::Window(i) => {
                let w = doc.layout.windows.get(i as usize).copied().unwrap_or_else(|| shared::windows::Window::new(Placement::default()));
                let half = Vec3::new(w.size.x * 0.5, w.size.y * 0.5, w.depth * 0.5);
                (Vec3::Y * (w.sill + half.y), half)
            }
            // A pool: its surface (a thin slab, its spot in the middle).
            ObjectId::Water(i) => {
                let size = doc.layout.waters.get(i as usize).map_or(Vec2::splat(6.0), |w| w.size);
                (Vec3::ZERO, Vec3::new(size.x * 0.5, 0.05, size.y * 0.5))
            }
            // A waterfall: from its lip down to where it lands.
            ObjectId::Waterfall(i) => {
                let f = doc
                    .layout
                    .waterfalls
                    .get(i as usize)
                    .copied()
                    .unwrap_or_else(|| shared::level::Waterfall::new(Placement::default()));
                let foot = f.local_at(f.fall_time());
                let half = Vec3::new(f.width * 0.6, (f.drop * 0.5).max(0.1), (foot.z * 0.5).max(0.1) + 0.05);
                (Vec3::new(0.0, -f.drop * 0.5, foot.z * 0.5), half)
            }
            ObjectId::Perk(_) | ObjectId::Wunderfizz => standing(shared::perks::MACHINE_HALF_EXTENTS),
            ObjectId::PackAPunch => standing(shared::pap::MACHINE_HALF_EXTENTS),
            ObjectId::AmmoCrate(_) => standing(shared::ammo::CRATE_HALF_EXTENTS),
            ObjectId::MysteryBox => standing(shared::mystery_box::HALF_EXTENTS),
            ObjectId::ArmorStation => standing(shared::armor::HALF_EXTENTS),
            ObjectId::CraftingTable => standing(shared::crafting::HALF_EXTENTS),
            ObjectId::ExfilRadio => standing(shared::exfil::RADIO_HALF_EXTENTS),
            ObjectId::RampageInducer => standing(shared::rampage::HALF_EXTENTS),
            ObjectId::PowerStation => standing(shared::power_station::HALF_EXTENTS),
            // The area's middle: a marker to grab it by (its rectangle's
            // drawn round it, `input::draw_gizmos`).
            ObjectId::ExfilArea => standing(Vec3::splat(0.4)),
            // A light: a small box round where it hangs (to click).
            ObjectId::PowerLight(_) => (Vec3::ZERO, Vec3::splat(0.3)),
            // A fire: a small box over where its flames start (to click).
            ObjectId::Flame(_) => (Vec3::Y * 0.2, Vec3::splat(0.2)),
            // The lever, up on the wall, and down to the ground where it's
            // used from.
            // The sign: post and board.
            ObjectId::WallBuy(_) => (Vec3::new(-0.02, 1.3, 0.0), Vec3::new(1.0, 1.5, 0.15)),
            // A person: the in-game body's height.
            ObjectId::Reference(_) | ObjectId::PlayerSpawn(_) => {
                standing(Vec3::new(0.35, crate::EYE_HEIGHT * 0.5 + 0.1, 0.25))
            }
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
    out.extend(ObjectId::OPTIONAL.into_iter().filter(|o| o.get(doc).is_some()));
    out.extend((0..doc.layout.ammo_crates.len() as u8).map(ObjectId::AmmoCrate));
    out.extend((0..doc.layout.player_spawns.len() as u8).map(ObjectId::PlayerSpawn));
    out.extend((0..doc.layout.power_lights.len() as u8).map(ObjectId::PowerLight));
    out.extend((0..doc.layout.flames.len() as u8).map(ObjectId::Flame));
    out.extend((0..doc.layout.doors.len() as u8).map(ObjectId::Door));
    out.extend((0..doc.layout.windows.len() as u8).map(ObjectId::Window));
    out.extend((0..doc.layout.waters.len() as u8).map(ObjectId::Water));
    out.extend((0..doc.layout.waterfalls.len() as u8).map(ObjectId::Waterfall));
    out.extend(ObjectId::REFERENCES.into_iter().filter(|o| o.get(doc).is_some()));
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
    /// `map`'s layout, everything at `sizes`: the repo's file when there is
    /// one (it may be newer than this build), else the built-in one.
    fn load(map: MapId, sizes: &ObjectSizes) -> Self {
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
        let layout = layout.sized(sizes);
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
    /// How the map's lit (View → Look).
    pub(crate) look: Look,
    /// Clean view (H): nothing of the editor's own shows, just the map —
    /// since when (elapsed seconds), for the hint that it's on.
    pub(crate) clean: Option<f32>,
}

/// How the editor lights the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Look {
    /// Ambient light lifted and the fog pushed away, so a night map's
    /// workable — the power lights not lit.
    #[default]
    Editor,
    /// As a game has it: its own light and fog, the power off...
    GamePowerOff,
    /// ...or on — its power lights lit.
    GamePowerOn,
}

impl Look {
    pub(crate) const ALL: [Look; 3] = [Look::Editor, Look::GamePowerOff, Look::GamePowerOn];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Look::Editor => "Editor (bright, no fog)",
            Look::GamePowerOff => "In game — power off",
            Look::GamePowerOn => "In game — power on",
        }
    }
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            grid: true,
            labels: true,
            ranges: false,
            look: Look::Editor,
            clean: None,
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
    /// How big each kind of thing is, on every map — as edited, and as last
    /// saved ([`share_sizes`]).
    pub(crate) sizes: ObjectSizes,
    saved_sizes: ObjectSizes,
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
        // The repo's sizes when there are (they may be newer than this
        // build), else the built-in ones.
        let sizes = std::fs::read_to_string(shared::level::sizes_source_path())
            .ok()
            .and_then(|text| match ObjectSizes::from_ron(&text) {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!("level editor: couldn't read {}: {e}", shared::level::SIZES_FILE);
                    None
                }
            })
            .unwrap_or_else(|| shared::level::sizes().clone());
        Self {
            map,
            perk_set: PerkSet::Classic,
            docs: MapId::PLACES.iter().map(|&p| Doc::load(p, &sizes)).collect(),
            saved_sizes: sizes.clone(),
            sizes,
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
        // Every map's sizes go with it.
        let sizes_text = self.sizes.to_ron();
        if self.sizes != self.saved_sizes {
            info!("level editor: {}:\n{sizes_text}", shared::level::SIZES_FILE);
            if in_repo {
                if let Err(e) = std::fs::write(shared::level::sizes_source_path(), &sizes_text) {
                    self.toast(format!("Couldn't write {}: {e} (printed to the console)", shared::level::SIZES_FILE), now);
                    return;
                }
            }
            self.saved_sizes = self.sizes.clone();
            // (Every map's saved layout is at the saved sizes now.)
            for doc in &mut self.docs {
                doc.saved = doc.saved.clone().sized(&self.saved_sizes);
            }
        }
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
    mut view_model: Query<&mut Camera, With<ViewModelCamera>>,
    mut ambient: ResMut<AmbientLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
    mut ground_fog: ResMut<crate::GroundFogHidden>,
) {
    *editor = Editor::default();
    ground_fog.0 = false;
    for (e, t) in backup.transforms.drain(..) {
        if let Ok(mut now) = transforms.get_mut(e) {
            *now = t;
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

/// The map's own ambient brightness and fog while the editor's look
/// ([`Look::Editor`]) has them lifted — put back when it's turned off, and on the way out.
#[derive(Resource, Default)]
struct LightingBackup(Option<(f32, FogFalloff)>);

/// Ambient brightness (and no fog to speak of, ground fog included) in the
/// editor's look.
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
    mut ground_fog: ResMut<crate::GroundFogHidden>,
) {
    let editor_look = editor.view.look == Look::Editor;
    if ground_fog.0 != editor_look {
        ground_fog.0 = editor_look;
    }
    if current.is_changed() {
        backup.0 = None;
    }
    match (editor_look, backup.0.is_some()) {
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

/// On the editor's power lights.
#[derive(Component)]
struct EditorPowerLight;

/// Light the shown map's power lights as a game would with the power on
/// ([`Look::GamePowerOn`] — and only on a night map, as in a game) — dark
/// otherwise — and keep them matching the layout as it's edited.
fn sync_editor_lights(
    editor: Res<Editor>,
    lights: Query<(Entity, &crate::power::PowerLightSlot), With<EditorPowerLight>>,
    glows: Query<(Entity, &ChildOf, &MeshMaterial3d<StandardMaterial>), With<crate::power::PowerLightGlow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    if editor.docs.is_empty() {
        return;
    }
    let lit = editor.view.look == Look::GamePowerOn && editor.map.is_night();
    crate::power::sync_power_lights(
        &editor.doc().layout.power_lights,
        if lit { 1.0 } else { 0.0 },
        editor.is_changed(),
        &lights,
        &glows,
        &mut commands,
        &mut meshes,
        &mut materials,
        || (StateScoped(AppState::LevelEditor), EditorPowerLight),
    );
}

/// On the editor's fires.
#[derive(Component)]
struct EditorFlame;

/// Keep the shown layout's fires burning, as they would in a game (always —
/// they don't wait for the power), matching it as it's edited.
fn sync_editor_flames(
    editor: Res<Editor>,
    assets: Option<Res<crate::molotov::MolotovAssets>>,
    molotov: Res<crate::molotov::MolotovSettings>,
    existing: Query<(Entity, &crate::flames::MapFlame), With<EditorFlame>>,
    mut materials: ResMut<Assets<crate::molotov::FireMaterial>>,
    mut commands: Commands,
    mut shown: Local<Vec<shared::level::Flame>>,
) {
    let wanted: &[shared::level::Flame] = if editor.docs.is_empty() { &[] } else { &editor.doc().layout.flames };
    crate::flames::sync_flames(
        wanted,
        &mut shown,
        &existing,
        assets.as_deref(),
        &molotov,
        // (Silent in the editor.)
        None,
        &mut materials,
        &mut commands,
        || (StateScoped(AppState::LevelEditor), EditorFlame),
    );
}

/// On the editor's water.
#[derive(Component)]
struct EditorWater;

/// Keep the shown layout's pools and waterfalls as a game has them,
/// matching it as it's edited.
fn sync_editor_water(
    editor: Res<Editor>,
    existing: Query<(Entity, &crate::water::MapWater), With<EditorWater>>,
    mut builder: crate::water::WaterBuilder,
    mut commands: Commands,
    mut shown: Local<crate::water::ShownWater>,
) {
    let (waters, falls): (&[shared::level::Water], &[shared::level::Waterfall]) = if editor.docs.is_empty() {
        (&[], &[])
    } else {
        let layout = &editor.doc().layout;
        (&layout.waters, &layout.waterfalls)
    };
    // (Silent in the editor.)
    builder.sync(waters, falls, &mut shown, &existing, None, &mut commands, || {
        (StateScoped(AppState::LevelEditor), EditorWater)
    });
}

/// On the editor's windows' solid look (`sync_editor_windows`).
#[derive(Component)]
struct EditorWindow;

/// A window's see-through opening box — the editor's own, hidden in a
/// clean view (its boards stay: the game has them).
#[derive(Component)]
struct EditorWindowGlass;

/// Draw the shown layout's windows solid, to line up against the openings
/// cut in the map: the opening as a see-through box, and the boards as the
/// game has them (on the outside face). Rebuilt whenever a window changes.
#[allow(clippy::too_many_arguments)]
fn sync_editor_windows(
    editor: Res<Editor>,
    look: Option<Res<crate::windows::BoardLook>>,
    existing: Query<Entity, With<EditorWindow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut glass: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    mut shown: Local<Vec<shared::windows::Window>>,
    mut glasses: Query<&mut Visibility, With<EditorWindowGlass>>,
    mut commands: Commands,
) {
    let glass_vis = if editor.view.clean.is_some() { Visibility::Hidden } else { Visibility::Inherited };
    for mut vis in &mut glasses {
        vis.set_if_neq(glass_vis);
    }
    let Some(look) = look else { return };
    let wanted: &[shared::windows::Window] = if editor.docs.is_empty() { &[] } else { &editor.doc().layout.windows };
    if wanted == shown.as_slice() && existing.iter().count() == wanted.len() {
        return;
    }
    *shown = wanted.to_vec();
    for e in &existing {
        commands.entity(e).despawn();
    }
    let (mesh, material) = glass
        .get_or_insert_with(|| {
            (
                meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
                materials.add(StandardMaterial {
                    base_color: Color::srgba(0.95, 0.75, 0.45, 0.28),
                    alpha_mode: AlphaMode::Blend,
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                }),
            )
        })
        .clone();
    for window in wanted {
        let (opening, _, half) = window.opening();
        let size = window.board_size();
        commands
            .spawn((
                StateScoped(AppState::LevelEditor),
                EditorWindow,
                Transform::from_translation(window.at.pos).with_rotation(window.rotation()),
                Visibility::default(),
            ))
            .with_children(|root| {
                root.spawn((
                    EditorWindowGlass,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(window.local(opening)).with_scale(half * 2.0),
                    glass_vis,
                    bevy::pbr::NotShadowCaster,
                ));
                for slot in 0..shared::windows::BOARDS {
                    let (center, tilt) = window.board_local(slot);
                    root.spawn((Transform::from_translation(center).with_rotation(tilt), Visibility::default()))
                        .with_children(|b| crate::windows::board_visual(b, &look, size));
                }
            });
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
) -> Option<(&'static str, Transform)> {
    Some(match id {
        ObjectId::Perk(p) => (machine_model(p).0, machines.model_transform(p)),
        ObjectId::Wunderfizz => (WUNDERFIZZ_MODEL, machines.wunderfizz_model_transform()),
        ObjectId::PackAPunch => (PAP_MODEL, pap.model_transform()),
        ObjectId::AmmoCrate(_) => (AMMO_CRATE_MODEL, ammo.model_transform()),
        ObjectId::PowerSwitch => (LEVER_MODEL, lever.model_transform()),
        ObjectId::MysteryBox => (crate::mystery_box::MYSTERY_BOX_MODEL, Transform::IDENTITY),
        ObjectId::ArmorStation => (crate::armor::ARMOR_STATION_MODEL, crate::armor::model_transform()),
        ObjectId::CraftingTable => (crate::crafting::CRAFTING_TABLE_MODEL, crate::crafting::model_transform()),
        ObjectId::ExfilRadio => (crate::exfil::RADIO_MODEL, Transform::IDENTITY),
        ObjectId::RampageInducer => (crate::rampage::RAMPAGE_MODEL, Transform::IDENTITY),
        ObjectId::PowerStation => (crate::power::POWER_STATION_MODEL, Transform::IDENTITY),
        // (No model — just its marker and rectangle; a light, its marker and
        // reach.)
        // (A fire draws itself, live — `sync_editor_flames`.)
        // (A window's drawn by `input::draw_gizmos`: its opening, boards and
        // the way zombies come in.)
        // (The water draws itself, live — `sync_editor_water`.)
        ObjectId::ExfilArea
        | ObjectId::PowerLight(_)
        | ObjectId::Flame(_)
        | ObjectId::Window(_)
        | ObjectId::Water(_)
        | ObjectId::Waterfall(_) => return None,
        ObjectId::WallBuy(_) => (crate::wall_buys::SIGN_MODEL, Transform::IDENTITY),
        // (Turned to face +Z at the model's own size — the root stretches it
        // to the door's, `root_transform_of`.)
        ObjectId::Door(_) => (crate::doors::DOOR_MODEL, crate::doors::model_transform(shared::doors::MODEL_SIZE)),
        // At their in-game sizes; both models face +Z as made (the zombie's
        // panel turn is from the game's -Z facing).
        // (A start spot shows who'll stand there.)
        ObjectId::Reference(RefKind::Player) | ObjectId::PlayerSpawn(_) => (
            "models/characters/soldier.glb",
            Transform::from_scale(Vec3::splat(avatars.0.scale)),
        ),
        ObjectId::Reference(RefKind::Zombie) => (
            crate::ZOMBIE_MODEL,
            Transform::from_scale(Vec3::splat(avatars.1.scale.max(0.001)))
                .with_rotation(Quat::from_rotation_y((avatars.1.yaw_offset_deg - 180.0).to_radians())),
        ),
    })
}

fn root_transform(at: Placement) -> Transform {
    crate::util::placed(at)
}

/// [`root_transform`] for `id` in `doc` — a door's stretched to its own size.
fn root_transform_of(id: ObjectId, doc: &Doc, at: Placement) -> Transform {
    match id {
        ObjectId::Door(i) => {
            let size = doc.layout.doors.get(i as usize).map_or(shared::doors::MODEL_SIZE, |d| d.size);
            root_transform(at.with_scale(1.0)).with_scale(size / shared::doors::MODEL_SIZE)
        }
        _ => root_transform(at),
    }
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
    mut spawned: Query<(Entity, &EditorObject, &mut Transform, &mut Visibility)>,
    mut commands: Commands,
) {
    if editor.docs.is_empty() {
        return;
    }
    let doc = editor.doc();
    let wanted = editor.objects();
    for (e, obj, mut t, mut vis) in &mut spawned {
        match obj.0.get(doc).filter(|_| wanted.contains(&obj.0)) {
            Some(at) => {
                t.set_if_neq(root_transform_of(obj.0, doc, at));
                // (The people standing in for a player / zombie and on the
                // start spots are the editor's own — not in a clean view.)
                let stand_in = matches!(obj.0, ObjectId::Reference(_) | ObjectId::PlayerSpawn(_));
                vis.set_if_neq(if stand_in && editor.view.clean.is_some() {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                });
            }
            None => commands.entity(e).despawn(),
        }
    }
    for id in wanted {
        if spawned.iter().any(|(_, o, ..)| o.0 == id) {
            continue;
        }
        let Some(at) = id.get(doc) else { continue };
        let mut object = commands.spawn((
            StateScoped(AppState::LevelEditor),
            EditorObject(id),
            root_transform_of(id, doc, at),
            Visibility::default(),
        ));
        let Some((model, offset)) = model_of(id, &machines, &pap, &ammo, &lever, &avatars) else {
            continue;
        };
        let scene = SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(model)));
        // The stand-ins stand idling, as in game.
        match id {
            ObjectId::Reference(RefKind::Player) | ObjectId::PlayerSpawn(_) => {
                object.with_children(|o| {
                    o.spawn((
                        crate::SoldierVisual,
                        crate::HeldWeapon::new(Some(shared::weapon::SlotWeapon::Gun(WeaponId::Sniper))),
                        scene,
                        offset,
                    ))
                    .observe(crate::start_soldier_animation);
                });
            }
            ObjectId::Reference(RefKind::Zombie) => {
                object.with_children(|o| {
                    o.spawn((scene, offset)).observe(crate::start_zombie_animation);
                });
            }
            // (Its glass as the game has it — `power::glaze_power_station`.)
            ObjectId::PowerStation => {
                object.with_child((crate::power::PowerStationModel, scene, offset));
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

/// A thing's size is its kind's, on every map: one changed on the shown map
/// (a scale, an undo, a revert) becomes every map's.
fn share_sizes(mut editor: ResMut<Editor>) {
    if editor.docs.is_empty() {
        return;
    }
    let used = editor.doc().layout.sizes_used();
    let changed: Vec<(String, f32)> = used
        .into_iter()
        .filter(|(key, scale)| editor.sizes.get(key) != *scale)
        .collect();
    if changed.is_empty() {
        return;
    }
    let editor = editor.as_mut();
    for (key, scale) in changed {
        editor.sizes.set(&key, scale);
    }
    let sizes = editor.sizes.clone();
    for doc in &mut editor.docs {
        let sized = doc.layout.clone().sized(&sizes);
        if sized != doc.layout {
            doc.layout = sized;
        }
    }
}
