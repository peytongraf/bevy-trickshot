// The Rampage Inducer's icon (`client/src/rampage.rs`): the round skull
// icon, clipped clean to its circle, inside a ring — like the exfil's
// (`exfil_icon.wgsl`), but the source picture is wider than it's tall, its
// circle in the middle of it (`uv_circle`). While interact's held at the
// inducer, a blue arc fills the ring clockwise from the top. Driven by
// `RampageIconMaterial`.

#import bevy_ui::ui_vertex_output::UiVertexOutput

// x: progress (0..=1), y: opacity, z: 1 to draw the ring at all.
@group(1) @binding(0) var<uniform> params: vec4<f32>;
// Where the icon's circle is in its picture (uv): its centre, then its
// radius across and down.
@group(1) @binding(1) var<uniform> uv_circle: vec4<f32>;
@group(1) @binding(2) var icon_texture: texture_2d<f32>;
@group(1) @binding(3) var icon_sampler: sampler;

const TAU: f32 = 6.28318530718;
const RING_INNER: f32 = 0.86;
const ICON_RADIUS: f32 = 0.8;
const RING_BLUE: vec3<f32> = vec3<f32>(0.18, 0.58, 1.0);

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

    // The icon: its circle mapped onto ours.
    let icon_uv = uv_circle.xy + (p / icon_radius) * uv_circle.zw;
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
    let ring = mix(vec4<f32>(0.0, 0.0, 0.0, 0.6), vec4<f32>(RING_BLUE, 1.0), lit);

    // Ring over icon.
    let ring_a = ring.a * in_ring;
    let rgb = mix(icon_rgb, ring.rgb, ring_a);
    let alpha = ring_a + icon_a * (1.0 - ring_a);
    return vec4<f32>(rgb, alpha * opacity);
}
