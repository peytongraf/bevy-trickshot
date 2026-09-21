//! Bot *players* for [`crate::GameMode::FreeForAll`]: computer-controlled
//! opponents that sit in the lobby's party list like real members, move around
//! the map, and shoot at everyone else (real players and other bots).
//!
//! Not to be confused with [`crate::bots`], the stationary target dummies of
//! `Freestyle`.
//!
//! Each bot is a lobby member with a fake [`PeerId`] (see [`bot_peer`]) and a
//! [`BotDifficulty`]; the server (`server::ai`) drives one player entity per
//! bot through the very same input path a real client uses.

use lightyear::prelude::PeerId;
use serde::{Deserialize, Serialize};

/// Most bots one lobby can hold at once.
pub const MAX_BOTS: usize = 20;

/// A bot's fake peer id is `PeerId::Local(BOT_PEER_BASE + n)` — `Local` is
/// never what a real netcode client gets, so it can't collide with a player.
pub const BOT_PEER_BASE: u64 = 0xB07_0000_0000;

/// The fake peer id for the `n`th bot the server has ever created.
pub fn bot_peer(n: u64) -> PeerId {
    PeerId::Local(BOT_PEER_BASE + n)
}

/// Whether `peer` is a bot (as opposed to a real connected player). Anything
/// that sends a message to a peer must skip these — there's no client behind
/// them.
pub fn is_bot_peer(peer: PeerId) -> bool {
    matches!(peer, PeerId::Local(n) if n >= BOT_PEER_BASE)
}

/// One basic name for each of the [`MAX_BOTS`] possible bots — a bot gets a
/// random one nobody else in its lobby is using.
pub const BOT_NAMES: [&str; MAX_BOTS] = [
    "Alex", "Blake", "Casey", "Drew", "Ellis", "Finn", "Gray", "Harper", "Indy", "Jordan", "Kai",
    "Logan", "Morgan", "Nico", "Owen", "Parker", "Quinn", "Riley", "Sam", "Tyler",
];

/// How good a bot is — Call of Duty's four bot difficulty names.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BotDifficulty {
    #[default]
    Recruit,
    Regular,
    Hardened,
    Veteran,
}

/// What a [`BotDifficulty`] means in play — see [`BotDifficulty::skill`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotSkill {
    /// Seconds an enemy has to stay in view before the bot starts aiming at it.
    pub reaction_secs: f32,
    /// Peak random aim error (degrees) on each shot.
    pub aim_error_deg: f32,
    /// Seconds between shots (the sniper is bolt-action, so never faster than
    /// the weapon cycles).
    pub fire_interval_secs: f32,
    /// How fast the bot swings its aim onto a target (degrees / second).
    pub turn_deg_per_sec: f32,
    /// Farthest (m) at which the bot notices an enemy it has line of sight to.
    pub sight_range: f32,
    /// Whether the bot sprints when closing on a far target (else it walks).
    pub sprints: bool,
}

impl BotDifficulty {
    /// Every difficulty, easiest first.
    pub const ALL: [BotDifficulty; 4] = [
        BotDifficulty::Recruit,
        BotDifficulty::Regular,
        BotDifficulty::Hardened,
        BotDifficulty::Veteran,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BotDifficulty::Recruit => "RECRUIT",
            BotDifficulty::Regular => "REGULAR",
            BotDifficulty::Hardened => "HARDENED",
            BotDifficulty::Veteran => "VETERAN",
        }
    }

    pub fn skill(self) -> BotSkill {
        match self {
            BotDifficulty::Recruit => BotSkill {
                reaction_secs: 1.4,
                aim_error_deg: 7.0,
                fire_interval_secs: 3.8,
                turn_deg_per_sec: 90.0,
                sight_range: 70.0,
                sprints: false,
            },
            BotDifficulty::Regular => BotSkill {
                reaction_secs: 0.9,
                aim_error_deg: 4.0,
                fire_interval_secs: 2.8,
                turn_deg_per_sec: 150.0,
                sight_range: 90.0,
                sprints: false,
            },
            BotDifficulty::Hardened => BotSkill {
                reaction_secs: 0.55,
                aim_error_deg: 2.0,
                fire_interval_secs: 2.1,
                turn_deg_per_sec: 240.0,
                sight_range: 120.0,
                sprints: true,
            },
            BotDifficulty::Veteran => BotSkill {
                reaction_secs: 0.3,
                aim_error_deg: 0.7,
                fire_interval_secs: 1.7,
                turn_deg_per_sec: 380.0,
                sight_range: 160.0,
                sprints: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_is_a_name_for_every_possible_bot_and_they_are_unique() {
        let mut names: Vec<&str> = BOT_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MAX_BOTS);
    }

    #[test]
    fn harder_bots_react_faster_aim_better_and_shoot_more() {
        let skills: Vec<BotSkill> = BotDifficulty::ALL.iter().map(|d| d.skill()).collect();
        for w in skills.windows(2) {
            assert!(w[1].reaction_secs < w[0].reaction_secs);
            assert!(w[1].aim_error_deg < w[0].aim_error_deg);
            assert!(w[1].fire_interval_secs < w[0].fire_interval_secs);
            assert!(w[1].turn_deg_per_sec > w[0].turn_deg_per_sec);
            assert!(w[1].sight_range > w[0].sight_range);
        }
    }

    #[test]
    fn bot_peers_are_recognised_and_real_ones_are_not() {
        assert!(is_bot_peer(bot_peer(0)));
        assert!(is_bot_peer(bot_peer(19)));
        assert!(!is_bot_peer(PeerId::Netcode(12345)));
        assert!(!is_bot_peer(PeerId::Local(3)));
        assert!(!is_bot_peer(PeerId::Server));
    }

    #[test]
    fn the_four_difficulties_use_call_of_dutys_names() {
        let labels: Vec<&str> = BotDifficulty::ALL.iter().map(|d| d.label()).collect();
        assert_eq!(labels, ["RECRUIT", "REGULAR", "HARDENED", "VETERAN"]);
    }
}
