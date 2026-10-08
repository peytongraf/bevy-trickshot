//! Call-of-Duty-style hit detection: whatever we fire is tested against
//! every enemy's actual animated model, exactly as it's drawn on this screen
//! — an arm flung out, a hand, a finger counts; a hair past it doesn't.
//!
//! [`Models::hits_along`] does the testing: it skins each enemy's mesh near
//! a path on the CPU in its current pose (the same joint matrices the GPU
//! drew last frame with), runs the path through its triangles, and notes
//! the nearest spot it struck on each — and which bone that spot hangs off,
//! for head / upper / lower body ([`shared::hitbox::zone_of_bone`]).
//!
//! * A bullet ([`claim_shot_hits`]): its whole path
//!   ([`shared::ballistics::shot_path`]) the moment it's fired; every hit
//!   rides up with the shot ([`PlayerInput::fire_hits`]).
//! * Our Ray Gun bolt and thrown knife ([`track_own_projectiles`]): followed
//!   in flight — the bolt down its straight line, the knife down the path
//!   the server works out for it ([`shared::KnifeFlight`]) — each frame's
//!   stretch tested against the models as they are that frame; the first
//!   enemy one strikes is claimed ([`shared::ProjectileClaim`]). A bolt that
//!   reaches its surface without striking one says so too, so it bursts
//!   there at once, here and on the server.
//!
//! The server checks every claim over before anything takes damage. (A knife
//! stab is the server's own, a Call-of-Duty lunge: whoever's close and
//! roughly under the crosshair.)
//!
//! Enemies: other players and `FreeForAll` bots (the soldier — not its gun),
//! `Freestyle` target bots, and `Zombies` zombies, hellhounds and bosses.

use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::render::mesh::{Indices, VertexAttributeValues};
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::{LocalId, MessageReceiver, TriggerSender};
use shared::ballistics::{ground_impact, shot_path};
use shared::hitbox::zone_of_bone;
use shared::weapon::{WeaponId, RAYGUN_BOLT_SPEED};
use shared::{Bot, ClaimTarget, GameMode, Lobby, PlayerId, PlayerPose, Projectile, ShotClaim};

use crate::net::{BotPose, GameClient, RemoteAvatar};
use crate::{AppState, PendingShot, WorldModelCamera};

pub(crate) struct HitDetectionPlugin;

impl Plugin for HitDetectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShotHits>()
            .init_resource::<OwnProjectiles>()
            .add_systems(Update, track_own_projectiles.run_if(in_state(AppState::InGame)))
            .add_systems(OnExit(AppState::InGame), forget);
    }
}

/// This tick's shot's hits ([`claim_shot_hits`]), for `net::write_input` to
/// send with it. `None` with no shot waiting.
#[derive(Resource, Default)]
pub(crate) struct ShotHits(pub(crate) Option<Vec<ShotClaim>>);

/// Our Ray Gun bolts and thrown knives still in flight.
#[derive(Resource, Default)]
pub(crate) struct OwnProjectiles(Vec<OwnProjectile>);

struct OwnProjectile {
    projectile: Projectile,
    /// Where it was fired / thrown from, exactly as we sent it — how the
    /// server knows which it was.
    origin: Vec3,
    /// When (`Time<Virtual>`) it left.
    fired_at: f32,
    /// Where it is when: `(seconds since it left, position)`. A knife's
    /// comes from the server ([`shared::KnifeFlight`]).
    path: Option<Vec<(f32, Vec3)>>,
    /// How far (s) into its flight it's been tested so far.
    tested: f32,
}

impl OwnProjectiles {
    /// A knife we just threw from `origin` — tested once the server sends
    /// its path.
    pub(crate) fn thrown_knife(&mut self, origin: Vec3, now: f32) {
        self.0.push(OwnProjectile {
            projectile: Projectile::Knife,
            origin,
            fired_at: now,
            path: None,
            tested: 0.0,
        });
    }
}

/// A knife's path that never comes (s) — given up on.
const KNIFE_PATH_WAIT_SECS: f32 = 3.0;

/// Most claims one shot sends (nearest first).
const MAX_CLAIMS: usize = 16;

/// `soldier.glb`'s gun: skinned to the right hand like the body, but no
/// part of the player — a shot that clips it doesn't count.
const SOLDIER_GUN_MESH: &str = "Mesh1/Primitive0";

/// A sphere (centre above the feet, radius, in m — generous) every pose of
/// each kind of model stays inside, to skip skinning one the shot comes
/// nowhere near.
fn bounds(boss: bool) -> (f32, f32) {
    if boss { (1.5, 3.0) } else { (1.0, 2.0) }
}

fn forget(mut hits: ResMut<ShotHits>, mut own: ResMut<OwnProjectiles>) {
    hits.0 = None;
    own.0.clear();
}

/// What kind of enemy a model is — who a projectile may hit depends on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// Another player, or a `FreeForAll` bot.
    Player,
    /// A `Zombies` zombie, hellhound or boss.
    Zombie,
    /// A `Freestyle` target bot.
    Bot,
}

/// One enemy a path struck ([`Models::hits_along`]).
pub(crate) struct ModelHit {
    /// How far down the path (m).
    pub(crate) along: f32,
    pub(crate) kind: Kind,
    pub(crate) claim: ShotClaim,
}

/// Every enemy's model, as drawn — for testing what we fire against them.
#[derive(SystemParam)]
pub(crate) struct Models<'w, 's> {
    avatars: Query<
        'w,
        's,
        (
            Entity,
            &'static RemoteAvatar,
            &'static GlobalTransform,
            Has<crate::SoldierVisual>,
            Has<crate::boss::BossVisual>,
        ),
    >,
    sources: Query<'w, 's, (&'static PlayerPose, Option<&'static PlayerId>, Option<&'static BotPose>)>,
    bots: Query<'w, 's, &'static Bot>,
    children: Query<'w, 's, &'static Children>,
    skinned: Query<'w, 's, (&'static Mesh3d, &'static SkinnedMesh)>,
    bones: Query<'w, 's, &'static GlobalTransform>,
    names: Query<'w, 's, &'static Name>,
    meshes: Res<'w, Assets<Mesh>>,
    bindposes: Res<'w, Assets<SkinnedMeshInverseBindposes>>,
}

impl Models<'_, '_> {
    /// Every living enemy `path` (a polyline) passes through, nearest
    /// first: where on them, and on which part.
    pub(crate) fn hits_along(&self, path: &[Vec3]) -> Vec<ModelHit> {
        let legs = path_legs(path);
        let mut hits = Vec::new();
        for (avatar_e, avatar, avatar_tf, soldier, boss) in &self.avatars {
            let Ok((pose, id, stand_in)) = self.sources.get(avatar.src) else {
                continue;
            };
            if !pose.alive {
                continue;
            }
            let (target, kind) = match (id, stand_in) {
                (_, Some(stand_in)) => match self.bots.get(stand_in.bot) {
                    Ok(bot) if bot.alive => (ClaimTarget::Bot(bot.pos.to_array()), Kind::Bot),
                    _ => continue,
                },
                (Some(id), None) => (
                    ClaimTarget::Player(id.0.to_bits()),
                    if pose.zombie.is_zombie() { Kind::Zombie } else { Kind::Player },
                ),
                (None, None) => continue,
            };
            let (up, radius) = bounds(boss);
            let centre = avatar_tf.translation() + Vec3::Y * up;
            if !legs.iter().any(|leg| leg.distance_to(centre) <= radius) {
                continue;
            }

            // The nearest spot it strikes on any of its meshes.
            let mut best: Option<(f32, Vec3, Entity)> = None;
            for e in self.children.iter_descendants(avatar_e) {
                let Ok((mesh3d, skin)) = self.skinned.get(e) else {
                    continue;
                };
                if soldier && mesh3d.0.path().and_then(|p| p.label()) == Some(SOLDIER_GUN_MESH) {
                    continue;
                }
                let (Some(mesh), Some(inverse)) = (self.meshes.get(&mesh3d.0), self.bindposes.get(&skin.inverse_bindposes))
                else {
                    continue;
                };
                let Some(verts) = SkinnedVerts::new(mesh, skin, inverse, &self.bones) else {
                    continue;
                };
                if let Some((along, point, joint)) = verts.first_hit(&legs, centre, radius) {
                    if best.is_none_or(|(b, ..)| along < b) {
                        best = Some((along, point, joint));
                    }
                }
            }
            let Some((along, point, joint)) = best else {
                continue;
            };
            let bone = self.names.get(joint).map_or("?", |n| n.as_str());
            let zone = zone_of_bone(bone);
            debug!("struck {:?} at {along:.1} m on {bone} ({zone:?})", avatar.src);
            hits.push(ModelHit {
                along,
                kind,
                claim: ShotClaim {
                    target,
                    zone,
                    point: point.to_array(),
                },
            });
        }
        hits.sort_by(|a, b| a.along.total_cmp(&b.along));
        hits
    }
}

/// Test the waiting shot against every enemy's model ([`ShotHits`]) — or, a
/// Ray Gun's, set its bolt off to be followed ([`track_own_projectiles`]).
pub(crate) fn claim_shot_hits(
    pending: Res<PendingShot>,
    weapon: Res<crate::Weapon>,
    time: Res<Time<Virtual>>,
    cam: Query<&GlobalTransform, With<WorldModelCamera>>,
    rapier: ReadRapierContext,
    models: Models,
    mut own: ResMut<OwnProjectiles>,
    mut out: ResMut<ShotHits>,
) {
    let Some(dir) = pending.0 else {
        out.0 = None;
        return;
    };
    let Ok(cam) = cam.single() else {
        return;
    };
    // (Exactly what `net::write_input` sends the server, right after this.)
    let origin = cam.translation();
    let aim = dir.normalize_or(Vec3::NEG_Z);
    if weapon.primary == WeaponId::RayGun {
        // Its bolt flies to the first surface in its way (or its max range).
        let max_range = WeaponId::RayGun.spec().max_range;
        let wall = rapier
            .single()
            .ok()
            .and_then(|r| r.cast_ray(origin, aim, max_range, true, QueryFilter::only_fixed()))
            .map(|(_, d)| d);
        let ground = ground_impact(origin, aim).map(|p| origin.distance(p));
        let reach = [wall, ground].into_iter().flatten().fold(max_range, f32::min);
        own.0.push(OwnProjectile {
            projectile: Projectile::RayGun,
            origin,
            fired_at: time.elapsed_secs(),
            path: Some(vec![(0.0, origin), (reach / RAYGUN_BOLT_SPEED, origin + aim * reach)]),
            tested: 0.0,
        });
        out.0 = None;
        return;
    }
    let path = shot_path(weapon.primary, origin, aim);
    let mut hits = models.hits_along(&path);
    hits.truncate(MAX_CLAIMS);
    out.0 = Some(hits.into_iter().map(|h| h.claim).collect());
}

/// Follow our Ray Gun bolts and thrown knives in flight: this frame's
/// stretch of each tested against the models as they are now, and the first
/// enemy it may hit that it strikes claimed. (A bolt's burst shows there at
/// once.)
#[allow(clippy::too_many_arguments)]
fn track_own_projectiles(
    time: Res<Time<Virtual>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    models: Models,
    mut flights: Query<&mut MessageReceiver<shared::KnifeFlight>>,
    mut sender: Query<&mut TriggerSender<shared::ProjectileClaim>, With<GameClient>>,
    mut landed: EventWriter<crate::RayGunLanded>,
    mut own: ResMut<OwnProjectiles>,
) {
    // A knife's path, from the server.
    for mut rx in &mut flights {
        for flight in rx.receive() {
            let origin = Vec3::from_array(flight.origin);
            if let Some(knife) = own.0.iter_mut().find(|p| {
                p.projectile == Projectile::Knife && p.path.is_none() && p.origin.distance(origin) < 0.01
            }) {
                knife.path = Some(flight.path.iter().map(|p| (p[3], Vec3::new(p[0], p[1], p[2]))).collect());
            }
        }
    }
    let mode = crate::zombies_hud::my_lobby(&local, &lobbies).map(|l| l.mode);
    let now = time.elapsed_secs();
    own.0.retain_mut(|p| {
        let age = now - p.fired_at;
        let Some(path) = &p.path else {
            return age < KNIFE_PATH_WAIT_SECS;
        };
        let end = path.last().map_or(0.0, |(t, _)| *t);
        let to = age.min(end);
        let stretch = path_between(path, p.tested, to);
        p.tested = to;
        let hit = models
            .hits_along(&stretch)
            .into_iter()
            .find(|h| may_hit(p.projectile, mode, h.kind));
        if let Some(hit) = hit {
            if let Ok(mut s) = sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::ProjectileClaim {
                    projectile: p.projectile,
                    origin: p.origin.to_array(),
                    claim: Some(hit.claim),
                });
            }
            if p.projectile == Projectile::RayGun {
                landed.write(crate::RayGunLanded {
                    end: Vec3::from_array(hit.claim.point),
                    owner: crate::BoltOwner::Own,
                    claimed: true,
                });
            }
            return false;
        }
        if to < end {
            return true;
        }
        // A bolt at its surface, having struck no one: it bursts there now —
        // here, and (told so) on the server.
        if p.projectile == Projectile::RayGun {
            if let Ok(mut s) = sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::ProjectileClaim {
                    projectile: p.projectile,
                    origin: p.origin.to_array(),
                    claim: None,
                });
            }
            if let Some(&(_, at)) = path.last() {
                landed.write(crate::RayGunLanded {
                    end: at,
                    owner: crate::BoltOwner::Own,
                    claimed: true,
                });
            }
        }
        false
    });
}

/// Whether `projectile` can hurt a `kind` of enemy in `mode` — what the
/// server allows (a Ray Gun only zombies; a knife only the mode's enemies),
/// so one passes through anyone it can't, as it does there.
fn may_hit(projectile: Projectile, mode: Option<GameMode>, kind: Kind) -> bool {
    match (projectile, mode) {
        (Projectile::RayGun, _) => kind == Kind::Zombie,
        (Projectile::Knife, Some(GameMode::Zombies)) => kind == Kind::Zombie,
        (Projectile::Knife, Some(GameMode::FreeForAll)) => kind == Kind::Player,
        (Projectile::Knife, Some(GameMode::Freestyle)) => kind == Kind::Bot,
        (Projectile::Knife, None) => false,
    }
}

/// The stretch of a timed `path` between `from` and `to` seconds into it.
fn path_between(path: &[(f32, Vec3)], from: f32, to: f32) -> Vec<Vec3> {
    let at = |t: f32| -> Vec3 {
        let i = path.partition_point(|(pt, _)| *pt <= t);
        match (i.checked_sub(1).and_then(|j| path.get(j)), path.get(i)) {
            (Some(&(t0, p0)), Some(&(t1, p1))) => p0.lerp(p1, ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0)),
            (Some(&(_, p)), None) | (None, Some(&(_, p))) => p,
            (None, None) => Vec3::ZERO,
        }
    };
    if path.is_empty() || to <= from {
        return Vec::new();
    }
    let mut out = vec![at(from)];
    out.extend(path.iter().filter(|(t, _)| *t > from && *t < to).map(|(_, p)| *p));
    out.push(at(to));
    out
}

/// One straight piece of a path, and how far down the path it starts.
struct Leg {
    from: Vec3,
    dir: Vec3,
    len: f32,
    start: f32,
}

impl Leg {
    fn distance_to(&self, p: Vec3) -> f32 {
        let t = (p - self.from).dot(self.dir).clamp(0.0, self.len);
        p.distance(self.from + self.dir * t)
    }
}

fn path_legs(path: &[Vec3]) -> Vec<Leg> {
    let mut start = 0.0;
    path.windows(2)
        .filter_map(|w| {
            let d = w[1] - w[0];
            let len = d.length();
            (len > 1e-6).then(|| {
                let leg = Leg { from: w[0], dir: d / len, len, start };
                start += len;
                leg
            })
        })
        .collect()
}

/// A skinned mesh's triangles where they are right now: every vertex moved
/// by its bones' current transforms, as the GPU draws it.
struct SkinnedVerts<'a> {
    positions: Vec<Vec3>,
    joints: &'a [[u16; 4]],
    weights: &'a [[f32; 4]],
    indices: Vec<u32>,
    /// Each joint index's bone entity.
    bones: &'a [Entity],
}

impl<'a> SkinnedVerts<'a> {
    fn new(
        mesh: &'a Mesh,
        skin: &'a SkinnedMesh,
        inverse: &SkinnedMeshInverseBindposes,
        bones: &Query<&GlobalTransform>,
    ) -> Option<Self> {
        let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            return None;
        };
        let Some(VertexAttributeValues::Uint16x4(joints)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX) else {
            return None;
        };
        let Some(VertexAttributeValues::Float32x4(weights)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT) else {
            return None;
        };
        let indices: Vec<u32> = match mesh.indices() {
            Some(Indices::U16(i)) => i.iter().map(|&i| i as u32).collect(),
            Some(Indices::U32(i)) => i.clone(),
            None => (0..pos.len() as u32).collect(),
        };
        // Each joint's skinning matrix: where its bone is now × its inverse
        // bind pose (Bevy's own skinning; the mesh entity's transform plays
        // no part).
        let mats: Vec<Mat4> = skin
            .joints
            .iter()
            .zip(inverse.iter())
            .map(|(&bone, inv)| bones.get(bone).map_or(Mat4::IDENTITY, |g| g.compute_matrix() * *inv))
            .collect();
        let positions = pos
            .iter()
            .zip(joints.iter().zip(weights.iter()))
            .map(|(p, (j, w))| {
                let p = Vec3::from_array(*p);
                (0..4)
                    .filter(|&k| w[k] > 0.0)
                    .map(|k| mats.get(j[k] as usize).map_or(p, |m| m.transform_point3(p)) * w[k])
                    .sum()
            })
            .collect();
        Some(Self {
            positions,
            joints,
            weights,
            indices,
            bones: &skin.joints,
        })
    }

    /// The first triangle any of `legs` (near `centre`, within `radius`)
    /// passes through: how far down the path, where, and the bone that spot
    /// hangs off most.
    fn first_hit(&self, legs: &[Leg], centre: Vec3, radius: f32) -> Option<(f32, Vec3, Entity)> {
        let mut best: Option<(f32, Vec3, [u32; 3], Vec3)> = None;
        for leg in legs.iter().filter(|l| l.distance_to(centre) <= radius) {
            for tri in self.indices.chunks_exact(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| self.positions[i as usize]);
                let Some((t, u, v)) = ray_triangle(leg.from, leg.dir, a, b, c) else {
                    continue;
                };
                if t > leg.len {
                    continue;
                }
                let along = leg.start + t;
                if best.is_none_or(|(b, ..)| along < b) {
                    best = Some((along, leg.from + leg.dir * t, [tri[0], tri[1], tri[2]], Vec3::new(1.0 - u - v, u, v)));
                }
            }
            // (Legs come in order: one struck on this leg beats any later.)
            if best.is_some() {
                break;
            }
        }
        let (along, point, tri, bary) = best?;
        // The bone with the most say over that spot.
        let mut pull: HashMap<u16, f32> = HashMap::default();
        for (k, &i) in tri.iter().enumerate() {
            for n in 0..4 {
                *pull.entry(self.joints[i as usize][n]).or_default() += bary[k] * self.weights[i as usize][n];
            }
        }
        let joint = pull.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))?.0;
        Some((along, point, *self.bones.get(joint as usize)?))
    }
}

/// Möller–Trumbore: where the ray `origin + t·dir` (`t >= 0`) crosses the
/// triangle `a b c`, either face — `(t, u, v)`, `u`/`v` the barycentric
/// weights of `b` and `c`.
fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t >= 0.0).then_some((t, u, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_through_a_triangle_hits_it_and_one_beside_it_misses() {
        let (a, b, c) = (Vec3::new(-1.0, 0.0, -5.0), Vec3::new(1.0, 0.0, -5.0), Vec3::new(0.0, 2.0, -5.0));
        let (t, u, v) = ray_triangle(Vec3::new(0.0, 0.5, 0.0), Vec3::NEG_Z, a, b, c).unwrap();
        assert!((t - 5.0).abs() < 1e-5 && u >= 0.0 && v >= 0.0 && u + v <= 1.0);
        assert!(ray_triangle(Vec3::new(1.5, 0.5, 0.0), Vec3::NEG_Z, a, b, c).is_none());
        // Behind the shooter doesn't count.
        assert!(ray_triangle(Vec3::new(0.0, 0.5, -10.0), Vec3::NEG_Z, a, b, c).is_none());
    }

    #[test]
    fn a_stretch_of_a_timed_path_runs_between_the_two_times() {
        let path = [(0.0, Vec3::ZERO), (1.0, Vec3::X * 10.0), (2.0, Vec3::new(10.0, 0.0, 10.0))];
        // Inside one piece.
        assert_eq!(path_between(&path, 0.2, 0.5), vec![Vec3::X * 2.0, Vec3::X * 5.0]);
        // Across the corner, which it keeps.
        let s = path_between(&path, 0.5, 1.5);
        assert_eq!(s, vec![Vec3::X * 5.0, Vec3::X * 10.0, Vec3::new(10.0, 0.0, 5.0)]);
        // Nothing new yet.
        assert!(path_between(&path, 1.0, 1.0).is_empty());
    }

    #[test]
    fn a_projectile_only_strikes_who_it_could_hurt() {
        use Kind::*;
        assert!(may_hit(Projectile::RayGun, Some(GameMode::Zombies), Zombie));
        assert!(!may_hit(Projectile::RayGun, Some(GameMode::Zombies), Player));
        assert!(may_hit(Projectile::Knife, Some(GameMode::FreeForAll), Player));
        assert!(!may_hit(Projectile::Knife, Some(GameMode::Zombies), Player));
        assert!(may_hit(Projectile::Knife, Some(GameMode::Freestyle), Bot));
        assert!(!may_hit(Projectile::Knife, Some(GameMode::Freestyle), Player));
    }
}
