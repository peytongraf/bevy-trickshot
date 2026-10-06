//! `Zombies` field upgrades (`shared::field_upgrade`) — the Aether Shroud:
//! building each member's charges while they're up in a running game,
//! using one on [`UseFieldUpgrade`], and running it down — publishing it all
//! onto [`shared::LobbyMember::field_upgrade`] in whole seconds.
//!
//! What it does while it's up is read straight off the lobby elsewhere
//! ([`shrouded`]): `ai::drive_bots` doesn't go after a shrouded player, and
//! `pvp` lets no damage through to them. The speed boost and the reload are
//! client-side (`client::aether_shroud`).
//!
//! Everything resets with the game: a lobby's clocks are forgotten once its
//! game isn't running (and its members' published state with them), and
//! `lobby` clears `field_upgrade` whenever a game starts or ends.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::*;
use shared::bot_players::is_bot_peer;
use shared::field_upgrade::{FieldUpgrade, FieldUpgradeClock};
use shared::{FillFieldUpgrade, GameMode, Lobby, PlayerId, PlayerPose, UseFieldUpgrade};

use crate::killcam::EndingLobbies;
use crate::pvp::PlayerCombat;
use crate::wall_buys::standing_in_game;

/// Each running `Zombies` game's members' field upgrade clocks.
#[derive(Resource, Default)]
struct FieldUpgradeClocks(HashMap<(Entity, PeerId), FieldUpgradeClock>);

pub struct FieldUpgradesPlugin;

impl Plugin for FieldUpgradesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FieldUpgradeClocks>()
            .add_observer(on_use_field_upgrade)
            .add_observer(on_fill_field_upgrade)
            .add_systems(
                FixedUpdate,
                run_field_upgrades.after(crate::pvp::apply_player_hits),
            );
    }
}

/// Whether `peer` has the Aether Shroud up in `lobby` — the zombies can't
/// see them and nothing hurts them.
pub(crate) fn shrouded(lobby: &Lobby, peer: PeerId) -> bool {
    lobby.mode == GameMode::Zombies
        && lobby.started
        && lobby.members.iter().any(|m| m.peer == peer && m.field_upgrade.active())
}

/// Build charges, run the ones in use down, and publish (only on a change,
/// so it replicates once a second).
fn run_field_upgrades(
    time: Res<Time>,
    endings: Res<EndingLobbies>,
    mut clocks: ResMut<FieldUpgradeClocks>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerCombat)>,
) {
    let dt = time.delta_secs();
    clocks.0.retain(|(e, peer), _| {
        lobbies
            .get(*e)
            .is_ok_and(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(*peer))
    });
    for (lobby_e, mut lobby) in &mut lobbies {
        let running = lobby.started && lobby.mode == GameMode::Zombies;
        // Paused, or the game's ending: everything holds still.
        let frozen = lobby.paused || endings.is_ending(lobby_e);
        let mut changed = Vec::new();
        for m in lobby.members.iter().filter(|m| !is_bot_peer(m.peer)) {
            if !running {
                if m.field_upgrade != FieldUpgrade::default() {
                    changed.push((m.peer, FieldUpgrade::default()));
                }
                continue;
            }
            let clock = clocks.0.entry((lobby_e, m.peer)).or_default();
            if !frozen {
                let up = players
                    .iter()
                    .find(|(id, _)| id.0 == m.peer)
                    .is_some_and(|(_, c)| c.alive && c.down.is_none());
                clock.tick(dt, up);
                // Down (or dead): it's over.
                if !up {
                    clock.active_left = 0.0;
                }
            }
            let published = clock.published();
            if m.field_upgrade != published {
                changed.push((m.peer, published));
            }
        }
        if changed.is_empty() {
            continue;
        }
        for (peer, state) in changed {
            if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
                if m.field_upgrade.active() && !state.active() {
                    info!("{peer:?}'s Aether Shroud wore off");
                }
                m.field_upgrade = state;
            }
        }
    }
}

/// Use a stored charge: up, in a running game, with one stored and none
/// in use.
fn on_use_field_upgrade(
    trigger: Trigger<RemoteTrigger<UseFieldUpgrade>>,
    endings: Res<EndingLobbies>,
    mut clocks: ResMut<FieldUpgradeClocks>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let Some((lobby_e, ..)) = standing_in_game(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Some(clock) = clocks.0.get_mut(&(lobby_e, peer)) else {
        return;
    };
    if !clock.try_use() {
        return;
    }
    let published = clock.published();
    if let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) {
        if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
            m.field_upgrade = published;
        }
    }
    info!("{peer:?} used their Aether Shroud");
}

/// Debug: the leader gets every charge at once.
fn on_fill_field_upgrade(
    trigger: Trigger<RemoteTrigger<FillFieldUpgrade>>,
    mut clocks: ResMut<FieldUpgradeClocks>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
) {
    let peer = trigger.from;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.leader == peer && l.started && l.mode == GameMode::Zombies)
    else {
        return;
    };
    let clock = clocks.0.entry((lobby_e, peer)).or_default();
    clock.charges = shared::field_upgrade::MAX_CHARGES;
    clock.charge = 0.0;
    let published = clock.published();
    if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
        m.field_upgrade = published;
    }
    info!("{peer:?} filled their field upgrade (debug)");
}
