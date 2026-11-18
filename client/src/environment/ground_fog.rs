//! Ground fog: a low, uneven fog lying over a map's floor, on top of (and
//! separate from) the even `DistanceFog` — thickest at a set height and
//! thinning out above it, broken into pools by a fixed noise pattern, so it
//! looks the same every game. Each map has its own ([`GroundFogLook`], part
//! of its [`super::SceneTuning`] — the debug panel's "Fog & Sky" → the map →
//! "Ground fog"); a density of 0 — or the player turning it off in Graphics
//! (`Settings::ground_fog`) — skips the pass entirely.
//!
//! One fullscreen post-process pass (`assets/shaders/ground_fog.wesl`) on
//! the world and scope cameras, after the main pass and before bloom: it
//! reads the depth buffer to know how far each pixel's ray runs through the
//! fog. So those cameras' depth textures are made readable
//! ([`sync_ground_fog`]). The view model's camera draws afterwards, so the
//! gun's never fogged.

use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::ecs::query::QueryItem;
use bevy::post_process::bloom::bloom;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::view::{ExtractedView, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::color::Mix;
use bevy_egui::egui;

use crate::player::WorldModelCamera;
use crate::weapons::ScopeCamera;
use crate::util::color_from_parts;

use super::map::CurrentMap;

const SHADER_ASSET_PATH: &str = "shaders/ground_fog.wesl";

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

impl SyncComponent<RenderApp> for GroundFogUniform {
    // (Its uniform index and pipeline go with it once it stops being
    // extracted.)
    type Target = (Self, DynamicUniformIndex<Self>, ViewGroundFogPipeline);
}

impl ExtractComponent<RenderApp> for GroundFogUniform {
    type QueryData = &'static GroundFogUniform;
    type QueryFilter = ();
    type Out = GroundFogUniform;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
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
            .init_gpu_resource::<SpecializedRenderPipelines<GroundFogPipeline>>()
            .add_systems(RenderStartup, init_ground_fog_pipeline)
            .add_systems(Render, prepare_ground_fog_pipelines.in_set(RenderSystems::Prepare))
            // After the main pass, before bloom (so the fog glows with the
            // rest of the frame) and long before tonemapping.
            .add_systems(Core3d, ground_fog_pass.before(bloom).in_set(Core3dSystems::PostProcess));
    }
}

/// Copy the current map's [`GroundFogLook`] onto the world and scope
/// cameras' [`GroundFogUniform`] — adding it, and making their depth
/// readable, the first time a camera shows up (they're respawned each game).
/// A dog round darkens it along with the rest of the fog, and the easter
/// egg gone wrong thickens and tints it (`egg_mood.rs`).
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
    (egg_fade, egg_look): (Res<super::EggMoodFade>, Res<super::EggMoodNow>),
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
    let e = egg_fade.0;
    let egg_lerp = |a: f32, b: f32| a + (b - a) * e;
    let color = color_from_parts(g.color)
        .mix(&color_from_parts(dog_look.fog_color), dog_fade.0)
        .mix(&color_from_parts(egg_look.ground_fog_color), e);
    let color = LinearRgba::from(color) * g.brightness;
    let (sin, cos) = g.drift_dir_deg.to_radians().sin_cos();
    // (Wrapped so it never loses precision; the pattern jumps once an hour.)
    let drift = Vec2::new(cos, sin) * g.drift_speed * time.elapsed_secs_wrapped();
    let value = GroundFogUniform {
        color: Vec4::new(color.red, color.green, color.blue, 1.0),
        drift: drift.extend(0.0).extend(0.0),
        density: if hidden.0 || !settings.ground_fog {
            0.0
        } else {
            egg_lerp(g.density, egg_look.ground_fog_density.max(g.density))
        },
        base_height: g.base_height,
        falloff_height: egg_lerp(g.falloff_height, egg_look.ground_fog_falloff.max(g.falloff_height)).max(0.01),
        max_distance: g.max_distance.max(0.1),
        max_opacity: egg_lerp(g.max_opacity, egg_look.ground_fog_opacity.max(g.max_opacity)),
        splotch_size: g.splotch_size.max(0.01),
        splotchiness: g.splotchiness,
        contrast: g.contrast,
        seed: g.seed as f32,
    };
    for mut u in &mut uniforms {
        *u = value;
    }
}

/// Draw the fog for one world / scope camera view.
fn ground_fog_pass(
    // (The uniform itself, not just its index — see `vfx::fullscreen_pass`.)
    view: ViewQuery<(
        &ViewTarget,
        &ViewDepthStencilTexture,
        &ViewUniformOffset,
        &DynamicUniformIndex<GroundFogUniform>,
        &ViewGroundFogPipeline,
        &GroundFogUniform,
    )>,
    fog_pipeline: Res<GroundFogPipeline>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    uniforms: Res<ComponentUniforms<GroundFogUniform>>,
    mut ctx: RenderContext,
) {
    let (view_target, depth, view_offset, uniform_index, view_pipeline, _) = view.into_inner();
    let Some(pipeline) = pipeline_cache.get_render_pipeline(view_pipeline.id) else {
        return;
    };
    let Some(view_binding) = view_uniforms.uniforms.binding() else {
        return;
    };
    let Some(uniform_binding) = uniforms.uniforms().binding() else {
        return;
    };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else {
        return;
    };
    let layout = if view_pipeline.msaa {
        &fog_pipeline.layout_msaa
    } else {
        &fog_pipeline.layout
    };

    let post_process = view_target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        "ground_fog_bind_group",
        &pipeline_cache.get_bind_group_layout(layout),
        &BindGroupEntries::sequential((post_process.source, depth_view, view_binding, uniform_binding)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("ground_fog_pass"),
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
    pass.set_bind_group(0, &bind_group, &[view_offset.offset, uniform_index.index()]);
    pass.draw(0..3, 0..1);
}

/// The layouts for each kind of depth buffer: plain, and multisampled (MSAA
/// on).
#[derive(Resource)]
struct GroundFogPipeline {
    layout: BindGroupLayoutDescriptor,
    layout_msaa: BindGroupLayoutDescriptor,
    fullscreen_shader: FullscreenShader,
    shader: Handle<Shader>,
}

fn init_ground_fog_pipeline(
    mut commands: Commands,
    fullscreen_shader: Res<FullscreenShader>,
    asset_server: Res<AssetServer>,
) {
    let make_layout = |label: &'static str, depth: BindGroupLayoutEntryBuilder| {
        BindGroupLayoutDescriptor::new(
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
    commands.insert_resource(GroundFogPipeline {
        layout: make_layout("ground_fog_bind_group_layout", texture_depth_2d()),
        layout_msaa: make_layout("ground_fog_msaa_bind_group_layout", texture_depth_2d_multisampled()),
        fullscreen_shader: fullscreen_shader.clone(),
        shader: asset_server.load(SHADER_ASSET_PATH),
    });
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GroundFogKey {
    format: TextureFormat,
    msaa: bool,
}

impl SpecializedRenderPipeline for GroundFogPipeline {
    type Key = GroundFogKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let (label, layout, shader_defs) = if key.msaa {
            ("ground_fog_msaa_pipeline", &self.layout_msaa, vec!["MULTISAMPLED".into()])
        } else {
            ("ground_fog_pipeline", &self.layout, vec![])
        };
        RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                entry_point: Some("fragment".into()),
                // Before tonemapping, in the cameras' HDR format.
                targets: vec![Some(ColorTargetState {
                    format: key.format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

/// This view's specialized fog pipeline.
#[derive(Component)]
pub(crate) struct ViewGroundFogPipeline {
    id: CachedRenderPipelineId,
    msaa: bool,
}

fn prepare_ground_fog_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<GroundFogPipeline>>,
    pipeline: Res<GroundFogPipeline>,
    views: Query<(Entity, &ExtractedView, &ExtractedCamera, &Msaa), With<GroundFogUniform>>,
) {
    for (entity, view, camera, msaa) in &views {
        // (Only drawn into HDR targets, as every in-game camera is.)
        if !camera.hdr {
            commands.entity(entity).remove::<ViewGroundFogPipeline>();
            continue;
        }
        let key = GroundFogKey {
            format: view.target_format,
            msaa: msaa.samples() > 1,
        };
        let id = pipelines.specialize(&pipeline_cache, &pipeline, key);
        commands.entity(entity).insert(ViewGroundFogPipeline { id, msaa: key.msaa });
    }
}
