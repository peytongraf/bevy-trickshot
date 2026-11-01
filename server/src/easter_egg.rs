//! Der Tempel's easter egg, the server's side (`shared::easter_egg`): it
//! owns how far the squad's got (`Lobby::easter_egg`, reset whenever a game
//! starts or ends — `crate::lobby`).
//!
//! So far (`client/notes/der-tempel-easter-egg/todo.md`):
//!
//! * the first Big Steve killed (in reach — not outside a window) drops
//!   Barrett's blood where he fell ([`drop_vial`]); anyone alive in reach
//!   picks it up ([`shared::EggInteract`]);
//! * once the power's on the society's case is open, and anyone alive at it
//!   takes the dagger;
//! * with both, every living player stands in the ritual area and accepts
//!   — then the ritual begins ([`run_altar`]). Anyone stepping out, going
//!   down or declining clears everyone's acceptance;
//! * the man on the radio remarks on each thing found, then hints at the
//!   altar once both are ([`gathered_radio`]);
//! * the ritual runs [`shared::easter_egg::RITUAL_SECS`] (held while the
//!   game's paused) — the zombies holding off meanwhile
//!   (`Lobby::bots_hold_fire`), the clients locking their players in place
//!   — then it goes wrong ([`run_ritual`]);
//! * the leader's debug panel can set any of it ([`shared::SetEasterEgg`])
//!   — without the radio.

use bevy::prelude::*;
use lightyear::prelude::*;
use shared::easter_egg::{EggAction, EggStep, VIAL_REACH};
use shared::radio::RadioLine;
use shared::{EggInteract, Lobby, PlayerId, PlayerPose, SetEasterEgg};

use crate::lobby::LobbyPlayer;
use crate::pvp::{PlayerCombat, ZombieKilled};
use crate::sim::EYE_HEIGHT;

pub struct EasterEggPlugin;

impl Plugin for EasterEggPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_set_easter_egg)
            .add_observer(on_egg_interact)
            .init_resource::<Rituals>()
            .add_systems(
                FixedUpdate,
                (drop_vial, run_altar, run_ritual).chain().after(crate::pvp::apply_player_hits),
            );
    }
}

/// Feet under a pose.
fn feet(pose: &PlayerPose) -> Vec3 {
    pose.translation - Vec3::Y * EYE_HEIGHT
}

/// Debug: the leader sets the egg's progress — only in a running game it's
/// part of (`Zombies` on Der Tempel).
fn on_set_easter_egg(trigger: Trigger<RemoteTrigger<SetEasterEgg>>, mut lobbies: Query<(Entity, &mut Lobby)>) {
    let peer = trigger.from;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.leader == peer && shared::easter_egg::available(l))
    else {
        return;
    };
    let egg = trigger.trigger.egg.clone().normalized();
    if lobby.easter_egg != egg {
        info!("lobby {lobby_e:?}: easter egg set to {egg:?} (debug)");
        lobby.easter_egg = egg;
    }
}

/// The first Big Steve killed while the vial's still to be found drops it
/// where he fell — unless that's outside a window, out of reach (the next
/// one drops it then; a Nuke's kills don't count either).
fn drop_vial(mut kills: EventReader<ZombieKilled>, mut lobbies: Query<&mut Lobby>) {
    for kill in kills.read() {
        if !kill.anim.is_boss() || kill.outside {
            continue;
        }
        let Ok(mut lobby) = lobbies.get_mut(kill.lobby) else { continue };
        let egg = &lobby.easter_egg;
        if !shared::easter_egg::available(&lobby) || egg.step != EggStep::Gathering || egg.vial || egg.vial_at.is_some()
        {
            continue;
        }
        lobby.easter_egg.vial_at = Some(kill.feet);
        info!("lobby {:?}: Big Steve dropped Barrett's blood at {}", kill.lobby, kill.feet);
    }
}

/// A player's easter egg interact: they must be in a running, unpaused game
/// it's part of, alive, and where it is; anything else is ignored.
fn on_egg_interact(
    trigger: Trigger<RemoteTrigger<EggInteract>>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
    mut radio: EventWriter<crate::radio::SayRadio>,
) {
    let peer = trigger.from;
    let action = trigger.trigger.action;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| shared::easter_egg::available(l) && l.has(peer))
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.paused {
        return;
    }
    let Some((_, pose, combat)) = players.iter().find(|(id, ..)| id.0 == peer) else {
        return;
    };
    if !combat.alive {
        return;
    }
    let feet = feet(pose);
    let map = lobby.map;
    let egg = &lobby.easter_egg;
    // (Slack: their pose is a moment old.)
    const SLACK: f32 = 0.75;
    match action {
        EggAction::PickUpVial => {
            let Some(at) = egg.vial_at else { return };
            if egg.step != EggStep::Gathering || !shared::easter_egg::within(at, feet, VIAL_REACH, SLACK) {
                return;
            }
            let egg = &mut lobby.easter_egg;
            egg.vial = true;
            egg.vial_at = None;
            info!("lobby {lobby_e:?}: {peer:?} picked up Barrett's blood");
            gathered_radio(lobby_e, &lobby, RadioLine::EggVialFound, &mut radio);
        }
        EggAction::TakeDagger => {
            if egg.step != EggStep::Gathering
                || egg.dagger
                || !shared::easter_egg::case_open(&lobby)
                || !shared::easter_egg::at_case(map, feet, SLACK)
            {
                return;
            }
            lobby.easter_egg.dagger = true;
            info!("lobby {lobby_e:?}: {peer:?} took the Hierophant's dagger");
            gathered_radio(lobby_e, &lobby, RadioLine::EggDaggerTaken, &mut radio);
        }
        EggAction::Accept => {
            if !egg.at_altar() || egg.accepted.contains(&peer) || !shared::easter_egg::in_ritual_area(map, feet) {
                return;
            }
            lobby.easter_egg.accepted.push(peer);
            info!("lobby {lobby_e:?}: {peer:?} accepted the ritual");
        }
        EggAction::Decline => {
            if egg.accepted.is_empty() {
                return;
            }
            lobby.easter_egg.accepted.clear();
            info!("lobby {lobby_e:?}: {peer:?} declined the ritual — everyone's acceptance cleared");
        }
    }
}

/// How long (s) each lobby's ritual has been going.
#[derive(Resource, Default)]
struct Rituals(bevy::platform::collections::HashMap<Entity, f32>);

/// The ritual's clock, for each lobby doing it (however it got there — the
/// altar, or the debug panel), held while the game's paused; once it's run
/// its length, it goes wrong. A lobby not doing it has none.
fn run_ritual(time: Res<Time>, mut rituals: ResMut<Rituals>, mut lobbies: Query<(Entity, &mut Lobby)>) {
    let dt = time.delta_secs();
    rituals.0.retain(|e, _| {
        lobbies
            .get(*e)
            .is_ok_and(|(_, l)| shared::easter_egg::available(l) && l.easter_egg.step == EggStep::Ritual)
    });
    for (lobby_e, mut lobby) in &mut lobbies {
        if !shared::easter_egg::available(&lobby) || lobby.easter_egg.step != EggStep::Ritual {
            continue;
        }
        let t = rituals.0.entry(lobby_e).or_insert(0.0);
        if !lobby.paused {
            *t += dt;
        }
        if *t >= shared::easter_egg::RITUAL_SECS {
            rituals.0.remove(&lobby_e);
            lobby.easter_egg.step = EggStep::Unleashed;
            info!("lobby {lobby_e:?}: the ritual's done — and it's gone wrong");
        }
    }
}

/// The man on the radio on something gathered (`line`) — then, with both
/// had, his hint at the altar (each once a game, waiting their turn —
/// `crate::radio`).
fn gathered_radio(lobby_e: Entity, lobby: &Lobby, line: RadioLine, radio: &mut EventWriter<crate::radio::SayRadio>) {
    radio.write(crate::radio::SayRadio { lobby: lobby_e, line });
    if lobby.easter_egg.gathered() {
        radio.write(crate::radio::SayRadio {
            lobby: lobby_e,
            line: RadioLine::EggBothHeld,
        });
    }
}

/// At the altar: anyone who's accepted stepping out of the ritual area (or
/// going down, or leaving) clears everyone's acceptance; once every living
/// player's accepted, the ritual begins.
fn run_altar(
    endings: Res<crate::killcam::EndingLobbies>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat), With<LobbyPlayer>>,
) {
    for (lobby_e, mut lobby) in &mut lobbies {
        if !shared::easter_egg::available(&lobby) || !lobby.easter_egg.at_altar() || endings.is_ending(lobby_e) {
            continue;
        }
        if lobby.easter_egg.accepted.is_empty() {
            continue;
        }
        let map = lobby.map;
        let real = lobby.real_peers();
        // Each real member's state: alive, and in the area.
        let state = |peer: PeerId| {
            players
                .iter()
                .find(|(id, ..)| id.0 == peer)
                .map(|(_, pose, combat)| (combat.alive, shared::easter_egg::in_ritual_area(map, feet(pose))))
        };
        let egg = &lobby.easter_egg;
        let broken = egg
            .accepted
            .iter()
            .any(|&p| !real.contains(&p) || state(p).is_none_or(|(alive, inside)| !alive || !inside));
        if broken {
            lobby.easter_egg.accepted.clear();
            info!("lobby {lobby_e:?}: someone left the ritual area — the ritual's acceptance cleared");
            continue;
        }
        let living: Vec<PeerId> = real.into_iter().filter(|&p| state(p).is_some_and(|(alive, _)| alive)).collect();
        if !living.is_empty() && living.iter().all(|p| egg.accepted.contains(p)) {
            let egg = &mut lobby.easter_egg;
            egg.accepted.clear();
            egg.step = EggStep::Ritual;
            info!("lobby {lobby_e:?}: every living player accepted — the ritual begins");
        }
    }
}
