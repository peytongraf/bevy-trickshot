//! The compass along the very top of the screen, Call of Duty's: a strip of
//! bearings every 15° — the numbers, the in-between points (NE, SE, SW,
//! NW), the cardinal points bigger and bolder, north in yellow — each with
//! a little tick over it, sliding past as we turn, fading out toward the
//! strip's ends; and our heading in the middle, under a tick of its own,
//! the bearings either side of it dimming out of its way.
//!
//! The heading is the world camera's facing across the ground — north is
//! -Z, east +X, clockwise from north — so it's whoever we're watching's
//! when spectating. It shows in every mode, with the rest of the HUD
//! ([`menu::HudElement`]: hidden outside a game, behind menus and during
//! kill cams).

use bevy::prelude::*;

use crate::{menu, WorldModelCamera, HUD_FONT};

/// How tall (logical px) the compass is, from the top of the screen — the
/// top-centre readouts (the match timer, the pre-game countdown) sit under
/// it.
pub(crate) const COMPASS_HEIGHT: f32 = 50.0;
/// How wide (logical px) the strip is...
const WIDTH: f32 = 560.0;
/// ...and how many degrees either side of our heading it shows.
const HALF_SPAN_DEG: f32 = 90.0;
/// Every bearing's slot (px wide), its label centred in it.
const SLOT_WIDTH: f32 = 60.0;
/// Bearings this close (degrees) to our heading dim out of its way, down to
/// [`NEAR_HEADING_ALPHA`]...
const CLEAR_OF_HEADING_DEG: f32 = 14.0;
const NEAR_HEADING_ALPHA: f32 = 0.0;
/// ...and from this far out (fraction of [`HALF_SPAN_DEG`]) they fade
/// toward the ends.
const EDGE_FADE_FROM: f32 = 0.55;

const NORTH_YELLOW: Color = Color::srgb(0.96, 0.92, 0.25);
/// A plain number's whiteness (a little softer than the points).
const NUMBER_ALPHA: f32 = 0.85;

pub(crate) struct CompassPlugin;

impl Plugin for CompassPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_compass).add_systems(Update, update_compass);
    }
}

/// One bearing's slot on the strip (its tick and label are children).
#[derive(Component)]
struct CompassMark(f32);

/// A bearing's label, and its colour at full strength.
#[derive(Component)]
struct MarkLabel {
    deg: f32,
    color: Color,
}

/// A bearing's tick.
#[derive(Component)]
struct MarkTick(f32);

/// Our heading, in the middle.
#[derive(Component)]
struct CompassHeading;

/// What a bearing's labelled, how big, and in what colour.
fn label_of(deg: u32) -> (String, f32, Color) {
    let white = Color::WHITE;
    match deg {
        0 => ("N".into(), 28.0, NORTH_YELLOW),
        90 => ("E".into(), 28.0, white),
        180 => ("S".into(), 28.0, white),
        270 => ("W".into(), 28.0, white),
        45 => ("NE".into(), 20.0, white),
        135 => ("SE".into(), 20.0, white),
        225 => ("SW".into(), 20.0, white),
        315 => ("NW".into(), 20.0, white),
        n => (n.to_string(), 20.0, white.with_alpha(NUMBER_ALPHA)),
    }
}

fn shadow() -> TextShadow {
    TextShadow {
        offset: Vec2::splat(1.5),
        color: Color::srgba(0.0, 0.0, 0.0, 0.7),
    }
}

fn spawn_compass(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    commands
        .spawn((
            menu::HudElement,
            GlobalZIndex(4),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                height: Val::Px(COMPASS_HEIGHT),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            row.spawn((
                Node {
                    width: Val::Px(WIDTH),
                    height: Val::Percent(100.0),
                    overflow: Overflow::clip(),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|strip| {
                for deg in (0..360).step_by(15) {
                    let (text, size, color) = label_of(deg);
                    let deg = deg as f32;
                    strip
                        .spawn((
                            CompassMark(deg),
                            Node {
                                position_type: PositionType::Absolute,
                                top: Val::Px(4.0),
                                width: Val::Px(SLOT_WIDTH),
                                flex_direction: FlexDirection::Column,
                                align_items: AlignItems::Center,
                                row_gap: Val::Px(2.0),
                                ..default()
                            },
                            Visibility::Hidden,
                        ))
                        .with_children(|slot| {
                            slot.spawn((
                                MarkTick(deg),
                                Node {
                                    width: Val::Px(1.5),
                                    height: Val::Px(5.0),
                                    ..default()
                                },
                                BackgroundColor(Color::WHITE.with_alpha(0.6)),
                            ));
                            slot.spawn((
                                MarkLabel { deg, color },
                                Text::new(text),
                                TextFont {
                                    font: font.clone(),
                                    font_size: size,
                                    ..default()
                                },
                                TextColor(color),
                                shadow(),
                            ));
                        });
                }
                // Our heading: its tick, and the number under it.
                strip
                    .spawn(Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(0.0),
                        left: Val::Px((WIDTH - SLOT_WIDTH) * 0.5),
                        width: Val::Px(SLOT_WIDTH),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(1.0),
                        ..default()
                    })
                    .with_children(|mid| {
                        mid.spawn((
                            Node {
                                width: Val::Px(2.0),
                                height: Val::Px(8.0),
                                ..default()
                            },
                            BackgroundColor(Color::WHITE),
                        ));
                        mid.spawn((
                            CompassHeading,
                            Text::new("0"),
                            TextFont {
                                font,
                                font_size: 32.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            shadow(),
                        ));
                    });
            });
        });
}

/// Our heading (degrees clockwise from north, `0..360`) facing `forward`.
fn heading_deg(forward: Vec3) -> f32 {
    forward.x.atan2(-forward.z).to_degrees().rem_euclid(360.0)
}

/// How far (degrees, `-180..180`) `deg` is round from `heading` — positive
/// to the right.
fn off_heading(deg: f32, heading: f32) -> f32 {
    (deg - heading + 540.0).rem_euclid(360.0) - 180.0
}

/// How strongly a bearing `off` degrees from our heading shows: dimmed
/// beside the heading and fading toward the ends.
fn mark_alpha(off: f32) -> f32 {
    let edge = (off.abs() / HALF_SPAN_DEG - EDGE_FADE_FROM) / (1.0 - EDGE_FADE_FROM);
    let edge = 1.0 - edge.clamp(0.0, 1.0);
    let near = (off.abs() / CLEAR_OF_HEADING_DEG).clamp(0.0, 1.0);
    let near = NEAR_HEADING_ALPHA + (1.0 - NEAR_HEADING_ALPHA) * near * near;
    edge * edge * near
}

/// Slide the bearings past as we turn, and show our heading.
fn update_compass(
    camera: Query<&GlobalTransform, With<WorldModelCamera>>,
    mut marks: Query<(&CompassMark, &mut Node, &mut Visibility)>,
    mut labels: Query<(&MarkLabel, &mut TextColor), Without<CompassHeading>>,
    mut ticks: Query<(&MarkTick, &mut BackgroundColor)>,
    mut heading_text: Query<&mut Text, With<CompassHeading>>,
) {
    let Ok(camera) = camera.single() else { return };
    let forward = camera.forward().as_vec3();
    if Vec2::new(forward.x, forward.z).length_squared() < 1e-6 {
        return;
    }
    let heading = heading_deg(forward);
    let px_per_deg = WIDTH * 0.5 / HALF_SPAN_DEG;
    for (mark, mut node, mut vis) in &mut marks {
        let off = off_heading(mark.0, heading);
        let shown = off.abs() < HALF_SPAN_DEG;
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        if shown {
            let left = Val::Px(WIDTH * 0.5 + off * px_per_deg - SLOT_WIDTH * 0.5);
            if node.left != left {
                node.left = left;
            }
        }
    }
    for (label, mut color) in &mut labels {
        let a = mark_alpha(off_heading(label.deg, heading));
        color.set_if_neq(TextColor(label.color.with_alpha(label.color.alpha() * a)));
    }
    for (tick, mut color) in &mut ticks {
        let a = mark_alpha(off_heading(tick.0, heading));
        color.set_if_neq(BackgroundColor(Color::WHITE.with_alpha(0.6 * a)));
    }
    if let Ok(mut text) = heading_text.single_mut() {
        // (Rounded, 360 back round to 0.)
        let wanted = ((heading.round() as u32) % 360).to_string();
        if text.0 != wanted {
            text.0 = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn north_is_minus_z_and_east_plus_x() {
        assert!(heading_deg(Vec3::NEG_Z).abs() < 1e-3);
        assert!((heading_deg(Vec3::X) - 90.0).abs() < 1e-3);
        assert!((heading_deg(Vec3::Z) - 180.0).abs() < 1e-3);
        assert!((heading_deg(Vec3::NEG_X) - 270.0).abs() < 1e-3);
    }

    #[test]
    fn bearings_either_side_of_north_wrap_round() {
        assert!((off_heading(15.0, 350.0) - 25.0).abs() < 1e-3);
        assert!((off_heading(345.0, 10.0) + 25.0).abs() < 1e-3);
        assert_eq!(mark_alpha(0.0), 0.0);
        assert_eq!(mark_alpha(HALF_SPAN_DEG), 0.0);
        assert_eq!(mark_alpha(30.0), 1.0);
    }
}
