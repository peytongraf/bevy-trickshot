//! Ground fog: a low, uneven fog lying over a map's floor, on top of (and
//! separate from) the even `DistanceFog` — thickest at a set height and
//! thinning out above it, broken into pools by a fixed noise pattern, so it
//! looks the same every game. Each map has its own ([`GroundFogLook`], part
//! of its [`super::SceneTuning`] — the debug panel's "Fog & Sky" → the map →
//! "Ground fog"); a density of 0 — or the player turning it off in Graphics
//! (`Settings::ground_fog`) — skips the pass entirely.
//!
//! One fullscreen post-process pass (`assets/shaders/ground_fog.wgsl`) on
//! the world and scope cameras, after the main pass and before bloom: it
//! reads the depth buffer to know how far each pixel's ray runs through the
//! fog. So those cameras' depth textures are made readable
//! ([`sync_ground_fog`]). The view model's camera draws afterwards, so the
//! gun's never fogged.

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
use bevy::render::render_resource::binding_types::{
    texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::view::{ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::RenderApp;
use bevy::color::Mix;
use bevy_egui::egui;

use crate::player::WorldModelCamera;
use crate::weapons::ScopeCamera;
use crate::util::color_from_parts;

use super::map::CurrentMap;

const SHADER_ASSET_PATH: &str = "shaders/ground_fog.wgsl";

/// One map's ground fog.
#[derive(Clone, Debug)]
pub(crate) struct GroundFogLook {
    /// How thick it is at its base (per metre; 0 = none).
    pub(crate) density: f32,
    /// The height (m, world) it's thickest at — the floor it lies on. It
    /// fades out quickly below, so a lower floor stays clear.
    pub(crate) base_height: f32,
    /// How quickly it thins out going up: the height (m) over which it
    /// drops to about a third.
    pub(crate) falloff_height: f32,
    pub(crate) color: [f32; 3],
    /// Multiplies its colour (above 1 glows).
    pub(crate) brightness: f32,
    /// The furthest (m) it's summed along a view — the sky and anything
    /// further off only get this much of it.
    pub(crate) max_distance: f32,
    /// The most it ever hides (0..1).
    pub(crate) max_opacity: f32,
    /// How wide (m) its pools and gaps are.
    pub(crate) splotch_size: f32,
    /// 0 = perfectly even, 1 = all pools and gaps.
    pub(crate) splotchiness: f32,
    /// How sharp the pools' edges are (1 = soft, higher = crisper, with
    /// clearer gaps between).
    pub(crate) contrast: f32,
    /// Picks the pattern — the same seed always lays it out the same.
    pub(crate) seed: u32,
    /// How fast (m/s) it drifts (0 = it stays put)...
    pub(crate) drift_speed: f32,
    /// ...and which way (degrees around from +x).
    pub(crate) drift_dir_deg: f32,
}

impl GroundFogLook {
    /// None at all.
    pub(crate) fn off() -> Self {
        Self {
            density: 0.0,
            base_height: 0.0,
            falloff_height: 0.7,
            color: [0.6, 0.62, 0.66],
            brightness: 1.0,
            max_distance: 150.0,
            max_opacity: 0.95,
            splotch_size: 9.0,
            splotchiness: 0.8,
            contrast: 2.0,
            seed: 1,
            drift_speed: 0.0,
            drift_dir_deg: 0.0,
        }
    }
}

/// One map's "Ground fog" sliders.
pub(crate) fn ground_fog_sliders(ui: &mut egui::Ui, g: &mut GroundFogLook) {
    ui.label("Low, uneven fog over the floor, on top of the even fog above (density 0 = off).");
    ui.add(egui::Slider::new(&mut g.density, 0.0f32..=1.0).logarithmic(true).text("density at the base (/m)"));
    ui.add(egui::Slider::new(&mut g.base_height, -20.0f32..=40.0).text("base height (m, world)"));
    ui.add(egui::Slider::new(&mut g.falloff_height, 0.05f32..=10.0).logarithmic(true).text("thins out over (m)"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut g.color);
        ui.label("colour");
    });
    ui.add(egui::Slider::new(&mut g.brightness, 0.0f32..=10.0).text("brightness"));
    ui.add(egui::Slider::new(&mut g.max_opacity, 0.0f32..=1.0).text("most it hides"));
    ui.add(egui::Slider::new(&mut g.max_distance, 5.0f32..=1000.0).logarithmic(true).text("summed out to (m)"));
    ui.add(egui::Slider::new(&mut g.splotch_size, 0.5f32..=100.0).logarithmic(true).text("pool size (m)"));
    ui.add(egui::Slider::new(&mut g.splotchiness, 0.0f32..=1.0).text("splotchiness"));
    ui.add(egui::Slider::new(&mut g.contrast, 0.5f32..=6.0).text("pool edge sharpness"));
    ui.add(egui::Slider::new(&mut g.seed, 0u32..=999).text("pattern seed"));
    ui.add(egui::Slider::new(&mut g.drift_speed, 0.0f32..=3.0).text("drift (m/s)"));
    ui.add(egui::Slider::new(&mut g.drift_dir_deg, -180.0f32..=180.0).text("drift direction (deg)"));
}

/// Set while the level editor's own bright look is on — no ground fog then.
#[derive(Resource, Default)]
pub(crate) struct GroundFogHidden(pub(crate) bool);

pub(crate) use uniform::GroundFogUniform;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct.
#[allow(dead_code)]
mod uniform {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Per-camera uniform the shader reads (same order as its `GroundFog`).
    /// Only extracted while `density > 0`, so the pass is skipped otherwise.
    #[derive(Component, Default, Clone, Copy, ShaderType)]
    pub(crate) struct GroundFogUniform {
        pub(crate) color: Vec4,
        pub(crate) drift: Vec4,
        pub(crate) density: f32,
        pub(crate) base_height: f32,
        pub(crate) falloff_height: f32,
        pub(crate) max_distance: f32,
        pub(crate) max_opacity: f32,
        pub(crate) splotch_size: f32,
        pub(crate) splotchiness: f32,
        pub(crate) contrast: f32,
        pub(crate) seed: f32,
    }
}

impl ExtractComponent for GroundFogUniform {
    type QueryData = &'static GroundFogUniform;
    type QueryFilter = ();
    type Out = GroundFogUniform;

    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self::Out> {
        (item.density > 0.0 && item.max_opacity > 0.0).then_some(*item)
    }
}

pub(crate) struct GroundFogPlugin;

impl Plugin for GroundFogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GroundFogHidden>()
            .add_plugins((
                ExtractComponentPlugin::<GroundFogUniform>::default(),
                UniformComponentPlugin::<GroundFogUniform>::default(),
            ))
            .add_systems(PostUpdate, sync_ground_fog);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_render_graph_node::<ViewNodeRunner<GroundFogNode>>(Core3d, GroundFogLabel)
            .add_render_graph_edges(Core3d, (Node3d::EndMainPass, GroundFogLabel, Node3d::Bloom));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<GroundFogPipeline>();
    }
}

/// Copy the current map's [`GroundFogLook`] onto the world and scope
/// cameras' [`GroundFogUniform`] — adding it, and making their depth
/// readable, the first time a camera shows up (they're respawned each game).
/// A dog round darkens it along with the rest of the fog.
#[allow(clippy::too_many_arguments)]
fn sync_ground_fog(
    mut commands: Commands,
    time: Res<Time>,
    current: Res<CurrentMap>,
    looks: Res<super::MapLooks>,
    hidden: Res<GroundFogHidden>,
    settings: Res<crate::settings::Settings>,
    dog_fade: Res<super::DogRoundFade>,
    dog_look: Res<super::DogRoundLook>,
    mut new_cams: Query<
        (Entity, &mut Camera3d),
        (Or<(With<WorldModelCamera>, With<ScopeCamera>)>, Without<GroundFogUniform>),
    >,
    mut uniforms: Query<&mut GroundFogUniform, Or<(With<WorldModelCamera>, With<ScopeCamera>)>>,
) {
    for (cam, mut camera_3d) in &mut new_cams {
        let usages = TextureUsages::from(camera_3d.depth_texture_usages) | TextureUsages::TEXTURE_BINDING;
        camera_3d.depth_texture_usages = usages.into();
        commands.entity(cam).insert(GroundFogUniform::default());
    }

    let g = &looks.get(current.0).ground_fog;
    let color = color_from_parts(g.color).mix(&color_from_parts(dog_look.fog_color), dog_fade.0);
    let color = LinearRgba::from(color) * g.brightness;
    let (sin, cos) = g.drift_dir_deg.to_radians().sin_cos();
    // (Wrapped so it never loses precision; the pattern jumps once an hour.)
    let drift = Vec2::new(cos, sin) * g.drift_speed * time.elapsed_secs_wrapped();
    let value = GroundFogUniform {
        color: Vec4::new(color.red, color.green, color.blue, 1.0),
        drift: drift.extend(0.0).extend(0.0),
        density: if hidden.0 || !settings.ground_fog { 0.0 } else { g.density },
        base_height: g.base_height,
        falloff_height: g.falloff_height.max(0.01),
        max_distance: g.max_distance.max(0.1),
        max_opacity: g.max_opacity,
        splotch_size: g.splotch_size.max(0.01),
        splotchiness: g.splotchiness,
        contrast: g.contrast,
        seed: g.seed as f32,
    };
    for mut u in &mut uniforms {
        *u = value;
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct GroundFogLabel;

#[derive(Default)]
struct GroundFogNode;

impl ViewNode for GroundFogNode {
    // (The uniform itself, not just its index — see `vfx::shroom`'s node.)
    type ViewQuery = (
        &'static ViewTarget,
        &'static ViewDepthTexture,
        &'static ViewUniformOffset,
        &'static Msaa,
        &'static DynamicUniformIndex<GroundFogUniform>,
        &'static GroundFogUniform,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, depth, view_offset, msaa, uniform_index, _): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        if !view_target.is_hdr() {
            return Ok(());
        }
        let fog_pipeline = world.resource::<GroundFogPipeline>();
        let (layout, pipeline_id) = if msaa.samples() > 1 {
            (&fog_pipeline.layout_msaa, fog_pipeline.pipeline_msaa)
        } else {
            (&fog_pipeline.layout, fog_pipeline.pipeline)
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(pipeline) = pipeline_cache.get_render_pipeline(pipeline_id) else {
            return Ok(());
        };
        let Some(view_binding) = world.resource::<ViewUniforms>().uniforms.binding() else {
            return Ok(());
        };
        let uniforms = world.resource::<ComponentUniforms<GroundFogUniform>>();
        let Some(uniform_binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };

        let post_process = view_target.post_process_write();
        let bind_group = render_context.render_device().create_bind_group(
            "ground_fog_bind_group",
            layout,
            &BindGroupEntries::sequential((
                post_process.source,
                depth.view(),
                view_binding,
                uniform_binding,
            )),
        );
        let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("ground_fog_pass"),
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
        pass.set_bind_group(0, &bind_group, &[view_offset.offset, uniform_index.index()]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

/// One pipeline for each kind of depth buffer: plain, and multisampled
/// (MSAA on).
#[derive(Resource)]
struct GroundFogPipeline {
    layout: BindGroupLayout,
    layout_msaa: BindGroupLayout,
    pipeline: CachedRenderPipelineId,
    pipeline_msaa: CachedRenderPipelineId,
}

impl FromWorld for GroundFogPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let make_layout = |label: &'static str, depth: BindGroupLayoutEntryBuilder| {
            render_device.create_bind_group_layout(
                label,
                &BindGroupLayoutEntries::sequential(
                    ShaderStages::FRAGMENT,
                    (
                        texture_2d(TextureSampleType::Float { filterable: false }),
                        depth,
                        uniform_buffer::<ViewUniform>(true),
                        uniform_buffer::<GroundFogUniform>(true),
                    ),
                ),
            )
        };
        let layout = make_layout("ground_fog_bind_group_layout", texture_depth_2d());
        let layout_msaa = make_layout("ground_fog_msaa_bind_group_layout", texture_depth_2d_multisampled());
        let shader: Handle<Shader> = world.load_asset(SHADER_ASSET_PATH);
        let cache = world.resource_mut::<PipelineCache>();
        let queue = |label: &'static str, layout: &BindGroupLayout, shader_defs: Vec<ShaderDefVal>| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some(label.into()),
                layout: vec![layout.clone()],
                vertex: fullscreen_shader_vertex_state(),
                fragment: Some(FragmentState {
                    shader: shader.clone(),
                    shader_defs,
                    entry_point: "fragment".into(),
                    // Before tonemapping, in the cameras' HDR format (every
                    // in-game camera is `hdr: true`).
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
            })
        };
        let pipeline = queue("ground_fog_pipeline", &layout, vec![]);
        let pipeline_msaa = queue("ground_fog_msaa_pipeline", &layout_msaa, vec!["MULTISAMPLED".into()]);
        Self {
            layout,
            layout_msaa,
            pipeline,
            pipeline_msaa,
        }
    }
}
