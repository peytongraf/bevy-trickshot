//! Drawing a map's top-down picture from its collision model's triangles —
//! nothing hand-made: every map gets one the moment it loads.
//!
//! The triangles are rasterised from above into a height map (the highest
//! surface over each texel). The picture keeps, per texel, what the
//! minimap's shader (`shaders/minimap.wgsl`) needs to shade it against the
//! height *we're* at — so whatever level we stand on reads as the floor,
//! cover and buildings lighter, drops darker, on a map of any number of
//! levels:
//!
//! * red: its height, across [`BakedMap::heights`]' range;
//! * green: whether it's an edge — the height drops away by more than
//!   [`EDGE_M`] close by (a building's side, a wall, the map's own edge) —
//!   outlined on the higher side, Call of Duty's minimap look;
//! * blue: a soft light on slopes and ramps, from the north-west (half of
//!   it, `0.5` = flat);
//! * alpha: whether there's anything there at all (off the map is clear).
//!
//! Row `j`, column `i` of the image is world `z`, `x` — so north (-Z) is the
//! top and east (+X) the right, as on the compass.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// How many texels a metre gets...
const PX_PER_M: f32 = 6.0;
/// ...unless that would make a side longer than this.
const MAX_SIDE: u32 = 2048;
/// Clear space (m) left round the map's edge.
const PAD_M: f32 = 2.0;
/// A drop of more than this (m) to a neighbour is an edge — outlined.
const EDGE_M: f32 = 0.6;
/// How wide (m) an edge's outline is.
const OUTLINE_M: f32 = 0.3;
/// Where the slope light comes from (north-west, up high).
const LIGHT: Vec3 = Vec3::new(-0.5, 0.75, -0.5);
/// How strongly slopes are lit.
const SLOPE_SHADE: f32 = 0.9;

/// A map's top-down picture and the stretch of the world it covers.
pub(crate) struct BakedMap {
    pub(crate) image: Image,
    /// The world `(x, z)` of its top-left corner...
    pub(crate) min: Vec2,
    /// ...and how far (m) it reaches east and south.
    pub(crate) size: Vec2,
    /// The lowest height in it, and how much higher (m) the highest is — red
    /// `0..1` spans this.
    pub(crate) heights: Vec2,
}

/// The picture of the world-space `triangles`; `None` if there are none.
pub(crate) fn bake(triangles: &[[Vec3; 3]]) -> Option<BakedMap> {
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for v in triangles.iter().flatten() {
        lo = lo.min(v.xz());
        hi = hi.max(v.xz());
    }
    if !(lo.x < hi.x && lo.y < hi.y) {
        return None;
    }
    let min = lo - Vec2::splat(PAD_M);
    let size = hi - lo + Vec2::splat(2.0 * PAD_M);
    let ppm = PX_PER_M.min(MAX_SIDE as f32 / size.max_element());
    let (w, h) = ((size.x * ppm).ceil() as usize, (size.y * ppm).ceil() as usize);
    // (What the image really spans, once rounded up to whole texels.)
    let size = Vec2::new(w as f32, h as f32) / ppm;

    let heights = rasterise(triangles, min, ppm, w, h);
    let finite = heights.iter().copied().filter(|y| y.is_finite());
    let lowest = finite.clone().fold(f32::INFINITY, f32::min);
    let range = (finite.fold(f32::NEG_INFINITY, f32::max) - lowest).max(0.01);
    let lowest_near = neighbourhood_min(&heights, w, h, ((OUTLINE_M * ppm).round() as usize).max(1));

    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let mut data = Vec::with_capacity(w * h * 4);
    for j in 0..h {
        for i in 0..w {
            let k = j * w + i;
            let height = heights[k];
            if height == f32::NEG_INFINITY {
                data.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let edge = height - lowest_near[k] > EDGE_M;
            data.extend_from_slice(&[
                byte((height - lowest) / range),
                if edge { 255 } else { 0 },
                byte(slope_light(&heights, w, h, i, j, ppm) * 0.5),
                255,
            ]);
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: w as u32,
            height: h as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        // (Not sRGB: these are numbers, not colours.)
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    Some(BakedMap {
        image,
        min,
        size,
        heights: Vec2::new(lowest, range),
    })
}

/// The highest surface over each texel (`NEG_INFINITY` where there's none):
/// every triangle filled in from above, and its edges traced too — so a wall
/// that's only an upright plane, with no top to fill, still shows.
fn rasterise(triangles: &[[Vec3; 3]], min: Vec2, ppm: f32, w: usize, h: usize) -> Vec<f32> {
    let mut heights = vec![f32::NEG_INFINITY; w * h];
    // Texel space: texel (i, j)'s centre is at (i, j).
    let to_px = |v: Vec3| (v.xz() - min) * ppm - Vec2::splat(0.5);
    let mut raise = |i: i64, j: i64, y: f32| {
        if i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h {
            let k = j as usize * w + i as usize;
            heights[k] = heights[k].max(y);
        }
    };
    for tri in triangles {
        let p = tri.map(to_px);
        let y = tri.map(|v| v.y);
        let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if area.abs() > 1e-6 {
            let lo = p[0].min(p[1]).min(p[2]).ceil();
            let hi = p[0].max(p[1]).max(p[2]).floor();
            for j in lo.y as i64..=hi.y as i64 {
                for i in lo.x as i64..=hi.x as i64 {
                    let q = Vec2::new(i as f32, j as f32);
                    let w0 = (p[2] - p[1]).perp_dot(q - p[1]) / area;
                    let w1 = (p[0] - p[2]).perp_dot(q - p[2]) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                        raise(i, j, w0 * y[0] + w1 * y[1] + w2 * y[2]);
                    }
                }
            }
        }
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let steps = ((p[b] - p[a]).length() * 2.0).ceil().max(1.0) as usize;
            for s in 0..=steps {
                let t = s as f32 / steps as f32;
                let q = p[a].lerp(p[b], t).round();
                raise(q.x as i64, q.y as i64, y[a] + (y[b] - y[a]) * t);
            }
        }
    }
    heights
}

/// The lowest height within `r` texels of each texel (a square around it;
/// off the image counts as nothing there).
fn neighbourhood_min(heights: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut rows = vec![f32::NEG_INFINITY; w * h];
    for j in 0..h {
        for i in 0..w {
            let edge = i < r || i + r >= w;
            let lo = (i.saturating_sub(r)..=(i + r).min(w - 1))
                .map(|x| heights[j * w + x])
                .fold(f32::INFINITY, f32::min);
            rows[j * w + i] = if edge { f32::NEG_INFINITY } else { lo };
        }
    }
    let mut out = vec![f32::NEG_INFINITY; w * h];
    for j in 0..h {
        for i in 0..w {
            if j < r || j + r >= h {
                continue;
            }
            out[j * w + i] = (j - r..=j + r).map(|y| rows[y * w + i]).fold(f32::INFINITY, f32::min);
        }
    }
    out
}

/// How lit a texel is by its slope (`1` flat): only across smooth ground —
/// across an edge it's left flat, the outline marks it instead.
fn slope_light(heights: &[f32], w: usize, h: usize, i: usize, j: usize, ppm: f32) -> f32 {
    let at = |x: usize, y: usize| heights[y * w + x];
    let here = at(i, j);
    let slope = |a: f32, b: f32| {
        let smooth = a.is_finite() && b.is_finite() && (a - here).abs() < EDGE_M && (b - here).abs() < EDGE_M;
        if smooth {
            (b - a) * ppm * 0.5
        } else {
            0.0
        }
    };
    let dx = if i > 0 && i + 1 < w { slope(at(i - 1, j), at(i + 1, j)) } else { 0.0 };
    let dz = if j > 0 && j + 1 < h { slope(at(i, j - 1), at(i, j + 1)) } else { 0.0 };
    let normal = Vec3::new(-dx, 1.0, -dz).normalize();
    let light = LIGHT.normalize();
    (1.0 + SLOPE_SHADE * (normal.dot(light) - light.y)).clamp(0.55, 1.4)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 20 m square of ground with a 4 m tall, 4 m wide block in the middle.
    fn ground_and_block() -> Vec<[Vec3; 3]> {
        let quad = |y: f32, a: Vec2, b: Vec2| {
            let (p0, p1, p2, p3) = (
                Vec3::new(a.x, y, a.y),
                Vec3::new(b.x, y, a.y),
                Vec3::new(b.x, y, b.y),
                Vec3::new(a.x, y, b.y),
            );
            [[p0, p1, p2], [p0, p2, p3]]
        };
        let mut tris = Vec::new();
        tris.extend(quad(0.0, Vec2::splat(-10.0), Vec2::splat(10.0)));
        tris.extend(quad(4.0, Vec2::splat(-2.0), Vec2::splat(2.0)));
        tris
    }

    fn texel(baked: &BakedMap, x: f32, z: f32) -> [u8; 4] {
        let w = baked.image.width() as f32;
        let h = baked.image.height() as f32;
        let i = ((x - baked.min.x) / baked.size.x * w) as usize;
        let j = ((z - baked.min.y) / baked.size.y * h) as usize;
        let k = (j * w as usize + i) * 4;
        let d = baked.image.data.as_ref().unwrap();
        [d[k], d[k + 1], d[k + 2], d[k + 3]]
    }

    #[test]
    fn block_is_higher_than_ground_and_outlined() {
        let baked = bake(&ground_and_block()).unwrap();
        let ground = texel(&baked, -6.0, -6.0);
        let block = texel(&baked, 0.0, 0.0);
        let edge = texel(&baked, 1.95, 0.0);
        let off_map = texel(&baked, -11.5, -11.5);
        assert_eq!(baked.heights, Vec2::new(0.0, 4.0));
        assert_eq!((ground[0], ground[1], ground[3]), (0, 0, 255));
        assert_eq!((block[0], block[1]), (255, 0));
        assert_eq!(edge[1], 255);
        assert_eq!(off_map[3], 0);
    }
}
