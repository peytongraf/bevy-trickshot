//! `Zombies`' man on the radio, Sergeant Frank Maddox: a handler talking to
//! the whole squad at once, not from anywhere in the world. The server says
//! when he speaks (`server::radio`, [`crate::RadioSaid`]) and — knowing how
//! long each line runs — keeps every operator quiet meanwhile, and holds the
//! first dog round's start and the first boss round's boss back until he's
//! done (`server::zombies`). The lines live
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
}

impl RadioLine {
    pub const ALL: [Self; 3] = [Self::Intro, Self::FirstDogRound, Self::FirstBossRound];

    /// Its clip, under `client/assets/`.
    pub fn path(self) -> &'static str {
        match self {
            Self::Intro => "audio/radio/maddox/intro.mp3",
            Self::FirstDogRound => "audio/radio/maddox/first_dog_round.mp3",
            Self::FirstBossRound => "audio/radio/maddox/first_boss_round.mp3",
        }
    }

    /// How long (s) it runs (`ffprobe` of the clip — update it with the
    /// clip).
    pub fn secs(self) -> f32 {
        match self {
            Self::Intro => 33.96,
            Self::FirstDogRound => 11.15,
            Self::FirstBossRound => 18.99,
        }
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
