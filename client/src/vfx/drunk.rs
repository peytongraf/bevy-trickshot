//! Liquid Courage's screen effect: a subtle drunk look — the view sways and
//! rolls a little as if your head won't quite hold still, a faint double image
//! drifts apart and back together, the edges darken, and there's a warm
//! flush. It kicks in hard right after the perk's bought, then settles
//! ([`super::perk_kick`]). Tuned from the debug panel ("Zombies perks" → "Liquid
//! Courage").
//!
//! One fullscreen post-process pass (`assets/shaders/drunk.wgsl`) on the
//! [`ViewModelCamera`], exactly like the shroom pass and chained right after
//! it (tonemapping → shroom → drunk), so with both perks the drunk pass works
//! on the already-shroomed frame and the two stack. Each pass only runs while
//! its own effect is faded in.

use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::core_pipeline::fullscreen_vertex_shader::fullscreen_shader_vertex_state;
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_graph::{
    NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::view::ViewTarget;
use bevy::render::RenderApp;

use super::perk_kick::{KickSettings, KickState};
use super::shroom::ShroomLabel;
use crate::player::ViewModelCamera;

const SHADER_ASSET_PATH: &str = "shaders/drunk.wgsl";

/// Panel-adjustable drunk look ("Liquid Courage" debug section).
#[derive(Resource, Clone)]
pub(crate) struct DrunkSettings {
    /// Debug: show it without owning the perk.
    pub(crate) enabled: bool,
    /// Seconds to fade fully in / out (0 = instant).
    pub(crate) fade_secs: f32,
    /// Peak roll of the head sway (degrees)...
    pub(crate) sway_roll_deg: f32,
    /// ...peak drift of it (fraction of screen height)...
    pub(crate) sway_drift: f32,
    /// ...and how fast it wanders.
    pub(crate) sway_speed: f32,
    /// Double vision: how far apart the two images get (fraction of screen
    /// height)...
    pub(crate) double_offset: f32,
    /// ...how strong the ghost image is (0..=1)...
    pub(crate) double_mix: f32,
    /// ...and how fast it drifts apart and back.
    pub(crate) double_speed: f32,
    /// How dark the edges get (0..=1).
    pub(crate) vignette: f32,
    /// Softening blur radius (fraction of screen height), stronger toward the
    /// edges — only during the kick-in: it grows with the ramp up and is gone
    /// again by the end of the ramp down.
    pub(crate) blur: f32,
    /// How much the dark edges share in the kick-in, as a fraction of the
    /// kick's peak (0.5 = the edges peak at half what everything else does).
    pub(crate) vignette_kick_scale: f32,
    /// Warm, reddish flush (0..=1).
    pub(crate) flush: f32,
    /// The strong spell right after it switches on.
    pub(crate) kick: KickSettings,
}

impl Default for DrunkSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            fade_secs: 3.0,
            sway_roll_deg: 0.8,
            sway_drift: 0.0085,
            sway_speed: 0.6,
            double_offset: 0.001,
            double_mix: 0.35,
            double_speed: 0.5,
            vignette: 0.48,
            blur: 0.0025,
            vignette_kick_scale: 0.5,
            flush: 0.27,
            kick: KickSettings::default(),
        }
    }
}

/// Whether the local player has the `Zombies` Liquid Courage perk right now
/// (set by `zombies_hud::sync_owned_perks`).
#[derive(Resource, Default)]
pub(crate) struct LiquidCouragePerk(pub(crate) bool);

/// How far the effect is faded in, 0..=1.
#[derive(Resource, Default)]
struct DrunkLevel(f32);

/// The drunk effect's kick-in (`pub(crate)` for the debug panel's replay).
#[derive(Resource, Default)]
pub(crate) struct DrunkKick(pub(crate) KickState);

pub(crate) use uniform::DrunkUniform;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct.
#[allow(dead_code)]
mod uniform {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(Component, Default, Clone, Copy, ShaderType)]
    pub(crate) struct DrunkUniform {
        pub(crate) strength: f32,
        pub(crate) time: f32,
        pub(crate) sway_roll: f32,
        pub(crate) sway_drift: f32,
        pub(crate) sway_speed: f32,
        pub(crate) double_offset: f32,
        pub(crate) double_mix: f32,
        pub(crate) double_speed: f32,
        /// Final edge darkening (fade and its own kick share already in).
        pub(crate) vignette: f32,
        /// Final blur radius (only non-zero during the kick-in).
        pub(crate) blur: f32,
        pub(crate) flush: f32,
    }
}

impl ExtractComponent for DrunkUniform {
    type QueryData = &'static DrunkUniform;
    type QueryFilter = ();
    type Out = DrunkUniform;

    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self::Out> {
        (item.strength > 0.0).then_some(*item)
    }
}

/// Must be added after `ShroomPlugin` (its node is chained after the shroom
/// one).
pub(crate) struct DrunkPlugin;

impl Plugin for DrunkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DrunkSettings>()
            .init_resource::<DrunkLevel>()
            .init_resource::<DrunkKick>()
            .init_resource::<LiquidCouragePerk>()
            .add_plugins((
                ExtractComponentPlugin::<DrunkUniform>::default(),
                UniformComponentPlugin::<DrunkUniform>::default(),
            ))
            .add_systems(Update, sync_drunk);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_render_graph_node::<ViewNodeRunner<DrunkNode>>(Core3d, DrunkLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    ShroomLabel,
                    DrunkLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<DrunkPipeline>();
    }
}

/// Fades [`DrunkLevel`] toward on / off and copies the settings onto the
/// view-model camera's [`DrunkUniform`] (added the first time a camera shows
/// up — the cameras are respawned each game).
fn sync_drunk(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<DrunkSettings>,
    perk: Res<LiquidCouragePerk>,
    mut level: ResMut<DrunkLevel>,
    mut kick: ResMut<DrunkKick>,
    new_cams: Query<Entity, (With<ViewModelCamera>, Without<DrunkUniform>)>,
    mut uniforms: Query<&mut DrunkUniform, With<ViewModelCamera>>,
) {
    for cam in &new_cams {
        commands.entity(cam).insert(DrunkUniform::default());
    }

    let target = if settings.enabled || perk.0 { 1.0 } else { 0.0 };
    level.0 = if settings.fade_secs <= 0.0 {
        target
    } else {
        let step = time.delta_secs() / settings.fade_secs;
        level.0 + (target - level.0).clamp(-step, step)
    };
    // Eased fade, times the kick-in's boost (1 once it's settled).
    let boost = kick.0.update(target > 0.0, time.delta_secs(), &settings.kick);
    let fade = level.0 * level.0 * (3.0 - 2.0 * level.0);
    let strength = fade * settings.kick.multiplier(boost);
    // The dark edges only take a share of the kick, and the blur is there
    // for the kick alone.
    let edge_peak = (settings.kick.peak * settings.vignette_kick_scale).max(1.0);
    let vignette = settings.vignette * fade * (1.0 + (edge_peak - 1.0) * boost);
    let blur = settings.blur * fade * boost;

    let s = &*settings;
    for mut u in &mut uniforms {
        *u = DrunkUniform {
            strength,
            time: time.elapsed_secs_wrapped(),
            sway_roll: s.sway_roll_deg.to_radians(),
            sway_drift: s.sway_drift,
            sway_speed: s.sway_speed,
            double_offset: s.double_offset,
            double_mix: s.double_mix,
            double_speed: s.double_speed,
            vignette,
            blur,
            flush: s.flush,
        };
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct DrunkLabel;

#[derive(Default)]
struct DrunkNode;

impl ViewNode for DrunkNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static DynamicUniformIndex<DrunkUniform>,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, uniform_index): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let drunk_pipeline = world.resource::<DrunkPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(pipeline) = pipeline_cache.get_render_pipeline(drunk_pipeline.pipeline_id) else {
            return Ok(());
        };
        let uniforms = world.resource::<ComponentUniforms<DrunkUniform>>();
        let Some(uniform_binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };

        let post_process = view_target.post_process_write();
        let bind_group = render_context.render_device().create_bind_group(
            "drunk_bind_group",
            &drunk_pipeline.layout,
            &BindGroupEntries::sequential((
                post_process.source,
                &drunk_pipeline.sampler,
                uniform_binding,
            )),
        );
        let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("drunk_pass"),
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
struct DrunkPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for DrunkPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let layout = render_device.create_bind_group_layout(
            "drunk_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<DrunkUniform>(true),
                ),
            ),
        );
        let sampler = render_device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        });
        let shader = world.load_asset(SHADER_ASSET_PATH);
        let pipeline_id =
            world
                .resource_mut::<PipelineCache>()
                .queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("drunk_pipeline".into()),
                    layout: vec![layout.clone()],
                    vertex: fullscreen_shader_vertex_state(),
                    fragment: Some(FragmentState {
                        shader,
                        shader_defs: vec![],
                        entry_point: "fragment".into(),
                        // Same HDR target as the shroom pass.
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
        Self {
            layout,
            sampler,
            pipeline_id,
        }
    }
}
