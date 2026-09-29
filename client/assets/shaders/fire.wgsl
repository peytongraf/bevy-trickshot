// A flame (or, with `mode` 1, a column of smoke): a camera-facing quad whose
// base sits on the entity's origin, rising along world up (leaning toward the
// camera's own up when seen from steeply above/below).
//
// Flames are drawn additively in HDR so the world camera's bloom catches
// them: two layers of rotated-octave gradient noise scroll upward at
// different speeds (hot gas accelerates as it rises), the second warped by
// the first, displacing and eroding a teardrop profile into licking tongues;
// the result is coloured by "heat" from deep red at the cooling edges through
// orange to a pale yellow core. A few embers drift up off the top. Smoke is
// alpha-blended: a widening, billowing column, lit orange at its base by the
// fire.
//
// Per instance (no per-flame material needed):
// * size — the entity's x / y scale (width / height in metres of world space);
// * seed — the z scale divided by the x scale, minus one (0..1), so it stays
//   put however the flame moves or its parent is scaled;
// * `wind` bends the top of the flame by that many metres per metre of height
//   (the CPU sets it from how fast the flame is moving — it trails behind).
// Driven by `FireMaterial` in `client/src/molotov.rs`.

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::{view, globals},
}

struct FireParams {
    // Linear rgb of the hottest part, the body, and the cooling tips.
    core: vec4<f32>,
    mid: vec4<f32>,
    tip: vec4<f32>,
    // xyz: how far (m) the top leans per metre of height.
    wind: vec4<f32>,
    // Smoke: rgb colour, a = opacity.
    smoke: vec4<f32>,
    brightness: f32,
    // How fast the flame licks upward.
    speed: f32,
    // How much the noise bends and tears the flame (0 = a calm candle).
    turbulence: f32,
    // Noise frequency — higher = smaller, busier tongues.
    detail: f32,
    // 0..1 overall strength (fading in / out).
    fade: f32,
    // How bright the embers are (0 = none).
    embers: f32,
    // How far below the anchor the quad's bottom edge sits (fraction of height).
    sink: f32,
    // How far (m) the quad is pulled toward the camera, so a flame on the
    // ground doesn't slice into it.
    pull: f32,
    // 0 = flame (additive), 1 = smoke (alpha blended).
    mode: f32,
}
@group(2) @binding(0) var<uniform> p: FireParams;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn unit(h: u32) -> f32 {
    return f32(h) / 4294967295.0;
}

// A pseudo-random unit-ish gradient per lattice point.
fn grad(i: vec2<i32>) -> vec2<f32> {
    let h = pcg(bitcast<u32>(i.x) ^ pcg(bitcast<u32>(i.y) + 0x9E3779B9u));
    let a = unit(h) * 6.2831853;
    return vec2<f32>(cos(a), sin(a));
}

// Gradient noise, roughly -0.7..0.7, smooth (quintic fade).
fn gnoise(q: vec2<f32>) -> f32 {
    let fl = floor(q);
    let i = vec2<i32>(fl);
    let f = q - fl;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(grad(i), f);
    let b = dot(grad(i + vec2<i32>(1, 0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(grad(i + vec2<i32>(0, 1)), f - vec2<f32>(0.0, 1.0));
    let d = dot(grad(i + vec2<i32>(1, 1)), f - vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Five octaves, each rotated so no grid lines line up. About -0.6..0.6.
fn fbm(q_in: vec2<f32>) -> f32 {
    var q = q_in;
    var sum = 0.0;
    var amp = 0.55;
    let rot = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    for (var i = 0; i < 5; i++) {
        sum += gnoise(q) * amp;
        q = rot * q * 2.03 + vec2<f32>(3.1, 1.7);
        amp *= 0.5;
    }
    return sum;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let center = (world_from_local * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    let width = length(world_from_local[0].xyz);
    let height = length(world_from_local[1].xyz);
    let depth = length(world_from_local[2].xyz);
    let seed = clamp(depth / max(width, 1e-6) - 1.0, 0.0, 1.0);

    let to_cam_raw = view.world_position - center;
    let dist = max(length(to_cam_raw), 1e-4);
    let to_cam = to_cam_raw / dist;
    let cam_right = view.world_from_view[0].xyz;
    let cam_up = view.world_from_view[1].xyz;
    let up = normalize(mix(cam_up, vec3<f32>(0.0, 1.0, 0.0), 0.7));
    var right = cross(up, to_cam);
    if (length(right) < 1e-3) {
        right = cam_right;
    }
    right = normalize(right);

    let pull = min(p.pull, dist * 0.5);
    let local = vertex.position.xy; // -0.5..0.5
    let rise = local.y + 0.5;       // 0 at the base .. 1 at the top
    var world = center + to_cam * pull
        + right * local.x * width
        + up * (rise - p.sink) * height;
    // Trailing behind the motion: nothing at the base, most at the top.
    world += p.wind.xyz * pow(rise, 1.6) * height;

    out.world_position = vec4<f32>(world, 1.0);
    // x: -1..1 across, y: 0..1 up, z: this flame's seed.
    out.world_normal = vec3<f32>(local.x * 2.0, rise, seed);
    out.position = position_world_to_clip(world);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

fn flame(x: f32, y: f32, seed: f32) -> vec4<f32> {
    let t = globals.time * p.speed;
    let s = seed * 57.0;
    let d = p.detail;
    let turb = p.turbulence;

    // Big slow billows, then fine fast detail warped by them.
    let q1 = vec2<f32>(x * 0.9 * d + s, y * 0.9 * d - t * 2.2);
    let n1 = fbm(q1);
    let q2 = vec2<f32>(x * 1.9 * d - s * 1.3, y * 1.8 * d - t * 3.7)
        + vec2<f32>(n1, n1 * 0.5) * 1.6 * turb;
    let n2 = fbm(q2);

    // Pushed sideways by the noise, more the higher it gets.
    let dx = x + (n1 * 0.6 + n2 * 0.3) * turb * (0.12 + y * 0.95);
    // Teardrop profile: full at the base, a point at the top.
    let w = mix(0.6, 0.06, pow(y, 0.8));
    let r = dx / w;
    let body = exp(-r * r * 1.9);
    // The top is ragged: noise decides where the tongues end.
    let top = 1.0 - smoothstep(0.2, 1.05, y - n2 * 0.45 * turb);
    let bottom = smoothstep(0.0, 0.07, y);
    var heat = body * top * bottom;
    // Inner structure: hotter streaks and cooler gaps.
    heat *= 0.65 + 0.9 * clamp(n2 + 0.5, 0.0, 1.0);
    heat = clamp(heat * 1.3, 0.0, 1.0);

    var col = p.tip.rgb * smoothstep(0.0, 0.3, heat);
    col = mix(col, p.mid.rgb, smoothstep(0.22, 0.6, heat));
    col = mix(col, p.core.rgb, smoothstep(0.6, 0.97, heat));
    var rgb = col * smoothstep(0.02, 0.35, heat);

    // Embers: small specks drifting up and out of the top, each fading out.
    var sparks = 0.0;
    let sid = u32(seed * 65535.0);
    for (var i = 0u; i < 6u; i++) {
        let h = pcg(sid * 16u + i);
        let rate = 0.3 + unit(h) * 0.4;
        let phase = globals.time * rate * p.speed + unit(pcg(h));
        let life = fract(phase);
        let cycle = u32(floor(phase));
        let hx = unit(pcg(h ^ (cycle * 2654435761u)));
        let ex = (hx - 0.5) * 0.8 + sin(life * 5.0 + f32(i)) * 0.12;
        let ey = 0.3 + life * 0.7;
        let e = vec2<f32>((x - ex) * 0.45, y - ey);
        sparks += exp(-dot(e, e) * 12000.0) * sin(life * 3.14159);
    }
    rgb += p.mid.rgb * sparks * p.embers;

    // Premultiplied, alpha 0: purely additive (`AlphaMode::Add`).
    return vec4<f32>(rgb * p.brightness * p.fade, 0.0);
}

fn smoke(x: f32, y: f32, seed: f32) -> vec4<f32> {
    let t = globals.time * p.speed;
    let s = seed * 41.0;
    let d = p.detail * 0.45;
    let q = vec2<f32>(x * 1.1 * d + s, y * 1.2 * d - t * 0.55);
    let warp = fbm(q * 1.6 + vec2<f32>(-s, -t * 0.3));
    let n = fbm(q + vec2<f32>(warp, warp * 0.6) * 1.2);

    // A column that widens and drifts as it rises.
    let w = mix(0.3, 0.95, y);
    let r = (x + n * 0.55 * y) / w;
    var dens = exp(-r * r * 2.2) * smoothstep(0.0, 0.22, y) * (1.0 - smoothstep(0.5, 1.0, y));
    dens *= clamp(0.45 + n * 1.6, 0.0, 1.0);
    let a = clamp(dens * p.smoke.a * p.fade, 0.0, 1.0);
    // Lit by the fire underneath at first, then its own grey.
    let lit = mix(p.mid.rgb * 0.45, p.smoke.rgb, smoothstep(0.05, 0.45, y));
    return vec4<f32>(lit, a);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let x = in.world_normal.x;
    let y = clamp(in.world_normal.y, 0.0, 1.0);
    let seed = in.world_normal.z;
    if (p.mode > 0.5) {
        return smoke(x, y, seed);
    }
    return flame(x, y, seed);
}
