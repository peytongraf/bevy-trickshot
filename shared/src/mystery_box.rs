//! The `Zombies` Mystery Box, Call of Duty's: pay [`COST`] at it and its
//! lid swings open on an amber glow, every prize it can give cycles up out
//! of it, and it settles on one ([`roll`]) — the better the prize, the
//! rarer, though the odds lean toward the good ones as the rounds go on
//! ([`weights`]). The one who paid can take it ([`crate::TakeBoxPrize`]) —
//! a gun swaps for the weapon in their hands, a lethal or tactical is one
//! more of it (the other kind of its sort dropped) — or leave it, and it sinks back in and the lid shuts.
//!
//! Where it can stand is the map's layout's
//! ([`crate::level::ZombiesLayout::mystery_boxes`], placed in the level
//! editor): a stack of cinder blocks at each spot, always there, and the box
//! on top of one of them — a random one when a game starts. Like a modern
//! Call of Duty, once it's been spun a few times at one spot every spin has
//! a growing chance ([`teddy_chance`]) of landing on the teddy bear instead:
//! the spin's refunded, the bear laughs and floats off, the box rises up
//! and is gone in a puff and a lightning strike that leaves a bear on the
//! blocks — like every other spot the box isn't on — and a moment later it
//! drops onto another spot's blocks
//! ([`BoxPhase::Leaving`] → [`BoxPhase::Away`] → [`BoxPhase::Arriving`]).
//!
//! The server runs each spin (`server::mystery_box`) and every client draws
//! it from the lobby's [`MysteryBoxSpin`] ([`crate::Lobby::mystery_box`]) and
//! where the box is ([`BoxSpot`], [`crate::Lobby::box_spot`]).

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::level::Placement;
use crate::perks::PERK_USE_HEIGHT;
use crate::weapon::WeaponId;
use crate::MapId;

/// Points a spin costs.
pub const COST: u32 = 950;

/// Half the box's width, height and depth (m) — `mystery_box.glb` as made
/// (its origin is the middle of its bottom; its front, where the lid opens
/// toward, faces +Z).
pub const HALF_EXTENTS: Vec3 = Vec3::new(0.86, 0.17, 0.26);

/// How tall (m, as made) the cinder blocks the box sits on are —
/// `mystery_box_cinder_blocks.glb`, whose origin is the ground under its
/// middle, like a spot's placement.
pub const BLOCKS_HEIGHT: f32 = 0.2;

/// Most spots a layout keeps for the box.
pub const MAX_SPOTS: usize = 8;

/// How far (m, across the ground) from the spot just in front of the box a
/// player can use it...
pub const USE_RADIUS: f32 = 1.6;
/// ...that spot being this far (m) out from the box's middle.
const USE_SPOT_AHEAD: f32 = 0.7;

/// Seconds the lid takes to swing open (or shut) — its clip's length.
pub const LID_SECS: f32 = 0.5417;
/// Seconds from paying to the box settling on its prize: the lid opens and
/// the prizes cycle up out of it, slowing down.
pub const SPIN_SECS: f32 = 4.5;
/// Seconds the prize hovers there to be taken.
pub const OFFER_SECS: f32 = 9.0;
/// Seconds a prize nobody took takes to sink back in...
pub const SINK_SECS: f32 = 1.5;
/// ...and then the lid shuts: the whole of the closing.
pub const CLOSE_SECS: f32 = SINK_SECS + LID_SECS;

/// Seconds from the bear showing to the box being gone from its spot (the
/// bear laughing and floating off, the lid shutting, the box rising and
/// vanishing — `client::mystery_box` times it all inside this)...
pub const LEAVE_SECS: f32 = 15.5;
/// ...then gone this long...
pub const AWAY_SECS: f32 = 3.0;
/// ...then dropping onto its new spot's blocks, until it can be spun again.
pub const ARRIVE_SECS: f32 = 1.5;

/// Spins at a spot that can never land on the bear...
pub const TEDDY_SAFE_SPINS: u8 = 4;
/// ...then the chance on the next, growing by this much a spin after...
const TEDDY_FIRST_CHANCE: f32 = 0.1;
const TEDDY_CHANCE_STEP: f32 = 0.05;
/// ...up to this.
const TEDDY_MAX_CHANCE: f32 = 0.5;

/// The chance a spin lands on the teddy bear after `spins_here` spins at
/// the box's spot — none with nowhere else for it to go (`spots` 1 or 0).
pub fn teddy_chance(spins_here: u8, spots: usize) -> f32 {
    if spots < 2 || spins_here < TEDDY_SAFE_SPINS {
        return 0.0;
    }
    let extra = (spins_here - TEDDY_SAFE_SPINS) as f32;
    (TEDDY_FIRST_CHANCE + TEDDY_CHANCE_STEP * extra).min(TEDDY_MAX_CHANCE)
}

/// The spot the box moves to from `from`, out of `spots`, from `r` (uniform
/// in 0..1): any other one — or `from` itself if it's the only one.
pub fn next_spot(from: u8, spots: usize, r: f32) -> u8 {
    if spots < 2 {
        return from;
    }
    let pick = ((r.clamp(0.0, 0.999_999) * (spots - 1) as f32) as usize).min(spots - 2);
    let pick = if pick >= from as usize { pick + 1 } else { pick };
    pick as u8
}

/// What the box can give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BoxPrize {
    ThrowingKnife,
    Molotov,
    Frag,
    /// A tactical ([`crate::flash_bang`]).
    FlashBang,
    MonkeyBomb,
    M17,
    Ak74,
    Sniper,
    RayGun,
}

impl BoxPrize {
    /// Every prize, worst to best — all of them cycle through a spin,
    /// whatever it can land on.
    pub const ALL: [BoxPrize; 9] = [
        BoxPrize::ThrowingKnife,
        BoxPrize::Molotov,
        BoxPrize::Frag,
        BoxPrize::FlashBang,
        BoxPrize::MonkeyBomb,
        BoxPrize::M17,
        BoxPrize::Ak74,
        BoxPrize::Sniper,
        BoxPrize::RayGun,
    ];

    /// The gun it is, if it's one.
    pub const fn gun(self) -> Option<WeaponId> {
        match self {
            BoxPrize::M17 => Some(WeaponId::M17),
            BoxPrize::Ak74 => Some(WeaponId::Ak74),
            BoxPrize::Sniper => Some(WeaponId::Sniper),
            BoxPrize::RayGun => Some(WeaponId::RayGun),
            BoxPrize::ThrowingKnife
            | BoxPrize::Molotov
            | BoxPrize::Frag
            | BoxPrize::FlashBang
            | BoxPrize::MonkeyBomb => None,
        }
    }

    /// The lethal it is, if it's one.
    pub const fn lethal(self) -> Option<crate::lethal::LethalKind> {
        match self {
            BoxPrize::ThrowingKnife => Some(crate::lethal::LethalKind::ThrowingKnife),
            BoxPrize::Molotov => Some(crate::lethal::LethalKind::Molotov),
            BoxPrize::MonkeyBomb => Some(crate::lethal::LethalKind::MonkeyBomb),
            BoxPrize::Frag => Some(crate::lethal::LethalKind::Frag),
            BoxPrize::FlashBang => Some(crate::lethal::LethalKind::FlashBang),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            BoxPrize::ThrowingKnife => "THROWING KNIFE",
            BoxPrize::Molotov => "MOLOTOV",
            BoxPrize::MonkeyBomb => "MONKEY BOMB",
            BoxPrize::Frag => "FRAG",
            BoxPrize::FlashBang => "FLASH BANG",
            BoxPrize::M17 => WeaponId::M17.label(),
            BoxPrize::Ak74 => WeaponId::Ak74.label(),
            BoxPrize::Sniper => WeaponId::Sniper.label(),
            BoxPrize::RayGun => WeaponId::RayGun.label(),
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Each prize's weight ([`BoxPrize::ALL`]'s order) on round 1...
const EARLY_WEIGHTS: [f32; 9] = [30.0, 21.0, 16.0, 12.0, 9.0, 8.0, 6.0, 4.0, 2.0];
/// ...leaning, round by round, to these by [`LATE_ROUND`] (and on after).
const LATE_WEIGHTS: [f32; 9] = [2.0, 2.0, 3.0, 3.0, 4.0, 3.0, 5.0, 7.0, 72.0];
pub const LATE_ROUND: u32 = 55;

/// Each prize's weight on `round` — the throwing knife's the most likely
/// early on and the Ray Gun the least, until by [`LATE_ROUND`] the Ray Gun's
/// by far the likeliest.
pub fn weights(round: u32) -> [f32; 9] {
    let t = (round.max(1) - 1) as f32 / (LATE_ROUND - 1) as f32;
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| EARLY_WEIGHTS[i] + (LATE_WEIGHTS[i] - EARLY_WEIGHTS[i]) * t)
}

/// The chance of each prize on `round`, leaving out what `excluded` says
/// can't be given (all zero if nothing can).
pub fn chances(round: u32, excluded: impl Fn(BoxPrize) -> bool) -> [f32; 9] {
    let mut w = weights(round);
    for p in BoxPrize::ALL {
        if excluded(p) {
            w[p.index()] = 0.0;
        }
    }
    let total: f32 = w.iter().sum();
    if total <= 0.0 {
        return [0.0; 9];
    }
    w.map(|x| x / total)
}

/// The prize a spin lands on, on `round`, from `r` (uniform in 0..1) —
/// never one `excluded` rules out (what the player already carries); `None`
/// if that's everything.
pub fn roll(round: u32, excluded: impl Fn(BoxPrize) -> bool, r: f32) -> Option<BoxPrize> {
    let chances = chances(round, excluded);
    let mut left = r.clamp(0.0, 0.999_999);
    let mut last = None;
    for p in BoxPrize::ALL {
        let c = chances[p.index()];
        if c <= 0.0 {
            continue;
        }
        last = Some(p);
        if left < c {
            return Some(p);
        }
        left -= c;
    }
    last
}

/// Where a spin's up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxPhase {
    /// The lid opening, the prizes cycling up ([`SPIN_SECS`]).
    Spinning,
    /// The prize hovering, there to take ([`OFFER_SECS`]).
    Offering,
    /// Taken, or sinking back in, then the lid shutting ([`CLOSE_SECS`]).
    Closing,
    /// It landed on the teddy bear: the box leaving its spot ([`LEAVE_SECS`]).
    Leaving,
    /// Gone from the map ([`AWAY_SECS`]).
    Away,
    /// Dropping onto its new spot ([`BoxSpot::at`], already moved there —
    /// [`ARRIVE_SECS`]).
    Arriving,
}

/// A spin of the box: `user` paid for it, and it lands on `prize` — or on
/// the teddy bear (`teddy`), and the box moves. `id` tells one spin from the
/// next. Server-owned, cleared once the lid's shut (or the box has landed
/// on its new spot) and whenever a game starts or ends.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MysteryBoxSpin {
    pub id: u32,
    pub user: lightyear::prelude::PeerId,
    pub prize: BoxPrize,
    pub phase: BoxPhase,
    /// The prize was taken (so it's gone, not sinking back in).
    pub taken: bool,
    /// It lands on the teddy bear instead of `prize`: the box moves.
    pub teddy: bool,
    /// Points paid for it — handed back if it lands on the bear (none for
    /// the debug panel's free spin).
    pub paid: u32,
}

/// Where the box is in a game: which of the layout's spots
/// ([`crate::level::ZombiesLayout::mystery_boxes`]) it's on, and how many
/// spins it's had there. (A teddy bear sits on every other spot's blocks.)
/// Server-owned; set afresh whenever a game starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxSpot {
    pub at: u8,
    pub spins: u8,
}

impl BoxSpot {
    /// A game's start on `map`: on a random spot (from `r`, uniform in 0..1).
    pub fn start(map: MapId, r: f32) -> Self {
        let n = spots(map).len();
        let at = if n == 0 { 0 } else { ((r.clamp(0.0, 0.999_999) * n as f32) as usize).min(n - 1) as u8 };
        Self { at, spins: 0 }
    }
}

/// Every spot the box can stand on, on `map` (each a stack of cinder
/// blocks, the box on one of them).
pub fn spots(map: MapId) -> &'static [Placement] {
    &crate::level::layout(map).mystery_boxes
}

/// Spot `i` on `map`, if it has one.
pub fn spot(map: MapId, i: u8) -> Option<Placement> {
    spots(map).get(i as usize).copied()
}

/// The spot the box is on in `lobby`'s game (the ground under its blocks),
/// if the map has any.
pub fn current(lobby: &crate::Lobby) -> Option<Placement> {
    spot(lobby.map, lobby.box_spot.at)
}

/// The box itself, standing on the blocks at spot `at`.
pub fn on_blocks(at: Placement) -> Placement {
    Placement {
        pos: at.pos + Vec3::Y * BLOCKS_HEIGHT * at.scale,
        ..at
    }
}

/// Where a player stands to use a box standing at `at`.
pub fn use_spot(at: Placement) -> Vec3 {
    at.pos + at.rotation() * Vec3::Z * (USE_SPOT_AHEAD * at.scale.max(0.5))
}

/// Whether feet at `feet` can use a box standing at `at` (`slack` widens
/// it — the server allows for the pose being a moment old).
pub fn in_range_of(at: Placement, feet: Vec3, slack: f32) -> bool {
    let spot = use_spot(at);
    Vec3::new(feet.x - spot.x, 0.0, feet.z - spot.z).length() <= USE_RADIUS + slack
        && (feet.y - at.pos.y).abs() <= PERK_USE_HEIGHT + slack
}

/// Whether feet at `feet` can use the box in `lobby`'s game, where it is.
pub fn in_range(lobby: &crate::Lobby, feet: Vec3, slack: f32) -> bool {
    current(lobby).is_some_and(|at| in_range_of(at, feet, slack))
}

/// Half the size (m, as made) of a spot's solid stack: the blocks and the
/// box on them.
pub fn stack_half_extents() -> Vec3 {
    Vec3::new(HALF_EXTENTS.x, BLOCKS_HEIGHT * 0.5 + HALF_EXTENTS.y, HALF_EXTENTS.z)
}

/// Each spot's solid box on `map`: `(centre, rotation, half extents)` — the
/// blocks and a box on them, whether or not the box is there right now
/// (it's only ever away for a few seconds).
pub fn solid_boxes(map: MapId) -> Vec<(Vec3, Quat, Vec3)> {
    spots(map)
        .iter()
        .map(|at| {
            let half = stack_half_extents() * at.scale;
            (at.pos + Vec3::Y * half.y, at.rotation(), half)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: BoxPrize) -> bool {
        false
    }

    #[test]
    fn early_on_the_knife_is_likeliest_and_the_ray_gun_rarest() {
        let c = chances(1, none);
        for p in BoxPrize::ALL {
            assert!(c[BoxPrize::ThrowingKnife.index()] >= c[p.index()]);
            assert!(c[BoxPrize::RayGun.index()] <= c[p.index()]);
        }
        // Better is rarer, all the way up.
        assert!(c.windows(2).all(|w| w[0] > w[1]), "{c:?}");
    }

    #[test]
    fn the_ray_gun_gets_likelier_every_round_until_its_very_likely() {
        let ray = |round| chances(round, none)[BoxPrize::RayGun.index()];
        for round in 1..LATE_ROUND {
            assert!(ray(round + 1) > ray(round), "round {round}");
        }
        assert!(ray(LATE_ROUND) >= 0.7, "{}", ray(LATE_ROUND));
        assert_eq!(ray(LATE_ROUND), ray(80));
    }

    #[test]
    fn it_never_lands_on_whats_ruled_out() {
        let carried = |p: BoxPrize| matches!(p, BoxPrize::RayGun | BoxPrize::ThrowingKnife);
        for i in 0..1000 {
            let p = roll(30, carried, i as f32 / 1000.0).unwrap();
            assert!(!carried(p), "{p:?}");
        }
        assert_eq!(roll(30, |_| true, 0.5), None);
        // Only one left: always that one.
        assert_eq!(roll(1, |p| p != BoxPrize::Sniper, 0.99), Some(BoxPrize::Sniper));
    }

    #[test]
    fn the_rolls_follow_the_chances() {
        let c = chances(1, none);
        let n = 10_000;
        let knives = (0..n)
            .filter(|i| roll(1, none, *i as f32 / n as f32) == Some(BoxPrize::ThrowingKnife))
            .count();
        assert!((knives as f32 / n as f32 - c[0]).abs() < 0.01);
    }

    #[test]
    fn the_bear_only_comes_after_a_few_spins_and_gets_likelier() {
        for spins in 0..TEDDY_SAFE_SPINS {
            assert_eq!(teddy_chance(spins, 3), 0.0);
        }
        let mut last = 0.0;
        for spins in TEDDY_SAFE_SPINS..40 {
            let c = teddy_chance(spins, 3);
            assert!(c > 0.0 && c >= last && c <= TEDDY_MAX_CHANCE, "{spins}: {c}");
            last = c;
        }
        // Nowhere to go: never.
        assert_eq!(teddy_chance(30, 1), 0.0);
    }

    #[test]
    fn it_always_moves_somewhere_else() {
        for spots in 2..=MAX_SPOTS {
            for from in 0..spots as u8 {
                let mut seen = vec![false; spots];
                for i in 0..1000 {
                    let to = next_spot(from, spots, i as f32 / 1000.0);
                    assert!((to as usize) < spots && to != from, "{from} → {to} of {spots}");
                    seen[to as usize] = true;
                }
                assert_eq!(seen.iter().filter(|s| **s).count(), spots - 1, "every other spot comes up");
            }
        }
        assert_eq!(next_spot(0, 1, 0.5), 0);
    }

    #[test]
    fn its_used_from_in_front() {
        let at = Placement::new(Vec3::new(2.0, 1.0, 5.0), 90.0);
        // (Turned 90°: its front faces +X.)
        assert!(in_range_of(at, at.pos + Vec3::X * 1.2, 0.0));
        assert!(!in_range_of(at, at.pos - Vec3::X * 2.0, 0.0));
    }
}
