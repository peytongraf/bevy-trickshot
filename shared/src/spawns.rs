//! Player spawn-point placement for [`crate::GameMode::FreeForAll`] (initial
//! spawn and every respawn) — kept separate from [`crate::bots`] since it's
//! spreading *players* out rather than placing static targets.

use bevy::math::Vec3;

use crate::bots::rand01;
use crate::map;
use crate::protocol::{GameMode, MapId};

/// Spawns land on a ring this far from the map centre — far enough apart
/// that two players rarely start right on top of each other, comfortably
/// inside the sniper's `max_range` (300 m, see [`crate::weapon`]) and the
/// ground's playable extent ([`crate::ballistics::GROUND_HALF_EXTENT`], 100 m).
const RING_MIN_RADIUS: f32 = 15.0;
const RING_MAX_RADIUS: f32 = 70.0;

/// A candidate closer than this to any currently-alive player is rerolled —
/// best-effort spawn protection, not a hard guarantee.
const MIN_SEPARATION: f32 = 12.0;

/// How many times to reroll before giving up and returning the least-bad try.
const RETRIES: u32 = 8;

/// Clearance kept from a map's walls (see [`map::point_blocked`]) when
/// picking a spawn — generous relative to a player's actual body radius so a
/// spawn never lands flush against a container, only never inside one.
const WALL_CLEARANCE: f32 = 1.0;

/// One hand-placed spawn point: where on the ground (`x`, `y`, `z` — the
/// feet; the eye is `EYE_HEIGHT` above, the `y = 1.7` in
/// `client/notes/shipment-spawn-points.md`) and which way to face. `yaw_deg` follows the game's convention: `0` faces
/// -Z, positive turns **left**, negative turns **right** (the notes' "130 left"
/// is `+130`, "140 right" is `-140`). Pitch is always `0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw_deg: f32,
}

const fn sp(x: f32, z: f32, yaw_deg: f32) -> SpawnPoint {
    SpawnPoint { x, y: 0.0, z, yaw_deg }
}

/// A spawn point up on (or down on) a platform `y` metres off the ground.
const fn sp_at(x: f32, y: f32, z: f32, yaw_deg: f32) -> SpawnPoint {
    SpawnPoint { x, y, z, yaw_deg }
}

/// `AshesOfTheDamned`'s spawn points, spread over every platform, each
/// facing the middle of the map.
const ASHES_OF_THE_DAMNED_SPAWNS: [SpawnPoint; 12] = [
    // Ground-level platform.
    sp(-15.0, -15.0, -135.0),
    sp(15.0, -15.0, 135.0),
    sp(0.0, -8.0, 180.0),
    sp(-18.0, -4.0, -102.5),
    sp(18.0, -4.0, 102.5),
    // The raised deck behind it (up the ramp).
    sp_at(-15.0, 5.0, 30.0, -26.6),
    sp_at(15.0, 5.0, 30.0, 26.6),
    sp_at(0.0, 5.0, 24.0, 0.0),
    // The high platforms either end, and the sunken one.
    sp_at(-50.0, 10.0, -25.0, -116.6),
    sp_at(-50.0, 10.0, 25.0, -63.4),
    sp_at(0.0, 10.0, -70.0, 180.0),
    sp_at(45.0, -10.0, 0.0, 90.0),
];

/// `VesperStation`'s spawn points, a couple in each zone (upstairs in the
/// barracks, up on the plateau for the power and the lab).
const VESPER_STATION_SPAWNS: [SpawnPoint; 13] = [
    // The spawn square.
    sp(0.0, -12.0, 180.0),
    sp(-10.0, 9.0, 90.0),
    sp(10.0, -15.0, 45.0),
    // The barracks: downstairs, upstairs, and its yard.
    sp(-33.0, 8.0, -90.0),
    sp_at(-33.0, 4.0, 8.0, -90.0),
    sp(-21.0, -15.0, 0.0),
    // The power: its yard and the generator hall.
    sp_at(-30.0, 3.0, -25.0, 180.0),
    sp_at(-27.0, 3.0, -56.0, 90.0),
    // The lab: the front room and the core.
    sp_at(0.0, 3.0, -26.0, 0.0),
    sp_at(-8.0, 3.0, -52.0, -90.0),
    // The rail yard.
    sp(28.0, -20.0, 90.0),
    sp(20.0, 10.0, 0.0),
    sp(32.0, -44.0, 180.0),
];

/// `DieMaschine`'s spawn points, over every area: the Yard, the Nacht
/// building's floors and roof, the Crash Site, the Pond and the labs.
const DIE_MASCHINE_SPAWNS: [SpawnPoint; 16] = [
    sp(0.0, -8.0, 180.0),
    sp(-8.0, 4.0, -90.0),
    sp(-25.0, -6.0, -90.0),
    sp(-30.0, 5.0, -90.0),
    sp_at(-30.0, 3.6, -5.0, 90.0),
    sp_at(-22.0, 3.6, 4.0, 90.0),
    sp_at(-28.0, 7.2, -3.0, 0.0),
    sp(-44.0, -46.0, 90.0),
    sp(4.0, -52.0, 0.0),
    sp(-60.0, -10.0, 180.0),
    sp(-70.0, 20.0, 90.0),
    sp_at(-56.0, -6.0, -59.0, -90.0),
    sp_at(-32.0, -6.0, -55.0, 0.0),
    sp_at(-32.0, -10.0, -20.0, 180.0),
    sp_at(2.0, -10.0, -58.0, 90.0),
    sp_at(0.0, -10.0, -20.0, 0.0),
];

/// `Shipment` / `ShipmentDay`'s spawn points, from
/// `client/notes/shipment-spawn-points.md`.
const SHIPMENT_SPAWNS: [SpawnPoint; 11] = [
    sp(27.0, -27.0, 130.0),
    sp(6.7, -27.5, -140.0),
    sp(0.0, -26.8, 92.0),
    sp(-26.0, -27.0, -130.0),
    sp(-24.5, -0.85, 170.0),
    sp(-26.0, 28.5, -48.0),
    sp(-6.5, 28.5, 30.0),
    sp(-2.5, 23.5, -90.0),
    sp(26.5, 28.0, 36.0),
    sp(7.0, 28.5, -37.0),
    sp(25.0, -1.5, 12.0),
];

/// The hand-placed spawn points for `map`, if it has any. Everyone who spawns
/// there — at the start of a match or on every respawn, players and bots
/// alike — uses one of these (see [`spawn_point`]); maps without them fall back
/// to a random spot on a ring around the map's centre.
/// Whether a game of `mode` on `map` puts each player on a start spot the
/// server picks (and sends them, `crate::PlayerRespawn` with `immediate`):
/// always on a map with hand-placed spawns, and in every mode but
/// `Freestyle` — which otherwise leaves a player where their client already
/// is.
pub fn has_start_spawn(mode: GameMode, map: MapId) -> bool {
    designated_spawns(map).is_some() || mode != GameMode::Freestyle
}

/// A `Zombies` player's start spot on `map`, from the spots its layout has
/// (`crate::level::ZombiesLayout::player_spawns`, placed in the level
/// editor): a random one nobody in `taken` is on yet (any of them, once
/// they're all taken), facing the way it's turned. `None` if it has none.
pub fn zombies_start(map: MapId, seed: u64, taken: &[Vec3]) -> Option<(Vec3, f32)> {
    let spots = &crate::level::layout(map).player_spawns;
    if spots.is_empty() {
        return None;
    }
    let free: Vec<_> = spots
        .iter()
        .filter(|s| taken.iter().all(|t| t.distance(s.pos) > 0.5))
        .collect();
    let pick = if free.is_empty() {
        &spots[(rand01(seed) * spots.len() as f32) as usize % spots.len()]
    } else {
        free[(rand01(seed) * free.len() as f32) as usize % free.len()]
    };
    // (A placement's turn faces +Z at 0; a player's yaw faces -Z.)
    Some((pick.pos, (pick.yaw_deg + 180.0).to_radians()))
}

pub fn designated_spawns(map: MapId) -> Option<&'static [SpawnPoint]> {
    match map {
        MapId::Shipment | MapId::ShipmentDay => Some(&SHIPMENT_SPAWNS),
        MapId::AshesOfTheDamned => Some(&ASHES_OF_THE_DAMNED_SPAWNS),
        MapId::VesperStation => Some(&VESPER_STATION_SPAWNS),
        MapId::DieMaschine => Some(&DIE_MASCHINE_SPAWNS),
        _ => None,
    }
}

/// Pick a ground spawn point (position + facing) for `GameMode::FreeForAll`,
/// preferring one at least [`MIN_SEPARATION`] from every position in
/// `others` (typically every other currently-alive player in the lobby),
/// never inside one of `map`'s walls, and never outside `map`'s playable
/// interior (see [`map::in_bounds`]) — `Shipment`'s ring radius reaches well
/// past its walls, so a candidate can clear all of them while still landing
/// beyond the map entirely.
pub fn spawn_point(seed: u64, others: &[Vec3], map: MapId) -> (Vec3, f32) {
    // A map with hand-placed spawns: a random one, facing the way it says —
    // preferring one clear of everyone (in the ground plane), else the one
    // that's farthest from the nearest person.
    if let Some(points) = designated_spawns(map) {
        let start = (rand01(seed) * points.len() as f32) as usize % points.len();
        let mut best: Option<(SpawnPoint, f32)> = None;
        for k in 0..points.len() {
            let p = points[(start + k) % points.len()];
            let min_dist = others
                .iter()
                .map(|o| Vec3::new(o.x - p.x, 0.0, o.z - p.z).length())
                .fold(f32::INFINITY, f32::min);
            if min_dist >= MIN_SEPARATION {
                best = Some((p, min_dist));
                break;
            }
            if best.is_none_or(|(_, d)| min_dist > d) {
                best = Some((p, min_dist));
            }
        }
        let p = best.map(|(p, _)| p).unwrap_or(points[start]);
        return (Vec3::new(p.x, p.y, p.z), p.yaw_deg.to_radians());
    }
    let mut best: Option<(Vec3, f32, f32)> = None; // (pos, yaw, min_dist)
    for i in 0..RETRIES {
        let s = seed ^ (i as u64).wrapping_mul(0x2545_f491_4f6c_dd1d);
        let r = (RING_MIN_RADIUS + rand01(s) * (RING_MAX_RADIUS - RING_MIN_RADIUS))
            * map::area_scale(map);
        let a = rand01(s ^ 0xa1) * core::f32::consts::TAU;
        let pos = Vec3::new(r * a.cos(), 0.0, r * a.sin());
        let yaw = rand01(s ^ 0xb2) * core::f32::consts::TAU;
        let scale = map::placement(map).scale;
        if map::point_blocked(map, pos.x, pos.z, WALL_CLEARANCE, scale)
            || !map::in_bounds(map, pos.x, pos.z, scale)
        {
            continue;
        }
        let min_dist = others
            .iter()
            .map(|o| (*o - pos).length())
            .fold(f32::INFINITY, f32::min);
        if min_dist >= MIN_SEPARATION {
            return (pos, yaw);
        }
        if best.map(|(_, _, d)| min_dist > d).unwrap_or(true) {
            best = Some((pos, yaw, min_dist));
        }
    }
    best.map(|(p, y, _)| (p, y))
        .unwrap_or((Vec3::ZERO, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_point_lands_within_the_ring() {
        for seed in 0..2000u64 {
            let (pos, _) = spawn_point(seed, &[], MapId::BasicMap);
            let r = pos.length();
            assert!(
                (RING_MIN_RADIUS - 1.0e-3..=RING_MAX_RADIUS + 1.0e-3).contains(&r),
                "seed {seed} landed at radius {r}"
            );
        }
    }

    #[test]
    fn spawn_point_avoids_others_when_possible() {
        let others = [Vec3::new(20.0, 0.0, 0.0)];
        for seed in 0..500u64 {
            let (pos, _) = spawn_point(seed, &others, MapId::BasicMap);
            let d = (pos - others[0]).length();
            assert!(d >= MIN_SEPARATION - 1.0e-3, "seed {seed} landed {d}m away");
        }
    }

    #[test]
    fn shipment_spawns_are_the_hand_placed_points_facing_as_set() {
        let points = designated_spawns(MapId::Shipment).unwrap();
        assert_eq!(points.len(), 11);
        assert_eq!(designated_spawns(MapId::ShipmentDay), Some(points));
        assert!(designated_spawns(MapId::BasicMap).is_none());
        for seed in 0..2000u64 {
            let (pos, yaw) = spawn_point(seed, &[], MapId::Shipment);
            let p = points
                .iter()
                .find(|p| p.x == pos.x && p.z == pos.z)
                .unwrap_or_else(|| panic!("seed {seed} spawned off the list at {pos:?}"));
            assert_eq!(pos.y, 0.0, "spawns are on the ground (the eye is 1.7 above)");
            assert!((yaw - p.yaw_deg.to_radians()).abs() < 1e-6, "seed {seed}: wrong facing");
        }
    }

    #[test]
    fn every_shipment_spawn_gets_used() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0..2000u64 {
            let (pos, _) = spawn_point(seed, &[], MapId::Shipment);
            seen.insert((pos.x.to_bits(), pos.z.to_bits()));
        }
        assert_eq!(seen.len(), 11, "some spawn point never came up");
    }

    #[test]
    fn a_shipment_spawn_avoids_someone_standing_on_a_point_when_it_can() {
        let points = designated_spawns(MapId::Shipment).unwrap();
        let on_first = [Vec3::new(points[0].x, 0.0, points[0].z)];
        for seed in 0..500u64 {
            let (pos, _) = spawn_point(seed, &on_first, MapId::Shipment);
            let d = Vec3::new(pos.x - on_first[0].x, 0.0, pos.z - on_first[0].z).length();
            assert!(d >= MIN_SEPARATION - 1e-3, "seed {seed} spawned {d} m from someone");
        }
    }

    #[test]
    fn a_full_lobby_still_gets_a_spawn_when_everyone_else_is_at_every_point() {
        let points = designated_spawns(MapId::Shipment).unwrap();
        let everyone: Vec<Vec3> = points.iter().map(|p| Vec3::new(p.x, 0.0, p.z)).collect();
        let (pos, _) = spawn_point(1, &everyone, MapId::Shipment);
        assert!(points.iter().any(|p| p.x == pos.x && p.z == pos.z));
    }

    #[test]
    fn break_point_spawns_are_on_the_map_clear_of_walls_and_spread_out() {
        let scale = map::placement(MapId::BreakPoint).scale;
        let mut xs = (f32::MAX, f32::MIN);
        for seed in 0..3000u64 {
            let (pos, _) = spawn_point(seed, &[], MapId::BreakPoint);
            assert!(
                !map::point_blocked(MapId::BreakPoint, pos.x, pos.z, 0.0, scale),
                "seed {seed} landed inside a wall at {pos:?}"
            );
            assert!(map::in_bounds(MapId::BreakPoint, pos.x, pos.z, scale), "off the map at {pos:?}");
            xs = (xs.0.min(pos.x), xs.1.max(pos.x));
        }
        // Not all bunched at the fallback centre: the ring spans a fair part of the 48 m width.
        assert!(xs.1 - xs.0 > 20.0, "spawns only span x {xs:?}");
    }
}
