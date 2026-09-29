// The green glow around a `Zombies` power-up drop: a camera-facing quad
// centred on the drop, drawn additively — a soft pulsing core, a wider halo
// broken up by slowly swirling wisps, and a few sparks drifting up through
// it. Pulled toward the camera a little so it doesn't slice into the ground
// under the drop. Driven by `PowerUpGlowMaterial` in
// `client/src/power_ups.rs`.

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::{view, globals},
}

struct GlowParams {
    // Linear rgb; a = brightness.
    color: vec4<f32>,
    // Quad width (m).
    size: f32,
    pulse_speed: f32,
    pulse_amount: f32,
    // 0 = a plain soft glow … 1 = broken right up into wisps.
    swirl: f32,
    swirl_speed: f32,
    // How bright the drifting sparks are (0 = none).
    sparks: f32,
    // How far (m) the quad is pulled toward the camera.
    pull: f32,
    // Core size relative to the halo (bigger = a wider bright middle).
    core: f32,
}
@group(2) @binding(0) var<uniform> g: GlowParams;

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * 0.3183099 + vec3<f32>(0.71, 0.113, 0.419));
    let r = q * 17.0;
    return fract(r.x * r.y * r.z * (r.x + r.y + r.z));
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(
            mix(hash3(i + vec3<f32>(0.0, 0.0, 0.0)), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
            mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x),
            u.y,
        ),
        mix(
            mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
            mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x),
            u.y,
        ),
        u.z,
    );
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let center = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
    ).xyz;
    // Face the camera: the view's own right / up axes.
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let to_cam = view.world_position - center;
    let dist = max(length(to_cam), 1e-3);
    // (Never pulled past the camera itself.)
    let pull = min(g.pull, dist * 0.5);
    let world = center + to_cam / dist * pull
        + (right * vertex.position.x + up * vertex.position.y) * g.size;

    out.world_position = vec4<f32>(world, 1.0);
    // The quad's own -1..1 coordinates, for the fragment shader.
    out.world_normal = vec3<f32>(vertex.position.xy * 2.0, 0.0);
    out.position = position_world_to_clip(world);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_normal.xy;
    let r = length(p);
    let t = globals.time;

    // Soft bright middle and a wider halo.
    let core = exp(-r * r / max(g.core * g.core, 1e-3) * 1.5);
    let halo = exp(-r * r * 3.0);

    // Wisps: noise swirling round the middle and drifting outward.
    let ang = atan2(p.y, p.x);
    let swirl_t = t * g.swirl_speed;
    let polar = vec3<f32>(cos(ang + swirl_t * 0.5) * 1.7, sin(ang + swirl_t * 0.5) * 1.7, r * 3.0 - swirl_t);
    let n = value_noise(polar) * 0.65 + value_noise(polar * 2.1 + vec3<f32>(3.1, 1.7, swirl_t * 0.7)) * 0.35;
    let wisps = mix(1.0, smoothstep(0.25, 0.85, n) * 1.6, g.swirl);

    // A few sparks drifting up through it.
    var sparks = 0.0;
    for (var i = 0; i < 7; i++) {
        let fi = f32(i);
        let speed = 0.25 + hash3(vec3<f32>(fi, 1.0, 7.0)) * 0.35;
        let life = fract(hash3(vec3<f32>(fi, 3.0, 1.0)) + t * speed);
        let x = (hash3(vec3<f32>(fi, 5.0, floor(hash3(vec3<f32>(fi, 3.0, 1.0)) + t * speed))) - 0.5) * 0.9;
        let s = vec2<f32>(x + sin(t * 1.3 + fi) * 0.05, life * 1.5 - 0.75);
        let d = p - s;
        let fade = sin(life * 3.14159);
        sparks += exp(-dot(d, d) * 900.0) * fade;
    }

    let pulse = 1.0 + g.pulse_amount * sin(t * g.pulse_speed);
    // Nothing at the quad's edge.
    let edge = 1.0 - smoothstep(0.7, 1.0, r);
    let a = (core * 1.3 + halo * 0.8 * wisps) * pulse * edge + sparks * g.sparks;
    let col = g.color.rgb * g.color.a * a;
    // Premultiplied, alpha 0: purely additive (`AlphaMode::Add`).
    return vec4<f32>(col, 0.0);
}
