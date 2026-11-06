// The Exhumer's beam: a ribbon from the gun out to its reach, widening as it
// goes, turned once about its line to face the camera. It's drawn as a plume
// of soul-smoke rather than a shape: billowing, ragged at its edges, burning
// pale near the gun and deepening to green further out, all of it drifting
// along its length. Its smoke has some body (it dims what's behind it a
// little), so it stays green over a bright scene instead of washing out to
// white. Never the same twice (each beam has its own seed). Sized by its
// transform: local Y along the beam (its length; its origin half way along),
// X its width at the far end, Z's scale its seed (1..2). Its far end trails
// behind as it swings, bending it like a rope (`beam.bend`, so its mesh is cut
// along its length). Driven by `ExhumerBeamMaterial` in
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
    // How fast (m/s) its smoke drifts in toward the gun (negative: out).
    flow: f32,
    // How much it flickers (0..1).
    flicker: f32,
    // How much its edges billow and wander.
    turbulence: f32,
    // How far (m) from the gun it burns pale.
    heat: f32,
    // How smoky / broken up it is (0 = smooth).
    smoke: f32,
    // How much its smoke dims what's behind it (0 = purely additive).
    body: f32,
    _pad0: f32,
    // How far (m, world) its far end's off its line; w: how that's spread
    // along it (the offset at v is bend * v^w).
    bend: vec4<f32>,
}
@group(2) @binding(0) var<uniform> beam: BeamParams;

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
        // (Turned each octave, so its grid never shows.)
        q = mat2x2<f32>(1.6, 1.2, -1.2, 1.6) * q + vec2(1.7, 9.2);
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
    // Bent (as `BeamVisual::point`).
    let bend_at = max(v, 1e-3);
    let on_line = middle + axis * vertex.position.y * len + beam.bend.xyz * pow(bend_at, beam.bend.w);
    // Turned to face the camera once, as seen from the gun's end, not at each
    // point: our own beam runs almost straight away from us, so further out
    // the way that faces us swings wildly with the smallest change of aim.
    let to_cam = view.world_position - (middle - axis * len * 0.5);
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
    let t = globals.time;
    // Its smoke drifts along it.
    let flow = along + t * beam.flow;

    // Faded in at the gun, and out over its last stretch, so it never
    // stops on a hard edge.
    let tail = min(2.5, len * 0.45);
    let ends = smoothstep(0.0, 0.1, along) * (1.0 - smoothstep(len - tail, len, along));

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

    // Its middle wanders, more the further out.
    let wander = fbm(vec2(flow * 0.3, seed + t * 0.25)) - 0.47;
    let warp = fbm(vec2(flow * 0.5 + seed, u * 1.4 + t * 0.2)) - 0.47;
    let d = u - (wander * 0.9 + warp * 0.7) * beam.turbulence * (0.2 + 0.8 * v);

    // Billows: stretched along it, one drawn through the other so they curl.
    let large = fbm(vec2(flow * 0.7 + warp * 2.5, d * 1.5 + seed));
    let fine = fbm(vec2(flow * 1.9 - t * 0.5 + large * 2.0, d * 3.4 - seed));
    let puffs = large * 0.62 + fine * 0.38;

    // How far out it reaches here is the billows' doing, so its edge is never
    // a straight line. Tight and clean at the gun, ragged further out.
    let rag = beam.turbulence * (0.3 + 0.7 * v);
    let reach = clamp(0.55 + (puffs - 0.45) * 1.8 * rag, 0.18, 0.92);
    let shape = 1.0 - smoothstep(reach * 0.15, reach, abs(d));
    let clumps = mix(1.0, smoothstep(0.2, 0.68, puffs) * 1.5, beam.smoke * (0.35 + 0.65 * v));
    let density = shape * clumps;

    // Hottest down its middle, close to the gun; deepening further out.
    let near_gun = exp(-along / max(beam.heat, 0.05));
    let heat = clamp(density * 0.45 + near_gun * exp(-d * d * 7.0) * 1.1, 0.0, 1.0);
    let glow = beam.color.rgb * beam.color.a;
    let deep = glow * vec3(0.28, 0.72, 0.34);
    let pale = mix(glow, vec3(max(max(glow.r, glow.g), glow.b)), 0.45);
    let tone = mix(mix(deep, glow, smoothstep(0.0, 0.5, heat)), pale, smoothstep(0.55, 1.0, heat));

    let mask = ends * soft * (1.0 - smoothstep(0.86, 1.0, abs(u)));
    let amount = density * (0.45 + 0.75 * near_gun) * waver * mask;
    // Premultiplied: its light on top, and what's behind it dimmed by its
    // smoke's body.
    return vec4<f32>(tone * amount, clamp(amount * beam.body, 0.0, 1.0));
}
