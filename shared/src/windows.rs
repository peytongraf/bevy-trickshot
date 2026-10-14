//! `Zombies` windows: openings cut in a map's walls that zombies tear the
//! boards off and climb in through — never players (Call of Duty zombies'
//! barriers). Each map's are in its layout
//! ([`crate::level::ZombiesLayout::windows`], placed in the level editor over
//! the openings cut in its model); how many boards each has lost in a game is
//! [`crate::Lobby::windows_torn`], owned by the server.
//!
//! A window is one way in: a zombie comes up on the ground outside one
//! ([`Window::outside_spot`]), walks to the wall ([`Window::approach_spot`]),
//! rips the boards off one by one ([`tear_secs`] apart) and, once there are
//! none, climbs over the sill to [`Window::landing`] — and from there it's
//! like any other zombie (`server::ai`). Players rebuild them from inside
//! ([`Window::in_repair_range`]), a board every [`REPAIR_SECS`] for
//! [`REPAIR_POINTS`] each (up to [`REPAIR_POINTS_PER_ROUND`] a round).
//!
//! To everything but a player's body the opening is open — shots, grenades
//! and thrown knives go through, boards or no boards. A player's body is
//! kept out of it by the client, with plain maths ([`Window::body_push`]),
//! the way it keeps out of other players' bodies.

use bevy::math::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::level::Placement;
use crate::MapId;

/// Boards on a whole window.
pub const BOARDS: u8 = 6;

/// Most windows a map keeps.
pub const MAX_WINDOWS: usize = 32;

/// A new window's opening (m, `width × height`), sill height and wall
/// thickness — the sizes the maps' openings are cut to.
pub const DEFAULT_SIZE: Vec2 = Vec2::new(1.2, 1.4);
pub const DEFAULT_SILL: f32 = 0.9;
pub const DEFAULT_DEPTH: f32 = 0.5;

/// Seconds a player takes to put a board back.
pub const REPAIR_SECS: f32 = 1.0;
/// Points for each board put back...
pub const REPAIR_POINTS: u32 = 10;
/// ...up to this many a round, each player.
pub const REPAIR_POINTS_PER_ROUND: u32 = 500;
/// How far (m) in from the window's inside face a player can rebuild it.
pub const REPAIR_REACH: f32 = 1.6;

/// Where outside a zombie comes up: this far (m) out from the wall.
pub const OUTSIDE_SPAWN_DIST: f32 = 2.5;
/// Where it stands to tear the boards off: this far out from the wall.
pub const APPROACH_DIST: f32 = 0.55;
/// Where it lands inside, climbing in: this far in from the wall.
pub const LANDING_DIST: f32 = 0.8;
/// Seconds the climb over the sill takes.
pub const CLIMB_SECS: f32 = 1.4;

/// Seconds between a zombie's tears at a window's boards in round `round`
/// — quicker every round, to a limit.
pub fn tear_secs(round: u32) -> f32 {
    (1.8 - 0.06 * round.saturating_sub(1) as f32).max(0.7)
}

/// A board's height (m, up the opening — narrow, so the window's still
/// seen through between them) and thickness (out of the wall) — the board
/// model's own.
pub const BOARD_HEIGHT: f32 = 0.1;
pub const BOARD_THICKNESS: f32 = 0.03;

/// Each board's tilt (degrees, about the axis out of the window) and its
/// nudge sideways (m), bottom board first — a little crooked, nailed up in a
/// hurry.
const BOARD_TILTS: [f32; BOARDS as usize] = [6.0, -4.5, 8.0, -7.0, 3.5, -5.5];
const BOARD_NUDGES: [f32; BOARDS as usize] = [0.03, -0.04, 0.02, -0.02, 0.04, -0.03];

/// One window: the opening in the wall above the floor at `at` (under its
/// middle, halfway through the wall), its inside — where the players are —
/// the way it's turned (its +Z).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub at: Placement,
    /// The opening's `width × height` (m).
    pub size: Vec2,
    /// How high (m) above the floor the opening starts.
    pub sill: f32,
    /// How thick (m) the wall is.
    pub depth: f32,
}

impl Window {
    /// A fresh one at `at`, the default size.
    pub fn new(at: Placement) -> Self {
        Self {
            at: at.with_scale(1.0),
            size: DEFAULT_SIZE,
            sill: DEFAULT_SILL,
            depth: DEFAULT_DEPTH,
        }
    }

    pub fn rotation(&self) -> Quat {
        self.at.rotation()
    }

    /// `p` in the window's own frame (x across it, y up from its floor, z in
    /// from the middle of the wall).
    pub fn local(&self, p: Vec3) -> Vec3 {
        self.rotation().inverse() * (p - self.at.pos)
    }

    fn world(&self, local: Vec3) -> Vec3 {
        self.at.pos + self.rotation() * local
    }

    /// The opening: `(centre, rotation, half extents)`.
    pub fn opening(&self) -> (Vec3, Quat, Vec3) {
        let half = Vec3::new(self.size.x * 0.5, self.size.y * 0.5, self.depth * 0.5);
        (self.world(Vec3::Y * (self.sill + half.y)), self.rotation(), half)
    }

    /// Where outside a zombie comes up (at the inside floor's height — the
    /// server finds the real ground under it).
    pub fn outside_spot(&self) -> Vec3 {
        self.world(Vec3::NEG_Z * (self.depth * 0.5 + OUTSIDE_SPAWN_DIST))
    }

    /// Where a zombie stands to tear the boards off.
    pub fn approach_spot(&self) -> Vec3 {
        self.world(Vec3::NEG_Z * (self.depth * 0.5 + APPROACH_DIST))
    }

    /// Where a zombie lands inside, on the floor.
    pub fn landing(&self) -> Vec3 {
        self.world(Vec3::Z * (self.depth * 0.5 + LANDING_DIST))
    }

    /// The top of the sill, at the outside / inside face of the wall — the
    /// climb's way over.
    pub fn sill_outside(&self) -> Vec3 {
        self.world(Vec3::new(0.0, self.sill, -self.depth * 0.5))
    }

    pub fn sill_inside(&self) -> Vec3 {
        self.world(Vec3::new(0.0, self.sill, self.depth * 0.5))
    }

    /// Whether feet position `feet` is inside and close enough to rebuild
    /// it. `slack` widens it (the server allows for the client's pose being
    /// a moment old).
    pub fn in_repair_range(&self, feet: Vec3, slack: f32) -> bool {
        let l = self.local(feet);
        l.z >= self.depth * 0.5 - 0.2 - slack
            && l.z <= self.depth * 0.5 + REPAIR_REACH + slack
            && l.x.abs() <= self.size.x * 0.5 + 0.6 + slack
            && l.y >= -1.0 - slack
            && l.y <= self.sill + 0.6 + slack
    }

    /// The shove that gets a player's body — an upright cylinder of
    /// `radius` from `feet` up `height` — back out of the opening, if it's
    /// in it: straight out of the wall, back to the side it's on. (Below the
    /// sill and above the opening is the wall itself, which the map's own
    /// collision has.)
    pub fn body_push(&self, feet: Vec3, height: f32, radius: f32) -> Option<Vec3> {
        let l = self.local(feet);
        let face = self.depth * 0.5 + radius;
        let overlaps = l.y + height > self.sill
            && l.y < self.sill + self.size.y
            && l.x.abs() < self.size.x * 0.5 + radius
            && l.z.abs() < face;
        if !overlaps {
            return None;
        }
        let to = if l.z >= 0.0 { face } else { -face };
        Some(self.rotation() * Vec3::Z * (to - l.z))
    }

    /// Whether a mantle landing at `feet` would put a player in (or through)
    /// the opening — climbing onto the sill isn't allowed.
    pub fn blocks_mantle(&self, feet: Vec3) -> bool {
        let l = self.local(feet);
        l.x.abs() <= self.size.x * 0.5 + 0.4
            && l.z.abs() <= self.depth * 0.5 + 1.0
            && l.y >= self.sill - 0.6
            && l.y <= self.sill + self.size.y
    }

    /// A board's size: across the opening and a little past its sides,
    /// [`BOARD_HEIGHT`] high, [`BOARD_THICKNESS`] thick.
    pub fn board_size(&self) -> Vec3 {
        Vec3::new(self.size.x + 0.25, BOARD_HEIGHT, BOARD_THICKNESS)
    }

    /// Board `slot` (`0` the bottom one) where it's nailed up, in the
    /// window's frame: its middle, against the outside face — the zombies'
    /// side — and its tilt.
    pub fn board_local(&self, slot: u8) -> (Vec3, Quat) {
        let i = (slot as usize).min(BOARDS as usize - 1);
        let y = self.sill + self.size.y * (i as f32 + 0.5) / BOARDS as f32;
        let center = Vec3::new(BOARD_NUDGES[i], y, -(self.depth * 0.5 + BOARD_THICKNESS * 0.5 + 0.01));
        (center, Quat::from_rotation_z(BOARD_TILTS[i].to_radians()))
    }
}

/// `map`'s windows, from its layout.
pub fn windows(map: MapId) -> &'static [Window] {
    &crate::level::layout(map).windows
}

/// Boards still up on window `index`, by [`crate::Lobby::windows_torn`].
pub fn boards_left(torn: &[u8], index: usize) -> u8 {
    BOARDS - torn.get(index).copied().unwrap_or(0).min(BOARDS)
}

/// The window on `map` that feet position `feet` can rebuild, if any — one
/// missing boards, the nearest when two are close.
pub fn repairable_at(map: MapId, torn: &[u8], feet: Vec3, slack: f32) -> Option<usize> {
    windows(map)
        .iter()
        .enumerate()
        .filter(|&(i, w)| boards_left(torn, i) < BOARDS && w.in_repair_range(feet, slack))
        .min_by(|a, b| a.1.local(feet).length_squared().total_cmp(&b.1.local(feet).length_squared()))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Window {
        // Turned 90°: its inside is +x.
        Window::new(Placement::new(Vec3::new(10.0, 0.0, 5.0), 90.0))
    }

    #[test]
    fn a_body_in_the_opening_is_pushed_back_to_its_own_side() {
        let w = window();
        // Standing on the sill, a little in from the middle.
        let push = w.body_push(Vec3::new(10.1, 0.9, 5.0), 1.8, 0.25).unwrap();
        assert!(push.x > 0.0, "back inside");
        let out = w.body_push(Vec3::new(9.9, 0.9, 5.0), 1.8, 0.25).unwrap();
        assert!(out.x < 0.0, "back outside");
        // Leaning on the wall below the sill, or clear of it, isn't touched.
        assert!(w.body_push(Vec3::new(10.6, 0.0, 5.0), 0.8, 0.25).is_none());
        assert!(w.body_push(Vec3::new(11.0, 0.0, 5.0), 1.8, 0.25).is_none());
        assert!(w.body_push(Vec3::new(10.0, 0.9, 7.0), 1.8, 0.25).is_none(), "past its side");
    }

    #[test]
    fn rebuilt_from_inside_only() {
        let w = window();
        assert!(w.in_repair_range(Vec3::new(11.0, 0.0, 5.0), 0.0));
        assert!(!w.in_repair_range(Vec3::new(8.5, 0.0, 5.0), 0.0), "outside");
        assert!(!w.in_repair_range(Vec3::new(13.5, 0.0, 5.0), 0.0), "too far in");
    }

    #[test]
    fn the_way_in_runs_outside_to_inside() {
        let w = window();
        assert!(w.outside_spot().x < w.approach_spot().x);
        assert!(w.approach_spot().x < w.sill_outside().x);
        assert!(w.sill_inside().x < w.landing().x);
        assert_eq!(w.landing().y, 0.0);
    }

    #[test]
    fn boards_count_down_from_full() {
        assert_eq!(boards_left(&[], 3), BOARDS);
        assert_eq!(boards_left(&[0, 2], 1), BOARDS - 2);
        assert_eq!(boards_left(&[9], 0), 0);
        assert!(tear_secs(1) > tear_secs(20));
        assert!(tear_secs(200) >= 0.7);
    }

    #[test]
    fn boards_stack_up_the_opening() {
        let w = window();
        let (bottom, _) = w.board_local(0);
        let (top, _) = w.board_local(BOARDS - 1);
        assert!(bottom.y > w.sill && top.y < w.sill + w.size.y && bottom.y < top.y);
        assert!(bottom.z < -w.depth * 0.5, "on the outside face, the zombies' side");
    }
}
