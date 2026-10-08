// The end-of-game results' fire (`client/src/end_screen_fx.rs`): flames
// licking up from the bottom of the screen, behind the results. Two layers
// of rotated-octave gradient noise (as `fire.wgsl`) scroll upward, the
// second warped by the first, eroding a heat field that's hottest along the
// bottom edge into ragged tongues; heat is coloured from deep red at the
// cooling tips through orange to a pale yellow core. Driven by
// `EndScreenFireMaterial`.

#import bevy_ui::ui_vertex_output::UiVertexOutput

// a: x time (s), y opacity (faded in), z how fast it licks up, w how big
//    the tongues are (higher = smaller, busier).
// b: x how much the noise tears it (0 = a smooth glow), y brightness,
//    z / w unused.
@group(1) @binding(0) var<uniform> a: vec4<f32>;
@group(1) @binding(1) var<uniform> b: vec4<f32>;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn grad(i: vec2<i32>) -> vec2<f32> {
    let h = pcg(bitcast<u32>(i.x) ^ pcg(bitcast<u32>(i.y) + 0x9E3779B9u));
    let ang = f32(h) / 4294967295.0 * 6.2831853;
    return vec2<f32>(cos(ang), sin(ang));
}

fn gnoise(q: vec2<f32>) -> f32 {
    let fl = floor(q);
    let i = vec2<i32>(fl);
    let f = q - fl;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let n00 = dot(grad(i), f);
    let n10 = dot(grad(i + vec2<i32>(1, 0)), f - vec2<f32>(1.0, 0.0));
    let n01 = dot(grad(i + vec2<i32>(0, 1)), f - vec2<f32>(0.0, 1.0));
    let n11 = dot(grad(i + vec2<i32>(1, 1)), f - vec2<f32>(1.0, 1.0));
    return mix(mix(n00, n10, u.x), mix(n01, n11, u.x), u.y);
}

fn fbm(q_in: vec2<f32>) -> f32 {
    var q = q_in;
    var sum = 0.0;
    var amp = 0.55;
    let rot = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    for (var i = 0; i < 5; i++) {
        sum += gnoise(q) * amp;
        q = rot * q * 2.03 + vec2<f32>(3.1, 1.7);
        amp *= 0.5;
    }
    return sum;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let time = a.x;
    let opacity = a.y;
    let speed = a.z;
    let detail = a.w;
    let turbulence = b.x;
    let brightness = b.y;

    // x across (in units of the node's height, so tongues keep their shape
    // on any screen), y: 0 at the bottom edge .. 1 at the top.
    let aspect = in.size.x / max(in.size.y, 1.0);
    let x = in.uv.x * aspect;
    let y = 1.0 - in.uv.y;

    // Hot gas speeds up as it rises: the noise scrolls faster higher up.
    let scroll = time * speed * (1.0 + 0.6 * y);
    let q = vec2<f32>(x * detail, y * detail * 0.7 - scroll);
    let warp = fbm(q * 0.6 + vec2<f32>(0.0, -time * speed * 0.4));
    let n = fbm(q + vec2<f32>(warp * 1.6, warp));

    // Heat: hottest along the bottom, eaten into tongues by the noise.
    let heat = clamp(1.05 - y * 1.25 + n * turbulence * 1.4, 0.0, 1.0);
    let flame = smoothstep(0.08, 0.55, heat);

    let deep = vec3<f32>(0.55, 0.04, 0.0);
    let orange = vec3<f32>(1.0, 0.38, 0.04);
    let core = vec3<f32>(1.0, 0.86, 0.45);
    var rgb = mix(deep, orange, smoothstep(0.15, 0.6, heat));
    rgb = mix(rgb, core, smoothstep(0.65, 0.95, heat));

    let alpha = flame * opacity;
    return vec4<f32>(rgb * brightness, clamp(alpha, 0.0, 1.0));
}
