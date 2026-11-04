// A player's soul out of its body (the easter egg's trip into the pyramid):
// their own avatar, drawn see-through and glowing — additively, so it never
// hides what's behind it. Brightest round its edges (a rim where the
// surface turns away from us), faint inside with a ghost of its own
// texture's detail, and light rippling up through it, flickering. Driven
// by `GhostMaterial` in `client/src/easter_egg/ascension.rs`.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
}

struct GhostParams {
    // Linear rgb; a = how bright overall.
    color: vec4<f32>,
    // How bright its rim, and its inside.
    rim: f32,
    body: f32,
    // How strongly light ripples up through it.
    flow: f32,
    _pad0: f32,
}
@group(2) @binding(0) var<uniform> ghost: GhostParams;
@group(2) @binding(1) var ghost_texture: texture_2d<f32>;
@group(2) @binding(2) var ghost_sampler: sampler;

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
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    let t = globals.time;
    var n = normalize(in.world_normal);
    if (!is_front) {
        n = -n;
    }
    let to_eye = normalize(view.world_position.xyz - in.world_position.xyz);
    let facing = clamp(abs(dot(n, to_eye)), 0.0, 1.0);
    let rim = pow(1.0 - facing, 2.2);

    // Its own detail, faintly (the texture's brightness).
    var detail = 0.6;
#ifdef VERTEX_UVS_A
    let c = textureSample(ghost_texture, ghost_sampler, in.uv).rgb;
    detail = clamp(dot(c, vec3(0.299, 0.587, 0.114)) * 1.6, 0.0, 1.0);
#endif

    // Light rippling up through it, in the world (so it runs up the whole
    // figure as one).
    let p = in.world_position.xyz;
    let across = p.x * 1.7 + p.z * 1.3;
    let up = vnoise(vec2(across * 2.0, p.y * 3.0 - t * 1.4)) * 0.6
        + vnoise(vec2(across * 5.0 + 7.0, p.y * 7.0 - t * 2.6)) * 0.4;
    let ripple = mix(1.0, 0.45 + 1.1 * up, ghost.flow);
    let flicker = 0.9 + 0.1 * sin(t * 13.0 + p.y * 4.0);

    let glow = (ghost.rim * rim + ghost.body * (0.35 + 0.65 * detail)) * ripple * flicker;
    // (Additive, `AlphaMode::Add`: alpha 0.)
    return vec4<f32>(ghost.color.rgb * ghost.color.a * glow, 0.0);
}
