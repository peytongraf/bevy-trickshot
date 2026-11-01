//! Even textures across a map: a tiling material (Der Tempel's sandstone
//! walls, say) laid on every object that uses it at one real-world size,
//! rather than however each object happened to be unwrapped — a cube
//! stretched into a long wall keeps its 0..1-per-face UVs, so its blocks
//! come out huge, and a small one's tiny, side by side.
//!
//! Once the map's in, each mesh using one of [`MapUvSettings`]' materials
//! gets its UVs projected from the map itself, box-style: each face from
//! the side it faces most (left / right, up / down, front / back), one
//! repeat every so many metres — and the material's own texture transform
//! (which only scaled the old UVs) dropped. Only the UVs change: the
//! vertices, and so the map's collision, stay exactly as they were. A
//! curved mesh (a pillar) keeps its own unwrap — box projection would seam
//! it.
//!
//! The original UVs and transforms are kept ([`Originals`]), so turning a
//! material off (the debug panel's "Map textures" section) puts it back.
//! Re-run on a settings change; the projection only ever reads positions,
//! so re-running is harmless. Nothing to reset between games: it's the
//! map's own assets, worked out afresh whenever the map's loaded.

use bevy::ecs::system::SystemParam;
use bevy::gltf::GltfMaterialName;
use bevy::math::{Affine2, Affine3A};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use bevy_egui::egui;

use crate::environment::{CurrentMap, MapLoadState, MapModel};

/// One material laid on evenly.
#[derive(Clone, Debug)]
pub(crate) struct MapUvEntry {
    /// Its name in the map's glb.
    pub(crate) material: &'static str,
    pub(crate) on: bool,
    /// Metres per repeat of its texture.
    pub(crate) metres: f32,
}

/// Tunables — the debug panel's "Map textures" section.
#[derive(Resource, Clone, Debug)]
pub(crate) struct MapUvSettings {
    pub(crate) enabled: bool,
    /// Der Tempel's tiling materials (the rest are emblems and panels laid
    /// out by hand, left alone).
    pub(crate) der_tempel: Vec<MapUvEntry>,
    /// A mesh counts as flat-sided (box-projected) if at least this much of
    /// it faces straight along an axis.
    pub(crate) flat_share: f32,
}

impl Default for MapUvSettings {
    fn default() -> Self {
        let entry = |material, on, metres| MapUvEntry { material, on, metres };
        Self {
            enabled: true,
            der_tempel: vec![
                entry("sandstone_blocks_05", true, 4.0),
                entry("patterned_brick_wall_03", true, 3.2),
                entry("stone_tiles_02", true, 5.0),
                entry("marble_01", true, 5.0),
                entry("mixed_rock_tiles", false, 4.0),
                entry("dark_rock.001", false, 4.0),
                entry("monastery_stone_floor", false, 4.0),
                entry("dar", false, 3.0),
                entry("roof_tiles", false, 3.0),
                entry("wood_planks", false, 2.0),
                entry("park_sand", false, 8.0),
                entry("floor_tiles_06", false, 4.0),
                entry("square_floor", false, 16.0),
            ],
            flat_share: 0.9,
        }
    }
}

impl MapUvSettings {
    /// `material`'s metres per repeat on `map`, if it's laid on evenly.
    fn metres(&self, map: shared::MapId, material: &str) -> Option<f32> {
        if !self.enabled || !matches!(map, shared::MapId::DerTempel | shared::MapId::DerTempelDay) {
            return None;
        }
        self.der_tempel
            .iter()
            .find(|e| e.on && e.material == material)
            .map(|e| e.metres.max(0.05))
    }
}

/// What the projection replaced, to put back.
#[derive(Resource, Default)]
struct Originals {
    meshes: HashMap<AssetId<Mesh>, (VertexAttributeValues, Option<VertexAttributeValues>)>,
    materials: HashMap<AssetId<StandardMaterial>, Affine2>,
}

pub(crate) struct MapUvPlugin;

impl Plugin for MapUvPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapUvSettings>()
            .init_resource::<Originals>()
            .add_systems(Update, apply_map_uvs);
    }
}

/// Once the map's in (and again whenever the settings change), lay each
/// listed material on evenly — or put back the ones that aren't.
#[allow(clippy::too_many_arguments)]
fn apply_map_uvs(
    settings: Res<MapUvSettings>,
    current: Res<CurrentMap>,
    load: Res<MapLoadState>,
    roots: Query<Entity, With<MapModel>>,
    children: Query<&Children>,
    parts: Query<(&Mesh3d, &GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
    (transforms, parents): (Query<&Transform>, Query<&ChildOf>),
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut originals: ResMut<Originals>,
    mut done: Local<Option<Entity>>,
) {
    let Ok(root) = roots.single() else { return };
    if !load.model_ready() || (*done == Some(root) && !settings.is_changed()) {
        return;
    }
    *done = Some(root);
    let mut laid = 0;
    for e in children.iter_descendants(root) {
        let Ok((mesh3d, name, material)) = parts.get(e) else { continue };
        let metres = settings.metres(current.0, &name.0);
        // Where it is in the world: its transforms up to (and with) the map's.
        let mut to_world = Affine3A::IDENTITY;
        let mut at = e;
        loop {
            if let Ok(t) = transforms.get(at) {
                to_world = t.compute_affine() * to_world;
            }
            if at == root {
                break;
            }
            let Ok(parent) = parents.get(at) else { break };
            at = parent.parent();
        }
        let Some(mesh) = meshes.get_mut(&mesh3d.0) else { continue };
        let Some(metres) = metres else {
            // Not (or no longer) laid on evenly: as it came, if it was.
            if let Some((uvs, tangents)) = originals.meshes.remove(&mesh3d.0.id()) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
                if let Some(tangents) = tangents {
                    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
                }
            }
            if let Some(transform) = originals.materials.remove(&material.0.id()) {
                if let Some(m) = materials.get_mut(&material.0) {
                    m.uv_transform = transform;
                }
            }
            continue;
        };
        let Some(uvs) = project(mesh, to_world, metres, settings.flat_share) else { continue };
        if !originals.meshes.contains_key(&mesh3d.0.id()) {
            let (Some(old_uvs), old_tangents) =
                (mesh.attribute(Mesh::ATTRIBUTE_UV_0).cloned(), mesh.attribute(Mesh::ATTRIBUTE_TANGENT).cloned())
            else {
                continue;
            };
            originals.meshes.insert(mesh3d.0.id(), (old_uvs, old_tangents));
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        // (Its normal map follows the new UVs.)
        if mesh.attribute(Mesh::ATTRIBUTE_TANGENT).is_some() {
            if let Err(err) = mesh.generate_tangents() {
                warn!("map textures: couldn't redo {}'s tangents: {err}", name.0);
            }
        }
        if let Some(m) = materials.get_mut(&material.0) {
            originals.materials.entry(material.0.id()).or_insert(m.uv_transform);
            m.uv_transform = Affine2::IDENTITY;
        }
        laid += 1;
    }
    if laid > 0 {
        info!("map textures: laid {laid} mesh(es) on evenly");
    }
}

/// `mesh`'s UVs projected from the world (it's placed by `to_world`): each
/// vertex from the axis its face is turned to most, `metres` a repeat.
/// `None` for a curved one (less than `flat_share` of it facing along an
/// axis), left as it is.
fn project(mesh: &Mesh, to_world: Affine3A, metres: f32, flat_share: f32) -> Option<Vec<[f32; 2]>> {
    let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return None;
    };
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
        return None;
    };
    // (Normals turn with the inverse transpose — a wall's a stretched cube.)
    let normal_matrix = to_world.matrix3.inverse().transpose();
    let mut flat = 0;
    let uvs: Vec<[f32; 2]> = positions
        .iter()
        .zip(normals)
        .map(|(p, n)| {
            let p = to_world.transform_point3a(Vec3::from_array(*p).into());
            let n = (normal_matrix * Vec3A::from_array(*n)).normalize_or_zero();
            let a = n.abs();
            if a.max_element() > 0.97 {
                flat += 1;
            }
            let (u, v) = if a.y >= a.x && a.y >= a.z {
                (p.x, if n.y >= 0.0 { p.z } else { -p.z })
            } else if a.x >= a.z {
                (if n.x >= 0.0 { -p.z } else { p.z }, -p.y)
            } else {
                (if n.z >= 0.0 { p.x } else { -p.x }, -p.y)
            };
            [u / metres, v / metres]
        })
        .collect();
    (flat as f32 >= flat_share * positions.len() as f32).then_some(uvs)
}

/// The debug panel's "Map textures" section.
#[derive(SystemParam)]
pub(crate) struct MapUvDebug<'w> {
    settings: ResMut<'w, MapUvSettings>,
}

impl MapUvDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Der Tempel's tiling materials, laid on at one size everywhere (metres per repeat) \
             instead of each object's own UVs. Off: as the map was made.",
        );
        let s = &mut *self.settings;
        ui.checkbox(&mut s.enabled, "On");
        for e in &mut s.der_tempel {
            ui.horizontal(|ui| {
                ui.checkbox(&mut e.on, e.material);
                ui.add_enabled(e.on, egui::Slider::new(&mut e.metres, 0.25f32..=20.0).text("m"));
            });
        }
        ui.add(egui::Slider::new(&mut s.flat_share, 0.0f32..=1.0).text("flat-sided if facing an axis (share)"));
        ui.horizontal(|ui| {
            if ui.button("Reset map textures").clicked() {
                *s = default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("{s:#?}");
            }
        });
    }
}
