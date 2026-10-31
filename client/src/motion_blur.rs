//! Motion blur on the world as the view moves — Call of Duty's "World Motion
//! Blur": a fast flick reads as one smooth sweep instead of a run of sharp
//! frames. Bevy's per-pixel blur ([`MotionBlur`], off the camera's motion
//! vectors) on the world camera and the scope's (the picture in the lens) —
//! and, its own setting ("Weapon Motion Blur", off by default), on the gun's
//! camera, blurring the gun as it moves in our hands.
//!
//! Bevy ties the blur's length to the frame (`shutter_angle`, a fraction of
//! one frame's motion), so at a high frame rate it'd all but vanish. Each
//! level is an exposure time instead ([`MotionBlurTuning`]), turned into the
//! shutter angle every frame — the same blur at 60 fps or 240. A frame the
//! camera jumps (a respawn, a kill cam cutting in) isn't blurred, or the
//! whole screen would smear across the cut — nor the scope camera's first
//! frame back on (it's off at the hip), whose last frame is from whenever we
//! last aimed.
//!
//! The prepasses it needs (depth, motion vectors) are on those cameras from
//! the start (`main.rs`), and never added or taken away here: changing them
//! mid-game left meshes' cached prepass pipelines out of step with the
//! camera's — a wgpu validation crash (on Der Tempel). Off
//! (`Settings::motion_blur`), a camera just loses its [`MotionBlur`].
//! Nothing to reset between games: the cameras are set afresh every frame.

use bevy::core_pipeline::motion_blur::MotionBlur;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::player::{ViewModelCamera, WorldModelCamera};
use crate::weapons::ScopeCamera;
use crate::settings::{MotionBlurLevel, Settings};
use crate::AppState;

/// Tunables — the debug panel's "Motion blur" section.
#[derive(Resource, Clone, Debug)]
pub(crate) struct MotionBlurTuning {
    /// Each level's exposure (ms): how long a stretch of the view's motion
    /// one frame blurs over.
    pub(crate) low_ms: f32,
    pub(crate) normal_ms: f32,
    pub(crate) high_ms: f32,
    /// Per-pixel samples each way along the blur (`n * 2 + 1` in all) —
    /// more for longer blurs, so they don't break into steps.
    pub(crate) low_samples: u32,
    pub(crate) normal_samples: u32,
    pub(crate) high_samples: u32,
    /// The longest blur, in frames of motion (Bevy's shutter angle) — a cap
    /// for a very low frame rate.
    pub(crate) max_shutter: f32,
    /// A camera that moves further than this (m) in one frame has jumped
    /// (a respawn, a kill cam): that frame's not blurred.
    pub(crate) cut_m: f32,
}

impl Default for MotionBlurTuning {
    fn default() -> Self {
        Self {
            low_ms: 4.0,
            normal_ms: 8.0,
            high_ms: 14.0,
            low_samples: 2,
            normal_samples: 3,
            high_samples: 4,
            max_shutter: 3.0,
            cut_m: 1.5,
        }
    }
}

impl MotionBlurTuning {
    /// `level`'s exposure (s) and samples — `None` off.
    fn level(&self, level: MotionBlurLevel) -> Option<(f32, u32)> {
        match level {
            MotionBlurLevel::Off => None,
            MotionBlurLevel::Low => Some((self.low_ms / 1000.0, self.low_samples)),
            MotionBlurLevel::Normal => Some((self.normal_ms / 1000.0, self.normal_samples)),
            MotionBlurLevel::High => Some((self.high_ms / 1000.0, self.high_samples)),
        }
    }
}

pub(crate) struct MotionBlurPlugin;

impl Plugin for MotionBlurPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MotionBlurTuning>().add_systems(
            PostUpdate,
            apply_motion_blur
                .after(TransformSystem::TransformPropagate)
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// Set each camera's blur for this frame: the level's exposure as a shutter
/// angle at this frame's length (real time — a kill cam's slow-mo doesn't
/// stretch it), or none on a jump or the frame it comes back on; or take it
/// off altogether.
#[allow(clippy::type_complexity)]
fn apply_motion_blur(
    time: Res<Time<Real>>,
    settings: Res<Settings>,
    tuning: Res<MotionBlurTuning>,
    mut cameras: Query<
        (Entity, &Camera, &GlobalTransform, Option<&mut MotionBlur>, Has<ViewModelCamera>),
        Or<(With<WorldModelCamera>, With<ScopeCamera>, With<ViewModelCamera>)>,
    >,
    // Where each camera was last frame it drew (none: it wasn't drawing).
    mut last: Local<bevy::platform::collections::HashMap<Entity, Vec3>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs().max(1e-4);
    let mut seen = Vec::new();
    for (entity, camera, global, blur, gun) in &mut cameras {
        let level = tuning.level(if gun { settings.weapon_motion_blur } else { settings.motion_blur });
        let Some((exposure, samples)) = level else {
            // (Only the blur itself: its prepasses stay — see the module docs.)
            if blur.is_some() {
                commands.entity(entity).remove::<MotionBlur>();
            }
            continue;
        };
        if !camera.is_active {
            continue;
        }
        seen.push(entity);
        let at = global.translation();
        let cut = last.insert(entity, at).is_none_or(|prev| prev.distance(at) > tuning.cut_m);
        let shutter_angle = if cut { 0.0 } else { (exposure / dt).min(tuning.max_shutter) };
        match blur {
            Some(mut blur) => {
                if blur.shutter_angle != shutter_angle || blur.samples != samples {
                    blur.shutter_angle = shutter_angle;
                    blur.samples = samples;
                }
            }
            None => {
                commands.entity(entity).insert(MotionBlur { shutter_angle, samples });
            }
        }
    }
    last.retain(|e, _| seen.contains(e));
}

/// The debug panel's "Motion blur" section.
#[derive(SystemParam)]
pub(crate) struct MotionBlurDebug<'w> {
    tuning: ResMut<'w, MotionBlurTuning>,
    settings: Res<'w, Settings>,
}

impl MotionBlurDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label(format!(
            "The world blurs as the view moves (Settings → Graphics → Motion blur: {}).",
            self.settings.motion_blur.label()
        ));
        let t = &mut *self.tuning;
        ui.add(egui::Slider::new(&mut t.low_ms, 0.0f32..=40.0).text("LOW exposure (ms)"));
        ui.add(egui::Slider::new(&mut t.normal_ms, 0.0f32..=40.0).text("NORMAL exposure (ms)"));
        ui.add(egui::Slider::new(&mut t.high_ms, 0.0f32..=40.0).text("HIGH exposure (ms)"));
        ui.add(egui::Slider::new(&mut t.low_samples, 0u32..=8).text("LOW samples (each way)"));
        ui.add(egui::Slider::new(&mut t.normal_samples, 0u32..=8).text("NORMAL samples (each way)"));
        ui.add(egui::Slider::new(&mut t.high_samples, 0u32..=8).text("HIGH samples (each way)"));
        ui.add(egui::Slider::new(&mut t.max_shutter, 0.5f32..=8.0).text("longest blur (frames of motion)"));
        ui.add(egui::Slider::new(&mut t.cut_m, 0.2f32..=10.0).text("no blur on a jump over (m)"));
        ui.horizontal(|ui| {
            if ui.button("Reset motion blur").clicked() {
                *t = default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("{t:#?}");
            }
        });
    }
}
