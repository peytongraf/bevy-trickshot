// Blue outline around a stopped throwing knife, so it's easy to spot on the
// ground: an inverted hull — a copy of the knife's mesh (normals smoothed so
// hard edges don't crack open) pushed out along its normals and drawn with
// front faces culled, so only the rim peeking out past the real knife shows.
// The push grows with distance, keeping the rim roughly the same thickness on
// screen near or far. Driven by `KnifeOutlineMaterial` in
// `client/src/knife_pickup.rs`.

import bevy_pbr::{
    render::mesh_functions,
    render::forward_io::{Vertex, VertexOutput},
    render::view_transformations::position_world_to_clip,
    render::mesh_view_bindings::view};
@if(SKINNED)
import bevy_pbr::render::skinning;

struct OutlineParams {
    // Linear rgb; a unused.
    color: vec4<f32>,
    // Push (m) per metre of distance from the camera.
    width: f32,
    // Least push (m), up close.
    min_width: f32,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> o: OutlineParams;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    // (A dropped AK-74's body is skinned to its view model's arm rig.)
@if(SKINNED)
    let world_from_local = skinning::skin_model(vertex.joint_indices, vertex.joint_weights, vertex.instance_index);
@else
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
@if(VERTEX_NORMALS && SKINNED)
    let n = normalize(skinning::skin_normals(world_from_local, vertex.normal));
@elif(VERTEX_NORMALS)
    let n = normalize(mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
@else
    let n = vec3<f32>(0.0, 1.0, 0.0);
    let base = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    ).xyz;
    let d = length(base - view.world_position);
    let world = base + n * max(o.min_width, d * o.width);

    out.world_position = vec4<f32>(world, 1.0);
    out.world_normal = n;
    out.position = position_world_to_clip(world);
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex.instance_index;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(o.color.rgb, 1.0);
}
