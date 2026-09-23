// Shroom screen effect: a slow, organic "breathing" warp of the whole frame
// plus a saturation / hue shimmer. One fullscreen pass, 3 texture taps.
// Driven by `ShroomUniform` in `client/src/vfx/shroom.rs`.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct ShroomUniform {
    strength: f32,
    time: f32,
    wave_amplitude: f32,
    wave_frequency: f32,
    wave_speed: f32,
    breathe_amplitude: f32,
    breathe_speed: f32,
    center_clear: f32,
    chromatic: f32,
    saturation: f32,
    hue_drift: f32,
    hue_speed: f32,
}
@group(0) @binding(2) var<uniform> s: ShroomUniform;

// Rotate a colour's hue by `angle` radians around the grey axis
// (Rodrigues rotation about (1,1,1)/√3 — cheap, no HSV round trip).
fn hue_rotate(c: vec3<f32>, angle: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735);
    let ca = cos(angle);
    return c * ca + cross(k, c) * sin(angle) + k * dot(k, c) * (1.0 - ca);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(screen_texture));
    let aspect = vec2<f32>(dims.x / dims.y, 1.0);
    let k = s.strength;

    // Centred, aspect-correct coordinates so the waves are round, not stretched.
    var p = (in.uv - 0.5) * aspect;
    let r = length(p);

    // "Breathing": a slow zoom pulse. Only ever zooms *in* (plus a bit of
    // headroom for the waves) so the screen edges never get pulled inward.
    let breathe = 0.5 + 0.5 * sin(s.time * s.breathe_speed);
    let zoom = 1.0 + k * (s.breathe_amplitude * breathe + s.wave_amplitude * 1.5);
    p = p / zoom;

    // Flowing warp: two sine layers, the second's domain bent by the first
    // (domain warping) so the motion reads as liquid rather than a regular
    // ripple. Incommensurate multipliers keep it from visibly repeating.
    let t = s.time * s.wave_speed;
    let f = s.wave_frequency;
    let q = vec2<f32>(
        sin(p.y * f + t) + sin(p.y * f * 0.53 - t * 0.71 + 1.7),
        cos(p.x * f * 0.91 - t * 0.83) + cos(p.x * f * 0.47 + t * 0.59 + 4.1),
    );
    let w = vec2<f32>(
        sin((p.y + 0.12 * q.x) * f * 1.37 + t * 1.13),
        cos((p.x + 0.12 * q.y) * f * 1.21 - t * 0.97),
    );
    // Keep the middle of the screen (where you aim) steadier than the edges.
    let mask = mix(1.0, smoothstep(0.0, 0.55, r), s.center_clear);
    p = p + (q * 0.25 + w * 0.5) * (s.wave_amplitude * k * mask);

    let uv = p / aspect + 0.5;

    // Colour fringing that grows toward the edges.
    let ca = (uv - 0.5) * (s.chromatic * k * r);
    var col = vec3<f32>(
        textureSample(screen_texture, texture_sampler, uv + ca).r,
        textureSample(screen_texture, texture_sampler, uv).g,
        textureSample(screen_texture, texture_sampler, uv - ca).b,
    );

    // Hue shimmer: a slow, spatially varying hue rotation.
    let hue = s.hue_drift * k * sin(s.time * s.hue_speed + p.x * 2.3 + p.y * 1.7);
    col = hue_rotate(col, hue);

    // Saturation boost around Rec.709 luma.
    let luma = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = max(mix(vec3<f32>(luma), col, mix(1.0, s.saturation, k)), vec3<f32>(0.0));

    return vec4<f32>(col, 1.0);
}
