//! Sounds through walls: a positional sound with the map between it and the
//! listener loses its highs (and a little level) — a zombie screaming
//! outside the temple reaches you underground dull and muffled.
//!
//! Every few frames each positional sound in earshot casts a handful of rays
//! from the listener: one at it, and a few at points around it (a spread
//! either side and above — not below, where a sound by the floor would
//! always find the floor in the way). The share blocked is how occluded it is,
//! so one just round a doorframe is only partly muffled — rays slip past the
//! edge — while one behind a whole building is fully. Only colliders block
//! (the map, doors, machines — the client has no others), and not one the
//! sound's inside of (a perk machine's own hum). Eased, so walking past a
//! doorway glides; a sound's first probe is taken as-is, so one that starts
//! behind a wall never pops in clear.
//!
//! How much a fully blocked sound loses is its [`super::mix::Profile`]'s
//! (`wall_hz`, `wall_db` — the audio mixer's profile editor), applied in
//! `apply_profiles`. The rays' settings are the debug panel's "Audio →
//! Sounds through walls".

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_rapier3d::prelude::{Collider, QueryFilter, ReadRapierContext};

use super::mix::Mix;
use super::{SpatialAudioSink, SpatialListener};

/// Panel-adjustable occlusion probing.
#[derive(Resource, Clone, Debug)]
pub(crate) struct OcclusionSettings {
    pub(crate) enabled: bool,
    /// How far (m) round the sound the side rays aim — wider lets more of a
    /// sound past an edge.
    pub(crate) spread: f32,
    /// Re-probe each sound every this many frames (spread across frames).
    pub(crate) every_frames: u32,
    /// How long (s) a change in occlusion takes to ease in.
    pub(crate) ease_secs: f32,
}

impl Default for OcclusionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            spread: 0.6,
            every_frames: 3,
            ease_secs: 0.2,
        }
    }
}

/// Probe how blocked each positional sound in earshot is, into its sink's
/// `occlusion` (`0` clear … `1` fully behind the map).
pub(super) fn occlude(
    time: Res<Time>,
    settings: Res<OcclusionSettings>,
    mix: Res<Mix>,
    mut frame: Local<u32>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    mut sinks: Query<(Entity, &mut SpatialAudioSink, Option<&GlobalTransform>)>,
    rapier: ReadRapierContext,
    colliders: Query<&Collider>,
) {
    *frame = frame.wrapping_add(1);
    let (Ok(ears), Ok(rapier)) = (listener.single(), rapier.single()) else {
        return;
    };
    let ears = ears.translation();
    let every = settings.every_frames.max(1);
    let step = time.delta_secs() / settings.ease_secs.max(1e-3);
    for (entity, mut sink, transform) in &mut sinks {
        let at = transform.map_or(Vec3::ZERO, GlobalTransform::translation);
        let distance = ears.distance(at);
        let audible = distance < mix.profile(&sink.profile).far;
        if !settings.enabled || !audible || distance < 0.5 {
            sink.occlusion = 0.0;
            sink.probed = settings.enabled && audible;
            continue;
        }
        // (A fresh sound now; otherwise in turn, a few frames apart.)
        if sink.probed && (entity.index().wrapping_add(*frame)) % every != 0 {
            sink.occlusion += (sink.occlusion_target - sink.occlusion).clamp(-step, step);
            continue;
        }

        // Colliders the sound's inside of — boxes only (a machine round its
        // own hum); the map's meshes never "contain" a point.
        let mut inside = Vec::new();
        rapier.intersections_with_point(at, QueryFilter::default().exclude_sensors(), |hit| {
            if colliders.get(hit).is_ok_and(|c| c.as_cuboid().is_some()) {
                inside.push(hit);
            }
            true
        });
        let skip = |e: Entity| !inside.contains(&e);
        let filter = QueryFilter::default().exclude_sensors().predicate(&skip);

        let dir = (at - ears) / distance;
        let side = dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X) * settings.spread;
        let up = Vec3::Y * settings.spread;
        let points = [at, at + side, at - side, at + up];
        let blocked = points
            .iter()
            .filter(|&&point| {
                let to = point - ears;
                let len = to.length();
                len > 0.05 && rapier.cast_ray(ears, to / len, len - 0.05, true, filter).is_some()
            })
            .count();
        sink.occlusion_target = blocked as f32 / points.len() as f32;
        if sink.probed {
            sink.occlusion += (sink.occlusion_target - sink.occlusion).clamp(-step, step);
        } else {
            sink.occlusion = sink.occlusion_target;
            sink.probed = true;
        }
    }
}

/// The debug panel's "Sounds through walls" section.
#[derive(SystemParam)]
pub(crate) struct OcclusionDebug<'w, 's> {
    settings: ResMut<'w, OcclusionSettings>,
    sinks: Query<'w, 's, &'static SpatialAudioSink>,
}

impl OcclusionDebug<'_, '_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Positional sounds with the map between them and you lose their highs and some level. \
             How much is each profile's (audio mixer → profiles → walls).",
        );
        let (playing, muffled) = self
            .sinks
            .iter()
            .fold((0, 0), |(n, m), s| (n + 1, m + usize::from(s.occlusion > 0.01)));
        ui.label(format!("{muffled} of {playing} positional sounds muffled now"));
        let s = &mut *self.settings;
        ui.checkbox(&mut s.enabled, "enabled");
        ui.add(egui::Slider::new(&mut s.spread, 0.0f32..=3.0).text("side rays aim this far round it (m)"));
        ui.add(egui::Slider::new(&mut s.every_frames, 1u32..=15).text("re-check each sound every (frames)"));
        ui.add(egui::Slider::new(&mut s.ease_secs, 0.0f32..=2.0).text("easing (s)"));
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                *s = OcclusionSettings::default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("{s:#?}");
            }
        });
    }
}
