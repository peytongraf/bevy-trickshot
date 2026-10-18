// A pool's surface (`shared::level::Water`): see-through — the floor under
// it shows through, bent by the surface and tinted the deeper the light
// goes — and rippled: a few octaves of gradient noise drifting in different
// directions, so the torches' highlights glitter and the floor below
// wobbles. Rings spread out from wherever a waterfall lands (`impacts`),
// churned white with foam that drifts away from it. The light the waves
// bend plays over the floor as caustics: the floor seen through it is
// brightened in a moving net of lines, where the bent view ray meets it.
//
// Anyone wading through it, falling into it or blowing it up leaves
// `ripples`: rings running out from where they were, churned white at
// first. And wherever anything stands in it — a leg, a body, the pool's
// own walls — a thin line of foam clings to it right at the waterline.
//
// The see-through part is done here rather than by Bevy (the material is
// still transmissive, so the scene behind is copied for it to sample, but
// Bevy's own share is switched off): Bevy samples the screen where the bent
// ray ends up without checking whether something nearer covers that spot,
// so a zombie standing in it showed up again as a tinted ghost beside
// itself. Here, with the camera's depth prepass, how deep the water really
// is at each pixel (to the floor, or to a leg in it) sets the bend, the
// tint and the waterline, and a bent sample that lands on something in
// front of the water falls back to the straight one. (A view without the
// prepass — the scope — guesses from `depth` instead.)
// Driven by `WaterMaterial` in `client/src/water.rs`.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{globals, view, view_transmission_texture, view_transmission_sampler},
    view_transformations::{frag_coord_to_uv, uv_to_ndc, ndc_to_uv, position_ndc_to_world, position_world_to_ndc},
}
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils
#endif

const MAX_IMPACTS: u32 = 8u;
const MAX_RIPPLES: u32 = 64u;
// How long (s) a ripple lasts, and how fast (m/s) its ring spreads.
const RIPPLE_LIFE: f32 = 2.4;
const RIPPLE_SPEED: f32 = 0.55;
// Its crests per metre (radians), and how steep they are.
const RIPPLE_K: f32 = 38.0;
const RIPPLE_SLOPE: f32 = 0.55;
// How far (m) up a leg / wall the waterline's foam reaches, at most.
const WATERLINE: f32 = 0.045;

struct WaterParams {
    // Where each waterfall lands (xyz, world) and how wide its splash is (w).
    impacts: array<vec4<f32>, 8>,
    // x: how hard it churns there (the waterfall's spray).
    impact_power: array<vec4<f32>, 8>,
    // How rough the surface is.
    waves: f32,
    // How bright the caustics are.
    caustics: f32,
    // How deep (m) the floor seems — where the caustics land.
    depth: f32,
    impact_count: u32,
    // Each ripple: where (x, z), when it started (`globals.time`) and how
    // strong it is (0 = none).
    ripples: array<vec4<f32>, 64>,
}
@group(2) @binding(100) var<uniform> water: WaterParams;

fn hash2(p: vec2<f32>) -> vec2<f32> {
    let q = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
    return fract(sin(q) * 43758.5453) * 2.0 - 1.0;
}

// Gradient noise, -1..1-ish.
fn gnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = dot(hash2(i), f);
    let b = dot(hash2(i + vec2(1.0, 0.0)), f - vec2(1.0, 0.0));
    let c = dot(hash2(i + vec2(0.0, 1.0)), f - vec2(0.0, 1.0));
    let d = dot(hash2(i + vec2(1.0, 1.0)), f - vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn rot(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2(c * p.x - s * p.y, s * p.x + c * p.y);
}

// The surface's height (m-ish) at `p` (world xz) at time `t`.
fn height(p: vec2<f32>, t: f32) -> f32 {
    // Slow swells crossing each other, then finer, quicker ripples.
    var h = gnoise(p * 0.45 + vec2(t * 0.09, t * 0.05)) * 0.5;
    h += gnoise(rot(p, 1.1) * 1.1 + vec2(-t * 0.16, t * 0.1)) * 0.3;
    h += gnoise(rot(p, 2.3) * 2.7 + vec2(t * 0.31, -t * 0.22)) * 0.13;
    h += gnoise(rot(p, 0.6) * 6.1 + vec2(-t * 0.55, -t * 0.4)) * 0.06;
    h *= water.waves;
    // Where a waterfall lands: rings running out from it, and a churning
    // boil right under it.
    for (var i = 0u; i < min(water.impact_count, MAX_IMPACTS); i++) {
        let c = water.impacts[i];
        let power = water.impact_power[i].x;
        let r = max(c.w, 0.1);
        let d = distance(p, c.xz);
        let rings = sin(d * 11.0 - t * 8.5) * exp(-d / (r * 2.2)) * smoothstep(0.0, r * 0.5, d);
        let boil = gnoise(p * 5.0 + vec2(t * 2.3, -t * 1.9)) * exp(-(d * d) / (r * r));
        h += power * (rings * 0.05 + boil * 0.18);
    }
    return h;
}

// Foam drifting out from each waterfall's landing, 0..1.
fn foam(p: vec2<f32>, t: f32) -> f32 {
    var f = 0.0;
    for (var i = 0u; i < min(water.impact_count, MAX_IMPACTS); i++) {
        let c = water.impacts[i];
        let power = water.impact_power[i].x;
        let r = max(c.w, 0.1);
        let to = p - c.xz;
        let d = length(to);
        let out = to / max(d, 1e-3);
        // Two copies of the pattern carried outward half a cycle apart,
        // each faded out as it wraps, so it flows without ever jumping.
        let speed = 0.45;
        let pa = fract(t * speed);
        let pb = fract(t * speed + 0.5);
        let na = gnoise((p - out * pa * 1.2) * 3.2) + gnoise((p - out * pa * 1.2) * 7.5) * 0.5;
        let nb = gnoise((p - out * pb * 1.2 + 17.3) * 3.2) + gnoise((p - out * pb * 1.2 + 17.3) * 7.5) * 0.5;
        let wa = 1.0 - abs(pa * 2.0 - 1.0);
        let n = na * wa + nb * (1.0 - wa);
        let reach = 1.0 - smoothstep(r * 0.3, r * 1.9, d);
        let core = 1.0 - smoothstep(r * 0.15, r * 0.6, d);
        let patches = smoothstep(0.05 - reach * 0.3, 0.45, n);
        f = max(f, clamp(power, 0.0, 1.5) * (reach * patches + core * 0.6));
    }
    return clamp(f, 0.0, 1.0);
}

// Light focused by the waves onto the floor: a moving net of bright lines,
// 0..1.
fn caustic(uv: vec2<f32>, time: f32) -> f32 {
    let tau = 6.28318;
    var p = uv * tau;
    var i = p;
    var c = 1.0;
    let inten = 0.005;
    for (var n = 0; n < 4; n++) {
        let t = time * (1.0 - (3.5 / f32(n + 1)));
        i = p + vec2(cos(t - i.x) + sin(t + i.y), sin(t - i.y) + cos(t + i.x));
        c += 1.0 / length(vec2(p.x / (sin(i.x + t) / inten), p.y / (cos(i.y + t) / inten)));
    }
    c /= 4.0;
    c = 1.17 - pow(c, 1.4);
    return clamp(pow(abs(c), 8.0), 0.0, 1.0);
}

// Every ripple's slope at `p` (xy) and how churned the water is there (z).
fn ripples(p: vec2<f32>, t: f32) -> vec3<f32> {
    var slope = vec2(0.0);
    var churn = 0.0;
    for (var i = 0u; i < MAX_RIPPLES; i++) {
        let r = water.ripples[i];
        let age = t - r.z;
        if r.w <= 0.0 || age < 0.0 || age > RIPPLE_LIFE {
            continue;
        }
        let to = p - r.xy;
        let d = length(to);
        let radius = 0.04 + age * RIPPLE_SPEED;
        let width = 0.05 + age * 0.12;
        // Fresh, it churns the water where it started.
        churn += r.w * exp(-age * 2.2) * exp(-(d * d) / (0.05 + age * 0.12));
        let x = d - radius;
        if abs(x) > width * 3.0 {
            continue;
        }
        // A couple of crests on a ring, flattening as it spreads and dies.
        let fade = 1.0 - smoothstep(RIPPLE_LIFE * 0.5, RIPPLE_LIFE, age);
        let env = exp(-(x * x) / (width * width)) * r.w * fade / (1.0 + radius * 2.5);
        slope += env * cos(x * RIPPLE_K) * RIPPLE_SLOPE * (to / max(d, 1e-3));
    }
    return vec3(slope, churn);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    let t = globals.time;
    let surface = in.world_position.xyz;
    let p = surface.xz;
    // The surface's slope: the waves' (from their height either side), and
    // the ripples'.
    let e = 0.03;
    let h = height(p, t);
    let dx = (height(p + vec2(e, 0.0), t) - h) / e;
    let dz = (height(p + vec2(0.0, e), t) - h) / e;
    let rip = ripples(p, t);
    let n = normalize(vec3(-dx * 0.35 - rip.x, 1.0, -dz * 0.35 - rip.y));
    pbr_input.N = n;
    pbr_input.clearcoat_N = n;

    let v = normalize(pbr_input.V);
    let into = refract(-v, n, 1.0 / 1.33);
    let bend = max(-into.y, 0.2);
    let uv = frag_coord_to_uv(in.position.xy);

    // How deep the water is here: down to whatever's behind it at this
    // pixel — the floor, or a leg standing in it.
#ifdef DEPTH_PREPASS
    let behind_depth = prepass_utils::prepass_depth(in.position, 0u);
    let behind = position_ndc_to_world(vec3(uv_to_ndc(uv), max(behind_depth, 1e-7)));
    // (Nothing behind it at all: as deep as it's said to be.)
    let column = select(clamp(surface.y - behind.y, 0.0, 6.0), water.depth, behind_depth <= 0.0);
#else
    let column = water.depth;
#endif

    // The bent view ray, down to that depth: where the floor seen here is,
    // and how far the light went through the water to get there.
    let path = column / bend;
    let bed = surface + into * path;
    var seen = ndc_to_uv(position_world_to_ndc(bed).xy);
#ifdef DEPTH_PREPASS
    // (Something in front of the water there — a leg, the rim — or above
    // it: look straight through instead.)
    let seen_px = vec4(seen * view.viewport.zw + view.viewport.xy, 0.0, 0.0);
    let seen_depth = prepass_utils::prepass_depth(seen_px, 0u);
    let seen_at = position_ndc_to_world(vec3(uv_to_ndc(seen), max(seen_depth, 1e-7)));
    if seen_depth > in.position.z || seen_depth <= 0.0 || seen_at.y > surface.y + 0.01 || any(seen < vec2(0.0)) || any(seen > vec2(1.0)) {
        seen = uv;
    }
#endif
    let background = textureSampleLevel(view_transmission_texture, view_transmission_sampler, seen, 0.0).rgb;

    // Absorbed on the way down and back (Bevy's own attenuation, along this
    // pixel's path).
    let absorb = pow(1.0 - pbr_input.material.attenuation_color.rgb, vec3(2.718282))
        / max(pbr_input.material.attenuation_distance, 1e-3);
    let tint = exp(-absorb * min(path, 8.0));

    // Caustics on the floor, where the bent view ray meets it (fading out
    // in the shallowest water, right at a leg or wall).
    let c = caustic(bed.xz * 0.55, t * 0.55);
    let lit = 1.0 + water.caustics * c * 2.5 * smoothstep(0.02, 0.15, column);

    // Foam: from the waterfalls, churned up by ripples, and in a thin,
    // broken line right at the waterline on anything standing in it.
    let churn = rip.z * (0.55 + 0.45 * gnoise(p * 9.0 + vec2(t * 1.3, -t * 0.9)));
    let line_width = WATERLINE * (0.55 + 0.45 * gnoise(p * 14.0 + vec2(-t * 0.6, t * 0.8)));
#ifdef DEPTH_PREPASS
    let waterline = (1.0 - smoothstep(line_width * 0.3, line_width, column)) * 0.75;
#else
    let waterline = 0.0;
#endif
    let f = clamp(max(max(foam(p, t), smoothstep(0.12, 0.8, churn)), waterline), 0.0, 1.0);

    // Lit as just its surface (reflections and highlights, plus the foam
    // as a diffuse white) — what's seen through it is added after.
    let through = pbr_input.material.specular_transmission * (1.0 - f * 0.9);
    let water_color = pbr_input.material.base_color.rgb;
    pbr_input.material.specular_transmission = 0.0;
    pbr_input.material.diffuse_transmission = 0.0;
    pbr_input.material.base_color = vec4(vec3(0.92, 0.95, 0.95) * f, 1.0);
    pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, 0.6, f);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    // (Less comes through the more it reflects — head on, nearly all.)
    let fresnel = 0.02 + 0.98 * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
    out.color = vec4(out.color.rgb + background * water_color * tint * lit * (1.0 - fresnel) * through, 1.0);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
