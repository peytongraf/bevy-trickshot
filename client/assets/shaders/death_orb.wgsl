// The Hierophant's ball of dark energy: a camera-facing quad (turned by its
// transform) drawn premultiplied, so it can both darken and glow. In the
// middle a sphere of near-black purple, its darkness slowly churning, lit
// to a bright purple round its rim (brighter the more edge-on); round it a
// purple glow; and red lightning crackling through it and out past its
// edge, re-forking many times a second. Driven by `DeathOrbMaterial` in
// `client/src/boss.rs`.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::globals,
}

struct OrbParams {
    // Linear rgb: the sphere's darkness...
    dark: vec4<f32>,
    // ...its rim and the glow round it (a = brightness)...
    rim: vec4<f32>,
    // ...and the lightning (a = brightness).
    bolt: vec4<f32>,
    // The sphere's radius as a share of the quad's half-width.
    core: f32,
    // How bright the glow round it is.
    glow: f32,
    // How fast its darkness churns.
    swirl: f32,
    // How much lightning (0 = none), and how often it re-forks (Hz).
    bolts: f32,
    bolt_hz: f32,
    // Its seed (so no two look alike).
    seed: f32,
    _pad0: f32,
    _pad1: f32,
}
@group(2) @binding(0) var<uniform> orb: OrbParams;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
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
        q = q * 2.07 + vec2(3.1, 1.7);
        amp *= 0.5;
    }
    return sum;
}

// Thin, jagged lines where the noise crosses its middle — lightning.
fn bolts(p: vec2<f32>, beat: f32) -> f32 {
    let n = fbm(p + vec2(beat * 7.13, beat * 3.71));
    let line = 1.0 - abs(n * 2.0 - 1.0);
    return pow(clamp(line, 0.0, 1.0), 28.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = globals.time;
    let p = in.uv * 2.0 - 1.0;
    let r = length(p);
    let core = max(orb.core, 0.05);
    let x = r / core;

    // The sphere: its "normal" facing us at the middle, edge-on at its rim.
    let inside = 1.0 - smoothstep(0.97, 1.0, x);
    let facing = sqrt(max(1.0 - min(x, 1.0) * min(x, 1.0), 0.0));
    let fresnel = pow(1.0 - facing, 3.0);

    // Its darkness churning: noise in a frame turning slowly about its
    // middle, pulled toward it (as if it's drawing everything in).
    let a = t * orb.swirl;
    let rot = mat2x2<f32>(cos(a), -sin(a), sin(a), cos(a));
    let q = rot * (p / core) * (1.0 + 0.35 * facing);
    let churn = fbm(q * 2.3 + vec2(orb.seed, t * 0.4));
    let deep = orb.dark.rgb * (0.35 + 1.3 * churn);

    // Lightning, re-forking `bolt_hz` a second (some beats darker than
    // others): through the sphere and out a little past it.
    let beat = floor(t * orb.bolt_hz) + orb.seed * 13.0;
    let flash = 0.45 + 0.55 * hash2(vec2(beat, orb.seed));
    let reach = 1.0 - smoothstep(1.0, 1.45, x);
    let bolt = (bolts(p / core * 1.6, beat) + 0.6 * bolts(p / core * 3.1 + 5.0, beat + 0.5))
        * reach * flash * orb.bolts;

    // The rim, and the glow round it: brightest at its edge, falling away
    // outside (and a fainter one well out).
    let out = max(x - 1.0, 0.0);
    let halo = exp(-out * 6.0) * 0.8 + exp(-out * 1.8) * 0.35;
    let glow = (1.0 - inside) * halo * (1.0 - smoothstep(0.85, 1.0, r)) * orb.glow;
    let rim = inside * fresnel;

    let rim_light = orb.rim.rgb * orb.rim.a * (rim + glow);
    let bolt_light = orb.bolt.rgb * orb.bolt.a * bolt;
    // Premultiplied (`AlphaMode::Premultiplied`): the sphere covers what's
    // behind it (alpha), everything else just adds light on top.
    let rgb = deep * inside * (1.0 - rim) + rim_light + bolt_light;
    return vec4<f32>(rgb, inside * 0.97);
}
