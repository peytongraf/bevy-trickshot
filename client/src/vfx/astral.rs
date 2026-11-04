//! The astral look: how the world's seen while our soul's out of our body
//! (the easter egg's trip into the pyramid, `easter_egg::ascension`, which
//! drives it through [`AstralScreen`]) — the frame breathing and swirling at
//! its edges, colours splitting apart toward them, a slowly turning lattice
//! of sacred geometry glimmering in iridescent light round the edges of
//! sight, and a pale glow closing in — all stronger while we're flying
//! (`travel`), the view pulsing like a tunnel, and gentler once we're in the
//! chamber.
//!
//! One fullscreen post-process pass (`assets/shaders/astral.wgsl`) on the
//! [`ViewModelCamera`], chained after the Aether Shroud's (tonemapping →
//! shroom → drunk → aether → astral), only run while it's faded in. Its
//! fade's reset on leaving a game (`easter_egg::ascension` puts
//! [`AstralScreen`] back to off).

use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::core_pipeline::fullscreen_vertex_shader::fullscreen_shader_vertex_state;
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_graph::{NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::view::ViewTarget;
use bevy::render::RenderApp;

use super::aether_shroud::AetherLabel;
use crate::player::ViewModelCamera;

const SHADER_ASSET_PATH: &str = "shaders/astral.wgsl";

/// What the astral look should be right now — set every frame by whoever
/// drives it (`easter_egg::ascension`); off by default.
#[derive(Resource, Clone, Copy, Default, PartialEq)]
pub(crate) struct AstralScreen {
    /// How strongly it's on (0 off .. 1).
    pub(crate) strength: f32,
    /// How much we're flying (0 still .. 1) — the edges race, the view
    /// pulses like a tunnel.
    pub(crate) travel: f32,
    /// How much it warps, splits the colours, shows the lattice, shimmers
    /// and closes in — each 0..1-ish.
    pub(crate) warp: f32,
    pub(crate) aberration: f32,
    pub(crate) pattern: f32,
    pub(crate) iridescence: f32,
    pub(crate) glow: f32,
    /// The glow closing in (linear rgb).
    pub(crate) tint: [f32; 3],
}

pub(crate) use uniform::AstralUniform;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct.
#[allow(dead_code)]
mod uniform {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Mirrors `AstralUniform` in `astral.wgsl`.
    #[derive(Component, Default, Clone, Copy, ShaderType)]
    pub(crate) struct AstralUniform {
        pub(crate) strength: f32,
        pub(crate) time: f32,
        pub(crate) travel: f32,
        pub(crate) warp: f32,
        pub(crate) aberration: f32,
        pub(crate) pattern: f32,
        pub(crate) iridescence: f32,
        pub(crate) glow: f32,
        pub(crate) tint_r: f32,
        pub(crate) tint_g: f32,
        pub(crate) tint_b: f32,
        pub(crate) _pad: f32,
    }
}

impl ExtractComponent for AstralUniform {
    type QueryData = &'static AstralUniform;
    type QueryFilter = ();
    type Out = AstralUniform;

    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self::Out> {
        (item.strength > 0.0).then_some(*item)
    }
}

/// Must be added after `AetherScreenPlugin` (its node is chained after that
/// one).
pub(crate) struct AstralPlugin;

impl Plugin for AstralPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AstralScreen>()
            .add_plugins((
                ExtractComponentPlugin::<AstralUniform>::default(),
                UniformComponentPlugin::<AstralUniform>::default(),
            ))
            .add_systems(PostUpdate, sync_astral);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_render_graph_node::<ViewNodeRunner<AstralNode>>(Core3d, AstralLabel)
            .add_render_graph_edges(Core3d, (AetherLabel, AstralLabel, Node3d::EndMainPassPostProcessing));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<AstralPipeline>();
    }
}

/// Copies [`AstralScreen`] onto the view-model camera's [`AstralUniform`]
/// (added the first time a camera shows up — the cameras are respawned each
/// game).
fn sync_astral(
    mut commands: Commands,
    time: Res<Time>,
    screen: Res<AstralScreen>,
    new_cams: Query<Entity, (With<ViewModelCamera>, Without<AstralUniform>)>,
    mut uniforms: Query<&mut AstralUniform, With<ViewModelCamera>>,
) {
    for cam in &new_cams {
        commands.entity(cam).insert(AstralUniform::default());
    }
    let s = *screen;
    for mut u in &mut uniforms {
        *u = AstralUniform {
            strength: s.strength,
            time: time.elapsed_secs_wrapped(),
            travel: s.travel,
            warp: s.warp,
            aberration: s.aberration,
            pattern: s.pattern,
            iridescence: s.iridescence,
            glow: s.glow,
            tint_r: s.tint[0],
            tint_g: s.tint[1],
            tint_b: s.tint[2],
            _pad: 0.0,
        };
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub(super) struct AstralLabel;

#[derive(Default)]
struct AstralNode;

impl ViewNode for AstralNode {
    // (The uniform itself, not just its index — see `drunk::DrunkNode`: a
    // stale index would keep the pass running after it's off.)
    type ViewQuery = (&'static ViewTarget, &'static DynamicUniformIndex<AstralUniform>, &'static AstralUniform);

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, uniform_index, _): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let astral_pipeline = world.resource::<AstralPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(pipeline) = pipeline_cache.get_render_pipeline(astral_pipeline.pipeline_id) else {
            return Ok(());
        };
        let uniforms = world.resource::<ComponentUniforms<AstralUniform>>();
        let Some(uniform_binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };
        let post_process = view_target.post_process_write();
        let bind_group = render_context.render_device().create_bind_group(
            "astral_bind_group",
            &astral_pipeline.layout,
            &BindGroupEntries::sequential((post_process.source, &astral_pipeline.sampler, uniform_binding)),
        );
        let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("astral_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post_process.destination,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[uniform_index.index()]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

#[derive(Resource)]
struct AstralPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for AstralPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let layout = render_device.create_bind_group_layout(
            "astral_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<AstralUniform>(true),
                ),
            ),
        );
        let sampler = render_device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        });
        let shader = world.load_asset(SHADER_ASSET_PATH);
        let pipeline_id = world.resource_mut::<PipelineCache>().queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("astral_pipeline".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen_shader_vertex_state(),
            fragment: Some(FragmentState {
                shader,
                shader_defs: vec![],
                entry_point: "fragment".into(),
                // (The same HDR target as the passes before it.)
                targets: vec![Some(ColorTargetState {
                    format: ViewTarget::TEXTURE_FORMAT_HDR,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            push_constant_ranges: vec![],
            zero_initialize_workgroup_memory: false,
        });
        Self { layout, sampler, pipeline_id }
    }
}
