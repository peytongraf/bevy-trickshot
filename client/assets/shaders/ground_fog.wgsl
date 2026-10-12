// Ground fog: a low, splotchy layer of fog lying over a map's floor — thick
// at `base_height`, thinning out exponentially above it, broken into pools by
// a fixed noise pattern across the ground (the same every game, for a given
// seed). One fullscreen pass over the world camera's HDR image, before bloom:
// the fog between the eye and whatever each pixel shows is summed in a few
// steps along its ray, clipped to the slab the fog lives in.
// Driven by `GroundFogUniform` in `client/src/environment/ground_fog.rs`.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_render::view::View

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
#ifdef MULTISAMPLED
@group(0) @binding(1) var depth_texture: texture_depth_multisampled_2d;
#else
@group(0) @binding(1) var depth_texture: texture_depth_2d;
#endif
@group(0) @binding(2) var<uniform> view: View;

struct GroundFog {
    // Linear RGB, brightness included.
    color: vec4<f32>,
    // xy: how far (m) the pattern's drifted on the wind.
    drift: vec4<f32>,
    density: f32,
    base_height: f32,
    falloff_height: f32,
    max_distance: f32,
    max_opacity: f32,
    splotch_size: f32,
    splotchiness: f32,
    contrast: f32,
    seed: f32,
}
@group(0) @binding(3) var<uniform> fog: GroundFog;

// Steps along each ray (inside the fog's slab only).
const STEPS: i32 = 8;
// How fast (m) the fog fades out below its base — so a lower floor of the
// map isn't buried in it.
const BELOW_FADE: f32 = 1.0;

fn hash(p: vec2<i32>, seed: u32) -> f32 {
    var h = (bitcast<u32>(p.x) * 0x8da6b343u) ^ (bitcast<u32>(p.y) * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967295.0);
}

fn value_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i, seed);
    let b = hash(i + vec2<i32>(1, 0), seed);
    let c = hash(i + vec2<i32>(0, 1), seed);
    let d = hash(i + vec2<i32>(1, 1), seed);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Three octaves, each turned against the last so the grid never shows.
fn fbm(p_in: vec2<f32>, seed: u32) -> f32 {
    let rot = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    var p = p_in;
    var sum = 0.0;
    var amp = 0.5;
    for (var o = 0u; o < 3u; o++) {
        sum += amp * value_noise(p, seed + o);
        p = rot * p * 2.03;
        amp *= 0.5;
    }
    return sum / 0.875;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(floor(in.position.xy));
    let scene = textureLoad(screen_texture, coord, 0);
    let depth = textureLoad(depth_texture, coord, 0);

    let ndc = in.uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    let eye = view.world_position;
    // (Reverse z: depth 1 is the near plane, 0 nothing at all — the sky.)
    let near_h = view.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(near_h.xyz / near_h.w - eye);
    var dist = fog.max_distance;
    if depth > 0.0 {
        let hit = view.world_from_clip * vec4<f32>(ndc, depth, 1.0);
        dist = min(dist, length(hit.xyz / hit.w - eye));
    }

    // Only the stretch of the ray inside the fog's slab.
    let lo = fog.base_height - BELOW_FADE * 3.0;
    let hi = fog.base_height + fog.falloff_height * 6.0;
    var t0 = 0.0;
    var t1 = dist;
    if abs(dir.y) < 1e-5 {
        if eye.y < lo || eye.y > hi {
            return scene;
        }
    } else {
        let ta = (lo - eye.y) / dir.y;
        let tb = (hi - eye.y) / dir.y;
        t0 = max(min(ta, tb), 0.0);
        t1 = min(max(ta, tb), dist);
    }
    if t1 <= t0 {
        return scene;
    }

    let seed = u32(fog.seed);
    let step = (t1 - t0) / f32(STEPS);
    var optical = 0.0;
    for (var i = 0; i < STEPS; i++) {
        let p = eye + dir * (t0 + (f32(i) + 0.5) * step);
        let h = p.y - fog.base_height;
        let height = select(exp(-h / fog.falloff_height), exp(h / BELOW_FADE), h < 0.0);
        let n = fbm((p.xz + fog.drift.xy) / fog.splotch_size, seed);
        let pools = clamp((n - 0.5) * fog.contrast + 0.5, 0.0, 1.0) * 2.0;
        optical += fog.density * height * mix(1.0, pools, fog.splotchiness) * step;
    }
    let amount = min(1.0 - exp(-optical), fog.max_opacity);
    return vec4<f32>(mix(scene.rgb, fog.color.rgb, amount), scene.a);
}
