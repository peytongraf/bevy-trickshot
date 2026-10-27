//! The `Zombies` Mystery Box (`shared::mystery_box`, `models/props/mystery_box.glb`).
//! Standing at it shows a card: its cost and whether we can spin it, or —
//! once a spin of ours has settled — the prize to take. The interact key
//! spins it ([`shared::SpinMysteryBox`]) or takes the prize
//! ([`shared::TakeBoxPrize`] → [`shared::BoxPrizeTaken`]): a gun swaps for
//! the weapon in hand (dropped where we stand, like a wall buy), a lethal
//! or tactical is one more of it (any other kind of its sort dropped).
//!
//! Everyone sees a spin, drawn from the lobby's replicated
//! [`Lobby::mystery_box`]: the lid swings open (its clip, scrubbed to the
//! lid's openness so it can run back to shut) on an amber glow fading in,
//! every prize cycles up out of it — slowing — until it settles on the one
//! rolled, which hovers there; taken, it's gone, and left, it sinks back in;
//! then the lid shuts and the glow fades. Its open, spin and close sounds
//! play from the box for the whole lobby ([`BoxSound`]), and only the one
//! who spun gets the prompt to take the prize (the server checks too).
//!
//! Where it can stand is the map's layout's spots (placed in the level
//! editor), each a stack of cinder blocks (`mystery_box_cinder_blocks.glb`)
//! that's always there; the box sits on whichever one the lobby says
//! ([`Lobby::box_spot`]). When a spin lands on the teddy bear
//! (`models/props/teddy_bear.glb`), it moves, as Black Ops 4's does: the
//! bear hovers in place of a prize, laughs, says "bye bye" and floats off;
//! the lid shuts in smoke; the box rises, rolling over backwards faster and
//! faster, and is gone in a puff; lightning strikes its old blocks and
//! leaves a bear sitting on them; and a moment later the box drops onto
//! another spot's blocks — each step with its sound, from the box
//! ([`TEDDY_CUES`], [`animate_teddy`]). As in Call of Duty, a bear sits on
//! the blocks of every spot the box isn't on. Where it sits on them, and
//! its size and turn, are the debug panel's ([`TeddySettings`],
//! [`MysteryBoxDebug`], which can also spin the box onto the bear).
//!
//! The box, its blocks, prize display, bears and card are
//! `StateScoped(InGame)` and taken away whenever we're not in a `Zombies`
//! game; the spin and the box's spot are server-owned and reset between
//! games — nothing carries into the next one.

use bevy::ecs::system::SystemParam;
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::audio_engine::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::scene::SceneInstanceReady;
use lightyear::prelude::{LocalId, MessageReceiver, TriggerSender};
use shared::mystery_box::{
    BoxPhase, BoxPrize, BYE_BYE_SECS, COST, HALF_EXTENTS, LEAVE_SECS, LID_SECS, SINK_SECS, SPIN_SECS,
};
use shared::weapon::SlotWeapon;
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::weapon_drops::DropModel;
use crate::zombies_hud::{zombies_game, CARD_RED, MONEY_YELLOW};
use crate::{killcam, menu, AppState, GameSounds, Lethal, Player, Weapon, EYE_HEIGHT, HUD_FONT};

pub(crate) const MYSTERY_BOX_MODEL: &str = "models/props/mystery_box.glb";
/// The cinder blocks at every spot, the box on top of one of them.
pub(crate) const BLOCKS_MODEL: &str = "models/props/mystery_box_cinder_blocks.glb";
/// The teddy bear a spin can land on.
pub(crate) const TEDDY_MODEL: &str = "models/props/teddy_bear.glb";

/// How far below its origin (m, as made) the bear's feet are — its model's
/// shifted up by this under [`TeddyInBox`] / [`SittingTeddy`], so they
/// stand (and turn) on its feet.
const TEDDY_FEET_Y: f32 = -0.449;
/// How high (m, the box as made) the bear's feet are as it hovers in the
/// box — just inside the rim.
const TEDDY_LOW: f32 = HALF_EXTENTS.y * 2.0 - 0.06;
/// How far (m) the bear floats up as it goes.
const TEDDY_RISE: f32 = 2.5;

// The box moving, in seconds into the spin's `BoxPhase::Leaving` (the bear
// showing) — timed off Black Ops 4's.
/// The bear laughs...
const LAUGH_AT: f32 = 0.3;
/// ...floats up and away...
const TEDDY_RISE_AT: f32 = 3.0;
const TEDDY_GONE_AT: f32 = 4.8;
/// ...saying "bye bye" (`BYE_BYE_SECS`, shared: the server times the
/// operator's quote about it off it)...
const BYE_BYE_AT: f32 = BYE_BYE_SECS;
/// ...the lid shuts in smoke...
const LID_SHUT_AT: f32 = 6.5;
/// ...the box rises with a whoosh, spinning faster, whirling...
const BOX_RISE_AT: f32 = 9.0;
const FLY_AWAY_AT: f32 = 9.5;
/// ...and is gone in a puff...
const POOF_AT: f32 = 13.8;
/// ...then lightning strikes its old blocks for this long, leaving the bear
/// sitting there...
const STRIKE_AT: f32 = 14.0;
const STRIKE_SECS: f32 = 0.8;
/// ...which lands with a crackle.
const LAND_FLUX_AT: f32 = 15.2;
const _: () = assert!(LAND_FLUX_AT < LEAVE_SECS);
/// Arriving: it drops this far (m) onto its new blocks, taking this long.
const DROP_HEIGHT: f32 = 1.2;
const DROP_SECS: f32 = 0.35;

/// The teddy bear's look, and how the box rises — the debug panel's
/// "Mystery Box (Zombies)" section ([`MysteryBoxDebug`]).
#[derive(Resource, Clone, PartialEq, Debug)]
pub(crate) struct TeddySettings {
    /// The bear's size, per the box's (spot's) size — 1 = as made. The
    /// bear in the box is this size too.
    pub(crate) scale: f32,
    /// Where the bear's feet are on a spot's cinder blocks (m, the blocks
    /// as made: from the ground under their middle, +Z their front).
    pub(crate) sit_offset: Vec3,
    /// Its turn there (°): pitch, yaw and roll, about its feet. The bear in
    /// the box takes the yaw.
    pub(crate) sit_rotation_deg: Vec3,
    /// How much higher (m) it hovers in the box.
    pub(crate) lift: f32,
    /// How far (m) the box rises before it's gone...
    pub(crate) box_rise: f32,
    /// ...and how fast (rad/s) it's rolling over backwards by then.
    pub(crate) box_spin_rate: f32,
}

impl Default for TeddySettings {
    fn default() -> Self {
        Self {
            scale: 0.55,
            sit_offset: Vec3::new(-0.17, 0.0, 0.35),
            sit_rotation_deg: Vec3::new(-20.0, 10.0, 0.0),
            lift: 0.0,
            box_rise: 2.5,
            box_spin_rate: 9.0,
        }
    }
}

impl TeddySettings {
    /// Its turn sitting on the blocks.
    fn sit_rotation(&self) -> Quat {
        let r = self.sit_rotation_deg;
        Quat::from_euler(EulerRot::YXZ, r.y.to_radians(), r.x.to_radians(), r.z.to_radians())
    }
}

/// The glow inside: amber.
pub(crate) const AMBER: Color = Color::srgb(1.0, 0.62, 0.15);
/// The glow light's brightness, fully faded in...
const GLOW_INTENSITY: f32 = 45_000.0;
/// ...and how long (s) it takes to fade in or out.
const GLOW_FADE_SECS: f32 = 0.6;

/// How high (m, the box as made) a prize's middle is at the bottom of the
/// box, and hovering at the top — just clear of the box, under the open lid.
const PRIZE_LOW: f32 = 0.06;
const PRIZE_HIGH: f32 = HALF_EXTENTS.y * 2.0 + 0.22;
/// Seconds into a spin the prizes start coming up (the lid part open).
const RISE_DELAY_SECS: f32 = 0.25;
/// Seconds each prize shows while cycling: this quick at first...
const CYCLE_FAST_SECS: f32 = 0.07;
/// ...slowing by up to this much by the time it settles.
const CYCLE_SLOWDOWN_SECS: f32 = 0.35;
/// How far (m) a hovering prize bobs, and how fast (rad/s).
const BOB: f32 = 0.015;
const BOB_SPEED: f32 = 2.2;

/// The lethals' sizes in the box (m, end to end).
const KNIFE_LENGTH: f32 = 0.34;
const MOLOTOV_LENGTH: f32 = 0.32;

/// A lethal's (or tactical's) world model and its real size end to end (m),
/// fitted by [`LethalModel`] — the monkey bomb aside (its own
/// `monkey_bomb::centered_transform`).
pub(crate) fn lethal_model(kind: shared::lethal::LethalKind) -> (&'static str, f32) {
    use shared::lethal::LethalKind as K;
    match kind {
        K::Molotov => ("models/weapons/molotov_1k.glb", MOLOTOV_LENGTH),
        K::Frag => (crate::frag::WORLD_MODEL, crate::frag::BOX_LENGTH),
        K::FlashBang => (crate::flash_bang::WORLD_MODEL, crate::flash_bang::BOX_LENGTH),
        K::ThrowingKnife | K::MonkeyBomb => ("models/weapons/throwing_knife.glb", KNIFE_LENGTH),
    }
}

/// Least time (s) between requests, so a double press before the server's
/// answer can't spin twice.
const PRESS_COOLDOWN_SECS: f32 = 0.6;

pub(crate) struct MysteryBoxPlugin;

impl Plugin for MysteryBoxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TeddySettings>()
            .add_observer(fit_lethal_model)
            .add_systems(OnEnter(AppState::InGame), spawn_box_card)
            .add_systems(
                Update,
                (
                    sync_mystery_box,
                    animate_mystery_box,
                    animate_teddy,
                    update_box_card,
                    use_mystery_box.run_if(menu::game_active.and(killcam::no_killcam)),
                    receive_prizes,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The box: its lid clip and `AnimationPlayer` (once the scene's spawned
/// one), and what it's drawing of the lobby's spin.
#[derive(Component)]
pub(crate) struct MysteryBox {
    graph: Handle<AnimationGraph>,
    clip: AnimationNodeIndex,
    player: Option<Entity>,
    glow_material: Handle<StandardMaterial>,
    /// How open the lid is (0 shut, 1 open) and how bright the glow (0..1).
    lid: f32,
    /// The game's paused — its effects hold still too.
    paused: bool,
    /// Gone from the map (moving between spots) — its effects too.
    hidden: bool,
    /// Whether the lid was opening last frame — its open / close sounds
    /// play as that flips.
    was_open: bool,
    glow: f32,
    /// The spin and phase being drawn, and seconds into that phase.
    spin_id: Option<u32>,
    phase: Option<BoxPhase>,
    phase_secs: f32,
    /// The prize showing while cycling, and seconds it's shown.
    cycling: usize,
    cycle_secs: f32,
    cycle_step: u32,
}

impl MysteryBox {
    /// How open it looks (0 shut, 1 open — the glow's fade), for its effects
    /// (`vfx::mystery_box`).
    pub(crate) fn openness(&self) -> f32 {
        self.glow
    }

    /// `dt`, or nothing while the game's paused.
    pub(crate) fn frame_secs(&self, dt: f32) -> f32 {
        if self.paused { 0.0 } else { dt }
    }

    /// Gone from the map while it moves between spots — no effects.
    pub(crate) fn hidden(&self) -> bool {
        self.hidden
    }
}

/// One of the box's sounds, playing from it (a child, so it goes with the
/// box), faded by distance by its profile in the mix.
#[derive(Component, Clone, Copy)]
enum BoxSound {
    Open,
    Spin,
    Close,
    /// One of the box moving's ([`TEDDY_CUES`]).
    Teddy,
}

/// A spot's cinder blocks (`ZombiesLayout::mystery_boxes`, by index).
#[derive(Component)]
struct BoxBlocks(#[allow(dead_code)] u8);

/// The teddy bear in the box, under the [`PrizeDisplay`].
#[derive(Component)]
struct TeddyInBox;

/// The teddy bear sitting on a spot's blocks (a child of its
/// [`BoxBlocks`]), there while the box is elsewhere.
#[derive(Component)]
struct SittingTeddy(u8);

/// The glow's light, under the box.
#[derive(Component)]
struct BoxGlowLight;

/// Where the prizes show — at real size, so not under the (scaled) box.
#[derive(Component)]
struct PrizeDisplay;

/// One prize's model, under the [`PrizeDisplay`] — only the one showing
/// is visible.
#[derive(Component)]
struct PrizeModel(BoxPrize);

/// A lethal's scene, under its [`PrizeModel`] — sized by
/// [`fit_lethal_model`] once it's in (a gun's is `weapon_drops`' upright
/// [`DropModel`]).
#[derive(Component)]
pub(crate) struct LethalModel {
    pub(crate) length: f32,
}

/// The bear's model, shifted up to stand on its feet at its parent's origin.
fn teddy_scene(asset_server: &AssetServer) -> impl Bundle {
    (
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(TEDDY_MODEL))),
        Transform::from_translation(Vec3::Y * -TEDDY_FEET_Y),
    )
}

/// Our `Zombies` game, on a map with spots for the box.
fn box_game<'a>(local: &Query<&LocalId, With<GameClient>>, lobbies: &'a Query<&Lobby>) -> Option<&'a Lobby> {
    zombies_game(local, lobbies).filter(|l| !shared::mystery_box::spots(l.map).is_empty())
}

/// Put the box, its prize display, the teddy bears and every spot's cinder
/// blocks on the map while we're in a `Zombies` game on a map with spots,
/// and take them away otherwise. (Where the box stands, and whether it's
/// showing, is [`animate_mystery_box`]'s.)
#[allow(clippy::too_many_arguments)]
fn sync_mystery_box(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    boxes: Query<Entity, With<MysteryBox>>,
    others: Query<Entity, Or<(With<PrizeDisplay>, With<BoxBlocks>)>>,
    blocks: Query<&BoxBlocks>,
    mut commands: Commands,
) {
    let Some(lobby) = box_game(&local, &lobbies) else {
        for e in boxes.iter().chain(&others) {
            commands.entity(e).despawn();
        }
        return;
    };

    // The cinder blocks at every spot — always there, solid with a box's
    // height on top (the server's `shared::mystery_box::solid_boxes`) — and
    // a teddy bear sitting on them while the box is elsewhere.
    let spots = shared::mystery_box::spots(lobby.map);
    if blocks.iter().count() != spots.len() {
        for e in &others {
            if blocks.contains(e) {
                commands.entity(e).despawn();
            }
        }
        let half = shared::mystery_box::stack_half_extents();
        for (i, &at) in spots.iter().enumerate() {
            commands
                .spawn((StateScoped(AppState::InGame), BoxBlocks(i as u8), crate::util::placed(at), Visibility::default()))
                .with_children(|b| {
                    b.spawn(SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(BLOCKS_MODEL))));
                    b.spawn((
                        crate::vfx::NoBulletHoles,
                        bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                        Transform::from_translation(Vec3::Y * half.y),
                    ));
                    b.spawn((SittingTeddy(i as u8), Transform::default(), Visibility::Hidden))
                        .with_child(teddy_scene(&asset_server));
                });
        }
    }

    if !boxes.is_empty() {
        return;
    }
    let (graph, clip) =
        AnimationGraph::from_clip(asset_server.load(GltfAssetLabel::Animation(0).from_asset(MYSTERY_BOX_MODEL)));
    let glow_material = materials.add(StandardMaterial {
        base_color: AMBER.with_alpha(0.0),
        emissive: LinearRgba::BLACK,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    let half = HALF_EXTENTS;
    let start = shared::mystery_box::current(lobby).map(shared::mystery_box::on_blocks).unwrap_or_default();
    commands
        .spawn((
            StateScoped(AppState::InGame),
            MysteryBox {
                graph: graphs.add(graph),
                clip,
                player: None,
                glow_material: glow_material.clone(),
                lid: 0.0,
                paused: false,
                hidden: false,
                was_open: false,
                glow: 0.0,
                spin_id: None,
                phase: None,
                phase_secs: 0.0,
                cycling: 0,
                cycle_secs: 0.0,
                cycle_step: 0,
            },
            crate::util::placed(start),
            Visibility::default(),
        ))
        .with_children(|b| {
            b.spawn(SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(MYSTERY_BOX_MODEL))));
            // The glow: a light low inside, and a sheet of it just under the
            // top, both faded in with the lid.
            b.spawn((
                BoxGlowLight,
                PointLight {
                    color: AMBER,
                    intensity: 0.0,
                    range: 4.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_translation(Vec3::Y * half.y * 1.4),
            ));
            b.spawn((
                Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::new(half.x * 0.94, half.z * 0.85)))),
                MeshMaterial3d(glow_material),
                Transform::from_translation(Vec3::Y * half.y * 1.75),
                NotShadowCaster,
            ));
        });
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PrizeDisplay,
            Transform::from_translation(start.pos),
            Visibility::default(),
        ))
        .with_children(|d| {
            // The bear the box lands on instead of a prize.
            d.spawn((TeddyInBox, Transform::default(), Visibility::Hidden))
                .with_child(teddy_scene(&asset_server));
            for prize in BoxPrize::ALL {
                let mut holder = d.spawn((PrizeModel(prize), Transform::default(), Visibility::Hidden));
                match prize.gun() {
                    Some(gun) => {
                        let weapon = SlotWeapon::Gun(gun);
                        holder.with_child((
                            DropModel {
                                weapon,
                                pap: 0,
                                upright: true,
                            },
                            SceneRoot(
                                asset_server
                                    .load(GltfAssetLabel::Scene(0).from_asset(crate::weapon_drops::model_path(weapon))),
                            ),
                            // (Hidden until it's fitted.)
                            Visibility::Hidden,
                        ));
                    }
                    // The monkey bomb: as made, upright, its middle at the
                    // origin (its skinned model isn't fitted by its shape).
                    None if prize == BoxPrize::MonkeyBomb => {
                        let height = crate::monkey_bomb::BOX_HEIGHT;
                        holder.with_child((
                            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(crate::monkey_bomb::MONKEY_MODEL))),
                            crate::monkey_bomb::centered_transform(height),
                        ));
                    }
                    None => {
                        let (path, length) = match prize {
                            BoxPrize::Molotov => ("models/weapons/molotov_1k.glb", MOLOTOV_LENGTH),
                            BoxPrize::Frag => (crate::frag::WORLD_MODEL, crate::frag::BOX_LENGTH),
                            BoxPrize::FlashBang => (crate::flash_bang::WORLD_MODEL, crate::flash_bang::BOX_LENGTH),
                            _ => ("models/weapons/throwing_knife.glb", KNIFE_LENGTH),
                        };
                        holder.with_child((
                            LethalModel { length },
                            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path))),
                            Visibility::Hidden,
                        ));
                    }
                }
            }
        });
}

/// A lethal's scene is in: size it by its shape and stand it with its
/// middle at the origin, its longest way along +X (or up, if that's
/// it — the molotov's bottle).
fn fit_lethal_model(
    trigger: Trigger<SceneInstanceReady>,
    models: Query<&LethalModel>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    mesh_entities: Query<&Mesh3d, Without<SkinnedMesh>>,
    meshes: Res<Assets<Mesh>>,
    mut commands: Commands,
) {
    let root = trigger.target();
    let Ok(model) = models.get(root) else { return };
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for entity in children.iter_descendants(root) {
        let Ok(mesh3d) = mesh_entities.get(entity) else { continue };
        let Some(positions) = meshes
            .get(&mesh3d.0)
            .and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION))
            .and_then(|a| a.as_float3())
        else {
            continue;
        };
        let m = crate::weapon_drops::root_from(entity, root, &parents, &transforms);
        for p in positions {
            let p = m.transform_point3(Vec3::from_array(*p));
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    let size = hi - lo;
    let long = size.max_element();
    if !long.is_finite() || long <= 1e-6 {
        warn!("mystery box: a lethal with no meshes to fit");
        return;
    }
    let scale = model.length / long;
    let turn = if size.z >= size.x && size.z >= size.y {
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    };
    let center = (lo + hi) * 0.5;
    commands.entity(root).insert((
        Transform {
            translation: -(turn * (center * scale)),
            rotation: turn,
            scale: Vec3::splat(scale),
        },
        Visibility::Inherited,
    ));
}

/// The next prize to show while cycling — never the same one twice running.
fn next_cycled(current: usize, spin: u32, step: u32) -> usize {
    let hop = 1 + (shared::bots::rand01(((spin as u64) << 32) | step as u64) * 3.999) as usize;
    (current + hop) % BoxPrize::ALL.len()
}

/// What the prize display's showing: a prize, or the teddy bear.
#[derive(Clone, Copy, PartialEq)]
enum Shown {
    Prize(BoxPrize),
    Teddy,
}

/// Where the box itself is in its spot's frame while it's moving: how far
/// (m) above its blocks, and how far (rad) it's rolled over backwards —
/// rising and rolling faster off its old spot, dropping onto its new one.
fn box_motion(spin: Option<&shared::mystery_box::MysteryBoxSpin>, t: f32, settings: &TeddySettings) -> (f32, f32) {
    match spin.map(|s| s.phase) {
        Some(BoxPhase::Leaving) if t >= BOX_RISE_AT => {
            let span = POOF_AT - BOX_RISE_AT;
            let k = ((t - BOX_RISE_AT) / span).clamp(0.0, 1.0);
            // (Rolling faster from still to `box_spin_rate`: the roll is its
            // integral.)
            (settings.box_rise * k * k, settings.box_spin_rate * span * k * k * 0.5)
        }
        Some(BoxPhase::Arriving) => {
            let k = (t / DROP_SECS).clamp(0.0, 1.0);
            (DROP_HEIGHT * (1.0 - k * k), 0.0)
        }
        _ => (0.0, 0.0),
    }
}

/// The bear in the box, `t` into the spin's [`BoxPhase::Leaving`]: how far
/// (m) it's floated up, and how big it still is (1 → 0 as it goes).
fn teddy_flight(t: f32) -> (f32, f32) {
    if t < TEDDY_RISE_AT {
        return (0.0, 1.0);
    }
    let k = ((t - TEDDY_RISE_AT) / (TEDDY_GONE_AT - TEDDY_RISE_AT)).clamp(0.0, 1.0);
    let shrink = ((k - 0.6) / 0.4).clamp(0.0, 1.0);
    (TEDDY_RISE * k * k, 1.0 - shrink * shrink)
}

/// The bear in the box, at `size` (1 = [`TeddySettings::scale`] of the
/// box's size `box_scale`), standing on its feet `lift` up, turned as it
/// sits on the blocks.
fn teddy_in_box(settings: &TeddySettings, box_scale: f32, size: f32) -> Transform {
    let s = settings.scale * box_scale.max(0.01) * size;
    Transform {
        translation: Vec3::Y * settings.lift,
        rotation: Quat::from_rotation_y(settings.sit_rotation_deg.y.to_radians()),
        scale: Vec3::splat(s.max(1e-4)),
    }
}

/// Draw the lobby's spin: the box where it is (rising and rolling off its
/// spot, gone, or dropping onto its new one, as it moves), the lid open or shutting
/// (with its sounds), the glow, and the prize cycling up, hovering, sinking
/// or gone — or the teddy bear, floating off.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn animate_mystery_box(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Res<GameSounds>,
    teddy: Res<TeddySettings>,
    mut boxes: Query<(Entity, &mut MysteryBox, &mut Transform, &mut Visibility)>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    mut lights: Query<&mut PointLight, With<BoxGlowLight>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut display: Query<&mut Transform, (With<PrizeDisplay>, Without<MysteryBox>)>,
    mut prizes: Query<(&PrizeModel, &mut Visibility), (Without<MysteryBox>, Without<TeddyInBox>)>,
    mut bear: Query<
        (&mut Transform, &mut Visibility),
        (With<TeddyInBox>, Without<MysteryBox>, Without<PrizeDisplay>, Without<PrizeModel>),
    >,
    mut commands: Commands,
) {
    let Some(lobby) = box_game(&local, &lobbies) else { return };
    let Some(spot) = shared::mystery_box::current(lobby) else { return };
    let at = shared::mystery_box::on_blocks(spot);
    let Some((box_e, mut mbox, mut box_tf, mut box_vis)) = boxes.iter_mut().next() else { return };
    let dt = if lobby.paused { 0.0 } else { time.delta_secs() };
    let spin = lobby.mystery_box;
    mbox.paused = lobby.paused;

    // A new spin, or the next phase of it: start its clock.
    if spin.map(|s| s.id) != mbox.spin_id {
        mbox.spin_id = spin.map(|s| s.id);
        mbox.cycling = 0;
        mbox.cycle_secs = 0.0;
        mbox.cycle_step = 0;
    }
    if spin.map(|s| s.phase) != mbox.phase {
        mbox.phase = spin.map(|s| s.phase);
        mbox.phase_secs = 0.0;
    }
    mbox.phase_secs += dt;
    let t = mbox.phase_secs;

    // Where it is: on its spot's blocks — or up off them, or dropping onto
    // them, while it moves — and gone from the poof until it lands.
    // (Rolling over backwards — its top away from someone in front of it,
    // about its long axis, through its middle — with a little wobble.)
    let (lift, roll) = box_motion(spin.as_ref(), t, &teddy);
    let wobble = if lift > 0.0 { 0.06 * (t * 7.0).sin() * (lift / teddy.box_rise.max(0.01)).min(1.0) } else { 0.0 };
    let tumble = Quat::from_rotation_x(-roll) * Quat::from_rotation_z(wobble);
    let middle = Vec3::Y * HALF_EXTENTS.y * at.scale.max(0.01);
    let mut placed = crate::util::placed(at);
    placed.rotation = at.rotation() * tumble;
    placed.translation += Vec3::Y * lift + at.rotation() * middle - placed.rotation * middle;
    box_tf.set_if_neq(placed);
    mbox.hidden = match spin.map(|s| s.phase) {
        Some(BoxPhase::Leaving) => t >= POOF_AT,
        Some(BoxPhase::Away) => true,
        _ => false,
    };
    box_vis.set_if_neq(if mbox.hidden { Visibility::Hidden } else { Visibility::Inherited });

    // The lid: open through the spin and the offer; shut once the prize is
    // taken, once it's sunk back in, or once the bear's floated off.
    let open = match spin {
        Some(s) => match s.phase {
            BoxPhase::Spinning | BoxPhase::Offering => true,
            BoxPhase::Closing => !s.taken && t < SINK_SECS,
            BoxPhase::Leaving => t < LID_SHUT_AT,
            BoxPhase::Away | BoxPhase::Arriving => false,
        },
        None => false,
    };
    // Its sounds, from the box: the lid opening and the spin's music
    // together, then the lid shutting.
    if open != mbox.was_open {
        mbox.was_open = open;
        let clips = if open {
            vec![(BoxSound::Open, sounds.mystery_box_open.clone()), (BoxSound::Spin, sounds.mystery_box_spin.clone())]
        } else {
            vec![(BoxSound::Close, sounds.mystery_box_close.clone())]
        };
        for (kind, clip) in clips {
            commands.entity(box_e).with_child((
                kind,
                AudioPlayer::new(clip),
                Transform::from_translation(Vec3::Y * HALF_EXTENTS.y),
                crate::positional_playback(crate::audio_engine::Volume::Linear(1.0)),
            ));
        }
    }
    let step = dt / LID_SECS;
    mbox.lid = if open { (mbox.lid + step).min(1.0) } else { (mbox.lid - step).max(0.0) };
    let fade = dt / GLOW_FADE_SECS;
    mbox.glow = if open { (mbox.glow + fade).min(1.0) } else { (mbox.glow - fade).max(0.0) };

    // The clip, scrubbed to the lid: found and held paused the first time.
    if mbox.player.is_none() {
        if let Some(found) = children.iter_descendants(box_e).find(|&e| players.contains(e)) {
            let mut player = players.get_mut(found).unwrap();
            player.start(mbox.clip).pause();
            commands.entity(found).insert(AnimationGraphHandle(mbox.graph.clone()));
            mbox.player = Some(found);
        }
    }
    if let Some(mut player) = mbox.player.and_then(|p| players.get_mut(p).ok()) {
        if let Some(clip) = player.animation_mut(mbox.clip) {
            clip.seek_to(mbox.lid * LID_SECS);
        }
    }

    // The glow.
    for mut light in &mut lights {
        light.intensity = GLOW_INTENSITY * mbox.glow * at.scale.max(0.2);
    }
    if let Some(m) = materials.get_mut(&mbox.glow_material) {
        let g = mbox.glow;
        m.base_color = AMBER.with_alpha(0.55 * g);
        let c = AMBER.to_linear();
        m.emissive = LinearRgba::rgb(c.red, c.green, c.blue) * (3.0 * g);
    }

    // The prize (or the bear): which shows, how high (the box as made),
    // and — the bear floating off — how much higher (m) and how big.
    let height = |k: f32| PRIZE_LOW + (PRIZE_HIGH - PRIZE_LOW) * k;
    let showing: Option<(Shown, f32, f32, f32)> = spin.and_then(|s| match s.phase {
        BoxPhase::Spinning => {
            if t < RISE_DELAY_SECS {
                return None;
            }
            let k = ((t - RISE_DELAY_SECS) / (SPIN_SECS - RISE_DELAY_SECS)).clamp(0.0, 1.0);
            mbox.cycle_secs += dt;
            if mbox.cycle_secs >= CYCLE_FAST_SECS + CYCLE_SLOWDOWN_SECS * k * k {
                mbox.cycle_secs = 0.0;
                mbox.cycle_step += 1;
                mbox.cycling = next_cycled(mbox.cycling, s.id, mbox.cycle_step);
            }
            Some((Shown::Prize(BoxPrize::ALL[mbox.cycling]), height(1.0 - (1.0 - k) * (1.0 - k)), 0.0, 1.0))
        }
        BoxPhase::Offering => Some((Shown::Prize(s.prize), height(1.0) + BOB * (t * BOB_SPEED).sin(), 0.0, 1.0)),
        BoxPhase::Closing if !s.taken && t < SINK_SECS => {
            let k = t / SINK_SECS;
            Some((Shown::Prize(s.prize), height(1.0 - k * k), 0.0, 1.0))
        }
        BoxPhase::Leaving if t < TEDDY_GONE_AT => {
            let (up, size) = teddy_flight(t);
            Some((Shown::Teddy, TEDDY_LOW + BOB * (t * BOB_SPEED).sin(), up, size))
        }
        BoxPhase::Closing | BoxPhase::Leaving | BoxPhase::Away | BoxPhase::Arriving => None,
    });
    for (model, mut vis) in &mut prizes {
        let shown = showing.is_some_and(|(p, ..)| p == Shown::Prize(model.0));
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
    if let Ok((mut tf, mut vis)) = bear.single_mut() {
        match showing {
            Some((Shown::Teddy, _, _, size)) => {
                tf.set_if_neq(teddy_in_box(&teddy, at.scale, size));
                vis.set_if_neq(Visibility::Inherited);
            }
            _ => {
                vis.set_if_neq(Visibility::Hidden);
            }
        }
    }
    if let (Some((_, y, up, _)), Ok(mut d)) = (showing, display.single_mut()) {
        let scale = at.scale.max(0.01);
        d.translation = at.pos + Vec3::Y * (y * scale + up);
        d.rotation = at.rotation();
    }
}

/// When (s into the spin's phase) each of the box's moving sounds plays,
/// and whether from the box (or from its old spot's blocks).
const TEDDY_CUES: [(BoxPhase, f32, TeddyCue); 9] = [
    (BoxPhase::Leaving, LAUGH_AT, TeddyCue::Sound(TeddySound::Laugh)),
    (BoxPhase::Leaving, BYE_BYE_AT, TeddyCue::Sound(TeddySound::ByeBye)),
    (BoxPhase::Leaving, LID_SHUT_AT, TeddyCue::Sound(TeddySound::Disappear)),
    (BoxPhase::Leaving, BOX_RISE_AT, TeddyCue::Sound(TeddySound::Woosh)),
    (BoxPhase::Leaving, FLY_AWAY_AT, TeddyCue::Sound(TeddySound::FlyAway)),
    (BoxPhase::Leaving, POOF_AT, TeddyCue::Sound(TeddySound::Poof)),
    (BoxPhase::Leaving, STRIKE_AT, TeddyCue::Strike),
    (BoxPhase::Leaving, LAND_FLUX_AT, TeddyCue::Sound(TeddySound::LandFlux)),
    (BoxPhase::Arriving, 0.0, TeddyCue::Sound(TeddySound::Land)),
];

/// One cue of the box moving.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TeddyCue {
    Sound(TeddySound),
    /// The lightning strike down onto the old spot's blocks.
    Strike,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TeddySound {
    Laugh,
    ByeBye,
    Disappear,
    Woosh,
    FlyAway,
    Poof,
    LandFlux,
    Land,
}

/// A cue more than this late (s) — joined partway through — is skipped
/// rather than played out of step.
const CUE_LATE_SECS: f32 = 0.5;

/// The box moving: its sounds on cue (from the box, or the strike's from
/// its old spot's blocks) and the lightning strike that leaves a bear
/// behind — and the bears sitting on every spot's blocks the box isn't on.
#[allow(clippy::too_many_arguments)]
fn animate_teddy(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Res<GameSounds>,
    teddy: Res<TeddySettings>,
    dog_assets: Option<Res<crate::dogs::DogAssets>>,
    dog_settings: Res<crate::dogs::DogSettings>,
    boxes: Query<(Entity, &MysteryBox)>,
    mut sitting: Query<(&SittingTeddy, &mut Transform, &mut Visibility)>,
    mut fired: Local<(Option<(u32, BoxPhase)>, u32)>,
    mut commands: Commands,
) {
    let Some(lobby) = box_game(&local, &lobbies) else {
        *fired = (None, 0);
        return;
    };
    let Some((box_e, mbox)) = boxes.iter().next() else { return };
    let spot = shared::mystery_box::current(lobby);
    let phase = lobby.mystery_box.map(|s| s.phase);
    let t = mbox.phase_secs;

    // The cues of this phase, each once.
    let key = lobby.mystery_box.map(|s| (s.id, s.phase));
    if key != fired.0 {
        *fired = (key, 0);
    }
    if let (Some((id, phase)), Some(spot)) = (key, spot) {
        for (i, &(cue_phase, at, cue)) in TEDDY_CUES.iter().enumerate() {
            let bit = 1 << i;
            if cue_phase != phase || t < at || fired.1 & bit != 0 {
                continue;
            }
            fired.1 |= bit;
            if t - at > CUE_LATE_SECS {
                continue;
            }
            let ground = spot.pos + Vec3::Y * shared::mystery_box::BLOCKS_HEIGHT * spot.scale;
            match cue {
                TeddyCue::Strike => {
                    if let Some(assets) = dog_assets.as_deref() {
                        crate::dogs::spawn_lightning(&mut commands, assets, &dog_settings, ground, id, STRIKE_SECS, false);
                    }
                }
                TeddyCue::Sound(which) => {
                    let clip = match which {
                        TeddySound::Laugh => sounds.mystery_box_teddy_laugh.clone(),
                        TeddySound::ByeBye => sounds.mystery_box_bye_bye.clone(),
                        TeddySound::Disappear => sounds.mystery_box_disappear.clone(),
                        TeddySound::Woosh => sounds.mystery_box_woosh.clone(),
                        TeddySound::FlyAway => sounds.mystery_box_fly_away.clone(),
                        TeddySound::Poof => sounds.mystery_box_poof.clone(),
                        TeddySound::LandFlux => sounds.mystery_box_land_flux.clone(),
                        TeddySound::Land => sounds.mystery_box_land.clone(),
                    };
                    let playback = crate::positional_playback(crate::audio_engine::Volume::Linear(1.0));
                    if which == TeddySound::LandFlux {
                        // (On the old spot, where the bear's landed.)
                        commands.spawn((
                            StateScoped(AppState::InGame),
                            BoxSound::Teddy,
                            AudioPlayer::new(clip),
                            Transform::from_translation(ground),
                            playback,
                        ));
                    } else {
                        commands.entity(box_e).with_child((
                            BoxSound::Teddy,
                            AudioPlayer::new(clip),
                            Transform::from_translation(Vec3::Y * HALF_EXTENTS.y),
                            playback,
                        ));
                    }
                }
            }
        }
    }

    // A bear sitting on every spot's blocks but the box's — and on the
    // box's too from the strike that leaves one there until the box is
    // down on its new spot.
    let gone = match phase {
        Some(BoxPhase::Leaving) => t >= STRIKE_AT,
        Some(BoxPhase::Away) => true,
        _ => false,
    };
    let placed = Transform {
        translation: teddy.sit_offset,
        rotation: teddy.sit_rotation(),
        scale: Vec3::splat(teddy.scale.max(1e-4)),
    };
    for (bear, mut tf, mut vis) in &mut sitting {
        let shown = bear.0 != lobby.box_spot.at || gone;
        tf.set_if_neq(placed);
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// What the interact key does at the box, standing at it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoxUse {
    Spin,
    TooPoor,
    /// Our spin's prize, on offer.
    Take(BoxPrize),
}

/// What we can do at the box, if we're standing at it in our `Zombies`
/// game — nothing while someone else's spin is going, or while it moves.
fn at_box(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>, player: &Transform) -> Option<BoxUse> {
    let lobby = zombies_game(local, lobbies)?;
    let at = shared::mystery_box::current(lobby)?;
    let me = local.iter().next()?.0;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !shared::mystery_box::in_range_of(at, feet, 0.0) {
        return None;
    }
    match lobby.mystery_box {
        None => {
            let points = lobby.members.iter().find(|m| m.peer == me)?.score;
            Some(if points >= COST { BoxUse::Spin } else { BoxUse::TooPoor })
        }
        Some(s) if s.user == me && s.phase == BoxPhase::Offering => Some(BoxUse::Take(s.prize)),
        Some(_) => None,
    }
}

/// Whether the interact key's the box's — not a dropped weapon's
/// (`knife_pickup`).
pub(crate) fn usable_in_reach(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&Lobby>,
    player: &Transform,
) -> bool {
    matches!(at_box(local, lobbies, player), Some(BoxUse::Spin | BoxUse::Take(_)))
}

// --- the card ----------------------------------------------------------------

#[derive(Component)]
struct BoxCard;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BoxCardText {
    Title,
    Detail,
    Action,
}

fn spawn_box_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let text = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(72.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                BoxCard,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    padding: UiRect::axes(Val::Px(22.0), Val::Px(8.0)),
                    row_gap: Val::Px(2.0),
                    min_width: Val::Px(260.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.88)),
                BorderColor(AMBER),
                BorderRadius::all(Val::Px(4.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((BoxCardText::Title, Text::new(""), text(30.0), TextColor(Color::WHITE)));
                card.spawn((BoxCardText::Detail, Text::new(""), text(20.0), TextColor(MONEY_YELLOW)));
                card.spawn((BoxCardText::Action, Text::new(""), text(20.0), TextColor(Color::WHITE)));
            });
        });
}

/// Show the card while we're at the box with something to do there (hidden
/// behind menus, during a kill cam and while dead, like the rest of the
/// HUD).
#[allow(clippy::too_many_arguments)]
fn update_box_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<BoxCard>>,
    mut texts: Query<(&BoxCardText, &mut Text, &mut TextColor)>,
) {
    let (mut vis, mut border) = card.into_inner();
    let here = (!menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .then(|| at_box(&local, &lobbies, &player))
        .flatten();
    let Some(what) = here else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    let key = binds.interact.label().to_uppercase();
    let (title, detail, detail_color, action, edge) = match what {
        BoxUse::Spin => (
            "MYSTERY BOX".to_string(),
            format!("COST  {}", crate::util::grouped(COST)),
            MONEY_YELLOW,
            format!("PRESS {key} TO SPIN"),
            AMBER,
        ),
        BoxUse::TooPoor => (
            "MYSTERY BOX".to_string(),
            format!("COST  {}", crate::util::grouped(COST)),
            CARD_RED,
            "NOT ENOUGH POINTS".to_string(),
            CARD_RED,
        ),
        BoxUse::Take(prize) => (
            prize.label().to_string(),
            prize.gun().map_or("LETHAL — FULL LOAD", |g| g.class_label()).to_string(),
            Color::srgba(1.0, 1.0, 1.0, 0.55),
            format!("PRESS {key} TO TAKE"),
            AMBER,
        ),
    };
    border.set_if_neq(BorderColor(edge));
    for (which, mut text, mut color) in &mut texts {
        let (wanted, tint) = match which {
            BoxCardText::Title => (title.clone(), Color::WHITE),
            BoxCardText::Detail => (detail.clone(), detail_color),
            BoxCardText::Action => (action.clone(), if what == BoxUse::TooPoor { CARD_RED } else { Color::WHITE }),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        color.set_if_neq(TextColor(tint));
    }
}

/// The interact key at the box, hands free: spin it (saying which lethal
/// we're full of, so it won't land on that), or take our prize — handing
/// over the weapon in hand's rounds for the drop, or how many of the other
/// lethal we carry.
#[allow(clippy::too_many_arguments)]
fn use_mystery_box(
    time: Res<Time>,
    mut last_press: Local<Option<f32>>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<Weapon>,
    hands: crate::weapons::Hands,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut spin_sender: Query<&mut TriggerSender<shared::SpinMysteryBox>, With<GameClient>>,
    mut take_sender: Query<&mut TriggerSender<shared::TakeBoxPrize>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) || !hands.free() {
        return;
    }
    let now = time.elapsed_secs();
    if last_press.is_some_and(|t| now - t < PRESS_COOLDOWN_SECS) {
        return;
    }
    match at_box(&local, &lobbies, &player) {
        Some(BoxUse::Spin) => {
            if let Ok(mut s) = spin_sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::SpinMysteryBox {
                    knives_full: weapon.lethal_full(Lethal::ThrowingKnife),
                    molotovs_full: weapon.lethal_full(Lethal::Molotov),
                    monkeys_full: weapon.lethal_full(Lethal::MonkeyBomb),
                    frags_full: weapon.lethal_full(Lethal::Frag),
                    flashes_full: weapon.lethal_full(Lethal::FlashBang),
                });
                *last_press = Some(now);
            }
        }
        Some(BoxUse::Take(prize)) => {
            let (mag, reserve) = weapon.held_ammo();
            if let Ok(mut s) = take_sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::TakeBoxPrize {
                    slot: weapon.held as u8,
                    mag,
                    reserve,
                    dropping: match prize {
                        BoxPrize::ThrowingKnife => weapon.carried_other_than(Lethal::ThrowingKnife),
                        BoxPrize::Molotov => weapon.carried_other_than(Lethal::Molotov),
                        BoxPrize::MonkeyBomb => weapon.carried_other_than(Lethal::MonkeyBomb),
                        BoxPrize::Frag => weapon.carried_other_than(Lethal::Frag),
                        BoxPrize::FlashBang => weapon.carried_other_than(Lethal::FlashBang),
                        _ => None,
                    },
                });
                *last_press = Some(now);
            }
        }
        _ => {}
    }
}

/// The server handed us our prize: a gun's in our hands (full), drawn, or
/// one more of the lethal / tactical (up to the most carried) — and the
/// pickup sound, just for us.
fn receive_prizes(
    mut receivers: Query<&mut MessageReceiver<shared::BoxPrizeTaken>>,
    mut weapon: ResMut<Weapon>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for got in rx.receive() {
            match got.prize {
                BoxPrize::ThrowingKnife => weapon.add_one(Lethal::ThrowingKnife),
                BoxPrize::Molotov => weapon.add_one(Lethal::Molotov),
                BoxPrize::MonkeyBomb => weapon.add_one(Lethal::MonkeyBomb),
                BoxPrize::Frag => weapon.add_one(Lethal::Frag),
                BoxPrize::FlashBang => weapon.add_one(Lethal::FlashBang),
                gun => {
                    if let Some(gun) = gun.gun() {
                        weapon.take_into_slot(got.slot as usize, SlotWeapon::Gun(gun), None);
                    }
                }
            }
            commands.spawn((AudioPlayer::new(sounds.pick_up_equipment.clone()), PlaybackSettings::DESPAWN));
        }
    }
}

// --- debug ---------------------------------------------------------------------

/// The debug panel's teddy bear controls (its "Mystery Box (Zombies)"
/// section, above the box's effects).
#[derive(SystemParam)]
pub(crate) struct MysteryBoxDebug<'w, 's> {
    settings: ResMut<'w, TeddySettings>,
    local: Query<'w, 's, &'static LocalId, With<GameClient>>,
    lobbies: Query<'w, 's, &'static Lobby>,
    spin: Query<'w, 's, &'static mut TriggerSender<shared::SpinTeddy>, With<GameClient>>,
}

impl MysteryBoxDebug<'_, '_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        let lobby = box_game(&self.local, &self.lobbies);
        match lobby {
            Some(l) => {
                let spots = shared::mystery_box::spots(l.map).len();
                let chance = shared::mystery_box::teddy_chance(l.box_spot.spins, spots);
                ui.label(format!(
                    "On spot {} of {spots}, spun {} time(s) there — next spin {:.0}% teddy bear{}.",
                    l.box_spot.at + 1,
                    l.box_spot.spins,
                    chance * 100.0,
                    match l.mystery_box.map(|s| s.phase) {
                        Some(BoxPhase::Leaving) => " (leaving)",
                        Some(BoxPhase::Away) => " (away)",
                        Some(BoxPhase::Arriving) => " (arriving)",
                        Some(_) => " (spinning)",
                        None => "",
                    }
                ));
            }
            None => {
                ui.label("Join a Zombies game on a map with Mystery Box spots to test.");
            }
        }
        let idle = lobby.is_some_and(|l| l.mystery_box.is_none() && !l.paused);
        if ui
            .add_enabled(idle, egui::Button::new("Spin onto the teddy bear (leader, free)"))
            .on_hover_text("Spins the box where it is and lands on the bear: the whole move plays, with its sounds, and it ends up on another spot (the same one if the map only has one).")
            .clicked()
        {
            if let Ok(mut s) = self.spin.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::SpinTeddy);
            }
        }
        let s = &mut *self.settings;
        ui.label("Teddy bear on the cinder blocks (every spot the box isn't on):");
        ui.add(egui::Slider::new(&mut s.scale, 0.1..=3.0).text("bear size (× box size)"));
        ui.add(egui::Slider::new(&mut s.sit_offset.x, -1.0..=1.0).text("bear x on blocks (m, as made)"));
        ui.add(egui::Slider::new(&mut s.sit_offset.y, -0.5..=1.0).text("bear y on blocks (m, as made)"));
        ui.add(egui::Slider::new(&mut s.sit_offset.z, -1.0..=1.0).text("bear z on blocks (m, as made — + is the front)"));
        ui.add(egui::Slider::new(&mut s.sit_rotation_deg.x, -180.0..=180.0).text("bear pitch (°)"));
        ui.add(egui::Slider::new(&mut s.sit_rotation_deg.y, -180.0..=180.0).text("bear yaw (°)"));
        ui.add(egui::Slider::new(&mut s.sit_rotation_deg.z, -180.0..=180.0).text("bear roll (°)"));
        ui.label("In the box (size and yaw are the ones above):");
        ui.add(egui::Slider::new(&mut s.lift, -0.5..=0.5).text("bear lift in box (m)"));
        ui.add(egui::Slider::new(&mut s.box_rise, 0.5..=6.0).text("box rise (m)"));
        ui.add(egui::Slider::new(&mut s.box_spin_rate, 0.0..=20.0).text("box roll by the poof (rad/s)"));
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                *s = TeddySettings::default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("Mystery Box teddy bear: {s:#?}");
            }
        });
        ui.separator();
    }
}
