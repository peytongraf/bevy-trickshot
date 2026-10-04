//! The AK-74 (`models/weapons/ak_74.glb`) — a loadout primary for `Zombies` and
//! `FreeForAll` (`shared::LobbyMember::loadout`), full-auto, in place of the
//! sniper.
//!
//! The view model is the same [`ViewModel`] entity the sniper uses:
//! [`sync_primary_model`] swaps its scene and [`ViewModelAnimation`] to
//! whichever primary our lobby loadout says, so every system that moves,
//! sways, hides or shows the view model works on either. Its clips are
//! separate (not one baked timeline like the sniper's), cross-faded through
//! the player's `AnimationTransitions` by [`super::PrimaryRig`]:
//!
//! * `Draw` / `Not_Draw` — the weapon's Show / Hide;
//! * `Shot` — every round fired (restarted each shot);
//! * `Reload_Fast` — a reload with rounds left (a mag swap); `Reload_Complete`
//!   — from empty (a mag *and* chambering a round);
//! * `Idle`, and `Start_Jump` → `Loop_Jump` (while airborne) → `Stop_Jump`
//!   — [`ak_locomotion`], whenever nothing else is playing.
//!
//! Walking and sprinting don't use the model's walk clips (`Walk` barely
//! moves — a ~1.5 mm sway): the procedural bob every view model shares
//! (`sway::WalkBob`, tuned under "Weapon sway") sways it instead.
//!
//! Everything about its look and feel is in [`AkSettings`] (the debug panel's
//! "AK-74" section).

use std::time::Duration;

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::weapon::{WeaponId, LOADOUT_WEAPONS};
use shared::Lobby;

use super::view_model::{AnimationSegment, PrimaryRig, SegAct, ViewModel, ViewModelAnimation, ViewModelOffset};
use super::weapon::Weapon;
use crate::net::GameClient;

pub(crate) const AK_MODEL: &str = "models/weapons/ak_74.glb";

/// `ak_74.glb`'s clips, in file order (`GltfAssetLabel::Animation(i)`).
const AK_CLIP_COUNT: usize = 13;
const CLIP_IDLE: usize = 1;
const CLIP_SHOT: usize = 2;
const CLIP_DRAW: usize = 3;
const CLIP_NOT_DRAW: usize = 4;
const CLIP_START_JUMP: usize = 5;
const CLIP_LOOP_JUMP: usize = 6;
const CLIP_STOP_JUMP: usize = 7;
const CLIP_RELOAD_FAST: usize = 11;
const CLIP_RELOAD_COMPLETE: usize = 12;

const SEG_IDLE: AnimationSegment = AnimationSegment::clip("AK Idle", CLIP_IDLE, 10.667);
const SEG_SHOT: AnimationSegment = AnimationSegment::clip("AK Shot", CLIP_SHOT, 0.25).with_act(SegAct::Shoot);
const SEG_DRAW: AnimationSegment = AnimationSegment::clip("AK Draw", CLIP_DRAW, 0.5).with_act(SegAct::Show);
const SEG_NOT_DRAW: AnimationSegment =
    AnimationSegment::clip("AK Not Draw", CLIP_NOT_DRAW, 0.333).with_act(SegAct::Hide);
const SEG_START_JUMP: AnimationSegment = AnimationSegment::clip("AK Start Jump", CLIP_START_JUMP, 0.417);
const SEG_LOOP_JUMP: AnimationSegment = AnimationSegment::clip("AK Loop Jump", CLIP_LOOP_JUMP, 0.017);
const SEG_STOP_JUMP: AnimationSegment = AnimationSegment::clip("AK Stop Jump", CLIP_STOP_JUMP, 0.417);
const SEG_RELOAD_FAST: AnimationSegment =
    AnimationSegment::clip("AK Reload Fast", CLIP_RELOAD_FAST, 1.767).with_act(SegAct::Reload);
const SEG_RELOAD_COMPLETE: AnimationSegment =
    AnimationSegment::clip("AK Reload Complete", CLIP_RELOAD_COMPLETE, 2.833).with_act(SegAct::ReloadEmpty);

/// The AK's segment for `act` (it has no bolt to work: a `Rechamber` is its
/// shot, and never asked for).
pub(crate) fn ak_seg(act: SegAct) -> AnimationSegment {
    match act {
        SegAct::Reload => SEG_RELOAD_FAST,
        SegAct::ReloadEmpty => SEG_RELOAD_COMPLETE,
        SegAct::Hide => SEG_NOT_DRAW,
        SegAct::Show => SEG_DRAW,
        SegAct::Shoot | SegAct::Rechamber | SegAct::Other => SEG_SHOT,
    }
}

/// The clip it rests in.
pub(crate) fn ak_idle_seg() -> AnimationSegment {
    SEG_IDLE
}

/// The round in the chamber — `Shot` throws it out to the right as the spent
/// shell (lower-cased, as `start_view_model_animation` matches names).
pub(crate) const AK_SHELL_NODE: &str = "bullet_single_55";

/// On the AK's shell bone: its rest pose (back in the chamber).
#[derive(Component)]
pub(crate) struct AkShell {
    pub(crate) rest: Transform,
}

/// Hold the shell in the chamber except while a `Shot` is actually playing.
/// Only `Shot` moves it; the idle / jump clips leave its rotation alone and
/// a cross-fade eases it back, so otherwise it'd fly back up out of the
/// ejection and sit twisted beside the gun until a reload reset it. A fresh
/// shot then throws it out from the chamber again. Runs right after the
/// animations are applied, before transforms propagate.
pub(crate) fn pin_ak_shell(
    view_model: Single<&ViewModelAnimation, With<ViewModel>>,
    players: Query<
        (&AnimationPlayer, &bevy::animation::prelude::AnimationTransitions),
        With<super::view_model::SniperAnimationPlayer>,
    >,
    mut shells: Query<(&AkShell, &mut Transform)>,
) {
    if view_model.weapon != WeaponId::Ak74 {
        return;
    }
    let Some(&shot) = view_model.nodes.get(CLIP_SHOT) else {
        return;
    };
    let firing = players.iter().any(|(player, transitions)| {
        transitions.get_main_animation() == Some(shot) && player.animation(shot).is_some_and(|a| !a.is_finished())
    });
    if firing {
        return;
    }
    for (shell, mut tf) in &mut shells {
        if *tf != shell.rest {
            *tf = shell.rest;
        }
    }
}

/// Magazine size, and how many mags' worth (loaded one included) a
/// `Zombies` player carries at most.
pub(crate) const AK_MAG_SIZE: u32 = 30;
pub(crate) const AK_ZOMBIES_TOTAL_MAGS: u32 = 6;

/// Everything tunable about the AK-74 (the debug panel's "AK-74" section).
#[derive(Resource, Clone)]
pub(crate) struct AkSettings {
    /// View-model poses at the hip and fully aimed (like the sniper's
    /// `ViewModelPoses`).
    pub(crate) hip: ViewModelOffset,
    pub(crate) ads: ViewModelOffset,
    /// World zoom at full ADS (×) — iron sights, so only a little.
    pub(crate) ads_zoom: f32,
    /// Seconds between rounds while the trigger's held (0.1 = 600 RPM).
    pub(crate) fire_interval: f32,
    /// Most a round strays off the aim point (degrees, up/down and
    /// left/right) at the hip, and fully aimed.
    pub(crate) hip_spread_deg: f32,
    pub(crate) ads_spread_deg: f32,
    /// Camera shake each round adds, and its backward kick (m).
    pub(crate) trauma_per_shot: f32,
    pub(crate) recoil_kick: f32,
    /// Muzzle flash placement (camera space at the hip pose, m — it follows
    /// the gun from there) and size.
    pub(crate) muzzle_translation: Vec3,
    pub(crate) muzzle_size: Vec2,
    /// Seconds each clip cross-fades into the next, and the (quicker) fade
    /// into each shot.
    pub(crate) blend_secs: f32,
    pub(crate) shot_blend_secs: f32,
    /// Playback speeds (×) of each clip.
    pub(crate) shot_speed: f32,
    pub(crate) reload_speed: f32,
    pub(crate) draw_speed: f32,
    pub(crate) idle_speed: f32,
    /// of Nitro Brew's movement multiplier).
    pub(crate) jump_speed: f32,
}

impl Default for AkSettings {
    fn default() -> Self {
        // `ak_74.glb` is in inches, the camera at its origin looking down +Z.
        let scale = 0.0254;
        Self {
            hip: ViewModelOffset {
                translation: Vec3::new(-0.03, -0.06, -0.09),
                yaw: std::f32::consts::PI,
                pitch: 0.0,
                scale,
            },
            ads: ViewModelOffset {
                translation: Vec3::new(-0.124, 0.018, 0.08),
                yaw: std::f32::consts::PI,
                pitch: -0.0035,
                scale,
            },
            ads_zoom: 1.3,
            fire_interval: 0.1,
            hip_spread_deg: 2.5,
            ads_spread_deg: 0.25,
            trauma_per_shot: 0.12,
            recoil_kick: 0.012,
            muzzle_translation: Vec3::new(0.10, -0.12, -0.9),
            muzzle_size: Vec2::new(0.45, 0.4),
            blend_secs: 0.15,
            shot_blend_secs: 0.04,
            shot_speed: 1.0,
            reload_speed: 1.0,
            draw_speed: 1.0,
            idle_speed: 1.0,
            jump_speed: 1.0,
        }
    }
}

impl AkSettings {
    /// Most a round strays off the aim point (radians, each axis) at ADS
    /// amount `ads_t`.
    pub(crate) fn spread_rad(&self, ads_t: f32) -> f32 {
        self.hip_spread_deg
            .lerp(self.ads_spread_deg, ads_t.clamp(0.0, 1.0))
            .max(0.0)
            .to_radians()
    }
}

/// The AK's idle / jump animation state ([`ak_locomotion`]).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum AkLoco {
    #[default]
    Idle,
    StartJump,
    Airborne,
    StopJump,
}

/// How the player's moving this frame, for [`ak_locomotion`].
pub(crate) struct AkMotion {
    pub(crate) grounded: bool,
    /// Going up (a jump, rather than walking off an edge).
    pub(crate) rising: bool,
}

impl AkLoco {
    fn seg(self) -> AnimationSegment {
        match self {
            AkLoco::Idle => SEG_IDLE,
            AkLoco::StartJump => SEG_START_JUMP,
            AkLoco::Airborne => SEG_LOOP_JUMP,
            AkLoco::StopJump => SEG_STOP_JUMP,
        }
    }

    fn looping(self) -> bool {
        matches!(self, AkLoco::Idle | AkLoco::Airborne)
    }

    /// Where a one-off clip leads once it's played out (or was cut off).
    fn settled(self) -> AkLoco {
        match self {
            AkLoco::StartJump => AkLoco::Airborne,
            AkLoco::StopJump => AkLoco::Idle,
            other => other,
        }
    }

    fn speed(self, cfg: &AkSettings) -> f32 {
        match self {
            AkLoco::Idle => cfg.idle_speed,
            AkLoco::StartJump | AkLoco::Airborne | AkLoco::StopJump => cfg.jump_speed,
        }
        .max(0.01)
    }
}

/// Play the AK's idle / jump clips while nothing else is: `Start_Jump`
/// the moment we jump, `Loop_Jump` while airborne, `Stop_Jump` the moment we
/// land, `Idle` otherwise — each one-off played once, all cross-faded. Picks
/// back up from wherever we are after a shot / reload / draw took over.
/// (Walking and sprinting are `sway::WalkBob`'s.)
pub(crate) fn ak_locomotion(rig: &mut PrimaryRig, loco: &mut AkLoco, m: AkMotion, cfg: &AkSettings) {
    // Something else took over since: carry on from where that leaves us.
    let mut state = *loco;
    let resumed = !rig.is_main(state.seg());
    if resumed {
        state = state.settled();
    }
    let done = |rig: &PrimaryRig, s: AkLoco| rig.reached(s.seg(), s.seg().end_secs());
    let next = if !m.grounded {
        match state {
            AkLoco::StartJump if !resumed && done(rig, state) => AkLoco::Airborne,
            AkLoco::StartJump | AkLoco::Airborne => state,
            _ if m.rising => AkLoco::StartJump,
            _ => AkLoco::Airborne,
        }
    } else {
        match state {
            AkLoco::StartJump | AkLoco::Airborne => AkLoco::StopJump,
            AkLoco::StopJump if !resumed && done(rig, state) => AkLoco::Idle,
            AkLoco::StopJump | AkLoco::Idle => state,
        }
    };
    let speed = next.speed(cfg);
    if next != *loco || resumed || !rig.is_main(next.seg()) {
        let repeat = if next.looping() {
            RepeatAnimation::Forever
        } else {
            RepeatAnimation::Never
        };
        rig.play_with(next.seg(), speed, repeat, Duration::from_secs_f32(cfg.blend_secs.max(0.0)));
    } else {
        // Every frame, so the panel applies live.
        rig.set_speed(next.seg(), speed);
    }
    *loco = next;
}

/// The view model's two scenes and animation graphs, loaded once up front so
/// switching primary doesn't hitch (the AK's is big).
#[derive(Resource)]
pub(crate) struct PrimaryModels {
    pub(crate) sniper_scene: Handle<Scene>,
    pub(crate) sniper_graph: Handle<AnimationGraph>,
    pub(crate) sniper_index: AnimationNodeIndex,
    pub(crate) ak_scene: Handle<Scene>,
    pub(crate) ak_graph: Handle<AnimationGraph>,
    pub(crate) ak_nodes: Vec<AnimationNodeIndex>,
}

impl PrimaryModels {
    /// Load both; `sniper_*` are the sniper's (already built by the caller).
    pub(crate) fn load(
        asset_server: &AssetServer,
        graphs: &mut Assets<AnimationGraph>,
        sniper_scene: Handle<Scene>,
        sniper_graph: Handle<AnimationGraph>,
        sniper_index: AnimationNodeIndex,
    ) -> Self {
        let clips = (0..AK_CLIP_COUNT)
            .map(|i| asset_server.load(GltfAssetLabel::Animation(i).from_asset(AK_MODEL)));
        let (graph, ak_nodes) = AnimationGraph::from_clips(clips);
        Self {
            sniper_scene,
            sniper_graph,
            sniper_index,
            ak_scene: asset_server.load(GltfAssetLabel::Scene(0).from_asset(AK_MODEL)),
            ak_graph: graphs.add(graph),
            ak_nodes,
        }
    }
}

/// Wear the primary we're carrying — in a mode with a loadout, the one the
/// server says we have this life (`LobbyMember::primary`; before the game,
/// our pick), the sniper otherwise; during a kill cam, the killer's: set
/// [`Weapon::primary`] (a full mag of it, reserve refilled for the mode), and
/// swap the view model's scene and animation graph in place. In a running
/// `Zombies` game the gun's our own to pick ([`Weapon::slots`] — wall buys,
/// pickups and swapping between two guns), so the view model just follows
/// [`Weapon::primary`].
pub(crate) fn sync_primary_model(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    killcam: Res<crate::killcam::ActiveKillCam>,
    models: Option<Res<PrimaryModels>>,
    view_model: Single<(&mut ViewModelAnimation, &mut SceneRoot), With<ViewModel>>,
    mut weapon: ResMut<Weapon>,
) {
    let me = local.iter().next().map(|l| l.0);
    let replayed = killcam.0.as_ref().and_then(|run| run.weapon());
    let wanted = replayed
        .or_else(|| {
            let lobby = lobbies.iter().find(|l| me.is_some_and(|me| l.has(me)))?;
            let m = lobby.members.iter().find(|m| Some(m.peer) == me)?;
            if !lobby.mode.has_loadout() {
                return None;
            }
            if lobby.started && lobby.mode == shared::GameMode::Zombies {
                return Some(weapon.primary);
            }
            Some(if lobby.started { m.primary } else { m.loadout })
        })
        .filter(|w| LOADOUT_WEAPONS.contains(w))
        .unwrap_or(WeaponId::Sniper);
    if weapon.primary != wanted {
        weapon.set_primary(wanted);
    }
    let Some(models) = models else { return };
    let (mut anim, mut scene) = view_model.into_inner();
    if anim.weapon == wanted {
        return;
    }
    let (scene_handle, graph, nodes) = match wanted {
        WeaponId::Ak74 => (models.ak_scene.clone(), models.ak_graph.clone(), models.ak_nodes.clone()),
        _ => (models.sniper_scene.clone(), models.sniper_graph.clone(), vec![models.sniper_index]),
    };
    *anim = ViewModelAnimation {
        graph,
        index: nodes[0],
        nodes,
        weapon: wanted,
    };
    // A new scene: `start_view_model_animation` arms it once it's in.
    scene.0 = scene_handle;
    info!("view model: now the {}", wanted.label());
}

/// The debug panel's "AK-74" section. `force_ads` is
/// `AdsTuning::force_full` (shared with the sniper's ADS section); `live` is
/// the idle / jump state right now (`Weapon::ak_loco`), to check the clips by.
pub(crate) fn ak_section(ui: &mut egui::Ui, s: &mut AkSettings, force_ads: &mut bool, live: AkLoco) {
    ui.label(format!("Idle / jump clip now: {live:?}"));
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
        ui.add(egui::Slider::new(&mut p.scale, 0.001f32..=0.1).logarithmic(true).text("scale"));
    };
    ui.collapsing("View model poses", |ui| {
        ui.checkbox(force_ads, "Force full ADS (ignore RMB) — to tune the aimed pose");
        pose(ui, "Hip", &mut s.hip);
        ui.separator();
        pose(ui, "Aimed (ADS)", &mut s.ads);
        ui.add(egui::Slider::new(&mut s.ads_zoom, 1.0f32..=4.0).text("ADS zoom (×)"));
    });
    ui.collapsing("Firing", |ui| {
        let mut rpm = 60.0 / s.fire_interval.max(0.01);
        if ui.add(egui::Slider::new(&mut rpm, 60.0f32..=1200.0).text("fire rate (RPM)")).changed() {
            s.fire_interval = 60.0 / rpm.max(1.0);
        }
        ui.add(egui::Slider::new(&mut s.hip_spread_deg, 0.0f32..=10.0).text("hip spread (°)"));
        ui.add(egui::Slider::new(&mut s.ads_spread_deg, 0.0f32..=5.0).text("ADS spread (°)"));
        ui.add(egui::Slider::new(&mut s.trauma_per_shot, 0.0f32..=1.0).text("shake per shot"));
        ui.add(egui::Slider::new(&mut s.recoil_kick, 0.0f32..=0.1).text("kick per shot (m)"));
        ui.label("Muzzle flash (camera space, at the hip — follows the gun)");
        ui.add(egui::Slider::new(&mut s.muzzle_translation.x, -1.0f32..=1.0).text("x"));
        ui.add(egui::Slider::new(&mut s.muzzle_translation.y, -1.0f32..=1.0).text("y"));
        ui.add(egui::Slider::new(&mut s.muzzle_translation.z, -3.0f32..=0.0).text("z"));
        ui.add(egui::Slider::new(&mut s.muzzle_size.x, 0.0f32..=2.0).text("width"));
        ui.add(egui::Slider::new(&mut s.muzzle_size.y, 0.0f32..=2.0).text("height"));
    });
    ui.collapsing("Animations", |ui| {
        ui.add(egui::Slider::new(&mut s.blend_secs, 0.0f32..=1.0).text("blend between clips (s)"));
        ui.add(egui::Slider::new(&mut s.shot_blend_secs, 0.0f32..=0.3).text("blend into a shot (s)"));
        ui.add(egui::Slider::new(&mut s.shot_speed, 0.1f32..=4.0).text("shot speed (×)"));
        ui.add(egui::Slider::new(&mut s.reload_speed, 0.1f32..=4.0).text("reload speed (×)"));
        ui.add(egui::Slider::new(&mut s.draw_speed, 0.1f32..=4.0).text("draw / hide speed (×)"));
        ui.add(egui::Slider::new(&mut s.idle_speed, 0.1f32..=4.0).text("idle speed (×)"));
        ui.add(egui::Slider::new(&mut s.jump_speed, 0.1f32..=4.0).text("jump clips speed (×)"));
    });
    ui.horizontal(|ui| {
        if ui.button("Copy AK-74 settings to console").clicked() {
            let p = |o: &ViewModelOffset| {
                format!(
                    "translation: Vec3::new({:.4}, {:.4}, {:.4}), yaw: {:.4}, pitch: {:.4}, scale: {:.5}",
                    o.translation.x, o.translation.y, o.translation.z, o.yaw, o.pitch, o.scale
                )
            };
            info!(
                "ak-74: hip {{ {} }}, ads {{ {} }}, ads_zoom: {:.2}, fire_interval: {:.3}, hip_spread_deg: {:.2}, \
                 ads_spread_deg: {:.2}, trauma_per_shot: {:.3}, recoil_kick: {:.4}, muzzle_translation: \
                 Vec3::new({:.3}, {:.3}, {:.3}), muzzle_size: Vec2::new({:.2}, {:.2}), blend_secs: {:.2}, \
                 shot_blend_secs: {:.3}, shot_speed: {:.2}, reload_speed: {:.2}, draw_speed: {:.2}, \
                 idle_speed: {:.2}, jump_speed: {:.2}",
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
                s.jump_speed,
            );
        }
        if ui.button("Reset AK-74 settings").clicked() {
            *s = AkSettings::default();
        }
    });
}
