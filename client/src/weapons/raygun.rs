//! The Ray Gun (`models/weapons/raygun.glb`) — `Zombies`' wonder weapon,
//! Call of Duty's: semi-auto, each shot a green bolt (`vfx::raygun`) that
//! bursts where it lands, the server hurting every zombie close by
//! (`shared::weapon::raygun_splash_damage`).
//!
//! It wears the same [`ViewModel`] the sniper and the AK-74 do
//! (`ak::sync_primary_model` swaps the scene in), so the sway, bob, ADS,
//! hiding and the rest work on it unchanged. Its clips — `idle`, `reload`,
//! `shoot` — cross-fade through the player's `AnimationTransitions` like the
//! AK's. It has no draw or put-away clip, so its Show / Hide are the idle
//! clip played on nodes of their own, and while one of those plays the gun
//! dips out of (or rises into) view in code ([`raygun_draw_lower`] →
//! [`GunLower`], which `ads::apply_ads` applies).
//!
//! There's no way to get one in a game yet but the debug panel's "Ray Gun"
//! window ([`raygun_debug_ui`]), which also tunes its look and feel
//! ([`RayGunSettings`]).

use bevy::animation::prelude::AnimationTransitions;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use lightyear::prelude::{LocalId, TriggerSender};
use shared::weapon::{SlotWeapon, WeaponId};

use super::view_model::{AnimationSegment, SegAct, SniperAnimationPlayer, ViewModel, ViewModelAnimation, ViewModelOffset};
use super::weapon::Weapon;
use crate::net::GameClient;

pub(crate) const RAYGUN_MODEL: &str = "models/weapons/raygun.glb";

/// `raygun.glb`'s clips, in file order — then the idle clip three more
/// times: the Show and Hide nodes (see the module docs), and the aimed hold
/// (its first frame, held still while aiming down sights).
pub(crate) const RAYGUN_CLIPS: [usize; 6] = [0, 1, 2, 0, 0, 0];
const CLIP_IDLE: usize = 0;
const CLIP_RELOAD: usize = 1;
const CLIP_SHOOT: usize = 2;
const CLIP_SHOW: usize = 3;
const CLIP_HIDE: usize = 4;
const CLIP_HOLD: usize = 5;

/// How long (clip seconds) its Show / Hide take at normal speed.
const DRAW_SECS: f32 = 0.4;

const SEG_IDLE: AnimationSegment = AnimationSegment::clip("Ray Gun Idle", CLIP_IDLE, 7.5);
const SEG_RELOAD: AnimationSegment =
    AnimationSegment::clip("Ray Gun Reload", CLIP_RELOAD, 3.75).with_act(SegAct::Reload);
const SEG_SHOOT: AnimationSegment = AnimationSegment::clip("Ray Gun Shoot", CLIP_SHOOT, 0.4167).with_act(SegAct::Shoot);
const SEG_SHOW: AnimationSegment = AnimationSegment::clip("Ray Gun Show", CLIP_SHOW, DRAW_SECS).with_act(SegAct::Show);
const SEG_HIDE: AnimationSegment = AnimationSegment::clip("Ray Gun Hide", CLIP_HIDE, DRAW_SECS).with_act(SegAct::Hide);

/// The Ray Gun's segment for `act` (one reload, full or not; no bolt).
pub(crate) fn raygun_seg(act: SegAct) -> AnimationSegment {
    match act {
        SegAct::Reload | SegAct::ReloadEmpty => SEG_RELOAD,
        SegAct::Hide => SEG_HIDE,
        SegAct::Show => SEG_SHOW,
        SegAct::Shoot | SegAct::Rechamber | SegAct::Other => SEG_SHOOT,
    }
}

/// The clip it rests in.
pub(crate) fn raygun_idle_seg() -> AnimationSegment {
    SEG_IDLE
}

/// Its still rest pose while aimed down sights (the idle clip's first
/// frame, not playing on).
pub(crate) fn raygun_hold_seg() -> AnimationSegment {
    AnimationSegment::clip("Ray Gun ADS Hold", CLIP_HOLD, 7.5)
}

/// Magazine size, and how many mags' worth (loaded one included) a
/// `Zombies` player carries at most — Call of Duty's 20 and 160.
pub(crate) const RAYGUN_MAG_SIZE: u32 = 20;
pub(crate) const RAYGUN_ZOMBIES_TOTAL_MAGS: u32 = 9;

/// The glTF names of its parts that wear the Pack-a-Punch camo.
pub(crate) const RAYGUN_CAMO_PARTS: [&str; 14] = [
    "Object_94",
    "Object_46",
    "Object_118",
    "Object_98",
    "Object_116",
    "Object_104",
    "Object_44",
    "Object_106",
    "Object_108",
    "Object_110",
    "Object_112",
    "Object_114",
    "Object_102",
    "barrel",
];

/// Everything tunable about the Ray Gun (the debug panel's "Ray Gun"
/// window).
#[derive(Resource, Clone)]
pub(crate) struct RayGunSettings {
    /// View-model poses at the hip and fully aimed.
    pub(crate) hip: ViewModelOffset,
    pub(crate) ads: ViewModelOffset,
    /// World zoom at full ADS (×).
    pub(crate) ads_zoom: f32,
    /// Least time (s) between shots.
    pub(crate) fire_interval: f32,
    /// Camera shake each shot adds, and its backward kick (m).
    pub(crate) trauma_per_shot: f32,
    pub(crate) recoil_kick: f32,
    /// Muzzle flash placement (camera space at the hip pose, m) and size.
    pub(crate) muzzle_translation: Vec3,
    pub(crate) muzzle_size: Vec2,
    /// Cross-fade between clips, and into each shot (s).
    pub(crate) blend_secs: f32,
    pub(crate) shot_blend_secs: f32,
    /// Playback speeds (×).
    pub(crate) shot_speed: f32,
    pub(crate) reload_speed: f32,
    pub(crate) draw_speed: f32,
    pub(crate) idle_speed: f32,
    /// How far (m) it drops, and how far (radians) it tips, when put away.
    pub(crate) lower_drop: f32,
    pub(crate) lower_tip: f32,
}

impl Default for RayGunSettings {
    fn default() -> Self {
        // `raygun.glb`'s camera sits at its origin looking down +Z, the gun
        // about 1.5 units long: turned round and shrunk to a pistol.
        let scale = 0.2;
        Self {
            hip: ViewModelOffset {
                translation: Vec3::new(0.0, 0.0, 0.0),
                yaw: std::f32::consts::PI,
                pitch: 0.0,
                scale,
            },
            ads: ViewModelOffset {
                translation: Vec3::new(-0.0963, 0.055, 0.08),
                yaw: std::f32::consts::PI,
                pitch: 0.0,
                scale,
            },
            ads_zoom: 1.25,
            fire_interval: 0.33,
            trauma_per_shot: 0.2,
            recoil_kick: 0.02,
            muzzle_translation: Vec3::new(0.05, -0.05, -0.46),
            muzzle_size: Vec2::new(0.25, 0.3),
            blend_secs: 0.15,
            shot_blend_secs: 0.03,
            shot_speed: 1.0,
            reload_speed: 1.0,
            draw_speed: 1.0,
            idle_speed: 1.0,
            lower_drop: 0.35,
            lower_tip: 0.7,
        }
    }
}

/// How far the gun in hand is put away right now (`0` up … `1` all the
/// way down) — only the Ray Gun's Show / Hide set it; `ads::apply_ads`
/// lowers the view model by it.
#[derive(Resource, Default, PartialEq)]
pub(crate) struct GunLower(pub(crate) f32);

/// Work out [`GunLower`] from the Ray Gun's Show / Hide node and how far
/// it's played.
pub(crate) fn raygun_draw_lower(
    view_model: Single<&ViewModelAnimation, With<ViewModel>>,
    players: Query<(&AnimationPlayer, &AnimationTransitions, &AnimationGraphHandle), With<SniperAnimationPlayer>>,
    mut lower: ResMut<GunLower>,
) {
    let vm = view_model.into_inner();
    let mut value = 0.0;
    if vm.weapon == WeaponId::RayGun {
        let (show, hide) = (vm.nodes.get(CLIP_SHOW).copied(), vm.nodes.get(CLIP_HIDE).copied());
        if let Some((player, transitions, _)) = players.iter().find(|(.., g)| g.0 == vm.graph) {
            let played = |node: Option<AnimationNodeIndex>| {
                node.and_then(|n| player.animation(n))
                    .map_or(1.0, |a| (a.seek_time() / DRAW_SECS).clamp(0.0, 1.0))
            };
            let main = transitions.get_main_animation();
            if main.is_some() && main == show {
                value = 1.0 - played(show);
            } else if main.is_some() && main == hide {
                value = played(hide);
            }
        }
    }
    // (Eased in and out.)
    let eased = value * value * (3.0 - 2.0 * value);
    lower.set_if_neq(GunLower(eased));
}

/// Debug: hand ourselves the Ray Gun (in a `Zombies` game — for free, in
/// place of the weapon in hand, which drops like a wall buy's), and tune it.
pub(crate) fn raygun_debug_ui(
    mut contexts: EguiContexts,
    mut settings: ResMut<RayGunSettings>,
    weapon: Res<Weapon>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut give: Query<&mut TriggerSender<shared::GiveWeapon>, With<GameClient>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let in_zombies = crate::zombies_hud::zombies_game(&local, &lobbies).is_some();
    egui::Window::new("Ray Gun")
        .default_open(false)
        .default_pos([20.0, 420.0])
        .show(ctx, |ui| {
            let carried = weapon.carries(SlotWeapon::Gun(WeaponId::RayGun));
            let button = ui.add_enabled(in_zombies && !carried, egui::Button::new("Equip the Ray Gun"));
            if !in_zombies {
                ui.label("(In a Zombies game.)");
            } else if carried {
                ui.label("(Carrying it.)");
            }
            if button.clicked() {
                let (mag, reserve) = weapon.held_ammo();
                if let Ok(mut s) = give.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::GiveWeapon {
                        weapon: WeaponId::RayGun,
                        slot: weapon.held as u8,
                        mag,
                        reserve,
                    });
                }
            }
            ui.separator();
            let s = &mut *settings;
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
                ui.add(egui::Slider::new(&mut p.scale, 0.01f32..=1.0).logarithmic(true).text("scale"));
            };
            ui.collapsing("View model poses", |ui| {
                pose(ui, "Hip", &mut s.hip);
                ui.separator();
                pose(ui, "Aimed (ADS)", &mut s.ads);
                ui.add(egui::Slider::new(&mut s.ads_zoom, 1.0f32..=3.0).text("ADS zoom (×)"));
            });
            ui.collapsing("Firing", |ui| {
                ui.add(egui::Slider::new(&mut s.fire_interval, 0.05f32..=1.0).text("between shots (s)"));
                ui.add(egui::Slider::new(&mut s.trauma_per_shot, 0.0f32..=1.0).text("shake per shot"));
                ui.add(egui::Slider::new(&mut s.recoil_kick, 0.0f32..=0.1).text("kick per shot (m)"));
                ui.label("Muzzle flash (camera space, at the hip)");
                ui.add(egui::Slider::new(&mut s.muzzle_translation.x, -1.0f32..=1.0).text("x"));
                ui.add(egui::Slider::new(&mut s.muzzle_translation.y, -1.0f32..=1.0).text("y"));
                ui.add(egui::Slider::new(&mut s.muzzle_translation.z, -2.0f32..=0.0).text("z"));
                ui.add(egui::Slider::new(&mut s.muzzle_size.x, 0.0f32..=1.0).text("width"));
                ui.add(egui::Slider::new(&mut s.muzzle_size.y, 0.0f32..=1.0).text("height"));
            });
            ui.collapsing("Animations", |ui| {
                ui.add(egui::Slider::new(&mut s.blend_secs, 0.0f32..=1.0).text("blend between clips (s)"));
                ui.add(egui::Slider::new(&mut s.shot_speed, 0.1f32..=4.0).text("shot speed (×)"));
                ui.add(egui::Slider::new(&mut s.reload_speed, 0.1f32..=4.0).text("reload speed (×)"));
                ui.add(egui::Slider::new(&mut s.draw_speed, 0.1f32..=4.0).text("draw / put away speed (×)"));
                ui.add(egui::Slider::new(&mut s.idle_speed, 0.1f32..=4.0).text("idle speed (×)"));
                ui.add(egui::Slider::new(&mut s.lower_drop, 0.0f32..=1.0).text("put away: drop (m)"));
                ui.add(egui::Slider::new(&mut s.lower_tip, 0.0f32..=1.5).text("put away: tip (rad)"));
            });
            ui.horizontal(|ui| {
                if ui.button("Copy Ray Gun settings to console").clicked() {
                    let p = |o: &ViewModelOffset| {
                        format!(
                            "translation: Vec3::new({:.4}, {:.4}, {:.4}), yaw: {:.4}, pitch: {:.4}, scale: {:.4}",
                            o.translation.x, o.translation.y, o.translation.z, o.yaw, o.pitch, o.scale
                        )
                    };
                    info!(
                        "ray gun: hip {{ {} }}, ads {{ {} }}, ads_zoom: {:.2}, fire_interval: {:.3}, \
                         trauma_per_shot: {:.3}, recoil_kick: {:.4}, muzzle_translation: Vec3::new({:.3}, {:.3}, {:.3}), \
                         muzzle_size: Vec2::new({:.2}, {:.2}), lower_drop: {:.2}, lower_tip: {:.2}",
                        p(&s.hip),
                        p(&s.ads),
                        s.ads_zoom,
                        s.fire_interval,
                        s.trauma_per_shot,
                        s.recoil_kick,
                        s.muzzle_translation.x,
                        s.muzzle_translation.y,
                        s.muzzle_translation.z,
                        s.muzzle_size.x,
                        s.muzzle_size.y,
                        s.lower_drop,
                        s.lower_tip,
                    );
                }
                if ui.button("Reset").clicked() {
                    *s = RayGunSettings::default();
                }
            });
        });
    Ok(())
}
