// Liquid Courage's drunk screen effect: a slow head sway / roll, a faint
// double image drifting apart and back, dark edges and a warm flush, plus a
// soft blur during the kick-in only. One fullscreen pass, 10 texture taps. `strength` goes above 1 during the
// kick-in right after buying the perk. Runs after the shroom pass, so the
// two stack. Driven by `DrunkUniform` in `client/src/vfx/drunk.rs`.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

struct DrunkUniform {
    strength: f32,
    time: f32,
    sway_roll: f32,
    sway_drift: f32,
    sway_speed: f32,
    double_offset: f32,
    double_mix: f32,
    double_speed: f32,
    // Final values (fade + kick already applied on the CPU side).
    vignette: f32,
    blur: f32,
    flush: f32,
}
@group(0) @binding(2) var<uniform> s: DrunkUniform;

// A soft 5-tap cross blur of radius `r` (uv units, already aspect-corrected).
fn soft(uv: vec2<f32>, r: vec2<f32>) -> vec3<f32> {
    let c = textureSample(screen_texture, texture_sampler, uv).rgb * 0.4;
    let a = textureSample(screen_texture, texture_sampler, uv + vec2<f32>(r.x, 0.0)).rgb;
    let b = textureSample(screen_texture, texture_sampler, uv - vec2<f32>(r.x, 0.0)).rgb;
    let d = textureSample(screen_texture, texture_sampler, uv + vec2<f32>(0.0, r.y)).rgb;
    let e = textureSample(screen_texture, texture_sampler, uv - vec2<f32>(0.0, r.y)).rgb;
    return c + (a + b + d + e) * 0.15;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(screen_texture));
    let aspect = vec2<f32>(dims.x / dims.y, 1.0);
    let k = s.strength;
    let t = s.time * s.sway_speed;

    // Head sway: a slow, irregular roll and drift (incommensurate sines so it
    // never settles into a rhythm), with a touch of zoom so the rotated
    // corners never show the edge of the frame.
    var p = (in.uv - 0.5) * aspect;
    let roll = s.sway_roll * k * (0.6 * sin(t * 0.9) + 0.4 * sin(t * 0.37 + 1.3));
    let cr = cos(roll);
    let sr = sin(roll);
    p = vec2<f32>(p.x * cr - p.y * sr, p.x * sr + p.y * cr);
    p = p / (1.0 + k * (abs(s.sway_roll) * 0.6 + s.sway_drift * 2.0));
    p = p + vec2<f32>(sin(t * 0.53 + 0.4), cos(t * 0.71)) * (s.sway_drift * k);
    let r = length(p);
    let uv = p / aspect + 0.5;

    // Double vision: a ghost copy that drifts apart (mostly sideways, like
    // eyes that won't converge) and back.
    let dt = s.time * s.double_speed;
    let sep = s.double_offset * k * (0.55 + 0.45 * sin(dt));
    let dir = normalize(vec2<f32>(1.0, 0.35 * sin(dt * 0.61 + 0.8)));
    let ghost = dir * sep / aspect;

    // Soft focus during the kick-in, blurrier toward the edges (radius 0 —
    // i.e. a plain sample — once it's settled).
    let br = vec2<f32>(s.blur * (0.35 + r)) / aspect;
    var col = mix(soft(uv, br), soft(uv + ghost, br), clamp(s.double_mix * k, 0.0, 0.9));

    // Tunnel-ish dark edges.
    col = col * max(1.0 - s.vignette * smoothstep(0.3, 0.95, r), 0.0);

    // Warm, reddish flush.
    col = mix(col, col * vec3<f32>(1.08, 0.86, 0.8), clamp(s.flush * k, 0.0, 1.0));

    return vec4<f32>(col, 1.0);
}
