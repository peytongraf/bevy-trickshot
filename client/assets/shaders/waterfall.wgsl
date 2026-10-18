// A waterfall's falling sheet (`shared::level::Waterfall`), lit like the
// rest of the map (so the torches catch it) and alpha blended. The mesh
// follows the water's arc off the lip; its uv is (across it, how far through
// its fall — by time, not distance). Everything's patterned by time since
// leaving the lip, so streaks travel with the water and stretch out as it
// speeds up. At the lip it's a smooth glassy sheet; lower down it tears into
// separate white, aerated strands, its edges fraying, and it thins out into
// the splash at the bottom (`water_spray.wgsl`, and the pool's own foam).
// Driven by `WaterfallMaterial` in `client/src/water.rs`.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    mesh_view_bindings::globals,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct WaterfallParams {
    // Across the sheet (world), to tilt its normal by the streaks.
    side: vec4<f32>,
    // How long (s) the water takes to fall.
    fall_time: f32,
    // How wide (m) it is at the lip.
    width: f32,
    // How solid the sheet is.
    opacity: f32,
    // Keeps neighbouring waterfalls from looking the same.
    seed: f32,
}
@group(2) @binding(100) var<uniform> fall: WaterfallParams;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Value noise, 0..1.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
        mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x),
        u.y,
    );
}

// Long vertical streaks travelling down with the water, 0..1. `x` is metres
// across, `s` seconds since the lip.
fn streaks(x: f32, s: f32, t: f32) -> f32 {
    let flow = s - t;
    // Columns that wander slowly, broken along their length.
    let a = vnoise(vec2(x * 7.0 + fall.seed, flow * 2.2));
    let b = vnoise(vec2(x * 17.0 - fall.seed, flow * 4.5 + 3.1));
    let c = vnoise(vec2(x * 41.0 + 7.7, flow * 9.0));
    return a * 0.55 + b * 0.3 + c * 0.15;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let t = globals.time;
    let along = clamp(in.uv.y, 0.0, 1.0);
    let s = along * fall.fall_time;
    // (Metres across, as wide as it is there — it spreads as it falls.)
    let spread = 1.0 + 0.25 * along;
    let x = (in.uv.x - 0.5) * fall.width * spread;

    let n = streaks(x, s, t);
    let n2 = streaks(x + 0.013, s, t);
    // Smooth near the lip, torn into strands further down.
    let torn = smoothstep(0.05, 0.85, along);
    let threshold = mix(0.0, 0.52, torn);
    let strands = smoothstep(threshold, threshold + 0.18, n);
    var alpha = mix(0.75, 1.0, n) * strands;
    // Frayed edges, more so lower down.
    let edge_noise = vnoise(vec2(in.uv.y * 9.0 - t * 2.0, fall.seed)) * 0.08;
    let edge = 0.04 + 0.12 * along + edge_noise;
    alpha *= smoothstep(0.0, edge, in.uv.x) * smoothstep(0.0, edge, 1.0 - in.uv.x);
    // In at the lip, and thinning into the splash at the bottom.
    alpha *= smoothstep(0.0, 0.03, along) * (1.0 - smoothstep(0.88, 1.0, along));
    alpha *= fall.opacity;

    // Clear and glassy at the lip, whiter (full of air) as it falls.
    let aerated = clamp(0.35 + along * 0.55 + (n - 0.5) * 0.6, 0.0, 1.0);
    let clear = vec3(0.55, 0.75, 0.78);
    pbr_input.material.base_color = vec4(mix(clear, vec3(0.95, 0.97, 0.98), aerated), alpha);
    pbr_input.material.perceptual_roughness = mix(0.15, 0.45, aerated);

    // The streaks ripple its surface, so highlights run down it.
    let slope = (n2 - n) / 0.013;
    let side = normalize(fall.side.xyz);
    pbr_input.N = normalize(pbr_input.N + side * clamp(slope * 0.02, -0.6, 0.6));

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
