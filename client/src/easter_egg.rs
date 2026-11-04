//! Der Tempel's easter egg (`shared::easter_egg`), the client's side — the
//! plan's in `client/notes/der-tempel-easter-egg/`.
//!
//! So far:
//!
//! * its pieces standing where the map's layout puts them (placed in the
//!   level editor) — the sarcophagus, the society's case and the pyramid's
//!   altar, each its model ([`spawn_piece_look`], shared with the editor).
//!   Once the power's on the case can be opened: taking the dagger swings
//!   its glass lid up on its hinges, and the dagger's gone once it's open
//!   ([`animate_case`]);
//! * Barrett's blood lying where a Big Steve dropped it, glowing and turning;
//! * the prompt and interact key ([`shared::EggInteract`]) at the vial, at
//!   the case, and in the ritual area once both are had — accept (or back
//!   out of) the ritual;
//! * what the squad's gathered, on the HUD;
//! * the ritual ([`shared::easter_egg::RITUAL_SECS`]): every player locked
//!   in place ([`EggRitual`], `menu::game_active`) while placeholder
//!   pieces play it out at the altar — the dagger plunged in, the vial
//!   poured over it, the pool of blood growing, the channel running to the
//!   sarcophagus and lighting it up — each phase captioned until its
//!   animations are made ([`animate_ritual`]). Once it's done, they stay as
//!   they ended;
//! * the Hierophant's line from the sarcophagus near the ritual's end, and
//!   once it's gone wrong, his gloating — the radio following him, from the
//!   server ([`hierophant_speaks`]). What the map does then, and its
//!   lights stuttering, is `environment::egg_mood`'s; the zombies dropping
//!   dead and the round held are the server's;
//! * the cut scene from the ritual until the fight's on — everyone held
//!   still, no weapon, the view turned onto the altar, the sarcophagus and
//!   him, black bars top and bottom (`cinematic`);
//! * him waking — the lid blasting off, his spirit pouring into the body,
//!   the stillness, the snap upright (`wake`);
//! * the fight — his health bar, his taunts, his look (`fight`);
//! * the music — the ambience out and `disastrous.mp3` in just before his
//!   ritual line, on until he's dead (`music`);
//! * before the ritual, the Hierophant whispering out of the sarcophagus
//!   now and then while we're near it ([`whisper`] — just ours to hear,
//!   positional, reverb and all; [`EggWhisperSettings`]);
//! * the debug panel's "Easter egg (Der Tempel)" section
//!   ([`EasterEggDebug`]): the progress the server keeps
//!   (`Lobby::easter_egg`), jumps to any step, the items given or taken, a
//!   reset, and the ritual area and the pyramid's spots drawn.
//!
//! Everything reads the replicated `Lobby::easter_egg`. The pieces, vial,
//! prompt and HUD are `StateScoped(InGame)` and taken away / hidden
//! whenever they stop applying; the debug drawing's switch
//! ([`EggDebugView`]), the whispers' timing ([`EggLocal`]) and the
//! ritual's clock ([`EggRitual`]) are reset on
//! leaving the game (his lines' timing is in [`EggLocal`] too), and a
//! whisper or line playing is `StateScoped(InGame)` too — a line also stops
//! if the egg's reset under it. Nothing carries into the
//! next one.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_rapier3d::prelude::Collider;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::easter_egg::{
    EasterEgg, EggStep, PYRAMID_ALTAR_HALF_EXTENTS, SARCOPHAGUS_HALF_EXTENTS, SOCIETY_CASE_HALF_EXTENTS,
};
use shared::level::{Placement, ZombiesLayout};
use shared::Lobby;

use crate::audio_engine::prelude::*;
use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::zombies_hud::zombies_game;
use crate::{killcam, menu, AppState, Player, BODY_FONT, EYE_HEIGHT, HUD_FONT};

pub(crate) use ascension::{EggSoul, EggSoulSettings, EggSoulView};
mod ascension;
mod cinematic;
mod death;
mod fight;
mod music;
mod ritual_deaths;
mod wake;

pub(crate) use cinematic::EggCinematic;
pub(crate) use fight::EggFightSettings;
pub(crate) use wake::{EggCameraShake, EggWake, EggWakeSettings};

pub(crate) struct EasterEggPlugin;

impl Plugin for EasterEggPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((wake::WakePlugin, fight::FightPlugin, music::MusicPlugin, death::DeathPlugin, ascension::AscensionPlugin))
            .add_plugins((ritual_deaths::RitualDeathsPlugin, cinematic::CinematicPlugin))
            .init_resource::<EggDebugView>()
            .init_resource::<EggWhisperSettings>()
            .init_resource::<EggLocal>()
            .init_resource::<EggRitual>()
            .add_systems(Startup, (make_placeholders, load_sounds))
            .add_systems(
                OnExit(AppState::InGame),
                |mut view: ResMut<EggDebugView>, mut local: ResMut<EggLocal>, mut ritual: ResMut<EggRitual>| {
                    *view = default();
                    *local = default();
                    *ritual = default();
                },
            )
            .add_systems(OnEnter(AppState::InGame), spawn_egg_ui)
            .add_systems(
                Update,
                (
                    sync_pieces,
                    animate_case,
                    sync_vial,
                    track_ritual,
                    sync_ritual_fx,
                    animate_ritual,
                    update_ritual_caption,
                    update_egg_prompt,
                    egg_interact.run_if(menu::game_active.and(killcam::no_killcam)),
                    pickup_sounds,
                    update_egg_hud,
                    whisper,
                    hierophant_speaks,
                    draw_spots,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The colour the editor and the debug drawing show the egg's things in.
pub(crate) const EGG_RED: Color = Color::srgb(0.78, 0.1, 0.16);

/// One of the egg's pieces with a model (a placeholder, for now).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component)]
pub(crate) enum EggPiece {
    Sarcophagus,
    SocietyCase,
    PyramidAltar,
}

impl EggPiece {
    pub(crate) const ALL: [EggPiece; 3] = [EggPiece::Sarcophagus, EggPiece::SocietyCase, EggPiece::PyramidAltar];

    /// Where `layout` puts it, if anywhere.
    pub(crate) fn placement(self, layout: &ZombiesLayout) -> Option<Placement> {
        match self {
            EggPiece::Sarcophagus => layout.sarcophagus,
            EggPiece::SocietyCase => layout.society_case,
            EggPiece::PyramidAltar => layout.pyramid_altar,
        }
    }

    /// Its solid box's half extents as made.
    pub(crate) fn half_extents(self) -> Vec3 {
        match self {
            EggPiece::Sarcophagus => SARCOPHAGUS_HALF_EXTENTS,
            EggPiece::SocietyCase => SOCIETY_CASE_HALF_EXTENTS,
            EggPiece::PyramidAltar => PYRAMID_ALTAR_HALF_EXTENTS,
        }
    }
}

// --- the placeholders ----------------------------------------------------------------

/// The plain-box stand-ins for the egg's pieces until their models are made
/// (see the easter egg's `todo.md`) — each a few boxes roughly its shape, at
/// its size, standing on the ground under its middle and facing +Z. Built
/// once; the game and the level editor both use them.
#[derive(Resource)]
pub(crate) struct EggPlaceholders {
    parts: Vec<(EggPiece, PartRole, Handle<Mesh>, Handle<StandardMaterial>, Transform)>,
    /// Barrett's blood: a glowing red vial.
    vial: (Handle<Mesh>, Handle<StandardMaterial>),
    /// The ritual's: the dagger, the pool of blood (a unit disc) and the
    /// channel (a unit box, stretched) — and their materials.
    pool: Handle<Mesh>,
    channel: Handle<Mesh>,
    glowing_blood: Handle<StandardMaterial>,
}

/// What a placeholder box is, for the ones that come and go in game.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component)]
pub(crate) enum PartRole {
    /// The dagger in the case — gone once it's taken (and the lid's open,
    /// [`animate_case`]).
    Dagger,
    /// The sarcophagus' lid — blasted off as he wakes (`wake`).
    Lid,
}

/// Where the sarcophagus' lid sits on it (its middle, in the sarcophagus'
/// own frame — what it tumbles about) — moved off it as he wakes.
#[derive(Component)]
pub(crate) struct LidRest(Vec3);

/// The sarcophagus' model: `sarcophagus_base` (a hollow stone chest) and
/// `sarcophagus_lid` ([`LID_NODE`] — blasted off as he wakes), origin on the
/// ground under its middle, its foot end facing +z.
pub(crate) const SARCOPHAGUS_MODEL: &str = "models/props/sarcophagus.glb";
/// Its lid's object, by name.
const LID_NODE: &str = "sarcophagus_lid";

/// The society's case: a dark stand with a gold-trimmed glass case on top,
/// the dagger lying on its velvet; origin on the ground under its middle,
/// its front facing +z. Its lid — the glass top ([`CASE_LID_NODES`]) —
/// swings up on its hinges along the back top edge ([`CASE_HINGE`]) as the
/// dagger's taken.
pub(crate) const SOCIETY_CASE_MODEL: &str = "models/props/society_case.glb";
/// The lid's objects, by name: the glass top and the gold trim round it.
const CASE_LID_NODES: [&str; 2] = ["glass_case_top", "upper_case_trim"];
/// The hinge line (in the case's own frame): along x, at the back top edge.
const CASE_HINGE: Vec3 = Vec3::new(0.0, 1.303, -0.23);
/// How far (rad) it swings open (back past upright), and over how long (s).
const CASE_LID_OPEN_ANGLE: f32 = 105.0 * std::f32::consts::PI / 180.0;
const CASE_LID_OPEN_SECS: f32 = 0.9;
/// Where the dagger lies on the velvet (its middle — the velvet's top at
/// 1.04 m, the blade ~1 cm thick either side of it).
const CASE_DAGGER_AT: Vec3 = Vec3::new(0.0, 1.051, 0.0);

/// The altar in the pyramid's chamber (where the Exhumers lie): a base, its
/// body and a flat top at 1 m, origin on the ground under its middle, its
/// front facing +z. (Its UVs are an even box projection — see the easter
/// egg's `todo.md` on re-exporting it.)
pub(crate) const PYRAMID_ALTAR_MODEL: &str = "models/props/pyramid_altar.glb";

/// The Hierophant's dagger (`models/props/hierophant_dagger.glb`, the 1k
/// textures — a 4k copy waits unused in `client/model_src/easter_egg/` for
/// a close-up, such as a first-person hand cut). As made it lies along +Z,
/// its tip that way, its flat facing ±X, [`DAGGER_MADE_LENGTH`] long; it's
/// drawn [`DAGGER_LENGTH`] long.
pub(crate) const DAGGER_MODEL: &str = "models/props/hierophant_dagger.glb";
const DAGGER_MADE_LENGTH: f32 = 4.45;
const DAGGER_LENGTH: f32 = 0.35;

/// How the dagger's held: lying flat across the society's case (its length
/// along the case's width, its flat facing up so its edges face the sides),
/// or point down (driven into the altar in the ritual).
#[derive(Clone, Copy)]
enum DaggerPose {
    Flat,
    PointDown,
}

/// The dagger's model at `at` (its middle), posed `pose`.
fn dagger(asset_server: &AssetServer, at: Vec3, pose: DaggerPose) -> impl Bundle {
    let rotation = match pose {
        // (Its flat, ±X as made, up — then its length, +Z as made, turned
        // to lie along +X, across the case.)
        DaggerPose::Flat => {
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
        }
        // (Its tip, +Z as made, down.)
        DaggerPose::PointDown => Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
    };
    (
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(DAGGER_MODEL))),
        Transform::from_translation(at)
            .with_rotation(rotation)
            .with_scale(Vec3::splat(DAGGER_LENGTH / DAGGER_MADE_LENGTH)),
    )
}

/// `piece`'s look under `root` (its root, placed): its model, or its
/// placeholder boxes until it has one — for the game and the level editor
/// alike. In game (`in_game`), the sarcophagus' lid is found once its model's
/// in and made the lid ([`tag_lid`]).
pub(crate) fn spawn_piece_look(
    root: &mut ChildSpawnerCommands,
    piece: EggPiece,
    placeholders: &EggPlaceholders,
    asset_server: &AssetServer,
    in_game: bool,
) {
    for part in placeholders.parts(piece) {
        root.spawn(part);
    }
    // The society's case, the dagger lying across its velvet (gone once
    // it's taken — `animate_case`).
    if piece == EggPiece::SocietyCase {
        let mut model = root.spawn((
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(SOCIETY_CASE_MODEL))),
            Transform::IDENTITY,
        ));
        if in_game {
            model.observe(tag_case_lid);
        }
        root.spawn((PartRole::Dagger, Visibility::default(), dagger(asset_server, CASE_DAGGER_AT, DaggerPose::Flat)));
    }
    if piece == EggPiece::PyramidAltar {
        root.spawn((
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(PYRAMID_ALTAR_MODEL))),
            Transform::IDENTITY,
        ));
    }
    if piece == EggPiece::Sarcophagus {
        let mut model = root.spawn((
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(SARCOPHAGUS_MODEL))),
            Transform::IDENTITY,
        ));
        if in_game {
            model.observe(tag_lid);
        }
    }
}

/// The sarcophagus' model's in: its lid ([`LID_NODE`], straight under the
/// model's root, so in the sarcophagus' own frame) is the lid, resting where
/// it was made.
fn tag_lid(
    trigger: Trigger<bevy::scene::SceneInstanceReady>,
    children: Query<&Children>,
    names: Query<&Name>,
    transforms: Query<&Transform>,
    mut commands: Commands,
) {
    let lid = children
        .iter_descendants(trigger.target())
        .find(|&e| names.get(e).is_ok_and(|n| n.as_str() == LID_NODE));
    let Some(lid) = lid else {
        warn!("{SARCOPHAGUS_MODEL} has no {LID_NODE:?} — its lid won't blast off");
        return;
    };
    let rest = transforms.get(lid).map_or(Vec3::ZERO, |t| t.translation);
    commands.entity(lid).insert((PartRole::Lid, LidRest(rest)));
}

/// One of the case's lid's objects ([`CASE_LID_NODES`]), and how it sits
/// shut (in the case's own frame — they're straight under the model's root).
#[derive(Component)]
pub(crate) struct CaseLid(Transform);

/// The case's model's in: its lid's objects are the lid, shut as made.
fn tag_case_lid(
    trigger: Trigger<bevy::scene::SceneInstanceReady>,
    children: Query<&Children>,
    names: Query<&Name>,
    transforms: Query<&Transform>,
    mut commands: Commands,
) {
    let mut found = 0;
    for e in children.iter_descendants(trigger.target()) {
        if names.get(e).is_ok_and(|n| CASE_LID_NODES.contains(&n.as_str())) {
            commands.entity(e).insert(CaseLid(transforms.get(e).copied().unwrap_or_default()));
            found += 1;
        }
    }
    if found < CASE_LID_NODES.len() {
        warn!("{SOCIETY_CASE_MODEL} is missing some of {CASE_LID_NODES:?} — its lid won't all open");
    }
}

impl EggPlaceholders {
    /// `piece`'s boxes, as children to put under its root.
    pub(crate) fn parts(
        &self,
        piece: EggPiece,
    ) -> impl Iterator<Item = (Mesh3d, MeshMaterial3d<StandardMaterial>, Transform, PartRole)> + '_ {
        self.parts
            .iter()
            .filter(move |(p, ..)| *p == piece)
            .map(|(_, role, mesh, material, t)| (Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), *t, *role))
    }
}

fn make_placeholders(
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let vial_glass = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 0.02, 0.04),
        emissive: LinearRgba::rgb(4.0, 0.1, 0.15),
        perceptual_roughness: 0.2,
        ..default()
    });
    let vial = (meshes.add(Capsule3d::new(0.025, 0.09)), vial_glass);
    // (Every piece has its model now — `SARCOPHAGUS_MODEL`,
    // `SOCIETY_CASE_MODEL`, `PYRAMID_ALTAR_MODEL` — so no boxes, until the
    // next piece that needs a stand-in.)
    let parts = Vec::new();
    let glowing_blood = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.01, 0.02),
        emissive: LinearRgba::rgb(6.0, 0.05, 0.08),
        perceptual_roughness: 0.15,
        ..default()
    });
    commands.insert_resource(EggPlaceholders {
        parts,
        vial,
        pool: meshes.add(Cylinder::new(1.0, 0.02)),
        channel: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        glowing_blood,
    });
}

// --- in game -------------------------------------------------------------------------

/// The egg's pieces where the layout puts them — in a `Zombies` game on its
/// map — solid to the local player as they are on the server.
fn sync_pieces(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    placeholders: Res<EggPlaceholders>,
    asset_server: Res<AssetServer>,
    mut spawned: Query<(Entity, &EggPiece, &mut Transform)>,
    mut commands: Commands,
) {
    let layout = zombies_game(&local, &lobbies)
        .filter(|l| shared::easter_egg::on_map(l.map))
        .map(|l| shared::level::layout(l.map));
    for (e, piece, mut t) in &mut spawned {
        match layout.and_then(|l| piece.placement(l)) {
            Some(at) => {
                t.set_if_neq(crate::util::placed(at));
            }
            None => commands.entity(e).despawn(),
        }
    }
    let Some(layout) = layout else { return };
    for piece in EggPiece::ALL {
        if spawned.iter().any(|(_, p, _)| *p == piece) {
            continue;
        }
        let Some(at) = piece.placement(layout) else { continue };
        let half = piece.half_extents();
        commands
            .spawn((StateScoped(AppState::InGame), piece, crate::util::placed(at), Visibility::default()))
            .with_children(|root| {
                spawn_piece_look(root, piece, &placeholders, &asset_server, true);
                root.spawn((
                    Collider::cuboid(half.x, half.y, half.z),
                    Transform::from_translation(Vec3::Y * half.y),
                ));
            });
    }
}

/// The egg in our game, if we're in one it's part of.
fn egg_game<'a>(local: &Query<&LocalId, With<GameClient>>, lobbies: &'a Query<&Lobby>) -> Option<&'a Lobby> {
    zombies_game(local, lobbies).filter(|l| shared::easter_egg::on_map(l.map))
}

/// The case: its lid swinging up on its hinges once the dagger's been taken
/// (back down if the egg's reset), and the dagger gone once it's open — so
/// it's opened, then taken. How open it is (0..=1) is kept here; out of a
/// game it's shut again, so the next starts with it closed.
fn animate_case(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut parts: Query<(&PartRole, &mut Visibility), Without<EggPiece>>,
    mut lids: Query<(&CaseLid, &mut Transform)>,
    new_lids: Query<(), Added<CaseLid>>,
    mut open: Local<f32>,
) {
    let Some(lobby) = egg_game(&local, &lobbies) else {
        *open = 0.0;
        return;
    };
    let taken = lobby.easter_egg.dagger;
    let step = time.delta_secs() / CASE_LID_OPEN_SECS;
    let before = *open;
    *open = (*open + if taken { step } else { -step }).clamp(0.0, 1.0);
    if *open != before || !new_lids.is_empty() {
        // (Up and back about the hinge: the front edge rises first, eased
        // in and out.)
        let x = *open;
        let swing = Quat::from_rotation_x(-CASE_LID_OPEN_ANGLE * x * x * (3.0 - 2.0 * x));
        for (shut, mut tf) in &mut lids {
            let shut = shut.0;
            *tf = Transform {
                translation: CASE_HINGE + swing * (shut.translation - CASE_HINGE),
                rotation: swing * shut.rotation,
                scale: shut.scale,
            };
        }
    }
    for (role, mut vis) in &mut parts {
        let shown = match role {
            PartRole::Lid => true,
            PartRole::Dagger => !taken || *open < 1.0,
        };
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// Barrett's blood lying on the ground.
#[derive(Component)]
struct VialDrop;

/// How high (m) the vial hovers over where it fell, how far it bobs, how
/// fast (rad/s) it turns.
const VIAL_HOVER: f32 = 0.45;
const VIAL_BOB: f32 = 0.06;
const VIAL_SPIN: f32 = 1.6;

/// The vial where it lies (while it's still to be picked up), hovering,
/// bobbing and turning, a red glow round it.
fn sync_vial(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    placeholders: Res<EggPlaceholders>,
    mut vials: Query<(Entity, &mut Transform), With<VialDrop>>,
    mut commands: Commands,
) {
    let at = egg_game(&local, &lobbies).and_then(|l| l.easter_egg.vial_at);
    let Some(at) = at else {
        for (e, _) in &vials {
            commands.entity(e).despawn();
        }
        return;
    };
    let t = time.elapsed_secs();
    let place = Transform::from_translation(at + Vec3::Y * (VIAL_HOVER + VIAL_BOB * (t * 2.0).sin()))
        .with_rotation(Quat::from_rotation_y(t * VIAL_SPIN) * Quat::from_rotation_z(0.35));
    if let Some((_, mut transform)) = vials.iter_mut().next() {
        *transform = place;
        return;
    }
    let (mesh, material) = placeholders.vial.clone();
    commands
        .spawn((StateScoped(AppState::InGame), VialDrop, place, Visibility::default()))
        .with_children(|v| {
            v.spawn((Mesh3d(mesh), MeshMaterial3d(material)));
            v.spawn((
                PointLight {
                    color: Color::srgb(1.0, 0.1, 0.1),
                    intensity: 30_000.0,
                    range: 3.0,
                    shadows_enabled: false,
                    ..default()
                },
                bevy::pbr::NotShadowCaster,
            ));
        });
}

// --- the prompt and interact ------------------------------------------------------------

/// What we could do with the egg where we stand.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Here {
    Vial,
    CaseLocked,
    Dagger,
    /// In the ritual area, both had: accept it — or, having accepted, back
    /// out (`ready` of `of` players accepted so far).
    Altar { accepted: bool, ready: usize, of: usize },
}

impl Here {
    fn title(self) -> &'static str {
        match self {
            Here::Vial => "BARRETT'S BLOOD",
            Here::CaseLocked | Here::Dagger => "SOCIETY CASE",
            Here::Altar { .. } => "THE RITUAL",
        }
    }

    fn line(self, key: &str) -> String {
        match self {
            Here::Vial => format!("Press {key} to pick up the vial"),
            Here::CaseLocked => "Locked. It's wired into the power".to_string(),
            Here::Dagger => format!("Press {key} to open the case and take the dagger"),
            Here::Altar { accepted: false, ready, of } => {
                format!("Press {key} to offer your blood  ({ready}/{of} ready)")
            }
            Here::Altar { accepted: true, ready, of } => {
                format!("Waiting for everyone  ({ready}/{of} ready)  ·  Press {key} to back out")
            }
        }
    }

    /// What the interact key does here, if anything.
    fn action(self) -> Option<shared::easter_egg::EggAction> {
        use shared::easter_egg::EggAction;
        match self {
            Here::Vial => Some(EggAction::PickUpVial),
            Here::CaseLocked => None,
            Here::Dagger => Some(EggAction::TakeDagger),
            Here::Altar { accepted: false, .. } => Some(EggAction::Accept),
            Here::Altar { accepted: true, .. } => Some(EggAction::Decline),
        }
    }
}

/// What we could do where `player` stands, if anything.
fn here(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>, player: &Transform) -> Option<Here> {
    use shared::easter_egg::EggStep;
    let lobby = egg_game(local, lobbies)?;
    let me = local.iter().next()?.0;
    let egg = &lobby.easter_egg;
    if egg.step != EggStep::Gathering {
        return None;
    }
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if egg.vial_at.is_some_and(|at| shared::easter_egg::within(at, feet, shared::easter_egg::VIAL_REACH, 0.0)) {
        return Some(Here::Vial);
    }
    if !egg.dagger && shared::easter_egg::at_case(lobby.map, feet, 0.0) {
        return Some(if shared::easter_egg::case_open(lobby) { Here::Dagger } else { Here::CaseLocked });
    }
    if egg.at_altar() && shared::easter_egg::in_ritual_area(lobby.map, feet) {
        return Some(Here::Altar {
            accepted: egg.accepted.contains(&me),
            ready: egg.accepted.len(),
            of: lobby.real_peers().len(),
        });
    }
    None
}

#[derive(Component)]
struct EggPrompt;

#[derive(Component)]
struct EggPromptTitle;

#[derive(Component)]
struct EggPromptText;

/// What the squad's gathered, on the HUD.
#[derive(Component)]
struct EggHud;

#[derive(Component)]
struct EggHudText;

const PROMPT_TITLE_BG: Color = Color::srgba(0.04, 0.04, 0.05, 0.92);
const PROMPT_LINE_BG: Color = Color::srgba(0.28, 0.03, 0.05, 0.9);

fn spawn_egg_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    let heading = asset_server.load(HUD_FONT);
    let body = asset_server.load(BODY_FONT);
    commands
        .spawn((
            StateScoped(AppState::InGame),
            EggPrompt,
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(57.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|col| {
            col.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(PROMPT_TITLE_BG),
            ))
            .with_child((
                EggPromptTitle,
                Text::new(""),
                TextFont {
                    font: heading.clone(),
                    font_size: 40.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            col.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(40.0), Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(PROMPT_LINE_BG),
            ))
            .with_child((
                EggPromptText,
                Text::new(""),
                TextFont {
                    font: body.clone(),
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.9, 0.9)),
            ));
        });
    commands
        .spawn((
            StateScoped(AppState::InGame),
            EggHud,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(24.0),
                top: Val::Percent(38.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            Visibility::Hidden,
        ))
        .with_child((
            EggHudText,
            Text::new(""),
            TextFont {
                font: body,
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.75, 0.72)),
        ));
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RitualCaption,
            crate::hud::KeepThroughHudFade,
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Percent(18.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_child((
            RitualCaptionText,
            Text::new(""),
            TextFont {
                font: asset_server.load(BODY_FONT),
                font_size: 26.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.82, 0.8)),
            TextShadow::default(),
        ));
}

/// Show the prompt where there's something to do (hidden behind menus,
/// during a kill cam and once dead, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_egg_prompt(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut prompt: Single<&mut Visibility, With<EggPrompt>>,
    mut title: Single<&mut Text, (With<EggPromptTitle>, Without<EggPromptText>)>,
    mut text: Single<&mut Text, (With<EggPromptText>, Without<EggPromptTitle>)>,
) {
    let here = player
        .filter(|_| !menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .and_then(|p| here(&local, &lobbies, &p));
    let Some(here) = here else {
        prompt.set_if_neq(Visibility::Hidden);
        return;
    };
    prompt.set_if_neq(Visibility::Inherited);
    if title.0 != here.title() {
        title.0 = here.title().to_string();
    }
    let line = here.line(&binds.interact.label().to_uppercase());
    if text.0 != line {
        text.0 = line;
    }
}

/// The interact key where there's something to do: ask the server
/// ([`shared::EggInteract`] — it re-checks it all).
fn egg_interact(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    death: Res<crate::death_effect::DeathEffect>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    mut tx: Query<&mut TriggerSender<shared::EggInteract>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) || death.is_active() {
        return;
    }
    let Some(action) = player.and_then(|p| here(&local, &lobbies, &p)).and_then(Here::action) else {
        return;
    };
    if let Ok(mut tx) = tx.single_mut() {
        tx.trigger::<shared::LobbyChannel>(shared::EggInteract { action });
    }
}

/// What [`pickup_sounds`] last saw: the vial and dagger had, and where the
/// vial lay.
#[derive(Default)]
struct LastGathered {
    vial: bool,
    dagger: bool,
    vial_at: Option<Vec3>,
}

/// The vial or the dagger just taken — by us (we're the one standing at
/// it): the normal equipment pickup sound, as for anything picked up.
fn pickup_sounds(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    sounds: Option<Res<crate::GameSounds>>,
    mut last: Local<LastGathered>,
    mut commands: Commands,
) {
    let Some(lobby) = egg_game(&local, &lobbies) else {
        // (Out of a game: forgotten, so the next starts fresh.)
        *last = default();
        return;
    };
    let egg = &lobby.easter_egg;
    let feet = player.map(|p| p.translation - Vec3::Y * EYE_HEIGHT);
    // (A little slack: the server's had it a tick or two.)
    let took_vial = egg.vial
        && !last.vial
        && last.vial_at.zip(feet).is_some_and(|(at, feet)| {
            shared::easter_egg::within(at, feet, shared::easter_egg::VIAL_REACH, 1.0)
        });
    let took_dagger =
        egg.dagger && !last.dagger && feet.is_some_and(|feet| shared::easter_egg::at_case(lobby.map, feet, 1.0));
    if took_vial || took_dagger {
        if let Some(sounds) = sounds {
            commands.spawn((AudioPlayer::new(sounds.pick_up_equipment.clone()), PlaybackSettings::DESPAWN));
        }
    }
    last.vial = egg.vial;
    last.dagger = egg.dagger;
    if egg.vial_at.is_some() || !egg.vial {
        last.vial_at = egg.vial_at;
    }
}

/// What the squad's gathered, while they're still gathering (hidden behind
/// menus and during a kill cam).
fn update_egg_hud(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut hud: Single<&mut Visibility, With<EggHud>>,
    mut text: Single<&mut Text, With<EggHudText>>,
) {
    let egg = egg_game(&local, &lobbies)
        .map(|l| &l.easter_egg)
        .filter(|e| e.step == shared::easter_egg::EggStep::Gathering && (e.vial || e.dagger))
        .filter(|_| !menu.is_open() && active_killcam.0.is_none());
    let Some(egg) = egg else {
        hud.set_if_neq(Visibility::Hidden);
        return;
    };
    hud.set_if_neq(Visibility::Inherited);
    let mut lines = Vec::new();
    if egg.vial {
        lines.push("Barrett's blood");
    }
    if egg.dagger {
        lines.push("The Hierophant's dagger");
    }
    let line = lines.join("\n");
    if text.0 != line {
        text.0 = line;
    }
}

// --- the ritual --------------------------------------------------------------------------

/// The ritual's clock here (s, held while the game's paused) — `Some` while
/// the squad's doing it, when every player's locked in place
/// (`menu::game_active`).
#[derive(Resource, Default)]
pub(crate) struct EggRitual {
    t: Option<f32>,
}

impl EggRitual {
    /// Whether we're locked in place for it.
    pub(crate) fn locked(&self) -> bool {
        self.t.is_some()
    }
}

/// When (s into the ritual) each of its phases starts, and its caption —
/// stand-ins for the animations and chant still to be made.
const RITUAL_PHASES: [(f32, &str); 5] = [
    (0.0, "The dagger is driven into the altar..."),
    (3.0, "...Barrett's blood poured over it..."),
    (6.0, "...each of you cuts your hand, and your blood runs into the channel..."),
    (9.5, "...the channel lights up, running to the sarcophagus..."),
    (13.0, "\"Sanguis datus. Porta clausa.\"  (Blood given. The door is closed.)"),
];

fn track_ritual(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut ritual: ResMut<EggRitual>,
) {
    let doing = egg_game(&local, &lobbies).is_some_and(|l| l.easter_egg.step == shared::easter_egg::EggStep::Ritual);
    match (doing, ritual.t) {
        (true, None) => ritual.t = Some(0.0),
        (true, Some(t)) if !paused.0 => ritual.t = Some(t + time.delta_secs()),
        (false, Some(_)) => ritual.t = None,
        _ => {}
    }
}

/// The ritual's placeholder pieces, under one root.
#[derive(Component)]
struct RitualFx;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum RitualPart {
    Dagger,
    Vial,
    Pool,
    Channel,
    PoolLight,
    SarcophagusLight,
}

/// The ritual's pieces there from when it begins (and as it ended, after —
/// until the egg's reset), on a map with the altar and sarcophagus placed.
fn sync_ritual_fx(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    placeholders: Res<EggPlaceholders>,
    asset_server: Res<AssetServer>,
    roots: Query<Entity, With<RitualFx>>,
    mut commands: Commands,
) {
    let wanted = egg_game(&local, &lobbies).is_some_and(|l| {
        let layout = shared::level::layout(l.map);
        l.easter_egg.step >= shared::easter_egg::EggStep::Ritual && layout.ritual_area.is_some() && layout.sarcophagus.is_some()
    });
    if !wanted {
        for e in &roots {
            commands.entity(e).despawn();
        }
        return;
    }
    if !roots.is_empty() {
        return;
    }
    let p = &*placeholders;
    let red = Color::srgb(1.0, 0.08, 0.06);
    commands
        .spawn((StateScoped(AppState::InGame), RitualFx, Transform::IDENTITY, Visibility::default()))
        .with_children(|fx| {
            // (The ritual moves its root; the model's point down under it.)
            fx.spawn((RitualPart::Dagger, Transform::default(), Visibility::Hidden))
                .with_child(dagger(&asset_server, Vec3::ZERO, DaggerPose::PointDown));
            fx.spawn((RitualPart::Vial, Mesh3d(p.vial.0.clone()), MeshMaterial3d(p.vial.1.clone()), Visibility::Hidden));
            fx.spawn((RitualPart::Pool, Mesh3d(p.pool.clone()), MeshMaterial3d(p.glowing_blood.clone()), Visibility::Hidden));
            fx.spawn((RitualPart::Channel, Mesh3d(p.channel.clone()), MeshMaterial3d(p.glowing_blood.clone()), Visibility::Hidden));
            for part in [RitualPart::PoolLight, RitualPart::SarcophagusLight] {
                fx.spawn((
                    part,
                    PointLight {
                        color: red,
                        intensity: 0.0,
                        range: 6.0,
                        shadows_enabled: false,
                        ..default()
                    },
                    bevy::pbr::NotShadowCaster,
                ));
            }
        });
}

/// `x` from 0 to 1 as `t` goes from `from` to `to` (held either side).
fn ramp(t: f32, from: f32, to: f32) -> f32 {
    ((t - from) / (to - from).max(1e-3)).clamp(0.0, 1.0)
}

/// Play the ritual out on its pieces by its clock — or, once it's over,
/// hold them as it ended.
#[allow(clippy::type_complexity)]
fn animate_ritual(
    time: Res<Time>,
    ritual: Res<EggRitual>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut parts: Query<(&RitualPart, &mut Transform, &mut Visibility), Without<PointLight>>,
    // (Apart from `parts` by the light.)
    mut lights: Query<(&RitualPart, &mut PointLight, &mut Transform)>,
) {
    let Some(lobby) = egg_game(&local, &lobbies) else { return };
    let layout = shared::level::layout(lobby.map);
    let (Some(area), Some(sarcophagus)) = (layout.ritual_area, layout.sarcophagus) else {
        return;
    };
    let t = ritual.t.unwrap_or(shared::easter_egg::RITUAL_SECS);
    let wobble = (time.elapsed_secs() * 9.0).sin() * 0.5 + 0.5;
    let center = area.at.pos;
    // The channel: across the floor from the altar's middle to the
    // sarcophagus' foot.
    let target = sarcophagus.pos;
    let run = target - center;
    let length = run.length().max(0.01);
    let along = ramp(t, 9.5, 13.0);
    for (part, mut transform, mut vis) in &mut parts {
        let (shown, place) = match part {
            // Down from above, the blade half buried.
            RitualPart::Dagger => {
                let fall = ramp(t, 0.3, 1.5).powi(3);
                let y = 1.4 + (0.1 - 1.4) * fall;
                (t >= 0.3, Transform::from_translation(center + Vec3::Y * y))
            }
            // Over the dagger, tipping over as it's poured.
            RitualPart::Vial => {
                let tip = ramp(t, 3.3, 5.0) * 2.2;
                (
                    (3.0..6.0).contains(&t),
                    Transform::from_translation(center + Vec3::new(0.08, 0.75, 0.0)).with_rotation(Quat::from_rotation_z(tip)),
                )
            }
            // Barrett's blood pooling, then theirs.
            RitualPart::Pool => {
                let r = 0.35 * ramp(t, 3.5, 6.0) + 0.45 * ramp(t, 6.0, 9.5);
                (
                    r > 0.0,
                    Transform::from_translation(center + Vec3::Y * 0.01).with_scale(Vec3::new(r.max(0.001), 1.0, r.max(0.001))),
                )
            }
            // Running out to the sarcophagus.
            RitualPart::Channel => {
                let reach = length * along;
                let mid = center + run * (reach / length) * 0.5 + Vec3::Y * 0.015;
                (
                    along > 0.0,
                    Transform::from_translation(mid)
                        .looking_to(run.normalize_or(Vec3::Z), Vec3::Y)
                        .with_scale(Vec3::new(0.14, 0.02, reach.max(0.001))),
                )
            }
            _ => continue,
        };
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        *transform = place;
    }
    for (part, mut light, mut transform) in &mut lights {
        match part {
            RitualPart::PoolLight => {
                let glow = ramp(t, 3.5, 9.5);
                light.intensity = 60_000.0 * glow * (0.8 + 0.2 * wobble);
                transform.translation = center + Vec3::Y * 0.4;
            }
            RitualPart::SarcophagusLight => {
                let glow = ramp(t, 12.5, 15.0);
                light.intensity = 250_000.0 * glow * (0.75 + 0.25 * wobble);
                light.range = 8.0;
                transform.translation = target + Vec3::Y * (SARCOPHAGUS_HALF_EXTENTS.y * 2.0 + 0.5) * sarcophagus.scale;
            }
            _ => {}
        }
    }
}

#[derive(Component)]
struct RitualCaption;

#[derive(Component)]
struct RitualCaptionText;

/// The ritual's caption for where it's got to (hidden behind menus).
fn update_ritual_caption(
    ritual: Res<EggRitual>,
    menu: Res<menu::Menu>,
    mut caption: Single<&mut Visibility, With<RitualCaption>>,
    mut text: Single<&mut Text, With<RitualCaptionText>>,
) {
    let line = ritual
        .t
        .filter(|_| !menu.is_open())
        .and_then(|t| RITUAL_PHASES.iter().rev().find(|(from, _)| t >= *from))
        .map(|(_, line)| *line);
    let Some(line) = line else {
        caption.set_if_neq(Visibility::Hidden);
        return;
    };
    caption.set_if_neq(Visibility::Inherited);
    if text.0 != line {
        text.0 = line.to_string();
    }
}

// --- the whispers -------------------------------------------------------------------------

/// The Hierophant's whispers out of the sarcophagus.
const WHISPERS: [&str; 3] = [
    "audio/zombies/easter_egg/hierophant/whisper_1.mp3",
    "audio/zombies/easter_egg/hierophant/whisper_2.mp3",
    "audio/zombies/easter_egg/hierophant/whisper_3.mp3",
];

/// His line from the sarcophagus near the ritual's end, and once it's gone
/// wrong.
const DURING_RITUAL: &str = "audio/zombies/easter_egg/hierophant/voice/during_ritual.mp3";
const RITUAL_GONE_WRONG: &str = "audio/zombies/easter_egg/hierophant/voice/ritual_gone_wrong.mp3";

#[derive(Resource)]
struct EggSounds {
    whispers: Vec<Handle<AudioSource>>,
    during_ritual: Handle<AudioSource>,
    gone_wrong: Handle<AudioSource>,
}

fn load_sounds(asset_server: Res<AssetServer>, mut commands: Commands) {
    commands.insert_resource(EggSounds {
        whispers: WHISPERS.iter().map(|p| asset_server.load(*p)).collect(),
        during_ritual: asset_server.load(DURING_RITUAL),
        gone_wrong: asset_server.load(RITUAL_GONE_WRONG),
    });
}

/// Panel-tunable whispers ("Easter egg (Der Tempel)").
#[derive(Resource, Clone, PartialEq, Debug)]
pub(crate) struct EggWhisperSettings {
    /// Heard within this far (m, across the ground) of the sarcophagus, and
    /// this far above or below it.
    pub(crate) reach: f32,
    pub(crate) reach_height: f32,
    /// The first one comes this long (s) after we get near...
    pub(crate) first_after: f32,
    /// ...then one every so often — a random gap (s) in this range, from
    /// one starting to the next (never over one still playing: the next
    /// waits for it, and [`WHISPER_BREATH`] after).
    pub(crate) gap: (f32, f32),
    /// Their volume (linear, before the mix).
    pub(crate) volume: f32,
    /// How high (m) over the sarcophagus' foot they come from.
    pub(crate) height: f32,
    /// His lines' volume (the ritual's and its going wrong — linear,
    /// before the mix; heard much further off, `mix.ron`'s
    /// "Hierophant voice").
    pub(crate) line_volume: f32,
}

impl Default for EggWhisperSettings {
    fn default() -> Self {
        Self {
            reach: 14.0,
            reach_height: 5.0,
            first_after: 6.0,
            gap: (9.0, 20.0),
            volume: 2.0,
            height: 0.8,
            line_volume: 2.0,
        }
    }
}

/// The whispers' timing in this game — reset on leaving it.
#[derive(Resource, Default)]
pub(crate) struct EggLocal {
    /// When the next whisper comes (`Time::elapsed_secs`), while we're near.
    next_whisper: Option<f32>,
    /// The one played last (not played twice running).
    last: Option<usize>,
    seq: u32,
    /// Debug: whisper now, wherever we are.
    pub(crate) whisper_now: bool,
    /// His ritual line's been said this ritual.
    ritual_line: bool,
    /// How long (s, not counting pauses) since it went wrong, while it has —
    /// and whether he's said his piece.
    unleashed_for: Option<f32>,
    gone_wrong_line: bool,
    /// The step it was at last frame.
    last_step: Option<EggStep>,
}

/// A whisper playing.
#[derive(Component)]
struct Whisper;

/// The least (s) between one whisper ending and the next starting.
const WHISPER_BREATH: f32 = 1.0;

/// Now and then, while we're near the sarcophagus and the ritual's still to
/// come, a whisper out of it — not while the game's paused.
#[allow(clippy::too_many_arguments)]
fn whisper(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    settings: Res<EggWhisperSettings>,
    sounds: Res<EggSounds>,
    mut egg_local: ResMut<EggLocal>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&Transform, With<Player>>>,
    playing: Query<(), With<Whisper>>,
    mut commands: Commands,
) {
    let s = &*settings;
    let now = time.elapsed_secs();
    let sarcophagus = egg_game(&local, &lobbies)
        .filter(|l| l.easter_egg.step == shared::easter_egg::EggStep::Gathering)
        .and_then(|l| shared::level::layout(l.map).sarcophagus);
    let Some(at) = sarcophagus else {
        egg_local.next_whisper = None;
        egg_local.whisper_now = false;
        return;
    };
    let near = player.is_some_and(|p| {
        let feet = p.translation - Vec3::Y * EYE_HEIGHT;
        let d = feet - at.pos;
        Vec3::new(d.x, 0.0, d.z).length() <= s.reach && d.y.abs() <= s.reach_height
    });
    let forced = std::mem::take(&mut egg_local.whisper_now);
    if !near && !forced {
        egg_local.next_whisper = None;
        return;
    }
    if paused.0 && !forced {
        return;
    }
    let next = *egg_local.next_whisper.get_or_insert(now + s.first_after);
    // (One at a time: the next waits for this one, and a breath after.)
    if !playing.is_empty() && !forced {
        egg_local.next_whisper = Some(next.max(now + WHISPER_BREATH));
        return;
    }
    if now < next && !forced {
        return;
    }
    egg_local.seq = egg_local.seq.wrapping_add(1);
    let roll = |k: u32| crate::util::rand01(egg_local.seq.wrapping_mul(2_654_435_761).wrapping_add(k));
    let (lo, hi) = (s.gap.0.min(s.gap.1), s.gap.0.max(s.gap.1));
    let gap = lo + roll(1) * (hi - lo);
    let count = sounds.whispers.len();
    let mut pick = ((roll(2) * count as f32) as usize).min(count.saturating_sub(1));
    if count > 1 && egg_local.last == Some(pick) {
        pick = (pick + 1) % count;
    }
    egg_local.last = Some(pick);
    egg_local.next_whisper = Some(now + gap);
    let Some(clip) = sounds.whispers.get(pick) else { return };
    commands.spawn((
        StateScoped(AppState::InGame),
        Whisper,
        AudioPlayer::new(clip.clone()),
        Transform::from_translation(at.pos + Vec3::Y * s.height * at.scale),
        crate::positional_playback(Volume::Linear(s.volume)),
    ));
}

/// One of his lines from the sarcophagus.
#[derive(Component)]
struct HierophantLine;

/// The Hierophant from the sarcophagus: his line as the ritual nears its
/// end ([`shared::easter_egg::RITUAL_VOICE_AT_SECS`]), then — it gone
/// wrong, and that line done ([`shared::easter_egg::UNLEASHED_VOICE_SECS`]
/// after, not counting pauses) — his gloating; the man on the radio comes
/// in after him (the server's). Each once, until the egg's reset — which
/// stops one playing.
#[allow(clippy::too_many_arguments)]
fn hierophant_speaks(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    ritual: Res<EggRitual>,
    settings: Res<EggWhisperSettings>,
    sounds: Res<EggSounds>,
    mut egg_local: ResMut<EggLocal>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    playing: Query<Entity, With<HierophantLine>>,
    mut commands: Commands,
) {
    let Some(lobby) = egg_game(&local, &lobbies) else {
        egg_local.ritual_line = false;
        egg_local.unleashed_for = None;
        egg_local.gone_wrong_line = false;
        return;
    };
    let step = lobby.easter_egg.step;
    // (Back to it going wrong from further on — a debug jump: all over
    // again.)
    if step == EggStep::Unleashed && egg_local.last_step.is_some_and(|l| l > EggStep::Unleashed) {
        egg_local.unleashed_for = None;
        egg_local.gone_wrong_line = false;
    }
    egg_local.last_step = Some(step);
    let Some(at) = shared::level::layout(lobby.map).sarcophagus else { return };
    let say = |commands: &mut Commands, clip: &Handle<AudioSource>| {
        commands.spawn((
            StateScoped(AppState::InGame),
            HierophantLine,
            AudioPlayer::new(clip.clone()),
            Transform::from_translation(at.pos + Vec3::Y * settings.height * at.scale),
            crate::positional_playback(Volume::Linear(settings.line_volume)),
        ));
    };
    // Reset (or not there yet): quiet, and ready to say it all again.
    if step < EggStep::Ritual {
        for e in &playing {
            commands.entity(e).despawn();
        }
        egg_local.ritual_line = false;
    }
    if ritual.t.is_some_and(|t| t >= shared::easter_egg::RITUAL_VOICE_AT_SECS) && !egg_local.ritual_line {
        egg_local.ritual_line = true;
        say(&mut commands, &sounds.during_ritual);
    }
    if !step.is_unleashed() {
        egg_local.unleashed_for = None;
        egg_local.gone_wrong_line = false;
        return;
    }
    let t = egg_local.unleashed_for.get_or_insert(0.0);
    if !paused.0 {
        *t += time.delta_secs();
    }
    if *t >= shared::easter_egg::UNLEASHED_VOICE_SECS && !egg_local.gone_wrong_line {
        egg_local.gone_wrong_line = true;
        // (Not if he's already up — a debug jump past it.)
        if step == EggStep::Unleashed {
            say(&mut commands, &sounds.gone_wrong);
        }
    }
}

/// Debug: the ritual area, the pyramid's spots and the pieces' boxes, drawn
/// over the map (the debug panel's switch).
fn draw_spots(
    view: Res<EggDebugView>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut gizmos: Gizmos,
) {
    if !view.show_spots {
        return;
    }
    let Some(lobby) = zombies_game(&local, &lobbies).filter(|l| shared::easter_egg::on_map(l.map)) else {
        return;
    };
    let layout = shared::level::layout(lobby.map);
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    if let Some(area) = layout.ritual_area {
        let c = area.corners().map(|p| p + Vec3::Y * 0.05);
        for i in 0..4 {
            gizmos.line(c[i], c[(i + 1) % 4], EGG_RED);
        }
    }
    for at in &layout.pyramid_spots {
        gizmos.circle(Isometry3d::new(at.pos + Vec3::Y * 0.05, flat), 0.4, EGG_RED);
        gizmos.arrow(at.pos + Vec3::Y * 0.05, at.pos + Vec3::Y * 0.05 + at.rotation() * Vec3::Z * 0.8, EGG_RED);
    }
    for piece in EggPiece::ALL {
        if let Some(at) = piece.placement(layout) {
            let half = piece.half_extents() * at.scale;
            gizmos.cuboid(
                Transform::from_translation(at.pos + Vec3::Y * half.y)
                    .with_rotation(at.rotation())
                    .with_scale(half * 2.0),
                EGG_RED,
            );
        }
    }
}

// --- the debug panel ---------------------------------------------------------------------

/// The debug panel's own switches for the egg — off again on leaving a game.
#[derive(Resource, Default)]
pub(crate) struct EggDebugView {
    /// Draw the ritual area, the pyramid's spots and the pieces' boxes.
    pub(crate) show_spots: bool,
}

/// What the debug panel's "Easter egg (Der Tempel)" section needs.
#[derive(SystemParam)]
pub(crate) struct EasterEggDebug<'w, 's> {
    view: ResMut<'w, EggDebugView>,
    whispers: ResMut<'w, EggWhisperSettings>,
    egg_local: ResMut<'w, EggLocal>,
    mood: ResMut<'w, crate::environment::EggMoodLook>,
    mood_fade: Res<'w, crate::environment::EggMoodFade>,
    wake_settings: ResMut<'w, EggWakeSettings>,
    wake: Res<'w, EggWake>,
    fight_settings: ResMut<'w, EggFightSettings>,
    death_settings: ResMut<'w, death::EggDeathSettings>,
    death: Res<'w, death::EggDeath>,
    soul_settings: ResMut<'w, ascension::EggSoulSettings>,
    exhumer: ResMut<'w, crate::exhumer::ExhumerFxSettings>,
    ritual_deaths: ResMut<'w, ritual_deaths::RitualDeathSettings>,
    // (Not `Weapon` or a `GiveWeapon` sender: the debug panel's system
    // already has both, elsewhere — `exhumer::ExhumerDebug` goes between.)
    exhumer_debug: ResMut<'w, crate::exhumer::ExhumerDebug>,
    soul: Res<'w, EggSoul>,
    music_settings: ResMut<'w, music::EggMusicSettings>,
    music_levels: Res<'w, music::EggMusicLevels>,
    ambient_level: Res<'w, crate::zombies_audio::ZombiesAmbientLevel>,
    local: Query<'w, 's, &'static LocalId, With<GameClient>>,
    lobbies: Query<'w, 's, &'static Lobby>,
    tx: Query<'w, 's, &'static mut TriggerSender<shared::SetEasterEgg>, With<GameClient>>,
    hierophant_tx: Query<'w, 's, &'static mut TriggerSender<shared::HierophantDebug>, With<GameClient>>,
}

impl EasterEggDebug<'_, '_> {
    /// Its section in the main debug panel (`debug_ui`).
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.view.show_spots, "Show the ritual area, pyramid spots and pieces' boxes");
        ui.collapsing("Sarcophagus whispers", |ui| {
            let s = &mut *self.whispers;
            ui.add(egui::Slider::new(&mut s.reach, 1.0f32..=40.0).text("heard within (m)"));
            ui.add(egui::Slider::new(&mut s.reach_height, 0.5f32..=20.0).text("and up / down (m)"));
            ui.add(egui::Slider::new(&mut s.first_after, 0.0f32..=30.0).text("first after (s)"));
            ui.add(egui::Slider::new(&mut s.gap.0, 1.0f32..=120.0).text("gap from (s)"));
            ui.add(egui::Slider::new(&mut s.gap.1, 1.0f32..=120.0).text("gap to (s)"));
            ui.add(egui::Slider::new(&mut s.volume, 0.0f32..=3.0).text("volume"));
            ui.add(egui::Slider::new(&mut s.height, 0.0f32..=3.0).text("height over it (m)"));
            ui.add(egui::Slider::new(&mut s.line_volume, 0.0f32..=3.0).text("his ritual / gone-wrong lines' volume"));
            ui.horizontal(|ui| {
                if ui.button("Whisper now").clicked() {
                    self.egg_local.whisper_now = true;
                }
                if ui.button("Reset").clicked() {
                    *s = EggWhisperSettings::default();
                }
                if ui.button("Print settings to console").clicked() {
                    info!("{s:#?}");
                }
            });
        });
        ui.collapsing("It goes wrong — the map", |ui| {
            crate::environment::egg_mood_section(ui, &mut self.mood, self.mood_fade.0);
        });
        ui.collapsing("He wakes", |ui| wake::wake_section(ui, &mut self.wake_settings, &self.wake));
        let egg_lobby = egg_game(&self.local, &self.lobbies);
        let leads = egg_lobby.is_some_and(|l| self.local.iter().next().is_some_and(|me| me.0 == l.leader));
        ui.collapsing("Music", |ui| {
            music::music_section(ui, &mut self.music_settings, &self.music_levels, self.ambient_level.0)
        });
        let poke = ui
            .collapsing("The fight", |ui| fight::fight_section(ui, &mut self.fight_settings, egg_lobby, leads))
            .body_returned
            .flatten();
        if let (Some(action), Ok(mut tx)) = (poke, self.hierophant_tx.single_mut()) {
            fight::send_poke(&mut tx, action);
        }
        ui.collapsing("His death", |ui| death::death_section(ui, &mut self.death_settings, &self.death));
        ui.collapsing("Into the pyramid", |ui| ascension::ascension_section(ui, &mut self.soul_settings, &self.soul));
        ui.collapsing("Ritual deaths (souls torn out as it goes wrong)", |ui| {
            ritual_deaths::ritual_deaths_section(ui, &mut self.ritual_deaths)
        });
        let in_zombies = zombies_game(&self.local, &self.lobbies).is_some();
        ui.collapsing("The Exhumer", |ui| {
            crate::exhumer::exhumer_section(ui, &mut self.exhumer, &mut self.exhumer_debug, in_zombies)
        });
        let me = self.local.iter().next().map(|l| l.0);
        let game = zombies_game(&self.local, &self.lobbies).filter(|l| shared::easter_egg::available(l));
        let Some(lobby) = game else {
            ui.label("Start a Zombies game on Der Tempel to use these.");
            return;
        };
        let is_leader = me == Some(lobby.leader);
        let now = lobby.easter_egg.clone();
        ui.label(format!(
            "Step: {}  ·  vial: {}  ·  dagger: {}",
            now.step.label(),
            if now.vial { "yes" } else { "no" },
            if now.dagger { "yes" } else { "no" },
        ));
        if !is_leader {
            ui.label("Only the party leader can change these.");
        }
        let mut wanted = None;
        ui.add_enabled_ui(is_leader, |ui| {
            ui.label("Jump to:");
            ui.horizontal_wrapped(|ui| {
                for step in EggStep::ALL {
                    if ui.selectable_label(now.step == step, step.label()).clicked() && now.step != step {
                        wanted = Some(EasterEgg::at(step));
                    }
                }
            });
            ui.add_enabled_ui(now.step == EggStep::Gathering, |ui| {
                ui.horizontal(|ui| {
                    let (mut vial, mut dagger) = (now.vial, now.dagger);
                    if ui.checkbox(&mut vial, "Have the vial").changed() {
                        wanted = Some(EasterEgg { vial, ..now.clone() });
                    }
                    if ui.checkbox(&mut dagger, "Have the dagger").changed() {
                        wanted = Some(EasterEgg { dagger, ..now.clone() });
                    }
                });
            });
            if ui.button("Reset the easter egg").clicked() {
                wanted = Some(EasterEgg::default());
            }
        });
        if let (Some(egg), Ok(mut tx)) = (wanted, self.tx.single_mut()) {
            tx.trigger::<shared::LobbyChannel>(shared::SetEasterEgg { egg });
        }
    }
}
