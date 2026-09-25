//! The party leader's pause (`shared::Lobby::paused`, toggled from the `Esc`
//! menu's PAUSE GAME button via `shared::SetPaused`). The server freezes the
//! clock, bots, zombies, knives and shots; here every member's client stops
//! taking gameplay input ([`GamePaused`] feeds `menu::game_active`) and shows
//! a big PAUSED over the world.
//!
//! Nothing to reset between games: [`GamePaused`] is re-derived from the
//! replicated lobby every frame (false outside a running game), and the text
//! is `StateScoped(InGame)`.

use bevy::prelude::*;
use lightyear::prelude::LocalId;

use crate::net::GameClient;
use crate::{menu, AppState, HUD_FONT};

pub(crate) struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GamePaused>()
            // Not gated on `InGame`: it's what clears the flag once the game
            // is left.
            .add_systems(Update, sync_paused)
            .add_systems(OnEnter(AppState::InGame), spawn_paused_text)
            .add_systems(
                Update,
                show_paused_text
                    .after(sync_paused)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Our running game is paused by the party leader.
#[derive(Resource, Default)]
pub(crate) struct GamePaused(pub(crate) bool);

#[derive(Component)]
struct PausedText;

fn sync_paused(
    state: Res<State<AppState>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut paused: ResMut<GamePaused>,
    mut menu: ResMut<menu::Menu>,
) {
    let me = local.iter().next().map(|l| l.0);
    let now = *state.get() == AppState::InGame
        && me.is_some_and(|me| lobbies.iter().any(|l| l.has(me) && l.started && l.paused));
    if paused.0 != now {
        paused.0 = now;
        // The pause menu's PAUSE / RESUME button reads it.
        if menu.screen == menu::Screen::Settings {
            menu.dirty = true;
        }
    }
}

/// Big white PAUSED with a black shadow, a little above the centre.
fn spawn_paused_text(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            // Over the HUD, under the menu.
            GlobalZIndex(40),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(30.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Visibility::Hidden,
            PausedText,
        ))
        .with_child((
            Text::new("PAUSED"),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: 140.0,
                ..default()
            },
            TextColor(Color::WHITE),
            TextShadow {
                offset: Vec2::splat(5.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.85),
            },
        ));
}

fn show_paused_text(paused: Res<GamePaused>, mut text: Single<&mut Visibility, With<PausedText>>) {
    text.set_if_neq(if paused.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}
