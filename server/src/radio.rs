//! `Zombies`' man on the radio ([`shared::radio`]), directed: something
//! asks for a line ([`SayRadio`] — the first dog round, `crate::zombies`) or
//! it's due (the intro, [`INTRO_DELAY_SECS`] after everyone's loaded in —
//! only in a game starting on round 1, `Lobby::start_round`),
//! and the whole lobby's told ([`RadioSaid`]). While a line runs the lobby's
//! on air ([`RadioDirector::on_air`]): `crate::quotes` lets no operator speak.
//!
//! Per game: a lobby that isn't running a `Zombies` game has its intro and
//! airtime dropped, so the next game gets both afresh.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::radio::{RadioLine, INTRO_DELAY_SECS};
use shared::{GameChannel, GameMode, Lobby, RadioSaid};

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
    /// On air until then (`Time::elapsed_secs`).
    until: f32,
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

fn direct_radio(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut requests: EventReader<SayRadio>,
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

    let mut said: Vec<(Entity, RadioLine)> = requests.read().map(|r| (r.lobby, r.line)).collect();
    // Everyone's in: the intro's due a moment later — if the game starts
    // on round 1 (none for a later start).
    for (lobby_e, lobby) in &lobbies {
        if running(lobby_e).is_none() || !lobby.members.iter().all(|m| m.loaded) {
            continue;
        }
        let air = director.games.entry(lobby_e).or_insert(Air {
            intro_at: (lobby.start_round <= 1).then_some(now + INTRO_DELAY_SECS),
            until: 0.0,
        });
        if air.intro_at.is_some_and(|at| now >= at) {
            air.intro_at = None;
            said.push((lobby_e, RadioLine::Intro));
        }
    }

    for (lobby_e, line) in said {
        let Some(lobby) = running(lobby_e) else {
            continue;
        };
        let air = director.games.entry(lobby_e).or_insert(Air { intro_at: None, until: 0.0 });
        air.until = air.until.max(now + line.airtime());
        if let Err(e) = sender.send::<_, GameChannel>(&RadioSaid { line }, server, &NetworkTarget::Only(lobby.real_peers())) {
            error!("failed to send radio line: {e:?}");
        }
        debug!("lobby {lobby_e:?}: radio says {line:?}");
    }
}
