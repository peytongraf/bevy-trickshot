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
/// stretching).
pub(crate) const ART: &str = "textures/menu/main_menu_bg.png";
const ART_SIZE: Vec2 = Vec2::new(1672.0, 940.0);
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

/// Generated once at startup: the vignette's soft dark ring.
const VIGNETTE_IMAGE: Handle<Image> = weak_handle!("6d656e75-5f76-6967-6e65-747465310001");

pub(crate) struct MenuBackdropPlugin;

impl Plugin for MenuBackdropPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, make_backdrop_images).add_systems(
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
}

/// Fill an out-of-game page root with the backdrop: the page's first
/// children (so everything spawned after draws on top), pinned to its edges
/// — padding included — whatever its layout. [`animate_menu_backdrop`] does
/// the rest.
pub fn menu_background(parent: &mut ChildSpawnerCommands, asset_server: &AssetServer) {
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
            frame.spawn((Backdrop::Art, ImageNode::new(asset_server.load(ART)), placed()));
            frame.spawn((Backdrop::Shade, full(), BackgroundColor(Color::BLACK.with_alpha(SHADE))));
            let smoke = asset_server.load(SMOKE);
            for layer in 0..FOG_LAYERS {
                for half in 0..2 {
                    frame.spawn((
                        Backdrop::Fog(layer, half),
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
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    BorderRadius::MAX,
                    BoxShadow::new(Color::NONE, Val::ZERO, Val::ZERO, Val::Px(1.0), Val::Px(6.0)),
                ));
            }
            frame.spawn((ImageNode::new(VIGNETTE_IMAGE).with_mode(NodeImageMode::Stretch), full()));
        });
}

/// The vignette: black, clear in the middle, darkening toward the edges.
fn make_backdrop_images(mut images: ResMut<Assets<Image>>) {
    let n = 128u32;
    let mut vignette = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let p = (Vec2::new(x as f32, y as f32) + 0.5) / n as f32 * 2.0 - 1.0;
            let t = ((p.length() - 0.35) / 0.75).clamp(0.0, 1.0);
            let a = t * t * (3.0 - 2.0 * t) * 0.9;
            vignette.extend_from_slice(&[0, 0, 0, (a * 255.0) as u8]);
        }
    }
    let _ = images.insert(
        &VIGNETTE_IMAGE,
        Image::new(
            Extent3d { width: n, height: n, depth_or_array_layers: 1 },
            TextureDimension::D2,
            vignette,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ),
    );
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
    window: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    mut layers: Query<(
        &Backdrop,
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

    // The art: covering, over-scanned, pushing in and out and panning.
    let zoom = ART_OVERSCAN + ART_ZOOM * (t * 0.05).sin();
    let art_size = ART_SIZE * (screen.x / ART_SIZE.x).max(screen.y / ART_SIZE.y) * zoom;
    let spare = (art_size - screen) * 0.5;
    let pan = Vec2::new((t * 0.037).sin(), (t * 0.029 + 1.3).sin()) * ART_PAN;
    let art_at = -spare + spare * pan;

    // The shade: a breath of flicker, and the odd brown-out — a quick
    // double stutter darker, somewhere in a second picked at random.
    let second = t.floor() as i64;
    let in_second = t.fract();
    let brownout = rand01(seed(0, second, 0xb20)) < BROWNOUT_CHANCE && {
        let at = 0.1 + rand01(seed(1, second, 0xb20)) * 0.6;
        let d = in_second - at;
        (0.0..0.07).contains(&d) || (0.13..0.17).contains(&d)
    };
    let shade = SHADE
        + SHADE_FLICKER * (noise(t * 6.0, 0x5ade) * 2.0 - 1.0)
        + if brownout { BROWNOUT_DEPTH } else { 0.0 };

    for (layer, mut node, bg, image, shadow) in &mut layers {
        match *layer {
            Backdrop::Art => place(&mut node, art_at, art_size),
            Backdrop::Shade => {
                if let Some(mut bg) = bg {
                    bg.0 = Color::BLACK.with_alpha(shade.clamp(0.0, 1.0));
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
                    let alpha = (0.05 + 0.02 * l) * (0.7 + 0.6 * noise(t * 0.15, 0xf00 + i as u32));
                    // The deepest band catches the red glow off the forge.
                    let c = if i == FOG_LAYERS - 1 {
                        Color::srgba(0.85, 0.55, 0.52, alpha)
                    } else {
                        Color::srgba(0.72, 0.76, 0.84, alpha)
                    };
                    image.color = c;
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
                let ash = i % ASH_EVERY == 0;
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
