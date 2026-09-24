//! Shroom "x-ray": while the shroom effect is on, enemy bots hidden behind
//! walls show through them as a bright, hazy, smoke-edged yellow ghost of
//! their model (never over them while they're in plain sight).
//!
//! How: each enemy avatar's meshes get a twin — same mesh, same
//! `SkinnedMesh` (so it follows the very same bones and animates in lockstep)
//! — drawn with [`ShroomXrayMaterial`] (`assets/shaders/shroom_xray.wgsl`).
//! Its pipeline flips the depth test to pass only where something is in
//! *front* of it and writes no depth, blending additively: so it appears
//! exactly where the enemy is hidden. (A twin rather than a second material
//! on the same entity: Bevy 0.16 keeps one material per entity.)
//!
//! "Enemy bots" = avatars with [`crate::BotLook`] (a `Zombies` zombie, a
//! `FreeForAll` bot player, a `Freestyle` target), live ones only — not
//! kill-cam stand-ins. Strength follows the shroom effect's own fade
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
    #[derive(Clone, Copy, Default, ShaderType)]
    pub(crate) struct XrayParams {
        /// rgb = colour, a = brightness multiplier.
        pub(crate) color: Vec4,
        pub(crate) strength: f32,
        pub(crate) inflate: f32,
        pub(crate) fill: f32,
        pub(crate) edge_softness: f32,
        pub(crate) smoke_scale: f32,
        pub(crate) smoke_speed: f32,
        pub(crate) smoke_amount: f32,
        pub(crate) shimmer: f32,
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

/// The one material every twin shares (built on first use).
#[derive(Resource)]
struct XrayMaterialHandle(Handle<ShroomXrayMaterial>);

/// On an enemy avatar once its meshes have their twins.
#[derive(Component)]
struct XrayTwinned;

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

fn params(settings: &ShroomSettings, level: f32) -> XrayParams {
    let [r, g, b] = settings.xray_color;
    let lin = Color::srgb(r, g, b).to_linear();
    XrayParams {
        color: Vec4::new(lin.red, lin.green, lin.blue, settings.xray_brightness),
        strength: settings.xray_opacity * level,
        inflate: settings.xray_inflate,
        fill: settings.xray_fill,
        edge_softness: settings.xray_edge_softness,
        smoke_scale: settings.xray_smoke_scale,
        smoke_speed: settings.xray_smoke_speed,
        smoke_amount: settings.xray_smoke_amount,
        shimmer: settings.xray_shimmer,
    }
}

/// Give every live enemy avatar whose scene has loaded a ghost twin per mesh.
#[allow(clippy::type_complexity)]
fn twin_enemy_meshes(
    mut commands: Commands,
    handle: Option<Res<XrayMaterialHandle>>,
    settings: Res<ShroomSettings>,
    level: Res<ShroomLevel>,
    mut materials: ResMut<Assets<ShroomXrayMaterial>>,
    avatars: Query<
        Entity,
        (
            With<crate::BotLook>,
            With<crate::SoldierAnimationPlayer>,
            Without<XrayTwinned>,
            Without<crate::killcam::KillCamPlayerGhost>,
        ),
    >,
    children: Query<&Children>,
    meshes: Query<(&Mesh3d, Option<&SkinnedMesh>), Without<XrayTwin>>,
) {
    if avatars.is_empty() {
        return;
    }
    let material = match handle {
        Some(h) => h.0.clone(),
        None => {
            let h = materials.add(ShroomXrayMaterial {
                params: params(&settings, level.0),
            });
            commands.insert_resource(XrayMaterialHandle(h.clone()));
            h
        }
    };
    for root in &avatars {
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
        commands.entity(root).insert(XrayTwinned);
    }
}

/// Follow the shroom effect's fade and the panel's look; hide the twins
/// outright while the effect is off.
fn update_xray(
    settings: Res<ShroomSettings>,
    level: Res<ShroomLevel>,
    handle: Option<Res<XrayMaterialHandle>>,
    mut materials: ResMut<Assets<ShroomXrayMaterial>>,
    mut twins: Query<&mut Visibility, With<XrayTwin>>,
) {
    let on = level.0 > 1e-3;
    for mut vis in &mut twins {
        vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
    let Some(handle) = handle else { return };
    let want = params(&settings, level.0);
    // (`get` first so an unchanged material isn't re-uploaded every frame.)
    let changed = materials.get(&handle.0).is_some_and(|m| {
        let p = m.params;
        p.color != want.color
            || p.strength != want.strength
            || p.inflate != want.inflate
            || p.fill != want.fill
            || p.edge_softness != want.edge_softness
            || p.smoke_scale != want.smoke_scale
            || p.smoke_speed != want.smoke_speed
            || p.smoke_amount != want.smoke_amount
            || p.shimmer != want.shimmer
    });
    if changed {
        if let Some(m) = materials.get_mut(&handle.0) {
            m.params = want;
        }
    }
}
