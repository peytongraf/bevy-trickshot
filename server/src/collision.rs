//! Server-side map collision: the same `.glb` collision models the client
//! builds its colliders from, parsed into one world-space triangle mesh per
//! map so thrown knives can bounce off exactly the geometry players walk on.
//!
//! **The client and the server always use the same files and the same
//! placement.** The models are embedded from `client/assets/models/` at build
//! time (so there's no runtime path to get wrong, and a deployed server can
//! never be missing one) and placed with `shared::map::placement` — the same
//! constants the client's `MapModel` transform is initialised from. The
//! flip side: changing one of those `.glb` files means rebuilding / redeploying
//! the server too, or its knives will bounce off the old geometry.
//!
//! The meshes are gathered the way the client's `AsyncSceneCollider` does it —
//! every triangle mesh in the scene, each with its node's full transform — and
//! built with the same `MERGE_DUPLICATE_VERTICES` flag `bevy_rapier3d` uses.

use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::Resource;
use gltf::mesh::Mode;
use parry3d::math::{Isometry, Point, Vector};
use parry3d::query::{cast_shapes, Ray, RayCast, ShapeCastOptions};
use parry3d::shape::{Ball, TriMesh, TriMeshFlags};
use shared::map::{self, CollisionWorld, MapPlacement, RayHit, WorldHit};
use shared::MapId;

const BASIC_MAP_GLB: &[u8] = include_bytes!("../../client/assets/models/basic_map.glb");
const SHIPMENT_GLB: &[u8] = include_bytes!("../../client/assets/models/shipment.glb");
// (The file really is spelled `ascenion_map.glb`.)
const ASCENSION_GLB: &[u8] = include_bytes!("../../client/assets/models/ascenion_map.glb");

const BREAK_POINT_GLB: &[u8] = include_bytes!("../../client/assets/models/break_point_map.glb");

/// One map's collision mesh, in world space.
pub struct MapMesh {
    mesh: TriMesh,
}

/// Every map's collision mesh, built once at startup.
#[derive(Resource)]
pub struct MapColliders {
    basic: MapMesh,
    shipment: MapMesh,
    ascension: MapMesh,
    break_point: MapMesh,
}

impl MapColliders {
    /// Parse the embedded collision models. Panics if one is unreadable — a
    /// server that can't collide knives with walls shouldn't start quietly.
    pub fn load() -> Self {
        Self {
            basic: MapMesh::from_glb(BASIC_MAP_GLB, map::placement(MapId::BasicMap))
                .expect("basic_map.glb collision model"),
            shipment: MapMesh::from_glb(SHIPMENT_GLB, map::placement(MapId::Shipment))
                .expect("shipment.glb collision model"),
            ascension: MapMesh::from_glb(ASCENSION_GLB, map::placement(MapId::Ascension))
                .expect("ascenion_map.glb collision model"),
            break_point: MapMesh::from_glb(BREAK_POINT_GLB, map::placement(MapId::BreakPoint))
                .expect("break_point_map.glb collision model"),
        }
    }

    /// The collision mesh for `map` (both Shipment variants share one).
    pub fn world(&self, map: MapId) -> &MapMesh {
        match map {
            MapId::BasicMap => &self.basic,
            MapId::Shipment | MapId::ShipmentDay => &self.shipment,
            MapId::Ascension => &self.ascension,
            MapId::BreakPoint => &self.break_point,
        }
    }
}

impl MapMesh {
    fn from_glb(bytes: &[u8], placement: MapPlacement) -> Result<Self, String> {
        let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| e.to_string())?;
        let blob = gltf.blob.as_deref();
        let scene = gltf
            .document
            .default_scene()
            .or_else(|| gltf.document.scenes().next())
            .ok_or("no scene")?;

        let root = Mat4::from_scale_rotation_translation(
            Vec3::splat(placement.scale),
            Quat::from_rotation_y(placement.yaw_deg.to_radians()),
            placement.position,
        );
        let mut vertices: Vec<Point<f32>> = Vec::new();
        let mut indices: Vec<[u32; 3]> = Vec::new();
        for node in scene.nodes() {
            collect(&node, root, blob, &mut vertices, &mut indices)?;
        }
        let mesh = TriMesh::with_flags(vertices, indices, TriMeshFlags::MERGE_DUPLICATE_VERTICES)
            .map_err(|e| format!("{e:?}"))?;
        Ok(Self { mesh })
    }

    /// The mesh's world-space bounds: `(min, max)`.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let a = self.mesh.local_aabb();
        (
            Vec3::new(a.mins.x, a.mins.y, a.mins.z),
            Vec3::new(a.maxs.x, a.maxs.y, a.maxs.z),
        )
    }
}

/// Walk `node` and its children, appending every triangle mesh's vertices
/// (transformed into world space) and indices.
fn collect(
    node: &gltf::Node,
    parent: Mat4,
    blob: Option<&[u8]>,
    vertices: &mut Vec<Point<f32>>,
    indices: &mut Vec<[u32; 3]>,
) -> Result<(), String> {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            if prim.mode() != Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|buffer| match buffer.source() {
                gltf::buffer::Source::Bin => blob,
                gltf::buffer::Source::Uri(_) => None,
            });
            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or("mesh without positions")?
                .collect();
            let base = vertices.len() as u32;
            for p in &positions {
                let w = world.transform_point3(Vec3::from_array(*p));
                vertices.push(Point::new(w.x, w.y, w.z));
            }
            let idx: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            for tri in idx.chunks_exact(3) {
                indices.push([base + tri[0], base + tri[1], base + tri[2]]);
            }
        }
    }
    for child in node.children() {
        collect(&child, world, blob, vertices, indices)?;
    }
    Ok(())
}

fn point(v: Vec3) -> Point<f32> {
    Point::new(v.x, v.y, v.z)
}

impl CollisionWorld for MapMesh {
    fn segment_blocked(&self, a: Vec3, b: Vec3) -> bool {
        let d = b - a;
        let len = d.length();
        if len < 1e-6 {
            return false;
        }
        self.raycast(a, d / len, len).is_some()
    }

    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
        let ray = Ray::new(point(origin), Vector::new(dir.x, dir.y, dir.z));
        let hit = self
            .mesh
            .cast_local_ray_and_get_normal(&ray, max_dist, false)?;
        // Meshes aren't guaranteed consistent winding: make the normal face
        // back toward the ray's origin.
        let mut n = Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z);
        if n.dot(dir) > 0.0 {
            n = -n;
        }
        Some(RayHit {
            distance: hit.time_of_impact,
            normal: n.normalize_or_zero(),
        })
    }

    fn sweep_sphere(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
        let d = to - from;
        if d.length_squared() < 1e-12 {
            return None;
        }
        let hit = cast_shapes(
            &Isometry::translation(from.x, from.y, from.z),
            &Vector::new(d.x, d.y, d.z),
            &Ball::new(radius),
            &Isometry::identity(),
            &Vector::zeros(),
            &self.mesh,
            ShapeCastOptions {
                max_time_of_impact: 1.0,
                target_distance: 0.0,
                stop_at_penetration: true,
                compute_impact_geometry_on_penetration: true,
            },
        )
        .ok()??;

        // `normal1` is the ball's outward normal at the contact — pointing
        // into the mesh — so the surface normal toward the sphere is its
        // negation. Meshes aren't guaranteed consistent winding, so make sure
        // it faces against the travel direction.
        let mut n = Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z) * -1.0;
        if n.dot(d) > 0.0 {
            n = -n;
        }
        Some(WorldHit {
            fraction: hit.time_of_impact.clamp(0.0, 1.0),
            normal: n.normalize_or_zero(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colliders() -> MapColliders {
        MapColliders::load()
    }

    #[test]
    fn both_maps_load_with_geometry() {
        let c = colliders();
        for map in [MapId::BasicMap, MapId::Shipment, MapId::Ascension, MapId::BreakPoint] {
            let (lo, hi) = c.world(map).bounds();
            assert!(hi.x > lo.x && hi.y >= lo.y && hi.z > lo.z, "{map:?}: {lo} .. {hi}");
        }
    }

    #[test]
    fn shipment_is_placed_at_its_shared_scale() {
        // The playable yard is roughly -52..55 across in the model's native
        // units (see `shared::map`'s wall boxes); the server must place it at
        // `SHIPMENT_SCALE`, not native size.
        let (lo, hi) = colliders().world(MapId::Shipment).bounds();
        let (w, d) = (hi.x - lo.x, hi.z - lo.z);
        let native_w = 107.0;
        let expected = native_w * map::SHIPMENT_SCALE;
        assert!(
            (w - expected).abs() < expected * 0.35 && d < native_w * 0.75,
            "width {w} (expected about {expected}), depth {d}",
        );
    }

    #[test]
    fn basic_map_is_placed_at_its_shared_offset() {
        let (lo, hi) = colliders().world(MapId::BasicMap).bounds();
        let p = map::BASIC_MAP_PLACEMENT.position;
        // The placement's own offset shifts the whole mesh — its centre can't
        // be sitting on the world origin by accident.
        let centre = (lo + hi) * 0.5;
        assert!(
            (centre.x - p.x).abs() < 40.0 && (centre.z - p.z).abs() < 40.0,
            "centre {centre} vs placement {p}",
        );
    }

    #[test]
    fn ascension_is_placed_at_the_basic_maps_scale() {
        // A ±100 m ground plane and a building rising to ~20 m natively.
        let s = map::BASIC_MAP_PLACEMENT.scale;
        assert_eq!(map::placement(MapId::Ascension).scale, s);
        let (lo, hi) = colliders().world(MapId::Ascension).bounds();
        assert!((lo.x + 100.0 * s).abs() < 1.0 && (hi.x - 100.0 * s).abs() < 1.5, "x {lo} .. {hi}");
        assert!(hi.y > 19.0 * s && hi.y < 22.0 * s, "height {}", hi.y);
    }

    #[test]
    fn a_ball_dropped_in_the_ascension_yard_lands_on_the_ground_and_the_roof_is_higher() {
        let c = colliders();
        let w = c.world(MapId::Ascension);
        // The open yard west of the building: bare ground.
        let yard = w.raycast(Vec3::new(-32.5, 40.0, 0.0), Vec3::NEG_Y, 100.0).expect("ground");
        assert!((yard.distance - 40.0).abs() < 0.5, "yard at {}", yard.distance);
        assert!(yard.normal.y > 0.9);
        // Over the upper level: something is much higher than the ground.
        let roof = w.raycast(Vec3::new(32.5, 40.0, -9.0), Vec3::NEG_Y, 100.0).expect("roof");
        assert!(roof.distance < 30.0, "expected a raised surface, hit at {}", roof.distance);
    }

    #[test]
    fn break_point_is_placed_at_point_six_scale() {
        // x ±60, z ±100 natively, walls up to 20 m tall.
        let s = 0.6;
        assert_eq!(map::placement(MapId::BreakPoint).scale, s);
        let (lo, hi) = colliders().world(MapId::BreakPoint).bounds();
        assert!((lo.x + 61.0 * s).abs() < 1.0 && (hi.x - 61.0 * s).abs() < 1.0, "x {lo} .. {hi}");
        assert!((lo.z + 101.0 * s).abs() < 1.0 && (hi.z - 101.0 * s).abs() < 1.0, "z {lo} .. {hi}");
        // Open ground near the middle (the origin itself is under a raised slab).
        let c = colliders();
        let ground = c
            .world(MapId::BreakPoint)
            .raycast(Vec3::new(0.0, 10.0, -4.0), Vec3::NEG_Y, 30.0)
            .expect("ground near the origin");
        assert!((ground.distance - 10.0).abs() < 0.5, "ground at {}", ground.distance);
    }

    #[test]
    fn a_falling_sphere_lands_on_the_shipment_floor() {
        let world = colliders();
        let w = world.world(MapId::Shipment);
        // Somewhere inside the yard (the scaled model spans about -30..30).
        let mut landed = 0;
        for &(x, z) in &[(0.0, 0.0), (10.0, 0.0), (25.0, -20.0), (-20.0, 20.0)] {
            let from = Vec3::new(x, 30.0, z);
            let to = Vec3::new(x, -30.0, z);
            if let Some(hit) = w.sweep_sphere(from, to, 0.06) {
                assert!(hit.normal.y > 0.5, "({x},{z}): normal {}", hit.normal);
                let y = from.y + (to.y - from.y) * hit.fraction;
                assert!(y > -5.0 && y < 25.0, "({x},{z}): landed at y {y}");
                landed += 1;
            }
        }
        assert!(landed >= 2, "only {landed} of 4 drops hit anything");
    }

    #[test]
    fn a_sphere_bounces_off_a_wall_it_flies_into() {
        // Fire one sphere along each cardinal direction from the middle of the
        // yard, a metre off the floor — every direction should reach a
        // container or the outer wall, with a horizontal-ish normal.
        let w = colliders();
        let w = w.world(MapId::Shipment);
        let origin = Vec3::new(0.0, 1.0, 0.0);
        let mut walls = 0;
        for dir in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
            if let Some(hit) = w.sweep_sphere(origin, origin + dir * 80.0, 0.06) {
                assert!(hit.normal.dot(dir) < 0.0, "normal faces away from travel");
                assert!(hit.normal.y.abs() < 0.5, "not a wall: {}", hit.normal);
                walls += 1;
            }
        }
        assert!(walls >= 3, "only {walls} of 4 directions hit a wall");
    }

    #[test]
    fn thrown_knives_stay_inside_the_yard_and_come_to_rest() {
        use shared::throwing_knife::KnifeBody;
        let c = colliders();
        let w = c.world(MapId::Shipment);
        let (lo, hi) = w.bounds();
        let dt = 1.0 / shared::TICK_HZ as f32;
        let mut rested = 0;
        // A fan of throws at different headings and elevations from the yard's
        // centre: each must stop within the flight cap, never tunnel out past
        // the outer wall or through the floor.
        for i in 0..48 {
            let yaw = i as f32 / 48.0 * std::f32::consts::TAU;
            let pitch = (i % 5) as f32 * 0.12 - 0.15;
            let dir = Vec3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos());
            let mut k = KnifeBody::thrown(Vec3::new(0.0, 1.6, 0.0), dir);
            let (mut steps, mut was_inside, mut prev_y) = (0, true, k.pos.y);
            while !k.finished() && steps < 2000 {
                assert!(k.step(dt, w, &[]).is_none());
                // Crossing the yard's edge while low means it went *through* the
                // outer wall (a high throw can legitimately sail over it).
                let inside = k.pos.x > lo.x
                    && k.pos.x < hi.x
                    && k.pos.z > lo.z
                    && k.pos.z < hi.z;
                assert!(
                    !(was_inside && !inside && prev_y < 2.0 && k.pos.y < 2.0),
                    "throw {i}: tunnelled out of the yard at {}",
                    k.pos,
                );
                was_inside = inside;
                prev_y = k.pos.y;
                // (Outside the yard there is no floor — a knife that cleared the wall
                // just falls away, and is removed at `KILL_FLOOR_Y`.)
                assert!(!inside || k.pos.y > -1.0, "throw {i}: fell through the floor at {}", k.pos);
                steps += 1;
            }
            assert!(k.finished(), "throw {i}: still going after {steps} steps");
            // Every knife either came to rest in the yard or (a high throw)
            // left it over the wall — none is still bouncing when its time
            // runs out.
            assert!(k.resting || !was_inside, "throw {i}: never settled, at {}", k.pos);
            if k.resting {
                rested += 1;
            }
        }
        assert!(rested >= 20, "only {rested} of 48 knives came to rest");
    }

    #[test]
    fn raycast_finds_the_first_surface_and_respects_max_dist() {
        let c = colliders();
        let w = c.world(MapId::Shipment);
        let o = Vec3::new(0.0, 30.0, 0.0);
        let hit = w.raycast(o, Vec3::NEG_Y, 100.0).expect("floor below");
        assert!((hit.distance - 30.0).abs() < 0.5, "floor at {} m below, expected ~30", hit.distance);
        assert!(hit.normal.y > 0.9, "floor normal points up: {}", hit.normal);
        assert!(w.raycast(o, Vec3::NEG_Y, 10.0).is_none(), "beyond max_dist");
        assert!(w.raycast(o, Vec3::Y, 100.0).is_none(), "nothing above");
    }

    #[test]
    fn segment_blocked_agrees_with_the_sweep() {
        let w = colliders();
        let w = w.world(MapId::Shipment);
        assert!(w.segment_blocked(Vec3::new(0.0, 30.0, 0.0), Vec3::new(0.0, -30.0, 0.0)));
        assert!(!w.segment_blocked(Vec3::new(0.0, 80.0, 0.0), Vec3::new(0.0, 60.0, 0.0)));
    }
}
