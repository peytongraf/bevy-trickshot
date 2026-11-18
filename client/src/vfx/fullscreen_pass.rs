//! The shared plumbing behind the view-model camera's stacked fullscreen
//! post-process passes — shroom → drunk → Aether Shroud → astral. Each is one
//! fragment shader reading the frame so far, a linear sampler and its own
//! dynamic uniform `U` (a component on the view-model camera), drawn in
//! `Core3d` after tonemapping. A pass only runs while its uniform is
//! extracted (each one's `ExtractComponent` skips it at zero strength), so a
//! faded-out effect costs nothing.
//!
//! Each effect's plugin calls [`add`] with the pass it runs after:
//! `tonemapping` for the first, [`run::<Previous>`](run) for the rest.

use std::marker::PhantomData;

use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::FullscreenShader;
use bevy::ecs::schedule::IntoSystemSet;
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::encase::internal::WriteInto;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};

/// One fullscreen pass's uniform: what it's called and which shader draws it.
pub(crate) trait FullscreenPass:
    Component + ShaderType + WriteInto + Clone + ExtractComponent<RenderApp, Out = Self>
{
    /// Debug label for the pipeline / pass.
    const LABEL: &'static str;
    /// The fragment shader (entry point `fragment`), an asset path.
    const SHADER: &'static str;
}

/// Register pass `U`, drawn after `after` (`tonemapping`, or the previous
/// pass's [`run`]).
pub(crate) fn add<U: FullscreenPass, M>(app: &mut App, after: impl IntoSystemSet<M>) {
    app.add_plugins((
        ExtractComponentPlugin::<U>::default(),
        UniformComponentPlugin::<U>::default(),
    ));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_gpu_resource::<SpecializedRenderPipelines<PassPipeline<U>>>()
        .add_systems(RenderStartup, init_pipeline::<U>)
        .add_systems(Render, prepare_pipelines::<U>.in_set(RenderSystems::Prepare))
        .add_systems(
            Core3d,
            run::<U>.after(tonemapping).after(after).in_set(Core3dSystems::PostProcess),
        );
}

#[derive(Resource)]
pub(crate) struct PassPipeline<U> {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen_shader: FullscreenShader,
    fragment_shader: Handle<Shader>,
    marker: PhantomData<fn() -> U>,
}

fn init_pipeline<U: FullscreenPass>(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    fullscreen_shader: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        U::LABEL,
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<U>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(PassPipeline::<U> {
        layout,
        sampler,
        fullscreen_shader: fullscreen_shader.clone(),
        fragment_shader: asset_server.load(U::SHADER),
        marker: PhantomData,
    });
}

impl<U: FullscreenPass> SpecializedRenderPipeline for PassPipeline<U> {
    /// The view's target format (the cameras are HDR).
    type Key = TextureFormat;

    fn specialize(&self, format: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some(U::LABEL.into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.fragment_shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

/// This view's specialized pipeline for pass `U`.
#[derive(Component)]
pub(crate) struct ViewPassPipeline<U> {
    id: CachedRenderPipelineId,
    marker: PhantomData<fn() -> U>,
}

fn prepare_pipelines<U: FullscreenPass>(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PassPipeline<U>>>,
    pipeline: Res<PassPipeline<U>>,
    views: Query<(Entity, &ExtractedView), With<U>>,
) {
    for (entity, view) in &views {
        let id = pipelines.specialize(&pipeline_cache, &pipeline, view.target_format);
        commands.entity(entity).insert(ViewPassPipeline::<U> {
            id,
            marker: PhantomData,
        });
    }
}

/// Draw pass `U` for one view. (It queries the uniform itself, not just its
/// index: only that's sure to be gone once the effect stops being
/// extracted, and running on stale data would flip the post-process
/// textures every frame long after the effect is off.)
pub(crate) fn run<U: FullscreenPass>(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<U>, &ViewPassPipeline<U>, &U)>,
    pass_pipeline: Res<PassPipeline<U>>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<U>>,
    mut ctx: RenderContext,
) {
    let (view_target, uniform_index, view_pipeline, _) = view.into_inner();
    let Some(pipeline) = pipeline_cache.get_render_pipeline(view_pipeline.id) else {
        return;
    };
    let Some(uniform_binding) = uniforms.uniforms().binding() else {
        return;
    };

    let post_process = view_target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        U::LABEL,
        &pipeline_cache.get_bind_group_layout(&pass_pipeline.layout),
        &BindGroupEntries::sequential((post_process.source, &pass_pipeline.sampler, uniform_binding)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some(U::LABEL),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post_process.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[uniform_index.index()]);
    pass.draw(0..3, 0..1);
}
