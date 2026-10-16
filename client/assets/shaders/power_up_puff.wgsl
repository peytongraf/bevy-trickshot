// One puff off a `Zombies` power-up drop, Cold War style: a little burst of
// green fire popping off the model's surface — a lumpy ball, white-hot in
// the middle and green round the edges, swelling and burning away over its
// short life — or (`smoke` = 1) a soft, dim green wisp drifting off it.
// A camera-facing quad sized by its transform's scale, drawn additively,
// and never in front of the model: it's depth-tested where it is or behind
// the model's far side, whichever's further, so the whole model always
// shows over it.
// Driven by `PowerUpPuffMaterial` in `client/src/power_ups.rs`.

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::view,
}

struct PuffParams {
    // Linear rgb of its green; a = brightness.
    color: vec4<f32>,
    // 0 at birth … 1 when it's gone.
    age: f32,
    // Its own random pattern and spin.
    seed: f32,
    // 0 = a burst of fire, 1 = a wisp of smoke.
    smoke: f32,
    // How white-hot its middle burns (fire only).
    heat: f32,
    // Its drop's model: middle (xyz) and bounding radius (w).
    model: vec4<f32>,
}
@group(2) @binding(0) var<uniform> puff: PuffParams;

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

fn fbm(p: vec3<f32>) -> f32 {
    return value_noise(p) * 0.55 + value_noise(p * 2.03 + vec3(5.2, 1.3, 0.0)) * 0.3
        + value_noise(p * 4.1 + vec3(1.7, 9.2, 0.0)) * 0.15;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let center = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
    ).xyz;
    let size = length(world_from_local[0].xyz);
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    // Turned its own way, so no two look alike.
    let a = puff.seed * 6.2831853;
    let c = cos(a);
    let s = sin(a);
    let q = vec2<f32>(vertex.position.x * c - vertex.position.y * s, vertex.position.x * s + vertex.position.y * c);
    let world = center + (right * q.x + up * q.y) * size;
    out.world_position = vec4<f32>(world, 1.0);
    out.world_normal = vec3<f32>(vertex.position.xy * 2.0, 0.0);
    out.position = position_world_to_clip(world);
    // Never drawn in front of the model: no nearer than the far side of its
    // bounding sphere (reverse-z: nearer is bigger), so wherever this
    // overlaps the model, the model shows.
    let to_cam = view.world_position - puff.model.xyz;
    let dist = max(length(to_cam), 1e-3);
    let back = position_world_to_clip(puff.model.xyz + to_cam / dist * min(-puff.model.w, dist * 0.5));
    out.position.z = min(out.position.z / out.position.w, back.z / back.w) * out.position.w;
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_normal.xy;
    let r = length(p);
    let age = clamp(puff.age, 0.0, 1.0);
    let seed = puff.seed * 37.0;

    // A lumpy ball: noise pushes its edge in and out, and it churns as it
    // ages.
    let n = fbm(vec3<f32>(p * 2.2 + vec2(seed, seed * 0.7), age * 1.6 + seed));
    let lumpy = r + (n - 0.5) * mix(0.75, 1.0, puff.smoke);
    // It swells, then burns away from the outside in (fire) or thins out
    // (smoke).
    let reach = mix(0.45, 0.95, sqrt(age));
    var density = 1.0 - smoothstep(reach * 0.55, reach, lumpy);
    density *= smoothstep(0.0, 0.12, age);
    let burn = mix(age * age, age, puff.smoke);
    density *= clamp(1.0 - burn * 1.1 + (n - 0.5) * 0.6, 0.0, 1.0);

    var col: vec3<f32>;
    if puff.smoke > 0.5 {
        col = puff.color.rgb * density * 0.6;
    } else {
        // White-hot where it's thickest and youngest, green round the edges.
        // (Billowing, not flat: the noise shades it.)
        let billow = 0.7 + 0.6 * fbm(vec3<f32>(p * 5.0 + vec2(seed * 1.3, seed), age * 2.5));
        let hot = smoothstep(0.45, 0.95, density) * (1.0 - age * 0.8) * puff.heat * billow;
        col = mix(puff.color.rgb, vec3(1.0, 1.0, 0.92) * max(max(puff.color.r, puff.color.g), puff.color.b) * 1.6, clamp(hot, 0.0, 1.0))
            * density * billow;
    }
    // Nothing at the quad's edge.
    col *= 1.0 - smoothstep(0.85, 1.0, r);
    // Premultiplied, alpha 0: purely additive (`AlphaMode::Add`).
    return vec4<f32>(col * puff.color.a, 0.0);
}
