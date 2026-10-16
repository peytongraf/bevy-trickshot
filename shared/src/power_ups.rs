//! `Zombies` power-ups, Call of Duty style: a killed zombie sometimes drops
//! one ([`roll`]), it floats where it fell for [`DROP_LIFETIME_SECS`]
//! (blinking for the last [`BLINK_SECS`]), and the first player to walk into
//! it ([`in_pickup_range`]) sets it off for the whole team:
//!
//! * **Max Ammo** — everyone's ammo (sniper, throwing knives and any molotovs) full.
//! * **Insta-Kill** — any hit kills a zombie, for [`TIMED_SECS`].
//! * **Double Points** — zombie kills score double, for [`TIMED_SECS`].
//! * **Nuke** — its flash goes off the moment it's grabbed, the announcer
//!   a beat later ([`NUKE_ANNOUNCER_DELAY_SECS`]); [`NUKE_KILL_DELAY_SECS`]
//!   after the grab every zombie up starts dying, each at a random moment
//!   within [`NUKE_KILL_SECS`] (bursting into flames) — Cold War's timing.
//!   Everyone gets [`NUKE_POINTS`], and no more rise till
//!   [`NUKE_SPAWN_PAUSE_SECS`] after the first dies.
//! * **Bonus Points** — [`BONUS_POINTS`] for whoever grabbed it.
//! * **Carpenter** — every window's boards go back up, nearest window to the
//!   drop first, a board every [`CARPENTER_BOARD_SECS`]; once they're all up
//!   everyone gets [`CARPENTER_POINTS`]. It only drops while at least
//!   [`CARPENTER_MIN_TORN`] boards are down across the map ([`roll`]).
//!
//! A drop that would land within a player's reach — a zombie knifed, or shot
//! point blank — is set down just out of it instead ([`drop_spot`]), so
//! picking it up stays their choice.
//!
//! The server owns all of it (`server::power_ups`); clients draw the drops
//! ([`crate::PowerUpDrop`]), play the sounds on [`crate::PowerUpGrabbed`], and
//! show the running timers from [`crate::Lobby::active_power_ups`].

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::map::CollisionWorld;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PowerUp {
    MaxAmmo,
    InstaKill,
    DoublePoints,
    Nuke,
    BonusPoints,
    Carpenter,
}

/// How long (s) Insta-Kill and Double Points last. Grabbing one while it's
/// already running starts it over.
pub const TIMED_SECS: f32 = 30.0;
/// How long (s) a drop floats there before it's gone.
pub const DROP_LIFETIME_SECS: f32 = 30.0;
/// It blinks for this long (s) before it goes.
pub const BLINK_SECS: f32 = 8.0;
/// Points every player gets from a Nuke (doubled by Double Points).
pub const NUKE_POINTS: u32 = 500;
/// Points Bonus Points gives whoever grabbed it.
pub const BONUS_POINTS: u32 = 500;
/// Points every player gets once a Carpenter's put every board back
/// (doubled by Double Points).
pub const CARPENTER_POINTS: u32 = 200;
/// Seconds between the boards a Carpenter puts back.
pub const CARPENTER_BOARD_SECS: f32 = 0.12;
/// A Carpenter only drops with at least this many boards down across all the
/// map's windows — a whole window's worth.
pub const CARPENTER_MIN_TORN: u32 = crate::windows::BOARDS as u32;
/// A Nuke flashes the moment it's grabbed; its announcer line starts this
/// long (s) after.
pub const NUKE_ANNOUNCER_DELAY_SECS: f32 = 0.65;
/// Seconds from grabbing a Nuke to its first zombie dying...
pub const NUKE_KILL_DELAY_SECS: f32 = 2.2;
/// ...and the rest die one by one, each at a random moment within this long
/// (s) of the first.
pub const NUKE_KILL_SECS: f32 = 1.5;
/// After a Nuke, no zombie rises for this long (s) — the whole time its
/// zombies are still dropping — unless it ended the round, whose own break
/// then follows.
pub const NUKE_SPAWN_PAUSE_SECS: f32 = NUKE_KILL_SECS;
/// How close (m, across the ground) a player's feet must get to a drop.
pub const PICKUP_RADIUS: f32 = 1.4;
/// ...and how far (m) above or below it.
pub const PICKUP_HEIGHT: f32 = 2.0;
/// A drop moved out of a player's reach ([`drop_spot`]) lands this much (m)
/// past it — a step away.
pub const DROP_CLEARANCE: f32 = 0.6;
/// ...on ground at most this far (m) above or below where the zombie fell
/// (not down off a ledge, nor up on one).
const DROP_STEP: f32 = 1.0;

impl PowerUp {
    pub const ALL: [PowerUp; 6] = [
        PowerUp::MaxAmmo,
        PowerUp::InstaKill,
        PowerUp::DoublePoints,
        PowerUp::Nuke,
        PowerUp::BonusPoints,
        PowerUp::Carpenter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PowerUp::MaxAmmo => "MAX AMMO",
            PowerUp::InstaKill => "INSTA-KILL",
            PowerUp::DoublePoints => "DOUBLE POINTS",
            PowerUp::Nuke => "NUKE",
            PowerUp::BonusPoints => "BONUS POINTS",
            PowerUp::Carpenter => "CARPENTER",
        }
    }

    /// The chance (0–1) a zombie kill drops this one — about 1 in 16 kills
    /// drops something, the common ones more often.
    pub fn drop_chance(self) -> f32 {
        match self {
            PowerUp::BonusPoints => 0.016,
            PowerUp::MaxAmmo => 0.014,
            PowerUp::DoublePoints => 0.012,
            PowerUp::InstaKill => 0.010,
            PowerUp::Carpenter => 0.009,
            PowerUp::Nuke => 0.008,
        }
    }

    /// How long (s) from the grab its announcer line
    /// (`audio/zombies/power_ups/<name>.mp3`) is done talking, and a breath
    /// after — an operator's quote about it waits this long, so the two
    /// don't talk over each other. (A Nuke's line starts late,
    /// [`NUKE_ANNOUNCER_DELAY_SECS`].)
    pub fn announcer_secs(self) -> f32 {
        const BREATH: f32 = 0.4;
        BREATH
            + match self {
                PowerUp::MaxAmmo => 2.64,
                PowerUp::InstaKill => 2.69,
                PowerUp::DoublePoints => 2.24,
                PowerUp::Nuke => NUKE_ANNOUNCER_DELAY_SECS + 1.87,
                PowerUp::BonusPoints => 3.19,
                PowerUp::Carpenter => 1.68,
            }
    }

    /// Runs for [`TIMED_SECS`] rather than happening at once.
    pub fn timed(self) -> bool {
        matches!(self, PowerUp::InstaKill | PowerUp::DoublePoints)
    }
}

/// What one zombie kill drops, from a uniform `r` in `[0, 1)`: at most one
/// power-up, each with its own [`PowerUp::drop_chance`]. `always` (the
/// debug "100% drops" toggle) makes every kill drop one, still weighted by
/// those chances. `boards_torn` is how many boards are down across the
/// map's windows: under [`CARPENTER_MIN_TORN`] a Carpenter can't drop (its
/// share drops nothing).
pub fn roll(r: f32, always: bool, boards_torn: u32) -> Option<PowerUp> {
    let allowed = |p: PowerUp| p != PowerUp::Carpenter || boards_torn >= CARPENTER_MIN_TORN;
    let total: f32 = PowerUp::ALL.iter().filter(|p| allowed(**p)).map(|p| p.drop_chance()).sum();
    let scale = if always { 1.0 / total } else { 1.0 };
    let mut acc = 0.0;
    let mut last = None;
    for p in PowerUp::ALL {
        if !allowed(p) {
            continue;
        }
        acc += p.drop_chance() * scale;
        if r < acc {
            return Some(p);
        }
        last = Some(p);
    }
    // (Floating-point slack at the very top of the range.)
    if always {
        last
    } else {
        None
    }
}

/// Whether feet at `feet` are close enough to grab a drop lying at `drop`
/// (its spot on the ground).
pub fn in_pickup_range(feet: Vec3, drop: Vec3) -> bool {
    let d = feet - drop;
    Vec3::new(d.x, 0.0, d.z).length() <= PICKUP_RADIUS && d.y.abs() <= PICKUP_HEIGHT
}

/// Where a power-up a zombie dropped at `feet` lands: right there — unless
/// one of the living players (feet at `players`) could already grab it
/// there, when it's set down just out of reach of the nearest of them,
/// straight on past the zombie from them if there's room, or swung round
/// them to the first open floor that is (clear of the walls, on ground about
/// level with the zombie's, out of everyone's reach). Nowhere like that:
/// right there after all.
pub fn drop_spot(feet: Vec3, players: &[Vec3], world: &dyn CollisionWorld) -> Vec3 {
    let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
    let Some(&nearest) = players
        .iter()
        .filter(|&&p| in_pickup_range(p, feet))
        .min_by(|a, b| flat(feet - **a).length().total_cmp(&flat(feet - **b).length()))
    else {
        return feet;
    };
    let away = flat(feet - nearest).try_normalize().unwrap_or(Vec3::X);
    let chest = feet + Vec3::Y;
    // Straight on, then further round each side in 30° steps.
    let turns = std::iter::once(0).chain((1..=6).flat_map(|k| [k, -k]));
    turns
        .filter_map(|k: i32| {
            let dir = Quat::from_rotation_y(k as f32 * 30f32.to_radians()) * away;
            let above = Vec3::new(nearest.x, feet.y, nearest.z) + dir * (PICKUP_RADIUS + DROP_CLEARANCE) + Vec3::Y * DROP_STEP;
            if world.segment_blocked(chest, above) {
                return None;
            }
            let ground = world.raycast(above, Vec3::NEG_Y, DROP_STEP * 2.0)?;
            let spot = above - Vec3::Y * ground.distance;
            (!players.iter().any(|&p| in_pickup_range(p, spot))).then_some(spot)
        })
        .next()
        .unwrap_or(feet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{RayHit, WorldHit};

    /// A floor at `y = 0` (with a drop-off past `x <= ledge`, if one), and
    /// optionally a wall filling `x >= wall`.
    struct World {
        wall: Option<f32>,
        ledge: Option<f32>,
    }

    impl CollisionWorld for World {
        fn segment_blocked(&self, a: Vec3, b: Vec3) -> bool {
            self.wall.is_some_and(|w| a.x.max(b.x) >= w)
        }

        fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
            if self.wall.is_some_and(|w| origin.x >= w) || self.ledge.is_some_and(|l| origin.x <= l) {
                return None;
            }
            (dir.y < 0.0 && origin.y >= 0.0)
                .then(|| origin.y / -dir.y)
                .filter(|d| *d <= max_dist)
                .map(|distance| RayHit { distance, normal: Vec3::Y })
        }

        fn sweep_sphere(&self, _from: Vec3, _to: Vec3, _radius: f32) -> Option<WorldHit> {
            None
        }
    }

    const OPEN: World = World { wall: None, ledge: None };

    #[test]
    fn a_drop_out_of_everyones_reach_stays_where_the_zombie_fell() {
        let feet = Vec3::new(5.0, 0.0, 0.0);
        assert_eq!(drop_spot(feet, &[Vec3::ZERO], &OPEN), feet);
        assert_eq!(drop_spot(feet, &[], &OPEN), feet);
    }

    #[test]
    fn a_knifed_zombies_drop_lands_just_out_of_reach_beyond_it() {
        let me = Vec3::ZERO;
        let feet = Vec3::new(1.0, 0.0, 0.0);
        let spot = drop_spot(feet, &[me], &OPEN);
        assert!(!in_pickup_range(me, spot), "{spot}");
        assert!((spot - Vec3::X * (PICKUP_RADIUS + DROP_CLEARANCE)).length() < 1e-4, "{spot}");
    }

    #[test]
    fn a_wall_behind_the_zombie_swings_the_drop_round_and_never_into_reach() {
        let me = Vec3::ZERO;
        let feet = Vec3::new(1.0, 0.0, 0.0);
        let world = World { wall: Some(1.5), ledge: None };
        let spot = drop_spot(feet, &[me], &world);
        assert!(spot.x < 1.5 && !in_pickup_range(me, spot), "{spot}");
        assert!(spot.y.abs() < 1e-4);
        // Nor off a drop-off.
        let world = World { wall: Some(1.5), ledge: Some(-0.5) };
        let spot = drop_spot(feet, &[me], &world);
        assert!(spot.x > -0.5 && spot.x < 1.5 && !in_pickup_range(me, spot), "{spot}");
    }

    #[test]
    fn a_drop_stays_out_of_every_players_reach() {
        let players = [Vec3::ZERO, Vec3::new(2.4, 0.0, 0.0)];
        let spot = drop_spot(Vec3::new(1.0, 0.0, 0.0), &players, &OPEN);
        assert!(players.iter().all(|&p| !in_pickup_range(p, spot)), "{spot}");
    }

    #[test]
    fn boxed_in_it_stays_where_the_zombie_fell() {
        let feet = Vec3::new(1.0, 0.0, 0.0);
        let world = World { wall: Some(-10.0), ledge: None };
        assert_eq!(drop_spot(feet, &[Vec3::ZERO], &world), feet);
    }

    #[test]
    fn chances_differ_and_stay_small() {
        let mut seen = Vec::new();
        for p in PowerUp::ALL {
            let c = p.drop_chance();
            assert!(c > 0.0 && c < 0.05);
            assert!(!seen.contains(&c.to_bits()), "{p:?} shares a chance");
            seen.push(c.to_bits());
        }
    }

    #[test]
    fn a_roll_drops_at_most_one_at_about_the_set_rate() {
        let n = 100_000;
        let mut counts = [0u32; PowerUp::ALL.len()];
        let mut none = 0;
        for i in 0..n {
            let r = (i as f32 + 0.5) / n as f32;
            match roll(r, false, CARPENTER_MIN_TORN) {
                Some(p) => counts[PowerUp::ALL.iter().position(|q| *q == p).unwrap()] += 1,
                None => none += 1,
            }
        }
        for (k, p) in PowerUp::ALL.iter().enumerate() {
            let rate = counts[k] as f32 / n as f32;
            assert!((rate - p.drop_chance()).abs() < 0.001, "{p:?}: {rate}");
        }
        assert!(none > n * 9 / 10);
    }

    #[test]
    fn the_test_toggle_always_drops_every_kind() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..1000 {
            let r = i as f32 / 1000.0;
            seen.insert(roll(r, true, CARPENTER_MIN_TORN).expect("no drop with the test toggle on"));
        }
        assert_eq!(seen.len(), PowerUp::ALL.len());
        assert!(roll(0.99999, true, CARPENTER_MIN_TORN).is_some());
    }

    #[test]
    fn no_carpenter_while_the_windows_are_mostly_boarded() {
        for i in 0..10_000 {
            let r = i as f32 / 10_000.0;
            assert_ne!(roll(r, false, CARPENTER_MIN_TORN - 1), Some(PowerUp::Carpenter));
            assert_ne!(roll(r, true, 0), Some(PowerUp::Carpenter));
        }
        assert!(roll(0.99999, true, 0).is_some());
    }

    #[test]
    fn pickup_range() {
        let drop = Vec3::new(10.0, 0.0, 5.0);
        assert!(in_pickup_range(drop + Vec3::X, drop));
        assert!(!in_pickup_range(drop + Vec3::X * 2.0, drop));
        assert!(!in_pickup_range(drop + Vec3::Y * 3.0, drop));
    }
}
