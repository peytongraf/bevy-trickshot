//! `Zombies`' man on the radio, Sergeant Frank Maddox: a handler talking to
//! the whole squad at once, not from anywhere in the world. The server says
//! when he speaks (`server::radio`, [`crate::RadioSaid`]) and — knowing how
//! long each line runs — keeps every operator quiet meanwhile, and holds the
//! first dog round's start and the first boss round's boss back until he's
//! done (`server::zombies`). Most lines wait their turn instead
//! ([`RadioLine::cuts_in`]). The lines live
//! under `client/assets/audio/radio/maddox/` ([`RadioLine::path`]).
//!
//! Each one's a call: a burst of static as he keys the mic
//! ([`SQUELCH_START_PATH`]), the line, and a click of static as he lets go
//! ([`SQUELCH_END_PATH`]) — [`RadioLine::airtime`] all told.

use serde::{Deserialize, Serialize};

/// How long (s) after a `Zombies` game starts (everyone's loaded in) he
/// gives the [`RadioLine::Intro`].
pub const INTRO_DELAY_SECS: f32 = 1.0;
/// How long (s) into the game's first boss round he gives the
/// [`RadioLine::FirstBossRound`]...
pub const FIRST_BOSS_ROUND_DELAY_SECS: f32 = 5.0;
/// ...and how long (s) after he's done its boss comes.
pub const FIRST_BOSS_AFTER_SECS: f32 = 1.0;
/// How long (s) a line that waits its turn ([`RadioLine::cuts_in`]) holds
/// off after it's asked for, so an operator's own line about the same thing
/// (the power, a boss kill) comes first...
pub const LEAD_IN_SECS: f32 = 1.0;
/// ...and how long (s) after that it waits for every operator to finish
/// talking before it's said over them.
pub const QUIET_WAIT_SECS: f32 = 5.0;
/// The rounds he tells the squad what Der Tempel is, and who's behind it.
pub const ROUND_2_LORE_ROUND: u32 = 2;
pub const ROUND_4_LORE_ROUND: u32 = 4;
/// Seconds left on the exfil's clock when he yells what's at stake (a
/// little before halfway)...
pub const EXFIL_STAKES_SECS: u32 = 50;
/// ...when he yells about the time...
pub const EXFIL_TIME_WARNING_SECS: u32 = 30;
/// ...and kills still needed when he yells about the last of them.
pub const EXFIL_FEW_LEFT_KILLS: u32 = 5;
/// The round he tells the squad to find the power, if it's still off.
pub const POWER_STILL_OFF_ROUND: u32 = 3;

/// The static as he keys the mic, before every line, and how long (s) it
/// runs...
pub const SQUELCH_START_PATH: &str = "audio/radio/squelch_start.mp3";
pub const SQUELCH_START_SECS: f32 = 0.47;
/// ...and as he lets go, after it.
pub const SQUELCH_END_PATH: &str = "audio/radio/squelch_end.mp3";
pub const SQUELCH_END_SECS: f32 = 0.18;

/// Something he says.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RadioLine {
    /// The briefing, as the game starts — only a game starting on round 1.
    Intro,
    /// The game's first dog round is coming — played in its place; the round
    /// itself starts once he's done.
    FirstDogRound,
    /// A boss is coming, a few seconds into the game's first boss round;
    /// it's struck in once he's done.
    FirstBossRound,
    /// What the site is — Der Tempel only, on round [`ROUND_2_LORE_ROUND`]...
    Round2Lore,
    /// ...and who's behind it, on round [`ROUND_4_LORE_ROUND`].
    Round4Lore,
    /// Someone's turned the power on.
    PowerOn,
    /// Round [`POWER_STILL_OFF_ROUND`]'s started and the power's still off.
    PowerStillOff,
    /// Der Wunderfizz has woken up (`crate::wunderfizz::ACTIVE_ROUND`).
    WunderfizzWakesUp,
    /// The game's first boss is dead.
    FirstBossKill,
    /// The exfil can be called — the game's first time...
    FirstExfilAvailable,
    /// ...and every time after.
    ExfilAvailable,
    /// The exfil's been called.
    ExfilCalled,
    /// [`EXFIL_STAKES_SECS`] left on the exfil's clock: no second bird...
    ExfilStakes,
    /// ...[`EXFIL_TIME_WARNING_SECS`] left...
    ExfilTimeWarning,
    /// ...and [`EXFIL_FEW_LEFT_KILLS`] kills still needed.
    ExfilFewLeft,
    /// The Rampage Inducer's been turned on for the first time.
    FirstRampageInducerOn,
    /// Everyone's down.
    GameOver,
}

impl RadioLine {
    pub const ALL: [Self; 17] = [
        Self::Intro,
        Self::FirstDogRound,
        Self::FirstBossRound,
        Self::Round2Lore,
        Self::Round4Lore,
        Self::PowerOn,
        Self::PowerStillOff,
        Self::WunderfizzWakesUp,
        Self::FirstBossKill,
        Self::FirstExfilAvailable,
        Self::ExfilAvailable,
        Self::ExfilCalled,
        Self::ExfilStakes,
        Self::ExfilTimeWarning,
        Self::ExfilFewLeft,
        Self::FirstRampageInducerOn,
        Self::GameOver,
    ];

    /// Its clip, under `client/assets/`.
    pub fn path(self) -> &'static str {
        match self {
            Self::Intro => "audio/radio/maddox/intro.mp3",
            Self::FirstDogRound => "audio/radio/maddox/first_dog_round.mp3",
            Self::FirstBossRound => "audio/radio/maddox/first_boss_round.mp3",
            Self::Round2Lore => "audio/radio/maddox/round_2_lore.mp3",
            Self::Round4Lore => "audio/radio/maddox/round_4_lore.mp3",
            Self::PowerOn => "audio/radio/maddox/power_on.mp3",
            Self::PowerStillOff => "audio/radio/maddox/power_still_off.mp3",
            Self::WunderfizzWakesUp => "audio/radio/maddox/wunderfizz_wakes_up.mp3",
            Self::FirstBossKill => "audio/radio/maddox/first_boss_kill.mp3",
            Self::FirstExfilAvailable => "audio/radio/maddox/first_exfil_available.mp3",
            Self::ExfilAvailable => "audio/radio/maddox/exfil_available.mp3",
            Self::ExfilCalled => "audio/radio/maddox/exfil_called.mp3",
            Self::ExfilStakes => "audio/radio/maddox/exfil_50_seconds_left.mp3",
            Self::ExfilTimeWarning => "audio/radio/maddox/exfil_30_seconds_left.mp3",
            Self::ExfilFewLeft => "audio/radio/maddox/exfil_few_enemies_left.mp3",
            Self::FirstRampageInducerOn => "audio/radio/maddox/first_rampage_inducer_on.mp3",
            Self::GameOver => "audio/radio/maddox/game_over.mp3",
        }
    }

    /// How long (s) it runs (`ffprobe` of the clip — update it with the
    /// clip).
    pub fn secs(self) -> f32 {
        match self {
            Self::Intro => 10.37,
            Self::FirstDogRound => 11.15,
            Self::FirstBossRound => 18.99,
            Self::Round2Lore => 18.13,
            Self::Round4Lore => 18.76,
            Self::PowerOn => 10.68,
            Self::PowerStillOff => 9.95,
            Self::WunderfizzWakesUp => 7.47,
            Self::FirstBossKill => 8.28,
            Self::FirstExfilAvailable => 13.01,
            Self::ExfilAvailable => 2.12,
            Self::ExfilCalled => 5.33,
            Self::ExfilStakes => 12.12,
            Self::ExfilTimeWarning => 6.14,
            Self::ExfilFewLeft => 6.77,
            Self::FirstRampageInducerOn => 7.50,
            Self::GameOver => 7.16,
        }
    }

    /// Whether it's said the moment it's asked for, cutting off whatever
    /// he's in the middle of — what the game's timed around (the first dog
    /// and boss rounds wait for him) or what nothing should wait on (the
    /// exfil's climax, over in seconds). Every
    /// other line waits its turn: [`LEAD_IN_SECS`], him being off air, and
    /// (for up to [`QUIET_WAIT_SECS`]) the operators being quiet.
    pub fn cuts_in(self) -> bool {
        matches!(
            self,
            Self::Intro
                | Self::FirstDogRound
                | Self::FirstBossRound
                | Self::ExfilStakes
                | Self::ExfilTimeWarning
                | Self::ExfilFewLeft
                | Self::GameOver
        )
    }

    /// Whether, asked for while he's saying `on_air`, it cuts that off and is
    /// said at once (rather than waiting its turn) — the exfil called
    /// stopping the call that it's there to call.
    pub fn cuts_off(self, on_air: Self) -> bool {
        self == Self::ExfilCalled && matches!(on_air, Self::FirstExfilAvailable | Self::ExfilAvailable)
    }

    /// Whether it's said at most once a game (asked for again, it's dropped).
    pub fn once_per_game(self) -> bool {
        !matches!(self, Self::ExfilAvailable | Self::ExfilCalled)
    }

    /// How long (s) he's on air saying it: the line, with the static either
    /// side.
    pub fn airtime(self) -> f32 {
        SQUELCH_START_SECS + self.secs() + SQUELCH_END_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_has_its_clip() {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/assets");
        for line in RadioLine::ALL {
            assert!(assets.join(line.path()).is_file(), "{line:?}: no {}", line.path());
        }
        for path in [SQUELCH_START_PATH, SQUELCH_END_PATH] {
            assert!(assets.join(path).is_file(), "no {path}");
        }
    }
}
