//! Bot pathfinding, computed automatically from each map's collision mesh —
//! there is no per-map navigation data to author. Drop a new (or edited)
//! `.glb` collision model in, and the graph is rebuilt from it at server start.
//!
//! **How it's built** ([`NavGraph::build`]): lay a 1 m grid over the mesh's
//! bounds and, in every cell, cast rays straight down to find each surface a
//! body could stand on — so a map with floors above floors (ramps up to slabs)
//! gets one node per level per cell. A surface is walkable if it isn't too steep
//! (`MAX_SLOPE_DEG`), has standing room above it, and a body-sized sphere fits
//! there. Neighbouring nodes (8 directions) are joined when the height change
//! is climbable, nothing solid is in the way at chest height, and the ground is
//! continuous between them (no gap or ledge). Connected regions are labelled;
//! tiny ones (the insides of solid blocks, slivers) are ignored, while every
//! real region — the yard, a walled-off interior, an upper floor reached by a
//! ramp — gets paths within it. Asking for a path between two regions with no
//! walkable connection between them returns `None`.
//!
//! **Queries** ([`NavGraph::find_path`]): A* over that graph, then the route is
//! straightened by dropping every waypoint a bot can walk past in a line.
//!
//! The rules here deliberately mirror `ai::move_bot` (same body radius, chest
//! height, step and slope limits), because a graph that says "you can walk
//! here" when the mover can't is how bots get stuck — a test walks a real bot
//! along a real path to keep the two honest.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

use bevy::math::Vec3;
use bevy::prelude::Resource;
use shared::map::CollisionWorld;
use shared::MapId;

use crate::collision::MapColliders;

/// Grid spacing (m).
const CELL: f32 = 1.0;
/// Body dimensions the graph is built for — `ai::move_bot` uses these very
/// constants, so the two can't disagree. The body is swept as a sphere of
/// `BODY_RADIUS` centred `CHEST_HEIGHT` above the feet; its underside
/// (`CHEST_HEIGHT - BODY_RADIUS` = 0.7 m) sits above the tallest step a body
/// climbs (`STEP_HEIGHT`), so steps never register as walls.
pub(crate) const BODY_RADIUS: f32 = 0.35;
const BODY_HEIGHT: f32 = 1.8;
pub(crate) const CHEST_HEIGHT: f32 = 1.05;
/// Steepest surface (degrees from flat) that counts as walkable. Ramps of ~40°
/// are fine; anything steeper is treated as a wall.
const MAX_SLOPE_DEG: f32 = 50.0;
/// The tallest step (m) a body climbs.
pub(crate) const STEP_HEIGHT: f32 = 0.6;
/// Extra height (m) a single step may add on top of what the slope allows.
const STEP_SLACK: f32 = STEP_HEIGHT;
/// A second surface within this distance below one already found isn't a
/// separate floor (a slab's underside, a thin ledge) — nobody fits between.
const MIN_LEVEL_GAP: f32 = BODY_HEIGHT + 0.1;
/// Most floors stacked over one cell.
const MAX_LEVELS: usize = 8;
/// A connected region with fewer nodes than this (~ m² of floor) isn't a place
/// anyone walks — it's inside a solid block or a sliver — and is ignored.
const MIN_REGION_NODES: u32 = 100;
/// Surfaces this far below the map (m) are ignored — nothing walkable is there.
const MIN_WALK_Y: f32 = -40.0;
/// Waypoints looked ahead when straightening a path.
const SMOOTH_LOOKAHEAD: usize = 24;
/// A waypoint counts as reached when the body is this close to it in the
/// horizontal plane and within `WAYPOINT_REACHED_HEIGHT` in height — tight
/// enough that it can't cut a corner across a ledge and think it got there
/// while still on the wrong level.
const WAYPOINT_REACHED_DIST: f32 = 0.5;
const WAYPOINT_REACHED_HEIGHT: f32 = 0.6;
/// The tallest jump between neighbouring ground samples `walk_clear` allows —
/// a margin under the mover's `STEP_HEIGHT`, because a body only just able to
/// climb a ledge in theory sticks on it when it clips the corner.
const LEDGE_MAX: f32 = STEP_HEIGHT - 0.2;
/// Within this (m) of a waypoint it always counts as passed, corner cut or not.
const WAYPOINT_ON_DIST: f32 = 0.15;
/// A* gives up after expanding this many nodes.
const MAX_EXPANSIONS: usize = 200_000;
const NO_NODE: u32 = u32::MAX;

/// The 8 neighbour directions in grid steps `(dx, dz)`; the first four are the
/// orthogonal ones (diagonals need both of their orthogonal edges).
const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// One map's walkable graph.
pub struct NavGraph {
    min_x: f32,
    min_z: f32,
    width: i32,
    depth: i32,
    /// Feet position of every node.
    nodes: Vec<Vec3>,
    /// Node ids in each grid cell (`iz * width + ix`), lowest floor first.
    cells: Vec<Vec<u32>>,
    /// Each node's neighbour in each of [`DIRS`], or [`NO_NODE`].
    edges: Vec<[u32; 8]>,
    /// Which connected region each node is in (`NO_NODE` for a region too small
    /// to matter) — paths only exist between nodes of the same region.
    region: Vec<u32>,
}

/// Every map's graph, built once at startup from `MapColliders`.
#[derive(Resource)]
pub struct NavGraphs {
    basic: NavGraph,
    shipment: NavGraph,
    ascension: NavGraph,
    break_point: NavGraph,
}

impl NavGraphs {
    pub fn build(colliders: &MapColliders) -> Self {
        let build = |map: MapId| {
            let world = colliders.world(map);
            let (lo, hi) = world.bounds();
            NavGraph::build(world, lo, hi)
        };
        Self {
            basic: build(MapId::BasicMap),
            shipment: build(MapId::Shipment),
            ascension: build(MapId::Ascension),
            break_point: build(MapId::BreakPoint),
        }
    }

    pub fn graph(&self, map: MapId) -> &NavGraph {
        match map {
            MapId::BasicMap => &self.basic,
            MapId::Shipment | MapId::ShipmentDay => &self.shipment,
            MapId::Ascension => &self.ascension,
            MapId::BreakPoint => &self.break_point,
        }
    }
}

impl NavGraph {
    /// Build the graph for a map whose collision mesh spans `min..max`.
    pub fn build(world: &dyn CollisionWorld, min: Vec3, max: Vec3) -> Self {
        let width = ((max.x - min.x) / CELL).ceil() as i32 + 1;
        let depth = ((max.z - min.z) / CELL).ceil() as i32 + 1;
        let mut g = NavGraph {
            min_x: min.x,
            min_z: min.z,
            width,
            depth,
            nodes: Vec::new(),
            cells: vec![Vec::new(); (width * depth) as usize],
            edges: Vec::new(),
            region: Vec::new(),
        };

        // 1. Nodes: every standable surface in every cell.
        let floor_limit = (min.y - 1.0).max(MIN_WALK_Y);
        let min_normal_y = MAX_SLOPE_DEG.to_radians().cos();
        for iz in 0..depth {
            for ix in 0..width {
                let x = min.x + (ix as f32 + 0.5) * CELL;
                let z = min.z + (iz as f32 + 0.5) * CELL;
                let mut y_from = max.y + 2.0;
                for _ in 0..MAX_LEVELS {
                    let Some(hit) = world.raycast(
                        Vec3::new(x, y_from, z),
                        Vec3::NEG_Y,
                        y_from - floor_limit,
                    ) else {
                        break;
                    };
                    let y = y_from - hit.distance;
                    if hit.normal.y >= min_normal_y && Self::standable(world, x, y, z) {
                        let id = g.nodes.len() as u32;
                        g.nodes.push(Vec3::new(x, y, z));
                        g.cells[(iz * width + ix) as usize].push(id);
                    }
                    y_from = y - MIN_LEVEL_GAP;
                }
            }
        }
        for cell in &mut g.cells {
            cell.sort_by(|a, b| a.cmp(b)); // (ids were pushed top-down; keep a stable order)
        }

        // 2. Edges. Orthogonal first; a diagonal only if both of its
        //    orthogonal edges exist (no cutting corners).
        g.edges = vec![[NO_NODE; 8]; g.nodes.len()];
        for pass in 0..2 {
            for id in 0..g.nodes.len() {
                for k in (pass * 4)..(pass * 4 + 4) {
                    let (dx, dz) = DIRS[k];
                    if pass == 1 {
                        let e = g.edges[id];
                        let ortho_x = if dx > 0 { e[0] } else { e[1] };
                        let ortho_z = if dz > 0 { e[2] } else { e[3] };
                        if ortho_x == NO_NODE || ortho_z == NO_NODE {
                            continue;
                        }
                    }
                    if let Some(to) = g.connect(world, id as u32, dx, dz) {
                        g.edges[id][k] = to;
                    }
                }
            }
        }

        // 3. Label connected regions (treating edges as undirected) and drop
        //    the tiny ones — the insides of solid blocks, slivers.
        g.region = g.label_regions();
        g
    }

    /// Room to stand at `(x, y, z)`: headroom, and the body sphere fits.
    fn standable(world: &dyn CollisionWorld, x: f32, y: f32, z: f32) -> bool {
        if world
            .raycast(Vec3::new(x, y + 0.05, z), Vec3::Y, BODY_HEIGHT)
            .is_some()
        {
            return false;
        }
        let c = Vec3::new(x, y + CHEST_HEIGHT, z);
        world
            .sweep_sphere(c, c + Vec3::Y * 0.02, BODY_RADIUS)
            .is_none()
    }

    fn cell_index(&self, ix: i32, iz: i32) -> Option<usize> {
        (ix >= 0 && iz >= 0 && ix < self.width && iz < self.depth)
            .then(|| (iz * self.width + ix) as usize)
    }

    fn cell_of(&self, p: Vec3) -> (i32, i32) {
        (
            ((p.x - self.min_x) / CELL).floor() as i32,
            ((p.z - self.min_z) / CELL).floor() as i32,
        )
    }

    /// The neighbour of `from` one step `(dx, dz)` away, if it can be walked
    /// to: a floor in that cell at a climbable height, a clear chest-height
    /// line, and continuous ground between.
    fn connect(&self, world: &dyn CollisionWorld, from: u32, dx: i32, dz: i32) -> Option<u32> {
        let a = self.nodes[from as usize];
        let (ix, iz) = self.cell_of(a);
        let idx = self.cell_index(ix + dx, iz + dz)?;
        let run = CELL * ((dx * dx + dz * dz) as f32).sqrt();
        let max_dy = MAX_SLOPE_DEG.to_radians().tan() * run + STEP_SLACK;
        // The floor in that cell closest in height (never hop between levels).
        let to = *self.cells[idx]
            .iter()
            .filter(|&&n| (self.nodes[n as usize].y - a.y).abs() <= max_dy)
            .min_by(|&&p, &&q| {
                let dp = (self.nodes[p as usize].y - a.y).abs();
                let dq = (self.nodes[q as usize].y - a.y).abs();
                dp.total_cmp(&dq)
            })?;
        let b = self.nodes[to as usize];
        if !Self::walk_clear(world, a, b) {
            return None;
        }
        Some(to)
    }

    /// Whether a body can walk the straight line from feet position `a` to `b`:
    /// nothing solid at chest or shin height, and the ground under it stays
    /// where a straight line between the two says it should (no gaps, no
    /// ledges, no stepping onto a different level).
    pub fn walk_clear(world: &dyn CollisionWorld, a: Vec3, b: Vec3) -> bool {
        // (The lower ray sits just above the tallest step a body climbs, 0.6 m —
        // the same limit as `ai::move_bot` — so steps don't count as walls.)
        // Also to either side, out to nearly the body's radius: a line that
        // clips the corner of a ledge or a wall is walkable for a point but not
        // for a body, which would jam on it (the corner between two ramps).
        let dir = Vec3::new(b.x - a.x, 0.0, b.z - a.z).normalize_or_zero();
        let side = Vec3::new(-dir.z, 0.0, dir.x) * (BODY_RADIUS - 0.05);
        for h in [CHEST_HEIGHT, STEP_HEIGHT + 0.05] {
            for offset in [Vec3::ZERO, side, -side] {
                let up = Vec3::Y * h + offset;
                if world.segment_blocked(a + up, b + up) {
                    return false;
                }
            }
        }
        let len = a.distance(b);
        // Where the height changes, walk the ground in steps as fine as the
        // mover's own (a few cm per tick): a jump of more than a step between
        // neighbouring samples is a ledge, and `ai::move_bot` refuses those. On
        // level ground a coarse check for gaps and hummocks is enough.
        let fine = (b.y - a.y).abs() > 0.05;
        let spacing = if fine { 0.1 } else { 0.75 };
        let steps = (len / spacing).ceil().max(1.0) as usize;
        let mut prev = a.y;
        for s in 1..steps {
            let t = s as f32 / steps as f32;
            let p = a.lerp(b, t);
            let above = p.y.max(a.y.max(b.y)).max(prev) + 1.0;
            let Some(hit) = world.raycast(Vec3::new(p.x, above, p.z), Vec3::NEG_Y, 3.0) else {
                return false; // a gap
            };
            let surface = above - hit.distance;
            if fine {
                if (surface - prev).abs() > LEDGE_MAX {
                    return false; // a ledge
                }
                prev = surface;
            } else if (surface - p.y).abs() > 0.5 {
                return false;
            }
        }
        if fine && (b.y - prev).abs() > LEDGE_MAX {
            return false;
        }
        true
    }

    fn label_regions(&self) -> Vec<u32> {
        let n = self.nodes.len();
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for (i, e) in self.edges.iter().enumerate() {
            for &to in e.iter().filter(|&&t| t != NO_NODE) {
                let (ra, rb) = (find(&mut parent, i as u32), find(&mut parent, to));
                if ra != rb {
                    parent[ra as usize] = rb;
                }
            }
        }
        let mut size = vec![0u32; n];
        for i in 0..n as u32 {
            let r = find(&mut parent, i);
            size[r as usize] += 1;
        }
        (0..n as u32)
            .map(|i| {
                let r = find(&mut parent, i);
                if size[r as usize] >= MIN_REGION_NODES {
                    r
                } else {
                    NO_NODE
                }
            })
            .collect()
    }

    /// The nearest node (in a real region) to feet position `p`, searching a few
    /// cells around it and only floors within a body's reach in height.
    pub fn nearest_node(&self, p: Vec3) -> Option<u32> {
        let (cx, cz) = self.cell_of(p);
        let mut best: Option<(f32, u32)> = None;
        for r in 0..=3i32 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let Some(idx) = self.cell_index(cx + dx, cz + dz) else {
                        continue;
                    };
                    for &n in &self.cells[idx] {
                        let q = self.nodes[n as usize];
                        if self.region[n as usize] == NO_NODE || (q.y - p.y).abs() > 2.0 {
                            continue;
                        }
                        let d = q.distance_squared(p);
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, n));
                        }
                    }
                }
            }
            if best.is_some() {
                break; // the nearest ring that has one is close enough
            }
        }
        best.map(|(_, n)| n)
    }

    /// A route for a body standing at feet position `from` to reach `to`: feet
    /// waypoints, straightened, ending at `to`. `None` if either end can't be
    /// placed on the map or nothing connects them.
    pub fn find_path(&self, world: &dyn CollisionWorld, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let start = self.nearest_node(from)?;
        let goal = self.nearest_node(to)?;
        if self.region[start as usize] != self.region[goal as usize] {
            return None; // no walkable connection between them
        }
        let mut nodes = self.astar(start, goal)?;
        // Start from where the body really is, end where it really needs to be.
        nodes.insert(0, from);
        nodes.push(to);
        Some(Self::smooth(world, nodes))
    }

    fn astar(&self, start: u32, goal: u32) -> Option<Vec<Vec3>> {
        #[derive(Copy, Clone)]
        struct Open(f32, u32);
        impl PartialEq for Open {
            fn eq(&self, o: &Self) -> bool {
                self.0 == o.0
            }
        }
        impl Eq for Open {}
        impl PartialOrd for Open {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Open {
            // Min-heap on the estimated total cost.
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.total_cmp(&self.0)
            }
        }

        thread_local! {
            /// Per-thread scratch: per-node best cost, parent, and the search
            /// generation each was last written in (so nothing is cleared).
            static SCRATCH: RefCell<(Vec<f32>, Vec<u32>, Vec<u32>, u32)> =
                const { RefCell::new((Vec::new(), Vec::new(), Vec::new(), 0)) };
        }
        SCRATCH.with(|s| {
            let (g, parent, stamp, generation) = &mut *s.borrow_mut();
            let n = self.nodes.len();
            if g.len() < n {
                g.resize(n, 0.0);
                parent.resize(n, NO_NODE);
                stamp.resize(n, 0);
            }
            *generation = generation.wrapping_add(1).max(1);
            let generation = *generation;
            let goal_pos = self.nodes[goal as usize];
            let h = |i: u32| self.nodes[i as usize].distance(goal_pos);

            let mut open = BinaryHeap::new();
            g[start as usize] = 0.0;
            parent[start as usize] = NO_NODE;
            stamp[start as usize] = generation;
            open.push(Open(h(start), start));
            let mut expansions = 0;
            while let Some(Open(_, cur)) = open.pop() {
                if cur == goal {
                    let mut path = Vec::new();
                    let mut at = goal;
                    while at != NO_NODE {
                        path.push(self.nodes[at as usize]);
                        at = parent[at as usize];
                    }
                    path.reverse();
                    return Some(path);
                }
                expansions += 1;
                if expansions > MAX_EXPANSIONS {
                    return None;
                }
                let gc = g[cur as usize];
                for &next in &self.edges[cur as usize] {
                    if next == NO_NODE || self.region[next as usize] == NO_NODE {
                        continue;
                    }
                    let cost = gc + self.nodes[cur as usize].distance(self.nodes[next as usize]);
                    if stamp[next as usize] != generation || cost < g[next as usize] {
                        stamp[next as usize] = generation;
                        g[next as usize] = cost;
                        parent[next as usize] = cur;
                        open.push(Open(cost + h(next), next));
                    }
                }
            }
            None
        })
    }

    /// Drop every waypoint that a body can walk past in a straight line.
    fn smooth(world: &dyn CollisionWorld, pts: Vec<Vec3>) -> Vec<Vec3> {
        if pts.len() <= 2 {
            return pts;
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < pts.len() - 1 {
            let mut j = (i + SMOOTH_LOOKAHEAD).min(pts.len() - 1);
            while j > i + 1 && !Self::walk_clear(world, pts[i], pts[j]) {
                j -= 1;
            }
            out.push(pts[j]);
            i = j;
        }
        out
    }

    /// The waypoint a body at feet position `feet` should head for next: skips
    /// (advancing `*index`) every waypoint it has already reached, and returns
    /// `None` once the whole path is done. A waypoint that's merely *near* is
    /// only passed early if cutting the corner toward the next one is walkable
    /// from where the body actually is — otherwise it keeps going to the
    /// waypoint itself, so it doesn't clip a ledge on the inside of a turn.
    pub fn next_waypoint(
        world: &dyn CollisionWorld,
        path: &[Vec3],
        index: &mut usize,
        feet: Vec3,
    ) -> Option<Vec3> {
        while let Some(&wp) = path.get(*index) {
            let flat = Vec3::new(wp.x - feet.x, 0.0, wp.z - feet.z).length();
            let near = flat < WAYPOINT_REACHED_DIST && (wp.y - feet.y).abs() < WAYPOINT_REACHED_HEIGHT;
            let cut_ok = flat < WAYPOINT_ON_DIST
                || path.get(*index + 1).is_none_or(|&next| Self::walk_clear(world, feet, next));
            if near && cut_ok {
                *index += 1;
            } else {
                return Some(wp);
            }
        }
        None
    }

    /// Node count (for tests).
    #[cfg(test)]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// How many nodes are in a real region (not a dropped sliver).
    pub fn usable_count(&self) -> usize {
        self.region.iter().filter(|&&r| r != NO_NODE).count()
    }

    /// Whether `a` and `b` (feet positions) are on the same connected region.
    #[cfg(test)]
    pub fn connected(&self, a: Vec3, b: Vec3) -> bool {
        match (self.nearest_node(a), self.nearest_node(b)) {
            (Some(x), Some(y)) => self.region[x as usize] == self.region[y as usize],
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::OnceLock;

    /// Built once and shared by every test in the crate (it takes a moment).
    pub(crate) fn built() -> &'static (MapColliders, NavGraphs) {
        static BUILT: OnceLock<(MapColliders, NavGraphs)> = OnceLock::new();
        BUILT.get_or_init(|| {
            let c = MapColliders::load();
            let n = NavGraphs::build(&c);
            (c, n)
        })
    }

    /// A path is a chain of walk-clear segments from where it starts to where
    /// it ends.
    fn assert_walkable(world: &dyn CollisionWorld, from: Vec3, to: Vec3, path: &[Vec3]) {
        assert!(!path.is_empty());
        assert!(path.last().unwrap().distance(to) < 0.01, "ends at {:?}, not {to:?}", path.last());
        // (The path doesn't repeat the start — the body is already there.)
        let mut prev = from;
        for &p in path {
            assert!(
                NavGraph::walk_clear(world, prev, p),
                "segment {prev:?} -> {p:?} isn't walkable"
            );
            prev = p;
        }
    }

    fn length(path: &[Vec3]) -> f32 {
        path.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    #[test]
    fn every_map_builds_a_usable_graph_quickly() {
        let t = std::time::Instant::now();
        let (_, n) = built();
        assert!(t.elapsed().as_secs() < 30, "building took {:?}", t.elapsed());
        for m in [MapId::BasicMap, MapId::Shipment, MapId::Ascension] {
            let g = n.graph(m);
            assert!(g.usable_count() > 1000, "{m:?}: only {} usable nodes", g.usable_count());
        }
    }

    #[test]
    fn a_path_across_shipment_goes_around_the_containers() {
        let (c, n) = built();
        let (w, g) = (c.world(MapId::Shipment), n.graph(MapId::Shipment));
        // Opposite sides of the yard — containers stand in the way of a straight walk.
        let snap = |p: Vec3| g.nodes[g.nearest_node(p).unwrap() as usize];
        let (from, to) = (snap(Vec3::new(-20.0, 0.0, -20.0)), snap(Vec3::new(20.0, 0.0, 20.0)));
        let path = g.find_path(w, from, to).expect("a route across the yard");
        assert_walkable(w, from, to, &path);
        assert!(length(&path) >= from.distance(to) - 0.01);
        assert!(length(&path) < from.distance(to) * 2.5, "absurdly long route: {}", length(&path));
    }

    #[test]
    fn many_random_paths_on_ascension_never_cross_a_wall() {
        let (c, n) = built();
        let (w, g) = (c.world(MapId::Ascension), n.graph(MapId::Ascension));
        let mut found = 0;
        for i in 0..40u64 {
            let r = |k: u64| shared::bots::rand01(i * 977 + k);
            // Anywhere on the yard side of the map.
            let a = Vec3::new(-60.0 + r(1) * 55.0, 0.0, -60.0 + r(2) * 120.0);
            let b = Vec3::new(-60.0 + r(3) * 55.0, 0.0, -60.0 + r(4) * 120.0);
            if let Some(path) = g.find_path(w, a, b) {
                assert_walkable(w, a, b, &path);
                found += 1;
            }
        }
        assert!(found >= 30, "only {found} of 40 yard routes were found");
    }

    #[test]
    fn a_path_climbs_ascensions_ramps_to_the_top_floor() {
        let (c, n) = built();
        let (w, g) = (c.world(MapId::Ascension), n.graph(MapId::Ascension));
        // Ground level inside the building, to the top platform (y ≈ 13.3).
        let (from, to) = (Vec3::new(9.5, 0.0, -6.5), Vec3::new(26.5, 13.3, -9.5));
        assert!(g.connected(from, to), "the ground and top floors are separate regions");
        let path = g.find_path(w, from, to).expect("a route up the ramps");
        assert_walkable(w, from, to, &path);
        assert!(path.iter().any(|p| p.y > 5.8), "never reached the middle level");
        assert!(length(&path) > 39.0, "a climb of 13 m can't be {} m long", length(&path));
    }

    #[test]
    fn two_regions_with_no_walkable_link_have_no_path() {
        let (c, n) = built();
        let (w, g) = (c.world(MapId::Ascension), n.graph(MapId::Ascension));
        // The yard and the walled-in building: whichever region a point is in,
        // asking for a path to the other must not invent one.
        let yard = Vec3::new(-32.5, 0.0, 0.0);
        let inside = Vec3::new(26.5, 13.3, -9.5);
        if !g.connected(yard, inside) {
            assert!(g.find_path(w, yard, inside).is_none());
        }
    }

    /// The point of the exercise: walk a real body along a real path with the
    /// real mover and check it gets there — the graph and `ai::move_bot` must
    /// agree on what can be walked, ramps included.
    #[test]
    fn a_bot_can_actually_walk_a_path_up_the_ramps() {
        let (c, n) = built();
        let (w, g) = (c.world(MapId::Ascension), n.graph(MapId::Ascension));
        let (from, to) = (Vec3::new(9.5, 0.0, -6.5), Vec3::new(26.5, 13.3, -9.5));
        let path = g.find_path(w, from, to).expect("a route up the ramps");

        let (mut feet, mut vy, mut idx) = (from, 0.0f32, 0usize);
        let dt = 1.0 / 64.0;
        for tick in 0..(120 * 64) {
            let Some(wp) = NavGraph::next_waypoint(w, &path, &mut idx, feet) else {
                assert!(
                    (feet.y - to.y).abs() < 1.0 && Vec3::new(feet.x - to.x, 0.0, feet.z - to.z).length() < 1.5,
                    "finished the path at {feet:?}, not {to:?}"
                );
                println!("walked it in {:.1} s", tick as f32 / 64.0);
                return;
            };
            let wish = Vec3::new(wp.x - feet.x, 0.0, wp.z - feet.z).normalize_or_zero();
            crate::ai::move_bot(w, &mut feet, &mut vy, wish, 5.0, dt);
        }
        panic!("didn't reach the top in 120 s; stuck at {feet:?} heading for waypoint {idx} of {}: {:?}", path.len(), path.get(idx));
    }

    /// Every hand-placed spawn point stands on walkable ground — not inside a
    /// container, not over a gap — with room for a body.
    #[test]
    fn every_designated_spawn_point_is_on_walkable_ground() {
        let (c, n) = built();
        for map in [MapId::Shipment, MapId::ShipmentDay] {
            let (w, g) = (c.world(map), n.graph(map));
            for (i, p) in shared::spawns::designated_spawns(map).unwrap().iter().enumerate() {
                let at = Vec3::new(p.x, 0.0, p.z);
                let node = g.nearest_node(at).unwrap_or_else(|| {
                    panic!("{map:?} spawn #{} at ({}, {}) has no walkable ground near it", i + 1, p.x, p.z)
                });
                let q = g.nodes[node as usize];
                assert!(
                    Vec3::new(q.x - at.x, 0.0, q.z - at.z).length() <= 1.0 && q.y.abs() < 0.5,
                    "{map:?} spawn #{} at ({}, {}): nearest floor is {q:?}",
                    i + 1,
                    p.x,
                    p.z
                );
                assert!(NavGraph::standable(w, p.x, 0.0, p.z), "{map:?} spawn #{}: no room to stand", i + 1);
            }
        }
    }

    #[test]
    fn dump() {
        let (_, n) = built();
        for m in [MapId::BasicMap, MapId::Shipment, MapId::Ascension] {
            let g = n.graph(m);
            println!("{m:?}: {} nodes, {} usable", g.node_count(), g.usable_count());
        }
    }

    /// Spawns / bot respawns on Break Point (hand-listed wall boxes, no
    /// designated points) must stand on ground a bot can walk from — not shut
    /// inside a room the graph can't leave.
    #[test]
    fn break_point_spawns_and_bot_starts_are_in_the_big_connected_region() {
        let (_, n) = built();
        let g = n.graph(MapId::BreakPoint);
        let anchor = shared::map::spawn_center(MapId::BreakPoint);
        let mut checked = 0;
        for seed in 0..300u64 {
            for (pos, _) in [
                shared::spawns::spawn_point(seed, &[], MapId::BreakPoint),
                shared::bots::respawn_pose(seed, MapId::BreakPoint),
            ] {
                assert!(
                    g.connected(anchor, pos),
                    "seed {seed}: {pos:?} isn't connected to the map centre"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 600);
    }
}
