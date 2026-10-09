// The minimap (`client/src/minimap/mod.rs`): the map's baked top-down
// picture (`minimap/bake.rs`) in a rounded square, centred on us and turned
// with us so straight ahead is always up — our view cone fanning out ahead
// and our arrow in the middle. Driven by `MinimapMaterial`.
//
// The picture holds heights, not colours: each spot's shaded by how high it
// is against where we stand, so the level we're on always reads as the
// floor, whatever the map's levels.

#import bevy_ui::ui_vertex_output::UiVertexOutput

// x, y: the world (x, z) the view's centred on; z: our heading (radians,
// clockwise from north = -Z); w: how far (m) it reaches from the centre to
// an edge.
@group(1) @binding(0) var<uniform> view: vec4<f32>;
// The picture's world rect: its top-left (x, z), then its size (m).
@group(1) @binding(1) var<uniform> rect: vec4<f32>;
@group(1) @binding(2) var map_texture: texture_2d<f32>;
@group(1) @binding(3) var map_sampler: sampler;
// x: the picture's lowest height; y: how much higher its highest is (m);
// z: the height our feet are at.
@group(1) @binding(4) var<uniform> heights: vec4<f32>;

const BACKGROUND: vec3<f32> = vec3<f32>(0.012, 0.014, 0.017);
const PIT: vec3<f32> = vec3<f32>(0.007, 0.009, 0.01);
const FLOOR: vec3<f32> = vec3<f32>(0.019, 0.024, 0.03);
const COVER: vec3<f32> = vec3<f32>(0.062, 0.073, 0.084);
const BUILDING: vec3<f32> = vec3<f32>(0.147, 0.162, 0.181);
const TALL: vec3<f32> = vec3<f32>(0.223, 0.246, 0.266);
const OUTLINE: vec3<f32> = vec3<f32>(0.61, 0.66, 0.7);
const BORDER: vec3<f32> = vec3<f32>(0.75, 0.78, 0.8);
const ARROW: vec3<f32> = vec3<f32>(1.0, 0.86, 0.25);
// px
const CORNER: f32 = 14.0;
const BORDER_WIDTH: f32 = 2.0;
const OPACITY: f32 = 0.92;
// Our view cone: its half-angle (radians) and strength.
const CONE_HALF: f32 = 0.6;
const CONE_ALPHA: f32 = 0.16;

// The shade of a surface `rel` metres over our feet (linear colour).
fn shade(rel: f32) -> vec3<f32> {
    if (rel < -0.4) {
        return mix(FLOOR, PIT, smoothstep(-0.4, -4.0, rel));
    }
    if (rel < 0.4) {
        return FLOOR;
    }
    if (rel < 1.5) {
        return mix(FLOOR, COVER, smoothstep(0.4, 1.5, rel));
    }
    if (rel < 3.5) {
        return mix(COVER, BUILDING, smoothstep(1.5, 3.5, rel));
    }
    return mix(BUILDING, TALL, smoothstep(3.5, 10.0, rel));
}

// Signed distance from `p` to the triangle `a b c` (negative inside).
fn sd_triangle(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let e0 = b - a;
    let e1 = c - b;
    let e2 = a - c;
    let v0 = p - a;
    let v1 = p - b;
    let v2 = p - c;
    let q0 = v0 - e0 * clamp(dot(v0, e0) / dot(e0, e0), 0.0, 1.0);
    let q1 = v1 - e1 * clamp(dot(v1, e1) / dot(e1, e1), 0.0, 1.0);
    let q2 = v2 - e2 * clamp(dot(v2, e2) / dot(e2, e2), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d = min(
        min(
            vec2<f32>(dot(q0, q0), s * (v0.x * e0.y - v0.y * e0.x)),
            vec2<f32>(dot(q1, q1), s * (v1.x * e1.y - v1.y * e1.x)),
        ),
        vec2<f32>(dot(q2, q2), s * (v2.x * e2.y - v2.y * e2.x)),
    );
    return -sqrt(d.x) * sign(d.y);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // px from the centre, y down.
    let p = (in.uv - 0.5) * in.size;
    let half = 0.5 * min(in.size.x, in.size.y);

    // The rounded square.
    let q = abs(p) - vec2<f32>(half - CORNER);
    let edge = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - CORNER;
    let inside = 1.0 - smoothstep(-1.0, 0.0, edge);
    if (inside <= 0.0) {
        discard;
    }

    // The world under this pixel: right is our right, up is ahead.
    let heading = view.z;
    let ahead = vec2<f32>(sin(heading), -cos(heading));
    let right = vec2<f32>(cos(heading), sin(heading));
    let s = p / half;
    let world = view.xy + (right * s.x - ahead * s.y) * view.w;
    let uv = (world - rect.xy) / rect.zw;
    // (Level 0: the picture has no mips, and after the `discard` above a
    // plain `textureSample` isn't allowed.)
    let tex = textureSampleLevel(map_texture, map_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0);
    let on_map = select(0.0, 1.0, all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)));
    // (Its channels are premultiplied by coverage once filtered, so
    // they're divided back out.)
    let cover = tex.a * on_map;
    let unmul = tex.rgb / max(tex.a, 0.001);
    let height = heights.x + unmul.r * heights.y;
    var ground = shade(height - heights.z) * (unmul.b * 2.0);
    ground = mix(ground, OUTLINE, clamp(unmul.g, 0.0, 1.0));
    var rgb = mix(BACKGROUND, ground, cover);

    // Our view cone, fading out with distance.
    let r = length(p) / half;
    let angle = abs(atan2(p.x, -p.y));
    let cone = (1.0 - smoothstep(CONE_HALF - 0.05, CONE_HALF, angle)) * (1.0 - smoothstep(0.1, 0.95, r));
    rgb = mix(rgb, vec3<f32>(1.0), cone * CONE_ALPHA);

    // Our arrow, pointing up, with a dark rim.
    let arrow = min(
        sd_triangle(p, vec2<f32>(0.0, -9.0), vec2<f32>(-7.0, 7.0), vec2<f32>(0.0, 3.0)),
        sd_triangle(p, vec2<f32>(0.0, -9.0), vec2<f32>(0.0, 3.0), vec2<f32>(7.0, 7.0)),
    );
    rgb = mix(rgb, vec3<f32>(0.0), 1.0 - smoothstep(1.0, 2.5, arrow));
    rgb = mix(rgb, ARROW, 1.0 - smoothstep(-0.5, 0.5, arrow));

    // The border.
    let border = smoothstep(-BORDER_WIDTH - 1.0, -BORDER_WIDTH, edge);
    rgb = mix(rgb, BORDER, border * 0.7);

    return vec4<f32>(rgb, inside * OPACITY);
}
