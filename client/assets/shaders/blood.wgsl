// Blood: liquid lying on a surface, lit like the rest of the map (so the
// candles and torches glint in it) and alpha blended. Two shapes:
//
// * a pool (`mode` 0) on a flat quad, its uv spanning it: spreading out from
//   where it started (`start`) as `spread` grows — not a circle, but a front
//   that runs on faster in some directions than others (a noise field
//   thresholded, so it creeps out unevenly, fingering, with a few droplets
//   ahead of it), thin and lighter at its edge where it's a film, dark and
//   deep in the middle, its surface curving down at the edge (a meniscus,
//   catching the light). Optionally held inside a circle (`clip` — a basin);
// * a run (`mode` 1) down a strip laid along its path, uv (across it, metres
//   along it): a stream of wandering width that's reached `reach` m along,
//   its front a rounded bead, its surface rippling as it flows.
//
// `glow` lights it from inside (the ritual taking hold), a slow churn of
// red. Driven by `BloodMaterial` in `client/src/easter_egg/ritual_fx.rs`.

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

struct BloodParams {
    // How far it's spread (a pool: in its quad's -1..1 units).
    spread: f32,
    // Keeps one pool from looking like the next.
    seed: f32,
    // How much it glows (0 none).
    glow: f32,
    // 0 a pool, 1 a run.
    mode: f32,
    // Where a pool starts (its quad's -1..1).
    start: vec2<f32>,
    // A pool held in a circle this big (-1..1 units; 0 = not held).
    clip: f32,
    // How far (m) a run's got along its strip.
    reach: f32,
    // How big (m) a pool's quad is from its middle to its side — so its
    // noise is in metres, the same size on any quad.
    size: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}
@group(2) @binding(100) var<uniform> blood: BloodParams;

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

// Three octaves, 0..1.
fn fbm(p: vec2<f32>) -> f32 {
    return vnoise(p) * 0.55 + vnoise(p * 2.03 + 17.0) * 0.3 + vnoise(p * 4.1 - 9.0) * 0.15;
}

// The surface's own directions in the world: along u, and along v.
fn uv_frame(world: vec3<f32>, uv: vec2<f32>) -> mat2x3<f32> {
    let dp1 = dpdx(world);
    let dp2 = dpdy(world);
    let du1 = dpdx(uv);
    let du2 = dpdy(uv);
    let det = du1.x * du2.y - du1.y * du2.x;
    if (abs(det) < 1e-12) {
        return mat2x3<f32>(vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0));
    }
    let t = (dp1 * du2.y - dp2 * du1.y) / det;
    let b = (dp2 * du1.x - dp1 * du2.x) / det;
    return mat2x3<f32>(normalize(t), normalize(b));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let t = globals.time;
    let seed = blood.seed;
    let frame = uv_frame(in.world_position.xyz, in.uv);

    // How much of it's here (alpha), how thick (0 a film .. 1 deep), and
    // which way (along u, v) its surface slopes down — to its edge.
    var cover = 0.0;
    var deep = 0.0;
    var slope = vec2(0.0);
    var churn = vec2(0.0);

    if (blood.mode < 0.5) {
        // --- a pool ---
        let p = (in.uv - 0.5) * 2.0;
        let q = p - blood.start;
        let m = q * blood.size;
        let d = length(q);
        // The front: how far each spot is "for the blood", the distance
        // bent by noise in metres — so it runs on ahead in some directions
        // and holds back in others, and creeps on unevenly as it spreads.
        let lobes = fbm(m * 2.6 + seed);
        let fine = vnoise(m * 11.0 - seed * 1.3);
        let f = d * (0.62 + 0.75 * lobes + 0.12 * fine);
        let s = blood.spread;
        let soft = 0.012 / max(blood.size, 0.05);
        cover = 1.0 - smoothstep(s - soft, s, f);
        deep = smoothstep(0.0, max(0.25 * s, 0.02), s - f);
        // Droplets thrown out just ahead of it.
        let spots = vnoise(m * 26.0 + seed * 3.7);
        let ahead = smoothstep(s, s * 1.04, f) * (1.0 - smoothstep(s * 1.15, s * 1.3, f));
        let drop = smoothstep(0.86, 0.9, spots) * ahead * step(0.08, s);
        cover = max(cover, drop * 0.9);
        // Held in its basin.
        if (blood.clip > 0.0) {
            let held = 1.0 - smoothstep(blood.clip - 0.01, blood.clip, length(p));
            cover *= held;
            // (Deep right to its wall once it's full.)
            deep = max(deep * held, held * smoothstep(1.0, 1.6, s));
        }
        // Down at the edge, outwards.
        let out_dir = select(vec2(0.0), q / max(d, 1e-4), d > 1e-4);
        slope = out_dir * (1.0 - deep) * 0.9;
        churn = m * 3.0;
    } else {
        // --- a run ---
        let y = in.uv.y;
        let x = in.uv.x - 0.5;
        // Its width wanders along it.
        let half = 0.5 * (0.5 + 0.45 * vnoise(vec2(y * 6.0, seed)) + 0.05 * vnoise(vec2(y * 31.0, seed + 4.0)));
        let across = abs(x) / half;
        // Its front: a rounded bead.
        let front = blood.reach + 0.025 * (1.0 - clamp(across * across, 0.0, 1.0));
        cover = (1.0 - smoothstep(0.85, 1.0, across)) * (1.0 - smoothstep(front - 0.008, front, y));
        cover *= step(0.001, blood.reach);
        deep = 1.0 - smoothstep(0.2, 1.0, across);
        // (Bulging at the bead.)
        deep = max(deep, (1.0 - smoothstep(0.0, 0.05, front - y)) * (1.0 - across));
        slope = vec2(sign(x) * smoothstep(0.4, 1.0, across) * 0.8, 0.0);
        // Ripples flowing down it.
        churn = vec2(x * 8.0, y * 5.0 - t * 2.5);
    }

    if (cover < 0.004) {
        discard;
    }

    // Thick blood is near black-red; a thin film shows a brighter red.
    let thin_col = vec3(0.32, 0.012, 0.02);
    let deep_col = vec3(0.075, 0.0, 0.004);
    let tint = pbr_input.material.base_color.rgb;
    pbr_input.material.base_color = vec4(mix(thin_col, deep_col, deep) * tint, cover * mix(0.78, 0.97, deep));
    // Glossy and wet — a touch rougher where it's a film drying on stone.
    pbr_input.material.perceptual_roughness = mix(0.22, 0.05, deep);

    // Its surface: the meniscus, and a slow wobble as it settles / flows.
    let n1 = vnoise(churn + vec2(t * 0.35, -t * 0.2));
    let n2 = vnoise(churn + vec2(0.37, 0.0) + vec2(t * 0.35, -t * 0.2));
    let n3 = vnoise(churn + vec2(0.0, 0.37) + vec2(t * 0.35, -t * 0.2));
    let wobble = vec2(n2 - n1, n3 - n1) * 0.35;
    let tilt = slope + wobble;
    pbr_input.N = normalize(pbr_input.N - (frame[0] * tilt.x + frame[1] * tilt.y) * 0.5);

    // The ritual taking hold: a glow churning slowly inside it.
    if (blood.glow > 0.0) {
        let inner = fbm(churn * 0.6 + vec2(t * 0.15, t * 0.1) + seed);
        let g = blood.glow * (0.45 + 0.55 * inner) * mix(0.35, 1.0, deep);
        pbr_input.material.emissive = vec4(vec3(1.0, 0.04, 0.03) * g * 6.0, 1.0);
    }

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
