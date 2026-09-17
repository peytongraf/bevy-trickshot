//! The CoD-style yellow "+N" score stack that pops up centre-screen when the
//! local player's shot scores.

use bevy::prelude::*;

use crate::{AppState, GameSounds, HUD_FONT};

/// Server told us a shot scored — the shooter's client pops a CoD-style yellow
/// stack. `total` is the shot's combined points (shown as its own line at the
/// top); `lines` are the itemised `(label, points)` that added up to it, top
/// to bottom below that.
#[derive(Event)]
pub(crate) struct TrickScoredEvent {
    pub(crate) total: u32,
    pub(crate) lines: Vec<(String, u32)>,
}

/// The score-popup stack (one per scored shot; a fresh one replaces the last).
#[derive(Component)]
pub(crate) struct ScorePopup {
    age: f32,
}

pub(crate) const SCORE_YELLOW: Color = Color::srgb(1.0, 0.82, 0.1);
pub(crate) const SCORE_POPUP_HOLD: f32 = 1.1;
pub(crate) const SCORE_POPUP_TTL: f32 = 2.6;

/// Spawn the yellow `+N  LABEL` stack, centred a little above the crosshair.
pub(crate) fn spawn_score_popup(
    mut events: EventReader<TrickScoredEvent>,
    existing: Query<Entity, With<ScorePopup>>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    sounds: Res<GameSounds>,
) {
    // Only the most recent shot matters if several land in one frame.
    let Some(ev) = events.read().last() else {
        return;
    };
    for e in &existing {
        commands.entity(e).despawn();
    }

    // `TrickScoredEvent` only ever fires for a kill *this* client just scored
    // (Practice resolves it locally; online, `net::receive_trick_scores`
    // already filters the server's broadcast down to our own shooter id).
    commands.spawn((
        AudioPlayer::new(sounds.kill_enemy.clone()),
        PlaybackSettings::DESPAWN,
    ));

    commands
        .spawn((
            ScorePopup { age: 0.0 },
            StateScoped(AppState::InGame),
            GlobalZIndex(9),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                top: Val::Percent(33.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(3.0),
                ..default()
            },
        ))
        .with_children(|col| {
            // The combined total leads the stack, bigger than the breakdown
            // below it, so it reads as the headline with the itemised lines
            // explaining where it came from.
            col.spawn((
                Text::new(format!("+{}  TOTAL", ev.total)),
                TextFont {
                    font: asset_server.load(HUD_FONT),
                    font_size: 32.0,
                    ..default()
                },
                TextColor(SCORE_YELLOW),
            ));
            for (label, points) in &ev.lines {
                col.spawn((
                    Text::new(format!("+{points}  {label}")),
                    TextFont {
                        font: asset_server.load(HUD_FONT),
                        font_size: 25.0,
                        ..default()
                    },
                    TextColor(SCORE_YELLOW),
                ));
            }
        });
}

/// Hold each popup briefly, then fade its lines out and despawn.
pub(crate) fn update_score_popups(
    time: Res<Time>,
    mut popups: Query<(Entity, &mut ScorePopup, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut commands: Commands,
) {
    for (entity, mut popup, children) in &mut popups {
        popup.age += time.delta_secs();
        if popup.age >= SCORE_POPUP_TTL {
            commands.entity(entity).despawn();
            continue;
        }
        let a = if popup.age < SCORE_POPUP_HOLD {
            1.0
        } else {
            1.0 - (popup.age - SCORE_POPUP_HOLD) / (SCORE_POPUP_TTL - SCORE_POPUP_HOLD)
        };
        for child in children {
            if let Ok(mut tc) = texts.get_mut(*child) {
                tc.0 = SCORE_YELLOW.with_alpha(a.clamp(0.0, 1.0));
            }
        }
    }
}
