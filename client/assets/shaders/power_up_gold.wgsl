// The polished gold of a `Zombies` power-up drop, Cold War style: a dark
// amber mirror. The game has no reflections to give a mirror-smooth metal
// anything to shine with, so on top of the usual lighting this reflects a
// made-up scene around it (fixed in the world, so it slides over the model
// as it spins): a glowing sky, brightest at the horizon, broken up by a
// skyline of dark buildings with lit windows, a near-black floor below a
// sharp horizon, and a few bright lights round it — all tinted by the gold
// (paler at grazing angles, as real metal goes), plus a faint rim round its
// edge. A busy, high-contrast reflection is what makes it read as a mirror
// rather than smooth plastic. Driven by `GoldMaterial` in
// `client/src/power_ups.rs`.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

struct GoldShine {
    // Overall strength of the made-up reflection.
    strength: f32,
    // How bright the sky above the horizon / the floor below it reflect.
    sky: f32,
    ground: f32,
    // How sharp the horizon line is (bigger = crisper).
    horizon: f32,
    // The bright lights round it: brightness, how many, and how narrow.
    streaks: f32,
    streak_count: f32,
    streak_width: f32,
    // The rim round its edge: strength and how tight to the edge.
    rim: f32,
    rim_power: f32,
    // How busy the scene is: the skyline, its windows and the sky's
    // mottling (0 = a plain studio).
    detail: f32,
}
@group(2) @binding(100) var<uniform> shine: GoldShine;

const PI: f32 = 3.14159265;

fn hash1(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise.
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2(1.0, 0.0));
    let c = hash2(i + vec2(0.0, 1.0));
    let d = hash2(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// The scene's brightness seen along world direction `r`.
fn studio(r: vec3<f32>) -> f32 {
    let el = r.y;
    // Round the horizon, 0..1.
    let around = atan2(r.z, r.x) / (2.0 * PI) + 0.5;
    let detail = shine.detail;

    // The sky: brightest low down, at the horizon, mottled like cloud.
    let cloud = noise(vec2(around * 14.0, el * 7.0)) * 0.6 + noise(vec2(around * 37.0, el * 19.0)) * 0.4;
    let mottle = mix(1.0, 0.15 + 1.7 * cloud, clamp(detail, 0.0, 1.0) * 0.85);
    var light = shine.sky * (0.15 + 0.85 * exp(-max(el, 0.0) * 4.0)) * mottle;

    // A skyline: dark blocks of different heights standing on the horizon,
    // a few of their windows lit.
    let cols = 30.0;
    let cell = floor(around * cols);
    let top = (0.03 + hash1(cell) * 0.3) * detail;
    let building = step(el, top) * step(hash1(cell + 17.0), 0.75);
    let window = step(0.85, hash2(vec2(cell * 3.0 + floor(fract(around * cols) * 3.0), floor(el * 45.0))));
    let facade = shine.ground * 2.0 + window * shine.sky * 0.8;
    light = mix(light, facade, building * clamp(detail, 0.0, 1.0));

    // A few bright lights round it, low in the sky (gone overhead, where
    // they'd all meet).
    let band = 0.5 + 0.5 * cos(around * 2.0 * PI * max(shine.streak_count, 1.0));
    light += shine.streaks * pow(band, max(shine.streak_width, 1.0)) * smoothstep(-0.05, 0.2, el)
        * (1.0 - smoothstep(0.35, 0.7, el));

    // Below a sharp horizon, the near-black floor (a little uneven).
    let floor_light = shine.ground * (0.4 + 1.2 * noise(vec2(around * 20.0, el * 10.0)));
    let h = clamp(el * shine.horizon, -1.0, 1.0) * 0.5 + 0.5;
    return mix(floor_light, light, h);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    let n = normalize(pbr_input.N);
    let v = normalize(pbr_input.V);
    let n_dot_v = clamp(dot(n, v), 0.0, 1.0);
    // (Its hue at full strength — how bright it reflects is `strength`'s.)
    let base = pbr_input.material.base_color.rgb;
    let gold = base / max(max(base.r, base.g), max(base.b, 1e-4));
    // Metal reflects in its own colour, going whiter at grazing angles.
    let fresnel = gold + (vec3(1.0) - gold) * pow(1.0 - n_dot_v, 5.0);
    let r = reflect(-v, n);
    let reflected = fresnel * studio(r) * shine.strength;
    let rim = gold * shine.rim * pow(1.0 - n_dot_v, max(shine.rim_power, 0.1));
    out.color = vec4(out.color.rgb + reflected + rim, out.color.a);

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
