//! `Zombies` windows (`shared::windows`): the boards nailed across each of
//! the map's window openings, as many as `Lobby::windows_torn` says are
//! still up — for everyone. They're nailed across the outside face, the
//! zombies' side. A board a zombie rips off is flung away outside with a
//! snap; one a player puts back spins up from below outside, hangs just off
//! its place for a moment and slams home against the wall, shedding a little
//! dust and trailing smoke the whole way, which keeps curling off it for a
//! moment once it's home ([`FloatingBoard`], [`SettlingBoard`],
//! [`BoardSmokeSettings`]). All three sounds come from the window.
//!
//! Inside and close to a window missing boards, a card says what holding
//! interact does: put a board back every [`REPAIR_SECS`] — asking the
//! server ([`shared::RepairBarrier`]), which checks it all again and pays
//! for each ([`shared::BarrierRepaired`]: a `+10` and a ching as it slams
//! home) — to the sound of hammering, for as long as it's held.
//!
//! The opening's kept clear of the player's body by
//! `player::movement::resolve_window_blockers`, from [`WindowBlockers`] —
//! plain maths, so shots, grenades and every other ray go straight through.
//!
//! The boards are `models/props/wood_plank.glb`, fitted to each window
//! ([`BOARD_MODEL_SIZE`]) — plain wooden boxes if it's ever missing.
//! Everything here is `StateScoped(InGame)` and taken away whenever we're
//! not in a `Zombies` game, so nothing carries into the next one.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;

use crate::audio_engine::prelude::*;
use lightyear::prelude::{LocalId, TriggerSender};
use shared::windows::{boards_left, repairable_at, Window, BOARDS, REPAIR_SECS};
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::util::{ease, rand01, rand_roll};
use crate::vfx::{impact_material, ImpactAssets, ImpactParticle, SmokeAssets, IMPACT_MAX};
use bevy::ecs::system::SystemParam;
use bevy_egui::egui;
use bevy::render::view::NoFrustumCulling;
use crate::zombies_hud::{zombies_game, PERK_CARD_WIDTH};
use crate::{killcam, menu, revive, AppState, GameSounds, Player, EYE_HEIGHT, HUD_FONT};

/// The board model...
pub(crate) const BOARD_MODEL: &str = "models/props/wood_plank.glb";
/// ...as made (m): length along x, height along y, thickness along z — it's
/// stretched to each window's [`Window::board_size`].
pub(crate) const BOARD_MODEL_SIZE: Vec3 = Vec3::new(1.4, 0.1, 0.03);

/// A stand-in board's colour (no model): old pine.
const BOARD_BROWN: Color = Color::srgb(0.45, 0.31, 0.18);
/// The card's colour: the same wood, lighter.
const BOARD_TAN: Color = Color::srgb(0.8, 0.62, 0.4);

/// A board being put back (the way Call of Duty's are): spinning up from
/// below outside for `RISE_SECS`, hanging just off its place, drifting in,
/// for `HOVER_SECS`, then slammed home in `SLAM_SECS` — with a little dust
/// coming off it the whole way, and a puff as it hits.
const RISE_SECS: f32 = 0.25;
const HOVER_SECS: f32 = 0.4;
const SLAM_SECS: f32 = 0.1;
const FLOAT_SECS: f32 = RISE_SECS + HOVER_SECS + SLAM_SECS;
/// Dust puffs a second off a board on its way in...
const BOARD_DUST_RATE: f32 = 9.0;
/// ...and in the puff as it slams home.
const SLAM_DUST: u32 = 8;
const DUST_TINT: [f32; 3] = [0.72, 0.66, 0.58];
/// Seconds a torn-off board flies before it's gone.
const FLING_SECS: f32 = 1.4;

/// The smoke pouring off a board being put back (Call of Duty's): wisps all
/// along it the whole way in, mostly left hanging where it's been, a puff
/// as it slams home, and a little more curling off it for a moment after.
/// Debug panel → "Window boards (Zombies)".
#[derive(Resource, Clone)]
pub(crate) struct BoardSmokeSettings {
    pub(crate) enabled: bool,
    /// Wisps a second off a board on its way in.
    pub(crate) rate: f32,
    /// Wisps in the puff as it slams home.
    pub(crate) slam_puffs: u32,
    /// Seconds it keeps smoking once it's home, tapering off...
    pub(crate) linger_secs: f32,
    /// ...from this many wisps a second.
    pub(crate) linger_rate: f32,
    /// Seconds each wisp hangs (±30%).
    pub(crate) lifetime: f32,
    /// Seconds each wisp takes to thicken in.
    pub(crate) fade_in: f32,
    /// Wisp size (m) as it leaves the board, and once it's spread out.
    pub(crate) start_size: f32,
    pub(crate) end_size: f32,
    /// How thick a single wisp gets at its thickest.
    pub(crate) opacity: f32,
    /// How fast it rises (m/s) once it's settled.
    pub(crate) rise: f32,
    /// Random drift (m/s) each wisp leaves with.
    pub(crate) spread: f32,
    /// How much of the board's own speed a wisp keeps (0 = left hanging
    /// where the board was, 1 = carried along with it).
    pub(crate) trail: f32,
    /// How quickly a wisp loses its starting speed and just rises.
    pub(crate) drag: f32,
    pub(crate) color: [f32; 3],
}

impl Default for BoardSmokeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            rate: 85.0,
            slam_puffs: 8,
            linger_secs: 0.4,
            linger_rate: 20.0,
            lifetime: 0.7,
            fade_in: 0.08,
            start_size: 0.05,
            end_size: 0.15,
            opacity: 0.03,
            rise: 0.15,
            spread: 0.03,
            trail: 0.0,
            drag: 0.0,
            color: [0.8, 0.8, 0.82],
        }
    }
}

pub(crate) struct WindowsPlugin;

impl Plugin for WindowsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WindowBlockers>()
            .init_resource::<Repairing>()
            .init_resource::<BoardSmokeSettings>()
            .add_systems(Startup, load_board_look)
            .add_systems(OnEnter(AppState::InGame), spawn_barrier_card)
            .add_systems(OnExit(AppState::InGame), |mut r: ResMut<Repairing>| *r = Repairing::default())
            .add_systems(
                Update,
                (
                    sync_windows,
                    sync_boards,
                    float_boards,
                    settle_boards,
                    fling_boards,
                    repair_barrier.run_if(menu::game_active.and(killcam::no_killcam).and(revive::up)),
                    stop_repairing.run_if(not(menu::game_active.and(killcam::no_killcam).and(revive::up))),
                    hammer,
                    receive_barrier_repaired,
                    update_barrier_card,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            // (Ungated: what takes them away once the game's left.)
            .add_systems(Update, clear_windows.run_if(not(in_state(AppState::InGame))));
    }
}

/// The windows of the `Zombies` game we're in (none out of one) — what keeps
/// the player out of their openings (`player::movement`, `player::mantle`).
#[derive(Resource, Default)]
pub(crate) struct WindowBlockers(pub(crate) Vec<Window>);

/// The window we're rebuilding (interact held at it), and when the next
/// board's due.
#[derive(Resource, Default)]
struct Repairing {
    window: Option<usize>,
    next_at: f32,
}

/// How a board's drawn: the model, or (without it) a wooden box. (The level
/// editor draws its windows' boards with it too.)
#[derive(Resource)]
pub(crate) struct BoardLook {
    model: Option<Handle<Scene>>,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn load_board_look(
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let have_model = bevy::asset::io::file::FileAssetReader::get_base_path()
        .join("assets")
        .join(BOARD_MODEL)
        .exists();
    commands.insert_resource(BoardLook {
        model: have_model.then(|| asset_server.load(GltfAssetLabel::Scene(0).from_asset(BOARD_MODEL))),
        mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        material: materials.add(StandardMaterial {
            base_color: BOARD_BROWN,
            perceptual_roughness: 0.85,
            ..default()
        }),
    });
}

/// A board of `size` (m) drawn as a child of `parent`, centred on it.
pub(crate) fn board_visual(parent: &mut ChildSpawnerCommands, look: &BoardLook, size: Vec3) {
    match &look.model {
        Some(model) => {
            parent.spawn((SceneRoot(model.clone()), Transform::from_scale(size / BOARD_MODEL_SIZE)));
        }
        None => {
            parent.spawn((
                Mesh3d(look.mesh.clone()),
                MeshMaterial3d(look.material.clone()),
                Transform::from_scale(size),
            ));
        }
    }
}

/// A window's root, where it stands.
#[derive(Component)]
struct WindowRoot {
    index: usize,
    /// Boards up as last seen (`None` till the first look, so a window
    /// already missing boards when we got here doesn't play it all out).
    seen: Option<u8>,
}

/// One of a window's board places (`0` the bottom one), drawn while that
/// board's up.
#[derive(Component)]
struct BoardSlot(u8);

/// A board being put back on window `window`'s slot `slot` (world
/// transforms): spinning up from `start` to `hover`, drifting from there to
/// `drift`, then slammed onto `to` — see [`RISE_SECS`].
#[derive(Component)]
struct FloatingBoard {
    window: usize,
    slot: u8,
    start: Transform,
    hover: Transform,
    drift: Transform,
    to: Transform,
    age: f32,
    /// Dust puffs owed (fractional).
    dust: f32,
    /// Smoke wisps owed (fractional).
    smoke: f32,
    /// Where it was last frame (its speed, for the smoke it leaves).
    last: Vec3,
}

/// A board just slammed home (world transform `at`, `size` m) still smoking
/// for [`BoardSmokeSettings::linger_secs`] — nothing drawn.
#[derive(Component)]
struct SettlingBoard {
    at: Transform,
    size: Vec3,
    age: f32,
    smoke: f32,
}

/// A board torn off, flying away.
#[derive(Component)]
struct FlungBoard {
    velocity: Vec3,
    spin: Vec3,
    age: f32,
}

/// Put the map's windows in while we're in a `Zombies` game (and take them
/// away otherwise).
fn sync_windows(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    look: Option<Res<BoardLook>>,
    roots: Query<Entity, With<WindowRoot>>,
    mut blockers: ResMut<WindowBlockers>,
    mut shown: Local<Option<shared::MapId>>,
    mut commands: Commands,
) {
    let Some(look) = look else { return };
    let map = zombies_game(&local, &lobbies).map(|l| l.map);
    let windows = map.map_or(&[][..], shared::windows::windows);
    if *shown == map && (map.is_none() || !roots.is_empty() || windows.is_empty()) {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    *shown = map;
    blockers.0 = windows.to_vec();
    for (index, window) in windows.iter().enumerate() {
        let size = window.board_size();
        commands
            .spawn((
                StateScoped(AppState::InGame),
                WindowRoot { index, seen: None },
                Transform::from_translation(window.at.pos).with_rotation(window.rotation()),
                Visibility::default(),
            ))
            .with_children(|root| {
                for slot in 0..BOARDS {
                    let (center, tilt) = window.board_local(slot);
                    root.spawn((
                        BoardSlot(slot),
                        Transform::from_translation(center).with_rotation(tilt),
                        Visibility::Hidden,
                    ))
                    .with_children(|b| board_visual(b, &look, size));
                }
            });
    }
}

/// Take the windows away out of a game.
fn clear_windows(
    roots: Query<Entity, Or<(With<WindowRoot>, With<FloatingBoard>, With<SettlingBoard>, With<FlungBoard>, With<Hammering>)>>,
    mut blockers: ResMut<WindowBlockers>,
    mut commands: Commands,
) {
    for e in &roots {
        commands.entity(e).despawn();
    }
    if !blockers.0.is_empty() {
        blockers.0.clear();
    }
}

/// Show each window's boards as `Lobby::windows_torn` has them — flinging
/// the ones just torn off, floating the ones just put back up into place.
#[allow(clippy::too_many_arguments)]
fn sync_boards(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<GameSounds>>,
    look: Option<Res<BoardLook>>,
    mut roots: Query<(&mut WindowRoot, &Children)>,
    mut slots: Query<(&BoardSlot, &GlobalTransform, &mut Visibility)>,
    floating: Query<&FloatingBoard>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let (Some(lobby), Some(look)) = (zombies_game(&local, &lobbies), look) else { return };
    let windows = shared::windows::windows(lobby.map);
    for (mut root, children) in &mut roots {
        let Some(window) = windows.get(root.index) else { continue };
        let left = boards_left(&lobby.windows_torn, root.index);
        let before = root.seen.unwrap_or(left);
        root.seen = Some(left);
        let (opening, ..) = window.opening();
        let sound = |clip: Option<Handle<AudioSource>>, commands: &mut Commands| {
            if let Some(clip) = clip {
                commands.spawn((
                    StateScoped(AppState::InGame),
                    AudioPlayer::new(clip),
                    Transform::from_translation(opening),
                    crate::positional_playback(Volume::Linear(1.0)),
                ));
            }
        };
        let pick = |set: &[Handle<AudioSource>], s: u32| {
            (!set.is_empty()).then(|| set[(rand01(s) * set.len() as f32) as usize % set.len()].clone())
        };
        for &child in children {
            let Ok((slot, global, mut vis)) = slots.get_mut(child) else { continue };
            *seq = seq.wrapping_add(1);
            let s = seq.wrapping_mul(2_654_435_761);
            // Just torn off: flung out of the window, away from the room.
            if slot.0 < before && slot.0 >= left {
                let out = window.rotation() * Vec3::NEG_Z;
                let side = window.rotation() * Vec3::X * (rand01(s ^ 0x1) - 0.5) * 2.0;
                commands
                    .spawn((
                        StateScoped(AppState::InGame),
                        FlungBoard {
                            velocity: out * (2.5 + 1.5 * rand01(s ^ 0x2)) + side + Vec3::Y * (1.5 + rand01(s ^ 0x3)),
                            spin: Vec3::new(rand01(s ^ 0x4) - 0.5, rand01(s ^ 0x5) - 0.5, rand01(s ^ 0x6) - 0.5) * 14.0,
                            age: 0.0,
                        },
                        global.compute_transform(),
                        Visibility::default(),
                        NotShadowCaster,
                    ))
                    .with_children(|b| board_visual(b, &look, window.board_size()));
                sound(sounds.as_deref().and_then(|snd| pick(&snd.board_snaps, s ^ 0x7)), &mut commands);
            }
            // Just put back: up from below outside, standing on end, to hang
            // tilted a little out from its place before it's slammed home.
            if slot.0 >= before && slot.0 < left {
                let place = global.compute_transform();
                let out = window.rotation() * Vec3::NEG_Z;
                let side = window.rotation() * Vec3::X;
                let r = |k: u32| rand01(s ^ k) - 0.5;
                let start = Transform::from_translation(place.translation + out * 0.9 - Vec3::Y * 1.3 + side * r(0x8) * 0.4)
                    .with_rotation(
                        place.rotation
                            * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2 * if r(0x9) < 0.0 { -1.0 } else { 1.0 } + r(0xa) * 0.5)
                            * Quat::from_rotation_x(r(0xb) * 0.8),
                    );
                let tilt = (8.0 + 8.0 * (r(0xc) + 0.5)).to_radians() * if r(0xd) < 0.0 { -1.0 } else { 1.0 };
                let hover = Transform::from_translation(
                    place.translation + out * 0.55 + side * r(0xe) * 0.15 + Vec3::Y * r(0xf) * 0.1,
                )
                .with_rotation(place.rotation * Quat::from_rotation_z(tilt) * Quat::from_rotation_y(r(0x10) * 0.25));
                let drift = Transform::from_translation(hover.translation - out * 0.1)
                    .with_rotation(place.rotation * Quat::from_rotation_z(tilt * 0.6) * Quat::from_rotation_y(r(0x10) * 0.15));
                commands
                    .spawn((
                        StateScoped(AppState::InGame),
                        FloatingBoard {
                            window: root.index,
                            slot: slot.0,
                            start,
                            hover,
                            drift,
                            to: place,
                            age: 0.0,
                            dust: 0.0,
                            smoke: 0.0,
                            last: start.translation,
                        },
                        start,
                        Visibility::default(),
                    ))
                    .with_children(|b| board_visual(b, &look, window.board_size()));
                sound(sounds.as_deref().map(|snd| snd.board_float.clone()), &mut commands);
            }
            let arriving = floating.iter().any(|f| f.window == root.index && f.slot == slot.0);
            vis.set_if_neq(if slot.0 < left && !arriving { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
}

/// Fly each board being put back in — up, hang, slam ([`RISE_SECS`]) —
/// shedding dust and smoke, and slam it home: the sound and a puff.
#[allow(clippy::too_many_arguments)]
fn float_boards(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<GameSounds>>,
    dust: Option<Res<ImpactAssets>>,
    smoke: (Option<Res<SmokeAssets>>, Res<BoardSmokeSettings>),
    particles: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut boards: Query<(Entity, &mut FloatingBoard, &mut Transform)>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let map = zombies_game(&local, &lobbies).map(|l| l.map);
    let dt = time.delta_secs();
    let mut budget = IMPACT_MAX.saturating_sub(particles.iter().count());
    let (smoke_assets, smoke_cfg) = smoke;
    let smoke = smoke_cfg.enabled.then_some(()).and(dust.as_deref().zip(smoke_assets.as_deref()));
    for (e, mut board, mut t) in &mut boards {
        board.age += dt;
        let age = board.age;
        let (from, to, k) = if age < RISE_SECS {
            // Up and spinning over, slowing as it gets there.
            let k = age / RISE_SECS;
            (board.start, board.hover, 1.0 - (1.0 - k) * (1.0 - k))
        } else if age < RISE_SECS + HOVER_SECS {
            (board.hover, board.drift, ease((age - RISE_SECS) / HOVER_SECS))
        } else {
            // Slammed: quicker and quicker into the wall.
            let k = ((age - RISE_SECS - HOVER_SECS) / SLAM_SECS).clamp(0.0, 1.0);
            (board.drift, board.to, k * k)
        };
        t.translation = from.translation.lerp(to.translation, k);
        t.rotation = from.rotation.slerp(to.rotation, k);
        let size = map
            .and_then(|m| shared::windows::windows(m).get(board.window))
            .map_or(Vec3::new(1.4, 0.1, 0.03), Window::board_size);
        let velocity = if dt > 0.0 { (t.translation - board.last) / dt } else { Vec3::ZERO };
        board.last = t.translation;
        if age < FLOAT_SECS {
            if let Some((quad, smoke_tex)) = smoke {
                board.smoke += smoke_cfg.rate * dt;
                while board.smoke >= 1.0 {
                    board.smoke -= 1.0;
                    *seq = seq.wrapping_add(1);
                    let s = seq.wrapping_mul(2_654_435_761);
                    let at = along_board(&t, size, s);
                    spawn_board_smoke(&mut commands, &mut materials, quad, smoke_tex, &smoke_cfg, at, velocity * smoke_cfg.trail, 1.0, s, &mut budget);
                }
            }
            board.dust += BOARD_DUST_RATE * dt;
            while board.dust >= 1.0 {
                board.dust -= 1.0;
                *seq = seq.wrapping_add(1);
                let s = seq.wrapping_mul(2_654_435_761);
                let along = t.rotation * Vec3::X * (rand01(s) - 0.5) * size.x;
                if let Some(assets) = dust.as_deref() {
                    spawn_board_dust(&mut commands, &mut materials, assets, t.translation + along, Vec3::Y * 0.15, 0.5, s, &mut budget);
                }
            }
            continue;
        }
        commands.entity(e).despawn();
        *seq = seq.wrapping_add(1);
        let s = seq.wrapping_mul(2_654_435_761);
        // The puff it knocks up, along it, out from the wall.
        if let (Some(assets), Some(window)) = (dust.as_deref(), map.and_then(|m| shared::windows::windows(m).get(board.window))) {
            let out = window.rotation() * Vec3::NEG_Z;
            for i in 0..SLAM_DUST {
                let s = s ^ (i + 1).wrapping_mul(0x9e37);
                let along = board.to.rotation * Vec3::X * (rand01(s) - 0.5) * size.x;
                let drift = out * (0.3 + 0.4 * rand01(s ^ 0x1)) + Vec3::Y * 0.2 * rand01(s ^ 0x2);
                spawn_board_dust(&mut commands, &mut materials, assets, board.to.translation + along, drift, 1.0, s, &mut budget);
            }
            if let Some((quad, smoke_tex)) = smoke {
                for i in 0..smoke_cfg.slam_puffs {
                    let s = s ^ (i + 1).wrapping_mul(0x7f4a_7c15);
                    let at = along_board(&board.to, size, s);
                    let drift = out * (0.2 + 0.3 * rand01(s ^ 0x31));
                    spawn_board_smoke(&mut commands, &mut materials, quad, smoke_tex, &smoke_cfg, at, drift, 1.3, s, &mut budget);
                }
            }
            if smoke.is_some() && smoke_cfg.linger_secs > 0.0 {
                commands.spawn((
                    StateScoped(AppState::InGame),
                    SettlingBoard {
                        at: board.to,
                        size,
                        age: 0.0,
                        smoke: 0.0,
                    },
                ));
            }
        }
        let opening = map
            .and_then(|m| shared::windows::windows(m).get(board.window))
            .map_or(board.to.translation, |w| w.opening().0);
        if let Some(clip) = sounds.as_deref().and_then(|snd| {
            (!snd.board_slams.is_empty())
                .then(|| snd.board_slams[(rand01(s) * snd.board_slams.len() as f32) as usize % snd.board_slams.len()].clone())
        }) {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(clip),
                Transform::from_translation(opening),
                crate::positional_playback(Volume::Linear(1.0)),
            ));
        }
    }
}

/// Keep a board that's just slammed home smoking for a moment, thinning out
/// ([`BoardSmokeSettings::linger_secs`]), then it's done.
#[allow(clippy::too_many_arguments)]
fn settle_boards(
    time: Res<Time>,
    cfg: Res<BoardSmokeSettings>,
    assets: (Option<Res<ImpactAssets>>, Option<Res<SmokeAssets>>),
    particles: Query<(), With<ImpactParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut boards: Query<(Entity, &mut SettlingBoard)>,
    mut seq: Local<u32>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let mut budget = IMPACT_MAX.saturating_sub(particles.iter().count());
    for (e, mut board) in &mut boards {
        board.age += dt;
        let (Some(quad), Some(tex)) = (assets.0.as_deref(), assets.1.as_deref()) else { continue };
        if !cfg.enabled || board.age >= cfg.linger_secs {
            commands.entity(e).despawn();
            continue;
        }
        let left = 1.0 - board.age / cfg.linger_secs.max(1.0e-4);
        board.smoke += cfg.linger_rate * left * dt;
        while board.smoke >= 1.0 {
            board.smoke -= 1.0;
            *seq = seq.wrapping_add(1);
            let s = seq.wrapping_mul(2_654_435_761);
            let at = along_board(&board.at, board.size, s);
            spawn_board_smoke(&mut commands, &mut materials, quad, tex, &cfg, at, Vec3::ZERO, left.max(0.4), s, &mut budget);
        }
    }
}

/// A random point on a board at `t` of `size` (m) — anywhere along it,
/// across its face.
fn along_board(t: &Transform, size: Vec3, s: u32) -> Vec3 {
    t.translation
        + t.rotation * Vec3::X * (rand01(s ^ 0x41) - 0.5) * size.x
        + t.rotation * Vec3::Y * (rand01(s ^ 0x42) - 0.5) * size.y
}

/// A wisp of smoke off a board at `at`, leaving at `drift` and settling into
/// a slow rise as it spreads out ([`BoardSmokeSettings`]); `strength` scales
/// how big and thick it is.
#[allow(clippy::too_many_arguments)]
fn spawn_board_smoke(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    quad: &ImpactAssets,
    smoke: &SmokeAssets,
    cfg: &BoardSmokeSettings,
    at: Vec3,
    drift: Vec3,
    strength: f32,
    s: u32,
    budget: &mut usize,
) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    let r = |k: u32| rand01(s ^ k) * 2.0 - 1.0;
    let jitter = Vec3::new(r(0x51), r(0x52), r(0x53)) * cfg.spread;
    let size = 0.75 + 0.5 * rand01(s ^ 0x54);
    commands.spawn((
        StateScoped(AppState::InGame),
        ImpactParticle {
            velocity: drift + jitter + Vec3::Y * cfg.rise,
            // (Negative: it settles into rising at `rise` against the drag.)
            gravity: -cfg.rise * cfg.drag,
            drag: cfg.drag,
            age: 0.0,
            lifetime: (cfg.lifetime * (0.7 + 0.6 * rand01(s ^ 0x55))).max(0.05),
            fade_in: cfg.fade_in,
            roll: rand_roll(s ^ 0x56),
            spin: r(0x57) * 0.5,
            scale0: cfg.start_size * size * strength,
            scale1: cfg.end_size * size * strength,
            peak_alpha: (cfg.opacity * strength).min(1.0),
            tint: cfg.color,
            floor: f32::NEG_INFINITY,
            hold: 0.0,
        },
        Mesh3d(quad.quad.clone()),
        MeshMaterial3d(materials.add(impact_material(smoke.texture.clone()))),
        Transform::from_translation(at).with_scale(Vec3::splat(cfg.start_size * size * strength)),
        NoFrustumCulling,
        NotShadowCaster,
    ));
}

/// The debug panel's "Window boards (Zombies)" section.
#[derive(SystemParam)]
pub(crate) struct BoardSmokeDebug<'w> {
    settings: ResMut<'w, BoardSmokeSettings>,
}

impl BoardSmokeDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        let s = &mut *self.settings;
        ui.label("Smoke off a board as it floats up into a window and slams home.");
        ui.checkbox(&mut s.enabled, "smoke");
        ui.add(egui::Slider::new(&mut s.rate, 0.0f32..=150.0).text("wisps / s floating"));
        ui.add(egui::Slider::new(&mut s.slam_puffs, 0..=40).text("wisps on slam"));
        ui.add(egui::Slider::new(&mut s.linger_secs, 0.0f32..=4.0).text("keeps smoking after (s)"));
        ui.add(egui::Slider::new(&mut s.linger_rate, 0.0f32..=80.0).text("wisps / s after"));
        ui.separator();
        ui.add(egui::Slider::new(&mut s.lifetime, 0.1f32..=5.0).text("wisp hangs (s)"));
        ui.add(egui::Slider::new(&mut s.fade_in, 0.0f32..=1.0).text("wisp fade in (s)"));
        ui.add(egui::Slider::new(&mut s.start_size, 0.01f32..=1.0).text("size leaving board (m)"));
        ui.add(egui::Slider::new(&mut s.end_size, 0.05f32..=3.0).text("size spread out (m)"));
        ui.add(egui::Slider::new(&mut s.opacity, 0.0f32..=1.0).text("opacity"));
        ui.add(egui::Slider::new(&mut s.rise, 0.0f32..=2.0).text("rise (m/s)"));
        ui.add(egui::Slider::new(&mut s.spread, 0.0f32..=1.0).text("spread (m/s)"));
        ui.add(egui::Slider::new(&mut s.trail, 0.0f32..=1.0).text("follows board"));
        ui.add(egui::Slider::new(&mut s.drag, 0.0f32..=8.0).text("drag"));
        ui.horizontal(|ui| {
            ui.color_edit_button_rgb(&mut s.color);
            ui.label("colour");
        });
        if ui.button("Copy settings to console").clicked() {
            info!(
                "board smoke: enabled: {}, rate: {:.2}, slam_puffs: {}, linger_secs: {:.2}, linger_rate: {:.2}, \
                 lifetime: {:.2}, fade_in: {:.2}, start_size: {:.3}, end_size: {:.3}, opacity: {:.3}, rise: {:.3}, \
                 spread: {:.3}, trail: {:.2}, drag: {:.2}, color: [{:.2}, {:.2}, {:.2}]",
                s.enabled,
                s.rate,
                s.slam_puffs,
                s.linger_secs,
                s.linger_rate,
                s.lifetime,
                s.fade_in,
                s.start_size,
                s.end_size,
                s.opacity,
                s.rise,
                s.spread,
                s.trail,
                s.drag,
                s.color[0],
                s.color[1],
                s.color[2],
            );
        }
        if ui.button("Reset").clicked() {
            *s = BoardSmokeSettings::default();
        }
    }
}

/// A faint puff of dust off a board at `at`, drifting at `drift` and
/// spreading as it hangs; `strength` scales how big and thick it is.
#[allow(clippy::too_many_arguments)]
fn spawn_board_dust(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    assets: &ImpactAssets,
    at: Vec3,
    drift: Vec3,
    strength: f32,
    s: u32,
    budget: &mut usize,
) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    let size = (0.25 + 0.2 * rand01(s ^ 0x21)) * strength.max(0.3);
    commands.spawn((
        StateScoped(AppState::InGame),
        ImpactParticle {
            velocity: drift,
            gravity: 0.1,
            drag: 1.5,
            age: 0.0,
            lifetime: 0.5 + 0.4 * rand01(s ^ 0x22),
            fade_in: 0.05,
            roll: rand_roll(s ^ 0x23),
            spin: (rand01(s ^ 0x24) * 2.0 - 1.0) * 0.6,
            scale0: size * 0.4,
            scale1: size,
            peak_alpha: 0.16 * strength.min(1.0),
            tint: DUST_TINT,
            floor: at.y - 5.0,
            hold: 0.0,
        },
        Mesh3d(assets.quad.clone()),
        MeshMaterial3d(materials.add(impact_material(assets.dust.clone()))),
        Transform::from_translation(at).with_scale(Vec3::splat(size * 0.4)),
        NoFrustumCulling,
        NotShadowCaster,
    ));
}

/// Fly each torn-off board away, tumbling, and then it's gone.
fn fling_boards(time: Res<Time>, mut boards: Query<(Entity, &mut FlungBoard, &mut Transform)>, mut commands: Commands) {
    let dt = time.delta_secs();
    for (e, mut board, mut t) in &mut boards {
        board.age += dt;
        if board.age >= FLING_SECS {
            commands.entity(e).despawn();
            continue;
        }
        board.velocity.y -= 9.8 * dt;
        t.translation += board.velocity * dt;
        let spin = board.spin * dt;
        t.rotation = Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z) * t.rotation;
    }
}

/// Interact held inside and close to a window missing boards: ask the
/// server to put one back every [`REPAIR_SECS`] (it checks again, and
/// takes one off `Lobby::windows_torn`).
#[allow(clippy::too_many_arguments)]
fn repair_barrier(
    time: Res<Time>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut repairing: ResMut<Repairing>,
    mut sender: Query<&mut TriggerSender<shared::RepairBarrier>, With<GameClient>>,
) {
    let now = time.elapsed_secs();
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let at = zombies_game(&local, &lobbies)
        .and_then(|lobby| repairable_at(lobby.map, &lobby.windows_torn, feet, 0.0));
    let held = binds.interact.pressed(&keys, &mouse);
    let Some(index) = at.filter(|_| held) else {
        if repairing.window.is_some() {
            *repairing = Repairing::default();
        }
        return;
    };
    if repairing.window != Some(index) {
        // (The first board goes back a moment in, not the instant it's
        // pressed.)
        *repairing = Repairing {
            window: Some(index),
            next_at: now + REPAIR_SECS * 0.5,
        };
    }
    if now < repairing.next_at {
        return;
    }
    repairing.next_at = now + REPAIR_SECS;
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::RepairBarrier(index as u8));
    }
}

/// Not rebuilding anything while the game's not ours to play (a menu, a
/// kill cam, down).
fn stop_repairing(mut repairing: ResMut<Repairing>) {
    if repairing.window.is_some() {
        *repairing = Repairing::default();
    }
}

/// The hammering, looped, for as long as we're rebuilding.
#[derive(Component)]
struct Hammering;

fn hammer(
    repairing: Res<Repairing>,
    sounds: Option<Res<GameSounds>>,
    playing: Query<Entity, With<Hammering>>,
    mut commands: Commands,
) {
    match (repairing.window.is_some(), playing.is_empty(), sounds) {
        (true, true, Some(sounds)) => {
            commands.spawn((
                StateScoped(AppState::InGame),
                Hammering,
                AudioPlayer::new(sounds.hammering.clone()),
                PlaybackSettings::LOOP,
            ));
        }
        (false, false, _) => {
            for e in &playing {
                commands.entity(e).despawn();
            }
        }
        _ => {}
    }
}

/// The server paid out a board we put back: a `+10` and a ching (none once
/// we're at the round's cap).
fn receive_barrier_repaired(
    time: Res<Time>,
    mut receivers: Query<&mut lightyear::prelude::MessageReceiver<shared::BarrierRepaired>>,
    sounds: Res<GameSounds>,
    mut scored: EventWriter<crate::TrickScoredEvent>,
    // (Shown as the board slams home, not the moment it's paid.)
    mut due: Local<Vec<(f32, u32)>>,
) {
    let now = time.elapsed_secs();
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if msg.points > 0 {
                due.push((now + FLOAT_SECS, msg.points));
            }
        }
    }
    due.retain(|&(at, points)| {
        if now < at {
            return true;
        }
        scored.write(crate::TrickScoredEvent {
            total: points,
            lines: Vec::new(),
            sound: Some(sounds.money_ching.clone()),
            color: crate::hud::SCORE_WHITE,
        });
        false
    });
}

// --- the card (at a window missing boards) ----------------------------------

#[derive(Component)]
struct BarrierCard;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BarrierCardText {
    Boards,
    Action,
}

fn spawn_barrier_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let heading = |size: f32| TextFont {
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
                top: Val::Percent(57.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                BarrierCard,
                Node {
                    width: Val::Px(PERK_CARD_WIDTH * 0.7),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(14.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(BOARD_TAN),
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((Text::new("BARRIER"), heading(30.0), TextColor(BOARD_TAN)));
                card.spawn((BarrierCardText::Boards, Text::new(""), heading(18.0), TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7))));
                card.spawn((BarrierCardText::Action, Text::new(""), heading(22.0), TextColor::WHITE));
            });
        });
}

/// Show the card while we're inside and close to a window missing boards
/// (hidden behind menus and during a kill cam, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_barrier_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    binds: Res<KeyBindings>,
    repairing: Res<Repairing>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut card: Single<&mut Visibility, With<BarrierCard>>,
    mut texts: Query<(&BarrierCardText, &mut Text)>,
) {
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    let shown = (!menu.is_open() && active_killcam.0.is_none())
        .then(|| {
            let lobby = zombies_game(&local, &lobbies)?;
            let index = repairable_at(lobby.map, &lobby.windows_torn, feet, 0.0)?;
            Some(boards_left(&lobby.windows_torn, index))
        })
        .flatten();
    let Some(left) = shown else {
        card.set_if_neq(Visibility::Hidden);
        return;
    };
    card.set_if_neq(Visibility::Inherited);
    let action = if repairing.window.is_some() {
        "REBUILDING...".to_string()
    } else {
        format!("HOLD {} TO REBUILD", binds.interact.label().to_uppercase())
    };
    for (which, mut text) in &mut texts {
        let wanted = match which {
            BarrierCardText::Boards => format!("{left} / {BOARDS} BOARDS"),
            BarrierCardText::Action => action.clone(),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}
