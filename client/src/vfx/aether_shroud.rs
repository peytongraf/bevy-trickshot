//! The Aether Shroud's screen effect (a `Zombies` field upgrade, see
//! `crate::aether_shroud`): while it's up the whole view goes a bright, icy
//! blue-purple with a fisheye warp toward the centre, colour fringing and a
//! shimmering purple haze at the edges — Cold War's look — with a stronger
//! warp and flash right as it goes up, and every so often a lightning bolt
//! flicking in from the edge of the screen ([`BOLTS`] at most at once,
//! placed and timed here at random and drawn by the shader). Tuned from its own debug window
//! ("Aether Shroud").
//!
//! One fullscreen post-process pass (`assets/shaders/aether_shroud.wgsl`) on
//! the [`ViewModelCamera`], exactly like the shroom and drunk passes and
//! chained right after them (tonemapping → shroom → drunk → aether), so it
//! grades whatever they've already done to the frame. It only runs while
//! the effect is faded in.

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

use super::drunk::DrunkLabel;
use crate::player::ViewModelCamera;

const SHADER_ASSET_PATH: &str = "shaders/aether_shroud.wgsl";

/// Panel-adjustable Aether Shroud look (the "Aether Shroud" debug window).
#[derive(Resource, Clone)]
pub(crate) struct AetherScreenSettings {
    /// Debug: show it without the Aether Shroud up.
    pub(crate) preview: bool,
    /// Seconds to fade fully in as it goes up...
    pub(crate) fade_in_secs: f32,
    /// ...and out as it wears off.
    pub(crate) fade_out_secs: f32,
    /// sRGB colours the frame's brightness is mapped onto: its darks...
    pub(crate) dark: [f32; 3],
    /// ...mids...
    pub(crate) mid: [f32; 3],
    /// ...and highlights.
    pub(crate) light: [f32; 3],
    /// How much of that colour grade replaces the real colours (0..=1).
    pub(crate) tint: f32,
    /// Brightening before the grade (1 = none)...
    pub(crate) exposure: f32,
    /// ...and its curve (< 1 lifts the darks, washing it out).
    pub(crate) gamma: f32,
    /// Fisheye warp toward the centre (0 = none).
    pub(crate) warp: f32,
    /// Colour fringing toward the edges.
    pub(crate) chromatic: f32,
    /// The light purple haze at the edges (0..=1).
    pub(crate) edge_glow: f32,
    /// The edges' wavy shimmer (fraction of screen height) and its speed.
    pub(crate) shimmer: f32,
    pub(crate) shimmer_speed: f32,
    /// Right as it goes up: extra warp on top of `warp`...
    pub(crate) kick_warp: f32,
    /// ...a brightening flash...
    pub(crate) kick_flash: f32,
    /// ...both easing away over this long (s).
    pub(crate) kick_secs: f32,
    /// Lightning from the screen's edges: seconds between bolts (a random
    /// gap in this range)...
    pub(crate) bolt_gap: (f32, f32),
    /// ...how many flick in together as it goes up...
    pub(crate) bolt_burst: u32,
    /// ...how long (s) each one's there...
    pub(crate) bolt_life: f32,
    /// ...how far in it reaches (fraction of screen height, a random length
    /// in this range)...
    pub(crate) bolt_length: (f32, f32),
    /// ...how jagged it is (sideways, fraction of its length)...
    pub(crate) bolt_jag: f32,
    /// ...its core's thickness and its glow's (fraction of screen height)...
    pub(crate) bolt_width: f32,
    pub(crate) bolt_glow: f32,
    /// ...how bright (0 = no bolts)...
    pub(crate) bolt_brightness: f32,
    /// ...and its glow's sRGB colour (the core's white).
    pub(crate) bolt_color: [f32; 3],
}

impl Default for AetherScreenSettings {
    fn default() -> Self {
        Self {
            preview: false,
            fade_in_secs: 0.25,
            fade_out_secs: 0.6,
            dark: [0.052, 0.047, 0.22],
            mid: [0.541, 0.51, 1.0],
            light: [1.0, 1.0, 1.0],
            tint: 0.54,
            exposure: 2.4,
            gamma: 0.73,
            warp: 0.48,
            chromatic: 0.026,
            edge_glow: 0.35,
            shimmer: 0.014,
            shimmer_speed: 1.6,
            kick_warp: 0.76,
            kick_flash: 2.5,
            kick_secs: 0.8,
            bolt_gap: (0.15, 0.5),
            bolt_burst: 3,
            bolt_life: 0.14,
            bolt_length: (0.35, 0.6),
            bolt_jag: 0.13,
            bolt_width: 0.0062,
            bolt_glow: 0.009,
            bolt_brightness: 2.6,
            bolt_color: [0.55, 0.6, 1.0],
        }
    }
}

impl AetherScreenSettings {
    /// These settings as Rust, to paste over [`Default`] (the debug
    /// window's "Print settings").
    pub(crate) fn to_rust(&self) -> String {
        format!(
            "AetherScreenSettings {{\n    preview: false,\n    fade_in_secs: {:.2},\n    fade_out_secs: {:.2},\n    \
             dark: {:?},\n    mid: {:?},\n    light: {:?},\n    tint: {:.3},\n    exposure: {:.3},\n    \
             gamma: {:.3},\n    warp: {:.3},\n    chromatic: {:.4},\n    edge_glow: {:.3},\n    \
             shimmer: {:.4},\n    shimmer_speed: {:.2},\n    kick_warp: {:.3},\n    kick_flash: {:.3},\n    \
             kick_secs: {:.2},\n    bolt_gap: ({:.2}, {:.2}),\n    bolt_burst: {},\n    bolt_life: {:.3},\n    \
             bolt_length: ({:.3}, {:.3}),\n    bolt_jag: {:.3},\n    bolt_width: {:.4},\n    bolt_glow: {:.4},\n    \
             bolt_brightness: {:.3},\n    bolt_color: {:?},\n}}",
            self.fade_in_secs,
            self.fade_out_secs,
            round3(self.dark),
            round3(self.mid),
            round3(self.light),
            self.tint,
            self.exposure,
            self.gamma,
            self.warp,
            self.chromatic,
            self.edge_glow,
            self.shimmer,
            self.shimmer_speed,
            self.kick_warp,
            self.kick_flash,
            self.kick_secs,
            self.bolt_gap.0,
            self.bolt_gap.1,
            self.bolt_burst,
            self.bolt_life,
            self.bolt_length.0,
            self.bolt_length.1,
            self.bolt_jag,
            self.bolt_width,
            self.bolt_glow,
            self.bolt_brightness,
            round3(self.bolt_color),
        )
    }
}

/// A colour's channels to 3 decimals, for printing.
pub(crate) fn round3(c: [f32; 3]) -> [f32; 3] {
    c.map(|x| (x * 1000.0).round() / 1000.0)
}

/// The most lightning bolts on screen at once.
pub(crate) const BOLTS: usize = 4;

/// One lightning bolt on screen.
#[derive(Clone, Copy, Default)]
struct Bolt {
    /// Where it comes in from (centred, aspect-corrected screen space: y
    /// down, ±0.5 top to bottom), the way it heads (radians) and how far.
    start: Vec2,
    angle: f32,
    length: f32,
    /// Its shape.
    seed: f32,
    /// Seconds it's been there (`None` = free slot).
    age: Option<f32>,
}

/// Whether our Aether Shroud is up right now (set by
/// `aether_shroud::sync_aether_shroud`).
#[derive(Resource, Default, PartialEq)]
pub(crate) struct AetherScreen(pub(crate) bool);

/// How far the effect is faded in (0..=1), and seconds since it last went up
/// (for the kick).
#[derive(Resource, Default)]
struct AetherLevel {
    level: f32,
    was_on: bool,
    since_on: f32,
    /// The lightning on screen, seconds to the next bolt, and a counter for
    /// the random rolls.
    bolts: [Bolt; BOLTS],
    next_bolt: f32,
    rolls: u32,
}

impl AetherLevel {
    fn roll(&mut self) -> f32 {
        self.rolls = self.rolls.wrapping_add(1);
        crate::util::rand01(self.rolls.wrapping_mul(2_654_435_761))
    }

    /// A new bolt in a free slot (or the oldest), from a random spot just
    /// off one of the screen's edges (`aspect` wide), heading roughly for
    /// the middle.
    fn spawn_bolt(&mut self, s: &AetherScreenSettings, aspect: f32) {
        let (hw, hh) = (aspect * 0.5, 0.5);
        let side = (self.roll() * 4.0) as u32;
        let along = self.roll() * 2.0 - 1.0;
        let start = match side {
            0 => Vec2::new(along * hw, -hh - 0.02),
            1 => Vec2::new(along * hw, hh + 0.02),
            2 => Vec2::new(-hw - 0.02, along * hh),
            _ => Vec2::new(hw + 0.02, along * hh),
        };
        let to_middle = (-start).to_angle();
        let angle = to_middle + (self.roll() - 0.5) * 1.2;
        let length = s.bolt_length.0 + (s.bolt_length.1 - s.bolt_length.0) * self.roll();
        let seed = self.roll() * 100.0;
        let slot = self
            .bolts
            .iter()
            .position(|b| b.age.is_none())
            .or_else(|| {
                (0..BOLTS).max_by(|a, b| {
                    self.bolts[*a].age.unwrap_or(0.0).total_cmp(&self.bolts[*b].age.unwrap_or(0.0))
                })
            })
            .unwrap_or(0);
        self.bolts[slot] = Bolt {
            start,
            angle,
            length,
            seed,
            age: Some(0.0),
        };
    }
}

pub(crate) use uniform::AetherUniform;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct.
#[allow(dead_code)]
mod uniform {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    #[derive(Component, Default, Clone, Copy, ShaderType)]
    pub(crate) struct AetherUniform {
        pub(crate) dark: Vec4,
        pub(crate) mid: Vec4,
        pub(crate) light: Vec4,
        pub(crate) strength: f32,
        pub(crate) time: f32,
        pub(crate) tint: f32,
        pub(crate) exposure: f32,
        pub(crate) gamma: f32,
        /// Final warp (fade and kick already in).
        pub(crate) warp: f32,
        pub(crate) chromatic: f32,
        pub(crate) edge_glow: f32,
        pub(crate) shimmer: f32,
        pub(crate) shimmer_speed: f32,
        pub(crate) flash: f32,
        /// Lightning: per bolt, (start x, start y, heading, length) and
        /// (shape seed, brightness — 0 = none, jag, unused).
        pub(crate) bolt_a: [Vec4; super::BOLTS],
        pub(crate) bolt_b: [Vec4; super::BOLTS],
        /// Core and glow thickness, then the glow's linear colour.
        pub(crate) bolt_width: f32,
        pub(crate) bolt_glow: f32,
        pub(crate) bolt_color: Vec4,
    }
}

impl ExtractComponent for AetherUniform {
    type QueryData = &'static AetherUniform;
    type QueryFilter = ();
    type Out = AetherUniform;

    fn extract_component(item: QueryItem<'_, Self::QueryData>) -> Option<Self::Out> {
        (item.strength > 0.0).then_some(*item)
    }
}

/// Must be added after `DrunkPlugin` (its node is chained after the drunk
/// one).
pub(crate) struct AetherScreenPlugin;

impl Plugin for AetherScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AetherScreenSettings>()
            .init_resource::<AetherLevel>()
            .init_resource::<AetherScreen>()
            .add_plugins((
                ExtractComponentPlugin::<AetherUniform>::default(),
                UniformComponentPlugin::<AetherUniform>::default(),
            ))
            .add_systems(Update, sync_aether_screen);

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_render_graph_node::<ViewNodeRunner<AetherNode>>(Core3d, AetherLabel)
            .add_render_graph_edges(
                Core3d,
                (DrunkLabel, AetherLabel, Node3d::EndMainPassPostProcessing),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<AetherPipeline>();
    }
}

/// The sRGB `c` as a linear `Vec4` for the shader.
fn linear(c: [f32; 3]) -> Vec4 {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    Vec4::new(l.red, l.green, l.blue, 1.0)
}

/// Fades [`AetherLevel`] toward on / off, runs the kick, and copies the
/// settings onto the view-model camera's [`AetherUniform`] (added the first
/// time a camera shows up — the cameras are respawned each game).
fn sync_aether_screen(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<AetherScreenSettings>,
    screen: Res<AetherScreen>,
    mut level: ResMut<AetherLevel>,
    new_cams: Query<Entity, (With<ViewModelCamera>, Without<AetherUniform>)>,
    mut uniforms: Query<(&mut AetherUniform, &Camera), With<ViewModelCamera>>,
) {
    for cam in &new_cams {
        commands.entity(cam).insert(AetherUniform::default());
    }

    let s = &*settings;
    let dt = time.delta_secs();
    let on = s.preview || screen.0;
    let aspect = uniforms
        .iter()
        .find_map(|(_, cam)| cam.logical_viewport_size())
        .map_or(16.0 / 9.0, |v| v.x / v.y.max(1.0));
    if on && !level.was_on {
        level.since_on = 0.0;
        // A burst of bolts as it goes up.
        for _ in 0..s.bolt_burst.min(BOLTS as u32) {
            level.spawn_bolt(s, aspect);
        }
        level.next_bolt = s.bolt_gap.0;
    }
    level.was_on = on;
    level.since_on += dt;
    let (target, secs) = if on { (1.0, s.fade_in_secs) } else { (0.0, s.fade_out_secs) };
    level.level = if secs <= 0.0 {
        target
    } else {
        let step = dt / secs;
        level.level + (target - level.level).clamp(-step, step)
    };
    let fade = level.level * level.level * (3.0 - 2.0 * level.level);
    // The kick: full right as it goes up, easing away.
    let kick = if on && s.kick_secs > 0.0 {
        let x = (1.0 - level.since_on / s.kick_secs).clamp(0.0, 1.0);
        x * x
    } else {
        0.0
    };

    // Lightning: age the bolts, and every so often another, while it's up.
    for b in level.bolts.iter_mut() {
        if let Some(age) = &mut b.age {
            *age += dt;
            if *age > s.bolt_life {
                b.age = None;
            }
        }
    }
    if on && s.bolt_brightness > 0.0 {
        level.next_bolt -= dt;
        if level.next_bolt <= 0.0 {
            level.spawn_bolt(s, aspect);
            let r = level.roll();
            level.next_bolt = s.bolt_gap.0 + (s.bolt_gap.1 - s.bolt_gap.0).max(0.0) * r;
        }
    } else if !on {
        level.bolts = Default::default();
    }
    let mut bolt_a = [Vec4::ZERO; BOLTS];
    let mut bolt_b = [Vec4::ZERO; BOLTS];
    for (i, b) in level.bolts.iter().enumerate() {
        let Some(age) = b.age else { continue };
        // A hard flicker: full on, a dip, back on, then gone.
        let t = age / s.bolt_life.max(1e-3);
        let flicker = if (0.35..0.55).contains(&t) { 0.35 } else { 1.0 };
        bolt_a[i] = Vec4::new(b.start.x, b.start.y, b.angle, b.length);
        bolt_b[i] = Vec4::new(b.seed, s.bolt_brightness * flicker, s.bolt_jag, 0.0);
    }

    for (mut u, _) in &mut uniforms {
        *u = AetherUniform {
            dark: linear(s.dark),
            mid: linear(s.mid),
            light: linear(s.light),
            strength: fade,
            time: time.elapsed_secs_wrapped(),
            tint: s.tint,
            exposure: s.exposure,
            gamma: s.gamma,
            warp: (s.warp + s.kick_warp * kick) * fade,
            chromatic: s.chromatic,
            edge_glow: s.edge_glow,
            shimmer: s.shimmer,
            shimmer_speed: s.shimmer_speed,
            flash: s.kick_flash * kick,
            bolt_a,
            bolt_b,
            bolt_width: s.bolt_width,
            bolt_glow: s.bolt_glow,
            bolt_color: linear(s.bolt_color),
        };
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct AetherLabel;

#[derive(Default)]
struct AetherNode;

impl ViewNode for AetherNode {
    // (The uniform itself too — see `DrunkNode`: without it the pass keeps
    // running on a stale index once the effect's off.)
    type ViewQuery = (
        &'static ViewTarget,
        &'static DynamicUniformIndex<AetherUniform>,
        &'static AetherUniform,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, uniform_index, _): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let aether_pipeline = world.resource::<AetherPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let Some(pipeline) = pipeline_cache.get_render_pipeline(aether_pipeline.pipeline_id) else {
            return Ok(());
        };
        let uniforms = world.resource::<ComponentUniforms<AetherUniform>>();
        let Some(uniform_binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };

        let post_process = view_target.post_process_write();
        let bind_group = render_context.render_device().create_bind_group(
            "aether_shroud_bind_group",
            &aether_pipeline.layout,
            &BindGroupEntries::sequential((
                post_process.source,
                &aether_pipeline.sampler,
                uniform_binding,
            )),
        );
        let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("aether_shroud_pass"),
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
struct AetherPipeline {
    layout: BindGroupLayout,
    sampler: Sampler,
    pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for AetherPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let layout = render_device.create_bind_group_layout(
            "aether_shroud_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<AetherUniform>(true),
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
                    label: Some("aether_shroud_pipeline".into()),
                    layout: vec![layout.clone()],
                    vertex: fullscreen_shader_vertex_state(),
                    fragment: Some(FragmentState {
                        shader,
                        shader_defs: vec![],
                        entry_point: "fragment".into(),
                        // Same HDR target as the shroom and drunk passes.
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
