// The Aether Shroud's screen effect (a `Zombies` field upgrade), after Cold
// War's: the whole view goes a bright, icy blue-purple — darks deep blue,
// mids lavender, highlights blown out toward white — with a fisheye warp
// pulling in toward the centre, colour fringing and a light, shimmering
// purple haze around the edges. A stronger warp and flash right as it goes
// up, and lightning bolts flicking in from the edges (placed and timed on
// the CPU). One fullscreen pass, 3 texture taps, chained after the drunk
// pass.
// Driven by `AetherUniform` in `client/src/vfx/aether_shroud.rs`.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct AetherUniform {
    // Linear colours the frame's brightness is mapped onto.
    dark: vec4<f32>,
    mid: vec4<f32>,
    light: vec4<f32>,
    // 0..=1 fade.
    strength: f32,
    time: f32,
    tint: f32,
    exposure: f32,
    gamma: f32,
    // Final warp (fade and the kick already in).
    warp: f32,
    chromatic: f32,
    edge_glow: f32,
    shimmer: f32,
    shimmer_speed: f32,
    // The flash right as it goes up (0 once settled).
    flash: f32,
    // Per bolt: (start x, start y, heading, length) and (shape seed,
    // brightness — 0 = none, jag, unused), in the centred, aspect-corrected
    // space `p` below.
    bolt_a: array<vec4<f32>, 4>,
    bolt_b: array<vec4<f32>, 4>,
    bolt_width: f32,
    bolt_glow: f32,
    bolt_color: vec4<f32>,
}
@group(0) @binding(2) var<uniform> s: AetherUniform;

fn hash11(x: f32) -> f32 {
    return fract(sin(x * 127.1) * 43758.5453);
}

// Piecewise-linear value noise in -1..1 — straight runs with sharp kinks,
// like lightning.
fn vnoise(x: f32) -> f32 {
    let i = floor(x);
    return mix(hash11(i), hash11(i + 1.0), fract(x)) * 2.0 - 1.0;
}

// A bolt's sideways wander at `x` (0..1 along it): a few octaves of kinks.
fn jag(x: f32, seed: f32) -> f32 {
    var d = 0.0;
    var amp = 0.5;
    var freq = 5.0;
    for (var o = 0; o < 4; o = o + 1) {
        d = d + vnoise(x * freq + seed * (f32(o) + 1.0) * 7.3) * amp;
        amp = amp * 0.5;
        freq = freq * 2.1;
    }
    return d;
}

// One jagged stroke from `start` heading `dir` for `len`: (core, glow) at `q`.
fn stroke(q: vec2<f32>, start: vec2<f32>, dir: vec2<f32>, len: f32, seed: f32, jag_amt: f32) -> vec2<f32> {
    let rel = q - start;
    let x = dot(rel, dir) / len;
    if (x < 0.0 || x > 1.0) {
        return vec2<f32>(0.0);
    }
    let perp = vec2<f32>(-dir.y, dir.x);
    let d = abs(dot(rel, perp) - jag(x, seed) * jag_amt * len);
    // Thinner and fading toward its tip.
    let taper = 1.0 - 0.7 * x;
    let fade = 1.0 - smoothstep(0.65, 1.0, x);
    let core = (1.0 - smoothstep(0.0, s.bolt_width * taper, d)) * fade;
    let glow = exp(-d / max(s.bolt_glow * taper, 1e-5)) * fade;
    return vec2<f32>(core, glow);
}

// Every bolt (and its two forks) at `q`: (core, glow).
fn lightning(q: vec2<f32>) -> vec2<f32> {
    var total = vec2<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let a = s.bolt_a[i];
        let b = s.bolt_b[i];
        if (b.y <= 0.0) {
            continue;
        }
        let start = a.xy;
        let dir = vec2<f32>(cos(a.z), sin(a.z));
        let perp = vec2<f32>(-dir.y, dir.x);
        let len = a.w;
        let seed = b.x;
        var lit = stroke(q, start, dir, len, seed, b.z);
        // Two forks off the main stroke, each a shorter bolt of its own.
        for (var f = 0; f < 2; f = f + 1) {
            let fs = seed + f32(f) * 3.7 + 1.3;
            let at = 0.25 + 0.45 * hash11(fs);
            let fork_start = start + dir * at * len + perp * jag(at, seed) * b.z * len;
            let side = select(-1.0, 1.0, f == 0);
            let turn = a.z + side * (0.35 + 0.5 * hash11(fs + 0.5));
            let fork = stroke(q, fork_start, vec2<f32>(cos(turn), sin(turn)), len * (1.0 - at) * 0.6, fs, b.z) * 0.7;
            lit = max(lit, fork);
        }
        total = total + lit * b.y;
    }
    return total;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(screen_texture));
    let aspect = vec2<f32>(dims.x / dims.y, 1.0);
    let k = s.strength;

    // Centred, aspect-corrected; `r` is ~1 at the corners.
    let p = (in.uv - 0.5) * aspect;
    let r = length(p) / length(0.5 * aspect);
    let edge = smoothstep(0.35, 1.0, r);

    // Fisheye warp toward the centre (always sampling inside the frame), and
    // a slow shimmer that only shows toward the edges.
    var q = p * (1.0 - s.warp * r * r);
    let t = s.time * s.shimmer_speed;
    q = q + vec2<f32>(sin(q.y * 18.0 + t), cos(q.x * 14.0 - t * 1.3)) * (s.shimmer * k * edge);

    // Colour fringing, growing toward the edges.
    let ca = s.chromatic * k * r;
    let red = textureSample(screen_texture, texture_sampler, q * (1.0 + ca) / aspect + 0.5).r;
    let mid_tap = textureSample(screen_texture, texture_sampler, q / aspect + 0.5);
    let blue = textureSample(screen_texture, texture_sampler, q * (1.0 - ca) / aspect + 0.5).b;
    let col = vec3<f32>(red, mid_tap.g, blue);

    // Brightness → deep blue / lavender / near-white.
    let l = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    let e = clamp(pow(max(l * (s.exposure + s.flash), 0.0), s.gamma), 0.0, 1.0);
    var graded = mix(s.dark.rgb, s.mid.rgb, smoothstep(0.0, 0.45, e));
    graded = mix(graded, s.light.rgb, smoothstep(0.45, 1.0, e));
    var res = mix(col, graded, clamp(s.tint * k, 0.0, 1.0));

    // A light purple haze closing in from the edges.
    res = mix(res, s.mid.rgb * 1.2, clamp(edge * edge * s.edge_glow * k, 0.0, 1.0));
    // The flash as it goes up.
    res = res + s.light.rgb * (s.flash * 0.25 * k);

    // Lightning, in screen space (over the warp): a white-hot core in a
    // coloured glow.
    let bolt = lightning(p);
    res = res + vec3<f32>(1.0) * bolt.x + s.bolt_color.rgb * (bolt.y * 0.8);

    return vec4<f32>(res, 1.0);
}
