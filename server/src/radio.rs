//! `Zombies`' man on the radio ([`shared::radio`]), directed: something
//! asks for a line ([`SayRadio`] — the first dog round, the power, the
//! exfil, ... — `crate::zombies` and the rest), or it's due (the intro,
//! [`INTRO_DELAY_SECS`] after everyone's loaded in — only in a game starting
//! on round 1, `Lobby::start_round`; the first boss's death, from the kills
//! here), and the whole lobby's told ([`RadioSaid`]). While a line runs the
//! lobby's on air ([`RadioDirector::on_air`]): `crate::quotes` lets no
//! operator speak.
//!
//! A line that cuts in ([`RadioLine::cuts_in`]), or cuts off the one he's
//! saying ([`RadioLine::cuts_off`]), is said at once; any other
//! waits its turn in the lobby's queue ([`LEAD_IN_SECS`], him off air, the
//! operators quiet for up to [`QUIET_WAIT_SECS`]) — dropped if what it's
//! about is no longer so by then ([`still_so`]). A once-a-game line
//! ([`RadioLine::once_per_game`]) asked for again is dropped.
//!
//! Per game: a lobby that isn't running a `Zombies` game has its intro,
//! airtime, queue and lines said dropped, so the next game gets them afresh.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::radio::{RadioLine, INTRO_DELAY_SECS, LEAD_IN_SECS, QUIET_WAIT_SECS};
use shared::{GameChannel, GameMode, Lobby, RadioSaid};

use crate::pvp::ZombieKilled;

/// The man on the radio says `line` to `lobby`, now.
#[derive(Event, Clone, Copy, Debug)]
pub struct SayRadio {
    pub lobby: Entity,
    pub line: RadioLine,
}

/// One running game's radio.
struct Air {
    /// When the intro's due; `None` once it's been said.
    intro_at: Option<f32>,
    /// On air until then (`Time::elapsed_secs`), saying this.
    until: f32,
    current: Option<RadioLine>,
    /// Lines waiting their turn, oldest first, and when each was asked for.
    queue: VecDeque<(RadioLine, f32)>,
    /// Every line said (or waiting) this game.
    said: HashSet<RadioLine>,
}

impl Air {
    fn new(lobby: &Lobby, now: f32) -> Self {
        Self {
            intro_at: (lobby.start_round <= 1).then_some(now + INTRO_DELAY_SECS),
            until: 0.0,
            current: None,
            queue: VecDeque::new(),
            said: HashSet::new(),
        }
    }
}

#[derive(Resource, Default)]
pub struct RadioDirector {
    games: HashMap<Entity, Air>,
}

impl RadioDirector {
    /// Whether he's talking to `lobby` right now.
    pub fn on_air(&self, lobby: Entity, now: f32) -> bool {
        self.games.get(&lobby).is_some_and(|a| now < a.until)
    }

    /// When he's done talking to `lobby` (in the past if he isn't).
    pub fn off_air_at(&self, lobby: Entity) -> f32 {
        self.games.get(&lobby).map_or(0.0, |a| a.until)
    }
}

pub struct RadioPlugin;

impl Plugin for RadioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RadioDirector>()
            .add_event::<SayRadio>()
            .add_systems(
                FixedUpdate,
                direct_radio
                    .after(crate::zombies::run_rounds)
                    .before(crate::quotes::direct_quotes),
            );
    }
}

/// Whether what `line`'s about is still so in `lobby`, for one that's
/// waited its turn.
fn still_so(line: RadioLine, lobby: &Lobby) -> bool {
    match line {
        RadioLine::PowerStillOff => !shared::power::has_power(lobby.map, lobby.power_on),
        RadioLine::FirstExfilAvailable | RadioLine::ExfilAvailable => shared::exfil::available(lobby),
        _ => true,
    }
}

#[allow(clippy::too_many_arguments)]
fn direct_radio(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut requests: EventReader<SayRadio>,
    mut kills: EventReader<ZombieKilled>,
    quotes: Res<crate::quotes::Director>,
    lobbies: Query<(Entity, &Lobby)>,
    mut director: ResMut<RadioDirector>,
) {
    let server = server.into_inner();
    let now = time.elapsed_secs();
    let running = |e: Entity| {
        lobbies
            .get(e)
            .ok()
            .filter(|(_, l)| l.started && l.mode == GameMode::Zombies)
            .map(|(_, l)| l)
    };
    director.games.retain(|e, _| running(*e).is_some());

    let mut asks: Vec<(Entity, RadioLine)> = requests.read().map(|r| (r.lobby, r.line)).collect();
    // The game's first boss down.
    asks.extend(
        kills
            .read()
            .filter(|k| k.anim.is_boss())
            .map(|k| (k.lobby, RadioLine::FirstBossKill)),
    );

    let mut said: Vec<(Entity, RadioLine)> = Vec::new();
    for (lobby_e, line) in asks {
        let Some(lobby) = running(lobby_e) else {
            continue;
        };
        let air = director.games.entry(lobby_e).or_insert_with(|| Air::new(lobby, now));
        if line.once_per_game() && !air.said.insert(line) {
            continue;
        }
        let cuts_off = now < air.until && air.current.is_some_and(|c| line.cuts_off(c));
        if line.cuts_in() || cuts_off {
            said.push((lobby_e, line));
        } else {
            air.queue.push_back((line, now));
        }
    }

    for (lobby_e, lobby) in &lobbies {
        if running(lobby_e).is_none() || !lobby.members.iter().all(|m| m.loaded) {
            continue;
        }
        let air = director.games.entry(lobby_e).or_insert_with(|| Air::new(lobby, now));
        // Everyone's in: the intro's due a moment later — if the game starts
        // on round 1 (none for a later start).
        if air.intro_at.is_some_and(|at| now >= at) {
            air.intro_at = None;
            said.push((lobby_e, RadioLine::Intro));
            continue;
        }
        // The next line waiting, once it's its turn.
        if said.iter().any(|(e, _)| *e == lobby_e) || now < air.until || air.intro_at.is_some() {
            continue;
        }
        while air.queue.front().is_some_and(|(line, _)| !still_so(*line, lobby)) {
            air.queue.pop_front();
        }
        let Some(&(line, asked)) = air.queue.front() else {
            continue;
        };
        let from = asked + LEAD_IN_SECS;
        if now >= from && (quotes.quiet(lobby_e, now) || now >= from + QUIET_WAIT_SECS) {
            air.queue.pop_front();
            said.push((lobby_e, line));
        }
    }

    for (lobby_e, line) in said {
        let Some(lobby) = running(lobby_e) else {
            continue;
        };
        let air = director.games.entry(lobby_e).or_insert_with(|| Air::new(lobby, now));
        // (One cutting in takes over from whatever he was saying.)
        air.until = now + line.airtime();
        air.current = Some(line);
        if line == RadioLine::GameOver {
            air.queue.clear();
        }
        if let Err(e) = sender.send::<_, GameChannel>(&RadioSaid { line }, server, &NetworkTarget::Only(lobby.real_peers())) {
            error!("failed to send radio line: {e:?}");
        }
        debug!("lobby {lobby_e:?}: radio says {line:?}");
    }
}
