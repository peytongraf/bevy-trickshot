//! The Exhumer — Der Tempel's wonder weapon, taken from the altar in the
//! pyramid's chamber at the end of its easter egg (`crate::easter_egg`; the
//! story's in `client/notes/der-tempel-easter-egg/story.md`): the relic the
//! Hierophant's first rite was done with, the instrument that pulls a soul
//! out of a body. Two triggers, Call of Duty's way:
//!
//! * **Aim (held) — draw souls in.** A beam out to [`BEAM_RANGE`] (stopped
//!   by walls) takes hold of the zombies it touches (the nearest
//!   [`BEAM_MAX_TARGETS`]) and drains them where they stand — an ordinary
//!   one's drained in [`DRAIN_SECS`], whatever its round ([`ExhumerDrain`]).
//!   Drained, it's exhumed: lifted into the air over [`EXHUME_SECS`]
//!   ([`ExhumerExhume`]), its soul torn out at the top, back into the
//!   Exhumer ([`ExhumerSouls`]), and the empty body dropped limp. Big Steve, a boss or a hellhound can't be
//!   held up, and drains far slower ([`HEAVY_DPS`]). It never runs out —
//!   but it takes time.
//! * **Fire — let them out.** One pull sends every soul it holds out at once
//!   ([`ReleaseSouls`]): one after each of the zombies in view (within
//!   [`RELEASE_RANGE`], nothing in the way) closest to where they're aimed
//!   ([`release_targets`]), tearing its soul out as it gets there
//!   ([`ExhumerSwarm`]). With fewer zombies about than souls, only as many
//!   go as there are zombies, and the rest stay. What can't be lifted is hurt
//!   instead ([`HEAVY_DAMAGE`]). Souls let out give nothing back.
//!
//! Its souls are its only ammo — at most [`MAX_SOULS`], kept by the server
//! (each player's, each game) — and it never reloads, takes no ammo from a
//! crate or a Max Ammo, has no aim-down-sight, and can't be Pack-a-Punched.
//! The Mystery Box never has it.

use bevy::math::{EulerRot, Quat, Vec3};
use lightyear::prelude::PeerId;
use serde::{Deserialize, Serialize};

/// The most souls it holds.
pub const MAX_SOULS: u32 = 24;

/// The beam: how far (m) it reaches...
pub const BEAM_RANGE: f32 = 18.0;
/// ...how close (m) to its line a zombie's chest has to be for it to take
/// hold, right at the gun — a cone, so a little more (m) for every metre
/// out (2 m off its line at its full reach): it doesn't need dead-on aim...
pub const BEAM_RADIUS: f32 = 1.0;
pub const BEAM_SPREAD: f32 = 0.055;
/// ...and how many it holds at once, the nearest first.
pub const BEAM_MAX_TARGETS: usize = 3;
/// How long (s) it takes to drain an ordinary zombie dead (its damage a
/// share of the zombie's health, so the same in any round)...
pub const DRAIN_SECS: f32 = 1.4;
/// ...and how much it takes (per second) out of what it can't lift.
pub const HEAVY_DPS: f32 = 150.0;
/// How often (s) what it's drained is taken (and the clients told).
pub const BEAM_TICK_SECS: f32 = 0.25;
/// A zombie drained: how long (s) it takes to lift it, its soul torn out
/// at the top...
pub const EXHUME_SECS: f32 = 1.2;
/// ...and how high (m) it's lifted.
pub const LIFT_HEIGHT: f32 = 2.2;

/// Letting the souls out: after zombies this far (m) off at most, this far
/// (°) from where the player's looking at most (on their screen), at this
/// speed (m/s)...
pub const RELEASE_RANGE: f32 = 50.0;
pub const RELEASE_VIEW_DEG: f32 = 55.0;
pub const SWARM_SPEED: f32 = 16.0;
/// ...no sooner than this (s) after the last time.
pub const RELEASE_COOLDOWN_SECS: f32 = 1.0;
/// What a soul let out does to what it can't lift (a boss, Big Steve, a
/// hellhound).
pub const HEAVY_DAMAGE: f32 = 900.0;

/// Whether a zombie in `anim` can be lifted (and drained at its own pace):
/// an ordinary one (a crawler too) — not a hellhound, a boss (Big Steve,
/// the Hierophant) or one still climbing out of the ground.
pub fn liftable(anim: crate::ZombieAnim) -> bool {
    anim.is_zombie() && !anim.is_dog() && !anim.is_boss() && anim != crate::ZombieAnim::Rising
}

/// The way the beam goes from a player looking `yaw` / `pitch` (a
/// `PlayerPose`'s).
pub fn beam_dir(yaw: f32, pitch: f32) -> Vec3 {
    Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z
}

/// How far (m) off the beam's line it takes hold, `along` (m) out from the
/// gun.
pub fn beam_radius(along: f32) -> f32 {
    BEAM_RADIUS + BEAM_SPREAD * along.max(0.0)
}

/// Which of `chests` (each with what it's tagged by) the beam from `eye`
/// along `dir` (unit) takes hold of, reaching `reach` (m): the ones inside
/// its cone ([`beam_radius`]), nearest first, [`BEAM_MAX_TARGETS`] at most.
pub fn beam_targets<T: Copy>(eye: Vec3, dir: Vec3, reach: f32, chests: impl Iterator<Item = (T, Vec3)>) -> Vec<T> {
    let mut held: Vec<(f32, T)> = chests
        .filter_map(|(tag, at)| {
            let along = (at - eye).dot(dir);
            let off = (at - (eye + dir * along)).length();
            (along > 0.0 && along <= reach && off <= beam_radius(along)).then_some((along, tag))
        })
        .collect();
    held.sort_by(|a, b| a.0.total_cmp(&b.0));
    held.into_iter().take(BEAM_MAX_TARGETS).map(|(_, t)| t).collect()
}

/// Which of `chests` (each with what it's tagged by) the souls let out from
/// `eye`, looking along `dir` (unit), go after — `n` at most: only those in
/// view (within [`RELEASE_RANGE`] and [`RELEASE_VIEW_DEG`] of the look, and
/// `sees` them — nothing in the way), the ones closest to where they're
/// aimed first (the nearer first, about as close). With each, how far off
/// (m) it is.
pub fn release_targets<T: Copy>(
    eye: Vec3,
    dir: Vec3,
    n: usize,
    chests: impl Iterator<Item = (T, Vec3)>,
    sees: impl Fn(Vec3) -> bool,
) -> Vec<(T, Vec3, f32)> {
    let cos_view = RELEASE_VIEW_DEG.to_radians().cos();
    let mut seen: Vec<(f32, f32, T, Vec3)> = chests
        .filter_map(|(tag, at)| {
            let to = at - eye;
            let d = to.length();
            if d <= 1e-3 || d > RELEASE_RANGE {
                return None;
            }
            let cos = to.dot(dir) / d;
            (cos >= cos_view && sees(at)).then(|| {
                // (Off the aim, in degrees — a degree's worth of it a few
                // metres' worth of distance, so the nearer of two about as
                // close to the aim goes first.)
                let off = cos.clamp(-1.0, 1.0).acos().to_degrees();
                (off + d * 0.05, d, tag, at)
            })
        })
        .collect();
    seen.sort_by(|a, b| a.0.total_cmp(&b.0));
    seen.into_iter().take(n).map(|(_, d, t, at)| (t, at, d)).collect()
}

/// Server → every member of the lobby: `shooter`'s beam has hold of
/// `zombies` (bot peer id, and how far drained, 0..=1) — held where they
/// stand while they're drained.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExhumerDrain {
    pub shooter: PeerId,
    pub zombies: Vec<(PeerId, f32)>,
}

/// Server → every member of the lobby: `zombies` (bot peer ids), drained by
/// `shooter`'s beam, are being exhumed — lifted [`LIFT_HEIGHT`] over
/// [`EXHUME_SECS`], their souls torn out at the top ([`ExhumerSouls`]) and
/// the bodies dropped.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExhumerExhume {
    pub shooter: PeerId,
    pub zombies: Vec<PeerId>,
}

/// Server → every member of the lobby: souls just torn out at `from` (the
/// zombies' chests) went into `shooter`'s Exhumer, which now holds `souls`.
/// (Also sent with none, when a release has spent some.)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExhumerSouls {
    pub shooter: PeerId,
    pub from: Vec<[f32; 3]>,
    pub souls: u32,
}

/// Server → every member of the lobby: `shooter` let their souls out from
/// `origin` — one after each of `targets` (bot peer id, where it was),
/// tearing its soul out when it gets there.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExhumerSwarm {
    pub shooter: PeerId,
    pub origin: [f32; 3],
    pub targets: Vec<(PeerId, [f32; 3])>,
}

/// Client → server: let my Exhumer's souls out (the server checks I hold
/// it, and any).
#[derive(bevy::prelude::Event, Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ReleaseSouls;

/// Where the `i`th of `n` Exhumers lies on the pyramid's altar at `altar`
/// (its placement): side by side along its top (up to 4 — a full lobby),
/// each turned to point front to back across it, and the yaw it lies at
/// (a dropped weapon lies along its own +X).
pub fn on_altar(altar: crate::level::Placement, i: usize, n: usize) -> (Vec3, f32) {
    let half = crate::easter_egg::PYRAMID_ALTAR_HALF_EXTENTS * altar.scale;
    let n = n.max(1);
    let across = if n == 1 { 0.0 } else { (i as f32 / (n - 1) as f32 - 0.5) * half.x * 1.6 };
    let local = Vec3::new(across, half.y * 2.0 + 0.02, 0.0);
    (altar.pos + altar.rotation() * local, altar.yaw_deg.to_radians() + std::f32::consts::FRAC_PI_2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ZombieAnim;

    #[test]
    fn only_ordinary_zombies_are_lifted() {
        assert!(liftable(ZombieAnim::Walk) && liftable(ZombieAnim::Crawl));
        assert!(!liftable(ZombieAnim::Dog) && !liftable(ZombieAnim::BossWalk) && !liftable(ZombieAnim::Rising));
        assert!(!liftable(ZombieAnim::None));
    }

    #[test]
    fn four_exhumers_fit_side_by_side_on_the_altar() {
        let altar = crate::level::Placement { pos: Vec3::ZERO, yaw_deg: 0.0, scale: 1.0 };
        let half = crate::easter_egg::PYRAMID_ALTAR_HALF_EXTENTS;
        // (Each lies front to back: its 0.45 m along the altar's depth.)
        const LENGTH: f32 = 0.45;
        assert!(LENGTH / 2.0 <= half.z, "longer than the altar's deep");
        let at: Vec<Vec3> = (0..4).map(|i| on_altar(altar, i, 4).0).collect();
        for p in &at {
            assert!(p.x.abs() + 0.08 <= half.x, "off the end: {p}");
            assert!((p.y - half.y * 2.0).abs() < 0.05, "not on top: {p}");
        }
        for w in at.windows(2) {
            assert!(w[1].x - w[0].x >= 0.3, "too close together");
        }
    }

    #[test]
    fn the_beam_holds_the_nearest_few_on_its_line_within_reach() {
        let eye = Vec3::ZERO;
        let dir = beam_dir(0.0, 0.0);
        assert!((dir - Vec3::NEG_Z).length() < 1e-5);
        let chests = [
            (1, Vec3::new(0.0, 0.0, -5.0)),
            (2, Vec3::new(0.3, 0.0, -2.0)),
            (3, Vec3::new(2.0, 0.0, -3.0)),  // off the line
            (4, Vec3::new(0.0, 0.0, 3.0)),   // behind
            (5, Vec3::new(0.0, 0.0, -30.0)), // too far
            (6, Vec3::new(0.0, 0.5, -8.0)),
            (7, Vec3::new(0.0, 0.0, -10.0)),
        ];
        let held = beam_targets(eye, dir, BEAM_RANGE, chests.into_iter());
        assert_eq!(held, vec![2, 1, 6]);
        // A wall in the way: nothing past it.
        assert_eq!(beam_targets(eye, dir, 3.0, chests.into_iter()), vec![2]);
    }

    #[test]
    fn souls_go_after_what_is_in_view_closest_to_the_aim_first() {
        let eye = Vec3::ZERO;
        let dir = beam_dir(0.0, 0.0);
        let chests = [
            (1, Vec3::new(0.0, 0.0, 3.0)),    // behind — the nearest of all
            (2, Vec3::new(4.0, 0.0, -4.0)),   // 45° off
            (3, Vec3::new(0.5, 0.0, -20.0)),  // right on the aim, far
            (4, Vec3::new(0.0, 0.0, -6.0)),   // on the aim, behind a wall
            (5, Vec3::new(-1.0, 0.0, -10.0)), // a little off
            (6, Vec3::new(0.0, 0.0, -80.0)),  // out of range
        ];
        let behind_wall = |at: Vec3| (at - Vec3::new(0.0, 0.0, -6.0)).length() > 0.1;
        let got: Vec<i32> = release_targets(eye, dir, 5, chests.into_iter(), behind_wall).iter().map(|t| t.0).collect();
        assert_eq!(got, vec![3, 5, 2]);
        // Fewer souls than that: the closest to the aim only.
        let got: Vec<i32> = release_targets(eye, dir, 1, chests.into_iter(), behind_wall).iter().map(|t| t.0).collect();
        assert_eq!(got, vec![3]);
        // About as close to the aim: the nearer.
        let pair = [(1, Vec3::new(0.0, 0.0, -30.0)), (2, Vec3::new(0.0, 0.1, -5.0))];
        assert_eq!(release_targets(eye, dir, 1, pair.into_iter(), |_| true)[0].0, 2);
    }

    #[test]
    fn the_beam_widens_into_a_cone() {
        let eye = Vec3::ZERO;
        let dir = beam_dir(0.0, 0.0);
        // 1.5 m off its line: too far out near the gun, held further out.
        let near = [(1, Vec3::new(1.5, 0.0, -2.0))];
        let far = [(1, Vec3::new(1.5, 0.0, -15.0))];
        assert!(beam_targets(eye, dir, BEAM_RANGE, near.into_iter()).is_empty());
        assert_eq!(beam_targets(eye, dir, BEAM_RANGE, far.into_iter()), vec![1]);
        assert!(beam_radius(BEAM_RANGE) < 2.5, "too wide at its reach");
    }
}
