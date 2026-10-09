//! The minimap, top left, Call of Duty's: the map from above, centred on us
//! and turned with us so straight ahead is always up, our arrow in the
//! middle with our view cone fanning out ahead of it. In `Zombies` it marks
//! where everything is — the perk machines (their icons), Der Wunderfizz,
//! the Pack-a-Punch, the Mystery Box, the power switch, the wall buys, the
//! ammo crate, the armor station, the crafting table, the exfil radio while
//! exfil can be called — and the rest of the party, as arrows facing where
//! they look. In `FreeForAll` an enemy who fires shows as a red dot where
//! they fired from, fading quickly — one dot per enemy, moved and refreshed
//! by each shot, so a sprayed AK is still one dot (Call of Duty's). In
//! `Freestyle` every standing target bot is a red dot, always.
//!
//! Holding the full map key (M by default) shows the whole map in the middle
//! of the screen instead — north up, not turning, with us and the party as
//! arrows and every `Zombies` icon, but no enemies' dots.
//!
//! The picture isn't made by hand: it's drawn from the map's own collision
//! model the moment it loads ([`bake`]), so every map has one, and moving
//! something in the model moves it here. It's shaded against the height
//! we're standing at, so the level we're on reads as the floor. Where the `Zombies` things stand is
//! the map's layout's (`shared::level`) — placed in the level editor.
//!
//! It centres on whoever the world camera follows — us, or the teammate
//! we're spectating — like the compass. It's part of the HUD
//! ([`menu::HudElement`]: hidden outside a game, behind menus and during
//! kill cams). The UI is `StateScoped(InGame)` and rebuilt each game; the
//! picture's kept per map (it's only the map's) and redrawn when the map
//! changes.

mod bake;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::mesh::{PrimitiveTopology, VertexAttributeValues};
use bevy::render::render_resource::{AsBindGroup, Extent3d, ShaderRef, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use lightyear::prelude::{Interpolated, LocalId, PeerId};
use shared::{Bot, GameMode, Lobby, PlayerId, PlayerPose, ZombieAnim};

use crate::keybinds::KeyBindings;
use crate::killcam::ActiveKillCam;
use crate::net::{BotPose, GameClient, RemoteAvatar};
use crate::zombies_hud::{my_lobby, zombies_game};
use crate::{menu, AppState, CurrentMap, MapLoadState, MapModel, Player, WorldModelCamera, EYE_HEIGHT, HUD_FONT};

const SHADER: &str = "shaders/minimap.wgsl";

/// How big (logical px) the minimap is — a square — and how far in from
/// the screen's top-left corner it sits.
/// (Under the FPS / ping readout.)
pub(crate) const MINIMAP_SIZE: f32 = 210.0;
pub(crate) const MINIMAP_TOP: f32 = crate::hud::FPS_ROW_TOP + crate::hud::FPS_ROW_HEIGHT + 4.0;
pub(crate) const MINIMAP_LEFT: f32 = 20.0;
/// How far (m) it shows from its centre to an edge.
const VIEW_RADIUS_M: f32 = 28.0;
/// An icon's size (px), and a teammate's arrow's.
const ICON_SIZE: f32 = 20.0;
const TEAMMATE_SIZE: f32 = 18.0;
const TEAMMATE_BLUE: Color = Color::srgb(0.3, 0.75, 1.0);
/// An enemy's red dot: its size (px) and colour; a shot's stays solid this
/// long (s), then fades out over this long.
const ENEMY_DOT_SIZE: f32 = 9.0;
const ENEMY_RED: Color = Color::srgb(0.95, 0.12, 0.1);
const SHOT_DOT_HOLD_SECS: f32 = 0.5;
const SHOT_DOT_FADE_SECS: f32 = 1.0;

pub(crate) struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<MinimapMaterial>::default())
            .init_resource::<MinimapPicture>()
            .init_resource::<ShotDots>()
            .add_event::<SomeoneFired>()
            .add_systems(OnExit(AppState::InGame), |mut dots: ResMut<ShotDots>| dots.0.clear())
            .add_systems(Startup, make_teammate_arrow)
            // Unconditional, like the map model itself: the map loads behind
            // the menus, so its picture can be ready by the time a game starts.
            .add_systems(Update, bake_current_map.after(crate::sync_map_model))
            .add_systems(OnEnter(AppState::InGame), spawn_minimap)
            .add_systems(
                Update,
                (toggle_full_map, sync_icons, sync_teammates, sync_enemy_dots, update_minimap)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The current map's picture ([`bake`]), once it's drawn.
#[derive(Resource, Default)]
struct MinimapPicture {
    /// The map it's been drawn for (`None` while the current one's still
    /// loading)...
    drawn_for: Option<shared::MapId>,
    /// ...and the picture — `None` if the model had nothing to draw.
    picture: Option<Picture>,
}

struct Picture {
    image: Handle<Image>,
    /// The world rect it covers: top-left `(x, z)`, then size (m).
    rect: Vec4,
    /// Its lowest height and its range ([`bake::BakedMap::heights`]).
    heights: Vec2,
}

/// Someone else's shot (`net::receive_shots`): who, and where from.
#[derive(Event)]
pub(crate) struct SomeoneFired {
    pub(crate) shooter: PeerId,
    pub(crate) from: Vec3,
}

/// `FreeForAll`: each enemy's last shot — where from, and when
/// (`Time::elapsed_secs`). Gone once faded; cleared on leaving the game.
#[derive(Resource, Default)]
struct ShotDots(Vec<(PeerId, Vec2, f32)>);

/// The teammates' arrow.
#[derive(Resource)]
struct TeammateArrow(Handle<Image>);

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
struct MinimapMaterial {
    /// x, y: the world `(x, z)` it's centred on; z: the heading (radians,
    /// clockwise from north); w: [`VIEW_RADIUS_M`].
    #[uniform(0)]
    view: Vec4,
    /// [`Picture::rect`].
    #[uniform(1)]
    rect: Vec4,
    #[texture(2)]
    #[sampler(3)]
    map: Handle<Image>,
    /// x, y: [`Picture::heights`]; z: the height our feet are at — what
    /// the picture's shaded against.
    #[uniform(4)]
    heights: Vec4,
    /// x: 1 to draw our arrow and view cone in the middle (the minimap; the
    /// full map has its own arrow, wherever we are).
    #[uniform(5)]
    mode: Vec4,
}

impl UiMaterial for MinimapMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

/// A map view — the minimap, or the full map (`full`) — its icons and
/// arrows inside it; and the icons it has up now (rebuilt whenever the
/// wanted set changes).
#[derive(Component, Default)]
struct MinimapFrame {
    full: bool,
    icons: Vec<IconSpec>,
}

/// One of the `Zombies` icons, standing at world `(x, z)`, `size` px across.
#[derive(Component)]
struct MinimapIcon {
    at: Vec2,
    size: f32,
}

/// A teammate's arrow: the entity whose `PlayerPose` it follows.
#[derive(Component)]
struct MinimapTeammate(Entity);

/// Whose an enemy's red dot is.
#[derive(Clone, Copy, PartialEq)]
enum EnemyKey {
    /// A `FreeForAll` enemy's last shot.
    Shot(PeerId),
    /// A `Freestyle` target bot.
    Bot(Entity),
}

/// An enemy's red dot: whose, where (world `(x, z)`) and how strong.
#[derive(Component)]
struct MinimapEnemy {
    key: EnemyKey,
    at: Vec2,
    alpha: f32,
}

// --- the picture ------------------------------------------------------------

/// Draw the current map's picture once its model's loaded (again whenever
/// the map changes).
#[allow(clippy::too_many_arguments)]
fn bake_current_map(
    current: Res<CurrentMap>,
    load: Res<MapLoadState>,
    models: Query<Entity, With<MapModel>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mesh_nodes: Query<(&Mesh3d, &GlobalTransform)>,
    meshes: Res<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut picture: ResMut<MinimapPicture>,
) {
    if current.is_changed() && picture.drawn_for != Some(current.0) {
        *picture = MinimapPicture::default();
    }
    if picture.drawn_for == Some(current.0) || !load.model_ready() {
        return;
    }
    let Ok(root) = models.single() else { return };
    // Left out: the stand-in person some models carry for scale.
    let skipped: Vec<Entity> = children
        .iter_descendants(root)
        .filter(|&e| names.get(e).is_ok_and(|n| n.as_str().starts_with("player_ref")))
        .flat_map(|e| std::iter::once(e).chain(children.iter_descendants(e)))
        .collect();
    let mut triangles = Vec::new();
    for e in children.iter_descendants(root) {
        let Ok((mesh, to_world)) = mesh_nodes.get(e) else { continue };
        if skipped.contains(&e) {
            continue;
        }
        // (Not loaded yet: try again next frame.)
        let Some(mesh) = meshes.get(&mesh.0) else { return };
        append_triangles(mesh, to_world.affine(), &mut triangles);
    }
    picture.drawn_for = Some(current.0);
    picture.picture = bake::bake(&triangles).map(|baked| Picture {
        image: images.add(baked.image),
        rect: Vec4::new(baked.min.x, baked.min.y, baked.size.x, baked.size.y),
        heights: baked.heights,
    });
}

/// `mesh`'s triangles, placed in the world by `to_world`.
fn append_triangles(mesh: &Mesh, to_world: Affine3A, out: &mut Vec<[Vec3; 3]>) {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return;
    }
    let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return;
    };
    let world: Vec<Vec3> = positions.iter().map(|&p| to_world.transform_point3(Vec3::from(p))).collect();
    let corner = |i: usize| world.get(i).copied();
    match mesh.indices() {
        Some(indices) => {
            let indices: Vec<usize> = indices.iter().collect();
            for tri in indices.chunks_exact(3) {
                if let (Some(a), Some(b), Some(c)) = (corner(tri[0]), corner(tri[1]), corner(tri[2])) {
                    out.push([a, b, c]);
                }
            }
        }
        None => out.extend(world.chunks_exact(3).map(|t| [t[0], t[1], t[2]])),
    }
}

/// A white arrow pointing up with a dark rim — tinted per use.
fn make_teammate_arrow(mut images: ResMut<Assets<Image>>, mut commands: Commands) {
    const N: usize = 48;
    let (tip, left, notch, right) = (
        Vec2::new(0.0, -0.9),
        Vec2::new(-0.7, 0.75),
        Vec2::new(0.0, 0.35),
        Vec2::new(0.7, 0.75),
    );
    let px = 2.0 / N as f32;
    let mut data = Vec::with_capacity(N * N * 4);
    for j in 0..N {
        for i in 0..N {
            let p = (Vec2::new(i as f32, j as f32) + 0.5) * px - Vec2::ONE;
            let d = sd_triangle(p, tip, left, notch).min(sd_triangle(p, tip, notch, right));
            let fill = (0.5 - d / px).clamp(0.0, 1.0);
            let rim = (0.5 - (d - 2.5 * px) / px).clamp(0.0, 1.0);
            let shade = (255.0 * fill) as u8;
            data.extend_from_slice(&[shade, shade, shade, (255.0 * rim) as u8]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    commands.insert_resource(TeammateArrow(images.add(image)));
}

/// Signed distance from `p` to the triangle `a b c` (negative inside).
fn sd_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let (e0, e1, e2) = (b - a, c - b, a - c);
    let (v0, v1, v2) = (p - a, p - b, p - c);
    let q = |v: Vec2, e: Vec2| v - e * (v.dot(e) / e.dot(e)).clamp(0.0, 1.0);
    let s = (e0.x * e2.y - e0.y * e2.x).signum();
    let (q0, q1, q2) = (q(v0, e0), q(v1, e1), q(v2, e2));
    let d = Vec2::new(q0.dot(q0), s * (v0.x * e0.y - v0.y * e0.x))
        .min(Vec2::new(q1.dot(q1), s * (v1.x * e1.y - v1.y * e1.x)))
        .min(Vec2::new(q2.dot(q2), s * (v2.x * e2.y - v2.y * e2.x)));
    -d.x.sqrt() * d.y.signum()
}

// --- the UI -------------------------------------------------------------------

/// The full map's frame: as big as fits in this share of the screen.
const FULL_MAP_SCREEN_SHARE: f32 = 0.8;
/// On the full map, icons and arrows are a little bigger.
const FULL_ICON_SIZE: f32 = 26.0;
const FULL_ARROW_SIZE: f32 = 22.0;
/// Our own arrow on the full map (the minimap draws it in its shader).
const OUR_YELLOW: Color = Color::srgb(1.0, 0.86, 0.25);

/// The full map's dimmed backdrop, shown while its key's held.
#[derive(Component)]
struct FullMapPanel;

/// Our own arrow on the full map.
#[derive(Component)]
struct FullMapPlayer;

fn minimap_material(materials: &mut Assets<MinimapMaterial>, full: bool) -> MaterialNode<MinimapMaterial> {
    MaterialNode(materials.add(MinimapMaterial {
        view: Vec4::new(0.0, 0.0, 0.0, VIEW_RADIUS_M),
        rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
        map: Handle::default(),
        heights: Vec4::ZERO,
        mode: Vec4::new(if full { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0),
    }))
}

/// The minimap (top left), and the full map (centre screen, while its key's
/// held) — both hidden until the map's picture is drawn.
fn spawn_minimap(mut commands: Commands, mut materials: ResMut<Assets<MinimapMaterial>>, arrow: Res<TeammateArrow>) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            menu::HudElement,
            GlobalZIndex(4),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(MINIMAP_LEFT),
                top: Val::Px(MINIMAP_TOP),
                width: Val::Px(MINIMAP_SIZE),
                height: Val::Px(MINIMAP_SIZE),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_child((
            MinimapFrame::default(),
            minimap_material(&mut materials, false),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                overflow: Overflow::clip(),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
        ));

    // (Over the rest of the HUD, but under the menus.)
    commands
        .spawn((
            StateScoped(AppState::InGame),
            menu::HudElement,
            GlobalZIndex(7),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|root| {
            root.spawn((
                FullMapPanel,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                Visibility::Hidden,
                Pickable::IGNORE,
            ))
            .with_children(|panel| {
                panel
                    .spawn((
                        MinimapFrame {
                            full: true,
                            ..default()
                        },
                        minimap_material(&mut materials, true),
                        Node {
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        Visibility::Hidden,
                        Pickable::IGNORE,
                    ))
                    .with_child((
                        FullMapPlayer,
                        ImageNode::new(arrow.0.clone()).with_color(OUR_YELLOW),
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Px(FULL_ARROW_SIZE),
                            height: Val::Px(FULL_ARROW_SIZE),
                            ..default()
                        },
                        // (Over the icons.)
                        ZIndex(1),
                        Visibility::Hidden,
                        Pickable::IGNORE,
                    ));
            });
        });
}

/// Show the full map while its key's held (not behind a menu or during a
/// kill cam — the HUD's hidden then anyway).
fn toggle_full_map(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    menu: Res<menu::Menu>,
    killcam: Res<ActiveKillCam>,
    mut panel: Query<&mut Visibility, With<FullMapPanel>>,
) {
    let held = binds.full_map.pressed(&keys, &mouse) && !menu.is_open() && killcam.0.is_none();
    for mut vis in &mut panel {
        vis.set_if_neq(if held { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// What an icon looks like.
#[derive(Clone, Copy, PartialEq, Debug)]
enum IconArt {
    /// Just this image (it's round already).
    Image(&'static str),
    /// A dark disc ringed in `color`, with an image in it...
    BadgeImage(&'static str, Color),
    /// ...or a short label.
    BadgeText(&'static str, Color),
}

#[derive(Clone, PartialEq, Debug)]
struct IconSpec {
    at: Vec2,
    art: IconArt,
}

/// Every icon our lobby's `Zombies` game should show (none outside one).
fn wanted_icons(lobby: &Lobby) -> Vec<IconSpec> {
    let layout = shared::level::layout(lobby.map);
    let mut icons = Vec::new();
    let mut add = |at: shared::level::Placement, art: IconArt| icons.push(IconSpec { at: at.pos.xz(), art });
    // (Badges first, so the perks' own icons sit over them.)
    if let Some(at) = layout.ammo_crate {
        add(at, IconArt::BadgeImage("textures/icons/weapons/bullet.png", Color::srgb(0.95, 0.78, 0.35)));
    }
    if let Some(at) = layout.crafting_table {
        add(at, IconArt::BadgeImage("textures/icons/weapons/frag.png", Color::srgb(1.0, 0.6, 0.2)));
    }
    if let Some(at) = layout.armor_station {
        add(at, IconArt::BadgeImage("textures/icons/armor/armor.png", Color::srgb(0.55, 0.8, 1.0)));
    }
    for wall_buy in &layout.wall_buys {
        add(wall_buy.at, IconArt::BadgeImage(crate::wall_buys::icon(wall_buy.weapon), Color::srgb(0.85, 0.85, 0.85)));
    }
    if let Some(at) = layout.power_switch {
        add(at, IconArt::BadgeText("PWR", Color::srgb(1.0, 0.85, 0.2)));
    }
    if let Some(at) = layout.mystery_box {
        add(at, IconArt::BadgeText("?", Color::srgb(0.35, 0.75, 1.0)));
    }
    if shared::wunderfizz::present(lobby) {
        add(layout.wunderfizz, IconArt::BadgeText("W", Color::srgb(0.95, 0.4, 0.85)));
    }
    if let Some(at) = layout.pack_a_punch {
        add(at, IconArt::BadgeText("PAP", Color::srgb(0.7, 0.4, 1.0)));
    }
    for perk in lobby.perk_set.machine_perks() {
        add(layout.perk(perk), IconArt::Image(crate::zombies_hud::perk_icon_path(perk)));
    }
    if let Some(at) = layout.exfil_radio.filter(|_| shared::exfil::available(lobby)) {
        add(at, IconArt::Image(crate::exfil::EXFIL_ICON));
    }
    icons
}

/// Keep each map view's icons matching what our game should show.
fn sync_icons(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut frames: Query<(Entity, &mut MinimapFrame)>,
    icons: Query<(Entity, &ChildOf), With<MinimapIcon>>,
    mut commands: Commands,
) {
    let wanted = zombies_game(&local, &lobbies).map(wanted_icons).unwrap_or_default();
    let font = asset_server.load(HUD_FONT);
    for (frame, mut shown) in &mut frames {
        if shown.icons == wanted {
            continue;
        }
        for (e, parent) in &icons {
            if parent.parent() == frame {
                commands.entity(e).despawn();
            }
        }
        let size = if shown.full { FULL_ICON_SIZE } else { ICON_SIZE };
        for spec in &wanted {
            let node = Node {
                position_type: PositionType::Absolute,
                width: Val::Px(size),
                height: Val::Px(size),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            };
            let mut icon = commands.spawn((MinimapIcon { at: spec.at, size }, Visibility::Hidden, Pickable::IGNORE));
            icon.insert(ChildOf(frame));
            match spec.art {
                IconArt::Image(path) => {
                    icon.insert((node, ImageNode::new(asset_server.load(path))));
                }
                IconArt::BadgeImage(path, color) | IconArt::BadgeText(path, color) => {
                    icon.insert((
                        Node {
                            border: UiRect::all(Val::Px(1.5)),
                            ..node
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
                        BorderColor(color),
                        BorderRadius::MAX,
                    ));
                    if matches!(spec.art, IconArt::BadgeImage(..)) {
                        icon.with_child((
                            ImageNode::new(asset_server.load(path)).with_color(color),
                            Node {
                                width: Val::Px(size * 0.62),
                                height: Val::Px(size * 0.62),
                                ..default()
                            },
                        ));
                    } else {
                        let text = if path.len() > 1 { 0.5 } else { 0.7 };
                        icon.with_child((
                            Text::new(path),
                            TextFont {
                                font: font.clone(),
                                font_size: size * text,
                                ..default()
                            },
                            TextColor(color),
                        ));
                    }
                }
            }
        }
        shown.icons = wanted.clone();
    }
}

/// Keep an arrow on each map view for each other member of our `Zombies`
/// party.
#[allow(clippy::type_complexity)]
fn sync_teammates(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    arrow: Res<TeammateArrow>,
    frames: Query<(Entity, &MinimapFrame)>,
    avatars: Query<&RemoteAvatar>,
    poses: Query<&PlayerPose, (With<PlayerId>, Without<BotPose>)>,
    arrows: Query<(Entity, &MinimapTeammate, &ChildOf)>,
    mut commands: Commands,
) {
    let zombies = zombies_game(&local, &lobbies).is_some();
    let is_teammate = |src: Entity| zombies && poses.get(src).is_ok_and(|p| p.zombie == ZombieAnim::None);
    let mut have = Vec::new();
    for (e, mate, parent) in &arrows {
        if is_teammate(mate.0) {
            have.push((parent.parent(), mate.0));
        } else {
            commands.entity(e).despawn();
        }
    }
    for (frame, view) in &frames {
        let size = if view.full { FULL_ARROW_SIZE } else { TEAMMATE_SIZE };
        for avatar in &avatars {
            if have.contains(&(frame, avatar.src)) || !is_teammate(avatar.src) {
                continue;
            }
            have.push((frame, avatar.src));
            commands.spawn((
                MinimapTeammate(avatar.src),
                ChildOf(frame),
                ImageNode::new(arrow.0.clone()).with_color(TEAMMATE_BLUE),
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(size),
                    height: Val::Px(size),
                    ..default()
                },
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
}

/// Keep a red dot on the minimap (not the full map) for each enemy that
/// should show: `FreeForAll` enemies who've just fired (from
/// `SomeoneFired`), `Freestyle`'s standing bots.
#[allow(clippy::too_many_arguments)]
fn sync_enemy_dots(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut fired: EventReader<SomeoneFired>,
    mut shots: ResMut<ShotDots>,
    bots: Query<(Entity, &Bot), With<Interpolated>>,
    frames: Query<(Entity, &MinimapFrame)>,
    mut dots: Query<(Entity, &mut MinimapEnemy)>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let lobby = my_lobby(&local, &lobbies).filter(|l| l.started);
    let mode = lobby.map(|l| l.mode);
    // (Only our own lobby's enemies — and one dot each, the latest shot.)
    for shot in fired.read() {
        if mode != Some(GameMode::FreeForAll) || !lobby.is_some_and(|l| l.has(shot.shooter)) {
            continue;
        }
        shots.0.retain(|(who, ..)| *who != shot.shooter);
        shots.0.push((shot.shooter, shot.from.xz(), now));
    }
    if mode != Some(GameMode::FreeForAll) {
        shots.0.clear();
    }
    shots.0.retain(|&(.., at)| now - at < SHOT_DOT_HOLD_SECS + SHOT_DOT_FADE_SECS);

    let mut wanted: Vec<(EnemyKey, Vec2, f32)> = shots
        .0
        .iter()
        .map(|&(who, spot, at)| {
            let fade = ((now - at - SHOT_DOT_HOLD_SECS) / SHOT_DOT_FADE_SECS).clamp(0.0, 1.0);
            (EnemyKey::Shot(who), spot, 1.0 - fade)
        })
        .collect();
    if mode == Some(GameMode::Freestyle) {
        wanted.extend(bots.iter().filter(|(_, b)| b.alive).map(|(e, b)| (EnemyKey::Bot(e), b.pos.xz(), 1.0)));
    }

    let Some((frame, _)) = frames.iter().find(|(_, view)| !view.full) else { return };
    let mut have = Vec::new();
    for (e, mut dot) in &mut dots {
        match wanted.iter().find(|w| w.0 == dot.key) {
            Some(&(key, at, alpha)) => {
                have.push(key);
                if dot.at != at || dot.alpha != alpha {
                    dot.at = at;
                    dot.alpha = alpha;
                }
            }
            None => commands.entity(e).despawn(),
        }
    }
    for (key, at, alpha) in wanted {
        if have.contains(&key) {
            continue;
        }
        commands.spawn((
            MinimapEnemy { key, at, alpha },
            ChildOf(frame),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(ENEMY_DOT_SIZE),
                height: Val::Px(ENEMY_DOT_SIZE),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(ENEMY_RED),
            BorderColor(Color::BLACK.with_alpha(0.7)),
            BorderRadius::MAX,
            Visibility::Hidden,
            Pickable::IGNORE,
        ));
    }
}

/// How one map view lays the world out: the minimap centred on us and
/// turned with us, or the full map — the whole picture, north up.
#[derive(Clone, Copy)]
struct Projection {
    full: bool,
    /// The view's size (px).
    size: Vec2,
    /// Where we are, and our heading (radians, clockwise from north).
    centre: Vec2,
    heading: f32,
    /// [`Picture::rect`].
    rect: Vec4,
}

impl Projection {
    /// Where world `(x, z)` is on the view (the top-left of an item `size`
    /// px across), if it's far enough inside to show.
    fn place(&self, at: Vec2, size: f32) -> Option<Vec2> {
        if self.full {
            let p = (at - self.rect.xy()) / self.rect.zw() * self.size;
            let inside = p.cmpge(Vec2::ZERO).all() && p.cmple(self.size).all();
            return inside.then_some(p - Vec2::splat(size * 0.5));
        }
        let half = self.size.x * 0.5;
        let ahead = Vec2::new(self.heading.sin(), -self.heading.cos());
        let right = Vec2::new(self.heading.cos(), self.heading.sin());
        let d = (at - self.centre) / VIEW_RADIUS_M * half;
        let p = Vec2::new(d.dot(right), -d.dot(ahead));
        let limit = half - size * 0.4;
        (p.x.abs() < limit && p.y.abs() < limit).then_some(p + Vec2::splat(half - size * 0.5))
    }

    /// How far (radians, clockwise — UI y is down) to turn an arrow facing
    /// `heading` on this view.
    fn turn(&self, heading: f32) -> Quat {
        Quat::from_rotation_z(if self.full { heading } else { heading - self.heading })
    }

    /// The material's view: the minimap's centred on us and turned with us;
    /// the full map's on the picture's middle, north up, reaching to its
    /// nearer edges (its frame has the picture's shape, so it all fits).
    fn view(&self) -> Vec4 {
        if self.full {
            let mid = self.rect.xy() + self.rect.zw() * 0.5;
            Vec4::new(mid.x, mid.y, 0.0, self.rect.z.min(self.rect.w) * 0.5)
        } else {
            Vec4::new(self.centre.x, self.centre.y, self.heading, VIEW_RADIUS_M)
        }
    }
}

/// Put `node` at `spot` (hidden if it's off the view).
fn set_spot(node: &mut Mut<Node>, vis: &mut Mut<Visibility>, spot: Option<Vec2>) {
    let Some(spot) = spot else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    if node.left != Val::Px(spot.x) {
        node.left = Val::Px(spot.x);
    }
    if node.top != Val::Px(spot.y) {
        node.top = Val::Px(spot.y);
    }
}

/// Follow the camera: lay out both map views — the minimap centred and
/// turned, the full map sized to the screen — and put the icons, teammates,
/// enemies' dots and (on the full map) us where they are on them.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_minimap(
    picture: Res<MinimapPicture>,
    current: Res<CurrentMap>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<&GlobalTransform, With<WorldModelCamera>>,
    // (Us, or — while spectating — moved to whoever we're watching.)
    player: Query<&Transform, With<Player>>,
    mut frames: Query<(Entity, &MinimapFrame, &MaterialNode<MinimapMaterial>, &mut Node, &mut Visibility)>,
    mut materials: ResMut<Assets<MinimapMaterial>>,
    mut icons: Query<(&MinimapIcon, &ChildOf, &mut Node, &mut Visibility), Without<MinimapFrame>>,
    mut mates: Query<
        (&MinimapTeammate, &ChildOf, &mut Node, &mut Transform, &mut Visibility),
        (Without<MinimapFrame>, Without<MinimapIcon>, Without<Player>),
    >,
    mut enemies: Query<
        (&MinimapEnemy, &ChildOf, &mut Node, &mut Visibility, &mut BackgroundColor, &mut BorderColor),
        (Without<MinimapFrame>, Without<MinimapIcon>, Without<MinimapTeammate>),
    >,
    mut us: Query<
        (&ChildOf, &mut Node, &mut Transform, &mut Visibility),
        (
            With<FullMapPlayer>,
            Without<MinimapFrame>,
            Without<MinimapIcon>,
            Without<MinimapTeammate>,
            Without<MinimapEnemy>,
            Without<Player>,
        ),
    >,
    poses: Query<&PlayerPose>,
) {
    let drawn = picture.picture.as_ref().filter(|_| picture.drawn_for == Some(current.0));
    let (Some(drawn), Ok(camera)) = (drawn, camera.single()) else {
        for (.., mut vis) in &mut frames {
            vis.set_if_neq(Visibility::Hidden);
        }
        return;
    };
    let centre = camera.translation().xz();
    let (yaw, _, _) = camera.rotation().to_euler(EulerRot::YXZ);
    // (Clockwise from north: a yaw turns us anticlockwise from -Z.)
    let heading = -yaw;
    // (Our feet, not the camera: crouching mustn't change what's floor.)
    let eyes = player.single().map_or(camera.translation().y, |t| t.translation.y);
    // The full map: as big as fits, the picture's shape.
    let screen = window.single().map_or(Vec2::new(1280.0, 720.0), |w| Vec2::new(w.width(), w.height()));
    let room = screen * FULL_MAP_SCREEN_SHARE;
    let fit = (room.x / drawn.rect.z).min(room.y / drawn.rect.w);
    let full_size = (drawn.rect.zw() * fit).round();

    let mut views = Vec::new();
    for (frame, view, material, mut node, mut vis) in &mut frames {
        vis.set_if_neq(Visibility::Inherited);
        let size = if view.full { full_size } else { Vec2::splat(MINIMAP_SIZE) };
        if view.full && (node.width != Val::Px(size.x) || node.height != Val::Px(size.y)) {
            node.width = Val::Px(size.x);
            node.height = Val::Px(size.y);
        }
        let projection = Projection {
            full: view.full,
            size,
            centre,
            heading,
            rect: drawn.rect,
        };
        if let Some(m) = materials.get_mut(&material.0) {
            m.view = projection.view();
            m.rect = drawn.rect;
            m.heights = drawn.heights.extend(eyes - EYE_HEIGHT).extend(0.0);
            if m.map != drawn.image {
                m.map = drawn.image.clone();
            }
        }
        views.push((frame, projection));
    }
    let view_of = |parent: &ChildOf| views.iter().find(|(f, _)| *f == parent.parent()).map(|(_, v)| *v);

    for (icon, parent, mut node, mut vis) in &mut icons {
        let spot = view_of(parent).and_then(|v| v.place(icon.at, icon.size));
        set_spot(&mut node, &mut vis, spot);
    }
    for (enemy, parent, mut node, mut vis, mut fill, mut rim) in &mut enemies {
        let spot = view_of(parent).and_then(|v| v.place(enemy.at, ENEMY_DOT_SIZE));
        set_spot(&mut node, &mut vis, spot);
        fill.set_if_neq(BackgroundColor(ENEMY_RED.with_alpha(enemy.alpha)));
        rim.set_if_neq(BorderColor(Color::BLACK.with_alpha(0.7 * enemy.alpha)));
    }
    for (mate, parent, mut node, mut transform, mut vis) in &mut mates {
        let (Ok(pose), Some(view)) = (poses.get(mate.0), view_of(parent)) else { continue };
        let at = pose.translation.xz();
        let size = if view.full { FULL_ARROW_SIZE } else { TEAMMATE_SIZE };
        // (Not over our own arrow — e.g. the one we're spectating.)
        let spot = view.place(at, size).filter(|_| pose.alive && at.distance(centre) > 1.0);
        set_spot(&mut node, &mut vis, spot);
        let turn = view.turn(-pose.yaw);
        if transform.rotation != turn {
            transform.rotation = turn;
        }
    }
    for (parent, mut node, mut transform, mut vis) in &mut us {
        let Some(view) = view_of(parent) else { continue };
        set_spot(&mut node, &mut vis, view.place(centre, FULL_ARROW_SIZE));
        let turn = view.turn(heading);
        if transform.rotation != turn {
            transform.rotation = turn;
        }
    }
}
