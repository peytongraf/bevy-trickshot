//! The dedicated HUD camera, and the top-level "is the HUD allowed to show
//! at all right now" gate every other HUD element defers to.

use bevy::prelude::*;
use bevy::render::camera::CameraOutputMode;
use bevy::render::render_resource::BlendState;
use bevy::render::view::RenderLayers;
use bevy::ui::IsDefaultUiCamera;
use bevy_egui::PrimaryEguiContext;

use crate::{killcam, menu, AppState, UI_LAYER};

/// The one persistent 2D camera: hosts every `bevy_ui` tree — the main menu and
/// lobby screens as well as the in-game HUD. Drawn last (order 2, no clear) so
/// the view-model gun (view-model camera, order 1) can't render over the HUD.
/// Lives for the whole process; menu screens paint their own opaque background.
/// Show the HUD (crosshair / ammo / FPS) only while actually in a game and no
/// menu overlay is up. The world itself is always loaded but hidden behind the
/// opaque lobby UI outside `InGame`.
pub(crate) fn hud_visibility(
    state: Res<State<AppState>>,
    menu: Res<menu::Menu>,
    killcam: Res<killcam::ActiveKillCam>,
    mut hud: Query<&mut Visibility, With<menu::HudElement>>,
) {
    if !(state.is_changed() || menu.is_changed() || killcam.is_changed()) {
        return;
    }
    let show = *state.get() == AppState::InGame && !menu.is_open() && killcam.0.is_none();
    let want = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut hud {
        if *v != want {
            *v = want;
        }
    }
}

pub(crate) fn setup_ui_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            // Always blend over the 3D view — left unset, Bevy may pick this
            // camera as the opaque one and black out the world (see the world
            // camera's `output_mode` in `main.rs`).
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            ..default()
        },
        RenderLayers::layer(UI_LAYER),
        IsDefaultUiCamera,
        // The debug panels draw here too, on top of everything and
        // untouched by 3D post-processing.
        PrimaryEguiContext,
    ));
}
