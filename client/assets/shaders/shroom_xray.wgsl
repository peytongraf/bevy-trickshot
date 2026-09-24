// Shroom "x-ray": a bright, hazy, smoke-edged ghost of an enemy seen through
// walls. Drawn on a twin of each enemy mesh (same skin, so it animates with
// it) whose pipeline only passes where something is *in front* of it (see
// `ShroomXrayMaterial::specialize`) — so it shows exactly where the enemy is
// hidden, and never over the enemy when you can see them. Driven by
// `ShroomXrayMaterial` in `client/src/vfx/shroom_xray.rs`.

#import bevy_pbr::{
    mesh_functions,
    skinning,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::{view, globals},
}

struct XrayParams {
    // rgb = the haze colour, a = brightness multiplier (glow, for bloom).
    color: vec4<f32>,
    strength: f32,
    inflate: f32,
    fill: f32,
    edge_softness: f32,
    smoke_scale: f32,
    smoke_speed: f32,
    smoke_amount: f32,
    shimmer: f32,
}
@group(2) @binding(0) var<uniform> x: XrayParams;

// --- cheap 3D value noise ---------------------------------------------

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

// Three octaves — soft, billowy.
fn fbm(p: vec3<f32>) -> f32 {
    return value_noise(p) * 0.55 + value_noise(p * 2.03) * 0.3 + value_noise(p * 4.01) * 0.15;
}

// Rotate a colour's hue by `angle` radians around the grey axis.
fn hue_rotate(c: vec3<f32>, angle: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735);
    let ca = cos(angle);
    return c * ca + cross(k, c) * sin(angle) + k * dot(k, c) * (1.0 - ca);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef SKINNED
    let world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex.instance_index,
    );
#else
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
#endif

#ifdef VERTEX_NORMALS
#ifdef SKINNED
    let n = normalize(skinning::skin_normals(world_from_local, vertex.normal));
#else
    let n = normalize(mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
#endif
#else
    let n = vec3<f32>(0.0, 1.0, 0.0);
#endif

    var world = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    // Puff the ghost out past the body, billowing a little, so its soft edge
    // reaches beyond the silhouette like smoke.
    let t = globals.time;
    let billow = value_noise(world.xyz * x.smoke_scale * 0.6 + vec3<f32>(0.0, -t * x.smoke_speed, 0.0));
    world = vec4<f32>(world.xyz + n * x.inflate * (0.6 + 0.8 * billow), 1.0);

    out.world_position = world;
    out.world_normal = n;
    out.position = position_world_to_clip(world.xyz);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let v = normalize(view.world_position - in.world_position.xyz);
    let n = normalize(in.world_normal);
    // 1 facing us (the body's middle) … 0 at the silhouette.
    let facing = clamp(abs(dot(n, v)), 0.0, 1.0);
    // Bright through the body, fading out toward (and past) the outline.
    let body = x.fill + (1.0 - x.fill) * pow(facing, x.edge_softness);

    // Drifting smoke breaking the haze up into wisps.
    let t = globals.time;
    let p = in.world_position.xyz * x.smoke_scale + vec3<f32>(0.0, -t * x.smoke_speed, t * x.smoke_speed * 0.3);
    let smoke = fbm(p);
    let wisp = mix(1.0, clamp(smoke * 1.8 - 0.25, 0.0, 1.5), x.smoke_amount);

    let a = clamp(x.strength * body * wisp, 0.0, 1.0);

    // A psychedelic hue wobble rolling across the body over time.
    let hue = x.shimmer * sin(t * 1.7 + dot(in.world_position.xyz, vec3<f32>(1.3, 2.1, 0.9)));
    let col = max(hue_rotate(x.color.rgb, hue), vec3<f32>(0.0)) * x.color.a;

    // Premultiplied, alpha 0: purely additive glow (`AlphaMode::Add`).
    return vec4<f32>(col * a, 0.0);
}
