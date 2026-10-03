//! The launch loading screen: an opaque cover over the window from the very
//! first frame until what the menu is drawn with — its fonts and backdrop
//! art — has loaded and the menu has been laid out with them, then a quick
//! fade out. Without it the menu shows up first with no font (text missing
//! or in the fallback face, every row sized wrong) and visibly reflows once
//! the fonts land.
//!
//! Those assets are requested in `PreStartup`, ahead of the big models and
//! sounds the rest of `Startup` queues, so they're near the front of the
//! loader's queue rather than behind them.
//!
//! Only ever shown once per launch — nothing here carries between games.

use bevy::prelude::*;

use crate::{BODY_FONT, HUD_FONT};

/// What the menu needs before it looks right: its fonts...
const FONTS: [&str; 2] = [HUD_FONT, BODY_FONT];
/// ...and its backdrop's images.
const IMAGES: [&str; 2] = [crate::menu_backdrop::ART, crate::menu_backdrop::SMOKE];
/// Frames to keep covering once everything's loaded, so text gets measured
/// with the real fonts and the layout settles (the window can also still be
/// resizing to its first size).
const SETTLE_FRAMES: u32 = 4;
/// Seconds the cover takes to fade away.
const FADE_SECS: f32 = 0.3;
/// Above every other UI layer.
const Z: i32 = i32::MAX - 1;
const COVER: Color = Color::srgb(0.02, 0.02, 0.025);

pub(crate) struct LoadingScreenPlugin;

impl Plugin for LoadingScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, spawn_loading_screen)
            .add_systems(Update, drive_loading_screen.run_if(resource_exists::<Preload>));
    }
}

/// The handles being waited on, and how far along the cover is. Removed with
/// the cover.
#[derive(Resource)]
struct Preload {
    handles: Vec<UntypedHandle>,
    /// Frames since everything finished loading.
    settled: u32,
    /// Seconds into the fade out, once it's started.
    fade: Option<f32>,
}

#[derive(Component)]
struct LoadingCover;

#[derive(Component)]
struct LoadingText;

fn spawn_loading_screen(mut commands: Commands, asset_server: Res<AssetServer>) {
    let handles = FONTS
        .iter()
        .map(|path| asset_server.load::<Font>(*path).untyped())
        .chain(IMAGES.iter().map(|path| asset_server.load::<Image>(*path).untyped()))
        .collect();
    commands.insert_resource(Preload {
        handles,
        settled: 0,
        fade: None,
    });
    commands
        .spawn((
            LoadingCover,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(COVER),
            GlobalZIndex(Z),
            // Swallows clicks meant for the menu underneath.
            Interaction::default(),
            bevy::ui::FocusPolicy::Block,
        ))
        .with_child((
            LoadingText,
            Text::new("LOADING"),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: 34.0,
                ..default()
            },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.0)),
        ));
}

fn drive_loading_screen(
    mut commands: Commands,
    mut preload: ResMut<Preload>,
    asset_server: Res<AssetServer>,
    time: Res<Time>,
    mut cover: Query<(Entity, &mut BackgroundColor), With<LoadingCover>>,
    mut text: Query<&mut TextColor, With<LoadingText>>,
) {
    let t = time.elapsed_secs();
    let Some(fade) = preload.fade else {
        // A slow pulse on the label while we wait.
        let pulse = 0.45 + 0.35 * (t * 3.0).sin();
        for mut c in &mut text {
            c.0 = c.0.with_alpha(pulse);
        }
        // A file that failed to load won't ever turn up — don't wait on it
        // forever (the menu just shows without it, as it would anyway).
        let ready = preload.handles.iter().all(|h| {
            asset_server.is_loaded_with_dependencies(h.id())
                || matches!(
                    asset_server.get_load_state(h.id()),
                    Some(bevy::asset::LoadState::Failed(_))
                )
        });
        if ready {
            preload.settled += 1;
            if preload.settled >= SETTLE_FRAMES {
                preload.fade = Some(0.0);
            }
        }
        return;
    };

    let fade = fade + time.delta_secs();
    preload.fade = Some(fade);
    let k = (1.0 - fade / FADE_SECS).clamp(0.0, 1.0);
    if k <= 0.0 {
        for (e, _) in &cover {
            commands.entity(e).despawn();
        }
        commands.remove_resource::<Preload>();
        return;
    }
    for (_, mut bg) in &mut cover {
        bg.0 = COVER.with_alpha(k);
    }
    for mut c in &mut text {
        c.0 = c.0.with_alpha(c.0.alpha().min(k));
    }
}
