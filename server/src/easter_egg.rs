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
//!   (`Lobby::bots_hold_fire`), the clients locking their players in place,
//!   the man on the radio talking them through it and giving the seal, and
//!   every operator repeating it — then it goes wrong ([`run_ritual`]);
//! * once it's gone wrong (however it got there — the ritual or the debug
//!   panel), every zombie up drops dead — no points, no drops — the round's
//!   own spawns are held until he's dead (`shared::easter_egg::holds_spawns`,
//!   `crate::zombies::run_rounds`), and once the Hierophant's had his say
//!   from the sarcophagus (the clients play that), the man on the radio
//!   realises what he's done; a breath after, he wakes
//!   ([`shared::easter_egg::EggStep::Awakening`] — the clients play it out)
//!   and once he's up the fight's on ([`run_unleashed`]) — the Hierophant
//!   himself, until he's killed ([`fight`]); then his death plays out
//!   ([`shared::easter_egg::EggStep::HisDeath`], the round still held) before
//!   it moves on;
//! * the leader's debug panel can set any of it ([`shared::SetEasterEgg`])
//!   — without the gathering's radio.

use bevy::prelude::*;
use lightyear::prelude::*;
use shared::easter_egg::{EggAction, EggStep, VIAL_REACH};
use shared::radio::RadioLine;
use shared::{EggInteract, Lobby, PlayerId, PlayerPose, SetEasterEgg};

use crate::lobby::LobbyPlayer;
use crate::pvp::{PlayerCombat, ZombieKilled};
use crate::sim::EYE_HEIGHT;
use crate::zombies::Zombie;

mod ascension;
mod fight;

pub(crate) use fight::{Hierophant, Raised};

pub struct EasterEggPlugin;

impl Plugin for EasterEggPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_set_easter_egg)
            .add_observer(on_egg_interact)
            .init_resource::<Rituals>()
            .init_resource::<Unleashings>()
            .init_resource::<fight::Fights>()
            .init_resource::<fight::DebugPokes>()
            .init_resource::<ascension::Ascents>()
            .init_resource::<ascension::Returned>()
            .add_observer(fight::on_hierophant_debug)
            .add_systems(
                FixedUpdate,
                (drop_vial, run_altar, run_ritual, run_unleashed, fight::run_fight, ascension::run_ascension)
                    .chain()
                    .after(crate::pvp::apply_player_hits),
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

/// Each lobby's ritual: how long (s) it's been going, and which of its
/// voices have been heard.
#[derive(Resource, Default)]
struct Rituals(bevy::platform::collections::HashMap<Entity, Ritual>);

#[derive(Default)]
struct Ritual {
    t: f32,
    begun: bool,
    sealed: bool,
    chanted: bool,
}

/// The ritual's clock, for each lobby doing it (however it got there — the
/// altar, or the debug panel), held while the game's paused: the man on the
/// radio talking them through it as it begins, then the seal
/// ([`shared::easter_egg::RITUAL_SEAL_AT_SECS`]), then every operator in it
/// repeating that ([`shared::easter_egg::RITUAL_CHANT_AT_SECS`] — the
/// players, and any teammate bot standing in the ritual area); once it's
/// run its length, it goes wrong. A lobby not doing it has none.
fn run_ritual(
    time: Res<Time>,
    mut rituals: ResMut<Rituals>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat), With<LobbyPlayer>>,
    mut radio: EventWriter<crate::radio::SayRadio>,
    mut quotes: EventWriter<crate::quotes::SayQuote>,
) {
    use shared::easter_egg::{RITUAL_CHANT_AT_SECS, RITUAL_SEAL_AT_SECS};
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
        let r = rituals.0.entry(lobby_e).or_default();
        if !lobby.paused {
            r.t += dt;
        }
        if !r.begun {
            r.begun = true;
            radio.write(crate::radio::SayRadio { lobby: lobby_e, line: RadioLine::EggRitualBegins });
        }
        if !r.sealed && r.t >= RITUAL_SEAL_AT_SECS {
            r.sealed = true;
            radio.write(crate::radio::SayRadio { lobby: lobby_e, line: RadioLine::EggRitualSeal });
        }
        if !r.chanted && r.t >= RITUAL_CHANT_AT_SECS {
            r.chanted = true;
            let map = lobby.map;
            for (id, pose, combat) in &players {
                let standing = combat.alive && combat.down.is_none();
                let ally = shared::bot_players::is_ally_peer(id.0);
                let there = !ally || shared::easter_egg::in_ritual_area(map, feet(pose));
                if lobby.has(id.0) && !shared::bot_players::is_bot_peer(id.0) && standing && there {
                    quotes.write(crate::quotes::SayQuote::by(lobby_e, id.0, shared::quotes::Quote::RitualChant));
                }
            }
        }
        if r.t >= shared::easter_egg::RITUAL_SECS {
            rituals.0.remove(&lobby_e);
            lobby.easter_egg.step = EggStep::Unleashed;
            info!("lobby {lobby_e:?}: the ritual's done — and it's gone wrong");
        }
    }
}

/// Each lobby where it's gone wrong: how long (s, not counting pauses)
/// since, and whether its zombies have dropped and the radio's spoken yet.
#[derive(Resource, Default)]
struct Unleashings(bevy::platform::collections::HashMap<Entity, Unleashing>);

#[derive(Default)]
struct Unleashing {
    t: f32,
    dropped: bool,
    radioed: bool,
    /// How long (s) he's been waking, while he is...
    waking: f32,
    /// ...or dying (his spirit fleeing into the pyramid).
    dying: f32,
    /// The step it was at last tick.
    last: Option<EggStep>,
}

/// Once it's gone wrong ([`shared::easter_egg::EggStep::is_unleashed`]):
/// every zombie up in the game (not him — a debug jump straight to the
/// fight) drops dead a moment after — killed outright,
/// so no points, drop or kill counted — then, the Hierophant done speaking,
/// the radio line; [`shared::easter_egg::unleashed_secs`] in, he wakes, and
/// [`shared::easter_egg::AWAKENING_SECS`] after that, the fight's on. A
/// lobby that's moved on (or reset) forgets its clock, so it all happens
/// again if it goes wrong again.
#[allow(clippy::type_complexity)]
fn run_unleashed(
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    mut unleashings: ResMut<Unleashings>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut zombies: Query<(&LobbyPlayer, &mut PlayerCombat), (With<Zombie>, Without<Hierophant>)>,
    mut radio: EventWriter<crate::radio::SayRadio>,
) {
    let dt = time.delta_secs();
    unleashings
        .0
        .retain(|e, _| lobbies.get(*e).is_ok_and(|(_, l)| shared::easter_egg::holds_spawns(l)));
    for (lobby_e, mut lobby) in &mut lobbies {
        if !shared::easter_egg::holds_spawns(&lobby) {
            continue;
        }
        let u = unleashings.0.entry(lobby_e).or_default();
        // (Back to it going wrong from further on — a debug jump: all over
        // again.)
        let step = lobby.easter_egg.step;
        if step == EggStep::Unleashed && u.last.is_some_and(|l| l != EggStep::Unleashed) {
            *u = Unleashing::default();
        }
        u.last = Some(step);
        if lobby.paused || endings.is_ending(lobby_e) {
            continue;
        }
        u.t += dt;
        if !u.dropped && u.t >= shared::easter_egg::UNLEASHED_ZOMBIES_DROP_SECS {
            u.dropped = true;
            let mut dropped = 0;
            for (lp, mut combat) in &mut zombies {
                if lp.lobby != lobby_e || !combat.alive {
                    continue;
                }
                combat.alive = false;
                combat.health = 0.0;
                combat.respawn_at = f32::INFINITY;
                dropped += 1;
            }
            info!("lobby {lobby_e:?}: it's gone wrong — {dropped} zombies dropped dead");
        }
        // (Not if a debug jump's gone past it.)
        if !u.radioed && u.t >= shared::easter_egg::UNLEASHED_RADIO_SECS && step == EggStep::Unleashed {
            u.radioed = true;
            radio.write(crate::radio::SayRadio {
                lobby: lobby_e,
                line: RadioLine::EggRitualGoesWrong,
            });
        }
        match step {
            EggStep::Unleashed if u.t >= shared::easter_egg::unleashed_secs() => {
                lobby.easter_egg.step = EggStep::Awakening;
                u.waking = 0.0;
                info!("lobby {lobby_e:?}: the Hierophant wakes");
            }
            EggStep::Awakening => {
                u.waking += dt;
                if u.waking >= shared::easter_egg::AWAKENING_SECS {
                    lobby.easter_egg.step = EggStep::BossFight;
                    info!("lobby {lobby_e:?}: the Hierophant's up — the fight's on");
                }
            }
            EggStep::HisDeath => {
                u.dying += dt;
                if u.dying >= shared::easter_egg::DEATH_SECS {
                    lobby.easter_egg.step = EggStep::Ascension;
                    info!("lobby {lobby_e:?}: his spirit's in the pyramid — the round goes on");
                }
            }
            _ => {
                u.waking = 0.0;
                u.dying = 0.0;
            }
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
