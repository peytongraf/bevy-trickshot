//! Frustum-culling bounds for the skinned characters (zombies, hellhounds,
//! the boss). Bevy culls a skinned mesh against the box around its rest
//! pose, which an animation (a swing, a fall) can leave — so instead each
//! one gets a box that holds the character in any pose: a cube around its
//! rest pose as wide every way as the character's longest side. Loose, but it
//! still keeps whatever's off screen — and outside each shadow-casting
//! light's view — from being drawn there, which `NoFrustumCulling` didn't.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::render::primitives::Aabb;
use bevy::render::view::NoFrustumCulling;

use crate::weapon_drops::{posed_points, root_from};

/// What [`CharacterBounds::fit`] needs, for a scene's `SceneInstanceReady`
/// observer.
#[derive(SystemParam)]
pub(crate) struct CharacterBounds<'w, 's> {
    parents: Query<'w, 's, &'static ChildOf>,
    transforms: Query<'w, 's, &'static Transform>,
    skinned: Query<'w, 's, (&'static Mesh3d, &'static SkinnedMesh)>,
    meshes: Res<'w, Assets<Mesh>>,
    bindposes: Res<'w, Assets<SkinnedMeshInverseBindposes>>,
    /// Each mesh's rest-pose box in its scene root's frame (the same for
    /// every copy of the scene) — so it's worked out once, not per spawn.
    rest: Local<'s, HashMap<AssetId<Mesh>, (Vec3, Vec3)>>,
}

impl CharacterBounds<'_, '_> {
    /// Give `entity` — a mesh in the scene under `root` — its any-pose box,
    /// if it's skinned (an unskinned one moves with its own transform, so
    /// Bevy's own box is right). One whose rest pose can't be read isn't
    /// culled at all.
    pub(crate) fn fit(&mut self, commands: &mut Commands, root: Entity, entity: Entity) {
        let Ok((mesh, skin)) = self.skinned.get(entity) else {
            return;
        };
        let rest = match self.rest.get(&mesh.id()) {
            Some(&rest) => rest,
            None => {
                let mut points = Vec::new();
                posed_points(
                    root,
                    entity,
                    &mesh.0,
                    Some(skin),
                    &self.meshes,
                    &self.bindposes,
                    &self.parents,
                    &self.transforms,
                    &mut points,
                );
                if points.is_empty() {
                    commands.entity(entity).insert(NoFrustumCulling);
                    return;
                }
                let lo = points.iter().fold(Vec3::MAX, |a, p| a.min(*p));
                let hi = points.iter().fold(Vec3::MIN, |a, p| a.max(*p));
                self.rest.insert(mesh.id(), (lo, hi));
                (lo, hi)
            }
        };
        // A cube around the rest pose's middle, reaching the character's
        // longest side out every way (lying down, arms out, ...)...
        let (lo, hi) = rest;
        let center = (lo + hi) * 0.5;
        let reach = Vec3::splat((hi - lo).max_element());
        // ...in the mesh entity's own frame, which its box is tested in.
        let mesh_from_root = root_from(entity, root, &self.parents, &self.transforms).inverse();
        let (mut min, mut max) = (Vec3::MAX, Vec3::MIN);
        for i in 0..8 {
            let corner = center + reach * Vec3::new(sign(i & 1), sign(i & 2), sign(i & 4));
            let p = mesh_from_root.transform_point3(corner);
            min = min.min(p);
            max = max.max(p);
        }
        commands.entity(entity).remove::<NoFrustumCulling>().insert(Aabb::from_min_max(min, max));
    }
}

fn sign(bit: usize) -> f32 {
    if bit == 0 { -1.0 } else { 1.0 }
}
