// Yellow lightning crackling behind the round number while the Rampage
// Inducer's on (`client/src/rampage.rs`, `RampageLightningMaterial`): a few
// jagged bolts across the panel, each reshaping now and then and flickering
// while it lasts, a bright core in a soft glow — fading out toward the
// panel's edges, and in and out as a whole with the inducer.

#import bevy_ui::ui_vertex_output::UiVertexOutput

// x: time (s); y: fade (0..1); z: brightness; w: how many bolts (up to 6).
@group(1) @binding(0) var<uniform> params: vec4<f32>;
// rgb: colour; w: glow size (px).
@group(1) @binding(1) var<uniform> colour: vec4<f32>;
// x: reshapes a second; y: how jagged (a share of a bolt's length);
// z: core thickness (px); w: how much of the panel's edge fades out (0..1).
@group(1) @binding(2) var<uniform> look: vec4<f32>;

const PI: f32 = 3.14159265;
const MAX_BOLTS: i32 = 6;

fn hash(n: f32) -> f32 {
    return fract(sin(n * 12.9898 + 78.233) * 43758.5453);
}

// Smooth 1D value noise in -1..1.
fn noise(x: f32, seed: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(hash(i + seed * 17.13), hash(i + 1.0 + seed * 17.13), u) * 2.0 - 1.0;
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let fade = params.y;
    if (fade <= 0.001) {
        discard;
    }
    let size = in.size;
    let px = in.uv * size;
    let centre = size * 0.5;
    let reach = size * 0.46;
    let glow_px = max(colour.w, 0.5);
    let core_px = max(look.z, 0.3);

    var core = 0.0;
    var glow = 0.0;
    let bolts = i32(clamp(params.w, 0.0, f32(MAX_BOLTS)));
    for (var i = 0; i < MAX_BOLTS; i = i + 1) {
        if (i >= bolts) {
            break;
        }
        let fi = f32(i);
        // Each bolt keeps a shape for one slot, its slots out of step.
        let t_slots = params.x * look.x + hash(fi * 3.7) * 10.0;
        let slot = floor(t_slots);
        let life = fract(t_slots);
        let seed = slot * 13.1 + fi * 7.3;
        // From one side of the number, across behind it to roughly the other.
        let a1 = hash(seed) * 2.0 * PI;
        let a2 = a1 + PI + (hash(seed + 1.0) - 0.5) * 1.8;
        let from = centre + vec2<f32>(cos(a1), sin(a1)) * reach * (0.55 + 0.45 * hash(seed + 2.0));
        let to = centre + vec2<f32>(cos(a2), sin(a2)) * reach * (0.55 + 0.45 * hash(seed + 3.0));
        let ab = to - from;
        let len = max(length(ab), 1.0);
        let dir = ab / len;
        let side = vec2<f32>(-dir.y, dir.x);
        let rel = px - from;
        let t = dot(rel, dir) / len;
        if (t < 0.0 || t > 1.0) {
            continue;
        }
        // Jagged: a few octaves of noise across it, pinned at its ends.
        let jag = (noise(t * 5.0, seed) * 0.6 + noise(t * 13.0, seed + 5.0) * 0.3 + noise(t * 31.0, seed + 9.0) * 0.12)
            * look.y * len * sin(t * PI);
        let d = abs(dot(rel, side) - jag);
        // Flickers while it lasts, dimming toward its end.
        let flicker = (0.55 + 0.45 * hash(seed + floor(life * 9.0))) * (1.0 - life * 0.6);
        core = core + (1.0 - smoothstep(core_px * 0.5, core_px, d)) * flicker;
        glow = glow + exp(-d / glow_px) * flicker;
    }

    // Faded out toward the panel's edges.
    let e = length((px - centre) / max(centre, vec2<f32>(1.0)));
    let edge = 1.0 - smoothstep(1.0 - look.w, 1.0, e);
    let core_c = clamp(core, 0.0, 1.0);
    let glow_c = clamp(glow * 0.55, 0.0, 1.0);
    let rgb = mix(colour.rgb, vec3<f32>(1.0, 1.0, 0.92), core_c * 0.7) * params.z;
    let alpha = clamp(core_c + glow_c, 0.0, 1.0) * fade * edge;
    return vec4<f32>(rgb, alpha);
}
