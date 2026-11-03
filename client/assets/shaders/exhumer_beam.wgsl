// The Exhumer's beam: a ribbon from the gun out to its reach that turns
// about its own line to face the camera, widening as it goes (the beam
// takes hold in a cone). Drawn additively: a soft, smoky haze of souls
// streaming back toward the gun, a white-hot core that writhes along its
// length, a few thin strands coiling round it (a helix seen side-on), bright
// pulses and flecks racing in, all wavering — never the same twice (each
// beam has its own seed). Sized by its transform: local Y along the beam
// (its length; its origin half way along), X its width at the far end, Z's
// scale its seed (1..2). Driven by `ExhumerBeamMaterial` in
// `client/src/exhumer.rs`.

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::{position_world_to_clip, position_world_to_view, depth_ndc_to_view_z},
    mesh_view_bindings::{view, globals},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils
#endif

struct BeamParams {
    // Linear rgb; a = brightness.
    color: vec4<f32>,
    // Its width at the gun, as a share of its width at the far end.
    near: f32,
    // How fast (m/s) what's in it streams back toward the gun.
    flow: f32,
    // How much it flickers (0..1).
    flicker: f32,
    // How bright the coiling strands are (0 = none).
    strands: f32,
    // How much the core writhes (share of the half-width).
    writhe: f32,
    // How smoky / broken up the haze is (0 = smooth).
    smoke: f32,
    // How strong the streaming swirl texture is.
    swirl: f32,
    _pad0: f32,
}
@group(2) @binding(0) var<uniform> beam: BeamParams;
// The green swirl of soul-smoke (`textures/vfx/soul_swirl.png`): streamed
// along the beam in two staggered layers so its ends never leave a gap.
@group(2) @binding(1) var swirl_texture: texture_2d<f32>;
@group(2) @binding(2) var swirl_sampler: sampler;

fn hash1(x: f32) -> f32 {
    return fract(sin(x * 127.1) * 43758.5453);
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn noise1(x: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(hash1(i), hash1(i + 1.0), u);
}

fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash2(i), hash2(i + vec2(1.0, 0.0)), u.x),
        mix(hash2(i + vec2(0.0, 1.0)), hash2(i + vec2(1.0, 1.0)), u.x),
        u.y,
    );
}

fn fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        sum += noise2(q) * amp;
        q = q * 2.03 + vec2(1.7, 9.2);
        amp *= 0.5;
    }
    return sum;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    // (Its origin's half way along it.)
    let middle = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
    ).xyz;
    let width = length(world_from_local[0].xyz);
    let len_vec = world_from_local[1].xyz;
    let len = length(len_vec);
    let axis = len_vec / max(len, 1e-4);
    let seed = length(world_from_local[2].xyz);
    // (The quad's -0.5..0.5 along its Y: 0 at the gun … 1 at the far end.)
    let v = vertex.position.y + 0.5;
    let on_line = middle + axis * vertex.position.y * len;
    // Turned about its own line to face the camera, at each point along it.
    let to_cam = view.world_position - on_line;
    var across = cross(axis, to_cam);
    if dot(across, across) < 1e-8 {
        across = cross(axis, vec3<f32>(0.0, 1.0, 0.0));
    }
    across = normalize(across);
    let half = 0.5 * width * mix(beam.near, 1.0, v);
    let world = on_line + across * vertex.position.x * 2.0 * half;
    out.world_position = vec4<f32>(world, 1.0);
    // x: −1..1 across, y: metres from the gun, z: its length; uv: its
    // seed, and its half-width (m) here.
    out.world_normal = vec3<f32>(vertex.position.x * 2.0, v * len, len);
    out.uv = vec2<f32>(seed, half);
    out.position = position_world_to_clip(world);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = in.world_normal.x;
    let along = in.world_normal.y;
    let len = max(in.world_normal.z, 0.1);
    let v = clamp(along / len, 0.0, 1.0);
    let seed = in.uv.x * 37.0;
    let half = max(in.uv.y, 1e-3);
    let t = globals.time;
    // Its pattern streams back toward the gun.
    let flow = along + t * beam.flow;

    // Faded in at the gun, and out over its last stretch, so it never
    // stops on a hard edge.
    let tail = min(1.5, len * 0.35);
    let ends = smoothstep(0.0, 0.35, along) * (1.0 - smoothstep(len - tail, len, along));

    // Melted into whatever it meets (a wall, the floor, a zombie) instead of
    // cutting into it on a hard line: fading out the closer the scene behind
    // it is (soft particles, off the depth prepass).
    var soft = 1.0;
#ifdef DEPTH_PREPASS
    let scene = -depth_ndc_to_view_z(prepass_utils::prepass_depth(in.position, 0u));
    let here = -position_world_to_view(in.world_position.xyz).z;
    soft = smoothstep(0.0, 0.6, scene - here);
#endif

    // Its whole brightness wavers, as the souls fight it.
    let waver = 1.0 - beam.flicker * (0.5 * noise1(t * 13.0 + seed) + 0.5 * noise1(t * 31.0 + seed * 2.0));

    // The core writhes about its line (more the further out), so it never
    // reads as a straight rod.
    let swing = beam.writhe * (0.25 + 0.75 * v);
    let centre = (noise1(along * 0.55 - t * 2.3 + seed) - 0.5) * 2.0 * swing
        + (noise1(along * 2.1 - t * 5.0 + seed * 3.0) - 0.5) * 0.5 * swing;
    let d = u - centre;

    // The haze: soft across, broken into streaming smoke.
    let smoke = fbm(vec2(flow * 0.9, d * 2.5 + seed));
    let wisps = fbm(vec2(flow * 2.7 - t * 1.5, d * 6.0 - seed));
    let haze = exp(-d * d * 3.5) * mix(1.0, 0.15 + 1.7 * smoke * (0.6 + 0.8 * wisps), beam.smoke);

    // The core: a thin white-hot line (a fixed few cm, whatever the width),
    // flickering along its length.
    let core_w = clamp(0.035 / half, 0.02, 0.5);
    let core_d = d / core_w;
    let core = exp(-core_d * core_d) * (0.65 + 0.7 * noise1(flow * 3.0 + seed));

    // Strands coiling round it: each a helix, so a wave seen side-on, its
    // twist travelling in toward the gun; each comes and goes on its own.
    var strands = 0.0;
    for (var i = 0; i < 3; i++) {
        let fi = f32(i);
        let phase = fi * 2.094 + seed;
        let freq = 1.6 + fi * 0.7;
        let amp = (0.35 + 0.25 * noise1(along * 0.4 + fi * 7.0 + t * 0.7)) * (0.3 + 0.7 * v);
        let x = centre + sin(flow * freq + phase) * amp
            + (noise1(along * 4.0 + t * 6.0 + fi * 13.0) - 0.5) * 0.08;
        let sw = clamp(0.012 / half, 0.008, 0.2);
        let sd = (u - x) / sw;
        let on = smoothstep(0.2, 0.8, noise1(t * (0.6 + fi * 0.27) + fi * 5.3 + seed));
        // (Brighter on the near side of the coil, where cos > 0.)
        let near_side = 0.55 + 0.45 * cos(flow * freq + phase);
        strands += exp(-sd * sd) * on * near_side;
    }

    // Pulses: bright knots racing back down it, a few metres apart.
    let p = fract(flow / 3.2 + hash1(floor(flow / 3.2) + seed) * 0.3);
    let pulse = pow(1.0 - abs(p - 0.5) * 2.0, 10.0) * exp(-d * d * 8.0);

    // Flecks: sparse sparks streaming in, scattered across it.
    let cell = vec2(floor(flow * 4.0), floor((u + 1.0) * 6.0));
    let fleck_on = step(0.86, hash2(cell + vec2(seed, 0.0)));
    let fleck_at = vec2(fract(flow * 4.0), fract((u + 1.0) * 6.0)) - 0.5;
    let fleck = fleck_on * exp(-dot(fleck_at, fleck_at) * 40.0) * exp(-d * d * 1.5);

    // The swirl streaming in: across the beam (bent with its writhing
    // core), a few metres a repeat along it, the second layer half a repeat
    // on, flipped and a little narrower.
    let period = 2.6;
    let across = clamp(d * 0.5 + 0.5, 0.0, 1.0);
    let s1 = textureSample(swirl_texture, swirl_sampler, vec2(fract(flow / period + seed), across));
    let s2 = textureSample(swirl_texture, swirl_sampler, vec2(fract(flow / (period * 1.3) + 0.5 + seed * 1.7), clamp(1.0 - (d * 0.6 + 0.5), 0.0, 1.0)));
    let swirl = s1.rgb * s1.a + s2.rgb * s2.a * 0.7;

    let green = beam.color.rgb * beam.color.a;
    // (The core and strands burn a little paler than the haze — but stay
    // green, so it doesn't wash out to white.)
    let pale = mix(green, vec3(max(max(green.r, green.g), green.b)), 0.35);
    let edge = 1.0 - smoothstep(0.85, 1.0, abs(u));

    let light = green * (haze * 0.3 + pulse * 0.6 + fleck * 1.0)
        + swirl * beam.color.rgb * beam.color.a * beam.swirl
        + pale * (core * 0.8 + strands * beam.strands);
    // Premultiplied, alpha 0: purely additive (`AlphaMode::Add`).
    return vec4<f32>(light * ends * edge * waver * soft, 0.0);
}
