//! The astral look: how the world's seen while our soul's out of our body
//! (the easter egg's trip into the pyramid, `easter_egg::ascension`, which
//! drives it through [`AstralScreen`]) — the frame breathing and swirling at
//! its edges, colours splitting apart toward them, a slowly turning lattice
//! of sacred geometry glimmering in iridescent light round the edges of
//! sight, and a pale glow closing in — all stronger while we're flying
//! (`travel`), the view pulsing like a tunnel, and gentler once we're in the
//! chamber.
//!
//! One fullscreen post-process pass (`assets/shaders/astral.wesl`) on the
//! [`ViewModelCamera`], chained after the Aether Shroud's (tonemapping →
//! shroom → drunk → aether → astral), only run while it's faded in. Its
//! fade's reset on leaving a game (`easter_egg::ascension` puts
//! [`AstralScreen`] back to off).

use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{DynamicUniformIndex, ExtractComponent};
use bevy::render::sync_component::SyncComponent;
use bevy::render::RenderApp;

use crate::player::ViewModelCamera;

const SHADER_ASSET_PATH: &str = "shaders/astral.wesl";

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

    /// Mirrors `AstralUniform` in `astral.wesl`.
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

impl SyncComponent<RenderApp> for AstralUniform {
    // (Its uniform index and pipeline go with it once it stops being
    // extracted.)
    type Target = (Self, DynamicUniformIndex<Self>, super::fullscreen_pass::ViewPassPipeline<Self>);
}

impl ExtractComponent<RenderApp> for AstralUniform {
    type QueryData = &'static AstralUniform;
    type QueryFilter = ();
    type Out = AstralUniform;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        (item.strength > 0.0).then_some(*item)
    }
}

impl super::fullscreen_pass::FullscreenPass for AstralUniform {
    const LABEL: &'static str = "astral";
    const SHADER: &'static str = SHADER_ASSET_PATH;
}

/// Must be added after `AetherScreenPlugin` (its node is chained after that
/// one).
pub(crate) struct AstralPlugin;

impl Plugin for AstralPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AstralScreen>()
            .add_systems(PostUpdate, sync_astral);

        super::fullscreen_pass::add::<AstralUniform, _>(app, super::fullscreen_pass::run::<super::aether_shroud::AetherUniform>);
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
