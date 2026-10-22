//! [`crate::GameMode::SearchAndDestroy`], Call of Duty's: two teams, one
//! life a round. The attackers carry a bomb to one of two sites (A or B)
//! and plant it; the defenders stop them, or defuse it once it's down.
//!
//! A round is won by:
//! * killing every enemy — though once the bomb's planted, killing every
//!   attacker isn't enough: it still has to be defused;
//! * the bomb going off (the attackers), or being defused (the defenders);
//! * the clock running out before it's planted (the defenders).
//!
//! The first team to [`crate::Lobby::snd_rounds_to_win`] rounds wins the
//! match; the teams swap sides every [`SWITCH_SIDES_EVERY`] rounds. Nobody
//! respawns until the next round — the dead watch a teammate. The server
//! runs it all (`server::search_destroy`) and replicates it on the lobby
//! ([`SndState`]); every member picks their own [`Team`] in the lobby room.
//!
//! Where each team starts and where the sites are on each map is
//! [`layout`].

use bevy::math::Vec3;
use lightyear::prelude::PeerId;
use serde::{Deserialize, Serialize};

use crate::MapId;

/// Seconds everyone's held at their spawn before a round starts.
pub const PRE_ROUND_SECS: u32 = 5;
/// Seconds the attackers have to plant the bomb.
pub const ROUND_SECS: u32 = 150;
/// Seconds the bomb burns once it's planted...
pub const FUSE_SECS: u32 = 45;
/// ...holding interact at a site to plant it...
pub const PLANT_SECS: f32 = 5.0;
/// ...and at the bomb to defuse it. Letting go starts either over.
pub const DEFUSE_SECS: f32 = 7.5;
/// Seconds the round's result is shown before the next one.
pub const ROUND_OVER_SECS: u32 = 6;
/// How far (m, across the ground) from a site's middle the bomb can be
/// planted...
pub const SITE_RADIUS: f32 = 3.0;
/// ...from the planted bomb it can be defused...
pub const DEFUSE_RADIUS: f32 = 2.0;
/// ...and from a dropped bomb an attacker picks it up, walking over it.
pub const PICKUP_RADIUS: f32 = 1.5;
/// How far up or down (m) from any of those still counts.
pub const USE_HEIGHT: f32 = 1.5;
/// Everyone within this (m) of the bomb when it goes off dies.
pub const BLAST_RADIUS: f32 = 12.0;
/// The teams swap sides after this many rounds (and again after as many
/// more).
pub const SWITCH_SIDES_EVERY: u32 = 2;
/// Rounds to win a match: the default, and the leader's range.
pub const DEFAULT_ROUNDS_TO_WIN: u32 = 6;
pub const MIN_ROUNDS_TO_WIN: u32 = 1;
pub const MAX_ROUNDS_TO_WIN: u32 = 10;
/// How far (m) from its team's spawn spot each player starts a round.
pub const SPAWN_SPREAD: f32 = 3.5;

/// One of the two teams. Every member's on one ([`crate::LobbyMember::team`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Team {
    #[default]
    Red,
    Blue,
}

impl Team {
    pub const ALL: [Team; 2] = [Team::Red, Team::Blue];

    pub fn index(self) -> usize {
        match self {
            Team::Red => 0,
            Team::Blue => 1,
        }
    }

    pub fn other(self) -> Team {
        match self {
            Team::Red => Team::Blue,
            Team::Blue => Team::Red,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Team::Red => "RED",
            Team::Blue => "BLUE",
        }
    }

    /// Its colour (linear-ish sRGB), for the lobby room and the HUD.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            Team::Red => [0.92, 0.3, 0.26],
            Team::Blue => [0.3, 0.58, 0.95],
        }
    }
}

/// One of the two bomb sites.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Site {
    A,
    B,
}

impl Site {
    pub const ALL: [Site; 2] = [Site::A, Site::B];

    pub fn label(self) -> &'static str {
        match self {
            Site::A => "A",
            Site::B => "B",
        }
    }
}

/// Where everything is on one map (feet / ground positions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SndLayout {
    /// Where the attackers start every round...
    pub attack_spawn: Vec3,
    /// ...and the defenders.
    pub defend_spawn: Vec3,
    /// The two sites' middles, A then B.
    pub sites: [Vec3; 2],
}

impl SndLayout {
    pub fn site(&self, site: Site) -> Vec3 {
        self.sites[site as usize]
    }

    /// Which way (yaw, the game's convention: 0 faces -Z) a player starting
    /// at `from` faces: toward the sites between the two spawns.
    pub fn facing(&self, from: Vec3) -> f32 {
        let to = (self.sites[0] + self.sites[1]) * 0.5 - from;
        f32::atan2(-to.x, -to.z)
    }
}

/// `map`'s spawns and sites: the attackers at one end, the defenders at
/// the other, the sites much nearer the defenders. (Each spot is on
/// walkable ground every spawn can reach — `server::nav`'s tests check.)
pub fn layout(map: MapId) -> SndLayout {
    match map {
        // The two raised strips either end, the sites out on the open
        // ground in front of the east one, either side of the tall block.
        MapId::BasicMap | MapId::BasicMapNight => SndLayout {
            attack_spawn: Vec3::new(-60.0, 2.6, 7.7),
            defend_spawn: Vec3::new(92.0, 2.6, 7.7),
            sites: [Vec3::new(50.0, 0.0, -22.3), Vec3::new(50.0, 0.0, 35.7)],
        },
        // Either end (two of its hand-placed spawns), the sites either side
        // of the middle row of containers on the defenders' half.
        MapId::Shipment | MapId::ShipmentDay => SndLayout {
            attack_spawn: Vec3::new(6.7, 0.0, -27.5),
            defend_spawn: Vec3::new(-2.5, 0.0, 23.5),
            sites: [Vec3::new(-20.1, 0.0, 9.7), Vec3::new(19.9, 0.0, 9.7)],
        },
        // South end to north end, the sites out on the open ground north of
        // the buildings.
        MapId::BreakPoint | MapId::BreakPointNight => SndLayout {
            attack_spawn: Vec3::new(19.9, 0.0, -52.1),
            defend_spawn: Vec3::new(-0.1, 0.0, 54.9),
            sites: [Vec3::new(-20.1, 0.0, 31.9), Vec3::new(21.9, 0.0, 39.9)],
        },
        // The south courtyard to the north one (the exfil's), the sites
        // either side of the pyramid on the way.
        MapId::DerTempel | MapId::DerTempelDay => SndLayout {
            attack_spawn: Vec3::new(-0.2, 0.0, 50.5),
            defend_spawn: Vec3::new(-0.2, 0.0, -66.5),
            sites: [Vec3::new(-30.2, 1.0, -29.5), Vec3::new(28.8, 0.0, -30.5)],
        },
    }
}

/// Whether `feet` is close enough to `at` across the ground (`radius`) and
/// up or down ([`USE_HEIGHT`]).
pub fn within(at: Vec3, feet: Vec3, radius: f32) -> bool {
    let d = feet - at;
    Vec3::new(d.x, 0.0, d.z).length() <= radius && d.y.abs() <= USE_HEIGHT
}

/// The site `feet` is at (to plant), if any.
pub fn site_at(map: MapId, feet: Vec3) -> Option<Site> {
    let l = layout(map);
    Site::ALL.into_iter().find(|&s| within(l.site(s), feet, SITE_RADIUS))
}

/// The team with fewer members in `lobby` (red on a tie) — where someone
/// joining goes.
pub fn smaller_team(lobby: &crate::Lobby) -> Team {
    let on = |t: Team| lobby.members.iter().filter(|m| m.team == t).count();
    if on(Team::Blue) < on(Team::Red) {
        Team::Blue
    } else {
        Team::Red
    }
}

/// Which team attacks in `round` (1-based): red first, swapping every
/// [`SWITCH_SIDES_EVERY`] rounds.
pub fn attackers_in(round: u32) -> Team {
    if (round.saturating_sub(1) / SWITCH_SIDES_EVERY) % 2 == 0 {
        Team::Red
    } else {
        Team::Blue
    }
}

/// What a round's in the middle of.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    /// Everyone held at their spawn, the countdown running.
    #[default]
    PreRound,
    /// Under way.
    Live,
    /// Over: who won, and how — shown until the next one.
    Over { winner: Team, how: RoundEnd },
}

/// How a round was won.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundEnd {
    /// Every enemy's dead.
    Eliminated,
    Detonated,
    Defused,
    /// The clock ran out with the bomb never planted.
    TimeUp,
}

impl RoundEnd {
    pub fn label(self) -> &'static str {
        match self {
            RoundEnd::Eliminated => "ENEMY TEAM ELIMINATED",
            RoundEnd::Detonated => "BOMB DETONATED",
            RoundEnd::Defused => "BOMB DEFUSED",
            RoundEnd::TimeUp => "TIME EXPIRED",
        }
    }
}

/// Where the bomb is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub enum Bomb {
    /// Not in play (before the first round).
    #[default]
    None,
    /// An attacker has it.
    Carried(PeerId),
    /// Lying where its carrier died, for any attacker to pick up.
    Dropped([f32; 3]),
    /// Down at `site`, burning.
    Planted { site: Site, at: [f32; 3] },
}

impl Bomb {
    pub fn planted(self) -> bool {
        matches!(self, Bomb::Planted { .. })
    }
}

/// A running `SearchAndDestroy` game, server-owned and replicated on the
/// lobby ([`crate::Lobby::snd`]) for everyone's HUD. Cleared whenever a
/// game starts (kept once it's over, for the results screen).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SndState {
    /// The round being played (1-based; 0 before the first).
    pub round: u32,
    /// Rounds won so far, by [`Team::index`].
    pub wins: [u32; 2],
    /// Who's attacking this round.
    pub attackers: Team,
    pub phase: Phase,
    /// Whole seconds left on whatever's counting down: the pre-round
    /// countdown, the round clock — the fuse once the bomb's planted — or
    /// the pause after a round.
    pub secs_left: u32,
    pub bomb: Bomb,
    /// Who's planting or defusing right now (each client times the bar from
    /// when this appears — it starts over whenever it changes).
    pub using: Option<PeerId>,
    /// Who's been killed this round — out until the next.
    pub dead: Vec<PeerId>,
}

impl SndState {
    pub fn wins_of(&self, team: Team) -> u32 {
        self.wins[team.index()]
    }

    pub fn is_dead(&self, peer: PeerId) -> bool {
        self.dead.contains(&peer)
    }

    /// Whether `team`'s attacking this round.
    pub fn attacking(&self, team: Team) -> bool {
        self.attackers == team
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_swap_every_few_rounds() {
        let sides: Vec<Team> = (1..=6).map(attackers_in).collect();
        assert_eq!(sides[0], Team::Red);
        assert_eq!(sides[SWITCH_SIDES_EVERY as usize], Team::Blue);
        assert_eq!(sides[SWITCH_SIDES_EVERY as usize - 1], Team::Red);
        assert_eq!(attackers_in(1 + 2 * SWITCH_SIDES_EVERY), Team::Red);
    }

    #[test]
    fn every_map_has_two_distinct_sites_nearer_the_defenders() {
        for map in MapId::ALL {
            let l = layout(map);
            assert!(l.sites[0].distance(l.sites[1]) > 2.0 * SITE_RADIUS, "{map:?}: the sites overlap");
            for s in l.sites {
                assert!(s.distance(l.defend_spawn) < s.distance(l.attack_spawn), "{map:?}: a site's nearer the attackers");
            }
        }
    }

    #[test]
    fn a_site_counts_only_close_by_and_on_its_level() {
        let l = layout(MapId::Shipment);
        let a = l.site(Site::A);
        assert_eq!(site_at(MapId::Shipment, a + Vec3::X * (SITE_RADIUS - 0.1)), Some(Site::A));
        assert_eq!(site_at(MapId::Shipment, a + Vec3::X * (SITE_RADIUS + 0.1)), None);
        assert_eq!(site_at(MapId::Shipment, a + Vec3::Y * (USE_HEIGHT + 0.5)), None);
        assert_eq!(site_at(MapId::Shipment, l.site(Site::B)), Some(Site::B));
    }
}
