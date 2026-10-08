//! The out-of-game screens' backdrop — the menu art
//! (`assets/textures/menu/`) brought to life the way a Call of Duty main
//! menu is, with layers of slow, never-quite-repeating motion:
//!
//! * the art itself drifts — a slow push in and out and a gentle pan,
//! * the darkening over it breathes, and now and then stutters darker for a
//!   moment, like the fortress's lights browning out,
//! * bands of fog roll across it (the smoke sprite, `vfx/smoke.png`),
//! * embers — and the odd fleck of grey ash — float up through it, glowing,
//! * and a vignette pulls the corners in.
//!
//! [`zombies_background`] is the same backdrop over the `Zombies` intro art
//! (shown while a zombies game loads — `game_start`), tuned to be scarier:
//! the lights flicker and brown out far more, the fog's colder and thicker,
//! more of what floats up is ash, a red glow throbs in from the edges like
//! a heartbeat, and now and then the storm flashes cold light across it.
//!
//! [`results_backdrop`] is just the embers, and an amber glow in from the
//! edges, over the world behind the end-of-game results — faded in, and
//! tuned, by `end_screen_fx` ([`ResultsGlow`]).
//!
//! Every layer is worked out from the clock alone each frame (per-ember
//! randomness is a hash of its index and its current lifetime), with no
//! state of its own: the pages rebuild on every click, and a rebuilt
//! backdrop simply picks up exactly where the old one was. It all runs
//! right before UI layout, so a freshly built backdrop is correct in its
//! very first frame (the page-rebuild flicker).

use bevy::asset::{weak_handle, RenderAssetUsages};
use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;

use crate::util::rand01;

/// The menu art, and its size in pixels (to cover the screen without
/// stretching)...
pub(crate) const ART: &str = "textures/menu/main_menu_bg.png";
const ART_SIZE: Vec2 = Vec2::new(1672.0, 940.0);
/// ...and the `Zombies` intro's.
const ZOMBIES_ART: &str = "textures/menu/zombies_loading_bg.png";
const ZOMBIES_ART_SIZE: Vec2 = Vec2::new(1672.0, 941.0);
/// The art's scale past just covering the screen, so its drift never shows
/// an edge...
const ART_OVERSCAN: f32 = 1.18;
/// ...how far it pushes in and out on top of that...
const ART_ZOOM: f32 = 0.05;
/// ...and how much of that spare margin its pan uses (under 1, so it can't).
const ART_PAN: f32 = 0.6;

/// The darkening over the art (so the menu on top stays readable)...
const SHADE: f32 = 0.52;
/// ...its gentle flicker...
const SHADE_FLICKER: f32 = 0.03;
/// ...and a brown-out: how much darker it dips, and the chance of one in any
/// given second.
const BROWNOUT_DEPTH: f32 = 0.14;
const BROWNOUT_CHANCE: f32 = 0.08;

/// Fog bands, each two smoke sprites wide so it always spans the screen.
const FOG_LAYERS: usize = 4;
pub(crate) const SMOKE: &str = "textures/vfx/smoke.png";

/// Embers (and ash) on screen at once.
const EMBERS: usize = 56;
/// One in this many is grey ash rather than a glowing ember.
const ASH_EVERY: usize = 4;

/// Generated once at startup: the vignette's soft dark ring, and the same
/// ring in white (tinted for the `Zombies` heartbeat).
const VIGNETTE_IMAGE: Handle<Image> = weak_handle!("6d656e75-5f76-6967-6e65-747465310001");
const GLOW_IMAGE: Handle<Image> = weak_handle!("6d656e75-5f76-6967-6e65-747465310002");

/// The `Zombies` heartbeat: seconds per beat, and how strong its red gets.
const HEARTBEAT_SECS: f32 = 1.5;
const HEARTBEAT_ALPHA: f32 = 0.32;
/// The `Zombies` storm: the chance of a lightning flash in any given
/// second, and how bright it gets.
const LIGHTNING_CHANCE: f32 = 0.045;
const LIGHTNING_ALPHA: f32 = 0.16;

pub(crate) struct MenuBackdropPlugin;

impl Plugin for MenuBackdropPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ResultsGlow>()
            .add_systems(Startup, make_backdrop_images)
            .add_systems(
            PostUpdate,
            animate_menu_backdrop.before(bevy::ui::UiSystem::Layout),
        );
    }
}

/// Which backdrop layer a node is.
#[derive(Component, Clone, Copy)]
enum Backdrop {
    Art,
    Shade,
    /// Layer, and which of its two sprites.
    Fog(usize, usize),
    Ember(usize),
    /// `Zombies` only: the red heartbeat in from the edges...
    Heartbeat,
    /// ...and the lightning.
    Lightning,
    /// The results screen's amber glow in from the edges.
    AmberGlow,
}

/// How [`results_backdrop`]'s layers show right now — set every frame by
/// `end_screen_fx`: how far faded in (0..1), the embers' opacity, and the
/// amber glow's colour and opacity.
#[derive(Resource, Default)]
pub(crate) struct ResultsGlow {
    pub(crate) fade: f32,
    pub(crate) embers: f32,
    pub(crate) glow: f32,
    pub(crate) glow_color: [f32; 3],
}

/// Which backdrop a layer belongs to.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Theme {
    Menu,
    Zombies,
    /// The end-of-game results' embers and glow ([`results_backdrop`]).
    Results,
}

/// How a [`Theme`] tunes the shared layers.
struct Look {
    art_size: Vec2,
    art_zoom: f32,
    shade: f32,
    shade_flicker: f32,
    brownout_depth: f32,
    brownout_chance: f32,
    fog_alpha: f32,
    fog_cold: Color,
    fog_glow: Color,
    ash_every: usize,
}

impl Theme {
    fn look(self) -> Look {
        match self {
            Theme::Menu | Theme::Results => Look {
                art_size: ART_SIZE,
                art_zoom: ART_ZOOM,
                shade: SHADE,
                shade_flicker: SHADE_FLICKER,
                brownout_depth: BROWNOUT_DEPTH,
                brownout_chance: BROWNOUT_CHANCE,
                fog_alpha: 1.0,
                fog_cold: Color::srgb(0.72, 0.76, 0.84),
                fog_glow: Color::srgb(0.85, 0.55, 0.52),
                ash_every: ASH_EVERY,
            },
            Theme::Zombies => Look {
                art_size: ZOMBIES_ART_SIZE,
                art_zoom: ART_ZOOM * 1.4,
                shade: 0.4,
                shade_flicker: 0.05,
                brownout_depth: 0.3,
                brownout_chance: 0.2,
                fog_alpha: 1.5,
                fog_cold: Color::srgb(0.55, 0.62, 0.72),
                fog_glow: Color::srgb(0.6, 0.18, 0.14),
                ash_every: 2,
            },
        }
    }
}

/// Keeps the `Zombies` intro art loaded from launch, so it's there the
/// moment a zombies game starts loading rather than popping in.
#[derive(Resource)]
#[allow(dead_code)]
struct KeepZombiesArt(Handle<Image>);

/// Fill an out-of-game page root with the backdrop: the page's first
/// children (so everything spawned after draws on top), pinned to its edges
/// — padding included — whatever its layout. [`animate_menu_backdrop`] does
/// the rest.
pub fn menu_background(parent: &mut ChildSpawnerCommands, asset_server: &AssetServer) {
    spawn_backdrop(parent, asset_server, Theme::Menu);
}

/// [`menu_background`], over the `Zombies` intro art and with its scarier
/// layers (see the module docs).
pub fn zombies_background(parent: &mut ChildSpawnerCommands, asset_server: &AssetServer) {
    spawn_backdrop(parent, asset_server, Theme::Zombies);
}

/// The embers floating up and an amber glow in from the edges — for over
/// the world behind the results screen (`end_screen_fx`), filling `parent`.
pub(crate) fn results_backdrop(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|frame| {
            frame.spawn((
                Backdrop::AmberGlow,
                Theme::Results,
                ImageNode {
                    image: GLOW_IMAGE,
                    color: Color::NONE,
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
            ));
            for i in 0..EMBERS {
                frame.spawn((
                    Backdrop::Ember(i),
                    Theme::Results,
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::MAX,
                    BoxShadow::new(Color::NONE, Val::ZERO, Val::ZERO, Val::Px(1.0), Val::Px(6.0)),
                ));
            }
        });
}

fn spawn_backdrop(parent: &mut ChildSpawnerCommands, asset_server: &AssetServer, theme: Theme) {
    let art = match theme {
        Theme::Menu | Theme::Results => ART,
        Theme::Zombies => ZOMBIES_ART,
    };
    let full = || Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    };
    let placed = || Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    parent
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|frame| {
            frame.spawn((Backdrop::Art, theme, ImageNode::new(asset_server.load(art)), placed()));
            frame.spawn((
                Backdrop::Shade,
                theme,
                full(),
                BackgroundColor(Color::BLACK.with_alpha(theme.look().shade)),
            ));
            let smoke = asset_server.load(SMOKE);
            for layer in 0..FOG_LAYERS {
                for half in 0..2 {
                    frame.spawn((
                        Backdrop::Fog(layer, half),
                        theme,
                        ImageNode {
                            image: smoke.clone(),
                            color: Color::NONE,
                            // (Alternate sprites mirrored, so the seam doesn't
                            // repeat one shape.)
                            flip_x: half == 1,
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        placed(),
                    ));
                }
            }
            for i in 0..EMBERS {
                frame.spawn((
                    Backdrop::Ember(i),
                    theme,
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::MAX,
                    BoxShadow::new(Color::NONE, Val::ZERO, Val::ZERO, Val::Px(1.0), Val::Px(6.0)),
                ));
            }
            if theme == Theme::Zombies {
                frame.spawn((
                    Backdrop::Lightning,
                    theme,
                    full(),
                    BackgroundColor(Color::NONE),
                ));
            }
            frame.spawn((ImageNode::new(VIGNETTE_IMAGE).with_mode(NodeImageMode::Stretch), full()));
            if theme == Theme::Zombies {
                frame.spawn((
                    Backdrop::Heartbeat,
                    theme,
                    ImageNode {
                        image: GLOW_IMAGE,
                        color: Color::NONE,
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    full(),
                ));
            }
        });
}

/// The vignette: black, clear in the middle, darkening toward the edges —
/// and its white twin. Also starts the `Zombies` art loading.
fn make_backdrop_images(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.insert_resource(KeepZombiesArt(asset_server.load(ZOMBIES_ART)));
    let n = 128u32;
    for (handle, rgb) in [(VIGNETTE_IMAGE, 0u8), (GLOW_IMAGE, 255u8)] {
        let mut ring = Vec::with_capacity((n * n * 4) as usize);
        for y in 0..n {
            for x in 0..n {
                let p = (Vec2::new(x as f32, y as f32) + 0.5) / n as f32 * 2.0 - 1.0;
                let t = ((p.length() - 0.35) / 0.75).clamp(0.0, 1.0);
                let a = t * t * (3.0 - 2.0 * t) * 0.9;
                ring.extend_from_slice(&[rgb, rgb, rgb, (a * 255.0) as u8]);
            }
        }
        let _ = images.insert(
            &handle,
            Image::new(
                Extent3d { width: n, height: n, depth_or_array_layers: 1 },
                TextureDimension::D2,
                ring,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            ),
        );
    }
}

/// Smooth value noise over time: a random value per whole step of `t`,
/// eased between.
fn noise(t: f32, seed: u32) -> f32 {
    let (i, f) = (t.floor(), t.fract());
    let a = rand01((i as i32 as u32).wrapping_mul(747_796_405) ^ seed);
    let b = rand01(((i as i32 + 1) as u32).wrapping_mul(747_796_405) ^ seed);
    a + (b - a) * f * f * (3.0 - 2.0 * f)
}

/// A seed from an ember's index and which lifetime it's on.
fn seed(i: usize, cycle: i64, salt: u32) -> u32 {
    (i as u32).wrapping_mul(2_654_435_761) ^ (cycle as u32).wrapping_mul(40_503) ^ salt
}

/// Move every backdrop layer to where the clock says it is (see the module
/// docs).
#[allow(clippy::type_complexity)]
fn animate_menu_backdrop(
    time: Res<Time<Real>>,
    results: Res<ResultsGlow>,
    window: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    mut layers: Query<(
        &Backdrop,
        &Theme,
        &mut Node,
        Option<&mut BackgroundColor>,
        Option<&mut ImageNode>,
        Option<&mut BoxShadow>,
    )>,
) {
    if layers.is_empty() {
        return;
    }
    let Ok(window) = window.single() else { return };
    // (`Val::Px` is in UI units — logical pixels over `UiScale`.)
    let screen = Vec2::new(window.width(), window.height()) / ui_scale.0.max(1e-3);
    // Sizes are designed at 1080p and scaled with the window.
    let unit = screen.y / 1080.0;
    let t = time.elapsed_secs();

    let second = t.floor() as i64;
    let in_second = t.fract();

    for (layer, theme, mut node, bg, image, shadow) in &mut layers {
        let look = theme.look();
        match *layer {
            // The art: covering, over-scanned, pushing in and out and panning.
            Backdrop::Art => {
                let zoom = ART_OVERSCAN + look.art_zoom * (t * 0.05).sin();
                let art_size = look.art_size
                    * (screen.x / look.art_size.x).max(screen.y / look.art_size.y)
                    * zoom;
                let spare = (art_size - screen) * 0.5;
                let pan = Vec2::new((t * 0.037).sin(), (t * 0.029 + 1.3).sin()) * ART_PAN;
                place(&mut node, -spare + spare * pan, art_size);
            }
            // The shade: a breath of flicker, and the odd brown-out — a
            // quick double stutter darker, somewhere in a second picked at
            // random.
            Backdrop::Shade => {
                let brownout = rand01(seed(0, second, 0xb20)) < look.brownout_chance && {
                    let at = 0.1 + rand01(seed(1, second, 0xb20)) * 0.6;
                    let d = in_second - at;
                    (0.0..0.07).contains(&d) || (0.13..0.17).contains(&d)
                };
                let shade = look.shade
                    + look.shade_flicker * (noise(t * 6.0, 0x5ade) * 2.0 - 1.0)
                    + if brownout { look.brownout_depth } else { 0.0 };
                if let Some(mut bg) = bg {
                    bg.0 = Color::BLACK.with_alpha(shade.clamp(0.0, 1.0));
                }
            }
            // A heartbeat: a strong thump and a softer one just after, then
            // a rest, swelling red in from the edges.
            Backdrop::Heartbeat => {
                let p = (t / HEARTBEAT_SECS).fract() * HEARTBEAT_SECS;
                let thump = |at: f32| (-((p - at) / 0.07).powi(2)).exp();
                let beat = 0.3 + 0.7 * (thump(0.0) + thump(HEARTBEAT_SECS) + 0.65 * thump(0.24));
                if let Some(mut image) = image {
                    image.color = Color::srgba(0.6, 0.0, 0.02, HEARTBEAT_ALPHA * beat);
                }
            }
            // The results' amber glow: in from the edges, breathing slowly.
            Backdrop::AmberGlow => {
                let breathe = 0.85 + 0.15 * (t * 1.3).sin();
                let [r, g, b] = results.glow_color;
                if let Some(mut image) = image {
                    image.color = Color::srgba(r, g, b, (results.glow * results.fade * breathe).clamp(0.0, 1.0));
                }
            }
            // Lightning: in the odd second, a bright flash and a fainter
            // flicker after it, each fading fast.
            Backdrop::Lightning => {
                let a = if rand01(seed(2, second, 0x11e)) < LIGHTNING_CHANCE {
                    let d = in_second - rand01(seed(3, second, 0x11e)) * 0.5;
                    let flash = |at: f32, len: f32| {
                        let d = d - at;
                        if (0.0..len).contains(&d) { 1.0 - d / len } else { 0.0 }
                    };
                    flash(0.0, 0.12).max(0.6 * flash(0.18, 0.3))
                } else {
                    0.0
                };
                if let Some(mut bg) = bg {
                    bg.0 = Color::srgba(0.75, 0.85, 1.0, LIGHTNING_ALPHA * a);
                }
            }
            Backdrop::Fog(i, half) => {
                let l = i as f32;
                // Lower bands are thicker, nearer (faster) and stronger.
                let size = Vec2::new(screen.x * 1.6, screen.y * (0.45 + 0.12 * l));
                let speed = screen.x * (0.008 + 0.006 * l) * if i % 2 == 0 { 1.0 } else { -1.0 };
                let offset = (t * speed + rand01(seed(i, 0, 0xf06)) * size.x).rem_euclid(size.x);
                let x = offset - size.x + half as f32 * size.x;
                let bob = (t * (0.07 + 0.03 * l) + l * 1.7).sin() * screen.y * 0.02;
                let y = screen.y * (0.3 + 0.13 * l) + bob;
                place(&mut node, Vec2::new(x, y), size);
                if let Some(mut image) = image {
                    let alpha = (0.05 + 0.02 * l)
                        * (0.7 + 0.6 * noise(t * 0.15, 0xf00 + i as u32))
                        * look.fog_alpha;
                    // The deepest band catches the red glow off the fires.
                    let c = if i == FOG_LAYERS - 1 { look.fog_glow } else { look.fog_cold };
                    image.color = c.with_alpha(alpha);
                }
            }
            Backdrop::Ember(i) => {
                // Each lives `life` seconds, rising from below the screen,
                // then is reborn somewhere new.
                let life = 7.0 + 8.0 * rand01(seed(i, -1, 0xe1));
                let tt = t + rand01(seed(i, -2, 0xe1)) * life;
                let cycle = (tt / life).floor() as i64;
                let f = (tt / life).fract();
                let r = |salt: u32| rand01(seed(i, cycle, salt));
                let ash = i % look.ash_every == 0;
                let rise = screen.y * (0.45 + 0.6 * r(1));
                let sway = (f * core::f32::consts::TAU * (1.0 + 2.0 * r(2)) + r(3) * 6.3).sin()
                    * (10.0 + 25.0 * r(4))
                    * unit;
                let x = screen.x * r(5) + (r(6) - 0.5) * screen.x * 0.18 * f + sway;
                let y = screen.y * 1.02 - f * rise;
                let d = (if ash { 2.5 + 2.5 * r(7) } else { 1.8 + 2.8 * r(7) }) * unit;
                place(&mut node, Vec2::new(x, y), Vec2::splat(d));
                // In and out over its life, flickering like it's burning.
                let life_alpha = (f * core::f32::consts::PI).sin().powf(0.7);
                let burn = 0.65 + 0.35 * noise(t * 5.0 + i as f32 * 3.1, 0xe7);
                let a = life_alpha * (0.35 + 0.65 * r(8)) * if ash { 0.45 } else { burn };
                // (The results' fade in with the rest of the end screen.)
                let a = if *theme == Theme::Results { a * results.embers * results.fade } else { a };
                let (core, glow) = if ash {
                    (Color::srgba(0.72, 0.72, 0.72, a), Color::NONE)
                } else {
                    let g = 0.35 + 0.25 * r(9);
                    (
                        Color::srgba(1.0, g + 0.2, 0.2, a),
                        Color::srgba(1.0, g, 0.1, a * 0.55),
                    )
                };
                if let Some(mut bg) = bg {
                    bg.0 = core;
                }
                if let Some(mut shadow) = shadow {
                    if let Some(s) = shadow.0.first_mut() {
                        s.color = glow;
                        s.blur_radius = Val::Px(6.0 * unit);
                    }
                }
            }
        }
    }
}

fn place(node: &mut Node, at: Vec2, size: Vec2) {
    node.left = Val::Px(at.x);
    node.top = Val::Px(at.y);
    node.width = Val::Px(size.x);
    node.height = Val::Px(size.y);
}
