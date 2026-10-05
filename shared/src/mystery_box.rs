//! The `Zombies` Mystery Box, Call of Duty's: pay [`COST`] at it and its
//! lid swings open on an amber glow, every prize it can give cycles up out
//! of it, and it settles on one ([`roll`]) — the better the prize, the
//! rarer, though the odds lean toward the good ones as the rounds go on
//! ([`weights`]). The one who paid can take it ([`crate::TakeBoxPrize`]) —
//! a gun swaps for the weapon in their hands, a lethal fills them up with
//! it — or leave it, and it sinks back in and the lid shuts.
//!
//! Where it stands is the map's layout's ([`crate::level::ZombiesLayout::mystery_box`],
//! placed in the level editor); it never moves. The server runs each spin
//! (`server::mystery_box`) and every client draws it from the lobby's
//! [`MysteryBoxSpin`] ([`crate::Lobby::mystery_box`]).

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::level::Placement;
use crate::perks::PERK_USE_HEIGHT;
use crate::weapon::WeaponId;
use crate::MapId;

/// Points a spin costs.
pub const COST: u32 = 950;

/// Half the box's width, height and depth (m) — `mystery_box.glb` as made
/// (its origin is the ground under its middle; its front, where the lid
/// opens toward, faces +Z).
pub const HALF_EXTENTS: Vec3 = Vec3::new(0.86, 0.17, 0.26);

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

/// What the box can give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BoxPrize {
    ThrowingKnife,
    Molotov,
    Ak74,
    Sniper,
    RayGun,
}

impl BoxPrize {
    /// Every prize, worst to best — all of them cycle through a spin,
    /// whatever it can land on.
    pub const ALL: [BoxPrize; 5] = [
        BoxPrize::ThrowingKnife,
        BoxPrize::Molotov,
        BoxPrize::Ak74,
        BoxPrize::Sniper,
        BoxPrize::RayGun,
    ];

    /// The gun it is, if it's one.
    pub const fn gun(self) -> Option<WeaponId> {
        match self {
            BoxPrize::Ak74 => Some(WeaponId::Ak74),
            BoxPrize::Sniper => Some(WeaponId::Sniper),
            BoxPrize::RayGun => Some(WeaponId::RayGun),
            BoxPrize::ThrowingKnife | BoxPrize::Molotov => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            BoxPrize::ThrowingKnife => "THROWING KNIFE",
            BoxPrize::Molotov => "MOLOTOV",
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
const EARLY_WEIGHTS: [f32; 5] = [40.0, 28.0, 18.0, 11.0, 3.0];
/// ...leaning, round by round, to these by [`LATE_ROUND`] (and on after).
const LATE_WEIGHTS: [f32; 5] = [3.0, 4.0, 6.0, 9.0, 78.0];
pub const LATE_ROUND: u32 = 55;

/// Each prize's weight on `round` — the throwing knife's the most likely
/// early on and the Ray Gun the least, until by [`LATE_ROUND`] the Ray Gun's
/// by far the likeliest.
pub fn weights(round: u32) -> [f32; 5] {
    let t = (round.max(1) - 1) as f32 / (LATE_ROUND - 1) as f32;
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| EARLY_WEIGHTS[i] + (LATE_WEIGHTS[i] - EARLY_WEIGHTS[i]) * t)
}

/// The chance of each prize on `round`, leaving out what `excluded` says
/// can't be given (all zero if nothing can).
pub fn chances(round: u32, excluded: impl Fn(BoxPrize) -> bool) -> [f32; 5] {
    let mut w = weights(round);
    for p in BoxPrize::ALL {
        if excluded(p) {
            w[p.index()] = 0.0;
        }
    }
    let total: f32 = w.iter().sum();
    if total <= 0.0 {
        return [0.0; 5];
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
}

/// A spin of the box: `user` paid for it, and it lands on `prize`. `id`
/// tells one spin from the next. Server-owned, cleared once the lid's shut
/// and whenever a game starts or ends.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MysteryBoxSpin {
    pub id: u32,
    pub user: lightyear::prelude::PeerId,
    pub prize: BoxPrize,
    pub phase: BoxPhase,
    /// The prize was taken (so it's gone, not sinking back in).
    pub taken: bool,
}

/// Where the box stands on `map`, if it has one.
pub fn placement(map: MapId) -> Option<Placement> {
    crate::level::layout(map).mystery_box
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

/// Whether feet at `feet` can use `map`'s box.
pub fn in_range(map: MapId, feet: Vec3, slack: f32) -> bool {
    placement(map).is_some_and(|at| in_range_of(at, feet, slack))
}

/// The box's solid box on `map`, if it has one: `(centre, rotation, half
/// extents)`.
pub fn solid_box(map: MapId) -> Option<(Vec3, Quat, Vec3)> {
    placement(map).map(|at| {
        let half = HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
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
    fn its_used_from_in_front() {
        let at = Placement::new(Vec3::new(2.0, 1.0, 5.0), 90.0);
        // (Turned 90°: its front faces +X.)
        assert!(in_range_of(at, at.pos + Vec3::X * 1.2, 0.0));
        assert!(!in_range_of(at, at.pos - Vec3::X * 2.0, 0.0));
    }
}
