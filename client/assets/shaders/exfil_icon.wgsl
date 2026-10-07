// The exfil icon (`client/src/exfil.rs`): the round white helicopter icon,
// clipped clean to its circle, inside a ring. While interact's held at the
// radio, a blue arc fills the ring clockwise from the top. Driven by
// `ExfilIconMaterial`.

#import bevy_ui::ui_vertex_output::UiVertexOutput

// x: progress (0..=1), y: opacity, z: 1 to draw the ring at all.
@group(1) @binding(0) var<uniform> params: vec4<f32>;
@group(1) @binding(1) var icon_texture: texture_2d<f32>;
@group(1) @binding(2) var icon_sampler: sampler;

const TAU: f32 = 6.28318530718;
const RING_INNER: f32 = 0.86;
const ICON_RADIUS: f32 = 0.8;
// The icon's circle fills this much of its (square) image.
const ICON_FILL: f32 = 0.99;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // -1..1 across the node, y down.
    let p = in.uv * 2.0 - 1.0;
    let r = length(p);
    // About one pixel, for soft edges.
    let px = 2.0 / max(in.size.x, 1.0);
    let opacity = params.y;
    let has_ring = params.z > 0.5;
    let icon_radius = select(1.0 - px, ICON_RADIUS, has_ring);

    // The icon: its art over white, so the circle's ragged edge comes out
    // a clean disc.
    let icon_uv = p / icon_radius * ICON_FILL * 0.5 + 0.5;
    let tex = textureSample(icon_texture, icon_sampler, clamp(icon_uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    let icon_rgb = mix(vec3<f32>(1.0), tex.rgb, tex.a);
    let icon_a = 1.0 - smoothstep(icon_radius - px, icon_radius + px, r);

    // The ring: how far round from the top, clockwise (0..1).
    var a = atan2(p.x, -p.y);
    if (a < 0.0) {
        a = a + TAU;
    }
    let frac = a / TAU;
    let in_ring = select(
        0.0,
        smoothstep(RING_INNER - px, RING_INNER + px, r) * (1.0 - smoothstep(1.0 - 2.0 * px, 1.0, r)),
        has_ring,
    );
    let lit = step(frac, params.x) * step(0.0001, params.x);
    let ring = mix(vec4<f32>(0.0, 0.0, 0.0, 0.6), vec4<f32>(0.18, 0.58, 1.0, 1.0), lit);

    // Ring over icon.
    let ring_a = ring.a * in_ring;
    let rgb = mix(icon_rgb, ring.rgb, ring_a);
    let alpha = ring_a + icon_a * (1.0 - ring_a);
    return vec4<f32>(rgb, alpha * opacity);
}
