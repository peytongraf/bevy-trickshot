//! The M17 (`models/weapons/m17.glb`) — a semi-auto pistol, the gun every
//! `Zombies` player starts with (`shared::GameMode::starting_primary`), in
//! place of a loadout pick.
//!
//! It wears the same [`ViewModel`] every primary does
//! (`ak::sync_primary_model` swaps the scene in), so the sway, bob, ADS,
//! hiding and the rest work on it unchanged. Its clips cross-fade through
//! the player's `AnimationTransitions` like the AK's:
//!
//! * `Draw` / `Not_Draw` — its Show / Hide;
//! * `Fire` — every round (restarted each shot);
//! * `Reload_Fast` — a reload with rounds left; `Reload_Complete` — from
//!   empty (a mag *and* the slide);
//! * `Idle`, `Walk` (on the ground and moving — sped up the faster we go,
//!   so a sprint's quicker) and `Jump` (once, the moment we leave the
//!   ground) — [`m17_locomotion`], whenever nothing else is playing.
//!   Aimed down sights it holds still in its rest pose instead. (`Jog`
//!   isn't used.)
//!
//! Everything about its look and feel is in [`M17Settings`] (the debug
//! panel's "M17" section).

use std::time::Duration;

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy_egui::egui;

use super::view_model::{AnimationSegment, PrimaryRig, SegAct, ViewModelOffset};

pub(crate) const M17_MODEL: &str = "models/weapons/m17.glb";

/// `m17.glb`'s clips we use, by the file's order (`Idle`, `Fire`, `Walk`,
/// `Jog`, `Jump`, `Draw`, `Not_Draw`, `Reload_Fast`, `Reload_Complete`) —
/// `Jog` left out — then `Idle` again for the still aimed hold.
pub(crate) const M17_CLIPS: [usize; 9] = [0, 1, 2, 4, 5, 6, 7, 8, 0];
const CLIP_IDLE: usize = 0;
const CLIP_FIRE: usize = 1;
const CLIP_WALK: usize = 2;
const CLIP_JUMP: usize = 3;
const CLIP_DRAW: usize = 4;
const CLIP_NOT_DRAW: usize = 5;
const CLIP_RELOAD_FAST: usize = 6;
const CLIP_RELOAD_COMPLETE: usize = 7;
const CLIP_HOLD: usize = 8;

const SEG_IDLE: AnimationSegment = AnimationSegment::clip("M17 Idle", CLIP_IDLE, 4.167);
const SEG_FIRE: AnimationSegment = AnimationSegment::clip("M17 Fire", CLIP_FIRE, 0.5).with_act(SegAct::Shoot);
const SEG_WALK: AnimationSegment = AnimationSegment::clip("M17 Walk", CLIP_WALK, 3.333);
const SEG_JUMP: AnimationSegment = AnimationSegment::clip("M17 Jump", CLIP_JUMP, 1.667);
const SEG_DRAW: AnimationSegment = AnimationSegment::clip("M17 Draw", CLIP_DRAW, 0.833).with_act(SegAct::Show);
/// (Only the first 60% of `Not_Draw` — the gun's out of sight by then, and
/// the rest is the empty view.)
const SEG_NOT_DRAW: AnimationSegment =
    AnimationSegment::clip("M17 Not Draw", CLIP_NOT_DRAW, 1.167 * 0.6).with_act(SegAct::Hide);
const SEG_RELOAD_FAST: AnimationSegment =
    AnimationSegment::clip("M17 Reload Fast", CLIP_RELOAD_FAST, 2.917).with_act(SegAct::Reload);
const SEG_RELOAD_COMPLETE: AnimationSegment =
    AnimationSegment::clip("M17 Reload Complete", CLIP_RELOAD_COMPLETE, 3.333).with_act(SegAct::ReloadEmpty);
const SEG_HOLD: AnimationSegment = AnimationSegment::clip("M17 ADS Hold", CLIP_HOLD, 4.167);

/// The M17's segment for `act` (no bolt to work: a `Rechamber` is its
/// shot, and never asked for).
pub(crate) fn m17_seg(act: SegAct) -> AnimationSegment {
    match act {
        SegAct::Reload => SEG_RELOAD_FAST,
        SegAct::ReloadEmpty => SEG_RELOAD_COMPLETE,
        SegAct::Hide => SEG_NOT_DRAW,
        SegAct::Show => SEG_DRAW,
        SegAct::Shoot | SegAct::Rechamber | SegAct::Other => SEG_FIRE,
    }
}

/// The clip it rests in.
pub(crate) fn m17_idle_seg() -> AnimationSegment {
    SEG_IDLE
}

/// Magazine size at each Pack-a-Punch level (0 = not packed; see
/// `weapons::mag_size`), and how many mags' worth (loaded one included) a
/// `Zombies` player carries at most — 17 and 119 unpacked.
pub(crate) const M17_MAG_SIZES: [u32; 4] = [17, 21, 25, 30];
pub(crate) const M17_ZOMBIES_TOTAL_MAGS: u32 = 7;

/// The glTF names of its parts that wear the Pack-a-Punch camo — the slide
/// and the frame (not the arms, nor the loose shells and round).
pub(crate) const M17_CAMO_PARTS: [&str; 2] = ["Object_70", "Object_71"];

/// Everything tunable about the M17 (the debug panel's "M17" section).
#[derive(Resource, Clone)]
pub(crate) struct M17Settings {
    /// View-model poses at the hip and fully aimed.
    pub(crate) hip: ViewModelOffset,
    pub(crate) ads: ViewModelOffset,
    /// World zoom at full ADS (×) — iron sights, so only a little.
    pub(crate) ads_zoom: f32,
    /// Least time (s) between shots, however fast the trigger's pulled.
    pub(crate) fire_interval: f32,
    /// Most a round strays off the aim point (degrees, up/down and
    /// left/right) at the hip, and fully aimed.
    pub(crate) hip_spread_deg: f32,
    pub(crate) ads_spread_deg: f32,
    /// Camera shake each shot adds, and its backward kick (m).
    pub(crate) trauma_per_shot: f32,
    pub(crate) recoil_kick: f32,
    /// Muzzle flash placement (camera space at the hip pose, m — it follows
    /// the gun from there) and size.
    pub(crate) muzzle_translation: Vec3,
    pub(crate) muzzle_size: Vec2,
    /// Cross-fade between clips, and into each shot (s).
    pub(crate) blend_secs: f32,
    pub(crate) shot_blend_secs: f32,
    /// Playback speeds (×). `walk_speed` is at walking pace — it's sped up
    /// (or slowed) by how much faster (or slower) than that we're going.
    pub(crate) shot_speed: f32,
    pub(crate) reload_speed: f32,
    pub(crate) draw_speed: f32,
    pub(crate) idle_speed: f32,
    pub(crate) walk_speed: f32,
    pub(crate) jump_speed: f32,
    /// Slower than this (m/s) on the ground is standing still.
    pub(crate) walk_min_speed: f32,
}

impl Default for M17Settings {
    fn default() -> Self {
        // `m17.glb`'s arms are the Ray Gun's (the same rig, the same size):
        // the camera at its origin looking down +Z — turned round and
        // shrunk the same way.
        let scale = 0.2;
        Self {
            hip: ViewModelOffset {
                translation: Vec3::new(-0.05, 0.0, 0.02),
                yaw: 3.0543,
                pitch: 0.0,
                roll: 0.0,
                scale,
            },
            ads: ViewModelOffset {
                translation: Vec3::new(-0.075, 0.0355, 0.1),
                yaw: -std::f32::consts::PI,
                pitch: 0.0,
                roll: 0.0873,
                scale,
            },
            ads_zoom: 1.2,
            fire_interval: 0.12,
            hip_spread_deg: 1.8,
            ads_spread_deg: 0.2,
            trauma_per_shot: 0.14,
            recoil_kick: 0.012,
            muzzle_translation: Vec3::new(0.06, -0.04, -0.3),
            muzzle_size: Vec2::new(0.2, 0.16),
            blend_secs: 0.15,
            shot_blend_secs: 0.03,
            shot_speed: 1.0,
            reload_speed: 1.0,
            draw_speed: 1.0,
            idle_speed: 1.0,
            walk_speed: 1.0,
            jump_speed: 1.4,
            walk_min_speed: 0.5,
        }
    }
}

impl M17Settings {
    /// Most a round strays off the aim point (radians, each axis) at ADS
    /// amount `ads_t`.
    pub(crate) fn spread_rad(&self, ads_t: f32) -> f32 {
        self.hip_spread_deg
            .lerp(self.ads_spread_deg, ads_t.clamp(0.0, 1.0))
            .max(0.0)
            .to_radians()
    }
}

/// Which of its rest clips the M17's playing ([`m17_locomotion`]).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum M17Loco {
    #[default]
    Idle,
    Walk,
    Jump,
    /// Aimed down sights: still, in its rest pose.
    Hold,
}

impl M17Loco {
    fn seg(self) -> AnimationSegment {
        match self {
            M17Loco::Idle => SEG_IDLE,
            M17Loco::Walk => SEG_WALK,
            M17Loco::Jump => SEG_JUMP,
            M17Loco::Hold => SEG_HOLD,
        }
    }
}

/// [`m17_locomotion`]'s state: the clip, and whether we were off the
/// ground last frame (so a jump's caught the moment it starts).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct M17LocoState {
    pub(crate) clip: M17Loco,
    airborne: bool,
}

/// How the player's moving this frame, for [`m17_locomotion`].
pub(crate) struct M17Motion {
    pub(crate) grounded: bool,
    /// Going up (a jump, rather than walking off an edge).
    pub(crate) rising: bool,
    /// Speed along the ground (m/s).
    pub(crate) speed: f32,
    /// Aimed down sights (any amount).
    pub(crate) aiming: bool,
}

/// Play the M17's rest clips while nothing else is: `Jump` once the moment
/// we jump (played out even once we've landed), `Walk` on the ground while
/// moving — at its speed times how much faster than walking pace we're
/// going — `Idle` standing still, and aimed down sights its rest pose held
/// still. All cross-faded; picks back up after a shot / reload / draw.
pub(crate) fn m17_locomotion(rig: &mut PrimaryRig, state: &mut M17LocoState, m: M17Motion, cfg: &M17Settings) {
    let airborne = !m.grounded;
    let took_off = airborne && !state.airborne && m.rising;
    state.airborne = airborne;
    let resumed = !rig.is_main(state.clip.seg());
    let still_jumping = state.clip == M17Loco::Jump && !resumed && !rig.reached(SEG_JUMP, SEG_JUMP.end_secs());
    let next = if took_off || still_jumping {
        M17Loco::Jump
    } else if m.aiming {
        M17Loco::Hold
    } else if m.grounded && m.speed > cfg.walk_min_speed {
        M17Loco::Walk
    } else {
        M17Loco::Idle
    };
    let pace = (m.speed / crate::player::DEFAULT_WALK_SPEED).clamp(0.3, 3.0);
    let speed = match next {
        M17Loco::Idle => cfg.idle_speed.max(0.01),
        M17Loco::Walk => (cfg.walk_speed * pace).max(0.01),
        M17Loco::Jump => cfg.jump_speed.max(0.01),
        M17Loco::Hold => 0.0,
    };
    if next != state.clip || resumed || took_off {
        let repeat = if next == M17Loco::Jump {
            RepeatAnimation::Never
        } else {
            RepeatAnimation::Forever
        };
        rig.play_with(next.seg(), speed, repeat, Duration::from_secs_f32(cfg.blend_secs.max(0.0)));
    } else {
        // Every frame — the walk follows our speed, and the panel applies
        // live.
        rig.set_speed(next.seg(), speed);
    }
    state.clip = next;
}

/// The debug panel's "M17" section. `force_ads` is `AdsTuning::force_full`
/// (shared with the other guns'); `live` is the rest clip right now.
pub(crate) fn m17_section(ui: &mut egui::Ui, s: &mut M17Settings, force_ads: &mut bool, live: M17Loco) {
    ui.label(format!("Rest clip now: {live:?}"));
    let pose = |ui: &mut egui::Ui, label: &str, p: &mut ViewModelOffset| {
        ui.label(label);
        ui.add(egui::Slider::new(&mut p.translation.x, -0.5f32..=0.5).text("x"));
        ui.add(egui::Slider::new(&mut p.translation.y, -0.5f32..=0.5).text("y"));
        ui.add(egui::Slider::new(&mut p.translation.z, -0.5f32..=0.5).text("z"));
        let mut yaw = p.yaw.to_degrees();
        if ui.add(egui::Slider::new(&mut yaw, -180.0f32..=180.0).text("yaw (°)")).changed() {
            p.yaw = yaw.to_radians();
        }
        let mut pitch = p.pitch.to_degrees();
        if ui.add(egui::Slider::new(&mut pitch, -90.0f32..=90.0).text("pitch (°)")).changed() {
            p.pitch = pitch.to_radians();
        }
        let mut roll = p.roll.to_degrees();
        if ui.add(egui::Slider::new(&mut roll, -45.0f32..=45.0).text("roll (°)")).changed() {
            p.roll = roll.to_radians();
        }
        ui.add(egui::Slider::new(&mut p.scale, 0.01f32..=1.0).logarithmic(true).text("scale"));
    };
    ui.collapsing("View model poses", |ui| {
        ui.checkbox(force_ads, "Force full ADS (ignore RMB) — to tune the aimed pose");
        pose(ui, "Hip", &mut s.hip);
        ui.separator();
        pose(ui, "Aimed (ADS)", &mut s.ads);
        ui.add(egui::Slider::new(&mut s.ads_zoom, 1.0f32..=3.0).text("ADS zoom (×)"));
    });
    ui.collapsing("Firing", |ui| {
        ui.add(egui::Slider::new(&mut s.fire_interval, 0.03f32..=1.0).text("least between shots (s)"));
        ui.add(egui::Slider::new(&mut s.hip_spread_deg, 0.0f32..=10.0).text("hip spread (°)"));
        ui.add(egui::Slider::new(&mut s.ads_spread_deg, 0.0f32..=5.0).text("ADS spread (°)"));
        ui.add(egui::Slider::new(&mut s.trauma_per_shot, 0.0f32..=1.0).text("shake per shot"));
        ui.add(egui::Slider::new(&mut s.recoil_kick, 0.0f32..=0.1).text("kick per shot (m)"));
        ui.label("Muzzle flash (camera space, at the hip — follows the gun)");
        ui.add(egui::Slider::new(&mut s.muzzle_translation.x, -1.0f32..=1.0).text("x"));
        ui.add(egui::Slider::new(&mut s.muzzle_translation.y, -1.0f32..=1.0).text("y"));
        ui.add(egui::Slider::new(&mut s.muzzle_translation.z, -2.0f32..=0.0).text("z"));
        ui.add(egui::Slider::new(&mut s.muzzle_size.x, 0.0f32..=1.0).text("width"));
        ui.add(egui::Slider::new(&mut s.muzzle_size.y, 0.0f32..=1.0).text("height"));
    });
    ui.collapsing("Animations", |ui| {
        ui.add(egui::Slider::new(&mut s.blend_secs, 0.0f32..=1.0).text("blend between clips (s)"));
        ui.add(egui::Slider::new(&mut s.shot_blend_secs, 0.0f32..=0.3).text("blend into a shot (s)"));
        ui.add(egui::Slider::new(&mut s.shot_speed, 0.1f32..=4.0).text("shot speed (×)"));
        ui.add(egui::Slider::new(&mut s.reload_speed, 0.1f32..=4.0).text("reload speed (×)"));
        ui.add(egui::Slider::new(&mut s.draw_speed, 0.1f32..=4.0).text("draw / hide speed (×)"));
        ui.add(egui::Slider::new(&mut s.idle_speed, 0.1f32..=4.0).text("idle speed (×)"));
        ui.add(egui::Slider::new(&mut s.walk_speed, 0.1f32..=4.0).text("walk speed at walking pace (×)"));
        ui.add(egui::Slider::new(&mut s.jump_speed, 0.1f32..=4.0).text("jump speed (×)"));
        ui.add(egui::Slider::new(&mut s.walk_min_speed, 0.0f32..=3.0).text("walking from (m/s)"));
    });
    ui.horizontal(|ui| {
        if ui.button("Copy M17 settings to console").clicked() {
            let p = |o: &ViewModelOffset| {
                format!(
                    "translation: Vec3::new({:.4}, {:.4}, {:.4}), yaw: {:.4}, pitch: {:.4}, roll: {:.4}, scale: {:.4}",
                    o.translation.x, o.translation.y, o.translation.z, o.yaw, o.pitch, o.roll, o.scale
                )
            };
            info!(
                "m17: hip {{ {} }}, ads {{ {} }}, ads_zoom: {:.2}, fire_interval: {:.3}, hip_spread_deg: {:.2}, \
                 ads_spread_deg: {:.2}, trauma_per_shot: {:.3}, recoil_kick: {:.4}, muzzle_translation: \
                 Vec3::new({:.3}, {:.3}, {:.3}), muzzle_size: Vec2::new({:.2}, {:.2}), blend_secs: {:.2}, \
                 shot_blend_secs: {:.3}, shot_speed: {:.2}, reload_speed: {:.2}, draw_speed: {:.2}, \
                 idle_speed: {:.2}, walk_speed: {:.2}, jump_speed: {:.2}, walk_min_speed: {:.2}",
                p(&s.hip),
                p(&s.ads),
                s.ads_zoom,
                s.fire_interval,
                s.hip_spread_deg,
                s.ads_spread_deg,
                s.trauma_per_shot,
                s.recoil_kick,
                s.muzzle_translation.x,
                s.muzzle_translation.y,
                s.muzzle_translation.z,
                s.muzzle_size.x,
                s.muzzle_size.y,
                s.blend_secs,
                s.shot_blend_secs,
                s.shot_speed,
                s.reload_speed,
                s.draw_speed,
                s.idle_speed,
                s.walk_speed,
                s.jump_speed,
                s.walk_min_speed,
            );
        }
        if ui.button("Reset M17 settings").clicked() {
            *s = M17Settings::default();
        }
    });
}
