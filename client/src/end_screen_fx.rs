//! The end-of-game results' atmosphere: the moment the results screen
//! (`menu::Screen::MatchResults`) comes up, the world behind it drains
//! toward black and white (`revive::apply_down_view` does the grading), an
//! amber glow creeps in from the edges with embers floating up through it —
//! the menu backdrop's own (`menu_backdrop::results_backdrop`) — and fire
//! licks up along the bottom of the screen (`shaders/end_screen_fire.wgsl`),
//! all fading in together over [`EndScreenFxSettings::fade_secs`].
//!
//! Tuned in the debug panel's "End screen" section ([`EndScreenDebug`]),
//! which can also preview it over the game. Its layer is
//! `StateScoped(InGame)` and taken down as soon as the results are, and the
//! fade starts over every time — nothing carries between games.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy_egui::egui;

use crate::menu::{self, Screen};
use crate::menu_backdrop::{results_backdrop, ResultsGlow};
use crate::AppState;

const FIRE_SHADER: &str = "shaders/end_screen_fire.wgsl";

pub(crate) struct EndScreenFxPlugin;

impl Plugin for EndScreenFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<EndScreenFireMaterial>::default())
            .init_resource::<EndScreenFxSettings>()
            .init_resource::<EndScreenFx>()
            .add_systems(
                Update,
                (track_end_screen, show_layer, animate_layer)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnExit(AppState::InGame), reset);
    }
}

/// How the end screen's atmosphere looks — the "End screen" debug section.
#[derive(Resource, Clone)]
pub(crate) struct EndScreenFxSettings {
    /// How long (s) everything takes to fade in.
    pub(crate) fade_secs: f32,
    /// The world's colour saturation once faded in (1 = untouched, 0 =
    /// black and white).
    pub(crate) saturation: f32,
    /// The embers' opacity (× their own), and the amber glow's opacity and
    /// colour.
    pub(crate) embers: f32,
    pub(crate) glow: f32,
    pub(crate) glow_color: [f32; 3],
    /// The fire: how high up the screen it reaches (× its height), its
    /// opacity and brightness, how fast it licks up, how big its tongues are
    /// (higher = smaller) and how much it tears.
    pub(crate) fire_height: f32,
    pub(crate) fire_opacity: f32,
    pub(crate) fire_brightness: f32,
    pub(crate) fire_speed: f32,
    pub(crate) fire_detail: f32,
    pub(crate) fire_turbulence: f32,
    /// Debug: show it all over the game, without the results, for tuning.
    pub(crate) preview: bool,
}

impl Default for EndScreenFxSettings {
    fn default() -> Self {
        Self {
            fade_secs: 1.0,
            saturation: 0.4,
            embers: 1.0,
            glow: 0.36,
            glow_color: [1.0, 0.485, 0.399],
            fire_height: 0.31,
            fire_opacity: 0.13,
            fire_brightness: 0.75,
            fire_speed: 0.15,
            fire_detail: 5.9,
            fire_turbulence: 1.17,
            preview: false,
        }
    }
}

/// Whether the end screen's atmosphere is up, and for how long (s) — how
/// far it's faded in.
#[derive(Resource, Default)]
pub(crate) struct EndScreenFx {
    pub(crate) active: bool,
    shown_for: f32,
}

impl EndScreenFx {
    /// `0` just shown … `1` all the way in.
    pub(crate) fn fade(&self, settings: &EndScreenFxSettings) -> f32 {
        if !self.active {
            return 0.0;
        }
        (self.shown_for / settings.fade_secs.max(0.01)).clamp(0.0, 1.0)
    }
}

/// The fire's material: see `shaders/end_screen_fire.wgsl` for its two
/// parameter vectors.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone, Default)]
pub(crate) struct EndScreenFireMaterial {
    #[uniform(0)]
    a: Vec4,
    #[uniform(1)]
    b: Vec4,
}

impl UiMaterial for EndScreenFireMaterial {
    fn fragment_shader() -> ShaderRef {
        FIRE_SHADER.into()
    }
}

/// The layer over the world (under the results' own panel).
#[derive(Component)]
struct EndScreenLayer;

#[derive(Component)]
struct EndScreenFire;

fn reset(mut fx: ResMut<EndScreenFx>, mut glow: ResMut<ResultsGlow>) {
    *fx = EndScreenFx::default();
    glow.fade = 0.0;
}

/// Up while the results are (or the preview's on), its clock starting over
/// each time.
fn track_end_screen(
    time: Res<Time<Real>>,
    menu: Res<menu::Menu>,
    settings: Res<EndScreenFxSettings>,
    mut fx: ResMut<EndScreenFx>,
) {
    let active = menu.screen == Screen::MatchResults || settings.preview;
    if active {
        fx.shown_for += time.delta_secs();
    } else {
        fx.shown_for = 0.0;
    }
    fx.active = active;
}

/// Put the layer up with the results, and take it down with them.
fn show_layer(
    fx: Res<EndScreenFx>,
    layer: Query<Entity, With<EndScreenLayer>>,
    mut materials: ResMut<Assets<EndScreenFireMaterial>>,
    mut commands: Commands,
) {
    match (fx.active, layer.single()) {
        (true, Err(_)) => {
            commands
                .spawn((
                    Name::new("EndScreenFx"),
                    EndScreenLayer,
                    StateScoped(AppState::InGame),
                    // (Just under the results screen, 50.)
                    GlobalZIndex(49),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|layer| {
                    results_backdrop(layer);
                    layer.spawn((
                        EndScreenFire,
                        MaterialNode(materials.add(EndScreenFireMaterial::default())),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            width: Val::Percent(100.0),
                            ..default()
                        },
                    ));
                });
        }
        (false, Ok(layer)) => commands.entity(layer).despawn(),
        _ => {}
    }
}

/// Fade the layer in, and keep the fire and the glow to the settings.
fn animate_layer(
    time: Res<Time<Real>>,
    fx: Res<EndScreenFx>,
    settings: Res<EndScreenFxSettings>,
    mut glow: ResMut<ResultsGlow>,
    mut fire: Query<(&MaterialNode<EndScreenFireMaterial>, &mut Node), With<EndScreenFire>>,
    mut materials: ResMut<Assets<EndScreenFireMaterial>>,
) {
    let fade = fx.fade(&settings);
    glow.fade = fade;
    glow.embers = settings.embers;
    glow.glow = settings.glow;
    glow.glow_color = settings.glow_color;
    for (handle, mut node) in &mut fire {
        let height = Val::Percent(settings.fire_height.clamp(0.0, 1.0) * 100.0);
        if node.height != height {
            node.height = height;
        }
        if let Some(m) = materials.get_mut(&handle.0) {
            m.a = Vec4::new(
                time.elapsed_secs_wrapped(),
                settings.fire_opacity * fade,
                settings.fire_speed,
                settings.fire_detail,
            );
            m.b = Vec4::new(settings.fire_turbulence, settings.fire_brightness, 0.0, 0.0);
        }
    }
}

/// The "End screen" debug section.
#[derive(SystemParam)]
pub(crate) struct EndScreenDebug<'w> {
    settings: ResMut<'w, EndScreenFxSettings>,
}

impl EndScreenDebug<'_> {
    /// Its section in the main debug panel (`debug_ui`).
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        let settings = &mut *self.settings;
        let s = &mut *settings;
        ui.checkbox(&mut s.preview, "Preview over the game");
        ui.add(egui::Slider::new(&mut s.fade_secs, 0.0f32..=5.0).text("fades in over (s)"));
        ui.collapsing("World", |ui| {
            ui.add(egui::Slider::new(&mut s.saturation, 0.0f32..=1.0).text("colour (0 = black & white)"));
        });
        ui.collapsing("Amber glow & embers", |ui| {
            ui.horizontal(|ui| {
                ui.label("glow colour");
                ui.color_edit_button_rgb(&mut s.glow_color);
            });
            ui.add(egui::Slider::new(&mut s.glow, 0.0f32..=1.0).text("glow opacity"));
            ui.add(egui::Slider::new(&mut s.embers, 0.0f32..=2.0).text("embers opacity"));
        });
        ui.collapsing("Fire", |ui| {
            ui.add(egui::Slider::new(&mut s.fire_height, 0.0f32..=1.0).text("reaches up (× screen)"));
            ui.add(egui::Slider::new(&mut s.fire_opacity, 0.0f32..=1.0).text("opacity"));
            ui.add(egui::Slider::new(&mut s.fire_brightness, 0.0f32..=3.0).text("brightness"));
            ui.add(egui::Slider::new(&mut s.fire_speed, 0.0f32..=3.0).text("speed"));
            ui.add(egui::Slider::new(&mut s.fire_detail, 0.5f32..=10.0).text("detail (smaller tongues)"));
            ui.add(egui::Slider::new(&mut s.fire_turbulence, 0.0f32..=2.0).text("turbulence"));
        });
        ui.horizontal(|ui| {
            if ui.button("Copy to console").clicked() {
                info!(
                    "end screen: fade_secs: {:.2}, saturation: {:.2}, embers: {:.2}, glow: {:.2}, \
                     glow_color: [{:.3}, {:.3}, {:.3}], fire_height: {:.2}, fire_opacity: {:.2}, \
                     fire_brightness: {:.2}, fire_speed: {:.2}, fire_detail: {:.2}, fire_turbulence: {:.2}",
                    s.fade_secs, s.saturation, s.embers, s.glow, s.glow_color[0], s.glow_color[1],
                    s.glow_color[2], s.fire_height, s.fire_opacity, s.fire_brightness, s.fire_speed,
                    s.fire_detail, s.fire_turbulence,
                );
            }
            if ui.button("Reset").clicked() {
                *s = EndScreenFxSettings {
                    preview: s.preview,
                    ..default()
                };
            }
        });
    }
}
