//! `Zombies` damage numbers, Call of Duty style: every hit we land on a
//! zombie (any weapon — the server's [`shared::ZombieDamaged`]) pops its
//! damage up where it landed, which flies off in a random direction and
//! fades out quickly. White, or yellow for a critical (a headshot / knife).
//!
//! Drawn on the HUD, pinned to the hit's spot projected onto the screen each
//! frame, so it always faces us. `StateScoped(InGame)`, and each lives well
//! under a second — nothing carries into the next game.

use bevy::prelude::*;
use lightyear::prelude::MessageReceiver;

use crate::hud::{SCORE_WHITE, SCORE_YELLOW};
use crate::util::rand01;
use crate::{killcam, menu, AppState, WorldModelCamera, HUD_FONT};

/// How long (s) a number is up.
const LIFE_SECS: f32 = 0.75;
/// Fraction of its life it holds full opacity before fading.
const HOLD: f32 = 0.35;
/// How far (px) it flies from the hit.
const TRAVEL_PX: f32 = 70.0;
/// Font size (px) up close...
const FONT_SIZE: f32 = 30.0;
/// ...shrinking with distance down to this fraction of it...
const FAR_SCALE: f32 = 0.6;
/// ...from this far (m) out.
const NEAR_DIST: f32 = 8.0;
/// It starts this much bigger and snaps down to size over [`POP_SECS`].
const POP: f32 = 0.4;
const POP_SECS: f32 = 0.08;

pub(crate) struct DamageNumbersPlugin;

impl Plugin for DamageNumbersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            spawn_damage_numbers.run_if(in_state(AppState::InGame)),
        )
        .add_systems(
            PostUpdate,
            update_damage_numbers
                .before(bevy::ui::UiSystem::Layout)
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// One floating number (a zero-size node its text is centred on).
#[derive(Component)]
struct DamageNumber {
    /// Where the hit landed.
    world: Vec3,
    /// Which way (screen space, unit) it flies.
    dir: Vec2,
    age: f32,
    color: Color,
}

fn spawn_damage_numbers(
    mut receivers: Query<&mut MessageReceiver<shared::ZombieDamaged>>,
    asset_server: Res<AssetServer>,
    mut seed: Local<u32>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            *seed = seed.wrapping_add(1);
            let angle = rand01(seed.wrapping_mul(2_654_435_761)) * std::f32::consts::TAU;
            let color = if msg.critical { SCORE_YELLOW } else { SCORE_WHITE };
            commands
                .spawn((
                    DamageNumber {
                        world: Vec3::from_array(msg.point),
                        dir: Vec2::from_angle(angle),
                        age: 0.0,
                        color,
                    },
                    StateScoped(AppState::InGame),
                    // Under the score popup (9).
                    GlobalZIndex(8),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(0.0),
                        height: Val::Px(0.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    // Placed (and shown) by `update_damage_numbers`.
                    Visibility::Hidden,
                ))
                .with_child((
                    Text::new(msg.damage.to_string()),
                    TextFont {
                        font: asset_server.load(HUD_FONT),
                        font_size: FONT_SIZE,
                        ..default()
                    },
                    TextColor(color),
                    TextShadow {
                        offset: Vec2::splat(1.5),
                        color: Color::srgba(0.0, 0.0, 0.0, 0.75),
                    },
                    TextLayout::new_with_no_wrap(),
                    Node {
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
        }
    }
}

#[allow(clippy::type_complexity)]
fn update_damage_numbers(
    time: Res<Time>,
    (menu, active_killcam): (Res<menu::Menu>, Res<killcam::ActiveKillCam>),
    camera: Single<(&Camera, &GlobalTransform), With<WorldModelCamera>>,
    mut numbers: Query<(Entity, &mut DamageNumber, &mut Node, &mut Visibility, &Children)>,
    mut texts: Query<(&mut TextColor, &mut TextFont, &mut TextShadow)>,
    mut commands: Commands,
) {
    let (cam, cam_gt) = camera.into_inner();
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    let dt = time.delta_secs();

    for (entity, mut number, mut node, mut vis, children) in &mut numbers {
        number.age += dt;
        let t = number.age / LIFE_SECS;
        if t >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        // `world_to_viewport` fails for points behind the camera.
        let screen = hud_up
            .then(|| cam.world_to_viewport(cam_gt, number.world).ok())
            .flatten();
        let Some(screen) = screen else {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        };
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }

        // Fast out, easing to a stop.
        let travel = 1.0 - (1.0 - t).powi(3);
        let at = screen + number.dir * TRAVEL_PX * travel;
        node.left = Val::Px(at.x);
        node.top = Val::Px(at.y);

        let dist = cam_gt.translation().distance(number.world);
        let scale = (NEAR_DIST / dist.max(0.01)).clamp(FAR_SCALE, 1.0);
        let pop = 1.0 + POP * (1.0 - number.age / POP_SECS).max(0.0);
        let alpha = if t < HOLD { 1.0 } else { 1.0 - (t - HOLD) / (1.0 - HOLD) };
        for child in children.iter() {
            if let Ok((mut color, mut font, mut shadow)) = texts.get_mut(child) {
                color.0 = number.color.with_alpha(alpha);
                shadow.color = Color::srgba(0.0, 0.0, 0.0, 0.75 * alpha);
                let size = (FONT_SIZE * scale * pop).round();
                if font.font_size != size {
                    font.font_size = size;
                }
            }
        }
    }
}
