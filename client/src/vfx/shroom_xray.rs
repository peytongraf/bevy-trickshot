//! Shroom "x-ray": while the shroom effect is on, enemy bots hidden behind
//! walls show through them as a bright, hazy, smoke-edged yellow ghost of
//! their model (never over them while they're in plain sight). The classic
//! Death Perception perk uses the same twins with a second material: a clean
//! orange outline instead of the ghost.
//!
//! How: each enemy avatar's meshes get a twin — same mesh, same
//! `SkinnedMesh` (so it follows the very same bones and animates in lockstep)
//! — drawn with [`ShroomXrayMaterial`] (`assets/shaders/shroom_xray.wgsl`).
//! Its pipeline flips the depth test to pass only where something is in
//! *front* of it and writes no depth, blending additively: so it appears
//! exactly where the enemy is hidden. "In front" means in front of the enemy's
//! whole body — a sphere `ShroomSettings::xray_body_radius` round its middle,
//! passed per enemy in its own material ([`XrayParams::anchor`]): the vertex
//! shader pulls the twin toward the camera along each view ray to that
//! sphere's near side (same spot on screen, only its depth moves), so none of
//! the enemy's own parts — an arm behind its back, its gun, its legs — can
//! ever count as cover; only real cover does. (A twin rather than a second material
//! on the same entity: Bevy 0.16 keeps one material per entity.)
//!
//! "Enemy bots" = `Zombies` zombies (their own model, [`crate::ZombieVisual`])
//! and soldier avatars with [`crate::BotLook`] (a `FreeForAll` bot player, a
//! `Freestyle` target), live ones only — not kill-cam stand-ins. Strength follows the shroom effect's own fade
//! ([`ShroomLevel`]); the twins are hidden entirely while it's off. They're
//! children of the avatar, so they go with it — nothing to reset per game.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderRef,
    SpecializedMeshPipelineError,
};
use bevy::render::view::NoFrustumCulling;

use super::shroom::{ShroomLevel, ShroomSettings};

const SHADER_ASSET_PATH: &str = "shaders/shroom_xray.wgsl";

use params::XrayParams;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct (same as `shroom`'s
// uniform).
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Mirrors `XrayParams` in the shader.
    #[derive(Clone, Copy, Default, PartialEq, ShaderType)]
    pub(crate) struct XrayParams {
        /// rgb = colour, a = brightness multiplier.
        pub(crate) color: Vec4,
        /// xyz = the middle of this enemy's body, w = how far (m) its own
        /// parts reach from there: only something nearer the camera than
        /// that whole sphere counts as hiding it.
        pub(crate) anchor: Vec4,
        pub(crate) strength: f32,
        pub(crate) inflate: f32,
        pub(crate) fill: f32,
        pub(crate) edge_softness: f32,
        pub(crate) smoke_scale: f32,
        pub(crate) smoke_speed: f32,
        pub(crate) smoke_amount: f32,
        pub(crate) shimmer: f32,
        pub(crate) min_gap: f32,
        /// `0` = the shroom's ghost, `1` = an outline only.
        pub(crate) outline: f32,
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct ShroomXrayMaterial {
    #[uniform(0)]
    params: XrayParams,
}

impl Material for ShroomXrayMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    /// Only draw where the ghost is *behind* something already drawn — reverse-Z,
    /// so "farther than what's there" is `Less` — and leave the depth buffer alone.
    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_compare = CompareFunction::Less;
            depth.depth_write_enabled = false;
        }
        Ok(())
    }
}

/// On an enemy avatar once its meshes have their twins: the twins' own
/// material — one per enemy, since it carries where that enemy is
/// ([`XrayParams::anchor`]).
#[derive(Component)]
struct XrayTwinned(Handle<ShroomXrayMaterial>);

/// A twin mesh drawing the ghost.
#[derive(Component)]
struct XrayTwin;

pub(crate) struct ShroomXrayPlugin;

impl Plugin for ShroomXrayPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ShroomXrayMaterial> {
            // No depth prepass / shadows: the ghost must never write depth or
            // cast anything.
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .add_systems(Update, (twin_enemy_meshes, update_xray).chain());
    }
}

/// The shroom's ghost look (`anchor` is filled in per enemy).
fn params(settings: &ShroomSettings, level: f32) -> XrayParams {
    let [r, g, b] = settings.xray_color;
    let lin = Color::srgb(r, g, b).to_linear();
    XrayParams {
        color: Vec4::new(lin.red, lin.green, lin.blue, settings.xray_brightness),
        anchor: Vec4::ZERO,
        strength: settings.xray_opacity * level,
        inflate: settings.xray_inflate,
        fill: settings.xray_fill,
        edge_softness: settings.xray_edge_softness,
        smoke_scale: settings.xray_smoke_scale,
        smoke_speed: settings.xray_smoke_speed,
        smoke_amount: settings.xray_smoke_amount,
        shimmer: settings.xray_shimmer,
        min_gap: settings.xray_min_gap,
        outline: 0.0,
    }
}

/// Death Perception's outline (the classic perk, `ClassicPerks`): the same
/// see-through-walls twin, drawn as an orange rim instead of a ghost.
fn outline_params(settings: &ShroomSettings, classic: &crate::zombies_hud::ClassicPerks) -> XrayParams {
    let [r, g, b] = classic.death_perception_color;
    let lin = Color::srgb(r, g, b).to_linear();
    XrayParams {
        color: Vec4::new(lin.red, lin.green, lin.blue, classic.death_perception_brightness),
        anchor: Vec4::ZERO,
        strength: 1.0,
        inflate: classic.death_perception_inflate,
        fill: 0.0,
        edge_softness: classic.death_perception_sharpness,
        smoke_scale: 1.0,
        smoke_speed: 0.0,
        smoke_amount: 0.0,
        shimmer: 0.0,
        min_gap: settings.xray_min_gap,
        outline: 1.0,
    }
}

/// Give every live enemy avatar whose scene has loaded a ghost twin per mesh,
/// all sharing a material of the enemy's own.
#[allow(clippy::type_complexity)]
fn twin_enemy_meshes(
    mut commands: Commands,
    settings: Res<ShroomSettings>,
    level: Res<ShroomLevel>,
    mut materials: ResMut<Assets<ShroomXrayMaterial>>,
    // (Once the scene's in — the animation player is found then.)
    avatars: Query<
        Entity,
        (
            Or<(
                (With<crate::BotLook>, With<crate::SoldierAnimationPlayer>),
                With<crate::ZombieAnimationPlayer>,
            )>,
            Without<XrayTwinned>,
            Without<crate::killcam::KillCamPlayerGhost>,
        ),
    >,
    children: Query<&Children>,
    meshes: Query<(&Mesh3d, Option<&SkinnedMesh>), Without<XrayTwin>>,
) {
    for root in &avatars {
        let material = materials.add(ShroomXrayMaterial {
            params: params(&settings, level.0),
        });
        for entity in children.iter_descendants(root) {
            let Ok((mesh, skin)) = meshes.get(entity) else {
                continue;
            };
            // A child of the mesh it copies: a skinned twin follows the
            // shared bones; an unskinned one inherits the mesh's transform.
            let mut twin = commands.spawn((
                XrayTwin,
                Mesh3d(mesh.0.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                NoFrustumCulling,
                NotShadowCaster,
                ChildOf(entity),
            ));
            if let Some(skin) = skin {
                twin.insert(skin.clone());
            }
        }
        commands.entity(root).insert(XrayTwinned(material));
    }
}

/// Follow the shroom effect's fade and the panel's look — or, with Death
/// Perception, its outline — and where each enemy is; hide the twins outright
/// while neither is on.
fn update_xray(
    settings: Res<ShroomSettings>,
    level: Res<ShroomLevel>,
    classic: Res<crate::zombies_hud::ClassicPerks>,
    mut materials: ResMut<Assets<ShroomXrayMaterial>>,
    avatars: Query<(&GlobalTransform, &XrayTwinned)>,
    mut twins: Query<&mut Visibility, With<XrayTwin>>,
) {
    let outline = classic.has(shared::perks::Perk::DeathPerception);
    let on = outline || level.0 > 1e-3;
    for mut vis in &mut twins {
        vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
    if !on {
        return;
    }
    let base = if outline {
        outline_params(&settings, &classic)
    } else {
        params(&settings, level.0)
    };
    for (gt, twinned) in &avatars {
        // The middle of the enemy's body (the avatar's root is at its feet),
        // and how far its own parts reach from there.
        let middle = gt.translation() + Vec3::Y * settings.xray_body_height;
        let want = XrayParams {
            anchor: middle.extend(settings.xray_body_radius),
            ..base
        };
        // (`get` first so an unchanged material isn't re-uploaded.)
        if materials.get(&twinned.0).is_some_and(|m| m.params != want) {
            if let Some(m) = materials.get_mut(&twinned.0) {
                m.params = want;
            }
        }
    }
}
