//! The "Debug Mode" egui tuning panel: lil-gui-style sliders for every
//! live-tunable setting across every game system, plus the key that frees
//! the cursor so they can be dragged.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use bevy_egui::{egui, EguiContexts};

use crate::audio::*;
use crate::avatars::*;
use crate::environment::*;
use crate::keybinds::KeyBindings;
use crate::player::*;
use crate::settings::Settings;
use crate::vfx::*;
use crate::weapons::*;
use crate::{menu, set_cursor_grabbed};

/// lil-gui-style panel for dialing in the ADS pose. Press `Esc` to free the
/// cursor, drag the sliders, `Esc` again to get back into the game.
#[allow(clippy::type_complexity)]
pub(crate) fn ads_tuning_ui(
    mut contexts: EguiContexts,
    mut poses: ResMut<ViewModelPoses>,
    mut tuning: ResMut<AdsTuning>,
    mut muzzle: ResMut<MuzzleFlashSettings>,
    mut smoke: ResMut<SmokeSettings>,
    mut rocks: ResMut<RockSettings>,
    mut dust: ResMut<DustSettings>,
    mut movement: ResMut<MovementSettings>,
    (mut slide_cfg, mut footsteps, mut sound_vol, mut crosshair_cfg): (
        ResMut<SlideSettings>,
        ResMut<FootstepSettings>,
        ResMut<SoundVolumes>,
        ResMut<CrosshairSettings>,
    ),
    mut sway: ResMut<WeaponSwaySettings>,
    mut shake_cfg: ResMut<ShakeSettings>,
    mut anim: ResMut<AnimationSettings>,
    mut scene: ResMut<SceneTuning>,
    mut tracer: ResMut<TracerSettings>,
    binds: Res<KeyBindings>,
    // Bundled — a system function tops out at 16 top-level params.
    misc: (
        Res<Shake>,
        Res<Ads>,
        ResMut<MapSettings>,
        ResMut<ShipmentSettings>,
        ResMut<BloodSettings>,
        ResMut<NoScopeSpread>,
        ResMut<IdleSwaySettings>,
        ResMut<AimSwaySettings>,
        ResMut<RemoteAvatarSettings>,
        ResMut<SoldierAnimSettings>,
        ResMut<RemoteSoundSettings>,
        ResMut<WaterSettings>,
        ResMut<ShipmentSceneTuning>,
        ResMut<ShipmentLightSettings>,
        (
            ResMut<RainSettings>,
            ResMut<KnifeViewModelSettings>,
            ResMut<FluoroLightSettings>,
            ResMut<BulbLightSettings>,
            ResMut<MantleSettings>,
            ResMut<ShipmentDaySceneTuning>,
            Res<Settings>,
            ResMut<LensSettings>,
        ),
    ),
) -> Result {
    let (
        shake,
        ads,
        mut map,
        mut shipment,
        mut blood,
        mut noscope,
        mut idle_sway,
        mut aim_sway,
        mut remote_avatar,
        mut soldier_anim,
        mut remote_sound,
        mut water,
        mut shipment_scene,
        mut shipment_light,
        (mut rain, mut knife_view, mut fluoro, mut bulbs, mut mantle_cfg, mut shipment_day_scene, settings, mut lens_cfg),
    ) = misc;
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("ADS tuning")
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
        .resizable(false)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.label(format!(
                "{}: free / lock the cursor",
                binds.cursor_toggle.label()
            ));
            ui.separator();
            ui.checkbox(&mut tuning.force_full, "Force full ADS (ignore RMB)");
            ui.label(format!("ads.t = {:.2}", ads.t));

            ui.separator();
            ui.collapsing("FOV", |ui| {
                let optic = Optic::live(&settings);
                ui.label(format!(
                    "{:.0}x scope at {:.0}° hip FOV: world {:.2}°, scope {:.2}°",
                    optic.zoom,
                    optic.hip_fov_deg,
                    full_ads_fov_rad(optic).to_degrees(),
                    full_scope_fov_rad(optic, &tuning).to_degrees(),
                ));
                ui.add(
                    egui::Slider::new(&mut tuning.lens_fit, 0.3f32..=1.2)
                        .text("lens fit  (scope tan / world tan; same for every zoom)"),
                );
                if ui.button("Reset FOV").clicked() {
                    tuning.lens_fit = AdsTuning::default().lens_fit;
                }
            });

            ui.separator();
            ui.collapsing("ADS speed", |ui| {
                ui.add(
                    egui::Slider::new(&mut tuning.ads_duration_ms, 20.0f32..=1000.0)
                        .text("ADS time (ms)  (lower = snappier)")
                        .suffix(" ms")
                        .max_decimals(0),
                );
                ui.add(
                    egui::Slider::new(&mut tuning.ads_ease, 0.0f32..=1.0)
                        .text("easing  (0 = linear, 1 = ease in/out)"),
                );
                ui.add(
                    egui::Slider::new(&mut tuning.scope_picture_at, 0.0f32..=0.95)
                        .text("scope picture in at (ads.t)  (higher = later)"),
                );
                if ui.button("Reset ADS speed").clicked() {
                    let d = AdsTuning::default();
                    tuning.ads_duration_ms = d.ads_duration_ms;
                    tuning.ads_ease = d.ads_ease;
                    tuning.scope_picture_at = d.scope_picture_at;
                }
            });

            ui.separator();
            ui.collapsing("ADS pose", |ui| {
                let a = &mut poses.ads;
                ui.add(egui::Slider::new(&mut a.translation.x, -0.4f32..=0.4).text("x  (right +)"));
                ui.add(egui::Slider::new(&mut a.translation.y, -0.4f32..=0.4).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut a.translation.z, -0.8f32..=0.0).text("z  (forward -)"),
                );
                ui.add(
                    egui::Slider::new(&mut a.yaw, (-PI)..=PI)
                        .text("yaw")
                        .step_by(0.001),
                );
                ui.add(
                    egui::Slider::new(&mut a.pitch, -0.6f32..=0.6)
                        .text("pitch")
                        .step_by(0.001),
                );
                ui.add(
                    egui::Slider::new(&mut a.scale, 0.001f32..=0.05)
                        .text("scale")
                        .logarithmic(true),
                );

                if ui.button("Copy pose to console").clicked() {
                    info!(
                        "ads: ViewModelOffset {{ translation: Vec3::new({:.4}, {:.4}, {:.4}), \
                         yaw: {:.4}, pitch: {:.4}, scale: {:.5} }},",
                        a.translation.x, a.translation.y, a.translation.z, a.yaw, a.pitch, a.scale,
                    );
                }
                if ui.button("Reset to default").clicked() {
                    *a = ViewModelPoses::default().ads;
                }
            });

            ui.separator();
            ui.collapsing("Hip pose", |ui| {
                let h = &mut poses.hip;
                ui.add(egui::Slider::new(&mut h.translation.x, -0.4f32..=0.4).text("x  (right +)"));
                ui.add(egui::Slider::new(&mut h.translation.y, -0.4f32..=0.4).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut h.translation.z, -0.8f32..=0.0).text("z  (forward -)"),
                );
                ui.add(
                    egui::Slider::new(&mut h.yaw, (-PI)..=PI)
                        .text("yaw")
                        .step_by(0.001),
                );
                ui.add(
                    egui::Slider::new(&mut h.pitch, -0.6f32..=0.6)
                        .text("pitch")
                        .step_by(0.001),
                );
                ui.add(
                    egui::Slider::new(&mut h.scale, 0.001f32..=0.05)
                        .text("scale")
                        .logarithmic(true),
                );

                if ui.button("Copy hip pose to console").clicked() {
                    info!(
                        "hip: ViewModelOffset {{ translation: Vec3::new({:.4}, {:.4}, {:.4}), \
                         yaw: {:.4}, pitch: {:.4}, scale: {:.5} }},",
                        h.translation.x, h.translation.y, h.translation.z, h.yaw, h.pitch, h.scale,
                    );
                }
                if ui.button("Reset hip pose to default").clicked() {
                    *h = ViewModelPoses::default().hip;
                }
            });

            ui.separator();
            ui.collapsing("Knife", |ui| {
                let k = &mut *knife_view;
                ui.label("models/knife.glb — position and scale only (see WeaponSlot::Secondary)");
                ui.label(
                    "Position range is wide on purpose — push it out past the normal hip-pose \
                     range to stand it next to a bot in world space and check the scale reads \
                     right, then dial it back in for the actual view-model pose.",
                );
                ui.add(
                    egui::Slider::new(&mut k.translation.x, -20.0f32..=20.0).text("x  (right +)"),
                );
                ui.add(egui::Slider::new(&mut k.translation.y, -20.0f32..=20.0).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut k.translation.z, -40.0f32..=5.0).text("z  (forward -)"),
                );
                ui.add(
                    egui::Slider::new(&mut k.scale, 0.001f32..=0.05)
                        .text("scale")
                        .logarithmic(true),
                );

                if ui.button("Copy knife pose to console").clicked() {
                    info!(
                        "knife: translation: Vec3::new({:.4}, {:.4}, {:.4}), scale: {:.5}",
                        k.translation.x, k.translation.y, k.translation.z, k.scale,
                    );
                }
                if ui.button("Reset knife pose to default").clicked() {
                    *k = KnifeViewModelSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Muzzle flash", |ui| {
                let m = &mut *muzzle;
                ui.label("position");
                ui.add(egui::Slider::new(&mut m.translation.x, -0.6f32..=0.6).text("x  (right +)"));
                ui.add(egui::Slider::new(&mut m.translation.y, -0.6f32..=0.6).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut m.translation.z, -2.0f32..=0.0).text("z  (forward -)"),
                );
                ui.label("size (m)");
                ui.add(egui::Slider::new(&mut m.size.x, 0.01f32..=2.0).text("width"));
                ui.add(egui::Slider::new(&mut m.size.y, 0.01f32..=2.0).text("height"));

                if ui.button("Copy muzzle flash to console").clicked() {
                    info!(
                        "muzzle: translation Vec3::new({:.4}, {:.4}, {:.4}), \
                         size Vec2::new({:.4}, {:.4})",
                        m.translation.x, m.translation.y, m.translation.z, m.size.x, m.size.y,
                    );
                }
                if ui.button("Reset muzzle flash").clicked() {
                    *m = MuzzleFlashSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Tracer", |ui| {
                let tr = &mut *tracer;
                ui.label("flash (just fired)");
                ui.add(
                    egui::Slider::new(&mut tr.flash_secs, 0.0f32..=0.3).text("flash duration (s)"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.flash_radius, 0.005f32..=0.15)
                        .text("flash radius (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.flash_emissive_boost, 0.0f32..=15.0)
                        .text("flash glow (emissive ×)"),
                );
                ui.horizontal(|ui| {
                    ui.label("flash color");
                    ui.color_edit_button_rgb(&mut tr.flash_color);
                });

                ui.label("smoke trail");
                ui.add(
                    egui::Slider::new(&mut tr.smoke_secs, 0.1f32..=8.0).text("fade duration (s)"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.smoke_start_alpha, 0.0f32..=1.0)
                        .text("starting opacity"),
                );
                ui.add(
                    egui::Slider::new(&mut tr.smoke_radius, 0.01f32..=0.4).text("end radius (m)"),
                );
                ui.horizontal(|ui| {
                    ui.label("smoke color");
                    ui.color_edit_button_rgb(&mut tr.smoke_color);
                });

                if ui.button("Reset tracer").clicked() {
                    *tr = TracerSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Map", |ui| {
                let mp = &mut *map;
                ui.label("position");
                ui.add(egui::Slider::new(&mut mp.position.x, -100.0f32..=100.0).text("x"));
                ui.add(egui::Slider::new(&mut mp.position.y, -20.0f32..=20.0).text("y"));
                ui.add(egui::Slider::new(&mut mp.position.z, -100.0f32..=100.0).text("z"));
                ui.add(
                    egui::Slider::new(&mut mp.rotation_deg, -180.0f32..=180.0)
                        .text("rotation°  (yaw)"),
                );
                ui.add(
                    egui::Slider::new(&mut mp.scale, 0.1f32..=5.0)
                        .text("scale")
                        .logarithmic(true),
                );

                if ui.button("Copy map transform to console").clicked() {
                    info!(
                        "map: position Vec3::new({:.2}, {:.2}, {:.2}), rotation_deg: {:.1}, \
                         scale: {:.3}",
                        mp.position.x, mp.position.y, mp.position.z, mp.rotation_deg, mp.scale,
                    );
                }
                if ui.button("Reset map transform").clicked() {
                    *mp = MapSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Shipment map", |ui| {
                let sh = &mut *shipment;
                ui.label("models/shipment.glb — spawned at the origin, scale only");
                ui.add(
                    egui::Slider::new(&mut sh.scale, 0.05f32..=2.0)
                        .text("scale")
                        .logarithmic(true),
                );
                ui.label(
                    "Only affects this client's own rendering + collision — the server's \
                     spawn/respawn placement always uses shared::map::SHIPMENT_SCALE, so \
                     update that constant to match once you've found the right number.",
                );

                if ui.button("Copy shipment scale to console").clicked() {
                    info!("shipment: SHIPMENT_SCALE = {:.3};", sh.scale);
                }
                if ui.button("Reset shipment scale").clicked() {
                    *sh = ShipmentSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Water", |ui| {
                let w = &mut *water;
                ui.label("Shipment / Shipment Day — the MW3-style cargo-ship setting's ocean plane");
                ui.add(
                    egui::Slider::new(&mut w.level_drop, -3.0f32..=8.0)
                        .text("level drop below ground (m)"),
                );
                ui.horizontal(|ui| {
                    ui.label("tint");
                    ui.color_edit_button_rgb(&mut w.tint);
                });
                ui.horizontal(|ui| {
                    ui.label("tint (Shipment Day)");
                    ui.color_edit_button_rgb(&mut w.day_tint);
                });
                ui.add(egui::Slider::new(&mut w.alpha, 0.0f32..=1.0).text("opacity"));
                ui.add(
                    egui::Slider::new(&mut w.roughness, 0.0f32..=1.0)
                        .text("roughness  (lower = shinier)"),
                );
                ui.add(egui::Slider::new(&mut w.reflectance, 0.0f32..=1.0).text("reflectance"));
                ui.add(
                    egui::Slider::new(&mut w.normal_tiling, 5.0f32..=200.0)
                        .text("ripple tiling  (higher = smaller ripples)")
                        .logarithmic(true),
                );
                ui.add(
                    egui::Slider::new(&mut w.scroll_speed.x, -0.1f32..=0.1).text("ripple scroll x"),
                );
                ui.add(
                    egui::Slider::new(&mut w.scroll_speed.y, -0.1f32..=0.1).text("ripple scroll y"),
                );

                if ui.button("Copy water settings to console").clicked() {
                    info!(
                        "water: level_drop: {:.3}, tint: Color::srgb({:.3}, {:.3}, {:.3}), \
                         alpha: {:.3}, roughness: {:.3}, reflectance: {:.3}, normal_tiling: \
                         {:.1}, scroll_speed: Vec2::new({:.4}, {:.4})",
                        w.level_drop,
                        w.tint[0],
                        w.tint[1],
                        w.tint[2],
                        w.alpha,
                        w.roughness,
                        w.reflectance,
                        w.normal_tiling,
                        w.scroll_speed.x,
                        w.scroll_speed.y,
                    );
                }
                if ui.button("Reset water").clicked() {
                    *w = WaterSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Remote players", |ui| {
                let ra = &mut *remote_avatar;
                ui.label("models/soldier.glb");
                ui.add(
                    egui::Slider::new(&mut ra.scale, 0.01f32..=100.0)
                        .text("scale")
                        .logarithmic(true),
                );
                if ui.button("Reset remote player scale").clicked() {
                    *ra = RemoteAvatarSettings::default();
                }

                ui.separator();
                ui.label("Player walk/sprint speed also drives these clips — see \"Movement\".");
                let sa = &mut *soldier_anim;
                ui.add(
                    egui::Slider::new(&mut sa.base_walk_speed, 0.1f32..=8.0)
                        .text(format!("walk anim speed (× at {WALK_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_sprint_speed, 0.1f32..=8.0)
                        .text(format!("sprint anim speed (× at {SPRINT_SPEED} m/s)")),
                );
                ui.label("No walk-and-shoot clip — runAndShooting covers both aiming states:");
                ui.add(
                    egui::Slider::new(&mut sa.base_aim_walk_speed, 0.1f32..=8.0)
                        .text(format!("aim+walk anim speed (× at {WALK_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_aim_sprint_speed, 0.1f32..=8.0)
                        .text(format!("aim+sprint anim speed (× at {SPRINT_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_crouch_walk_speed, 0.1f32..=8.0)
                        .text(format!("crouch walk anim speed (× at {CROUCH_SPEED} m/s)")),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_strafe_speed, 0.1f32..=8.0).text(format!(
                        "strafe anim speed (× at {} m/s)",
                        WALK_SPEED * STRAFE_SPEED_MULT
                    )),
                );
                ui.add(
                    egui::Slider::new(&mut sa.base_backpaddle_speed, 0.1f32..=8.0).text(format!(
                        "backpaddle anim speed (× at {} m/s)",
                        WALK_SPEED * BACKWARD_SPEED_MULT
                    )),
                );
                ui.add(
                    egui::Slider::new(&mut sa.death_speed, 0.1f32..=5.0)
                        .text("death anim speed (×)"),
                );
                if ui.button("Reset remote player anim speed").clicked() {
                    *sa = SoldierAnimSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Remote sounds", |ui| {
                ui.label("Other players' footsteps/jump/slide/reload/rechamber/shot/dive.");
                let rs = &mut *remote_sound;
                ui.add(egui::Slider::new(&mut rs.volume, 0.0f32..=3.0).text("volume (×)"));
                ui.add(
                    egui::Slider::new(&mut rs.max_distance, 5.0f32..=300.0)
                        .text("max distance (m)"),
                );
                if ui.button("Reset remote sounds").clicked() {
                    *rs = RemoteSoundSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Smoke", |ui| {
                let sm = &mut *smoke;
                ui.label("spawn offset (from camera)");
                ui.add(
                    egui::Slider::new(&mut sm.spawn_offset.x, -0.8f32..=0.8).text("x  (right +)"),
                );
                ui.add(egui::Slider::new(&mut sm.spawn_offset.y, -0.8f32..=0.8).text("y  (up +)"));
                ui.add(
                    egui::Slider::new(&mut sm.spawn_offset.z, -3.0f32..=0.0).text("z  (forward -)"),
                );
                ui.add(egui::Slider::new(&mut sm.scale, 0.02f32..=3.0).text("scale (m)"));
                ui.add(egui::Slider::new(&mut sm.rise_rate, 0.0f32..=4.0).text("rise rate (m/s)"));
                ui.add(egui::Slider::new(&mut sm.spread, 0.0f32..=2.0).text("spread (m/s)"));
                ui.add(egui::Slider::new(&mut sm.fade_in, 0.0f32..=5.0).text("fade in (s)"));
                ui.add(egui::Slider::new(&mut sm.fade_time, 0.1f32..=10.0).text("fade out (s)"));
                ui.add(
                    egui::Slider::new(&mut sm.spawn_rate, 0.0f32..=150.0).text("spawn rate (/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut sm.duration, 0.05f32..=5.0).text("burst duration (s)"),
                );
                ui.add(egui::Slider::new(&mut sm.max_opacity, 0.0f32..=1.0).text("max opacity"));

                if ui.button("Copy smoke to console").clicked() {
                    info!(
                        "smoke: spawn_offset Vec3::new({:.4}, {:.4}, {:.4}), scale {:.4}, \
                         rise_rate {:.4}, spread {:.4}, fade_in {:.4}, fade_time {:.4}, \
                         spawn_rate {:.4}, duration {:.4}, max_opacity {:.4}",
                        sm.spawn_offset.x,
                        sm.spawn_offset.y,
                        sm.spawn_offset.z,
                        sm.scale,
                        sm.rise_rate,
                        sm.spread,
                        sm.fade_in,
                        sm.fade_time,
                        sm.spawn_rate,
                        sm.duration,
                        sm.max_opacity,
                    );
                }
                if ui.button("Reset smoke").clicked() {
                    *sm = SmokeSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Impact rocks", |ui| {
                let r = &mut *rocks;
                ui.label("debris kicked up where a shot hits the ground");
                ui.add(egui::Slider::new(&mut r.count, 0u32..=40).text("rocks per hit"));
                ui.add(egui::Slider::new(&mut r.speed, 0.0f32..=20.0).text("launch speed (m/s)"));
                ui.add(egui::Slider::new(&mut r.spread_deg, 0.0f32..=90.0).text("cone spread (°)"));
                ui.add(egui::Slider::new(&mut r.gravity, 0.0f32..=60.0).text("gravity (m/s²)"));
                ui.add(egui::Slider::new(&mut r.spin, 0.0f32..=40.0).text("tumble (rad/s)"));
                ui.add(egui::Slider::new(&mut r.scale, 0.01f32..=0.5).text("size (m)"));
                ui.add(egui::Slider::new(&mut r.lifetime, 0.1f32..=4.0).text("lifetime (s)"));
                if ui.button("Reset rocks").clicked() {
                    *r = RockSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Impact dust", |ui| {
                let d = &mut *dust;
                ui.label("dust cloud where a shot hits the ground");
                ui.add(egui::Slider::new(&mut d.count, 0u32..=40).text("puffs per hit"));
                ui.add(egui::Slider::new(&mut d.speed, 0.0f32..=12.0).text("launch speed (m/s)"));
                ui.add(egui::Slider::new(&mut d.spread_deg, 0.0f32..=90.0).text("cone spread (°)"));
                ui.add(egui::Slider::new(&mut d.rise, 0.0f32..=4.0).text("extra rise (m/s)"));
                ui.add(egui::Slider::new(&mut d.drag, 0.0f32..=10.0).text("drag (/s)"));
                ui.add(egui::Slider::new(&mut d.start_scale, 0.02f32..=2.0).text("start size (m)"));
                ui.add(egui::Slider::new(&mut d.end_scale, 0.02f32..=4.0).text("end size (m)"));
                ui.add(egui::Slider::new(&mut d.lifetime, 0.1f32..=4.0).text("lifetime (s)"));
                ui.add(egui::Slider::new(&mut d.fade_in, 0.0f32..=1.0).text("fade in (s)"));
                ui.add(egui::Slider::new(&mut d.opacity, 0.0f32..=1.0).text("opacity"));
                if ui.button("Reset dust").clicked() {
                    *d = DustSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Blood splatter", |ui| {
                let b = &mut *blood;
                ui.label("squirted from a bot along the shot where it hits");
                ui.add(egui::Slider::new(&mut b.count, 0u32..=60).text("droplets per hit"));
                ui.add(egui::Slider::new(&mut b.speed, 0.0f32..=25.0).text("squirt speed (m/s)"));
                ui.add(egui::Slider::new(&mut b.spread_deg, 0.0f32..=90.0).text("spray cone (°)"));
                ui.add(egui::Slider::new(&mut b.gravity, 0.0f32..=60.0).text("gravity (m/s²)"));
                ui.add(egui::Slider::new(&mut b.drag, 0.0f32..=10.0).text("drag (/s)"));
                ui.add(egui::Slider::new(&mut b.scale, 0.01f32..=0.8).text("droplet size (m)"));
                ui.add(egui::Slider::new(&mut b.growth, 1.0f32..=4.0).text("grow ×  (over life)"));
                ui.add(egui::Slider::new(&mut b.lifetime, 0.1f32..=4.0).text("lifetime (s)"));
                ui.add(egui::Slider::new(&mut b.opacity, 0.0f32..=1.0).text("opacity"));
                ui.horizontal(|ui| {
                    ui.label("tint  (white = texture as-is)");
                    ui.color_edit_button_rgb(&mut b.color);
                });
                if ui.button("Reset blood").clicked() {
                    *b = BloodSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Footsteps", |ui| {
                let f = &mut *footsteps;
                ui.checkbox(&mut f.enabled, "enabled");
                ui.add(egui::Slider::new(&mut f.volume, 0.0f32..=10.0).text("overall volume"));
                ui.label("stride — metres per step (lower = faster cadence)");
                ui.add(egui::Slider::new(&mut f.walk_stride, 0.5f32..=5.0).text("walk"));
                ui.add(egui::Slider::new(&mut f.sprint_stride, 0.5f32..=5.0).text("sprint"));
                ui.add(egui::Slider::new(&mut f.crouch_stride, 0.5f32..=5.0).text("crouch"));
                ui.add(egui::Slider::new(&mut f.prone_stride, 0.5f32..=5.0).text("prone"));
                ui.label("per-stance volume");
                ui.add(egui::Slider::new(&mut f.walk_volume, 0.0f32..=1.0).text("walk"));
                ui.add(egui::Slider::new(&mut f.sprint_volume, 0.0f32..=1.0).text("sprint"));
                ui.add(egui::Slider::new(&mut f.crouch_volume, 0.0f32..=1.0).text("crouch"));
                ui.add(egui::Slider::new(&mut f.prone_volume, 0.0f32..=1.0).text("prone"));
                ui.add(
                    egui::Slider::new(&mut f.pitch_jitter, 0.0f32..=0.5).text("pitch jitter (±)"),
                );
                ui.add(
                    egui::Slider::new(&mut f.min_speed, 0.0f32..=3.0).text("stopped below (m/s)"),
                );
                if ui.button("Reset footsteps").clicked() {
                    *f = FootstepSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Sound volumes", |ui| {
                let v = &mut *sound_vol;
                ui.label("per-sound multiplier (1 = built-in level)");
                for (label, slot) in [
                    ("shot", &mut v.shot),
                    ("rechamber", &mut v.rechamber),
                    ("reload", &mut v.reload),
                    ("ambient", &mut v.ambient),
                    ("shipment ambient", &mut v.shipment_ambient),
                    ("aim in", &mut v.aim_in),
                    ("aim out", &mut v.aim_out),
                    ("out of ammo", &mut v.out_of_ammo),
                    ("slide", &mut v.slide),
                    ("dive", &mut v.dive),
                    ("kill enemy", &mut v.kill_enemy),
                    ("jump land", &mut v.jump_land),
                    ("teleport", &mut v.teleport),
                ] {
                    ui.add(egui::Slider::new(slot, 0.0f32..=10.0).text(label));
                }
                if ui.button("Reset sound volumes").clicked() {
                    *v = SoundVolumes::default();
                }
            });

            ui.separator();
            ui.collapsing("Movement", |ui| {
                let m = &mut *movement;
                ui.add(
                    egui::Slider::new(&mut m.walk_speed, 0.0f32..=20.0).text("walk speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.sprint_speed, 0.0f32..=30.0)
                        .text("sprint speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.strafe_speed_mult, 0.1f32..=1.5)
                        .text("strafe speed (×)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.backward_speed_mult, 0.1f32..=1.5)
                        .text("backward speed (×)"),
                );
                ui.add(egui::Slider::new(&mut m.gravity, 0.0f32..=60.0).text("gravity (m/s²)"));
                ui.add(
                    egui::Slider::new(&mut m.jump_speed, 0.0f32..=20.0).text("jump strength (m/s)"),
                );
                if ui.button("Reset movement").clicked() {
                    *m = MovementSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Slide", |ui| {
                let s = &mut *slide_cfg;
                ui.label("crouch = key while still; slide = key while moving; jump cancels");
                ui.add(egui::Slider::new(&mut s.crouch_drop, 0.0f32..=1.5).text("crouch drop (m)"));
                ui.add(
                    egui::Slider::new(&mut s.crouch_speed, 0.0f32..=8.0)
                        .text("crouch-walk speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.slide_speed, 0.0f32..=20.0)
                        .text("slide launch speed (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.sprint_bonus, 0.0f32..=12.0)
                        .text("sprint slide bonus (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.friction, 0.5f32..=25.0).text("slide friction (m/s²)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.min_speed, 0.1f32..=8.0).text("slide end speed (m/s)"),
                );
                ui.add(egui::Slider::new(&mut s.max_time, 0.2f32..=4.0).text("slide time cap (s)"));
                ui.add(
                    egui::Slider::new(&mut s.duck_speed, 2.0f32..=30.0).text("duck-down rate (/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.stand_speed, 2.0f32..=30.0).text("stand-up rate (/s)"),
                );
                if ui.button("Reset slide").clicked() {
                    *s = SlideSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Dive & prone", |ui| {
                let s = &mut *slide_cfg;
                ui.label("prone key while still toggles prone; while moving = dolphin dive");
                ui.add(egui::Slider::new(&mut s.prone_drop, 0.2f32..=1.6).text("prone drop (m)"));
                ui.add(
                    egui::Slider::new(&mut s.prone_speed, 0.0f32..=6.0).text("prone crawl (m/s)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.dive_speed, 0.0f32..=22.0)
                        .text("dive launch speed (m/s)"),
                );
                ui.add(egui::Slider::new(&mut s.dive_jump, 0.0f32..=12.0).text("dive hop (m/s)"));
                ui.add(
                    egui::Slider::new(&mut s.dive_tuck_speed, 2.0f32..=30.0)
                        .text("dive tuck rate (/s)"),
                );
                if ui.button("Reset dive & prone").clicked() {
                    let d = SlideSettings::default();
                    s.prone_drop = d.prone_drop;
                    s.prone_speed = d.prone_speed;
                    s.dive_speed = d.dive_speed;
                    s.dive_jump = d.dive_jump;
                    s.dive_tuck_speed = d.dive_tuck_speed;
                }
            });

            ui.separator();
            ui.collapsing("Mantle", |ui| {
                let m = &mut *mantle_cfg;
                ui.label(
                    "Player-facing on/off is Settings → Controls → Automatic Mantle, not here.",
                );
                ui.add(
                    egui::Slider::new(&mut m.min_height, 0.0f32..=1.5)
                        .text("min ledge height (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.max_height, 0.5f32..=3.5)
                        .text("max ledge height (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.probe_height, 0.2f32..=2.0)
                        .text("wall probe height (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.forward_dist, 0.1f32..=2.0)
                        .text("forward probe distance (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.ledge_probe_forward, 0.05f32..=1.0)
                        .text("ledge-top probe offset (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut m.duration, 0.1f32..=1.5).text("climb duration (s)"),
                );
                if ui.button("Reset mantle").clicked() {
                    *m = MantleSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Weapon sway", |ui| {
                let w = &mut *sway;
                ui.label(
                    "the gun angles away from the turn and catches up — this is the ADS \
                     sway now (the scope reticle itself never moves, see Crosshair)",
                );
                ui.add(
                    egui::Slider::new(&mut w.hip_strength, 0.0f32..=2.0)
                        .text("hip strength (s of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.ads_strength, 0.0f32..=1.0)
                        .text("ADS strength (s of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.return_speed, 1.0f32..=20.0).text("catch-up speed"),
                );
                ui.add(
                    egui::Slider::new(&mut w.max_offset_deg, 0.0f32..=30.0).text("max offset (°)"),
                );
                ui.label("shift — the gun also translates the way it's angled");
                ui.add(
                    egui::Slider::new(&mut w.hip_shift_m, 0.0f32..=0.1)
                        .text("hip shift (m per rad of lag)"),
                );
                ui.add(
                    egui::Slider::new(&mut w.ads_shift_m, 0.0f32..=0.2)
                        .text("ADS shift (m per rad of lag)"),
                );
                ui.label(format!(
                    "live strength @ ads.t {:.2} = {:.4}",
                    ads.t,
                    w.hip_strength.lerp(w.ads_strength, ads.t.clamp(0.0, 1.0)),
                ));

                if ui.button("Copy weapon sway to console").clicked() {
                    info!(
                        "weapon sway: hip_strength {:.4}, ads_strength {:.4}, \
                         return_speed {:.4}, max_offset_deg {:.4}, hip_shift_m {:.4}, \
                         ads_shift_m {:.4}",
                        w.hip_strength,
                        w.ads_strength,
                        w.return_speed,
                        w.max_offset_deg,
                        w.hip_shift_m,
                        w.ads_shift_m,
                    );
                }
                if ui.button("Reset weapon sway").clicked() {
                    *w = WeaponSwaySettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Idle sway", |ui| {
                let s = &mut *idle_sway;
                ui.label("weapon 'breathing' drift while standing still, hip only");
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.x, 0.0f32..=2.0).text("amplitude X (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.y, 0.0f32..=2.0).text("amplitude Y (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.x, 0.02f32..=1.0)
                        .text("frequency X (Hz)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.y, 0.02f32..=1.0)
                        .text("frequency Y (Hz)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.blend_speed, 0.2f32..=10.0)
                        .text("blend speed (stop / go)"),
                );
                if ui.button("Reset idle sway").clicked() {
                    *s = IdleSwaySettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Aim sway", |ui| {
                let s = &mut *aim_sway;
                ui.label(
                    "REAL aim breathing while scoped (scales up into ADS) — actually turns \
                     the camera, so it moves where a shot lands. The reticle stays pinned to \
                     the screen; the world drifts under it instead.",
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.x, 0.0f32..=1.0).text("amplitude X (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.amplitude_deg.y, 0.0f32..=1.0).text("amplitude Y (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.x, 0.02f32..=1.0)
                        .text("frequency X (Hz)"),
                );
                ui.add(
                    egui::Slider::new(&mut s.frequency_hz.y, 0.02f32..=1.0)
                        .text("frequency Y (Hz)"),
                );
                if ui.button("Reset aim sway").clicked() {
                    *s = AimSwaySettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Crosshair", |ui| {
                let c = &mut *crosshair_cfg;

                ui.label("size on the glass");
                ui.add(
                    egui::Slider::new(&mut c.scale, 0.3f32..=3.0)
                        .text("scale  (>1 pushes the ends past the edge)"),
                );

                ui.label("aim-in drift — starts off-centre, slides to the middle");
                ui.add(
                    egui::Slider::new(&mut c.aim_in_frac.x, -10.0f32..=10.0)
                        .text("start X  (+ = left, × scope half-view)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.aim_in_frac.y, -10.0f32..=10.0)
                        .text("start Y  (+ = up, × scope half-view)"),
                );
                ui.label("counter-sway — reticle moves opposite the weapon sway");
                ui.add(
                    egui::Slider::new(&mut c.sway_counter.x, -10.0f32..=10.0)
                        .text("counter X  (half-views per rad of yaw sway)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.sway_counter.y, -10.0f32..=10.0)
                        .text("counter Y  (half-views per rad of pitch sway)"),
                );
                ui.label(
                    "that raise slide is the reticle's only motion — no turn lag any more, \
                     it's pinned to dead centre once scoped (see Weapon sway / Aim sway \
                     instead: that's where the sway went).",
                );

                ui.checkbox(&mut c.center_dot_always, "keep centre dot on (no fade)");

                if ui.button("Reset crosshair").clicked() {
                    *c = CrosshairSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Scope lens (hip)", |ui| {
                let l = &mut *lens_cfg;
                ui.label("how the lens glass looks when not aiming — it fades to a matte black backing as you scope in");
                ui.horizontal(|ui| {
                    ui.label("tint");
                    egui::color_picker::color_edit_button_rgb(ui, &mut l.tint);
                });
                ui.add(egui::Slider::new(&mut l.alpha, 0.0f32..=1.0).text("opacity"));
                ui.add(
                    egui::Slider::new(&mut l.roughness, 0.0f32..=1.0)
                        .text("roughness  (low = tight, mirror-like glint)"),
                );
                ui.add(egui::Slider::new(&mut l.metallic, 0.0f32..=1.0).text("metallic"));
                ui.add(egui::Slider::new(&mut l.reflectance, 0.0f32..=1.0).text("reflectance"));
                if ui.button("Reset lens").clicked() {
                    *l = LensSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Camera shake", |ui| {
                let c = &mut *shake_cfg;
                ui.label("per-shot kick — up/down + side/side only");
                ui.add(
                    egui::Slider::new(&mut c.trauma_per_shot, 0.0f32..=1.0).text("trauma per shot"),
                );
                ui.add(egui::Slider::new(&mut c.decay, 0.5f32..=12.0).text("trauma decay (/s)"));
                ui.add(egui::Slider::new(&mut c.frequency, 5.0f32..=120.0).text("frequency"));
                ui.add(
                    egui::Slider::new(&mut c.pos_max, 0.0f32..=0.4)
                        .text("up/down + L/R amount (m)"),
                );
                ui.add(egui::Slider::new(&mut c.ads_scale, 0.0f32..=1.0).text(
                    "ADS scale — jitter + punch + shudder left at full ADS \
                         (ramps to full at the hip)",
                ));
                ui.separator();
                ui.label("view punch — rotates gun + cameras together (scaled by ADS scale)");
                ui.add(
                    egui::Slider::new(&mut c.view_punch_deg, 0.0f32..=12.0)
                        .text("view punch up (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.view_jitter_deg, 0.0f32..=6.0)
                        .text("view punch chaos (°)"),
                );
                ui.separator();
                ui.label(
                    "weapon shudder — gun kicks back toward the eye, muzzle climbs \
                     (scaled by ADS scale)",
                );
                ui.add(
                    egui::Slider::new(&mut c.weapon_kick, 0.0f32..=0.4)
                        .text("weapon kick back (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.weapon_kick_deg, 0.0f32..=15.0)
                        .text("weapon muzzle climb (°)"),
                );
                ui.separator();
                ui.label("front/back: eye punches back off the scope, then returns");
                ui.add(
                    egui::Slider::new(&mut c.recoil_kick, 0.0f32..=2.0).text("backward kick (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut c.recoil_return, 2.0f32..=60.0)
                        .text("return speed (/s)"),
                );
                ui.label(format!("recoil now: {:.3} m", shake.recoil));

                if ui.button("Copy camera shake to console").clicked() {
                    info!(
                        "camera shake: trauma_per_shot {:.4}, decay {:.4}, frequency {:.4}, \
                         pos_max {:.4}, view_punch_deg {:.4}, view_jitter_deg {:.4}, \
                         weapon_kick {:.4}, weapon_kick_deg {:.4}, recoil_kick {:.4}, \
                         recoil_return {:.4}, ads_scale {:.4}",
                        c.trauma_per_shot,
                        c.decay,
                        c.frequency,
                        c.pos_max,
                        c.view_punch_deg,
                        c.view_jitter_deg,
                        c.weapon_kick,
                        c.weapon_kick_deg,
                        c.recoil_kick,
                        c.recoil_return,
                        c.ads_scale,
                    );
                }
                if ui.button("Reset camera shake").clicked() {
                    *c = ShakeSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("No-scope spread", |ui| {
                let n = &mut *noscope;
                ui.label("random up/down + L/R miss angle — wide at the hip, gone at full ADS");
                ui.add(
                    egui::Slider::new(&mut n.hip_max_deg, 0.0f32..=15.0)
                        .text("max miss at hip (°)"),
                );
                ui.add(
                    egui::Slider::new(&mut n.curve, 1.0f32..=6.0)
                        .text("accuracy curve  (1 = linear, higher = tightens late)"),
                );
                ui.label(format!(
                    "cap now @ ads.t {:.2}: ±{:.2}°",
                    ads.t,
                    noscope_spread_angle(n, ads.t).to_degrees(),
                ));
                if ui.button("Reset no-scope spread").clicked() {
                    *n = NoScopeSpread::default();
                }
            });

            ui.separator();
            ui.collapsing("Animations", |ui| {
                let a = &mut *anim;
                ui.add(
                    egui::Slider::new(&mut a.rechamber_speed, 0.1f32..=4.0)
                        .text("rechamber speed (×)"),
                );
                if ui.button("Reset animations").clicked() {
                    *a = AnimationSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Fog & Sky (Basic Map)", |ui| {
                scene_tuning_sliders(ui, &mut scene);
                if ui.button("Reset fog & sky").clicked() {
                    *scene = SceneTuning::default();
                }
            });

            ui.separator();
            ui.collapsing("Fog & Sky (Shipment)", |ui| {
                ui.label("MW3-style setting: dark, foggy, overcast, out on open water");
                scene_tuning_sliders(ui, &mut shipment_scene.0);
                if ui.button("Reset fog & sky").clicked() {
                    *shipment_scene = ShipmentSceneTuning::default();
                }
            });

            ui.separator();
            ui.collapsing("Fog & Sky (Shipment Day)", |ui| {
                ui.label("Bright, clear daytime — barely any fog, no rain");
                scene_tuning_sliders(ui, &mut shipment_day_scene.0);
                if ui.button("Reset fog & sky").clicked() {
                    *shipment_day_scene = ShipmentDaySceneTuning::default();
                }
            });

            ui.separator();
            ui.collapsing("Shipment Lights", |ui| {
                ui.checkbox(
                    &mut shipment_light.markers_visible,
                    "show position/aim markers",
                );
                ui.label(
                    "Off by default — the bulb + rod gizmo is only there to help place the \
                     lights, not something to leave on.",
                );
            });

            ui.separator();
            ui.collapsing("Shipment Light 1", |ui| {
                shipment_light_sliders(ui, &mut shipment_light.lights[0], 0);
            });

            ui.separator();
            ui.collapsing("Shipment Light 2", |ui| {
                shipment_light_sliders(ui, &mut shipment_light.lights[1], 1);
            });

            ui.separator();
            ui.collapsing("Fluorescent Light", |ui| {
                let f = &mut *fluoro;
                ui.label("Shipment only — the tube fixture model inside one container");
                ui.checkbox(&mut f.markers_visible, "show position marker");
                ui.add(egui::Slider::new(&mut f.position.x, -80.0f32..=80.0).text("x"));
                ui.add(egui::Slider::new(&mut f.position.y, 0.0f32..=60.0).text("y (height)"));
                ui.add(egui::Slider::new(&mut f.position.z, -80.0f32..=80.0).text("z"));
                point_light_sliders(
                    ui,
                    &mut f.color,
                    &mut f.intensity,
                    &mut f.range,
                    &mut f.shadows_enabled,
                );

                if ui.button("Copy fluorescent light to console").clicked() {
                    info!(
                        "fluoro: position: Vec3::new({:.2}, {:.2}, {:.2}), color: \
                         Color::srgb({:.3}, {:.3}, {:.3}), intensity: {:.0}, range: {:.1}, \
                         shadows_enabled: {}",
                        f.position.x,
                        f.position.y,
                        f.position.z,
                        f.color[0],
                        f.color[1],
                        f.color[2],
                        f.intensity,
                        f.range,
                        f.shadows_enabled,
                    );
                }
                if ui.button("Reset fluorescent light").clicked() {
                    *f = FluoroLightSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Bulb Lights", |ui| {
                let b = &mut *bulbs;
                ui.label(
                    "Shipment only — two bare bulbs in another container; everything below is \
                     shared between both, only position is per-bulb",
                );
                ui.checkbox(&mut b.markers_visible, "show position markers");
                ui.label("Bulb 1 position");
                ui.add(egui::Slider::new(&mut b.positions[0].x, -80.0f32..=80.0).text("x"));
                ui.add(
                    egui::Slider::new(&mut b.positions[0].y, 0.0f32..=60.0).text("y (height)"),
                );
                ui.add(egui::Slider::new(&mut b.positions[0].z, -80.0f32..=80.0).text("z"));
                ui.label("Bulb 2 position");
                ui.add(egui::Slider::new(&mut b.positions[1].x, -80.0f32..=80.0).text("x"));
                ui.add(
                    egui::Slider::new(&mut b.positions[1].y, 0.0f32..=60.0).text("y (height)"),
                );
                ui.add(egui::Slider::new(&mut b.positions[1].z, -80.0f32..=80.0).text("z"));
                ui.separator();
                ui.label("Shared");
                point_light_sliders(
                    ui,
                    &mut b.color,
                    &mut b.intensity,
                    &mut b.range,
                    &mut b.shadows_enabled,
                );

                if ui.button("Copy bulb lights to console").clicked() {
                    info!(
                        "bulbs: positions: [Vec3::new({:.2}, {:.2}, {:.2}), Vec3::new({:.2}, \
                         {:.2}, {:.2})], color: Color::srgb({:.3}, {:.3}, {:.3}), intensity: \
                         {:.0}, range: {:.1}, shadows_enabled: {}",
                        b.positions[0].x,
                        b.positions[0].y,
                        b.positions[0].z,
                        b.positions[1].x,
                        b.positions[1].y,
                        b.positions[1].z,
                        b.color[0],
                        b.color[1],
                        b.color[2],
                        b.intensity,
                        b.range,
                        b.shadows_enabled,
                    );
                }
                if ui.button("Reset bulb lights").clicked() {
                    *b = BulbLightSettings::default();
                }
            });

            ui.separator();
            ui.collapsing("Rain", |ui| {
                let r = &mut *rain;
                ui.label("Shipment only — real 3D streaks, not a screen overlay");
                ui.checkbox(&mut r.enabled, "enabled");
                ui.add(egui::Slider::new(&mut r.count, 0..=RAIN_MAX_DROPS).text("streak count"));
                ui.add(
                    egui::Slider::new(&mut r.radius, 2.0f32..=60.0)
                        .text("radius around player (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut r.spawn_height, 2.0f32..=60.0)
                        .text("spawn height above player (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut r.fall_speed, 0.5f32..=30.0).text("fall speed (m/s)"),
                );
                ui.add(egui::Slider::new(&mut r.wind.x, -10.0f32..=10.0).text("wind x (m/s)"));
                ui.add(egui::Slider::new(&mut r.wind.y, -10.0f32..=10.0).text("wind z (m/s)"));
                ui.separator();
                ui.label("Streak look");
                ui.add(
                    egui::Slider::new(&mut r.streak_length, 0.05f32..=3.0)
                        .text("streak length (m)"),
                );
                ui.add(
                    egui::Slider::new(&mut r.streak_radius, 0.002f32..=0.1)
                        .text("streak thickness (m)"),
                );
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut r.color);
                    ui.label("colour");
                });
                ui.add(egui::Slider::new(&mut r.opacity, 0.0f32..=1.0).text("opacity"));

                if ui.button("Reset rain").clicked() {
                    *r = RainSettings::default();
                }
            });
        });
    Ok(())
}

/// Position/aim/cone sliders + copy/reset buttons for one [`ShipmentLight`]
/// slot — shared by "Shipment Light 1" and "Shipment Light 2". `index` is
/// only needed for the reset button, to pull that slot's own default back
/// out of [`ShipmentLightSettings::default`] rather than some other light's.
pub(crate) fn shipment_light_sliders(ui: &mut egui::Ui, l: &mut ShipmentLight, index: usize) {
    ui.add(egui::Slider::new(&mut l.position.x, -80.0f32..=80.0).text("x"));
    ui.add(egui::Slider::new(&mut l.position.y, 0.0f32..=60.0).text("y (height)"));
    ui.add(egui::Slider::new(&mut l.position.z, -80.0f32..=80.0).text("z"));
    ui.add(egui::Slider::new(&mut l.yaw_deg, -180.0f32..=180.0).text("yaw°  (heading)"));
    ui.add(
        egui::Slider::new(&mut l.pitch_deg, -89.0f32..=89.0).text("pitch°  (negative tilts down)"),
    );
    ui.separator();
    ui.label("Cone / beam");
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut l.color);
        ui.label("colour");
    });
    ui.add(
        egui::Slider::new(&mut l.intensity, 0.0f32..=100_000_000.0)
            .logarithmic(true)
            .text("intensity (lumens)"),
    );
    ui.add(egui::Slider::new(&mut l.range, 1.0f32..=200.0).text("range (m)"));
    ui.add(
        egui::Slider::new(&mut l.inner_angle_deg, 0.0f32..=89.0)
            .text("inner cone half-angle°  (hard core)"),
    );
    ui.add(
        egui::Slider::new(&mut l.outer_angle_deg, 0.0f32..=89.0)
            .text("outer cone half-angle°  (full spread — the \"triangle\")"),
    );
    ui.checkbox(&mut l.shadows_enabled, "cast shadows");
    ui.separator();
    ui.label("Glow  (the always-visible bulb at the fixture — see ShipmentLightGlow)");
    ui.add(
        egui::Slider::new(&mut l.glow_intensity, 0.0f32..=20_000_000.0)
            .logarithmic(true)
            .text("glow intensity (lumens)"),
    );

    if ui.button("Copy light settings to console").clicked() {
        info!(
            "shipment light {index}: position: Vec3::new({:.2}, {:.2}, {:.2}), yaw_deg: {:.1}, \
             pitch_deg: {:.1}, color: Color::srgb({:.3}, {:.3}, {:.3}), intensity: {:.0}, \
             range: {:.1}, inner_angle_deg: {:.1}, outer_angle_deg: {:.1}, shadows_enabled: {}, \
             glow_intensity: {:.0}",
            l.position.x,
            l.position.y,
            l.position.z,
            l.yaw_deg,
            l.pitch_deg,
            l.color[0],
            l.color[1],
            l.color[2],
            l.intensity,
            l.range,
            l.inner_angle_deg,
            l.outer_angle_deg,
            l.shadows_enabled,
            l.glow_intensity,
        );
    }
    if ui.button("Reset light").clicked() {
        *l = ShipmentLightSettings::default().lights[index].clone();
    }
}

/// Colour/intensity/range/shadow sliders shared by the "Fluorescent Light"
/// and "Bulb Lights" debug-panel sections — the parts of a `PointLight` that
/// aren't position.
pub(crate) fn point_light_sliders(
    ui: &mut egui::Ui,
    color: &mut [f32; 3],
    intensity: &mut f32,
    range: &mut f32,
    shadows_enabled: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(color);
        ui.label("colour");
    });
    ui.add(
        egui::Slider::new(intensity, 0.0f32..=6_000_000.0)
            .logarithmic(true)
            .text("intensity (lumens)"),
    );
    ui.add(egui::Slider::new(range, 0.5f32..=60.0).text("range (m)"));
    ui.checkbox(shadows_enabled, "cast shadows");
}

/// In debug mode, the "Lock / Unlock Cursor" key (rebindable, `L` by default)
/// frees the cursor so egui sliders can be dragged, and locks it again. Also
/// re-locks automatically if debug mode is switched off while the cursor is loose.
pub(crate) fn debug_cursor_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    settings: Res<Settings>,
    menu: Res<menu::Menu>,
    window: Single<&mut Window, With<PrimaryWindow>>,
) {
    if menu.is_open() {
        return; // the Esc menu owns the cursor
    }
    let mut window = window.into_inner();
    let loose = window.cursor_options.grab_mode == CursorGrabMode::None;

    if !settings.debug_mode {
        if loose {
            set_cursor_grabbed(&mut window, true);
        }
        return;
    }
    if binds.cursor_toggle.just_pressed(&keys, &mouse) {
        set_cursor_grabbed(&mut window, loose);
    }
}
