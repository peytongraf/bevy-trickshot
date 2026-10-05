//! Dropped weapons (`shared::WeaponDrop`, from `server::wall_buys`): a
//! `Zombies` player who buys a wall gun or picks a weapon up drops the one
//! in their hands. Each lies on its side where it fell — just the weapon, no
//! arms: its view model's scene with only the gun's own meshes left showing
//! (the ones that wear the Pack-a-Punch camo, and the sniper's loose bits),
//! fitted to a real-world size by its own shape ([`fit_model`]), in its
//! level's camo (`pap`) and outlined in its level's colour
//! ([`outline_color`], the outline being `knife_pickup`'s). Picking one up
//! is `knife_pickup`'s card and interact key.
//!
//! The visuals follow the replicated drops and are `StateScoped(InGame)` —
//! nothing to reset between games.

use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::render::mesh::VertexAttributeValues;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;
use shared::weapon::{SlotWeapon, WeaponId};
use shared::WeaponDrop;

use crate::knife_pickup::{OutlineTint, OutlineWhenLoaded, OUTLINE_BLUE};
use crate::AppState;

pub(crate) struct WeaponDropsPlugin;

impl Plugin for WeaponDropsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(fit_dropped_model)
            .add_systems(Update, sync_drop_visuals.run_if(in_state(AppState::InGame)));
    }
}

/// The outline round a dropped weapon packed to `level`: blue unpacked (like
/// a throwing knife's), then each level's camo colour.
pub(crate) fn outline_color(level: u8) -> Color {
    match level {
        0 => OUTLINE_BLUE,
        1 => Color::srgb(1.0, 0.55, 0.1),
        2 => Color::srgb(1.0, 0.15, 0.55),
        _ => Color::srgb(0.65, 0.35, 1.0),
    }
}

/// The view model scene a weapon's drawn from.
pub(crate) fn model_path(weapon: SlotWeapon) -> &'static str {
    match weapon {
        SlotWeapon::Gun(WeaponId::Ak74) => "models/weapons/ak_74.glb",
        SlotWeapon::Gun(WeaponId::RayGun) => crate::weapons::RAYGUN_MODEL,
        SlotWeapon::Gun(_) => "models/weapons/sniper.glb",
        SlotWeapon::Knife => "models/weapons/knife.glb",
    }
}

/// How long (m) a weapon lying on the ground is, end to end.
pub(crate) fn real_length(weapon: SlotWeapon) -> f32 {
    match weapon {
        SlotWeapon::Gun(WeaponId::Ak74) => 0.95,
        SlotWeapon::Gun(WeaponId::RayGun) => 0.45,
        SlotWeapon::Gun(_) => 1.25,
        SlotWeapon::Knife => 0.32,
    }
}

/// A dropped weapon's visual: on the ground where it lies, turned its way.
#[derive(Component)]
struct DropVisual {
    /// The replicated [`WeaponDrop`].
    src: Entity,
}

/// The weapon's scene, under its [`DropVisual`] — laid flat and sized by
/// [`fit_dropped_model`] once it's in. `pap` follows the drop's level (its
/// camo, `pap::tag_camo_parts`).
#[derive(Component)]
pub(crate) struct DropModel {
    pub(crate) weapon: SlotWeapon,
    pub(crate) pap: u8,
    /// Stood up as it's held — sights up, barrel along +X, its middle at
    /// the origin ([`fit_upright`]) — instead of laid on its side (the
    /// Mystery Box's, `mystery_box`).
    pub(crate) upright: bool,
}

/// A visual for every drop, and none for one that's gone.
fn sync_drop_visuals(
    drops: Query<(Entity, &WeaponDrop)>,
    visuals: Query<(Entity, &DropVisual)>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    for (e, visual) in &visuals {
        if !drops.contains(visual.src) {
            commands.entity(e).despawn();
        }
    }
    for (src, drop) in &drops {
        if visuals.iter().any(|(_, v)| v.src == src) {
            continue;
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                DropVisual { src },
                Transform::from_translation(drop.pos).with_rotation(Quat::from_rotation_y(drop.yaw)),
                Visibility::default(),
            ))
            .with_child((
                DropModel {
                    weapon: drop.weapon,
                    pap: drop.pap,
                    upright: false,
                },
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(model_path(drop.weapon)))),
                // (Hidden until it's laid down — see `fit_dropped_model`.)
                Visibility::Hidden,
                OutlineWhenLoaded,
                OutlineTint(outline_color(drop.pap)),
            ));
    }
}

/// `entity`'s transform from `root`'s frame (the product of the local
/// transforms on the way down).
pub(crate) fn root_from(
    entity: Entity,
    root: Entity,
    parents: &Query<&ChildOf>,
    transforms: &Query<&Transform>,
) -> Affine3A {
    let mut m = Affine3A::IDENTITY;
    let mut at = entity;
    while at != root {
        if let Ok(t) = transforms.get(at) {
            m = t.compute_affine() * m;
        }
        let Ok(parent) = parents.get(at) else { break };
        at = parent.parent();
    }
    m
}

/// The weapon's main directions through `points`: the middle, then the
/// axes its points spread along most (the barrel), next (its height) and
/// least (its thickness) — each with the points' extent along it.
fn principal_axes(points: &[Vec3]) -> (Vec3, [(Vec3, f32, f32); 3]) {
    let n = points.len().max(1) as f32;
    let mean = points.iter().copied().sum::<Vec3>() / n;
    let mut cov = Mat3::ZERO;
    for p in points {
        let d = *p - mean;
        cov += Mat3::from_cols(d * d.x, d * d.y, d * d.z);
    }
    cov *= 1.0 / n;
    let power = |m: Mat3, mut v: Vec3| {
        for _ in 0..64 {
            let w = m * v;
            if w.length_squared() < 1e-20 {
                break;
            }
            v = w.normalize();
        }
        v
    };
    let e1 = power(cov, Vec3::new(0.58, 0.57, 0.59));
    let l1 = e1.dot(cov * e1);
    let deflated = cov - Mat3::from_cols(e1 * e1.x, e1 * e1.y, e1 * e1.z) * l1;
    let e2 = power(deflated, e1.any_orthonormal_vector());
    let e2 = (e2 - e1 * e1.dot(e2)).normalize_or(e1.any_orthonormal_vector());
    let e3 = e1.cross(e2).normalize();
    let mut center = mean;
    let axes = [e1, e2, e3].map(|axis| {
        let (lo, hi) = points
            .iter()
            .map(|p| (*p - mean).dot(axis))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), d| (lo.min(d), hi.max(d)));
        center += axis * (lo + hi) * 0.5;
        (axis, lo, hi)
    });
    (center, axes)
}

/// Lay a scene shaped like `points` (in its own frame) on its side,
/// `length` long: its longest direction along the ground (+X), its thinnest
/// straight up, resting on the ground at the origin.
fn fit_model(points: &[Vec3], length: f32) -> Option<Transform> {
    if points.len() < 3 {
        return None;
    }
    let (center, [(barrel, lo1, hi1), _, (thin, lo3, hi3)]) = principal_axes(points);
    let long = hi1 - lo1;
    if long <= 1e-6 {
        return None;
    }
    let scale = length / long;
    let turn = Quat::from_mat3(&Mat3::from_cols(barrel, thin, barrel.cross(thin)).transpose());
    let lift = Vec3::Y * (hi3 - lo3) * 0.5 * scale;
    Some(Transform {
        translation: lift - turn * (center * scale),
        rotation: turn,
        scale: Vec3::splat(scale),
    })
}

/// Stand a scene shaped like `points` (in its own frame, upright as made —
/// a view model's, sights up) `length` long: its longer way across the
/// ground along +X, its middle at the origin.
pub(crate) fn fit_upright(points: &[Vec3], length: f32) -> Option<Transform> {
    if points.len() < 3 {
        return None;
    }
    let lo = points.iter().copied().reduce(Vec3::min)?;
    let hi = points.iter().copied().reduce(Vec3::max)?;
    let size = hi - lo;
    let long = size.x.max(size.z);
    if long <= 1e-6 {
        return None;
    }
    let scale = length / long;
    // (Barrel along Z as made: turned a quarter so it runs along X.)
    let turn = if size.z > size.x {
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    };
    let center = (lo + hi) * 0.5;
    Some(Transform {
        translation: -(turn * (center * scale)),
        rotation: turn,
        scale: Vec3::splat(scale),
    })
}

/// A dropped weapon's scene is in: hide everything but the weapon (the
/// arms), and lay it down by its shape — the weapon's vertices, posed as
/// the scene stands (the AK-74's skinned to its rig), give its size and
/// directions.
#[allow(clippy::too_many_arguments)]
fn fit_dropped_model(
    trigger: Trigger<SceneInstanceReady>,
    models: Query<&DropModel>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    mesh_entities: Query<(&Mesh3d, Option<&SkinnedMesh>)>,
    meshes: Res<Assets<Mesh>>,
    bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
    mut commands: Commands,
) {
    let root = trigger.target();
    let Ok(model) = models.get(root) else { return };
    let parts = crate::pap::camo_parts(model.weapon);
    let named_part = |e: Entity| names.get(e).is_ok_and(|n| crate::pap::is_camo_part(parts, n.as_str()));
    let mut points = Vec::new();
    for entity in children.iter_descendants(root) {
        let Ok((mesh3d, skinned)) = mesh_entities.get(entity) else { continue };
        // The weapon: its camo parts, and (the sniper's bolt, trigger and
        // such) anything not skinned to the arm rig.
        let part = named_part(entity) || parents.get(entity).is_ok_and(|p| named_part(p.parent()));
        if !part && skinned.is_some() {
            commands.entity(entity).insert(Visibility::Hidden);
            continue;
        }
        let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
        let Some(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).and_then(|a| a.as_float3()) else {
            continue;
        };
        let step = (positions.len() / 4000).max(1);
        match skinned {
            Some(skin) => {
                commands.entity(entity).insert(NoFrustumCulling);
                let (Some(VertexAttributeValues::Uint16x4(joints)), Some(VertexAttributeValues::Float32x4(weights))) = (
                    mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX),
                    mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT),
                ) else {
                    continue;
                };
                let Some(inverse) = bindposes.get(&skin.inverse_bindposes) else { continue };
                let mats: Vec<Affine3A> = skin
                    .joints
                    .iter()
                    .zip(inverse.iter())
                    .map(|(&joint, inv)| root_from(joint, root, &parents, &transforms) * Affine3A::from_mat4(*inv))
                    .collect();
                for i in (0..positions.len()).step_by(step) {
                    let v = Vec3::from_array(positions[i]);
                    let posed: Vec3 = (0..4)
                        .filter_map(|k| {
                            let m = mats.get(joints[i][k] as usize)?;
                            Some(m.transform_point3(v) * weights[i][k])
                        })
                        .sum();
                    points.push(posed);
                }
            }
            None => {
                let m = root_from(entity, root, &parents, &transforms);
                points.extend((0..positions.len()).step_by(step).map(|i| m.transform_point3(Vec3::from_array(positions[i]))));
            }
        }
    }
    let fit = if model.upright {
        fit_upright(&points, real_length(model.weapon))
    } else {
        fit_model(&points, real_length(model.weapon))
    };
    match fit {
        Some(fit) => {
            commands.entity(root).insert((fit, Visibility::Inherited));
        }
        None => warn!("dropped {}: no weapon meshes to fit", model.weapon.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_thin_shape_lies_along_the_ground_at_its_length() {
        // A 2 × 0.4 × 0.1 box standing on its end, tipped about.
        let tilt = Quat::from_rotation_z(1.2) * Quat::from_rotation_x(0.3);
        let mut points = Vec::new();
        for x in [-1.0, 1.0] {
            for y in [-0.2, 0.2] {
                for z in [-0.05, 0.05] {
                    points.push(tilt * Vec3::new(x, y, z) + Vec3::new(3.0, 1.0, -2.0));
                }
            }
        }
        let fit = fit_model(&points, 1.0).unwrap();
        let placed: Vec<Vec3> = points.iter().map(|p| fit.transform_point(*p)).collect();
        let lo = placed.iter().copied().reduce(Vec3::min).unwrap();
        let hi = placed.iter().copied().reduce(Vec3::max).unwrap();
        // 1 m long along X, thinnest (0.05 m after scaling) up, on the ground.
        assert!((hi.x - lo.x - 1.0).abs() < 1e-3, "{lo} {hi}");
        assert!((hi.y - lo.y - 0.05).abs() < 1e-3, "{lo} {hi}");
        assert!(lo.y.abs() < 1e-3 && ((lo.x + hi.x) * 0.5).abs() < 1e-3);
    }
}
