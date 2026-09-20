//! The centre-screen crosshair overlay: a plain dot for the sniper, a
//! four-tick reticle while the throwing knife is held.

use bevy::prelude::*;

use crate::menu;
use crate::{scope_picture_amount, Ads, AdsTuning, CrosshairSettings};
use crate::killcam::ActiveKillCam;
use crate::{AppState, ThrowingKnife, Weapon, WeaponSlot};

/// The dead-centre white dot; `fade_crosshair` fades it out as the player aims
/// in, `update_crosshair_visibility` shows it only while the sniper is the
/// active weapon and the throwing knife isn't held.
#[derive(Component)]
pub(crate) struct CenterDot;

/// The throwing-knife reticle (four ticks with a gap in the middle); shown in
/// place of [`CenterDot`] while [`ThrowingKnife::active`] is set.
#[derive(Component)]
pub(crate) struct ThrowingKnifeCrosshair;

/// Root of the crosshair overlay (both [`CenterDot`] and
/// [`ThrowingKnifeCrosshair`] live under it). Unlike `menu::HudElement`
/// (which `hud_visibility` also hides the instant a kill cam starts), this
/// root stays available through a replay — `killcam::drive_killcam` drives
/// `Weapon::slot` and [`ThrowingKnife::active`] off the recorded samples, so
/// the replay shows the same crosshair the shooter had at each moment. It's
/// still hidden outside a live game or behind a menu, via
/// `crosshair_root_visibility`.
#[derive(Component)]
pub(crate) struct CrosshairRoot;

/// Same gating as `hud_visibility` (live game, no menu overlay) but *without*
/// the kill-cam check — a replay should still show the crosshair the shooter
/// had at each moment.
pub(crate) fn crosshair_root_visibility(
    state: Res<State<AppState>>,
    menu: Res<menu::Menu>,
    mut root: Query<&mut Visibility, With<CrosshairRoot>>,
) {
    if !(state.is_changed() || menu.is_changed()) {
        return;
    }
    let show = *state.get() == AppState::InGame && !menu.is_open();
    let want = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut root {
        if *v != want {
            *v = want;
        }
    }
}

/// A small white dot dead-centre for lining the scope up, plus the (initially
/// hidden) throwing-knife reticle shown in its place while the knife is held.
/// Both live under one `CrosshairRoot` so they share the same centring node
/// and the same visibility gating (`crosshair_root_visibility`).
pub(crate) fn setup_crosshair(mut commands: Commands) {
    let bar = |width: f32, height: f32| {
        (
            Node {
                width: Val::Px(width),
                height: Val::Px(height),
                ..default()
            },
            BackgroundColor(Color::WHITE),
            BorderRadius::MAX,
        )
    };

    commands
        .spawn((
            CrosshairRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                CenterDot,
                Node {
                    width: Val::Px(5.0),
                    height: Val::Px(5.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                BorderColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                BorderRadius::MAX,
            ));

            root.spawn((
                ThrowingKnifeCrosshair,
                Visibility::Hidden,
                Node {
                    position_type: PositionType::Absolute,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    ..default()
                },
            ))
            .with_children(|knife| {
                knife.spawn(bar(2.0, 40.0));
                knife
                    .spawn(Node {
                        column_gap: Val::Px(16.0),
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(bar(20.0, 2.0));
                        row.spawn(bar(20.0, 2.0));
                    });
                knife.spawn(bar(2.0, 40.0));
            });
        });
}

/// Fade the centre dot out as the player aims down the scope — fully gone once
/// the sight picture has come in, fully back at the hip — so it never sits over
/// the sight picture but still gives an aim reference through the raise.
pub(crate) fn fade_crosshair(
    ads: Res<Ads>,
    tuning: Res<AdsTuning>,
    crosshair: Res<CrosshairSettings>,
    dot: Single<(&mut BackgroundColor, &mut BorderColor), With<CenterDot>>,
) {
    let a = if crosshair.center_dot_always {
        1.0
    } else {
        1.0 - scope_picture_amount(ads.t, &tuning)
    };
    let (mut bg, mut border) = dot.into_inner();
    bg.0 = Color::srgba(1.0, 1.0, 1.0, a);
    border.0 = Color::srgba(0.0, 0.0, 0.0, 0.6 * a);
}

/// Pick the reticle: the centre dot only while the sniper is the active
/// weapon and the throwing knife isn't up; the throwing-knife crosshair
/// while it is ([`ThrowingKnife::crosshair_up`] — from the key press until the
/// arms have slid away again). During a kill cam it reads the recorded sample
/// under the replay's playhead directly, so the replay shows exactly the
/// crosshair the shooter had at each moment.
pub(crate) fn update_crosshair_visibility(
    weapon: Res<Weapon>,
    knife: Res<ThrowingKnife>,
    killcam: Res<ActiveKillCam>,
    mut dot: Query<&mut Visibility, (With<CenterDot>, Without<ThrowingKnifeCrosshair>)>,
    mut reticle: Query<&mut Visibility, (With<ThrowingKnifeCrosshair>, Without<CenterDot>)>,
) {
    let (sniper_active, throwing) = killcam
        .0
        .as_ref()
        .and_then(|run| run.crosshair_state())
        .unwrap_or((weapon.slot == WeaponSlot::Primary, knife.crosshair_up()));
    let dot_want = if sniper_active && !throwing {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    let reticle_want = if throwing {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if let Ok(mut v) = dot.single_mut() {
        v.set_if_neq(dot_want);
    }
    if let Ok(mut v) = reticle.single_mut() {
        v.set_if_neq(reticle_want);
    }
}
