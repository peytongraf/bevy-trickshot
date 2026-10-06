// The Aether Shroud's HUD icon (`client/src/aether_shroud.rs`): the icon
// clipped to a circle inside a ring. While a charge builds, a yellow arc
// fills the ring clockwise from the top; once every charge is stored the
// whole ring is yellow; while it's up, a purple arc runs down with the time
// left. Driven by `ShroudIconMaterial`.

#import bevy_ui::ui_vertex_output::UiVertexOutput

// x: build progress (0..=1), y: opacity, z: 1 while it's up, w: what's left
// of it (0..=1).
@group(1) @binding(0) var<uniform> params: vec4<f32>;
@group(1) @binding(1) var icon_texture: texture_2d<f32>;
@group(1) @binding(2) var icon_sampler: sampler;

const TAU: f32 = 6.28318530718;
const RING_INNER: f32 = 0.84;
const ICON_RADIUS: f32 = 0.86;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // -1..1 across the node, y down.
    let p = in.uv * 2.0 - 1.0;
    let r = length(p);
    // About one pixel, for soft edges.
    let px = 2.0 / max(in.size.x, 1.0);
    let opacity = params.y;

    // The icon, filling the inner circle.
    let icon_uv = p / ICON_RADIUS * 0.5 + 0.5;
    var icon = textureSample(icon_texture, icon_sampler, clamp(icon_uv, vec2<f32>(0.0), vec2<f32>(1.0)));
    icon.a = icon.a * (1.0 - smoothstep(ICON_RADIUS - px, ICON_RADIUS + px, r));

    // The ring: how far round from the top, clockwise (0..1).
    var a = atan2(p.x, -p.y);
    if (a < 0.0) {
        a = a + TAU;
    }
    let frac = a / TAU;
    let in_ring = smoothstep(RING_INNER - px, RING_INNER + px, r) * (1.0 - smoothstep(1.0 - 2.0 * px, 1.0, r));
    let is_up = params.z > 0.5;
    let filled = select(params.x, params.w, is_up);
    let fill_color = select(vec3<f32>(1.0, 0.62, 0.02), vec3<f32>(0.55, 0.2, 1.0), is_up);
    let lit = step(frac, filled) * step(0.0001, filled);
    let ring = mix(vec4<f32>(0.0, 0.0, 0.0, 0.7), vec4<f32>(fill_color, 1.0), lit);

    // Ring over icon.
    let ring_a = ring.a * in_ring;
    let rgb = mix(icon.rgb, ring.rgb, ring_a);
    let alpha = ring_a + icon.a * (1.0 - ring_a);
    return vec4<f32>(rgb, alpha * opacity);
}
