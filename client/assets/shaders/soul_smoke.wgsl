// The Exhumer's soul-smoke: a smoke sprite (the green swirl, or a puff),
// placed and turned to face the camera by its transform, tinted and drawn
// additively — and melted into whatever it meets (the floor, a wall, a
// zombie) rather than cutting into it on a hard line: it fades out the
// closer the scene behind it is (soft particles, off the depth prepass).
// Driven by `SoulSmokeMaterial` in `client/src/exhumer.rs`.

#import bevy_pbr::{
    forward_io::VertexOutput,
    view_transformations::{position_world_to_view, depth_ndc_to_view_z},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils
#endif

struct SmokeParams {
    // Linear rgb tint; a = brightness.
    color: vec4<f32>,
    // How far (m) in front of what's behind it it fades in over.
    soft: f32,
    // How much it dims what's behind it (0: purely additive).
    body: f32,
    _pad1: f32,
    _pad2: f32,
}
@group(2) @binding(0) var<uniform> smoke: SmokeParams;
@group(2) @binding(1) var smoke_texture: texture_2d<f32>;
@group(2) @binding(2) var smoke_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(smoke_texture, smoke_sampler, in.uv);
    var soft = 1.0;
#ifdef DEPTH_PREPASS
    let scene = -depth_ndc_to_view_z(prepass_utils::prepass_depth(in.position, 0u));
    let here = -position_world_to_view(in.world_position.xyz).z;
    soft = smoothstep(0.0, smoke.soft, scene - here);
#endif
    // (The texture's straight alpha: premultiplied here.)
    return vec4<f32>(c.rgb * c.a * smoke.color.rgb * smoke.color.a * soft, clamp(c.a * soft * smoke.body, 0.0, 1.0));
}
