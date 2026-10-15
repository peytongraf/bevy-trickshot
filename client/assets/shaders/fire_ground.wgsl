// The burning ground under a molotov's fire: a mesh laid over the floor
// where it burns, its vertices carrying how far into the fire they are
// (`uv.x`, 1 well inside .. 0 past the edge) — so the edge follows the
// fire's own ragged shape, not a circle. Glowing, flickering heat crawls
// across it (rotated-octave gradient noise, the second layer warped by the
// first), its edge eaten into by the same noise, coloured like the flames
// (deep red where it's cooling, through orange, to a pale yellow core) and
// drawn additively in HDR so the bloom catches it. The flames stand on it
// (`fire.wgsl`); it's what joins them up into one fire.
//
// Driven by `FireGroundMaterial` in `client/src/molotov.rs`.

#import bevy_pbr::{
    mesh_functions,
    view_transformations::position_world_to_clip,
    mesh_view_bindings::globals,
}

struct GroundParams {
    // Linear rgb of the hottest part, the body, and the cooling edges.
    core: vec4<f32>,
    mid: vec4<f32>,
    tip: vec4<f32>,
    brightness: f32,
    // How fast the heat crawls.
    speed: f32,
    // Noise frequency (per m) — higher = smaller, busier patches.
    detail: f32,
    // 0..1 overall strength (fading in / out).
    fade: f32,
    // How much the noise eats into the edge (0 = smooth).
    ragged: f32,
}
@group(2) @binding(0) var<uniform> p: GroundParams;

struct GroundVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    // x: how far into the fire (0..1), y: this patch's seed.
    @location(2) uv: vec2<f32>,
}

struct GroundOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

// (The same noise as `fire.wgsl`'s.)
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn unit(h: u32) -> f32 {
    return f32(h) / 4294967295.0;
}

fn grad(i: vec2<i32>) -> vec2<f32> {
    let h = pcg(bitcast<u32>(i.x) ^ pcg(bitcast<u32>(i.y) + 0x9E3779B9u));
    let a = unit(h) * 6.2831853;
    return vec2<f32>(cos(a), sin(a));
}

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
fn vertex(v: GroundVertex) -> GroundOut {
    var out: GroundOut;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let world = (world_from_local * vec4<f32>(v.position, 1.0)).xyz;
    out.world = world;
    out.uv = v.uv;
    out.clip = position_world_to_clip(world);
    return out;
}

@fragment
fn fragment(in: GroundOut) -> @location(0) vec4<f32> {
    let cover = clamp(in.uv.x, 0.0, 1.0);
    let s = in.uv.y * 37.0;
    let t = globals.time * p.speed;
    let q = in.world.xz * p.detail + vec2<f32>(s, -s);

    // Big slow drifts of heat, then fine fast flicker warped by them.
    let n1 = fbm(q * 0.5 + vec2<f32>(t * 0.15, -t * 0.35));
    let n2 = fbm(q * 1.3 + vec2<f32>(n1 * 1.5, n1) + vec2<f32>(-t * 0.5, t * 0.25));

    // The edge, eaten into by the noise: tongues of fire reaching out,
    // gaps biting in.
    let edge = smoothstep(0.12, 0.55, cover + (n1 * 0.6 + n2 * 0.4) * p.ragged);
    // Hotter patches and cooler gaps across it, flickering.
    var heat = edge * clamp(0.3 + n2 * 1.4 + n1 * 0.7, 0.0, 1.0);
    heat *= 0.85 + 0.15 * sin(t * 7.0 + n1 * 9.0 + s);
    // Hottest towards the middle of the fire.
    heat *= 0.6 + 0.4 * smoothstep(0.3, 1.0, cover);
    heat = clamp(heat * 1.25, 0.0, 1.0);

    var col = p.tip.rgb * smoothstep(0.0, 0.3, heat);
    col = mix(col, p.mid.rgb, smoothstep(0.25, 0.65, heat));
    col = mix(col, p.core.rgb, smoothstep(0.7, 1.0, heat));
    let rgb = col * smoothstep(0.03, 0.4, heat);

    // Premultiplied, alpha 0: purely additive (`AlphaMode::Add`).
    return vec4<f32>(rgb * p.brightness * p.fade, 0.0);
}
