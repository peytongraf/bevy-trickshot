//! Seeing past bits of the map, like hiding a roof in Blender to work on
//! the floor under it — the view only: nothing here touches the layout, so
//! nothing of it is ever saved.
//!
//! * V over a piece of the map hides it (the whole Blender object — every
//!   material of it), Shift+V brings back the last one hidden, Alt+V brings
//!   them all back;
//! * C cuts away everything that sits wholly above a height (View → Cut
//!   away above), PageUp / PageDown raising / lowering it — to look straight
//!   down into a floor.
//!
//! A hidden piece is out of the way for the editor's clicks too: moving a
//! thing along the surface, dropping it to the ground or adding one where
//! the view looks all go through it to what's showing ([`ColliderDisabled`]
//! — the editor's offline, nothing walks on the map here). Everything's put
//! back as it was on the way out ([`unhide_all`]).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::primitives::Aabb;
use bevy_rapier3d::prelude::{ColliderDisabled, QueryFilter, ReadRapierContext};

use super::Editor;
use crate::MapGeometry;

/// How far (m) above the camera's focus a cut starts out.
pub(crate) const CUT_START_ABOVE_FOCUS: f32 = 2.5;
/// How far (m) PageUp / PageDown move the cut (Shift: a tenth of it).
pub(crate) const CUT_STEP: f32 = 0.5;

/// On a bit of the map the editor's hiding: how it was, to put back.
#[derive(Component)]
pub(super) struct EditorHidden {
    visibility: Visibility,
    had_collider_off: bool,
}

/// What it takes to find the piece of map under the cursor.
#[derive(SystemParam)]
pub(super) struct MapPieces<'w, 's> {
    parents: Query<'w, 's, &'static ChildOf>,
    names: Query<'w, 's, &'static Name>,
    roots: Query<'w, 's, (), With<MapGeometry>>,
}

impl MapPieces<'_, '_> {
    /// The name of the map piece (its glTF node — the Blender object) along
    /// `ray`, if there's one showing.
    pub(super) fn under(&self, rapier: &ReadRapierContext, ray: Ray3d) -> Option<String> {
        let ctx = rapier.single().ok()?;
        let (hit, _) = ctx.cast_ray(ray.origin, *ray.direction, 2_000.0, true, QueryFilter::default())?;
        // Only the map's own meshes: an ancestor's one of its scenes.
        if !self.parents.iter_ancestors(hit).any(|a| self.roots.contains(a)) {
            return None;
        }
        // The mesh is one material of its node (the object).
        let node = self.parents.get(hit).ok()?.parent();
        self.names.get(node).ok().map(|n| n.as_str().to_string())
    }
}

/// Hide the map's pieces the view hides ([`super::ViewOptions::hidden`],
/// [`super::ViewOptions::cut`]) and show the rest as they were.
#[allow(clippy::type_complexity)]
pub(super) fn sync_map_hiding(
    editor: Res<Editor>,
    roots: Query<Entity, With<MapGeometry>>,
    children: Query<&Children>,
    meshes: Query<
        (&ChildOf, &GlobalTransform, Option<&Aabb>, &Visibility, Option<&EditorHidden>, Has<ColliderDisabled>),
        With<Mesh3d>,
    >,
    names: Query<&Name>,
    mut commands: Commands,
) {
    let view = &editor.view;
    for root in &roots {
        for e in children.iter_descendants(root) {
            let Ok((parent, global, aabb, visibility, hidden, collider_off)) = meshes.get(e) else {
                continue;
            };
            let picked = !view.hidden.is_empty()
                && names
                    .get(parent.parent())
                    .is_ok_and(|n| view.hidden.iter().any(|h| h == n.as_str()));
            let above_cut = view.cut.zip(aabb).is_some_and(|(cut, aabb)| bottom(global, aabb) > cut);
            match (picked || above_cut, hidden) {
                (true, None) => {
                    commands.entity(e).insert((
                        EditorHidden {
                            visibility: *visibility,
                            had_collider_off: collider_off,
                        },
                        Visibility::Hidden,
                        ColliderDisabled,
                    ));
                }
                (false, Some(was)) => show(&mut commands, e, was),
                _ => {}
            }
        }
    }
}

/// Everything hidden shown again — on the way out of the editor.
pub(super) fn unhide_all(hidden: Query<(Entity, &EditorHidden)>, mut commands: Commands) {
    for (e, was) in &hidden {
        show(&mut commands, e, was);
    }
}

fn show(commands: &mut Commands, e: Entity, was: &EditorHidden) {
    let mut entity = commands.entity(e);
    entity.insert(was.visibility).remove::<EditorHidden>();
    if !was.had_collider_off {
        entity.remove::<ColliderDisabled>();
    }
}

/// The lowest point (world height) of a mesh's bounds.
fn bottom(global: &GlobalTransform, aabb: &Aabb) -> f32 {
    let affine = global.affine();
    let center = affine.transform_point3a(aabb.center);
    let m = affine.matrix3;
    // How far the box reaches up / down, turned and scaled into the world.
    let reach = Vec3::new(m.x_axis.y, m.y_axis.y, m.z_axis.y).abs().dot(Vec3::from(aabb.half_extents));
    center.y - reach
}
