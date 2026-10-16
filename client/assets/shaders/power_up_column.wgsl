// The column of green light a `Zombies` power-up drop stands in, Cold War
// style: from the ground up past the model, soft across and streaked
// lengthways with slowly flowing shafts, brightest round the model and where
// it meets the ground — and a few thin, bright strands of energy hanging
// down from the model, swaying, each coming and going on its own. An
// upright quad that turns about the vertical to face the camera, sized by
// its transform's scale (width × height, its middle at its translation; z
// carries the model's bounding radius), drawn additively, and never in
// front of the model: depth-tested where it stands or behind the model's
// far side, whichever's further, so the whole model always shows over it. Driven by
// `PowerUpColumnMaterial` in `client/src/power_ups.rs`.

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::{view, globals},
}

struct ColumnParams {
    // Linear rgb; a = brightness.
    color: vec4<f32>,
    // How strongly it's streaked into shafts (0 = smooth).
    streaks: f32,
    // How fast the shafts flow up (+) or down (−).
    flow: f32,
    // The hanging strands' brightness (0 = none).
    strands: f32,
    // Where the model's middle sits up the column (0 = ground, 1 = top).
    model_at: f32,
}
@group(2) @binding(0) var<uniform> col: ColumnParams;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash2(i), hash2(i + vec2(1.0, 0.0)), u.x),
        mix(hash2(i + vec2(0.0, 1.0)), hash2(i + vec2(1.0, 1.0)), u.x),
        u.y,
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
    let width = length(world_from_local[0].xyz);
    let height = length(world_from_local[1].xyz);
    // Turned about the vertical to face the camera.
    var to_cam = view.world_position - center;
    to_cam.y = 0.0;
    let across = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), select(vec3<f32>(0.0, 0.0, 1.0), to_cam, dot(to_cam, to_cam) > 1e-6)));
    let world = center + across * vertex.position.x * width + vec3<f32>(0.0, vertex.position.y * height, 0.0);
    out.world_position = vec4<f32>(world, 1.0);
    // x: −1..1 across, y: 0 at the ground … 1 at the top.
    out.world_normal = vec3<f32>(vertex.position.x * 2.0, vertex.position.y + 0.5, 0.0);
    out.position = position_world_to_clip(world);
    // Never drawn in front of the model: no nearer than the far side of its
    // bounding sphere (reverse-z: nearer is bigger), so wherever this
    // overlaps the model, the model shows.
    let radius = length(world_from_local[2].xyz);
    let model = center + vec3<f32>(0.0, (col.model_at - 0.5) * height, 0.0);
    let model_to_cam = view.world_position - model;
    let dist = max(length(model_to_cam), 1e-3);
    let back = position_world_to_clip(model + model_to_cam / dist * min(-radius, dist * 0.5));
    out.position.z = min(out.position.z / out.position.w, back.z / back.w) * out.position.w;
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = in.world_normal.x;
    let v = in.world_normal.y;
    let t = globals.time;

    // Soft across; fading out at the top, a little brighter round the
    // model and where it meets the ground.
    let across = exp(-u * u * 3.0) * (1.0 - smoothstep(0.7, 1.0, abs(u)));
    let along = (1.0 - smoothstep(0.6, 1.0, v)) * smoothstep(0.0, 0.04, v);
    let lift = 0.55 + 0.6 * exp(-pow((v - col.model_at) * 3.0, 2.0)) + 0.35 * exp(-v * 12.0);

    // Shafts: noise stretched lengthways, flowing.
    let shafts = noise(vec2(u * 7.0, v * 1.3 - t * col.flow)) * 0.65
        + noise(vec2(u * 17.0 + 3.1, v * 2.1 - t * col.flow * 1.6)) * 0.35;
    let streaked = mix(1.0, 0.25 + 1.5 * shafts, col.streaks);

    let light = across * along * lift * streaked;

    // Strands hanging from the model, swaying, each fading in and out.
    var strands = 0.0;
    let below = col.model_at;
    for (var i = 0; i < 4; i++) {
        let fi = f32(i);
        let on = smoothstep(0.55, 0.95, sin(t * (0.7 + fi * 0.23) + fi * 2.4));
        // From just under the model's middle down toward the ground.
        let k = clamp((below - v) / max(below, 1e-3), 0.0, 1.0);
        let reach = smoothstep(0.0, 0.08, k) * (1.0 - smoothstep(0.55 + fi * 0.1, 0.9, k));
        let x = (fi - 1.5) * 0.12 + sin(v * 9.0 + t * (1.3 + fi * 0.4) + fi * 1.7) * 0.06 * k
            + sin(v * 23.0 - t * 2.1 + fi) * 0.015;
        let d = (u - x) / 0.012;
        strands += exp(-d * d) * on * reach;
    }
    let green = col.color.rgb * col.color.a;
    // (The strands burn paler than the green round them.)
    let pale = mix(green, vec3(max(max(green.r, green.g), green.b)), 0.6);

    // Premultiplied, alpha 0: purely additive (`AlphaMode::Add`).
    return vec4<f32>(green * light + pale * strands * col.strands, 0.0);
}
