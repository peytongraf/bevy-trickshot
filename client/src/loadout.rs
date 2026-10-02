//! The player's loadout pick (`Settings::primary`) and how it reaches the
//! game. The Loadout screen (`menu::build_loadout`) and the lobby room's
//! LOADOUT section (`lobby_ui`) only ever change the setting; [`push_loadout`]
//! sends it to whatever lobby we're in (`shared::SetLoadout`), and the server
//! decides when it lands in our hands (`shared::LobbyMember::primary` — see
//! `weapons::sync_primary_model`):
//!
//! * between games — any mode, any time;
//! * a `Zombies` game in progress — locked;
//! * a `FreeForAll` game in progress — at once if we only just spawned and
//!   haven't fired, otherwise on our next spawn, which [`loadout_notice`]
//!   tells us, Call of Duty style.

use bevy::prelude::*;
use lightyear::prelude::*;
use shared::weapon::WeaponId;
use shared::GameMode;

use crate::net::GameClient;
use crate::settings::Settings;
use crate::{AppState, HUD_FONT};

pub(crate) struct LoadoutPlugin;

impl Plugin for LoadoutPlugin {
    fn build(&self, app: &mut App) {
        // Not gated on `AppState`: the lobby room needs it as much as a game.
        app.add_systems(Update, push_loadout).add_systems(
            Update,
            (loadout_notice, update_loadout_notice)
                .chain()
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// Where we stand, loadout-wise — what the Loadout screen and the lobby room
/// show.
#[derive(Clone, Copy, Default)]
pub(crate) struct LoadoutCtx {
    /// Our lobby's mode, if we're in one.
    pub(crate) mode: Option<GameMode>,
    /// That lobby's game is in progress.
    pub(crate) started: bool,
    /// The gun in our hands this life, while a game with a loadout is on.
    pub(crate) carrying: Option<WeaponId>,
}

impl LoadoutCtx {
    pub(crate) fn new(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&shared::Lobby>) -> Self {
        let me = local.iter().next().map(|l| l.0);
        let Some(lobby) = lobbies.iter().find(|l| me.is_some_and(|me| l.has(me))) else {
            return Self::default();
        };
        let carrying = (lobby.started && lobby.mode.has_loadout())
            .then(|| lobby.members.iter().find(|m| Some(m.peer) == me).map(|m| m.primary))
            .flatten();
        Self {
            mode: Some(lobby.mode),
            started: lobby.started,
            carrying,
        }
    }

    /// The pick can't change right now — a `Zombies` game is on.
    pub(crate) fn locked(&self) -> bool {
        self.started && self.mode == Some(GameMode::Zombies)
    }
}

/// Send our pick to the lobby we're in whenever it differs from what the
/// lobby has for us — once per pick, so a refused one (a `Zombies` game
/// started) isn't resent every frame.
fn push_loadout(
    settings: Res<Settings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut sender: Query<&mut TriggerSender<shared::SetLoadout>, With<GameClient>>,
    mut sent: Local<Option<WeaponId>>,
) {
    let me = local.iter().next().map(|l| l.0);
    let Some(lobby) = lobbies.iter().find(|l| me.is_some_and(|me| l.has(me))) else {
        *sent = None;
        return;
    };
    let Some(member) = lobby.members.iter().find(|m| Some(m.peer) == me) else {
        *sent = None;
        return;
    };
    let pick = settings.primary;
    if member.loadout == pick {
        *sent = None;
        return;
    }
    if *sent == Some(pick) || (lobby.started && lobby.mode == GameMode::Zombies) {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::SetLoadout { weapon: pick });
        *sent = Some(pick);
    }
}

/// The "changes next spawn" message.
#[derive(Component)]
struct LoadoutNotice {
    age: f32,
}

const NOTICE_TEXT: &str = "Loadout will change on next spawn";
const NOTICE_HOLD: f32 = 2.5;
const NOTICE_TTL: f32 = 3.3;

/// In a `FreeForAll` game: the moment the lobby takes a new pick from us
/// that *isn't* what we're carrying (too long since spawning, or already
/// fired), say it'll be ours next spawn. Drawn above the menus too, since
/// that's where the change is made.
fn loadout_notice(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    existing: Query<Entity, With<LoadoutNotice>>,
    mut seen: Local<Option<WeaponId>>,
) {
    let me = local.iter().next().map(|l| l.0);
    let member = lobbies
        .iter()
        .find(|l| me.is_some_and(|me| l.has(me)) && l.started && l.mode == GameMode::FreeForAll)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me));
    let Some(member) = member else {
        *seen = None;
        return;
    };
    let before = seen.replace(member.loadout);
    if before.is_none_or(|b| b == member.loadout) || member.primary == member.loadout {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn();
    }
    commands
        .spawn((
            LoadoutNotice { age: 0.0 },
            StateScoped(AppState::InGame),
            // Over the in-game menus (`menu`'s pages sit at 50).
            GlobalZIndex(60),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                top: Val::Percent(64.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_child((
            Text::new(NOTICE_TEXT),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: 30.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
}

/// Hold the message, then fade it out and despawn.
fn update_loadout_notice(
    time: Res<Time>,
    mut notices: Query<(Entity, &mut LoadoutNotice, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut commands: Commands,
) {
    for (entity, mut notice, children) in &mut notices {
        notice.age += time.delta_secs();
        if notice.age >= NOTICE_TTL {
            commands.entity(entity).despawn();
            continue;
        }
        let a = if notice.age < NOTICE_HOLD {
            1.0
        } else {
            1.0 - (notice.age - NOTICE_HOLD) / (NOTICE_TTL - NOTICE_HOLD)
        };
        for child in children {
            if let Ok(mut tc) = texts.get_mut(*child) {
                tc.0 = Color::WHITE.with_alpha(a.clamp(0.0, 1.0));
            }
        }
    }
}

/// Rough 0..1 ratings for the Loadout screen's stat bars, Call of Duty
/// style — derived from the weapon's real numbers where there is one.
pub(crate) fn weapon_stats(weapon: WeaponId) -> [(&'static str, f32); 5] {
    let spec = weapon.spec();
    let (fire_rate, accuracy, mobility) = match weapon {
        WeaponId::Sniper => (0.12, 0.92, 0.42),
        WeaponId::Marksman => (0.35, 0.8, 0.55),
        WeaponId::Ak74 => (0.78, 0.58, 0.72),
    };
    [
        ("DAMAGE", (spec.base_damage / 200.0).clamp(0.05, 1.0)),
        ("RANGE", (spec.max_range / 350.0).clamp(0.05, 1.0)),
        ("FIRE RATE", fire_rate),
        ("ACCURACY", accuracy),
        ("MOBILITY", mobility),
    ]
}
