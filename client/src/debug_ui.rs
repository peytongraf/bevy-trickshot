//! The "Debug Mode" egui tuning panel: lil-gui-style sliders for every
//! live-tunable setting across every game system, plus the key that frees
//! the cursor so they can be dragged.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use bevy_egui::{egui, EguiContexts};

use crate::health::LocalHealth;
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
    (mut slide_cfg, mut footsteps, mut sound_vol, mut crosshair_cfg, mut knife_sounds, local_health, mut drink, mut nitro, mut drunk, local_id, lobbies, mut bots_passive_tx, mut shroom_kick, mut drunk_kick, mut break_point_night_scene, (mut flashlight, (mut machines, mut classic), _current_map, mut map_lights, mut round_anim, mut explosion, mut bomb_test_tx, mut kanga, mut zombie_look, zombie_readout, mut zombie_voice, mut power_lever, mut pap, mut hum, (mut ammo_crate, mut power_ups, mut power_up_test_tx, mut molotov_dbg, mut drop_power_up_tx, mut ak_cfg), mut bots_frozen_tx)): (
        ResMut<SlideSettings>,
        ResMut<FootstepSettings>,
        ResMut<SoundVolumes>,
        ResMut<CrosshairSettings>,
        ResMut<KnifeSounds>,
        Res<LocalHealth>,
        ResMut<crate::DrinkArmsSettings>,
        ResMut<crate::zombies_hud::NitroBrew>,
        ResMut<crate::DrunkSettings>,
        Query<&lightyear::prelude::LocalId, With<crate::net::GameClient>>,
        Query<&shared::Lobby>,
        Query<
            &mut lightyear::prelude::TriggerSender<shared::SetBotsPassive>,
            With<crate::net::GameClient>,
        >,
        ResMut<crate::ShroomKick>,
        ResMut<crate::DrunkKick>,
        ResMut<BreakPointNightSceneTuning>,
        (
            ResMut<FlashlightSettings>,
            (ResMut<crate::zombies_hud::PerkMachineSettings>, ResMut<crate::zombies_hud::ClassicPerks>),
            Res<crate::CurrentMap>,
            ResMut<crate::power::MapLightSettings>,
            ResMut<crate::round_counter::RoundAnimSettings>,
            ResMut<ExplosionSettings>,
            Query<
                &mut lightyear::prelude::TriggerSender<shared::SetBombTest>,
                With<crate::net::GameClient>,
            >,
            ResMut<crate::zombies_hud::Kangabrew>,
            ResMut<ZombieAvatarSettings>,
            Res<ZombieAnimReadout>,
            ResMut<crate::zombie_sounds::ZombieSoundSettings>,
            ResMut<crate::power::PowerLeverSettings>,
            ResMut<crate::pap::PapSettings>,
            ResMut<crate::power::MachineHumSettings>,
            (
                ResMut<crate::ammo_crate::AmmoCrateSettings>,
                ResMut<crate::power_ups::PowerUpSettings>,
                Query<
                    &mut lightyear::prelude::TriggerSender<shared::SetPowerUpTest>,
                    With<crate::net::GameClient>,
                >,
                crate::molotov::MolotovDebug,
                Query<
                    &mut lightyear::prelude::TriggerSender<shared::DropPowerUp>,
                    With<crate::net::GameClient>,
                >,
                ResMut<crate::AkSettings>,
            ),
            Query<
                &mut lightyear::prelude::TriggerSender<shared::SetBotsFrozen>,
                With<crate::net::GameClient>,
            >,
        ),
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
        ResMut<LedgeJumpSettings>,
        (
            ResMut<RainSettings>,
            ResMut<KnifeViewModelSettings>,
            ResMut<ThrowArmsSettings>,
            ResMut<ThrowKnifeModelSettings>,
            ResMut<BulletHoleSettings>,
            ResMut<FluoroLightSettings>,
            ResMut<BulbLightSettings>,
            ResMut<MantleSettings>,
            ResMut<ShipmentDaySceneTuning>,
            ResMut<BreakPointSceneTuning>,
            Res<Settings>,
            ResMut<LensSettings>,
            (ResMut<SniperGlintSettings>, ResMut<crate::RemoteMuzzleSettings>, ResMut<crate::AimRecoilSettings>),
            ResMut<ShroomSettings>,
            (ResMut<crate::hud::NameTagSettings>, ResMut<crate::hud::HealthBarSettings>),
            (
                ResMut<BotLookSettings>,
                (
                    ResMut<crate::dogs::DogSettings>,
                    ResMut<crate::dogs::DogPreview>,
                    Res<crate::dogs::DogReadout>,
                ),
            ),
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
        mut ledge_jump,
        (mut rain, mut knife_view, mut arms_view, mut knife_model, mut bullet_holes, mut fluoro, mut bulbs, mut mantle_cfg, mut shipment_day_scene, mut break_point_scene, settings, mut lens_cfg, (mut sniper_glint, mut remote_muzzle, mut aim_recoil), mut shroom, (mut name_tags, mut health_bars), (mut bot_look, (mut dog_settings, mut dog_preview, dog_readout))),
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
            ui.collapsing("Bots", |ui| {
                // Lives on the server's lobby (`Lobby::bots_passive`) — the
                // box shows what the server has, and a click asks it to flip.
                let me = local_id.iter().next().map(|l| l.0);
                let lobby = me.and_then(|me| lobbies.iter().find(|l| l.has(me)).map(|l| (l, l.leader == me)));
                let Some((lobby, is_leader)) = lobby else {
                    ui.label("Not in a lobby.");
                    return;
                };
                let mut passive = lobby.bots_passive;
                if ui
                    .add_enabled(is_leader, egui::Checkbox::new(&mut passive, "Bots don't attack"))
                    .changed()
                {
                    if let Ok(mut tx) = bots_passive_tx.single_mut() {
                        tx.trigger::<shared::LobbyChannel>(shared::SetBotsPassive { passive });
                    }
                }
                ui.label(if is_leader {
                    "Zombies and Free For All bots still move and chase you, but never fire. \
                     Stays set for this lobby until turned off."
                } else {
                    "Only the party leader can change this."
                });
                // Likewise `Lobby::bots_frozen`.
                let mut frozen = lobby.bots_frozen;
                if ui
                    .add_enabled(is_leader, egui::Checkbox::new(&mut frozen, "Bots stay in place"))
                    .changed()
                {
                    if let Ok(mut tx) = bots_frozen_tx.single_mut() {
                        tx.trigger::<shared::LobbyChannel>(shared::SetBotsFrozen { frozen });
                    }
                }
                ui.label(if is_leader {
                    "Zombies and bots stand where they are and do nothing — no chasing, firing \
                     or swiping. Stays set for this lobby until turned off."
                } else {
                    "Only the party leader can change this."
                });
            });

            ui.separator();
            ui.collapsing("Weapon & arms", |ui| {
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
                ui.collapsing("AK-74", |ui| {
                    ui.label("Poses, firing and animations for the AK-74 — pick it in the loadout (Zombies / Free For All).");
                    let live = molotov_dbg.weapon.ak_loco();
                    crate::weapons::ak_section(ui, &mut ak_cfg, &mut tuning.force_full, live);
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
                    ui.separator();
                    ui.label(
                        "walking / sprinting bob — every weapon (sniper, AK-74, knife, \
                         throwing knife); × Nitro Brew's movement speed-up",
                    );
                    ui.add(egui::Slider::new(&mut w.walk_bob, 0.0f32..=0.05).text("bob size (m)"));
                    ui.add(egui::Slider::new(&mut w.walk_bob_hz, 0.0f32..=4.0).text("bob swings / s walking"));
                    ui.add(
                        egui::Slider::new(&mut w.sprint_bob_hz, 0.0f32..=6.0).text("bob swings / s sprinting"),
                    );
                    ui.add(egui::Slider::new(&mut w.walk_bob_ads, 0.0f32..=1.0).text("bob left when aimed"));
                    ui.add(
                        egui::Slider::new(&mut w.bob_move_threshold, 0.0f32..=3.0)
                            .text("counts as moving above (m/s)"),
                    );

                    if ui.button("Copy weapon sway to console").clicked() {
                        info!(
                            "weapon sway: hip_strength {:.4}, ads_strength {:.4}, \
                             return_speed {:.4}, max_offset_deg {:.4}, hip_shift_m {:.4}, \
                             ads_shift_m {:.4}, walk_bob {:.4}, walk_bob_hz {:.2}, \
                             sprint_bob_hz {:.2}, walk_bob_ads {:.2}, bob_move_threshold {:.2}",
                            w.hip_strength,
                            w.ads_strength,
                            w.return_speed,
                            w.max_offset_deg,
                            w.hip_shift_m,
                            w.ads_shift_m,
                            w.walk_bob,
                            w.walk_bob_hz,
                            w.sprint_bob_hz,
                            w.walk_bob_ads,
                            w.bob_move_threshold,
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
                ui.collapsing("Knife", |ui| {
                    let k = &mut *knife_view;
                    ui.label("models/weapons/knife.glb — position and scale only (see WeaponSlot::Secondary)");
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
                ui.collapsing("Throwing arms", |ui| {
                    let a = &mut *arms_view;
                    ui.label(
                        "models/characters/arms_throwing.glb — where the arms sit while the throwing-knife key \
                         is held. Angles are on top of the fixed half-turn that points the \
                         model down the view.",
                    );
                    ui.add(egui::Slider::new(&mut a.translation.x, -2.0f32..=2.0).text("x  (right +)"));
                    ui.add(egui::Slider::new(&mut a.translation.y, -2.0f32..=2.0).text("y  (up +)"));
                    ui.add(
                        egui::Slider::new(&mut a.translation.z, -3.0f32..=1.0).text("z  (forward -)"),
                    );
                    ui.add(egui::Slider::new(&mut a.yaw, -180.0f32..=180.0).text("yaw (°)"));
                    ui.add(egui::Slider::new(&mut a.pitch, -180.0f32..=180.0).text("pitch (°)"));
                    ui.add(egui::Slider::new(&mut a.roll, -180.0f32..=180.0).text("roll (°)"));
                    ui.add(
                        egui::Slider::new(&mut a.scale, 0.001f32..=0.05)
                            .text("scale")
                            .logarithmic(true),
                    );

                    if ui.button("Copy arms pose to console").clicked() {
                        info!(
                            "arms: translation: Vec3::new({:.4}, {:.4}, {:.4}), yaw: {:.1}, \
                             pitch: {:.1}, roll: {:.1}, scale: {:.5}",
                            a.translation.x, a.translation.y, a.translation.z, a.yaw, a.pitch, a.roll,
                            a.scale,
                        );
                    }
                    if ui.button("Reset arms pose to default").clicked() {
                        // Pose only — the timing values live in the "Throwing knife"
                        // section below.
                        let d = ThrowArmsSettings::default();
                        a.translation = d.translation;
                        a.yaw = d.yaw;
                        a.pitch = d.pitch;
                        a.roll = d.roll;
                        a.scale = d.scale;
                    }
                });

            ui.separator();
                ui.collapsing("Drinking arms", |ui| {
                    let d = &mut *drink;
                    ui.label(
                        "models/characters/arms_drinking.glb — the perk-drinking arms (played once on buying \
                         a perk), looping the trimmed drink clip while shown here. Angles are on top of the fixed half-turn that \
                         points the model down the view.",
                    );
                    let show_label = if d.show { "Hide drinking arms" } else { "Show drinking arms" };
                    if ui.button(show_label).clicked() {
                        d.show = !d.show;
                    }
                    ui.add(egui::Slider::new(&mut d.translation.x, -2.0f32..=2.0).text("x  (right +)"));
                    ui.add(egui::Slider::new(&mut d.translation.y, -2.0f32..=2.0).text("y  (up +)"));
                    ui.add(
                        egui::Slider::new(&mut d.translation.z, -3.0f32..=1.0).text("z  (forward -)"),
                    );
                    ui.add(egui::Slider::new(&mut d.yaw, -180.0f32..=180.0).text("yaw (°)"));
                    ui.add(egui::Slider::new(&mut d.pitch, -180.0f32..=180.0).text("pitch (°)"));
                    ui.add(egui::Slider::new(&mut d.roll, -180.0f32..=180.0).text("roll (°)"));
                    ui.add(
                        egui::Slider::new(&mut d.scale, 0.01f32..=10.0)
                            .text("scale")
                            .logarithmic(true),
                    );
                    ui.add(egui::Slider::new(&mut d.speed, 0.0f32..=10.0).text("animation speed"));
                    ui.add(egui::Slider::new(&mut d.glow, 0.0f32..=5.0).text("bottle glow"));

                    if ui.button("Copy drinking arms pose to console").clicked() {
                        info!(
                            "drinking arms: translation: Vec3::new({:.4}, {:.4}, {:.4}), yaw: {:.1}, \
                             pitch: {:.1}, roll: {:.1}, scale: {:.5}, speed: {:.2}, glow: {:.2}",
                            d.translation.x, d.translation.y, d.translation.z, d.yaw, d.pitch, d.roll,
                            d.scale, d.speed, d.glow,
                        );
                    }
                    if ui.button("Reset drinking arms pose to default").clicked() {
                        // Pose only — leaves the show toggle as it is.
                        *d = crate::DrinkArmsSettings {
                            show: d.show,
                            ..default()
                        };
                    }
                });

            ui.separator();
                ui.collapsing("Throwing knife model", |ui| {
                    let k = &mut *knife_model;
                    ui.label(
                        "models/weapons/throwing_knife.glb — the knife held in the throwing arms' hand, \
                         positioned in the ARMS' local space (a child of the arms), so it stays in \
                         the hand however the arms move. One unit = the arms' scale in metres \
                         (0.01 by default, so units are ~cm). Visible from the key press until the \
                         throw animation starts. Hold the throwing-knife key to see it.",
                    );
                    ui.checkbox(
                        &mut arms_view.debug_hold_key,
                        "Hold lethal key (as if held — untick to throw)",
                    );
                    ui.add(egui::Slider::new(&mut k.translation.x, -100.0f32..=100.0).text("x"));
                    ui.add(egui::Slider::new(&mut k.translation.y, -100.0f32..=100.0).text("y"));
                    ui.add(egui::Slider::new(&mut k.translation.z, -100.0f32..=100.0).text("z"));
                    ui.add(egui::Slider::new(&mut k.yaw, -180.0f32..=180.0).text("yaw (°)"));
                    ui.add(egui::Slider::new(&mut k.pitch, -180.0f32..=180.0).text("pitch (°)"));
                    ui.add(egui::Slider::new(&mut k.roll, -180.0f32..=180.0).text("roll (°)"));
                    ui.add(
                        egui::Slider::new(&mut k.scale, 0.1f32..=30.0)
                            .text("scale")
                            .logarithmic(true),
                    );
                    if ui.button("Copy knife model pose to console").clicked() {
                        info!(
                            "throwing knife model: translation: Vec3::new({:.3}, {:.3}, {:.3}), \
                             yaw: {:.1}, pitch: {:.1}, roll: {:.1}, scale: {:.3}",
                            k.translation.x, k.translation.y, k.translation.z, k.yaw, k.pitch, k.roll,
                            k.scale,
                        );
                    }
                    if ui.button("Reset knife model pose to default").clicked() {
                        *k = ThrowKnifeModelSettings::default();
                    }
                });

            ui.separator();
                ui.collapsing("Throwing knife", |ui| {
                    let a = &mut *arms_view;
                    ui.label(
                        "Hold the throwing-knife key: the equipped weapon plays its Hide (sped up), \
                         then the throwing arms slide up from below. On release the throw plays; the \
                         server-simulated knife is launched part-way through it. Afterwards the arms \
                         slide back down, and only then does the weapon start to show again.",
                    );
                    ui.add(
                        egui::Slider::new(&mut a.weapon_hide_speed, 0.5f32..=12.0)
                            .text("weapon hide speed (x normal)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut a.throw_release_secs, 0.0f32..=0.66)
                            .text("knife leaves the hand (s into the throw)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut a.slide_speed, 0.5f32..=20.0)
                            .text("arms show / hide speed (slides/s)")
                            .logarithmic(true),
                    );
                    ui.add(
                        egui::Slider::new(&mut a.hide_drop, 0.0f32..=2.0)
                            .text("arms hidden drop (m below)"),
                    );
                    if ui.button("Reset throwing knife timing to default").clicked() {
                        let d = ThrowArmsSettings::default();
                        a.weapon_hide_speed = d.weapon_hide_speed;
                        a.slide_speed = d.slide_speed;
                        a.hide_drop = d.hide_drop;
                        a.throw_release_secs = d.throw_release_secs;
                    }
                });
            });

            ui.separator();
            ui.collapsing("Movement", |ui| {
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
                ui.collapsing("Ledge jump", |ui| {
                    let l = &mut *ledge_jump;
                    ui.label(
                        "Grace period after running off an edge in which jump still works \
                         (Call of Duty style), instead of just dropping.",
                    );
                    ui.checkbox(&mut l.enabled, "enabled");
                    ui.add(
                        egui::Slider::new(&mut l.grace_secs, 0.0f32..=0.5)
                            .text("grace time (s)")
                            .max_decimals(3),
                    );
                    let m = &*movement;
                    ui.label(format!(
                        "≈ {:.2} m past the edge walking, {:.2} m sprinting",
                        l.grace_secs * m.walk_speed,
                        l.grace_secs * m.sprint_speed,
                    ));
                    if ui.button("Reset ledge jump").clicked() {
                        *l = LedgeJumpSettings::default();
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
            });

            ui.separator();
            ui.collapsing("Shot effects", |ui| {
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

                    ui.label("throwing knife trail");
                    ui.add(
                        egui::Slider::new(&mut tr.knife_trail_alpha, 0.0f32..=1.0)
                            .text("start opacity"),
                    );
                    ui.add(
                        egui::Slider::new(&mut tr.knife_trail_radius, 0.001f32..=0.1)
                            .text("radius (m)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut tr.knife_trail_secs, 0.1f32..=6.0)
                            .text("fade time (s)"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("knife trail color");
                        ui.color_edit_button_rgb(&mut tr.knife_trail_color);
                    });

                    if ui.button("Reset tracer").clicked() {
                        *tr = TracerSettings::default();
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
                ui.collapsing("Bullet impacts", |ui| {
                    ui.label(
                        "textures/vfx/bullet_hole.png stuck flat on the surface a shot hit — for every \
                         player in the lobby, removed after 1 minute. Scale applies to the holes \
                         already in the world too.",
                    );
                    ui.add(
                        egui::Slider::new(&mut bullet_holes.size, 0.05f32..=3.0)
                            .text("size (m)")
                            .logarithmic(true),
                    );
                    if ui.button("Reset bullet impact size").clicked() {
                        *bullet_holes = BulletHoleSettings::default();
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
                ui.collapsing("Aim recoil", |ui| {
                    let r = &mut *aim_recoil;
                    ui.label(
                        "Each shot really throws the aim off — up, and a random way left / right — \
                         and it stays off (unlike the camera shake). Per weapon.",
                    );
                    ui.checkbox(&mut r.enabled, "Enabled");
                    ui.add(egui::Slider::new(&mut r.kick_speed, 1.0f32..=100.0).text("kick speed (1/s)"));
                    for (name, w) in [("Sniper", &mut r.sniper), ("AK-74", &mut r.ak)] {
                        ui.separator();
                        ui.label(name);
                        ui.add(egui::Slider::new(&mut w.vertical_deg, 0.0f32..=8.0).text("vertical per shot (°)"));
                        ui.add(egui::Slider::new(&mut w.horizontal_deg, 0.0f32..=4.0).text("horizontal per shot, max (°)"));
                        ui.add(egui::Slider::new(&mut w.horizontal_bias, -1.0f32..=1.0).text("horizontal bias (left − / right +)"));
                        ui.add(egui::Slider::new(&mut w.ads_mult, 0.0f32..=1.5).text("left fully aimed (×)"));
                    }
                    if ui.button("Copy aim recoil to console").clicked() {
                        let f = |w: &crate::WeaponRecoil| {
                            format!(
                                "vertical_deg: {:.2}, horizontal_deg: {:.2}, horizontal_bias: {:.2}, ads_mult: {:.2}",
                                w.vertical_deg, w.horizontal_deg, w.horizontal_bias, w.ads_mult
                            )
                        };
                        info!(
                            "aim recoil: kick_speed: {:.1}, sniper {{ {} }}, ak {{ {} }}",
                            r.kick_speed,
                            f(&r.sniper),
                            f(&r.ak)
                        );
                    }
                    if ui.button("Reset aim recoil").clicked() {
                        *r = crate::AimRecoilSettings::default();
                    }
                });

            ui.separator();
                ui.collapsing("Sniper glint", |ui| {
                    let g = &mut *sniper_glint;
                    ui.label(
                        "vfx/sniper_glint.png sprite off another player's or bot's scope while \
                         they're ADS — gives away a camping sniper, Call of Duty-style. Always \
                         faces you, wherever you're standing.",
                    );
                    ui.label("offset from their eye position, in their own facing:");
                    ui.add(egui::Slider::new(&mut g.offset.x, -1.0f32..=1.0).text("x (right +)"));
                    ui.add(egui::Slider::new(&mut g.offset.y, -2.0f32..=3.0).text("y (up +)"));
                    ui.add(egui::Slider::new(&mut g.offset.z, -1.0f32..=1.0).text("z (forward +)"));
                    ui.add(egui::Slider::new(&mut g.scale, 0.01f32..=2.0).text("sprite size (m)"));
                    ui.add(
                        egui::Slider::new(&mut g.ads_threshold, 0.0f32..=0.99)
                            .text("ads amount to start fading in at"),
                    );

                    if ui.button("Copy sniper glint to console").clicked() {
                        info!(
                            "sniper glint: offset: Vec3::new({:.2}, {:.2}, {:.2}), scale: {:.2}, \
                             ads_threshold: {:.2}",
                            g.offset.x, g.offset.y, g.offset.z, g.scale, g.ads_threshold,
                        );
                    }
                    if ui.button("Reset sniper glint").clicked() {
                        *g = SniperGlintSettings::default();
                    }
                });
                ui.collapsing("Remote tracer start", |ui| {
                    let m = &mut *remote_muzzle;
                    ui.label(
                        "Where other players' / bots' shot tracers leave their gun: an offset \
                         from the sniper glint's spot (their scope), along where they're aiming. \
                         Same for every weapon (one soldier model).",
                    );
                    ui.checkbox(&mut m.show_marker, "Show marker on every remote gun");
                    ui.add(egui::Slider::new(&mut m.offset.x, -1.0f32..=1.0).text("x (right +)"));
                    ui.add(egui::Slider::new(&mut m.offset.y, -1.0f32..=1.0).text("y (up +)"));
                    ui.add(egui::Slider::new(&mut m.offset.z, -1.0f32..=2.0).text("z (forward +)"));
                    if ui.button("Copy remote tracer start to console").clicked() {
                        info!(
                            "remote tracer start: offset: Vec3::new({:.3}, {:.3}, {:.3})",
                            m.offset.x, m.offset.y, m.offset.z,
                        );
                    }
                    if ui.button("Reset remote tracer start").clicked() {
                        *m = crate::RemoteMuzzleSettings::default();
                    }
                });
            });

            ui.separator();
            ui.collapsing("Players & HUD", |ui| {
                ui.collapsing("Remote players", |ui| {
                    let ra = &mut *remote_avatar;
                    ui.label("models/characters/soldier.glb");
                    ui.add(
                        egui::Slider::new(&mut ra.scale, 0.01f32..=100.0)
                            .text("scale")
                            .logarithmic(true),
                    );
                    if ui.button("Reset remote player scale").clicked() {
                        *ra = RemoteAvatarSettings::default();
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("bot tint (× body texture)");
                        ui.color_edit_button_rgb(&mut bot_look.tint);
                    });
                    if ui.button("Copy bot tint to console").clicked() {
                        let [r, g, b] = bot_look.tint;
                        info!("bot tint: [{r:.3}, {g:.3}, {b:.3}]");
                    }
                    if ui.button("Reset bot tint").clicked() {
                        *bot_look = BotLookSettings::default();
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
                ui.collapsing("Name tags", |ui| {
                    let nt = &mut *name_tags;
                    ui.label("Diamond + name over other players' heads.");
                    ui.add(
                        egui::Slider::new(&mut nt.height, 0.0f32..=2.0)
                            .text("height above eye (m)"),
                    );
                    ui.add(egui::Slider::new(&mut nt.scale, 0.25f32..=4.0).text("scale (×)"));
                    ui.horizontal(|ui| {
                        ui.label("enemy color (Free For All)");
                        ui.color_edit_button_rgb(&mut nt.enemy_color);
                    });
                    ui.horizontal(|ui| {
                        ui.label("lobby member color (Freestyle)");
                        ui.color_edit_button_rgb(&mut nt.friendly_color);
                    });
                    if ui.button("Copy name tags to console").clicked() {
                        let [er, eg, eb] = nt.enemy_color;
                        let [fr, fg, fb] = nt.friendly_color;
                        info!(
                            "name tags: height: {:.2}, scale: {:.2}, enemy_color: [{er:.2}, {eg:.2}, {eb:.2}], \
                             friendly_color: [{fr:.2}, {fg:.2}, {fb:.2}]",
                            nt.height, nt.scale,
                        );
                    }
                    if ui.button("Reset name tags").clicked() {
                        *nt = crate::hud::NameTagSettings::default();
                    }
                });

            ui.separator();
                ui.collapsing("Zombie health bars", |ui| {
                    let hb = &mut *health_bars;
                    ui.label("Red bar over a zombie you've hurt (only your own hits).");
                    ui.add(egui::Slider::new(&mut hb.height, 0.0f32..=2.0).text("height above eye (m)"));
                    ui.add(egui::Slider::new(&mut hb.width, 10.0f32..=300.0).text("width (px)"));
                    ui.add(egui::Slider::new(&mut hb.thickness, 1.0f32..=30.0).text("thickness (px)"));
                    ui.add(egui::Slider::new(&mut hb.show_secs, 0.0f32..=10.0).text("shown after last hit (s)"));
                    ui.add(egui::Slider::new(&mut hb.fade_secs, 0.0f32..=5.0).text("then fades over (s)"));
                    ui.add(egui::Slider::new(&mut hb.back_alpha, 0.0f32..=1.0).text("black back opacity"));
                    if ui.button("Copy health bars to console").clicked() {
                        info!(
                            "health bars: height: {:.2}, width: {:.1}, thickness: {:.1}, show_secs: {:.2}, \
                             fade_secs: {:.2}, back_alpha: {:.2}",
                            hb.height, hb.width, hb.thickness, hb.show_secs, hb.fade_secs, hb.back_alpha,
                        );
                    }
                    if ui.button("Reset health bars").clicked() {
                        *hb = crate::hud::HealthBarSettings::default();
                    }
                });

            ui.separator();
                ui.collapsing("Health", |ui| {
                    ui.label(
                        "Health is server-side: shots and falls both take it off, it holds for 3 s \
                         after damage then recovers, and the client just shows it (red tint + blood, \
                         heartbeat). The fall-damage distances and recovery rate are constants in \
                         shared/src/health.rs.",
                    );
                    ui.label(format!(
                        "health (from the server): {:.0} / {:.0}",
                        local_health.health,
                        shared::health::FULL_HEALTH,
                    ));
                    ui.label(format!(
                        "fall damage: none under {:.0} m, lethal at {:.0} m",
                        shared::health::FALL_MIN_DISTANCE,
                        shared::health::FALL_MAX_DISTANCE,
                    ));
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
            });

            ui.separator();
            ui.collapsing("Audio", |ui| {
                ui.collapsing("Sound volumes", |ui| {
                    let v = &mut *sound_vol;
                    ui.label("per-sound multiplier (1 = built-in level)");
                    for (label, slot) in [
                        ("shot", &mut v.shot),
                        ("rechamber", &mut v.rechamber),
                        ("reload", &mut v.reload),
                        ("AK-74: shot", &mut v.ak_shot),
                        ("AK-74: reload", &mut v.ak_reload),
                        ("AK-74: fast reload", &mut v.ak_reload_fast),
                        ("Ray Gun: shot", &mut v.raygun_shot),
                        ("Ray Gun: reload", &mut v.raygun_reload),
                        ("Ray Gun: equip", &mut v.raygun_equip),
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
                        ("throwing knife: throw", &mut v.knife_throw),
                        ("throwing knife: hit enemy", &mut v.knife_hit),
                        ("throwing knife: in air", &mut v.knife_in_air),
                        ("knife: equip", &mut v.knife_equip),
                        ("molotov: light (held)", &mut v.molotov_light),
                        ("molotov: burst", &mut v.molotov_burst),
                        ("monkey bomb: prime", &mut v.monkey_bomb_prime),
                        ("monkey bomb: throw", &mut v.monkey_bomb_throw),
                        ("monkey bomb: land", &mut v.monkey_bomb_land),
                        ("monkey bomb: song", &mut v.monkey_bomb_song),
                        ("monkey bomb: bye bye", &mut v.monkey_bomb_explode_vox),
                        ("frag: pin pull", &mut v.frag_pin_pull),
                        ("frag: explosion", &mut v.frag_explosion),
                        ("flash bang: detonate", &mut v.flash_bang_detonate),
                        ("throwing knife: pick up", &mut v.pick_up_equipment),
                        ("sniper: equip", &mut v.sniper_equip),
                        ("heartbeat (at zero health)", &mut v.heartbeat),
                        ("hit marker", &mut v.hit_marker),
                        ("zombies: buy perk", &mut v.perk_buy),
                        ("zombies: buy ammo", &mut v.buy_ammo),
                        ("zombies: prone bonus ching", &mut v.money_ching),
                        ("zombies: power-up grab", &mut v.power_up_grab),
                        ("zombies: power-up loop", &mut v.power_up_loop),
                        ("zombies: power-up announcer", &mut v.power_up_announcer),
                        ("zombies: perk jingle", &mut v.perk_jingle),
                        ("zombies: perk quote", &mut v.perk_quote),
                        ("zombies: power on", &mut v.power_on),
                        ("zombies: mystery box open", &mut v.mystery_box_open),
                        ("zombies: mystery box close", &mut v.mystery_box_close),
                        ("zombies: mystery box spin", &mut v.mystery_box_spin),
                        ("zombies: armor destroyed", &mut v.armor_destroy),
                        ("zombies: aether shroud activate", &mut v.aether_activate),
                        ("zombies: aether shroud loop", &mut v.aether_active),
                        ("zombies: aether shroud deactivate", &mut v.aether_deactivate),
                        ("zombies: boss spawn", &mut v.boss_spawn),
                        ("zombies: boss growl", &mut v.boss_growl),
                        ("zombies: boss smash", &mut v.boss_attack),
                        ("zombies: boss death", &mut v.boss_death),
                        ("zombies: boss blast (flying)", &mut v.boss_blast),
                        ("zombies: boss blast hit", &mut v.boss_blast_hit),
                        ("zombies: round start", &mut v.round_start),
                        ("zombies: dog round start", &mut v.dog_round_start),
                        ("zombies: dog round end", &mut v.dog_round_end),
                        ("zombies: dog lightning (pre-spawn)", &mut v.dog_pre_spawn),
                        ("zombies: dog spawn", &mut v.dog_spawn),
                        ("zombies: dog bark", &mut v.dog_bark),
                        ("zombies: dog explosion", &mut v.dog_explosion),
                        ("zombies: player down", &mut v.player_down),
                        ("zombies: revived", &mut v.revived),
                        ("zombies: bomb shot explosion", &mut v.bomb_shot_explosion),
                        ("zombies: zombie moans", &mut v.zombie_moan),
                        ("zombies: zombie spawn", &mut v.zombie_spawn),
                        ("zombies: zombie death", &mut v.zombie_death),
                        ("zombies: zombie swipe hit", &mut v.zombie_attack),
                        ("zombies: final zombie", &mut v.final_zombie),
                        ("zombies: ambience", &mut v.zombies_ambient),
                        ("zombies: game over music", &mut v.zombies_game_over),
                    ] {
                        ui.add(egui::Slider::new(slot, 0.0f32..=10.0).text(label));
                    }
                    // One slider per clip in each folder-backed set (one of a set's
                    // clips plays at random per event).
                    let ks = &mut *knife_sounds;
                    for (title, set) in [
                        ("throwing knife: surface impacts", &mut ks.impact),
                        ("knife: stabs (hit a bot / player)", &mut ks.stab),
                        ("knife: swings (miss)", &mut ks.swing),
                    ] {
                        ui.collapsing(title, |ui| {
                            if set.clips.is_empty() {
                                ui.label("(no clips found in this folder)");
                            }
                            for clip in &mut set.clips {
                                ui.add(egui::Slider::new(&mut clip.volume, 0.0f32..=10.0).text(&clip.name));
                            }
                        });
                    }
                    if ui.button("Reset sound volumes").clicked() {
                        *v = SoundVolumes::default();
                        for clip in &mut ks.impact.clips {
                            clip.volume = 0.3;
                        }
                        for clip in ks.stab.clips.iter_mut().chain(&mut ks.swing.clips) {
                            clip.volume = 1.0;
                        }
                    }
                });

            ui.separator();
                ui.collapsing("Remote sounds", |ui| {
                    ui.label("Other players' (and bots') footsteps/jump/slide/reload/rechamber/shot/dive/throw.");
                    let rs = &mut *remote_sound;
                    ui.add(egui::Slider::new(&mut rs.volume, 0.0f32..=3.0).text("volume (×)"));
                    ui.add(
                        egui::Slider::new(&mut rs.max_distance, 5.0f32..=300.0)
                            .text("max distance (m)"),
                    );
                    ui.label("Per sound (× on top of volume) — players and bots alike:");
                    for bit in crate::killcam::ALL_SND_BITS {
                        if let Some((label, v)) = rs.field_mut(bit) {
                            ui.add(egui::Slider::new(v, 0.0f32..=4.0).text(label));
                        }
                    }
                    if ui.button("Reset remote sounds").clicked() {
                        *rs = RemoteSoundSettings::default();
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
            });

            ui.separator();
            ui.collapsing("Zombie model & animations", |ui| {
                let z = &mut *zombie_look;
                ui.label(
                    "The walk / run clips play at the zombie's real speed (m/s) × the rate \
                     below — raise it if the feet slide backward, lower it if they slide \
                     forward. Zombies walk (arms up once close to who they're after) until \
                     they're 3 m/s or faster — about round 10 — then run.",
                );
                match zombie_readout.nearest {
                    Some((speed, rate, state)) => ui.label(format!(
                        "Nearest zombie: {state}, {speed:.2} m/s, playing at {rate:.2}×"
                    )),
                    None => ui.label("Nearest zombie: none"),
                };
                ui.add(egui::Slider::new(&mut z.walk_per_mps, 0.05f32..=4.0).text("walk, arms down (× per m/s)"));
                ui.add(
                    egui::Slider::new(&mut z.walk_arms_up_per_mps, 0.05f32..=4.0)
                        .text("walk, arms up (× per m/s)"),
                );
                ui.add(egui::Slider::new(&mut z.run_per_mps, 0.02f32..=2.0).text("run (× per m/s)"));
                ui.add(egui::Slider::new(&mut z.idle_speed, 0.1f32..=3.0).text("idle speed (×)"));
                ui.add(egui::Slider::new(&mut z.attack_speed, 0.1f32..=3.0).text("attack speed (×)"));
                ui.add(egui::Slider::new(&mut z.death_speed, 0.1f32..=3.0).text("death speed (×)"));
                ui.add(egui::Slider::new(&mut z.blend_secs, 0.0f32..=1.0).text("blend between clips (s)"));
                ui.add(
                    egui::Slider::new(&mut z.min_move_speed, 0.0f32..=1.0)
                        .text("idle below (m/s)"),
                );
                ui.add(egui::Slider::new(&mut z.scale, 0.1f32..=3.0).text("model scale"));
                ui.add(egui::Slider::new(&mut z.yaw_offset_deg, -180.0f32..=180.0).text("model turn (deg)"));
                if ui.button("Copy zombie settings to console").clicked() {
                    info!(
                        "zombie: walk_per_mps: {:.3}, walk_arms_up_per_mps: {:.3}, run_per_mps: {:.3}, \
                         idle_speed: {:.2}, attack_speed: {:.2}, death_speed: {:.2}, blend_secs: {:.2}, \
                         min_move_speed: {:.2}, scale: {:.3}, yaw_offset_deg: {:.1}",
                        z.walk_per_mps, z.walk_arms_up_per_mps, z.run_per_mps, z.idle_speed,
                        z.attack_speed, z.death_speed, z.blend_secs, z.min_move_speed, z.scale,
                        z.yaw_offset_deg,
                    );
                }
                if ui.button("Reset zombie settings").clicked() {
                    *z = default();
                }
            });

            ui.separator();
            ui.collapsing("Zombie sounds", |ui| {
                let s = &mut *zombie_voice;
                ui.label("Loudness per sound: Sound volumes → \"zombies: …\".");
                ui.add(egui::Slider::new(&mut s.max_distance, 5.0f32..=150.0).text("heard up to (m)"));
                ui.add(egui::Slider::new(&mut s.moan_min_secs, 0.5f32..=30.0).text("moan every, min (s)"));
                ui.add(egui::Slider::new(&mut s.moan_max_secs, 0.5f32..=30.0).text("moan every, max (s)"));
                ui.add(egui::Slider::new(&mut s.max_moans_at_once, 1u32..=10).text("moans at once, max"));
                ui.add(egui::Slider::new(&mut s.moan_min_gap_secs, 0.0f32..=5.0).text("gap between moans (s)"));
                ui.add(egui::Slider::new(&mut s.low_moan_below, 0.0f32..=10.0).text("low moans below (m/s)"));
                ui.add(egui::Slider::new(&mut s.high_moan_above, 0.0f32..=12.0).text("high moans above (m/s)"));
                ui.add(egui::Slider::new(&mut s.final_min_secs, 0.5f32..=30.0).text("final zombie every, min (s)"));
                ui.add(egui::Slider::new(&mut s.final_max_secs, 0.5f32..=30.0).text("final zombie every, max (s)"));
                if ui.button("Reset zombie sounds").clicked() {
                    *s = default();
                }
            });

            ui.separator();
            ui.collapsing("Dogs (Zombies)", |ui| {
                crate::dogs::dogs_section(ui, &mut dog_settings, &mut dog_preview, &dog_readout);
            });

            ui.separator();
            ui.collapsing("Zombies round counter", |ui| {
                let r = &mut *round_anim;
                ui.add(egui::Slider::new(&mut r.size, 20.0f32..=160.0).text("size (px)"));
                ui.add(egui::Slider::new(&mut r.peak_scale, 1.0f32..=5.0).text("swell (× size)"));
                ui.add(egui::Slider::new(&mut r.slide_in_secs, 0.0f32..=3.0).text("slide to middle (s)"));
                ui.add(egui::Slider::new(&mut r.grow_secs, 0.0f32..=3.0).text("swell up (s)"));
                ui.add(egui::Slider::new(&mut r.hold_secs, 0.0f32..=5.0).text("hold (s)"));
                ui.add(egui::Slider::new(&mut r.return_secs, 0.0f32..=3.0).text("shrink + slide home (s)"));
                if ui.button("Replay round start (with sound)").clicked() {
                    r.replay = true;
                }
                if ui.button("Copy round counter settings to console").clicked() {
                    info!(
                        "round counter: size: {:.1}, peak_scale: {:.2}, slide_in_secs: {:.2}, grow_secs: {:.2}, \
                         hold_secs: {:.2}, return_secs: {:.2}",
                        r.size, r.peak_scale, r.slide_in_secs, r.grow_secs, r.hold_secs, r.return_secs,
                    );
                }
                if ui.button("Reset round counter").clicked() {
                    *r = default();
                }
            });

            ui.separator();
            ui.collapsing("Map lights (Break Point Night)", |ui| {
                let ml = &mut *map_lights;
                ui.label(
                    "In Zombies they're off until someone turns the power on, then fade in; \
                     in other modes they're just on.",
                );
                ui.checkbox(&mut ml.force_on, "force power on (untick / tick to replay the fade)");
                ui.add(egui::Slider::new(&mut ml.fade_secs, 0.0f32..=15.0).text("fade in (s)"));
                for (i, l) in ml.lights.iter_mut().enumerate() {
                    ui.separator();
                    ui.label(format!("Light {}", i + 1));
                    ui.checkbox(&mut l.enabled, "on");
                    ui.add(egui::Slider::new(&mut l.pos.x, -80.0f32..=80.0).text("x (m)"));
                    ui.add(egui::Slider::new(&mut l.pos.y, -10.0f32..=60.0).text("y (m)"));
                    ui.add(egui::Slider::new(&mut l.pos.z, -80.0f32..=80.0).text("z (m)"));
                    ui.horizontal(|ui| {
                        ui.label("colour");
                        ui.color_edit_button_rgb(&mut l.color);
                    });
                    ui.add(
                        egui::Slider::new(&mut l.intensity, 0.0f32..=100_000_000.0)
                            .logarithmic(true)
                            .text("intensity (lm)"),
                    );
                    ui.add(egui::Slider::new(&mut l.range, 1.0f32..=200.0).text("range (m)"));
                    ui.add(egui::Slider::new(&mut l.radius, 0.0f32..=5.0).text("radius (m)"));
                    ui.checkbox(&mut l.shadows, "shadows");
                }
                ui.separator();
                if ui.button("Copy map lights to console").clicked() {
                    info!("map lights: fade_secs: {:.2}", ml.fade_secs);
                    for (i, l) in ml.lights.iter().enumerate() {
                        info!(
                            "map light {}: enabled: {}, pos: ({:.2}, {:.2}, {:.2}), color: ({:.3}, {:.3}, {:.3}), \
                             intensity: {:.0}, range: {:.2}, radius: {:.2}, shadows: {}",
                            i + 1,
                            l.enabled,
                            l.pos.x,
                            l.pos.y,
                            l.pos.z,
                            l.color[0],
                            l.color[1],
                            l.color[2],
                            l.intensity,
                            l.range,
                            l.radius,
                            l.shadows,
                        );
                    }
                }
                if ui.button("Reset map lights").clicked() {
                    *ml = crate::power::MapLightSettings {
                        force_on: ml.force_on,
                        ..default()
                    };
                }
            });

            ui.separator();
            ui.collapsing("Machine hum (Zombies)", |ui| {
                let h = &mut *hum;
                ui.label(
                    "The buzz from every perk machine and the Pack-a-Punch once the power's on: \
                     full volume within the inner radius, fading to silence at the max distance.",
                );
                ui.add(egui::Slider::new(&mut h.volume, 0.0f32..=4.0).text("volume"));
                ui.add(egui::Slider::new(&mut h.full_distance, 0.0f32..=20.0).text("full volume within (m)"));
                ui.add(egui::Slider::new(&mut h.max_distance, 1.0f32..=60.0).text("silent past (m)"));
                ui.add(
                    egui::Slider::new(&mut h.falloff, 0.5f32..=4.0)
                        .text("falloff curve (1 = linear, higher = drops off sooner)"),
                );
                ui.add(egui::Slider::new(&mut h.fade_in_secs, 0.0f32..=10.0).text("swell in when powered (s)"));
                if ui.button("Copy machine hum settings to console").clicked() {
                    info!(
                        "machine hum: volume: {:.2}, full_distance: {:.2}, max_distance: {:.2}, falloff: {:.2}, \
                         fade_in_secs: {:.2}",
                        h.volume, h.full_distance, h.max_distance, h.falloff, h.fade_in_secs,
                    );
                }
                if ui.button("Reset machine hum").clicked() {
                    *h = default();
                }
            });

            ui.separator();
            ui.collapsing("Power lever (Zombies)", |ui| {
                let pl = &mut *power_lever;
                ui.label(
                    "The lever at the power switch, on this client only — the switch itself \
                     (where you press to buy it) is placed in the level editor.",
                );
                ui.label("Position (m, from the switch spot, in its frame)");
                ui.add(egui::Slider::new(&mut pl.offset.x, -80.0f32..=80.0).text("x (m)"));
                ui.add(egui::Slider::new(&mut pl.offset.y, -10.0f32..=60.0).text("y (m)"));
                ui.add(egui::Slider::new(&mut pl.offset.z, -80.0f32..=80.0).text("z (m)"));
                ui.label("Rotation (deg)");
                ui.add(egui::Slider::new(&mut pl.rotation_deg.y, -180.0f32..=180.0).text("turn (y)"));
                ui.add(egui::Slider::new(&mut pl.rotation_deg.x, -180.0f32..=180.0).text("pitch (x)"));
                ui.add(egui::Slider::new(&mut pl.rotation_deg.z, -180.0f32..=180.0).text("roll (z)"));
                ui.add(
                    egui::Slider::new(&mut pl.scale, 0.0005f32..=0.05)
                        .logarithmic(true)
                        .text("scale"),
                );
                ui.add(egui::Slider::new(&mut pl.anim_speed, 0.1f32..=4.0).text("animation speed"));
                if ui.button("Throw the lever again (and its sound)").clicked() {
                    pl.replay = true;
                }
                if ui.button("Copy power lever settings to console").clicked() {
                    info!(
                        "power lever: offset: ({:.2}, {:.2}, {:.2}), rotation_deg: ({:.1}, {:.1}, {:.1}), \
                         scale: {:.5}, anim_speed: {:.2}",
                        pl.offset.x,
                        pl.offset.y,
                        pl.offset.z,
                        pl.rotation_deg.x,
                        pl.rotation_deg.y,
                        pl.rotation_deg.z,
                        pl.scale,
                        pl.anim_speed,
                    );
                }
                if ui.button("Reset power lever").clicked() {
                    *pl = default();
                }
            });

            ui.separator();
            ui.collapsing("Power-ups (Zombies)", |ui| {
                // The test toggle lives on the server's lobby, like "Bots
                // don't attack".
                let me = local_id.iter().next().map(|l| l.0);
                match me.and_then(|me| lobbies.iter().find(|l| l.has(me)).map(|l| (l, l.leader == me))) {
                    Some((lobby, is_leader)) => {
                        let mut on = lobby.power_up_test;
                        if ui
                            .add_enabled(is_leader, egui::Checkbox::new(&mut on, "Every zombie kill drops a power-up (test)"))
                            .changed()
                        {
                            if let Ok(mut tx) = power_up_test_tx.single_mut() {
                                tx.trigger::<shared::LobbyChannel>(shared::SetPowerUpTest { on });
                            }
                        }
                        // Set one off now: dropped at the leader's feet, so
                        // they pick it up at once (in a running Zombies game).
                        ui.label("Set one off now (drops at your feet):");
                        ui.add_enabled_ui(is_leader, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for kind in shared::power_ups::PowerUp::ALL {
                                    if ui.button(kind.label()).clicked() {
                                        if let Ok(mut tx) = drop_power_up_tx.single_mut() {
                                            tx.trigger::<shared::LobbyChannel>(shared::DropPowerUp { kind });
                                        }
                                    }
                                }
                            });
                        });
                        if !is_leader {
                            ui.label("Only the party leader can change this.");
                        }
                    }
                    None => {
                        ui.label("Not in a lobby.");
                    }
                }
                ui.separator();
                let p = &mut *power_ups;
                ui.label("Motion");
                ui.add(egui::Slider::new(&mut p.height, 0.0f32..=3.0).text("float height (m)"));
                ui.add(egui::Slider::new(&mut p.bob_height, 0.0f32..=0.6).text("bob height (m)"));
                ui.add(egui::Slider::new(&mut p.bob_speed, 0.0f32..=3.0).text("bob speed (cycles/s)"));
                ui.add(egui::Slider::new(&mut p.spin_deg, -360.0f32..=360.0).text("spin (deg/s)"));
                ui.add(egui::Slider::new(&mut p.blink_hz, 0.5f32..=12.0).text("blink when expiring (per s)"));
                ui.label("Model scale");
                for kind in shared::power_ups::PowerUp::ALL {
                    ui.add(
                        egui::Slider::new(p.scale_mut(kind), 0.001f32..=10.0)
                            .logarithmic(true)
                            .text(kind.label()),
                    );
                }
                ui.label("Gold");
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut p.gold_color);
                    ui.label("colour");
                });
                ui.add(egui::Slider::new(&mut p.gold_metallic, 0.0f32..=1.0).text("metallic"));
                ui.add(egui::Slider::new(&mut p.gold_roughness, 0.0f32..=1.0).text("roughness"));
                ui.add(egui::Slider::new(&mut p.gold_emissive, 0.0f32..=3.0).text("self-glow"));
                ui.label("Green glow");
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut p.glow_color);
                    ui.label("colour (also the light)");
                });
                ui.add(egui::Slider::new(&mut p.glow_brightness, 0.0f32..=20.0).text("brightness"));
                ui.add(egui::Slider::new(&mut p.glow_size, 0.2f32..=6.0).text("size (m)"));
                ui.add(egui::Slider::new(&mut p.glow_core, 0.05f32..=1.0).text("core size"));
                ui.add(egui::Slider::new(&mut p.glow_pulse_speed, 0.0f32..=12.0).text("pulse speed"));
                ui.add(egui::Slider::new(&mut p.glow_pulse_amount, 0.0f32..=1.0).text("pulse amount"));
                ui.add(egui::Slider::new(&mut p.glow_swirl, 0.0f32..=1.0).text("wisps"));
                ui.add(egui::Slider::new(&mut p.glow_swirl_speed, 0.0f32..=4.0).text("wisp speed"));
                ui.add(egui::Slider::new(&mut p.glow_sparks, 0.0f32..=6.0).text("sparks"));
                ui.add(egui::Slider::new(&mut p.glow_pull, -3.0f32..=3.0).text("pulled toward camera (m, − pushes away)"));
                ui.add(
                    egui::Slider::new(&mut p.glow_depth_pull, -1.0f32..=1.0)
                        .text("depth-tested at (m toward camera from drop)"),
                );
                ui.add(egui::Slider::new(&mut p.light_lumens, 0.0f32..=200_000.0).logarithmic(true).text("light (lm)"));
                ui.add(egui::Slider::new(&mut p.light_range, 0.5f32..=20.0).text("light range (m)"));
                if ui.button("Copy power-up settings to console").clicked() {
                    info!(
                        "power-ups: height {:.2}, bob {:.2} @ {:.2}, spin {:.0}, blink {:.1}, scale {:?}, \
                         gold {:?} metallic {:.2} roughness {:.2} emissive {:.2}, glow {:?} x{:.2} size {:.2} \
                         core {:.2} pulse {:.2}/{:.2} wisps {:.2}/{:.2} sparks {:.2} pull {:.2} depth {:.2}, light {:.0} lm {:.1} m",
                        p.height, p.bob_height, p.bob_speed, p.spin_deg, p.blink_hz, p.scale,
                        p.gold_color, p.gold_metallic, p.gold_roughness, p.gold_emissive,
                        p.glow_color, p.glow_brightness, p.glow_size, p.glow_core,
                        p.glow_pulse_speed, p.glow_pulse_amount, p.glow_swirl, p.glow_swirl_speed,
                        p.glow_sparks, p.glow_pull, p.glow_depth_pull, p.light_lumens, p.light_range,
                    );
                }
                if ui.button("Reset power-ups").clicked() {
                    *p = default();
                }
            });

            ui.separator();
            ui.collapsing("Molotov (Zombies)", |ui| {
                crate::molotov::molotov_section(ui, &mut molotov_dbg, &mut arms_view.debug_hold_key);
            });

            ui.separator();
            ui.collapsing("Monkey bomb (Zombies)", |ui| {
                let d = &mut molotov_dbg;
                crate::monkey_bomb::monkey_section(
                    ui,
                    &mut d.monkey,
                    &mut d.weapon,
                    &mut arms_view.debug_hold_key,
                    &mut d.monkey_fuse_tx,
                );
            });

            ui.separator();
            ui.collapsing("Frag (Zombies)", |ui| {
                let d = &mut molotov_dbg;
                crate::frag::frag_section(ui, &mut d.frag, &mut d.weapon);
            });

            ui.separator();
            ui.collapsing("Flash bang (Zombies)", |ui| {
                let d = &mut molotov_dbg;
                crate::flash_bang::flash_section(ui, &mut d.flash, &mut d.weapon);
            });

            ui.separator();
            ui.collapsing("Zombies perks", |ui| {
                ui.collapsing("Perk machines", |ui| {
                    let m = &mut *machines;
                    ui.label(
                        "Moves the whole machine — model, collision, light, jingle and where its \
                         card shows — on this client only. The server (buying, bots, shots) \
                         still uses the spots in shared/src/perks.rs until they're copied there.",
                    );
                    ui.add(egui::Slider::new(&mut m.scale, 0.1f32..=2.0).text("scale (all machines)"));
                    ui.label("Light (all machines; colour is each perk's own)");
                    machine_light_ui(ui, &mut m.light, "perk machine light");
                    ui.collapsing("Fog (vented once the power's on)", |ui| {
                        machine_fog_ui(ui, &mut m.fog, "perk machine fog");
                    });
                    ui.label("Where each machine stands is placed in the level editor (main menu).");
                    for perk in shared::perks::Perk::ALL {
                        if perk.set() == shared::perks::PerkSet::Classic && perk.has_machine() {
                            let yaw = m.model_yaw_deg.entry(perk).or_insert(0.0);
                            ui.add(
                                egui::Slider::new(yaw, -180.0f32..=180.0)
                                    .text(format!("{} model turn inside its box (deg)", perk.label())),
                            );
                        }
                    }
                    ui.add(
                        egui::Slider::new(&mut m.wunderfizz_model_yaw_deg, -180.0f32..=180.0)
                            .text("Der Wunderfizz model turn inside its box (deg)"),
                    );
                    if ui.button("Copy perk machine looks to console").clicked() {
                        info!(
                            "perk machine scale: {:.3}, Der Wunderfizz model turn: {:.1}",
                            m.scale, m.wunderfizz_model_yaw_deg
                        );
                        for (perk, yaw) in &m.model_yaw_deg {
                            if *yaw != 0.0 {
                                info!("{} model turn: {yaw:.1} deg", perk.label());
                            }
                        }
                    }
                    if ui.button("Reset perk machines").clicked() {
                        *m = default();
                    }
                });
                ui.collapsing("Ammo crate", |ui| {
                    let a = &mut *ammo_crate;
                    ui.label("Where it stands is placed in the level editor (main menu).");
                    ui.add(
                        egui::Slider::new(&mut a.scale, 0.001f32..=100.0)
                            .logarithmic(true)
                            .text("scale (1 = as modelled)"),
                    );
                    if ui.button("Reset ammo crate").clicked() {
                        *a = default();
                    }
                });
                ui.collapsing("Pack-a-Punch machine", |ui| {
                    let p = &mut *pap;
                    ui.label("Where it stands is placed in the level editor (main menu).");
                    ui.add(
                        egui::Slider::new(&mut p.scale, 0.05f32..=3.0)
                            .logarithmic(true)
                            .text("scale (1 = as made, 1.86 m tall)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut p.model_yaw_deg, -180.0f32..=180.0)
                            .text("model turn inside its box (deg)"),
                    );
                    ui.collapsing("Light (blue, once the power's on)", |ui| {
                        machine_light_ui(ui, &mut p.light, "pap machine light");
                    });
                    ui.collapsing("Fog (vented once the power's on)", |ui| {
                        machine_fog_ui(ui, &mut p.fog, "pap machine fog");
                    });
                    ui.label("Camo (packed weapons)");
                    ui.add(egui::Slider::new(&mut p.camo_scroll.x, -1.0f32..=1.0).text("scroll u (/s)"));
                    ui.add(egui::Slider::new(&mut p.camo_scroll.y, -1.0f32..=1.0).text("scroll v (/s)"));
                    ui.add(egui::Slider::new(&mut p.camo_tiling, 0.1f32..=10.0).logarithmic(true).text("tiling"));
                    ui.add(egui::Slider::new(&mut p.camo_glow, 0.0f32..=10.0).text("glow"));
                    if ui.button("Copy Pack-a-Punch settings to console").clicked() {
                        info!(
                            "pap machine: scale: {:.3}, model_yaw_deg: {:.1}, camo_scroll: ({:.3}, {:.3}), \
                             camo_tiling: {:.2}, camo_glow: {:.2}",
                            p.scale,
                            p.model_yaw_deg,
                            p.camo_scroll.x,
                            p.camo_scroll.y,
                            p.camo_tiling,
                            p.camo_glow,
                        );
                    }
                    if ui.button("Reset Pack-a-Punch machine").clicked() {
                        *p = default();
                    }
                });
                ui.collapsing("Nitro Brew", |ui| {
                    let n = &mut *nitro;
                    ui.label(
                        "Zombies perk (yellow) — multipliers on movement, ADS, reload, rechamber \
                         and weapon swap speed while owned (1 = normal).",
                    );
                    ui.label(if n.owned { "owned: yes" } else { "owned: no" });
                    ui.checkbox(&mut n.debug_force, "Force on (act as if owned)");
                    ui.add(egui::Slider::new(&mut n.move_mult, 0.5f32..=3.0).text("movement speed"));
                    ui.add(egui::Slider::new(&mut n.ads_mult, 0.5f32..=5.0).text("ADS speed"));
                    ui.add(egui::Slider::new(&mut n.reload_mult, 0.5f32..=5.0).text("reload speed"));
                    ui.add(egui::Slider::new(&mut n.rechamber_mult, 0.5f32..=5.0).text("rechamber speed"));
                    ui.add(egui::Slider::new(&mut n.swap_mult, 0.5f32..=5.0).text("weapon swap speed"));
                    if ui.button("Copy Nitro Brew settings to console").clicked() {
                        info!(
                            "nitro brew: move_mult: {:.2}, ads_mult: {:.2}, reload_mult: {:.2}, \
                             rechamber_mult: {:.2}, swap_mult: {:.2}",
                            n.move_mult, n.ads_mult, n.reload_mult, n.rechamber_mult, n.swap_mult,
                        );
                    }
                    if ui.button("Reset Nitro Brew multipliers").clicked() {
                        *n = crate::zombies_hud::NitroBrew {
                            owned: n.owned,
                            debug_force: n.debug_force,
                            ..default()
                        };
                    }
                });

                ui.collapsing("Classic perks", |ui| {
                    let c = &mut *classic;
                    ui.label(
                        "Call of Duty's perks (a Zombies lobby's PERKS: CLASSIC), as in Cold War. \
                         Juggernog, Quick Revive and PhD Flopper's explosions are server-side; these \
                         are the client-side ones (1 = normal).",
                    );
                    let owned: Vec<&str> = c.owned.iter().map(|p| p.label()).collect();
                    ui.label(format!("owned: {}", if owned.is_empty() { "none".to_string() } else { owned.join(", ") }));
                    ui.checkbox(&mut c.debug_force, "Force all on (act as if owned)");
                    ui.add(egui::Slider::new(&mut c.speed_cola_reload, 0.5f32..=4.0).text("Speed Cola reload speed"));
                    ui.add(egui::Slider::new(&mut c.stamin_up_move, 0.5f32..=2.0).text("Stamin-Up movement speed"));
                    ui.add(egui::Slider::new(&mut c.double_tap_fire_rate, 0.5f32..=3.0).text("Double Tap rate of fire"));
                    ui.add(egui::Slider::new(&mut c.phd_slide_speed, 0.5f32..=3.0).text("PhD Flopper slide speed"));
                    ui.add(egui::Slider::new(&mut c.phd_slide, 0.5f32..=3.0).text("PhD Flopper slide length"));
                    ui.label("Death Perception outline (enemies behind walls)");
                    let [r, g, b] = &mut c.death_perception_color;
                    ui.add(egui::Slider::new(r, 0.0f32..=1.0).text("red"));
                    ui.add(egui::Slider::new(g, 0.0f32..=1.0).text("green"));
                    ui.add(egui::Slider::new(b, 0.0f32..=1.0).text("blue"));
                    ui.add(egui::Slider::new(&mut c.death_perception_brightness, 0.1f32..=20.0).text("glow"));
                    ui.add(egui::Slider::new(&mut c.death_perception_sharpness, 0.2f32..=8.0).text("thinness"));
                    ui.add(egui::Slider::new(&mut c.death_perception_inflate, 0.0f32..=0.2).text("puff out (m)"));
                    if ui.button("Reset classic perks").clicked() {
                        *c = crate::zombies_hud::ClassicPerks {
                            owned: std::mem::take(&mut c.owned),
                            debug_force: c.debug_force,
                            ..default()
                        };
                    }
                });

                ui.separator();
                ui.collapsing("Liquid Courage", |ui| {
                    let d = &mut *drunk;
                    ui.label(format!(
                        "Zombies perk (red) — takes {:.0}% of normal damage (server-side \
                         constant). The drunk look below stacks with the shroom effect.",
                        shared::perks::LIQUID_COURAGE_DAMAGE_MULT * 100.0
                    ));
                    ui.checkbox(&mut d.enabled, "enabled (without the perk)");
                    ui.add(egui::Slider::new(&mut d.fade_secs, 0.0f32..=15.0).text("fade in / out (s)"));
                    ui.label("Head sway");
                    ui.add(egui::Slider::new(&mut d.sway_roll_deg, 0.0f32..=5.0).text("roll (deg)"));
                    ui.add(egui::Slider::new(&mut d.sway_drift, 0.0f32..=0.03).text("drift"));
                    ui.add(egui::Slider::new(&mut d.sway_speed, 0.0f32..=3.0).text("speed"));
                    ui.label("Double vision");
                    ui.add(egui::Slider::new(&mut d.double_offset, 0.0f32..=0.04).text("separation"));
                    ui.add(egui::Slider::new(&mut d.double_mix, 0.0f32..=1.0).text("ghost strength"));
                    ui.add(egui::Slider::new(&mut d.double_speed, 0.0f32..=3.0).text("speed"));
                    ui.add(
                        egui::Slider::new(&mut d.double_offset_kick, 0.0f32..=0.08)
                            .text("separation at kick-in peak"),
                    );
                    ui.add(
                        egui::Slider::new(&mut d.double_mix_kick, 0.0f32..=1.0)
                            .text("ghost strength at kick-in peak"),
                    );
                    ui.label("Look");
                    ui.add(egui::Slider::new(&mut d.vignette, 0.0f32..=1.0).text("dark edges"));
                    ui.add(egui::Slider::new(&mut d.flush, 0.0f32..=1.0).text("warm flush"));
                    ui.add(egui::Slider::new(&mut d.blur, 0.0f32..=0.02).text("blur (kick-in only)"));
                    ui.add(
                        egui::Slider::new(&mut d.vignette_kick_scale, 0.0f32..=1.0)
                            .text("dark edges' share of the kick"),
                    );
                    ui.label("Kick-in (right after buying, then back to the above)");
                    ui.add(egui::Slider::new(&mut d.kick.peak, 1.0f32..=8.0).text("peak (× normal)"));
                    ui.add(egui::Slider::new(&mut d.kick.rise_secs, 0.0f32..=10.0).text("ramp up (s)"));
                    ui.add(egui::Slider::new(&mut d.kick.hold_secs, 0.0f32..=10.0).text("hold (s)"));
                    ui.add(egui::Slider::new(&mut d.kick.fall_secs, 0.0f32..=10.0).text("ramp down (s)"));
                    if ui.button("Replay kick-in").clicked() {
                        drunk_kick.0.replay();
                    }
                    if ui.button("Copy Liquid Courage settings to console").clicked() {
                        info!(
                            "liquid courage: sway_roll_deg: {:.2}, sway_drift: {:.4}, sway_speed: {:.2}, \
                             double_offset: {:.4}, double_mix: {:.2}, double_speed: {:.2}, \
                             double_offset_kick: {:.4}, double_mix_kick: {:.2}, vignette: {:.2}, \
                             flush: {:.2}, blur: {:.4}, vignette_kick_scale: {:.2}, kick peak: {:.2}, \
                             rise: {:.2}, hold: {:.2}, fall: {:.2}",
                            d.sway_roll_deg, d.sway_drift, d.sway_speed, d.double_offset,
                            d.double_mix, d.double_speed, d.double_offset_kick, d.double_mix_kick,
                            d.vignette, d.flush, d.blur, d.vignette_kick_scale, d.kick.peak,
                            d.kick.rise_secs, d.kick.hold_secs, d.kick.fall_secs,
                        );
                    }
                    if ui.button("Reset Liquid Courage").clicked() {
                        *d = crate::DrunkSettings {
                            enabled: d.enabled,
                            ..default()
                        };
                    }
                });

                ui.separator();
                ui.collapsing("Bomb Shot", |ui| {
                    ui.label(format!(
                        "Zombies perk (orange) — a 360 no-scope zombie kill explodes: zombies within \
                         {:.0} m die, out to {:.0} m take {:.0}–{:.0} damage (server-side constants in \
                         shared/src/perks.rs).",
                        shared::perks::BOMB_SHOT_KILL_RADIUS,
                        shared::perks::BOMB_SHOT_DAMAGE_RADIUS,
                        shared::perks::BOMB_SHOT_EDGE_DAMAGE_MAX,
                        shared::perks::BOMB_SHOT_EDGE_DAMAGE_MIN,
                    ));
                    // Lives on the server's lobby (`Lobby::bomb_test`), like
                    // "Bots don't attack".
                    let me = local_id.iter().next().map(|l| l.0);
                    if let Some((lobby, is_leader)) =
                        me.and_then(|me| lobbies.iter().find(|l| l.has(me)).map(|l| (l, l.leader == me)))
                    {
                        let mut on = lobby.bomb_test;
                        if ui
                            .add_enabled(
                                is_leader,
                                egui::Checkbox::new(&mut on, "Every zombie kill explodes (test, with damage)"),
                            )
                            .changed()
                        {
                            if let Ok(mut tx) = bomb_test_tx.single_mut() {
                                tx.trigger::<shared::LobbyChannel>(shared::SetBombTest { on });
                            }
                        }
                        if !is_leader {
                            ui.label("Only the party leader can change this.");
                        }
                    }
                    ui.separator();
                    explosion_section(ui, &mut explosion);
                });

                ui.separator();
                ui.collapsing("Kangabrew", |ui| {
                    let k = &mut *kanga;
                    ui.label(
                        "Zombies perk (green) — higher jumps, and in the air right up against a \
                         wall, jump again to kick off it (Black Ops 7 style).",
                    );
                    ui.label(if k.owned { "owned: yes" } else { "owned: no" });
                    ui.checkbox(&mut k.debug_force, "Force on (act as if owned)");
                    ui.add(
                        egui::Slider::new(&mut k.jump_height_mult, 1.0f32..=6.0)
                            .text("jump height (× normal)"),
                    );
                    ui.checkbox(&mut k.wall_jumps, "wall jumps");
                    ui.add(
                        egui::Slider::new(&mut k.wall_jump_height_mult, 0.0f32..=6.0)
                            .text("wall jump height (× normal jump)"),
                    );
                    ui.add(egui::Slider::new(&mut k.wall_push, 0.0f32..=15.0).text("push off wall (m/s)"));
                    ui.add(egui::Slider::new(&mut k.wall_steer, 0.0f32..=15.0).text("toward look (m/s)"));
                    ui.add(egui::Slider::new(&mut k.wall_reach, 0.05f32..=1.5).text("wall reach (m)"));
                    ui.add(egui::Slider::new(&mut k.max_wall_jumps, 0u32..=10).text("wall jumps per air time"));
                    ui.add(egui::Slider::new(&mut k.wall_cooldown, 0.0f32..=1.0).text("wall jump cooldown (s)"));
                    if ui.button("Copy Kangabrew settings to console").clicked() {
                        info!(
                            "kangabrew: jump_height_mult: {:.2}, wall_jumps: {}, wall_jump_height_mult: {:.2}, \
                             wall_push: {:.2}, wall_steer: {:.2}, wall_reach: {:.2}, max_wall_jumps: {}, \
                             wall_cooldown: {:.2}",
                            k.jump_height_mult, k.wall_jumps, k.wall_jump_height_mult, k.wall_push,
                            k.wall_steer, k.wall_reach, k.max_wall_jumps, k.wall_cooldown,
                        );
                    }
                    if ui.button("Reset Kangabrew").clicked() {
                        *k = crate::zombies_hud::Kangabrew {
                            owned: k.owned,
                            debug_force: k.debug_force,
                            ..default()
                        };
                    }
                });

            ui.separator();
                ui.collapsing("Shroom effect", |ui| {
                    let s = &mut *shroom;
                    ui.checkbox(&mut s.enabled, "enabled");
                    ui.add(egui::Slider::new(&mut s.fade_secs, 0.0f32..=15.0).text("fade in / out (s)"));
                    ui.label("Wavy distortion");
                    ui.add(
                        egui::Slider::new(&mut s.wave_amplitude, 0.0f32..=0.05)
                            .text("wave strength")
                            .fixed_decimals(4),
                    );
                    ui.add(egui::Slider::new(&mut s.wave_frequency, 0.5f32..=20.0).text("wave size  (higher = smaller)"));
                    ui.add(egui::Slider::new(&mut s.wave_speed, 0.0f32..=4.0).text("wave speed"));
                    ui.add(
                        egui::Slider::new(&mut s.center_clear, 0.0f32..=1.0)
                            .text("steady centre  (1 = aim point still)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut s.breathe_amplitude, 0.0f32..=0.08)
                            .text("breathing zoom")
                            .fixed_decimals(3),
                    );
                    ui.add(egui::Slider::new(&mut s.breathe_speed, 0.0f32..=4.0).text("breathing speed"));
                    ui.label("Colour");
                    ui.add(egui::Slider::new(&mut s.saturation, 0.0f32..=3.0).text("saturation"));
                    ui.add(egui::Slider::new(&mut s.hue_drift, 0.0f32..=1.5).text("hue shimmer (rad)"));
                    ui.add(egui::Slider::new(&mut s.hue_speed, 0.0f32..=3.0).text("hue shimmer speed"));
                    ui.add(
                        egui::Slider::new(&mut s.chromatic, 0.0f32..=0.1)
                            .text("colour fringing (edges)")
                            .fixed_decimals(3),
                    );
                    ui.label("Enemies through walls");
                    ui.horizontal(|ui| {
                        ui.label("ghost colour");
                        ui.color_edit_button_rgb(&mut s.xray_color);
                    });
                    ui.add(egui::Slider::new(&mut s.xray_brightness, 0.0f32..=12.0).text("glow (×)"));
                    ui.add(egui::Slider::new(&mut s.xray_opacity, 0.0f32..=1.0).text("opacity"));
                    ui.add(
                        egui::Slider::new(&mut s.xray_inflate, 0.0f32..=0.4)
                            .text("haze spread past body (m)")
                            .fixed_decimals(3),
                    );
                    ui.add(egui::Slider::new(&mut s.xray_fill, 0.0f32..=1.0).text("edge opacity  (0 = edges fade out)"));
                    ui.add(egui::Slider::new(&mut s.xray_edge_softness, 0.1f32..=6.0).text("edge softness"));
                    ui.add(egui::Slider::new(&mut s.xray_smoke_scale, 0.1f32..=12.0).text("smoke size  (higher = finer)"));
                    ui.add(egui::Slider::new(&mut s.xray_smoke_speed, 0.0f32..=4.0).text("smoke drift speed"));
                    ui.add(egui::Slider::new(&mut s.xray_smoke_amount, 0.0f32..=1.0).text("smokiness"));
                    ui.add(egui::Slider::new(&mut s.xray_shimmer, 0.0f32..=1.5).text("hue wobble (rad)"));
                    ui.add(
                        egui::Slider::new(&mut s.xray_body_radius, 0.0f32..=2.0)
                            .text("body radius (m) — own parts never count as cover"),
                    );
                    ui.add(egui::Slider::new(&mut s.xray_body_height, 0.0f32..=2.0).text("body middle above feet (m)"));
                    ui.add(
                        egui::Slider::new(&mut s.xray_min_gap, 0.0f32..=2.0)
                            .text("min wall gap (m)"),
                    );
                    ui.label("Aim assist (while aimed down sight)");
                    ui.checkbox(&mut s.assist_enabled, "aim assist enabled");
                    ui.add(egui::Slider::new(&mut s.assist_cone_deg, 0.1f32..=20.0).text("pull cone (° off crosshair)"));
                    ui.add(egui::Slider::new(&mut s.assist_strength, 0.0f32..=30.0).text("pull strength (/s)"));
                    ui.add(egui::Slider::new(&mut s.assist_max_speed_deg, 0.0f32..=180.0).text("max pull speed (°/s)"));
                    ui.add(egui::Slider::new(&mut s.assist_range, 5.0f32..=300.0).text("range (m)"));
                    ui.add(egui::Slider::new(&mut s.assist_min_ads, 0.0f32..=1.0).text("min scope-in (0 = hip too)"));
                    ui.label("Kick-in (right after buying, then back to the above)");
                    ui.add(egui::Slider::new(&mut s.kick.peak, 1.0f32..=8.0).text("peak (× normal)"));
                    ui.add(egui::Slider::new(&mut s.kick.rise_secs, 0.0f32..=10.0).text("ramp up (s)"));
                    ui.add(egui::Slider::new(&mut s.kick.hold_secs, 0.0f32..=10.0).text("hold (s)"));
                    ui.add(egui::Slider::new(&mut s.kick.fall_secs, 0.0f32..=10.0).text("ramp down (s)"));
                    if ui.button("Replay kick-in").clicked() {
                        shroom_kick.0.replay();
                    }
                    if ui.button("Reset shroom").clicked() {
                        *s = ShroomSettings {
                            enabled: s.enabled,
                            ..default()
                        };
                    }
                });
            });

            ui.separator();
            ui.collapsing("Maps & lighting", |ui| {
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
                    ui.label("models/maps/shipment.glb — spawned at the origin, scale only");
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

            ui.separator();
                ui.collapsing("Fog & Sky", |ui| {
                    ui.collapsing("Basic Map", |ui| {
                        scene_tuning_sliders(ui, &mut scene);
                        if ui.button("Reset fog & sky").clicked() {
                            *scene = SceneTuning::default();
                        }
                    });

                ui.separator();
                    ui.collapsing("Shipment", |ui| {
                        ui.label("MW3-style setting: dark, foggy, overcast, out on open water");
                        scene_tuning_sliders(ui, &mut shipment_scene.0);
                        if ui.button("Reset fog & sky").clicked() {
                            *shipment_scene = ShipmentSceneTuning::default();
                        }
                    });

                ui.separator();
                    ui.collapsing("Shipment Day", |ui| {
                        ui.label("Bright, clear daytime — barely any fog, no rain");
                        scene_tuning_sliders(ui, &mut shipment_day_scene.0);
                        if ui.button("Reset fog & sky").clicked() {
                            *shipment_day_scene = ShipmentDaySceneTuning::default();
                        }
                    });

                ui.separator();
                    ui.collapsing("Break Point", |ui| {
                        ui.label("Clear mid-day — bright sun, virtually no fog (very high visibility)");
                        scene_tuning_sliders(ui, &mut break_point_scene.0);
                        if ui.button("Reset fog & sky").clicked() {
                            *break_point_scene = BreakPointSceneTuning::default();
                        }
                    });

                ui.separator();
                    ui.collapsing("Break Point Night", |ui| {
                        ui.label("Full night — faint moonlight, dark fog; flashlights do the rest");
                        scene_tuning_sliders(ui, &mut break_point_night_scene.0);
                        if ui.button("Reset fog & sky").clicked() {
                            *break_point_night_scene = BreakPointNightSceneTuning::default();
                        }
                    });
                });

            ui.separator();
                ui.collapsing("Flashlight", |ui| {
                    let f = &mut *flashlight;
                    ui.label("Gun-mounted light — yours and every other player's (not zombies). Always on for Break Point Night.");
                    ui.checkbox(&mut f.force_on, "force on (every map)");
                    ui.add(
                        egui::Slider::new(&mut f.intensity, 0.0f32..=40_000_000.0)
                            .logarithmic(true)
                            .text("brightness (lm)"),
                    );
                    ui.add(egui::Slider::new(&mut f.range, 5.0f32..=300.0).text("range (m)"));
                    ui.add(egui::Slider::new(&mut f.outer_angle_deg, 1.0f32..=80.0).text("beam edge (° half-angle)"));
                    ui.add(egui::Slider::new(&mut f.inner_angle_deg, 0.0f32..=80.0).text("bright core (° half-angle)"));
                    ui.horizontal(|ui| {
                        ui.color_edit_button_rgb(&mut f.color);
                        ui.label("colour");
                    });
                    ui.label("mount offset from the eye (m)");
                    ui.add(egui::Slider::new(&mut f.offset.x, -1.0f32..=1.0).text("right"));
                    ui.add(egui::Slider::new(&mut f.offset.y, -1.0f32..=1.0).text("up"));
                    ui.add(egui::Slider::new(&mut f.offset.z, -1.5f32..=0.5).text("back (−forward)"));
                    ui.checkbox(&mut f.shadows, "shadows (yours)");
                    ui.add(egui::Slider::new(&mut f.remote_intensity_mult, 0.0f32..=3.0).text("other players' brightness (×)"));
                    ui.checkbox(&mut f.remote_shadows, "shadows (other players' — costly)");
                    if ui.button("Copy flashlight settings to console").clicked() {
                        info!(
                            "flashlight: intensity: {:.0}, range: {:.1}, outer_angle_deg: {:.1}, inner_angle_deg: {:.1}, \
                             color: {:?}, offset: {:?}, shadows: {}, remote_intensity_mult: {:.2}, remote_shadows: {}",
                            f.intensity, f.range, f.outer_angle_deg, f.inner_angle_deg, f.color, f.offset,
                            f.shadows, f.remote_intensity_mult, f.remote_shadows,
                        );
                    }
                    if ui.button("Reset flashlight").clicked() {
                        *f = FlashlightSettings {
                            force_on: f.force_on,
                            ..default()
                        };
                    }
                });

            ui.separator();
                ui.collapsing("Shipment lights", |ui| {
                    ui.collapsing("Markers", |ui| {
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
                });
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

/// A machine light's sliders (the perk machines' and the Pack-a-Punch's),
/// and a button printing them to the console as `name`.
fn machine_light_ui(ui: &mut egui::Ui, l: &mut crate::zombies_hud::MachineLight, name: &str) {
    ui.add(egui::Slider::new(&mut l.offset.x, -3.0f32..=3.0).text("x — across (m)"));
    ui.add(egui::Slider::new(&mut l.offset.y, -1.0f32..=6.0).text("y — up from the ground (m)"));
    ui.add(egui::Slider::new(&mut l.offset.z, -3.0f32..=3.0).text("z — out the front (m)"));
    ui.add(
        egui::Slider::new(&mut l.intensity, 0.0f32..=500_000.0)
            .logarithmic(true)
            .text("intensity (lm)"),
    );
    ui.add(egui::Slider::new(&mut l.range, 0.5f32..=30.0).text("range (m)"));
    ui.add(egui::Slider::new(&mut l.radius, 0.0f32..=2.0).text("radius (m)"));
    ui.checkbox(&mut l.shadows, "shadows");
    if ui.button(format!("Copy {name} to console")).clicked() {
        info!(
            "{name}: offset: ({:.2}, {:.2}, {:.2}), intensity: {:.0}, range: {:.2}, radius: {:.2}, shadows: {}",
            l.offset.x, l.offset.y, l.offset.z, l.intensity, l.range, l.radius, l.shadows,
        );
    }
    if ui.button("Reset light").clicked() {
        *l = default();
    }
}

/// A machine's fog sliders (the perk machines' and the Pack-a-Punch's), and
/// a button printing them to the console as `name`.
fn machine_fog_ui(ui: &mut egui::Ui, f: &mut crate::vfx::MachineFog, name: &str) {
    ui.checkbox(&mut f.enabled, "on");
    ui.add(egui::Slider::new(&mut f.rate, 0.0f32..=20.0).text("puffs / s per vent"));
    ui.add(egui::Slider::new(&mut f.life_secs, 0.2f32..=10.0).text("puff life (s)"));
    ui.add(egui::Slider::new(&mut f.fade_in_secs, 0.0f32..=3.0).text("fade in (s)"));
    ui.add(egui::Slider::new(&mut f.front_height, 0.0f32..=2.5).text("front vent height (m)"));
    ui.add(egui::Slider::new(&mut f.side_height, 0.0f32..=2.5).text("side vents height (m)"));
    ui.add(egui::Slider::new(&mut f.front_yaw_deg, -180.0f32..=180.0).text("which way is the front (°)"));
    ui.add(egui::Slider::new(&mut f.out_speed, 0.0f32..=3.0).text("out speed (m/s)"));
    ui.add(egui::Slider::new(&mut f.fall_speed, 0.0f32..=3.0).text("sink speed (m/s)"));
    ui.add(egui::Slider::new(&mut f.gravity, 0.0f32..=5.0).text("heaviness (m/s²)"));
    ui.add(egui::Slider::new(&mut f.floor_spread, 0.0f32..=3.0).text("floor spread (m/s²)"));
    ui.add(egui::Slider::new(&mut f.drag, 0.0f32..=5.0).text("drag"));
    ui.add(egui::Slider::new(&mut f.start_size, 0.05f32..=2.0).text("start size (m)"));
    ui.add(egui::Slider::new(&mut f.end_size, 0.05f32..=5.0).text("end size (m)"));
    ui.add(egui::Slider::new(&mut f.opacity, 0.0f32..=1.0).text("opacity"));
    ui.add(egui::Slider::new(&mut f.glow, 0.0f32..=1.0).text("glow"));
    ui.add(egui::Slider::new(&mut f.max_distance, 5.0f32..=150.0).text("vents within (m)"));
    if ui.button(format!("Copy {name} to console")).clicked() {
        info!(
            "{name}: rate: {:.2}, life_secs: {:.2}, fade_in_secs: {:.2}, front_height: {:.2}, \
             side_height: {:.2}, front_yaw_deg: {:.1}, out_speed: {:.2}, fall_speed: {:.2}, gravity: {:.2}, \
             floor_spread: {:.2}, drag: {:.2}, start_size: {:.2}, end_size: {:.2}, opacity: {:.3}, glow: {:.3}, \
             max_distance: {:.1}",
            f.rate, f.life_secs, f.fade_in_secs, f.front_height, f.side_height, f.front_yaw_deg, f.out_speed,
            f.fall_speed, f.gravity, f.floor_spread, f.drag, f.start_size, f.end_size, f.opacity, f.glow,
            f.max_distance,
        );
    }
    if ui.button("Reset fog").clicked() {
        *f = default();
    }
}
