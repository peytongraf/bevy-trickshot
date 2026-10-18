// The spray where a waterfall lands: camera-facing puffs of mist billowing
// up and spreading, and splashes of droplets thrown out of it (`kind`) —
// lit like the rest of the map (and lit through from behind), alpha blended.
// Moved on the CPU (`client/src/water.rs`, `animate_spray`); the quad's
// mesh carries vertex colours only so the vertex stage can hand the
// fragment each puff's own:
// * size — the entity's x scale (m across);
// * life — the z scale divided by the x scale, minus one (0 new .. 1 gone);
// * seed — the y scale divided by the x scale, minus one (0..1).
// Driven by `SprayMaterial` in `client/src/water.rs`.

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::{view, globals},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct SprayParams {
    // 0 = mist, 1 = droplets.
    kind: f32,
    // Overall opacity.
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
}
@group(2) @binding(100) var<uniform> spray: SprayParams;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let model = mesh_functions::get_world_from_local(vertex.instance_index);
    let center = model[3].xyz;
    let sx = length(model[0].xyz);
    let sy = length(model[1].xyz);
    let sz = length(model[2].xyz);
    let size = max(sx, 1e-4);
    let life = clamp(sz / size - 1.0, 0.0, 1.0);
    let seed = clamp(sy / size - 1.0, 0.0, 1.0);

    // Facing the camera, turned by its seed (so the puffs don't all match).
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let a = seed * 6.28318 + life * (seed - 0.5) * 2.0;
    let c = cos(a);
    let s = sin(a);
    let local = vertex.position.xy;
    let turned = vec2(c * local.x - s * local.y, s * local.x + c * local.y);
    let world = center + (right * turned.x + up * turned.y) * size;

    out.world_position = vec4(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = normalize(view.world_position.xyz - center);
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_COLORS
    out.color = vec4(life, seed, 0.0, 1.0);
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex.instance_index, model[3]);
#endif
    return out;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

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

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
#ifdef VERTEX_COLORS
    let life = in.color.x;
    let seed = in.color.y;
#else
    let life = 0.5;
    let seed = 0.0;
#endif
    let uv = in.uv - 0.5;
    let r = length(uv) * 2.0;

    var alpha = 0.0;
    if spray.kind < 0.5 {
        // Mist: a soft, lumpy puff, thinning as it spreads and fades.
        let lumps = vnoise(uv * 5.0 + seed * 31.0 + life * 1.5) * 0.6
            + vnoise(uv * 11.0 - seed * 17.0) * 0.4;
        let body = 1.0 - smoothstep(0.2, 1.0, r + (lumps - 0.5) * 0.5);
        let fade = smoothstep(0.0, 0.15, life) * pow(1.0 - life, 1.4);
        alpha = body * lumps * fade * 0.55;
    } else {
        // Droplets: a cluster of small bright specks, scattering.
        let cells = uv * 9.0 + seed * 13.0;
        let cell = floor(cells);
        let inner = fract(cells) - 0.5;
        let here = hash(cell + seed);
        let speck = 1.0 - smoothstep(0.08, 0.28, length(inner - (vec2(hash(cell + 3.1), hash(cell + 7.7)) - 0.5) * 0.5));
        let dense = step(0.55, here);
        let fade = smoothstep(0.0, 0.05, life) * (1.0 - smoothstep(0.55, 1.0, life));
        alpha = speck * dense * (1.0 - smoothstep(0.5, 1.0, r)) * fade;
    }
    alpha *= spray.opacity;
    if alpha < 0.004 {
        discard;
    }

    pbr_input.material.base_color = vec4(0.9, 0.94, 0.96, alpha);
    pbr_input.material.perceptual_roughness = 0.5;
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
