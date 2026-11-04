// The astral look (our soul out of our body — the easter egg's trip into
// the pyramid): the frame breathing and swirling toward its edges, its
// colours splitting apart there, a slowly turning kaleidoscopic lattice of
// sacred geometry (rings, spokes and a hexagonal web) glimmering in
// shifting iridescent colour round the edges of sight, an iridescent sheen
// over the edges, and a pale glow closing in. `travel` (flying) makes the
// edges race and the view pulse like a tunnel. The middle of the view stays
// clear. Driven by `AstralUniform` in `client/src/vfx/astral.rs`.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct AstralUniform {
    strength: f32,
    time: f32,
    travel: f32,
    warp: f32,
    aberration: f32,
    pattern: f32,
    iridescence: f32,
    glow: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    _pad: f32,
}
@group(0) @binding(2) var<uniform> s: AstralUniform;

const TAU: f32 = 6.28318530718;

fn hue(h: f32) -> vec3<f32> {
    let k = vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    return clamp(abs(fract(h + k) * 6.0 - 3.0) - 1.0, vec3(0.0), vec3(1.0));
}

// A thin bright line where `x` crosses a whole number, `w` wide (of a
// period).
fn line(x: f32, w: f32) -> f32 {
    let d = abs(fract(x) - 0.5);
    return smoothstep(0.5 - w, 0.5, d);
}

// A hexagonal web: bright along the edges of hexagons `scale` across.
fn hex_web(p: vec2<f32>) -> f32 {
    let r = vec2<f32>(1.0, 1.7320508);
    let h = r * 0.5;
    let a = (p - r * floor(p / r)) - h;
    let b = ((p - h) - r * floor((p - h) / r)) - h;
    let g = select(b, a, dot(a, a) < dot(b, b));
    // (Distance to the hexagon's edge.)
    let q = abs(g);
    let edge = 0.5 - max(dot(q, normalize(r)), q.x);
    return 1.0 - smoothstep(0.0, 0.035, edge);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(screen_texture));
    let aspect = vec2<f32>(dims.x / dims.y, 1.0);
    let k = s.strength;
    let t = s.time;
    let travel = s.travel;

    var p = (in.uv - 0.5) * aspect;
    let r0 = length(p);
    // How far out toward the edges (the middle stays clear).
    let edge = smoothstep(0.12, 0.85, r0);

    // Breathing and swirling: the edges turn slowly (faster flying), a
    // ripple runs out through them, and the whole view pulses in and out
    // like a tunnel while we fly.
    let swirl = s.warp * k * edge * edge * (0.08 + 0.1 * travel) * sin(t * (0.45 + 0.6 * travel) + r0 * 5.0);
    let cs = cos(swirl);
    let sn = sin(swirl);
    p = vec2<f32>(p.x * cs - p.y * sn, p.x * sn + p.y * cs);
    p = p * (1.0 + s.warp * k * edge * 0.012 * sin(r0 * 26.0 - t * (2.0 + 4.0 * travel)));
    p = p * (1.0 - travel * k * 0.035 * (0.5 + 0.5 * sin(t * 1.6)));
    let uv = p / aspect + 0.5;

    // The colours splitting apart toward the edges.
    let dir = select(vec2(0.0), p / max(length(p), 1e-4), length(p) > 1e-4);
    let ca = s.aberration * k * (0.15 + edge) * (0.005 + 0.004 * travel) / aspect * dir;
    var col = vec3<f32>(
        textureSample(screen_texture, texture_sampler, uv + ca).r,
        textureSample(screen_texture, texture_sampler, uv).g,
        textureSample(screen_texture, texture_sampler, uv - ca).b,
    );

    // The lattice: the view folded into twelve mirrored slices (turning
    // slowly), rings running in toward the middle, spokes and a hexagonal
    // web — only out toward the edges, flickering in and out.
    let ang = atan2(p.y, p.x) + t * 0.04;
    let seg = TAU / 12.0;
    let a = abs(fract(ang / seg + 0.5) - 0.5) * seg;
    let r = length(p);
    let q = vec2<f32>(cos(a), sin(a)) * r;
    let rings = line(r * 14.0 + t * (0.4 + 2.5 * travel), 0.035);
    let spokes = 1.0 - smoothstep(0.0, 0.004 + 0.004 * r, abs(a - seg * 0.25) * r);
    let web = hex_web(q * 9.0 + vec2(0.0, -t * (0.15 + 0.8 * travel)));
    let shimmer = 0.6 + 0.4 * sin(t * 2.3 + r * 9.0 + a * 20.0);
    let lattice = (rings * 0.45 + spokes * 0.35 + web * 0.6) * shimmer * smoothstep(0.3, 0.75, r);
    let iri = hue(ang / TAU * 2.0 + r * 0.8 - t * 0.06);
    col = col + iri * lattice * s.pattern * k * 0.22;

    // An iridescent sheen over the edges.
    let sheen = hue(r * 1.5 + ang / TAU - t * 0.1) * 1.5;
    col = mix(col, col * sheen, clamp(s.iridescence * k * edge * 0.45, 0.0, 1.0));

    // A pale glow closing in round the edges, breathing.
    let tint = vec3<f32>(s.tint_r, s.tint_g, s.tint_b);
    let breath = 0.75 + 0.25 * sin(t * 1.1);
    col = col + tint * s.glow * k * smoothstep(0.55, 1.15, r0) * breath * 0.35;

    return vec4<f32>(col, 1.0);
}
