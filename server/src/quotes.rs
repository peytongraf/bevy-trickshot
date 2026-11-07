//! `Zombies` operator quotes, directed: whatever happens in a game that a
//! player's operator has lines for ([`shared::quotes::Quote`]) asks for one
//! ([`SayQuote`]), and this decides whether it's said, by whom, and which
//! line — and tells the whole lobby ([`QuoteSaid`]), so everyone hears the
//! same line, from the speaker.
//!
//! * A quote said "by anyone" (a power-up, the first boss, a dog round, the
//!   exfil) goes to one player picked at random (a living one, if any's
//!   up).
//! * Each quote's [`Rule`](shared::quotes::Rule): the chance it's said at
//!   all, and how long before the same speaker says it again.
//! * No line (the same file) is heard twice in a game within
//!   [`LINE_COOLDOWN_SECS`]: a fresh one is picked, and if every line for
//!   the moment has been heard lately, nothing's said (by anyone — "anyone"
//!   is picked among the players with a fresh line). A quote that
//!   [cycles](shared::quotes::Quote::cycles) instead has each operator say
//!   every one of their lines for it before any again (each game).
//! * One voice talks at a time in a lobby ([`Floor`]), whoever's it is: the
//!   next line waits for the one playing to finish, the most important
//!   first ([`Priority`]), for as long as it's worth waiting
//!   ([`Priority::wait_secs`]) — then it's dropped. The more important can
//!   cut off the least ([`Priority::cuts_off`]; `QuoteSaid::cut_off`), and
//!   that line's gone. A scripted line's said on cue, over anything.
//! * The man on the radio (`crate::radio`) comes first: nothing starts while
//!   he's on air or next up, and the clients cut off whatever's playing as
//!   he starts. A line worth a long wait keeps its time till he's done.
//! * The easter egg's own voices hold the floor the same way
//!   ([`shared::easter_egg::holds_voices`]); the Hierophant in his fight
//!   takes his turn with everyone else ([`Director::hierophant_says`]).
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

use shared::bot_players::{is_ally_peer, is_bot_peer};
use shared::operator::Operator;
use shared::quotes::{
    lines, Priority, Quote, DOG_ROUND_DELAY_SECS, EXFIL_DELAY_SECS, GAP_SECS, LINE_COOLDOWN_SECS,
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
    /// When they last said each quote.
    said: HashMap<Quote, f32>,
}

/// A line waiting its turn: who says what, and until when it may.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Waiting {
    speaker: PeerId,
    quote: Quote,
    until: f32,
}

/// Who's talking in one lobby's game, and who's next.
#[derive(Default)]
struct Floor {
    /// How much the line playing matters.
    playing: Option<Priority>,
    /// Talking until then (`Time::elapsed_secs`, gap included).
    until: f32,
    /// Oldest first.
    waiting: Vec<Waiting>,
}

impl Floor {
    fn talking(&self, now: f32) -> bool {
        now < self.until
    }

    fn ask(&mut self, speaker: PeerId, quote: Quote, now: f32) {
        if !self.waiting.iter().any(|w| w.speaker == speaker && w.quote == quote) {
            self.waiting.push(Waiting { speaker, quote, until: now + quote.priority().wait_secs() });
        }
    }

    /// Time passing that doesn't count against `which` waiting.
    fn hold(&mut self, dt: f32, which: impl Fn(Priority) -> bool) {
        for w in &mut self.waiting {
            if which(w.quote.priority()) {
                w.until += dt;
            }
        }
    }

    fn forget_expired(&mut self, now: f32) {
        self.waiting.retain(|w| now <= w.until);
    }

    /// The line to say now, if any (taken off the list), and whether it
    /// cuts one off: the most important waiting, oldest first, if there's
    /// nobody talking or it can cut them off.
    fn next(&mut self, now: f32) -> Option<(Waiting, bool)> {
        let best = self
            .waiting
            .iter()
            .enumerate()
            .rev()
            .max_by_key(|(_, w)| w.quote.priority())
            .map(|(i, w)| (i, w.quote.priority()))?;
        let cut = self.talking(now);
        if cut && !self.playing.is_some_and(|p| best.1.cuts_off(p)) {
            return None;
        }
        Some((self.waiting.remove(best.0), cut))
    }

    /// A line starts now and runs `secs`. (Alongside one, unless it `cut`
    /// that off: the squad chanting together.)
    fn start(&mut self, priority: Priority, secs: f32, cut: bool, now: f32) {
        let end = now + secs + GAP_SECS;
        self.until = if cut { end } else { self.until.max(end) };
        self.playing = Some(priority);
    }

    /// The radio's cut off whatever was playing.
    fn hush(&mut self) {
        self.until = 0.0;
        self.playing = None;
    }
}

#[derive(Resource, Default)]
pub(crate) struct Director {
    voices: HashMap<PeerId, Voice>,
    floors: HashMap<Entity, Floor>,
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
    /// Whether nobody in `lobby` is talking right now. `crate::radio` waits
    /// for that.
    pub(crate) fn floor_free(&self, lobby: Entity, now: f32) -> bool {
        !self.floors.get(&lobby).is_some_and(|f| f.talking(now))
    }

    /// Whether the Hierophant could say a line of `priority` right now: a
    /// scripted one always, otherwise when nobody's talking, or about to.
    pub(crate) fn hierophant_may_speak(
        &self,
        lobby: Entity,
        priority: Priority,
        radio: &crate::radio::RadioDirector,
        now: f32,
    ) -> bool {
        priority == Priority::Scripted
            || (!radio.on_air(lobby, now)
                && !radio.next_up(lobby, now)
                && self
                    .floors
                    .get(&lobby)
                    .is_none_or(|f| !f.talking(now) && !f.waiting.iter().any(|w| w.quote.priority() >= priority)))
    }

    /// The Hierophant says a line running `secs`: it takes the floor,
    /// whether the line playing was cut off ([`HierophantLine::interrupts`]
    /// tells the clients so) or there was none.
    ///
    /// [`HierophantLine::interrupts`]: shared::easter_egg::HierophantLine::interrupts
    pub(crate) fn hierophant_says(&mut self, lobby: Entity, priority: Priority, secs: f32, now: f32) {
        self.floors.entry(lobby).or_default().start(priority, secs, true, now);
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

/// Every kill a player made: their line about it — none for the easter
/// egg's Hierophant (his death is his own moment: his scream, his spirit
/// fleeing — `easter_egg::fight`), whose operator lines come from its own
/// script.
fn kill_quotes(mut kills: EventReader<ZombieKilled>, lobbies: Query<&Lobby>, mut quotes: EventWriter<SayQuote>) {
    for kill in kills.read() {
        if lobbies.get(kill.lobby).is_ok_and(|l| l.easter_egg.is_hierophant(kill.victim)) {
            continue;
        }
        quotes.write(SayQuote::by(kill.lobby, kill.killer, kill_quote(kill)));
    }
}

/// Our client says we're low on / out of ammo, can't afford what we just
/// tried to buy, or a perk we (or, from the leader, a teammate bot) bought
/// has finished its jingle — all it may ask for.
fn on_request_quote(
    trigger: Trigger<RemoteTrigger<RequestQuote>>,
    lobbies: Query<(Entity, &Lobby)>,
    mut quotes: EventWriter<SayQuote>,
) {
    let from = trigger.from;
    let RequestQuote { quote, ally } = trigger.trigger;
    let Some((lobby_e, lobby)) =
        lobbies.iter().find(|(_, l)| l.started && l.mode == GameMode::Zombies && l.has(from))
    else {
        return;
    };
    let peer = match ally {
        Some(ally) if lobby.leader == from && is_ally_peer(ally) && lobby.has(ally) => ally,
        Some(_) => return,
        None => from,
    };
    let may_ask = match quote {
        Quote::LowAmmo | Quote::OutOfAmmo | Quote::CantAfford => ally.is_none(),
        Quote::Perk(perk) => lobby.members.iter().any(|m| m.peer == peer && m.perks.contains(&perk)),
        _ => false,
    };
    if may_ask {
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
    // Paused: nothing's said, and what's waiting keeps the time it has left.
    let dt = time.delta_secs();
    let paused = |e: Entity| lobbies.get(e).is_ok_and(|(_, l)| l.paused);
    let director = &mut *director;
    // Forget lobbies whose game's over (or gone).
    director.voices.retain(|_, v| running(v.lobby).is_some());
    director.floors.retain(|e, _| running(*e).is_some());
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
        } else if secs > 0.0 || paused(q.lobby) {
            director.scheduled.push((now + secs, SayQuote { delay: QuoteDelay::None, ..*q }));
        } else {
            asks.push(*q);
        }
    }
    director.scheduled.retain_mut(|(at, q)| {
        if paused(q.lobby) {
            *at += dt;
            true
        } else if *at > now {
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
    // (Who says what, and whether it cuts off the line playing.)
    let mut said: Vec<(Entity, PeerId, Quote, bool)> = Vec::new();
    let mut chatted: HashSet<PeerId> = HashSet::new();
    let mut chanting: HashSet<Entity> = HashSet::new();
    for (i, ask) in asks.into_iter().enumerate() {
        let Some(lobby) = running(ask.lobby) else {
            continue;
        };
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
        let priority = ask.quote.priority();
        // Chatter's one ask per speaker a tick (a frag's three kills are one
        // line).
        if priority == Priority::Chatter && !chatted.insert(speaker) {
            continue;
        }
        let rule = ask.quote.rule();
        if roll(now, speaker, ask.quote, 0xc4a + i as u64) >= rule.chance {
            continue;
        }
        let voice = director.voices.entry(speaker).or_insert(Voice {
            lobby: ask.lobby,
            said: HashMap::new(),
        });
        voice.lobby = ask.lobby;
        if voice.said.get(&ask.quote).is_some_and(|t| now - t < rule.cooldown) {
            continue;
        }
        if fresh_lines(&director.heard, ask.lobby, lobby, speaker, ask.quote, now).is_empty() {
            continue;
        }
        let floor = director.floors.entry(ask.lobby).or_default();
        if priority == Priority::Scripted {
            // On cue, over anything. (The first of a squad's chant cuts off
            // what's playing; the rest go with it.)
            let cut = chanting.insert(ask.lobby) && floor.talking(now);
            said.push((ask.lobby, speaker, ask.quote, cut));
        } else {
            floor.ask(speaker, ask.quote, now);
        }
    }

    // Whose turn it is, in each lobby.
    for (&lobby_e, floor) in director.floors.iter_mut() {
        let Some(lobby) = running(lobby_e) else {
            continue;
        };
        if lobby.paused {
            floor.hold(dt, |_| true);
            continue;
        }
        let on_air = radio.on_air(lobby_e, now);
        if on_air {
            // (The clients cut off whatever was playing as he started.)
            floor.hush();
        }
        if on_air || radio.next_up(lobby_e, now) || shared::easter_egg::holds_voices(lobby) {
            floor.hold(dt, |p| p == Priority::Story);
            floor.forget_expired(now);
            continue;
        }
        floor.forget_expired(now);
        // (Nothing new alongside a scripted line that's just gone out.)
        if said.iter().any(|(e, ..)| *e == lobby_e) {
            continue;
        }
        if let Some((line, cut)) = floor.next(now) {
            said.push((lobby_e, line.speaker, line.quote, cut));
        }
    }

    for (lobby_e, speaker, quote, cut_off) in said {
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
            voice.said.insert(quote, now);
        }
        director.floors.entry(lobby_e).or_default().start(quote.priority(), options[line].secs, cut_off, now);
        let msg = QuoteSaid {
            speaker,
            quote,
            line: line as u16,
            cut_off,
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

    const ADAM: PeerId = PeerId::Netcode(1);
    const VON: PeerId = PeerId::Netcode(2);

    /// A floor with `playing` running until 10 s, at 5 s.
    fn busy(playing: Priority) -> Floor {
        let mut floor = Floor::default();
        floor.start(playing, 5.0 - GAP_SECS, false, 5.0);
        floor
    }

    #[test]
    fn one_voice_at_a_time_then_the_most_important_oldest_first() {
        let mut floor = busy(Priority::Reaction);
        floor.ask(VON, Quote::BuyAmmo, 5.0);
        floor.ask(ADAM, Quote::Pap, 5.0);
        floor.ask(VON, Quote::Perk(shared::perks::Perk::Juggernog), 5.0);
        assert_eq!(floor.next(5.0), None, "someone's talking");
        let (first, cut) = floor.next(10.0).unwrap();
        assert_eq!((first.speaker, first.quote, cut), (ADAM, Quote::Pap, false));
        assert_eq!(floor.next(10.0).unwrap().0.quote, Quote::Perk(shared::perks::Perk::Juggernog));
        assert_eq!(floor.next(10.0).unwrap().0.quote, Quote::BuyAmmo);
    }

    #[test]
    fn a_line_that_cant_get_a_turn_in_time_is_dropped_and_chatter_never_waits() {
        let mut floor = busy(Priority::Story);
        floor.ask(ADAM, Quote::MeleeKill, 5.0);
        floor.ask(ADAM, Quote::BuyAmmo, 5.0);
        floor.ask(VON, Quote::Pap, 5.0);
        floor.forget_expired(5.1);
        assert_eq!(floor.waiting.iter().map(|w| w.quote).collect::<Vec<_>>(), [Quote::BuyAmmo, Quote::Pap]);
        floor.forget_expired(10.0);
        assert_eq!(floor.next(10.0).unwrap().0.quote, Quote::Pap);
    }

    #[test]
    fn the_important_cut_off_chatter_only() {
        let mut floor = busy(Priority::Chatter);
        floor.ask(VON, Quote::BuyAmmo, 5.0);
        assert_eq!(floor.next(5.0), None, "a reaction waits");
        floor.ask(VON, Quote::TeamMateDown, 5.0);
        let (line, cut) = floor.next(5.0).unwrap();
        assert_eq!((line.quote, cut), (Quote::TeamMateDown, true));

        let mut floor = busy(Priority::Reaction);
        floor.ask(VON, Quote::TeamMateDown, 5.0);
        assert_eq!(floor.next(5.0), None, "only chatter gets cut off");
    }

    #[test]
    fn a_cut_takes_over_the_floor_and_a_chant_goes_alongside() {
        let mut floor = busy(Priority::Chatter);
        floor.start(Priority::Callout, 1.0, true, 5.0);
        assert!(floor.until < 7.0, "the long line it cut isn't still counted");
        floor.start(Priority::Scripted, 8.0, false, 5.0);
        floor.start(Priority::Scripted, 2.0, false, 5.0);
        assert!(floor.talking(12.0), "until the longest of them's done");
    }

    #[test]
    fn asking_twice_is_one_line_and_a_held_line_keeps_its_time() {
        let mut floor = busy(Priority::Story);
        floor.ask(VON, Quote::Pap, 5.0);
        floor.ask(VON, Quote::Pap, 6.0);
        assert_eq!(floor.waiting.len(), 1);
        floor.hold(100.0, |p| p == Priority::Story);
        floor.forget_expired(100.0);
        assert_eq!(floor.waiting.len(), 1);
    }

    fn kill(cause: HitCause, anim: shared::ZombieAnim) -> ZombieKilled {
        ZombieKilled {
            lobby: Entity::PLACEHOLDER,
            feet: Vec3::ZERO,
            killer: PeerId::Netcode(1),
            victim: PeerId::Netcode(2),
            cause,
            anim,
            outside: false,
            drops: true,
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
