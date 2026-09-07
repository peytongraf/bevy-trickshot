//! Lobby lifecycle: create / join / leave / start, all driven by client
//! triggers. Lobbies are plain entities carrying a replicated [`shared::Lobby`]
//! component (replicated to everyone so every client's browser is live).
//!
//! `StartGame` (from the leader) flips `started` and spawns one replicated
//! player entity per member, scoped with `NetworkTarget::Only` to that lobby's
//! members so lobbies don't see each other.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{
    CreateLobby, GameChannel, JoinLobby, LeaveLobby, Lobby, LobbyError, LobbyMember, PlayerId,
    PlayerInput, PlayerName, PlayerPose, StartGame,
};

/// Tags a replicated in-world player entity with the lobby it belongs to, so the
/// session's players can be found again (rescoping / cleanup).
#[derive(Component)]
pub struct LobbyPlayer {
    pub lobby: Entity,
}

/// Max players per lobby.
const MAX_MEMBERS: usize = 8;

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_create)
            .add_observer(on_join)
            .add_observer(on_leave)
            .add_observer(on_start)
            .add_observer(on_disconnect);
    }
}

// --- helpers --------------------------------------------------------------

/// Remove `peer` from every lobby (optionally skipping `except`), promoting a
/// new leader or despawning the lobby as needed. In-world player entities for
/// the departing peer (and for the whole lobby, if it empties) are despawned
/// too — `get_entity` guards against a double-despawn when `ControlledBy`
/// already cleaned one up on disconnect.
fn remove_peer(
    peer: PeerId,
    except: Option<Entity>,
    lobbies: &mut Query<(Entity, &mut Lobby)>,
    players: &Query<(Entity, &PlayerId, &LobbyPlayer)>,
    commands: &mut Commands,
) {
    for (entity, mut lobby) in lobbies.iter_mut() {
        if Some(entity) == except || !lobby.has(peer) {
            continue;
        }
        lobby.members.retain(|m| m.peer != peer);

        let despawn = |commands: &mut Commands, e: Entity| {
            if let Ok(mut ec) = commands.get_entity(e) {
                ec.despawn();
            }
        };

        if lobby.members.is_empty() {
            for (pe, _, _) in players.iter().filter(|(_, _, lp)| lp.lobby == entity) {
                despawn(commands, pe);
            }
            despawn(commands, entity);
            info!("lobby {entity:?} is empty — despawned");
        } else {
            for (pe, pid, lp) in players.iter() {
                if lp.lobby == entity && pid.0 == peer {
                    despawn(commands, pe);
                }
            }
            if lobby.leader == peer {
                lobby.leader = lobby.members[0].peer;
                info!("lobby {entity:?}: leader left, promoted {:?}", lobby.leader);
            }
        }
    }
}

fn reject(sender: &mut ServerMultiMessageSender, server: &Server, peer: PeerId, reason: &str) {
    let msg = LobbyError {
        reason: reason.to_string(),
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(peer)) {
        error!("failed to send LobbyError to {peer:?}: {e:?}");
    }
}

// --- observers ----------------------------------------------------------

fn on_create(
    trigger: Trigger<RemoteTrigger<CreateLobby>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let ev = &trigger.trigger;

    // Creating implies leaving whatever lobby they were in.
    remove_peer(peer, None, &mut lobbies, &players, &mut commands);

    let name = if ev.name.trim().is_empty() {
        format!("{}'s lobby", ev.player_name)
    } else {
        ev.name.trim().to_string()
    };

    let entity = commands
        .spawn((
            Name::from("Lobby"),
            Lobby {
                name,
                leader: peer,
                started: false,
                members: vec![LobbyMember {
                    peer,
                    name: ev.player_name.clone(),
                    score: 0,
                }],
            },
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();
    info!("{peer:?} created lobby {entity:?}");
}

fn on_join(
    trigger: Trigger<RemoteTrigger<JoinLobby>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let target = trigger.trigger.lobby;
    let player_name = trigger.trigger.player_name.clone();

    // Validate against the target lobby without holding a mutable borrow.
    let Ok((_, lobby)) = lobbies.get(target) else {
        reject(&mut sender, &server, peer, "That lobby no longer exists.");
        return;
    };
    if lobby.started {
        reject(&mut sender, &server, peer, "That game has already started.");
        return;
    }
    if lobby.members.len() >= MAX_MEMBERS && !lobby.has(peer) {
        reject(&mut sender, &server, peer, "That lobby is full.");
        return;
    }

    remove_peer(peer, Some(target), &mut lobbies, &players, &mut commands);

    let Ok((_, mut lobby)) = lobbies.get_mut(target) else {
        return;
    };
    if !lobby.has(peer) {
        lobby.members.push(LobbyMember {
            peer,
            name: player_name,
            score: 0,
        });
        info!("{peer:?} joined lobby {target:?}");
    }
}

fn on_leave(
    trigger: Trigger<RemoteTrigger<LeaveLobby>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    mut commands: Commands,
) {
    remove_peer(trigger.from, None, &mut lobbies, &players, &mut commands);
    info!("{:?} left their lobby", trigger.from);
}

fn on_start(
    trigger: Trigger<RemoteTrigger<StartGame>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    clients: Query<(Entity, &RemoteId), With<ClientOf>>,
    mut commands: Commands,
) {
    let peer = trigger.from;
    let Some((lobby_entity, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.leader == peer && !l.started)
    else {
        return;
    };

    lobby.started = true;
    let members: Vec<shared::LobbyMember> = lobby.members.clone();
    info!(
        "lobby {lobby_entity:?} started by {peer:?} with {} member(s)",
        members.len()
    );

    let all: Vec<PeerId> = members.iter().map(|m| m.peer).collect();
    for member in &members {
        let Some((owner, _)) = clients.iter().find(|(_, rid)| rid.0 == member.peer) else {
            warn!(
                "no client link for {:?}; skipping player spawn",
                member.peer
            );
            continue;
        };
        let others: Vec<PeerId> = all.iter().copied().filter(|p| *p != member.peer).collect();

        let entity = commands
            .spawn((
                Name::from("Player"),
                LobbyPlayer {
                    lobby: lobby_entity,
                },
                PlayerId(member.peer),
                PlayerName(member.name.clone()),
                PlayerPose::default(),
                ActionState::<PlayerInput>::default(),
                Replicate::to_clients(NetworkTarget::Only(all.clone())),
                PredictionTarget::to_clients(NetworkTarget::Single(member.peer)),
                InterpolationTarget::to_clients(NetworkTarget::Only(others)),
                ControlledBy {
                    owner,
                    lifetime: Lifetime::SessionBased,
                },
            ))
            .id();
        info!("  spawned player {entity:?} for {:?}", member.peer);
    }
}

fn on_disconnect(
    trigger: Trigger<OnAdd, Disconnected>,
    clients: Query<&RemoteId, With<ClientOf>>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer)>,
    mut commands: Commands,
) {
    let Ok(peer) = clients.get(trigger.target()) else {
        return;
    };
    remove_peer(peer.0, None, &mut lobbies, &players, &mut commands);
}
