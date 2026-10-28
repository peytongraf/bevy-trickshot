//! `Zombies` operator quotes, directed: whatever happens in a game that a
//! player's operator has lines for ([`shared::quotes::Quote`]) asks for one
//! ([`SayQuote`]), and this decides whether it's said, by whom, and which
//! line — and tells the whole lobby ([`QuoteSaid`]), so everyone hears the
//! same line, from the speaker.
//!
//! * A quote said "by anyone" (a power-up, the first boss, a dog round, the
//!   exfil) goes to one player picked at random (a living one, if any's
//!   up); one moment never has two people talking at once — only the first
//!   request for a speaker in a tick counts.
//! * Each quote's [`Rule`](shared::quotes::Rule): the chance it's said at
//!   all, and how long before the same speaker says it again.
//! * No line (the same file) is heard twice in a game within
//!   [`LINE_COOLDOWN_SECS`]: a fresh one is picked, and if every line for
//!   the moment has been heard lately, nothing's said (by anyone — "anyone"
//!   is picked among the players with a fresh line). A quote that
//!   [cycles](shared::quotes::Quote::cycles) instead has each operator say
//!   every one of their lines for it before any again (each game).
//! * Nobody talks over themselves: a speaker's busy until their line's
//!   done (the lengths are in `shared::quotes`). A line asked for while
//!   they're still talking waits — said the moment they finish if that's
//!   within [`GRACE_SECS`], never if not.
//! * Nobody talks over the man on the radio (`crate::radio`): anything
//!   asked for (or waiting) while he's on air is dropped — except a delayed
//!   line ([`QuoteDelay`]) coming due then, which waits for him to finish.
//! * A dog round's and the exfil's lines come a few seconds after
//!   ([`SetQuoteDelays`] — the leader can change how long); a power-up's
//!   once its announcer's done ([`shared::power_ups::PowerUp::announcer_secs`]);
//!   the box moving's a couple of seconds after the bear's "bye bye"
//!   ([`shared::mystery_box::TEDDY_QUOTE_SECS`]); one about a round
//!   starting once its sting's done ([`ROUND_START_DELAY_SECS`]); and one
//!   answering the man on the radio once he's said his line about it
//!   ([`QuoteDelay::AfterRadio`]).
//! * Something happening to a teammate (going down, bleeding out) is said
//!   by someone else still standing ([`Speaker::AnyoneBut`]) — nobody, if
//!   there's no one.
//!
//! All of it is per game: a lobby that isn't running a `Zombies` game has
//! its speakers and anything scheduled dropped.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::bot_players::is_bot_peer;
use shared::operator::Operator;
use shared::quotes::{
    lines, Quote, DOG_ROUND_DELAY_SECS, EXFIL_DELAY_SECS, GAP_SECS, GRACE_SECS, LINE_COOLDOWN_SECS,
    ROUND_START_DELAY_SECS,
};
use shared::radio::RadioLine;
use shared::{GameChannel, GameMode, Lobby, PlayerId, QuoteSaid, RequestQuote, SetQuoteDelays};

use crate::pvp::{HitCause, PlayerCombat, ZombieKilled};

/// Something an operator could say about: `quote`, in `lobby`, by `who`,
/// `delay` after now.
#[derive(Event, Clone, Copy, Debug)]
pub struct SayQuote {
    pub lobby: Entity,
    pub who: Speaker,
    pub quote: Quote,
    pub delay: QuoteDelay,
}

impl SayQuote {
    /// `peer` says `quote`, now.
    pub fn by(lobby: Entity, peer: PeerId, quote: Quote) -> Self {
        Self {
            lobby,
            who: Speaker::Player(peer),
            quote,
            delay: QuoteDelay::None,
        }
    }

    /// Someone in the lobby (picked at random) says `quote`, now.
    pub fn anyone(lobby: Entity, quote: Quote) -> Self {
        Self {
            lobby,
            who: Speaker::Anyone,
            quote,
            delay: QuoteDelay::None,
        }
    }

    /// Someone standing in the lobby other than `peer` says `quote` (about
    /// them), now.
    pub fn anyone_but(lobby: Entity, peer: PeerId, quote: Quote) -> Self {
        Self {
            lobby,
            who: Speaker::AnyoneBut(peer),
            quote,
            delay: QuoteDelay::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    Player(PeerId),
    /// A member of the lobby (a teammate bot included), picked at random.
    Anyone,
    /// Likewise, but not them — and only someone standing (up, not down
    /// or bled out).
    AnyoneBut(PeerId),
}

/// When a [`SayQuote`] is said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteDelay {
    None,
    /// The lobby's dog-round / exfil delay ([`SetQuoteDelays`]).
    DogRound,
    Exfil,
    /// Once this power-up's announcer line is done.
    Announcer(shared::power_ups::PowerUp),
    /// The box landed on the teddy bear: a moment after its "bye bye".
    TeddyBear,
    /// A round's just begun: once its sting's done.
    RoundStart,
    /// Once the man on the radio's said this line (asked for at the same
    /// time) and is off air — or straight away if he never does.
    AfterRadio(RadioLine),
}

/// One player's voice in a game.
struct Voice {
    lobby: Entity,
    /// Talking until then (`Time::elapsed_secs`, gap included).
    busy_until: f32,
    /// A line waiting for them to finish, and until when it may.
    pending: Option<(Quote, f32)>,
    /// When they last said each quote.
    said: HashMap<Quote, f32>,
}

#[derive(Resource, Default)]
pub(crate) struct Director {
    voices: HashMap<PeerId, Voice>,
    /// Each lobby's (dog round, exfil) delays, if the leader set them.
    delays: HashMap<Entity, (f32, f32)>,
    /// Quotes waiting on their delay: when, and what (its `delay` cleared,
    /// or still [`QuoteDelay::AfterRadio`] while it waits on him).
    scheduled: Vec<(f32, SayQuote)>,
    heard: Heard,
}

/// The lines heard in each lobby's game.
#[derive(Default)]
struct Heard {
    /// When each line (its path) was last heard.
    at: HashMap<(Entity, &'static str), f32>,
    /// For a quote that [cycles](Quote::cycles): the lines each operator's
    /// said of it this time through, in order.
    round: HashMap<(Entity, Operator, Quote), Vec<&'static str>>,
}

impl Heard {
    fn retain(&mut self, running: impl Fn(Entity) -> bool) {
        self.at.retain(|(e, _), _| running(*e));
        self.round.retain(|(e, _, _), _| running(*e));
    }

    /// `line` of `operator`'s for `quote` is said now.
    fn said(&mut self, lobby_e: Entity, operator: Operator, quote: Quote, line: &'static str, now: f32) {
        self.at.insert((lobby_e, line), now);
        if quote.cycles() {
            let round = self.round.entry((lobby_e, operator, quote)).or_default();
            if round.len() >= lines(operator, quote).len() {
                round.clear();
            }
            round.push(line);
        }
    }
}

/// Which of `speaker`'s lines for `quote` haven't been heard in `lobby_e`'s
/// game for [`LINE_COOLDOWN_SECS`] (indices into [`lines`]) — or, for one
/// that [cycles](Quote::cycles), haven't been said yet this time through
/// (all of them but the very last once they all have).
fn fresh_lines(
    heard: &Heard,
    lobby_e: Entity,
    lobby: &Lobby,
    speaker: PeerId,
    quote: Quote,
    now: f32,
) -> Vec<usize> {
    let Some(member) = lobby.members.iter().find(|m| m.peer == speaker) else {
        return Vec::new();
    };
    let options = lines(member.operator, quote);
    if quote.cycles() {
        let round = heard.round.get(&(lobby_e, member.operator, quote)).map_or(&[][..], Vec::as_slice);
        let unsaid: Vec<usize> = (0..options.len()).filter(|&i| !round.contains(&options[i].path)).collect();
        if !unsaid.is_empty() {
            return unsaid;
        }
        // (Round again — just not straight into the line that ended the last.)
        return (0..options.len())
            .filter(|&i| options.len() == 1 || round.last() != Some(&options[i].path))
            .collect();
    }
    options
        .iter()
        .enumerate()
        .filter(|(_, l)| heard.at.get(&(lobby_e, l.path)).is_none_or(|t| now - t >= LINE_COOLDOWN_SECS))
        .map(|(i, _)| i)
        .collect()
}

impl Director {
    /// Whether no operator in `lobby` is talking right now — or about to:
    /// with a line waiting for its speaker to finish, or one scheduled (a
    /// power-up's, waiting out its announcer; a dog round's or the exfil's,
    /// its delay). `crate::radio` waits for that.
    pub(crate) fn quiet(&self, lobby: Entity, now: f32) -> bool {
        !self
            .voices
            .values()
            .any(|v| v.lobby == lobby && (now < v.busy_until || v.pending.is_some()))
            // (Not one waiting for him — they'd wait on each other.)
            && !self
                .scheduled
                .iter()
                .any(|(_, q)| q.lobby == lobby && !matches!(q.delay, QuoteDelay::AfterRadio(_)))
    }
}

pub struct QuotesPlugin;

impl Plugin for QuotesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Director>()
            .add_event::<SayQuote>()
            .add_observer(on_request_quote)
            .add_observer(on_set_quote_delays)
            .add_systems(
                FixedUpdate,
                (kill_quotes, direct_quotes)
                    .chain()
                    .after(crate::pvp::apply_player_hits),
            );
    }
}

/// A seeded roll in `0..1`.
fn roll(now: f32, peer: PeerId, quote: Quote, salt: u64) -> f32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (now.to_bits(), peer, quote, salt).hash(&mut h);
    shared::bots::rand01(h.finish())
}

/// What a kill's worth saying: a boss, what it was made with, a hellhound,
/// or just a kill.
fn kill_quote(kill: &ZombieKilled) -> Quote {
    if kill.anim.is_boss() {
        return Quote::BossKill;
    }
    match kill.cause {
        HitCause::Frag => Quote::FragKill,
        HitCause::Molotov => Quote::MolotovKill,
        HitCause::ThrowingKnife => Quote::ThrowingKnifeKill,
        HitCause::Melee => Quote::MeleeKill,
        HitCause::Phd => Quote::PhdKill,
        _ if kill.anim.is_dog() => Quote::DogKill,
        _ => Quote::EnemyKill,
    }
}

/// Every kill a player made: their line about it.
fn kill_quotes(mut kills: EventReader<ZombieKilled>, mut quotes: EventWriter<SayQuote>) {
    for kill in kills.read() {
        quotes.write(SayQuote::by(kill.lobby, kill.killer, kill_quote(kill)));
    }
}

/// Our client says we're low on / out of ammo, or can't afford what we
/// just tried to buy (all it may ask for).
fn on_request_quote(
    trigger: Trigger<RemoteTrigger<RequestQuote>>,
    lobbies: Query<(Entity, &Lobby)>,
    mut quotes: EventWriter<SayQuote>,
) {
    let peer = trigger.from;
    let quote = trigger.trigger.quote;
    if !matches!(quote, Quote::LowAmmo | Quote::OutOfAmmo | Quote::CantAfford) {
        return;
    }
    if let Some((lobby_e, _)) = lobbies.iter().find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(peer)) {
        quotes.write(SayQuote::by(lobby_e, peer, quote));
    }
}

/// The leader sets their lobby's dog-round / exfil quote delays.
fn on_set_quote_delays(
    trigger: Trigger<RemoteTrigger<SetQuoteDelays>>,
    lobbies: Query<(Entity, &Lobby)>,
    mut director: ResMut<Director>,
) {
    let req = trigger.trigger;
    if let Some((lobby_e, _)) = lobbies.iter().find(|(_, l)| l.leader == trigger.from) {
        director
            .delays
            .insert(lobby_e, (req.dog_round.clamp(0.0, 60.0), req.exfil.clamp(0.0, 60.0)));
    }
}

/// Decide on every quote asked for (and every one waiting), and tell the
/// lobby about the ones said.
#[allow(clippy::too_many_arguments)]
pub(crate) fn direct_quotes(
    time: Res<Time>,
    radio: Res<crate::radio::RadioDirector>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut requests: EventReader<SayQuote>,
    lobbies: Query<(Entity, &Lobby)>,
    combats: Query<(&PlayerId, &PlayerCombat)>,
    mut director: ResMut<Director>,
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
    let director = &mut *director;
    // Forget lobbies whose game's over (or gone).
    director.voices.retain(|_, v| running(v.lobby).is_some());
    director.delays.retain(|e, _| lobbies.contains(*e));
    director.scheduled.retain(|(_, q)| running(q.lobby).is_some());
    director.heard.retain(|e| running(e).is_some());

    // This tick's asks: new ones (a delayed one put off), and those due.
    let mut asks: Vec<SayQuote> = Vec::new();
    for q in requests.read() {
        let delay = director.delays.get(&q.lobby).copied().unwrap_or((DOG_ROUND_DELAY_SECS, EXFIL_DELAY_SECS));
        let secs = match q.delay {
            QuoteDelay::None => 0.0,
            QuoteDelay::DogRound => delay.0,
            QuoteDelay::Exfil => delay.1,
            QuoteDelay::Announcer(p) => p.announcer_secs(),
            QuoteDelay::TeddyBear => shared::mystery_box::TEDDY_QUOTE_SECS,
            QuoteDelay::RoundStart => ROUND_START_DELAY_SECS,
            // (Checked on from then — `waiting_on_radio`.)
            QuoteDelay::AfterRadio(line) => line.lead_in(),
        };
        if let QuoteDelay::AfterRadio(_) = q.delay {
            director.scheduled.push((now + secs, *q));
        } else if secs > 0.0 {
            director.scheduled.push((now + secs, SayQuote { delay: QuoteDelay::None, ..*q }));
        } else {
            asks.push(*q);
        }
    }
    director.scheduled.retain_mut(|(at, q)| {
        if *at > now {
            true
        } else if matches!(q.delay, QuoteDelay::AfterRadio(line) if radio.queued(q.lobby, line)) {
            // (He's yet to get to it: look again in a moment.)
            *at = now + 0.25;
            true
        } else if radio.on_air(q.lobby, now) {
            // (Due while he's on air: once he's done.)
            *at = radio.off_air_at(q.lobby);
            true
        } else {
            asks.push(SayQuote { delay: QuoteDelay::None, ..*q });
            false
        }
    });

    let alive = |peer: PeerId| combats.iter().find(|(id, _)| id.0 == peer).is_none_or(|(_, c)| c.alive);
    let mut said: Vec<(Entity, PeerId, Quote)> = Vec::new();
    let mut heard: HashSet<PeerId> = HashSet::new();
    for (i, ask) in asks.into_iter().enumerate() {
        let Some(lobby) = running(ask.lobby) else {
            continue;
        };
        if radio.on_air(ask.lobby, now) {
            continue;
        }
        let speaker = match ask.who {
            Speaker::Player(peer) if lobby.has(peer) && !is_bot_peer(peer) => peer,
            Speaker::Player(_) => continue,
            Speaker::Anyone | Speaker::AnyoneBut(_) => {
                let but = match ask.who {
                    Speaker::AnyoneBut(peer) => Some(peer),
                    _ => None,
                };
                // (Teammate bots too — they've an operator's voice. Only
                // those with a line for it not heard lately.)
                let real: Vec<PeerId> = lobby
                    .members
                    .iter()
                    .map(|m| m.peer)
                    .filter(|p| Some(*p) != but)
                    .filter(|p| !fresh_lines(&director.heard, ask.lobby, lobby, *p, ask.quote, now).is_empty())
                    .collect();
                let up: Vec<PeerId> = real.iter().copied().filter(|p| alive(*p)).collect();
                // (About a teammate: only someone standing.)
                let pool = if up.is_empty() && but.is_none() { real } else { up };
                if pool.is_empty() {
                    continue;
                }
                let r = roll(now, pool[0], ask.quote, 0xa11 + i as u64);
                pool[((r * pool.len() as f32) as usize).min(pool.len() - 1)]
            }
        };
        // One ask per speaker a tick (a frag's three kills are one line).
        if !heard.insert(speaker) {
            continue;
        }
        let rule = ask.quote.rule();
        if roll(now, speaker, ask.quote, 0xc4a + i as u64) >= rule.chance {
            continue;
        }
        let voice = director.voices.entry(speaker).or_insert(Voice {
            lobby: ask.lobby,
            busy_until: 0.0,
            pending: None,
            said: HashMap::new(),
        });
        voice.lobby = ask.lobby;
        if voice.said.get(&ask.quote).is_some_and(|t| now - t < rule.cooldown) {
            continue;
        }
        if fresh_lines(&director.heard, ask.lobby, lobby, speaker, ask.quote, now).is_empty() {
            continue;
        }
        if now >= voice.busy_until {
            said.push((ask.lobby, speaker, ask.quote));
        } else if voice.pending.is_none() && voice.busy_until - now <= GRACE_SECS {
            // Still talking: it can wait for them, a moment.
            voice.pending = Some((ask.quote, now + GRACE_SECS));
        }
    }
    // Lines that waited, and whose speaker's free now.
    for (peer, voice) in director.voices.iter_mut() {
        if now < voice.busy_until {
            continue;
        }
        if let Some((quote, until)) = voice.pending.take() {
            if now <= until && !radio.on_air(voice.lobby, now) && !said.iter().any(|(_, p, _)| p == peer) {
                said.push((voice.lobby, *peer, quote));
            }
        }
    }

    for (lobby_e, speaker, quote) in said {
        let Some(lobby) = running(lobby_e) else {
            continue;
        };
        let Some(member) = lobby.members.iter().find(|m| m.peer == speaker) else {
            continue;
        };
        let options = lines(member.operator, quote);
        // (Checked again: a line that waited, or two speakers with the same
        // operator this tick, may have had theirs heard since.)
        let fresh = fresh_lines(&director.heard, lobby_e, lobby, speaker, quote, now);
        if fresh.is_empty() {
            continue;
        }
        let r = roll(now, speaker, quote, 0x11e);
        let line = fresh[((r * fresh.len() as f32) as usize).min(fresh.len() - 1)];
        director.heard.said(lobby_e, member.operator, quote, options[line].path, now);
        if let Some(voice) = director.voices.get_mut(&speaker) {
            voice.busy_until = now + options[line].secs + GAP_SECS;
            voice.said.insert(quote, now);
            voice.pending = None;
        }
        let msg = QuoteSaid {
            speaker,
            quote,
            line: line as u16,
        };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(lobby.real_peers())) {
            error!("failed to send quote: {e:?}");
        }
        debug!("{speaker:?} says {quote:?} ({})", options[line].path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kill(cause: HitCause, anim: shared::ZombieAnim) -> ZombieKilled {
        ZombieKilled {
            lobby: Entity::PLACEHOLDER,
            feet: Vec3::ZERO,
            killer: PeerId::Netcode(1),
            victim: PeerId::Netcode(2),
            cause,
            anim,
            outside: false,
        }
    }

    #[test]
    fn a_kill_says_the_most_particular_thing_about_it() {
        use shared::ZombieAnim::*;
        assert_eq!(kill_quote(&kill(HitCause::Frag, BossWalk)), Quote::BossKill);
        assert_eq!(kill_quote(&kill(HitCause::Frag, Walk)), Quote::FragKill);
        assert_eq!(kill_quote(&kill(HitCause::Molotov, Dog)), Quote::MolotovKill);
        assert_eq!(kill_quote(&kill(HitCause::Gun, Dog)), Quote::DogKill);
        assert_eq!(kill_quote(&kill(HitCause::Gun, Run)), Quote::EnemyKill);
        assert_eq!(kill_quote(&kill(HitCause::Melee, Walk)), Quote::MeleeKill);
        assert_eq!(kill_quote(&kill(HitCause::Phd, Walk)), Quote::PhdKill);
        assert_eq!(kill_quote(&kill(HitCause::ThrowingKnife, Walk)), Quote::ThrowingKnifeKill);
    }
}
