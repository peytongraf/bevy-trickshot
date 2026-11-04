//! The easter egg's cut scene: from the ritual's start ([`EggStep::Ritual`])
//! through it going wrong ([`EggStep::Unleashed`]) and the Hierophant waking
//! ([`EggStep::Awakening`]), until the fight's on. While it runs:
//!
//! * every player's held still, no weapon up (`menu::game_active`,
//!   `menu::weapon_active`, `revive::apply_down_view`);
//! * the view's turned for them ([`aim_view`]) — onto the ritual altar as
//!   the dagger, the vial and their blood go in and it spills over, then
//!   onto the sarcophagus as he speaks, and
//!   onto him as he wakes and stands up;
//! * black bars slide in over the top and bottom of the screen
//!   ([`CinemaBar`]), and back out once it's over — the rest of the HUD
//!   fading out and back in with them (`hud::HudFade`; the ritual's caption
//!   and the radio stay).
//!
//! Watching a teammate (bled out, `revive::Spectate`) the view's theirs, so
//! it's left alone then. The bars' slide ([`EggCinematic`]) is reset on
//! leaving the game, and the bars themselves are `StateScoped(InGame)` —
//! nothing carries into the next.

use bevy::prelude::*;
use lightyear::prelude::LocalId;
use shared::easter_egg::{EggStep, WAKE_SNAP_AT};
use shared::Lobby;

use crate::net::GameClient;
use crate::player::PlayerHead;
use crate::{AppState, Player};

use super::{egg_game, EggRitual, EggWake};

pub(super) struct CinematicPlugin;

impl Plugin for CinematicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggCinematic>()
            .add_systems(OnEnter(AppState::InGame), spawn_bars)
            .add_systems(OnExit(AppState::InGame), |mut c: ResMut<EggCinematic>| *c = default())
            .add_systems(
                Update,
                (track_cinematic, aim_view, slide_bars)
                    .chain()
                    .after(super::track_ritual)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// How tall each bar is at full (share of the screen's height).
const BAR_HEIGHT_PCT: f32 = 11.0;
/// How long (s) the bars take to slide in or out.
const BAR_SLIDE_SECS: f32 = 0.8;
/// How fast the view turns onto what it's watching (the share of the way
/// left it closes, per second — eased, never a snap).
const TURN_RATE: f32 = 2.2;
/// The furthest (rad) it tips up or down — inside mouse look's own limit.
const PITCH_LIMIT: f32 = 1.3;

/// Whether the cut scene's on, and how far in the bars are (0..=1).
#[derive(Resource, Default)]
pub(crate) struct EggCinematic {
    on: bool,
    bars: f32,
}

impl EggCinematic {
    /// Held still, no weapon: the cut scene's playing.
    pub(crate) fn locked(&self) -> bool {
        self.on
    }
}

/// One of the two black bars, top and bottom.
#[derive(Component)]
struct CinemaBar;

fn spawn_bars(mut commands: Commands) {
    for top in [true, false] {
        commands.spawn((
            StateScoped(AppState::InGame),
            CinemaBar,
            crate::hud::KeepThroughHudFade,
            // (Over the HUD, under the menus and the egg's caption — which
            // sits mid-screen, clear of them anyway.)
            GlobalZIndex(4),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(0.0),
                top: if top { Val::Px(0.0) } else { Val::Auto },
                bottom: if top { Val::Auto } else { Val::Px(0.0) },
                ..default()
            },
            BackgroundColor(Color::BLACK),
            Visibility::Hidden,
        ));
    }
}

fn track_cinematic(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut cinematic: ResMut<EggCinematic>,
) {
    let on = egg_game(&local, &lobbies).is_some_and(|l| {
        matches!(l.easter_egg.step, EggStep::Ritual | EggStep::Unleashed | EggStep::Awakening)
    });
    cinematic.on = on;
    let step = time.delta_secs() / BAR_SLIDE_SECS;
    cinematic.bars = if on { (cinematic.bars + step).min(1.0) } else { (cinematic.bars - step).max(0.0) };
}

/// Where the view's watching right now, if anywhere.
fn watching(lobby: &Lobby, ritual: &EggRitual, wake: &EggWake) -> Option<Vec3> {
    let layout = shared::level::layout(lobby.map);
    let sarcophagus = layout.sarcophagus?;
    // (Over the body lying in it.)
    let in_it = sarcophagus.pos + Vec3::Y * 0.6 * sarcophagus.scale;
    match lobby.easter_egg.step {
        EggStep::Ritual => {
            // The altar's top — the dagger, the pour, the basin filling and
            // spilling over — till he speaks from the sarcophagus.
            let altar = layout
                .ritual_altar
                .map(|a| a.pos + Vec3::Y * (shared::easter_egg::RITUAL_ALTAR_TOP * 0.9) * a.scale)
                .or(layout.ritual_area.map(|a| a.at.pos + Vec3::Y * 0.3));
            let Some(altar) = altar else { return Some(in_it) };
            if ritual.t.unwrap_or(0.0) < shared::easter_egg::RITUAL_VOICE_AT_SECS {
                Some(altar)
            } else {
                Some(in_it)
            }
        }
        EggStep::Unleashed => Some(in_it),
        EggStep::Awakening => {
            if wake.t().unwrap_or(0.0) < WAKE_SNAP_AT {
                Some(in_it)
            } else {
                // Up on his feet: his face.
                let (feet, _) = shared::easter_egg::hierophant_start(lobby.map)?;
                Some(feet + Vec3::Y * 1.6)
            }
        }
        _ => None,
    }
}

/// The view turned onto what the cut scene's watching.
#[allow(clippy::too_many_arguments)]
fn aim_view(
    time: Res<Time>,
    cinematic: Res<EggCinematic>,
    ritual: Res<EggRitual>,
    wake: Res<EggWake>,
    spectate: Res<crate::revive::Spectate>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Option<Single<&mut Transform, (With<Player>, Without<PlayerHead>)>>,
    head: Option<Single<&mut Transform, (With<PlayerHead>, Without<Player>)>>,
) {
    if !cinematic.on || spectate.active() {
        return;
    }
    let (Some(mut player), Some(mut head)) = (player, head) else { return };
    let Some(lobby) = egg_game(&local, &lobbies) else { return };
    let Some(target) = watching(lobby, &ritual, &wake) else { return };
    let d = target - player.translation;
    let flat = d.with_y(0.0).length();
    if flat < 0.2 {
        return;
    }
    let yaw = f32::atan2(-d.x, -d.z);
    let pitch = f32::atan2(d.y, flat).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let k = 1.0 - (-TURN_RATE * time.delta_secs()).exp();
    player.rotation = player.rotation.slerp(Quat::from_rotation_y(yaw), k);
    head.rotation = head.rotation.slerp(Quat::from_rotation_x(pitch), k);
}

/// The bars slid in or out, and the rest of the HUD faded with them.
fn slide_bars(
    cinematic: Res<EggCinematic>,
    mut hud_fade: ResMut<crate::hud::HudFade>,
    mut bars: Query<(&mut Node, &mut Visibility), With<CinemaBar>>,
) {
    // (Eased in and out.)
    let x = cinematic.bars;
    let eased = x * x * (3.0 - 2.0 * x);
    let shown = 1.0 - eased;
    if hud_fade.0 != shown {
        hud_fade.0 = shown;
    }
    let h = BAR_HEIGHT_PCT * eased;
    for (mut node, mut vis) in &mut bars {
        vis.set_if_neq(if h > 0.0 { Visibility::Inherited } else { Visibility::Hidden });
        if node.height != Val::Percent(h) {
            node.height = Val::Percent(h);
        }
    }
}
