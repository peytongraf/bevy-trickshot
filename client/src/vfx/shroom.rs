//! Shroom screen effect: a subtle, slow "breathing" wavy distortion of the
//! whole first-person view plus a saturation / hue shimmer — the look of the
//! future mushroom perk. For now it's just toggled and tuned from the debug
//! panel (`ads_tuning_ui`'s "Shroom effect" section).
//!
//! One fullscreen post-process pass (`assets/shaders/shroom.wgsl`) run after
//! tonemapping on the [`ViewModelCamera`] — the last 3D camera to draw, and
//! it shares its render target with the world camera, so the pass sees world
//! and gun together. The HUD (its own 2D camera) and egui aren't distorted.
//! While the effect is off (or fully faded out) the pass doesn't run at all.

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

use crate::player::ViewModelCamera;

const SHADER_ASSET_PATH: &str = "shaders/shroom.wgsl";

/// Panel-adjustable shroom look (`ads_tuning_ui`'s "Shroom effect" section).
#[derive(Resource, Clone)]
pub(crate) struct ShroomSettings {
    pub(crate) enabled: bool,
    /// Seconds to fade fully in / out when toggled (0 = instant).
    pub(crate) fade_secs: f32,
    /// Warp displacement, as a fraction of screen height.
    pub(crate) wave_amplitude: f32,
    /// Spatial frequency of the waves (higher = smaller, busier ripples).
    pub(crate) wave_frequency: f32,
    /// How fast the waves flow.
    pub(crate) wave_speed: f32,
    /// Peak extra zoom of the slow "breathing" pulse (0.02 = 2%).
    pub(crate) breathe_amplitude: f32,
    /// Breathing pulse rate (radians / s).
    pub(crate) breathe_speed: f32,
    /// 0 = warp evenly everywhere, 1 = keep the screen centre (the aim) still.
    pub(crate) center_clear: f32,
    /// Colour fringing toward the screen edges.
    pub(crate) chromatic: f32,
    /// Saturation multiplier (1 = unchanged).
    pub(crate) saturation: f32,
    /// Peak hue rotation of the colour shimmer (radians).
    pub(crate) hue_drift: f32,
    /// How fast the hue shimmer moves.
    pub(crate) hue_speed: f32,
    /// See-through-walls enemy ghosts (`shroom_xray`): sRGB colour...
    pub(crate) xray_color: [f32; 3],
    /// ...glow multiplier (above 1 blooms)...
    pub(crate) xray_brightness: f32,
    /// ...peak opacity...
    pub(crate) xray_opacity: f32,
    /// ...how far (m) the haze puffs out past the body...
    pub(crate) xray_inflate: f32,
    /// ...base opacity right out at the edge (0 = edges fully fade)...
    pub(crate) xray_fill: f32,
    /// ...how quickly it fades toward the outline (higher = thinner, softer
    /// edge)...
    pub(crate) xray_edge_softness: f32,
    /// ...smoke wisp size (higher = finer), drift speed, and how much it
    /// breaks the haze up...
    pub(crate) xray_smoke_scale: f32,
    pub(crate) xray_smoke_speed: f32,
    pub(crate) xray_smoke_amount: f32,
    /// ...and the psychedelic hue wobble (radians).
    pub(crate) xray_shimmer: f32,
    /// Aim assist (`player::shroom_aim_assist`) while aimed down sight: on /
    /// off...
    pub(crate) assist_enabled: bool,
    /// ...how far off the crosshair (degrees) a bot still pulls...
    pub(crate) assist_cone_deg: f32,
    /// ...how hard it pulls (per second — higher snaps on quicker)...
    pub(crate) assist_strength: f32,
    /// ...the fastest it may turn the aim (degrees / s)...
    pub(crate) assist_max_speed_deg: f32,
    /// ...how far (m) away a bot can be...
    pub(crate) assist_range: f32,
    /// ...and how scoped in (`Ads::t`, 0..=1) the player must be.
    pub(crate) assist_min_ads: f32,
}

impl Default for ShroomSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            fade_secs: 3.0,
            wave_amplitude: 0.01,
            wave_frequency: 4.0,
            wave_speed: 0.5,
            breathe_amplitude: 0.07,
            breathe_speed: 0.7,
            center_clear: 1.0,
            chromatic: 0.023,
            saturation: 1.65,
            hue_drift: 0.15,
            hue_speed: 1.25,
            xray_color: [1.0, 0.92, 0.2],
            xray_brightness: 3.0,
            xray_opacity: 0.85,
            xray_inflate: 0.06,
            xray_fill: 0.1,
            xray_edge_softness: 1.5,
            xray_smoke_scale: 3.0,
            xray_smoke_speed: 0.8,
            xray_smoke_amount: 0.6,
            xray_shimmer: 0.35,
            assist_enabled: true,
            assist_cone_deg: 4.0,
            assist_strength: 5.0,
            assist_max_speed_deg: 25.0,
            assist_range: 150.0,
            assist_min_ads: 0.8,
        }
    }
}

/// Whether the local player has the `Zombies` Shroom Tea perk right now (set
/// by `zombies_hud::sync_shroom_perk`) — turns the effect on alongside the
/// debug panel's own toggle.
#[derive(Resource, Default)]
pub(crate) struct ShroomPerk(pub(crate) bool);

/// How far the effect is faded in right now, 0..=1 — eased toward
/// `ShroomSettings::enabled` over `fade_secs` by [`sync_shroom`].
#[derive(Resource, Default)]
pub(crate) struct ShroomLevel(pub(crate) f32);

/// Per-camera uniform the shader reads. Lives on the [`ViewModelCamera`];
/// only extracted to the render world while `strength > 0`, so the pass is
/// skipped entirely when the effect is off.
pub(crate) use uniform::ShroomUniform;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct.
#[allow(dead_code)]
mod uniform {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(Component, Default, Clone, Copy, ShaderType)]
    pub(crate) struct ShroomUniform {
        pub(crate) strength: f32,
        pub(crate) time: f32,
        pub(crate) wave_amplitude: f32,
        pub(crate) wave_frequency: f32,
        pub(crate) wave_speed: f32,
        pub(crate) breathe_amplitude: f32,
        pub(crate) breathe_speed: f32,
        pub(crate) center_clear: f32,
        pub(crate) chromatic: f32,
        pub(crate) saturation: f32,
        pub(crate) hue_drift: f32,
        pub(crate) hue_speed: f32,
    }
}

impl ExtractComponent for ShroomUniform {
    type QueryData = &'static ShroomUniform;
    type QueryFilter = ();
    type Out = ShroomUniform;

    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self::Out> {
        (item.strength > 0.0).then_some(*item)
    }
}

pub(crate) struct ShroomPlugin;

impl Plugin for ShroomPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShroomSettings>()
            .init_resource::<ShroomLevel>()
            .init_resource::<ShroomPerk>()
            .add_plugins((
                ExtractComponentPlugin::<ShroomUniform>::default(),
                UniformComponentPlugin::<ShroomUniform>::default(),
            ))
            .add_systems(Update, sync_shroom);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_render_graph_node::<ViewNodeRunner<ShroomNode>>(Core3d, ShroomLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    ShroomLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<ShroomPipeline>();
    }
}

/// Fades [`ShroomLevel`] toward the toggle and copies the settings onto the
/// view-model camera's [`ShroomUniform`] (adding it the first time a camera
/// shows up — the cameras are respawned each game).
fn sync_shroom(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<ShroomSettings>,
    perk: Res<ShroomPerk>,
    mut level: ResMut<ShroomLevel>,
    new_cams: Query<Entity, (With<ViewModelCamera>, Without<ShroomUniform>)>,
    mut uniforms: Query<&mut ShroomUniform, With<ViewModelCamera>>,
) {
    for cam in &new_cams {
        commands.entity(cam).insert(ShroomUniform::default());
    }

    let target = if settings.enabled || perk.0 { 1.0 } else { 0.0 };
    level.0 = if settings.fade_secs <= 0.0 {
        target
    } else {
        let step = time.delta_secs() / settings.fade_secs;
        level.0 + (target - level.0).clamp(-step, step)
    };
    // Smoothstep so the onset / wear-off eases in and out, not linearly.
    let strength = level.0 * level.0 * (3.0 - 2.0 * level.0);

    let s = &*settings;
    for mut u in &mut uniforms {
        *u = ShroomUniform {
            strength,
            time: time.elapsed_secs_wrapped(),
            wave_amplitude: s.wave_amplitude,
            wave_frequency: s.wave_frequency,
            wave_speed: s.wave_speed,
            breathe_amplitude: s.breathe_amplitude,
            breathe_speed: s.breathe_speed,
            center_clear: s.center_clear,
            chromatic: s.chromatic,
            saturation: s.saturation,
            hue_drift: s.hue_drift,
            hue_speed: s.hue_speed,
        };
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ShroomLabel;

#[derive(Default)]
struct ShroomNode;

impl ViewNode for ShroomNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static DynamicUniformIndex<ShroomUniform>,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, uniform_index): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let shroom_pipeline = world.resource::<ShroomPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(pipeline) = pipeline_cache.get_render_pipeline(shroom_pipeline.pipeline_id)
        else {
            return Ok(());
        };
        let uniforms = world.resource::<ComponentUniforms<ShroomUniform>>();
        let Some(uniform_binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };

        let post_process = view_target.post_process_write();
        let bind_group = render_context.render_device().create_bind_group(
            "shroom_bind_group",
            &shroom_pipeline.layout,
            &BindGroupEntries::sequential((
                post_process.source,
                &shroom_pipeline.sampler,
                uniform_binding,
            )),
        );
        let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("shroom_pass"),
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
struct ShroomPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for ShroomPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let layout = render_device.create_bind_group_layout(
            "shroom_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<ShroomUniform>(true),
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
                    label: Some("shroom_pipeline".into()),
                    layout: vec![layout.clone()],
                    vertex: fullscreen_shader_vertex_state(),
                    fragment: Some(FragmentState {
                        shader,
                        shader_defs: vec![],
                        entry_point: "fragment".into(),
                        // Every in-game camera is `hdr: true`, so the view
                        // target stays in the HDR format even after tonemapping.
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

