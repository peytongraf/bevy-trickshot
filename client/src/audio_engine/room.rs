//! Shapes the shared reverb to the space the listener's in — measured, not
//! authored, so every map gets it with nothing placed by hand.
//!
//! A few times a second a sphere of rays goes out from the listener against
//! every collider (the client only has static ones: the map, doors,
//! machines). From what they hit:
//! - **sky** — the share of upward rays that got away: open overhead means
//!   outside;
//! - **walls** — the share of level rays that hit something;
//! - **size** — the median distance to whatever the level and upward rays
//!   hit (the median, so a crate at your elbow doesn't shrink the room).
//!
//! `enclosure = (1 − sky) × walls` blends the reverb between an outdoor
//! preset and an indoor one, itself blended small → large by size. Both
//! numbers are eased over time so walking through a doorway glides rather
//! than snaps. Everything's tunable live in the debug panel's
//! "Audio → Reverb" section.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use kira::{Decibels, Tween};

use super::{AudioEngine, SpatialListener};

/// Rays in each probe's sphere.
const RAYS: usize = 32;

/// Rays pointing further up than this (the `y` of their direction) count
/// towards "sky"; ones within it either side, towards "walls". The rest
/// point at the floor and are left out.
const UP_Y: f32 = 0.35;

pub(super) struct RoomReverbPlugin;

impl Plugin for RoomReverbPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReverbSettings>()
            .init_resource::<RoomReadout>()
            .add_systems(Update, (probe_room, apply_reverb, draw_rays).chain());
    }
}

/// One reverb character: what the room's measurements blend between.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReverbPreset {
    /// How long it rings (kira's reverb feedback, `0`–`1`; `1` never stops).
    feedback: f32,
    /// How quickly the highs die away (`0` bright stone … `1` muffled).
    damping: f32,
    /// How loud the reverb is next to the dry sound (dB).
    level_db: f32,
}

impl ReverbPreset {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            feedback: self.feedback.lerp(other.feedback, t),
            damping: self.damping.lerp(other.damping, t),
            level_db: self.level_db.lerp(other.level_db, t),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, label: &str) {
        ui.label(label);
        ui.add(egui::Slider::new(&mut self.feedback, 0.0f32..=0.99).text("decay (feedback)"));
        ui.add(egui::Slider::new(&mut self.damping, 0.0f32..=1.0).text("damping (darker)"));
        ui.add(egui::Slider::new(&mut self.level_db, -40.0f32..=6.0).text("level (dB)"));
    }
}

/// Room-detection tuning ("Audio → Reverb" in the debug panel).
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct ReverbSettings {
    enabled: bool,
    /// Seconds between probes.
    interval: f32,
    /// How far each ray looks (m); a miss counts as open space.
    max_distance: f32,
    /// Time constant (s) the measurements ease toward each new probe with.
    smoothing: f32,
    /// Rooms this size (m, median distance to the surfaces) or smaller get
    /// [`Self::small`]; this size or bigger, [`Self::large`].
    small_size: f32,
    large_size: f32,
    outdoor: ReverbPreset,
    small: ReverbPreset,
    large: ReverbPreset,
    /// Draw the last probe's rays (green = hit, red = got away).
    show_rays: bool,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: 0.2,
            max_distance: 50.0,
            smoothing: 0.35,
            small_size: 3.0,
            large_size: 25.0,
            outdoor: ReverbPreset {
                feedback: 0.7,
                damping: 0.5,
                level_db: -20.0,
            },
            small: ReverbPreset {
                feedback: 0.8,
                damping: 0.3,
                level_db: -9.0,
            },
            large: ReverbPreset {
                feedback: 0.93,
                damping: 0.2,
                level_db: -4.0,
            },
            show_rays: false,
        }
    }
}

/// What the probes measured, eased, plus the reverb that came out of it.
/// Cleared whenever there's no listener (out of a match), so the next match
/// starts from its own first probe rather than easing in from the last one.
#[derive(Resource, Default)]
struct RoomReadout {
    /// Eased values below have something in them.
    valid: bool,
    /// The latest probe's raw numbers (`0`–`1`, `0`–`1`, m).
    sky: f32,
    walls: f32,
    size_raw: f32,
    /// Eased toward the latest probe.
    enclosure: f32,
    size: f32,
    /// What's being sent to the reverb.
    preset: Option<ReverbPreset>,
    /// Seconds since the last probe.
    since_probe: f32,
    /// Golden-angle twist on the ray sphere, advanced every probe so
    /// successive probes fill in between each other's rays.
    twist: f32,
    /// The last probe's rays, for `show_rays`: start, end, hit.
    rays: Vec<(Vec3, Vec3, bool)>,
}

/// Measure the room every [`ReverbSettings::interval`], and ease toward it
/// every frame.
fn probe_room(
    time: Res<Time>,
    settings: Res<ReverbSettings>,
    mut readout: ResMut<RoomReadout>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    rapier: ReadRapierContext,
) {
    let Ok(ears) = listener.single() else {
        if readout.valid {
            *readout = RoomReadout::default();
        }
        return;
    };
    let Ok(rapier) = rapier.single() else {
        return;
    };

    readout.since_probe += time.delta_secs();
    if !readout.valid || readout.since_probe >= settings.interval {
        readout.since_probe = 0.0;
        readout.twist = (readout.twist + GOLDEN_ANGLE) % std::f32::consts::TAU;
        let origin = ears.translation();
        let max = settings.max_distance.max(1.0);
        let (mut up, mut up_open, mut level, mut level_hit) = (0, 0, 0, 0);
        let mut distances = Vec::with_capacity(RAYS);
        readout.rays.clear();
        for i in 0..RAYS {
            let dir = sphere_dir(i, readout.twist);
            let hit = rapier
                .cast_ray(origin, dir, max, true, QueryFilter::default())
                .map(|(_, toi)| toi);
            readout.rays.push((origin, origin + dir * hit.unwrap_or(max), hit.is_some()));
            if dir.y > UP_Y {
                up += 1;
                up_open += usize::from(hit.is_none());
            } else if dir.y >= -UP_Y {
                level += 1;
                level_hit += usize::from(hit.is_some());
            } else {
                continue;
            }
            if let Some(d) = hit {
                distances.push(d);
            }
        }
        readout.sky = up_open as f32 / up.max(1) as f32;
        readout.walls = level_hit as f32 / level.max(1) as f32;
        readout.size_raw = if distances.is_empty() {
            max
        } else {
            distances.sort_by(f32::total_cmp);
            distances[distances.len() / 2]
        };
    }

    let enclosure = (1.0 - readout.sky) * readout.walls;
    let size = readout.size_raw;
    if readout.valid {
        let k = 1.0 - (-time.delta_secs() / settings.smoothing.max(0.01)).exp();
        readout.enclosure += (enclosure - readout.enclosure) * k;
        readout.size += (size - readout.size) * k;
    } else {
        readout.valid = true;
        readout.enclosure = enclosure;
        readout.size = size;
    }
}

/// Turn the eased measurements into the reverb's settings.
fn apply_reverb(
    settings: Res<ReverbSettings>,
    mut readout: ResMut<RoomReadout>,
    engine: Option<ResMut<AudioEngine>>,
) {
    if !readout.valid {
        readout.preset = None;
        return;
    }
    let s = &*settings;
    let span = (s.large_size - s.small_size).max(0.01);
    let t = ((readout.size - s.small_size) / span).clamp(0.0, 1.0);
    let indoor = s.small.lerp(s.large, t * t * (3.0 - 2.0 * t));
    let preset = s.outdoor.lerp(indoor, readout.enclosure.clamp(0.0, 1.0));
    readout.preset = Some(preset);

    let Some(mut engine) = engine else { return };
    let level = if s.enabled { Decibels(preset.level_db) } else { Decibels::SILENCE };
    engine.reverb_send.set_volume(level, Tween::default());
    engine.reverb.set_feedback(preset.feedback.clamp(0.0, 0.99) as f64, Tween::default());
    engine.reverb.set_damping(preset.damping.clamp(0.0, 1.0) as f64, Tween::default());
}

fn draw_rays(settings: Res<ReverbSettings>, readout: Res<RoomReadout>, mut gizmos: Gizmos) {
    if !settings.show_rays {
        return;
    }
    for &(from, to, hit) in &readout.rays {
        let color = if hit { Color::srgb(0.2, 1.0, 0.3) } else { Color::srgb(1.0, 0.25, 0.2) };
        gizmos.line(from, to, color);
    }
}

const GOLDEN_ANGLE: f32 = 2.399_963;

/// The `i`th of [`RAYS`] directions spread evenly over a sphere (a Fibonacci
/// sphere), turned `twist` about the vertical.
fn sphere_dir(i: usize, twist: f32) -> Vec3 {
    let y = 1.0 - 2.0 * (i as f32 + 0.5) / RAYS as f32;
    let r = (1.0 - y * y).sqrt();
    let phi = i as f32 * GOLDEN_ANGLE + twist;
    Vec3::new(r * phi.cos(), y, r * phi.sin())
}

/// The debug panel's "Audio → Reverb" section.
#[derive(SystemParam)]
pub(crate) struct ReverbDebug<'w> {
    settings: ResMut<'w, ReverbSettings>,
    readout: Res<'w, RoomReadout>,
}

impl ReverbDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        let r = &*self.readout;
        ui.label(
            "Reverb follows the space you're in, measured by rays from the camera: \
             open sky overhead = outside, otherwise how walled-in and how big it is.",
        );
        if r.valid {
            ui.label(format!(
                "sky {:.0}%   walls {:.0}%   size {:.1} m (now {:.1} m)",
                r.sky * 100.0,
                r.walls * 100.0,
                r.size,
                r.size_raw,
            ));
            ui.add(egui::ProgressBar::new(r.enclosure).text(format!("enclosure {:.0}%", r.enclosure * 100.0)));
            if let Some(p) = r.preset {
                ui.label(format!(
                    "→ decay {:.2}   damping {:.2}   level {:.1} dB",
                    p.feedback, p.damping, p.level_db
                ));
            }
        } else {
            ui.label("(Not in a match.)");
        }

        let s = &mut *self.settings;
        ui.checkbox(&mut s.enabled, "enabled");
        ui.checkbox(&mut s.show_rays, "show rays (green = hit, red = open)");
        ui.add(egui::Slider::new(&mut s.interval, 0.05f32..=1.0).text("probe every (s)"));
        ui.add(egui::Slider::new(&mut s.max_distance, 5.0f32..=150.0).text("ray length (m)"));
        ui.add(egui::Slider::new(&mut s.smoothing, 0.01f32..=2.0).text("easing (s)"));
        ui.add(egui::Slider::new(&mut s.small_size, 0.5f32..=20.0).text("small room ≤ (m)"));
        ui.add(egui::Slider::new(&mut s.large_size, 5.0f32..=80.0).text("large room ≥ (m)"));
        ui.separator();
        s.outdoor.ui(ui, "Outdoors");
        ui.separator();
        s.small.ui(ui, "Small room");
        ui.separator();
        s.large.ui(ui, "Large room");
        if ui.button("Reset reverb").clicked() {
            *s = ReverbSettings::default();
        }
    }
}
