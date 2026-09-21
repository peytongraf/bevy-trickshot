//! The end of a `FreeForAll` match: the server announces it (`shared::MatchEnding`)
//! and for exactly [`shared::MATCH_END_FREEZE_SECS`] nothing counts — the server
//! ignores hits and this client takes no input (`menu::game_active` goes false,
//! so movement, looking, shooting and knife throws all stop however hard the
//! player mashes keys) — while a full-screen overlay plays: a dim black wash
//! that flashes quickly to white and back, then VICTORY (green) or DEFEAT (red)
//! punches in. When the freeze is over the end-of-match replay starts
//! (`net::receive_killcam` holds it until then), then the results screen.
//!
//! The freeze stays in force (`MatchEndFreeze::active`) until the results screen
//! opens (`MatchEndedEvent`) or the game is left; only the overlay is timed.

use bevy::prelude::*;
use lightyear::prelude::*;

use crate::net::GameClient;
use crate::ui::{DEFEAT, VICTORY};
use crate::{AppState, MatchEndedEvent};

/// Overlay wash while nothing is flashing: black at this opacity.
const WASH_ALPHA: f32 = 0.25;
/// Peak opacity of the white flash.
const FLASH_ALPHA: f32 = 0.85;
/// Seconds to reach the white peak, and to be back to the black wash.
const FLASH_PEAK_SECS: f32 = 0.12;
const FLASH_END_SECS: f32 = 0.35;
/// When the headline starts to pop in, how long the pop takes, and how quickly
/// it fades in.
const TEXT_START_SECS: f32 = 0.30;
const TEXT_POP_SECS: f32 = 0.40;
const TEXT_FADE_SECS: f32 = 0.10;
/// Headline size at rest (px) and how much bigger it starts.
const TEXT_SIZE: f32 = 150.0;
const TEXT_START_SCALE: f32 = 3.0;
/// After landing the headline keeps swelling slowly, by this much overall.
const TEXT_DRIFT: f32 = 0.06;

/// Whether the end-of-match freeze is in force and how far the overlay is.
#[derive(Resource, Default)]
pub(crate) struct MatchEndFreeze {
    /// `MatchEnding` received and the results screen not shown yet: all
    /// gameplay input is off.
    pub(crate) active: bool,
    victory: bool,
    /// Seconds since `MatchEnding` arrived.
    elapsed: f32,
    spawned: bool,
}

impl MatchEndFreeze {
    pub(crate) fn start(&mut self, victory: bool) {
        *self = Self {
            active: true,
            victory,
            ..default()
        };
    }

    /// The timed VICTORY / DEFEAT screen is still up.
    pub(crate) fn overlay_running(&self) -> bool {
        self.active && self.elapsed < shared::MATCH_END_FREEZE_SECS
    }
}

#[derive(Component)]
struct MatchEndOverlay;

#[derive(Component)]
struct MatchEndText;

pub struct MatchEndPlugin;

impl Plugin for MatchEndPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchEndFreeze>()
            .add_systems(OnEnter(AppState::InGame), reset_freeze)
            .add_systems(OnExit(AppState::InGame), reset_freeze)
            .add_systems(
                Update,
                (receive_match_ending, clear_on_results, drive_overlay)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn reset_freeze(mut freeze: ResMut<MatchEndFreeze>) {
    *freeze = MatchEndFreeze::default();
}

/// The server says the match is over: freeze, and work out which headline.
fn receive_match_ending(
    mut receivers: Query<&mut MessageReceiver<shared::MatchEnding>>,
    local: Query<&LocalId, With<GameClient>>,
    mut freeze: ResMut<MatchEndFreeze>,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            let victory = me.is_some_and(|me| msg.winners.contains(&me));
            freeze.start(victory);
        }
    }
}

/// The results screen is opening: the freeze has done its job.
fn clear_on_results(mut ended: EventReader<MatchEndedEvent>, mut freeze: ResMut<MatchEndFreeze>) {
    if ended.read().next().is_some() {
        *freeze = MatchEndFreeze::default();
    }
}

/// Spawn, animate and remove the overlay.
fn drive_overlay(
    mut commands: Commands,
    time: Res<Time>,
    asset_server: Res<AssetServer>,
    mut freeze: ResMut<MatchEndFreeze>,
    mut wash: Query<(Entity, &mut BackgroundColor), With<MatchEndOverlay>>,
    mut text: Query<(&mut TextFont, &mut TextColor), With<MatchEndText>>,
) {
    if !freeze.active {
        return;
    }
    freeze.elapsed += time.delta_secs();

    if !freeze.overlay_running() {
        for (e, _) in &wash {
            commands.entity(e).try_despawn();
        }
        return;
    }

    if !freeze.spawned {
        freeze.spawned = true;
        commands
            .spawn((
                MatchEndOverlay,
                StateScoped(AppState::InGame),
                // Above the menus (50) — nothing should be reachable under it.
                GlobalZIndex(60),
                Pickable::IGNORE,
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(wash_color(0.0)),
            ))
            .with_children(|root| {
                root.spawn((
                    MatchEndText,
                    Text::new(if freeze.victory { "VICTORY" } else { "DEFEAT" }),
                    TextFont {
                        font: asset_server.load(crate::HUD_FONT),
                        font_size: TEXT_SIZE * TEXT_START_SCALE,
                        ..default()
                    },
                    TextColor(headline_color(freeze.victory, 0.0)),
                    TextLayout::new_with_justify(JustifyText::Center),
                    Pickable::IGNORE,
                ));
            });
        return;
    }

    for (_, mut bg) in &mut wash {
        bg.0 = wash_color(freeze.elapsed);
    }
    for (mut font, mut color) in &mut text {
        font.font_size = TEXT_SIZE * headline_scale(freeze.elapsed);
        color.0 = headline_color(freeze.victory, freeze.elapsed);
    }
}

// --- the animation, as pure functions of time -----------------------------

/// The full-screen colour `t` seconds in: black wash → quick white flash →
/// back to the black wash.
fn wash_color(t: f32) -> Color {
    let black = Vec4::new(0.0, 0.0, 0.0, WASH_ALPHA);
    let white = Vec4::new(1.0, 1.0, 1.0, FLASH_ALPHA);
    let v = if t <= 0.0 {
        black
    } else if t < FLASH_PEAK_SECS {
        black.lerp(white, t / FLASH_PEAK_SECS)
    } else if t < FLASH_END_SECS {
        white.lerp(black, (t - FLASH_PEAK_SECS) / (FLASH_END_SECS - FLASH_PEAK_SECS))
    } else {
        black
    };
    Color::srgba(v.x, v.y, v.z, v.w)
}

/// Ease-out with an overshoot: reaches 1 at `p = 1` after swinging past it.
fn ease_out_back(p: f32) -> f32 {
    const C1: f32 = 1.701_58;
    const C3: f32 = C1 + 1.0;
    let q = p.clamp(0.0, 1.0) - 1.0;
    1.0 + C3 * q * q * q + C1 * q * q
}

/// How big the headline is, relative to its resting size, `t` seconds in: it
/// slams down from [`TEXT_START_SCALE`]× (slightly past its size and back —
/// the "pop"), then keeps swelling a little.
fn headline_scale(t: f32) -> f32 {
    let p = ((t - TEXT_START_SECS) / TEXT_POP_SECS).clamp(0.0, 1.0);
    let pop = TEXT_START_SCALE + (1.0 - TEXT_START_SCALE) * ease_out_back(p);
    let drift = ((t - TEXT_START_SECS - TEXT_POP_SECS)
        / (shared::MATCH_END_FREEZE_SECS - TEXT_START_SECS - TEXT_POP_SECS))
        .clamp(0.0, 1.0);
    pop * (1.0 + TEXT_DRIFT * drift)
}

/// The headline's colour: green for a win, red otherwise, fading in.
fn headline_color(victory: bool, t: f32) -> Color {
    let alpha = ((t - TEXT_START_SECS) / TEXT_FADE_SECS).clamp(0.0, 1.0);
    let base = if victory { VICTORY } else { DEFEAT };
    base.with_alpha(alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(c: Color) -> f32 {
        c.to_srgba().alpha
    }

    #[test]
    fn the_wash_is_black_then_flashes_white_and_returns_to_black() {
        let start = wash_color(0.0).to_srgba();
        assert_eq!((start.red, start.alpha), (0.0, WASH_ALPHA));
        let peak = wash_color(FLASH_PEAK_SECS).to_srgba();
        assert!(peak.red > 0.99 && (peak.alpha - FLASH_ALPHA).abs() < 1e-4);
        let mid = wash_color(FLASH_PEAK_SECS * 0.5).to_srgba();
        assert!(mid.red > 0.0 && mid.red < 1.0);
        for t in [FLASH_END_SECS, 1.0, shared::MATCH_END_FREEZE_SECS] {
            let c = wash_color(t).to_srgba();
            assert_eq!((c.red, c.alpha), (0.0, WASH_ALPHA), "at {t}");
        }
    }

    #[test]
    fn the_headline_is_hidden_until_it_pops_in_then_settles_at_full_size() {
        assert_eq!(alpha(headline_color(true, TEXT_START_SECS - 0.01)), 0.0);
        assert!(alpha(headline_color(true, TEXT_START_SECS + TEXT_FADE_SECS)) > 0.999);
        // Starts huge...
        assert!((headline_scale(0.0) - TEXT_START_SCALE).abs() < 1e-4);
        // ...swings past its size (the pop)...
        let low = (0..40)
            .map(|i| headline_scale(TEXT_START_SECS + TEXT_POP_SECS * i as f32 / 40.0))
            .fold(f32::MAX, f32::min);
        assert!(low < 1.0, "never overshot: {low}");
        // ...and ends just above its resting size.
        let end = headline_scale(shared::MATCH_END_FREEZE_SECS);
        assert!(end > 1.0 && end <= 1.0 + TEXT_DRIFT + 1e-4, "{end}");
    }

    #[test]
    fn a_win_is_green_and_a_loss_is_red() {
        let win = headline_color(true, 1.0).to_srgba();
        let loss = headline_color(false, 1.0).to_srgba();
        assert!(win.green > win.red && loss.red > loss.green);
    }

    #[test]
    fn the_freeze_holds_input_off_for_exactly_the_freeze_then_only_the_overlay_ends() {
        let mut f = MatchEndFreeze::default();
        assert!(!f.active && !f.overlay_running());
        f.start(true);
        assert!(f.active && f.overlay_running());
        f.elapsed = shared::MATCH_END_FREEZE_SECS + 0.01;
        assert!(f.active, "input stays off until the results screen");
        assert!(!f.overlay_running());
    }
}
