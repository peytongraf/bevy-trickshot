//! The WEAPON EDITOR — main menu → WEAPON EDITOR ([`AppState::WeaponEditor`]).
//! Lines each weapon up in a remote player's hands: another player's soldier
//! stands on a stand, holding the weapon picked, through whichever of its
//! animations is picked — playing, or paused on any moment of it — and every
//! number of that weapon's grip (`avatars::held_weapon::Grip`) is there to
//! drag: where it sits in the right hand, how far that hand's moved from the
//! animation's, where the left hand goes on it, and its muzzle and scope.
//! Saving writes `client/data/weapon_grips.ron` (and prints it to the
//! console); a dev build reads that file at launch, a release build has it
//! built in.
//!
//! Changes show on every soldier at once — they're the live grips — and are
//! put back as last saved on leaving without saving.
//!
//! Offline and game-free, like the level editor: no lobby, no server. Left,
//! middle or right drag orbits (Shift pans), the wheel zooms. Esc, the EXIT
//! button and the mouse back button leave — asking first if anything's
//! unsaved. Everything's rebuilt on the way in and dropped on the way out;
//! the player rig the camera borrows, the view model camera and the light
//! and fog are put back as they were ([`exit`]); the scene is `StateScoped`.

use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::graph::AnimationNodeType;
use bevy::animation::RepeatAnimation;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::view::RenderLayers;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use shared::lethal::LethalKind;
use shared::perks::Perk;

use crate::avatars::held_weapon::{
    euler_deg, grips_source_path, ik_wanted, to_euler_deg, ArmPose, Grip, GripKey, HandTarget, StowOn, WeaponGrips,
    BAKED_GUN_CENTER, BAKED_GUN_TURN_DEG, GRIPS_DIR, GRIPS_FILE,
};
use crate::player::{PlayerBodyCapsule, PlayerHead, ViewModelCamera, WorldModelCamera};
use crate::weapons::{CameraRecoil, CameraShake};
use crate::{
    AppState, HeldWeapon, Player, RemoteAvatarSettings, SoldierAnimState, SoldierAnimationPlayer, SoldierAnimations,
    SoldierRig, SoldierVisual,
};

pub(crate) struct WeaponEditorPlugin;

impl Plugin for WeaponEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GripEditor>()
            .init_resource::<StudioBackup>()
            .init_gizmo_group::<GripGizmos>()
            .add_systems(Startup, configure_gizmos)
            .add_systems(OnEnter(AppState::WeaponEditor), enter)
            .add_systems(OnExit(AppState::WeaponEditor), exit)
            .add_systems(
                Update,
                (
                    (camera_input, keys, drive_camera, drive_soldier, draw_markers).chain(),
                    crate::sky_follow_camera,
                )
                    .run_if(in_state(AppState::WeaponEditor)),
            )
            .add_systems(EguiPrimaryContextPass, editor_ui.run_if(in_state(AppState::WeaponEditor)));
    }
}

/// Where the stand is — high over wherever the map is, lost in the fog.
const STUDIO: Vec3 = Vec3::new(0.0, 600.0, 0.0);

/// Radians of orbit per pixel dragged.
const ORBIT_PER_PX: f32 = 0.006;

/// The clips there are to pick, as the game plays them.
const CLIPS: [(SoldierAnimState, &str); 18] = [
    (SoldierAnimState::Idle, "Idle"),
    (SoldierAnimState::IdleNoGun, "Idle, no gun"),
    (SoldierAnimState::Aim, "Aiming"),
    (SoldierAnimState::Walk, "Walk"),
    (SoldierAnimState::AimWalk, "Aiming, moving"),
    (SoldierAnimState::Sprint, "Sprint"),
    (SoldierAnimState::StrafeLeft, "Strafe left"),
    (SoldierAnimState::StrafeRight, "Strafe right"),
    (SoldierAnimState::Backward, "Backward"),
    (SoldierAnimState::Crouch, "Crouch"),
    (SoldierAnimState::CrouchWalk, "Crouch walk"),
    (SoldierAnimState::Prone, "Prone"),
    (SoldierAnimState::ProneCrawl, "Prone, crawling forward"),
    (SoldierAnimState::ProneCrawlBack, "Prone, crawling back"),
    (SoldierAnimState::Jump, "Jump"),
    (SoldierAnimState::Reload, "Reload (hands free)"),
    (SoldierAnimState::Melee, "Melee (hands free)"),
    (SoldierAnimState::Dead, "Death (hands free)"),
];

fn clip_label(state: SoldierAnimState) -> &'static str {
    CLIPS.iter().find(|(s, _)| *s == state).map_or("?", |(_, l)| *l)
}

/// The markers on the weapon — drawn over it.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct GripGizmos;

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<GripGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 2.0;
    // (Only the world camera — not the HUD one.)
    config.render_layers = RenderLayers::layer(0);
}

/// The camera, orbiting `focus`: `distance` away, turned `yaw` about the
/// vertical (0 looks along -Z) and tipped `pitch` (negative looks down).
#[derive(Clone, Copy)]
struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        // From in front and to the soldier's right (it faces +Z), at the
        // height of its hands.
        Self {
            focus: STUDIO + Vec3::Y * 1.3,
            yaw: -0.7,
            pitch: -0.12,
            distance: 2.0,
        }
    }
}

impl Orbit {
    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    fn eye(&self) -> Vec3 {
        self.focus + self.rotation() * Vec3::Z * self.distance
    }
}

/// Everything the editor's doing — rebuilt on the way in, dropped on the way
/// out.
#[derive(Resource)]
struct GripEditor {
    weapon: GripKey,
    /// The other weapon carried — where it's carried shows, and SWAP plays
    /// the swap to it.
    also: Option<GripKey>,
    /// The Pack-a-Punch camo shown.
    pap: u8,
    clip: SoldierAnimState,
    /// The clip the soldier's been set playing.
    playing_clip: Option<SoldierAnimState>,
    playing: bool,
    /// Where the clip's at (s) — the scrubber's, while paused.
    time: f32,
    /// The clip's length (s), once it's in.
    length: f32,
    speed: f32,
    /// Where the soldier's looking, up (+) or down (degrees).
    look_deg: f32,
    /// The hands on the weapon (IK) — off shows the animation's own.
    hands: bool,
    /// `soldier.glb`'s own rifle shown, to line a weapon up against.
    show_rifle: bool,
    show_markers: bool,
    cam: Orbit,
    dragging: bool,
    /// The grips as last saved (or as they were coming in).
    saved: Option<WeaponGrips>,
    exit_prompt: bool,
    help_open: bool,
    /// A message for the status bar, and when it goes (elapsed seconds).
    toast: Option<(String, f32)>,
    /// What the throw section throws...
    throw_kind: LethalKind,
    /// ...a throw playing, this long (s) since its weapon started going
    /// away...
    throw_clock: Option<f32>,
    /// ...or the arm held up ready, to place what's in the hand by.
    throw_ready: bool,
    throw_layer: crate::UpperLayer,
    /// The perk the drink section drinks...
    drink_perk: Perk,
    /// ...a drink playing, this long (s) since the weapon started going
    /// away...
    drink_clock: Option<f32>,
    /// ...or held at this moment of it (`0..1`), to place the bottle by.
    drink_hold_at: Option<f32>,
    drink_layer: crate::UpperLayer,
    /// Play the knife's stab once...
    play_stab: bool,
    /// ...or over and over.
    loop_stab: bool,
}

impl Default for GripEditor {
    fn default() -> Self {
        Self {
            weapon: GripKey::Sniper,
            also: Some(GripKey::Knife),
            pap: 0,
            clip: SoldierAnimState::Aim,
            playing_clip: None,
            playing: false,
            time: 0.0,
            length: 0.0,
            speed: 1.0,
            look_deg: 0.0,
            hands: true,
            show_rifle: false,
            show_markers: true,
            cam: Orbit::default(),
            dragging: false,
            saved: None,
            exit_prompt: false,
            help_open: false,
            toast: None,
            throw_kind: LethalKind::Frag,
            throw_clock: None,
            throw_ready: false,
            throw_layer: crate::UpperLayer::default(),
            drink_perk: Perk::SpeedCola,
            drink_clock: None,
            drink_hold_at: None,
            drink_layer: crate::UpperLayer::default(),
            play_stab: false,
            loop_stab: false,
        }
    }
}

impl GripEditor {
    fn toast(&mut self, text: impl Into<String>, now: f32) {
        self.toast = Some((text.into(), now + 4.0));
    }

    fn dirty(&self, grips: &WeaponGrips) -> bool {
        self.saved.as_ref().is_some_and(|s| s != grips)
    }

    /// Save the grips: over the file in the repo when this is a dev build run
    /// from one, and to the console either way.
    fn save(&mut self, grips: &WeaponGrips, now: f32) {
        let text = grips.to_ron();
        info!("weapon editor: {GRIPS_FILE}:\n{text}");
        let message = if !std::path::Path::new(GRIPS_DIR).is_dir() {
            format!("No repo here — {GRIPS_FILE} printed to the console")
        } else {
            match std::fs::write(grips_source_path(), &text) {
                Ok(()) => format!("Saved client/data/{GRIPS_FILE} — rebuild the client to ship it"),
                Err(e) => {
                    self.toast(format!("Couldn't write {GRIPS_FILE}: {e} (printed to the console)"), now);
                    return;
                }
            }
        };
        self.saved = Some(grips.clone());
        self.toast(message, now);
    }

    /// EXIT / Esc / the mouse back button: leave, or ask first if anything's
    /// unsaved. `true` to leave now.
    fn request_exit(&mut self, grips: &WeaponGrips) -> bool {
        if self.dirty(grips) {
            self.exit_prompt = true;
            false
        } else {
            true
        }
    }
}

/// The soldier on the stand.
#[derive(Component)]
struct StandSoldier;

// --- in and out -------------------------------------------------------------------

/// What the editor borrowed or changed, to put back on the way out: the
/// player rig the camera rides (so a game starts from where it would have),
/// the body capsule's shadow, and the light and fog.
#[derive(Resource, Default)]
struct StudioBackup {
    transforms: Vec<(Entity, Transform)>,
    capsule: Option<(Entity, Visibility)>,
    look: Option<(f32, FogFalloff)>,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn enter(
    mut editor: ResMut<GripEditor>,
    grips: Res<WeaponGrips>,
    mut backup: ResMut<StudioBackup>,
    mut rig: Query<
        (Entity, &mut Transform, Has<Player>, Has<PlayerHead>),
        Or<(With<Player>, With<PlayerHead>, With<CameraShake>, With<CameraRecoil>, With<WorldModelCamera>)>,
    >,
    mut capsule: Query<(Entity, &mut Visibility), With<PlayerBodyCapsule>>,
    mut view_model: Query<&mut Camera, With<ViewModelCamera>>,
    mut ambient: ResMut<AmbientLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
    window: Single<&mut Window, With<PrimaryWindow>>,
    avatar: Res<RemoteAvatarSettings>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    *editor = GripEditor {
        saved: Some(grips.clone()),
        ..default()
    };
    crate::set_cursor_grabbed(&mut window.into_inner(), false);

    // The camera rig: the player (yaw) and head (pitch) carry the view; the
    // shake / recoil / sway nodes under them sit still.
    backup.transforms.clear();
    for (e, mut t, player, head) in &mut rig {
        backup.transforms.push((e, *t));
        if !player && !head {
            *t = Transform::IDENTITY;
        }
    }
    // (Its shadow would hang under the camera.)
    backup.capsule = capsule.iter_mut().next().map(|(e, mut v)| {
        let was = *v;
        *v = Visibility::Hidden;
        (e, was)
    });
    // No gun in the view.
    for mut cam in &mut view_model {
        cam.is_active = false;
    }
    // Bright, and the fog pushed out past the stand — the map far below
    // stays lost in it.
    backup.look = Some((ambient.brightness, fog.falloff.clone()));
    ambient.brightness = ambient.brightness.max(500.0);
    fog.falloff = FogFalloff::Linear { start: 60.0, end: 400.0 };

    // The stand, its lights, and the soldier on it.
    commands.spawn((
        StateScoped(AppState::WeaponEditor),
        Mesh3d(meshes.add(Circle::new(1.4))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.16, 0.17, 0.19),
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_translation(STUDIO).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
    ));
    for (offset, intensity) in [(Vec3::new(-1.6, 2.4, 2.2), 900_000.0), (Vec3::new(2.0, 1.6, -1.5), 350_000.0)] {
        commands.spawn((
            StateScoped(AppState::WeaponEditor),
            PointLight {
                intensity,
                range: 12.0,
                ..default()
            },
            Transform::from_translation(STUDIO + offset),
        ));
    }
    commands
        .spawn((
            StateScoped(AppState::WeaponEditor),
            StandSoldier,
            SoldierVisual,
            HeldWeapon::new(Some(editor.weapon.weapon())),
            Transform::from_translation(STUDIO).with_scale(Vec3::splat(avatar.scale)),
            Visibility::default(),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/characters/soldier.glb"))),
        ))
        .observe(crate::start_soldier_animation);
}

#[allow(clippy::too_many_arguments)]
fn exit(
    mut editor: ResMut<GripEditor>,
    mut grips: ResMut<WeaponGrips>,
    mut backup: ResMut<StudioBackup>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
    mut view_model: Query<&mut Camera, With<ViewModelCamera>>,
    mut ambient: ResMut<AmbientLight>,
    mut fog: Single<&mut DistanceFog, With<WorldModelCamera>>,
) {
    // Left without saving: the game goes on with what was saved.
    if let Some(saved) = editor.saved.take() {
        grips.set_if_neq(saved);
    }
    *editor = GripEditor::default();
    for (e, t) in backup.transforms.drain(..) {
        if let Ok(mut now) = transforms.get_mut(e) {
            *now = t;
        }
    }
    if let Some((e, v)) = backup.capsule.take() {
        if let Ok(mut now) = visibility.get_mut(e) {
            *now = v;
        }
    }
    for mut cam in &mut view_model {
        cam.is_active = true;
    }
    if let Some((brightness, falloff)) = backup.look.take() {
        ambient.brightness = brightness;
        fog.falloff = falloff;
    }
}

// --- input ----------------------------------------------------------------------------

/// Left, middle or right drag orbits (Shift pans); the wheel zooms.
fn camera_input(
    mut editor: ResMut<GripEditor>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let buttons = [MouseButton::Left, MouseButton::Middle, MouseButton::Right];
    // A drag that started over the stand keeps going over a panel.
    if mouse.any_just_pressed(buttons) && !egui.is_pointer_over_area() && !editor.exit_prompt {
        editor.dragging = true;
    }
    if !mouse.any_pressed(buttons) {
        editor.dragging = false;
    }
    let mut cam = editor.cam;
    if editor.dragging && motion.delta != Vec2::ZERO {
        let d = motion.delta;
        if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
            let per_px = 2.0 * cam.distance * (FRAC_PI_2 * 0.5).tan() / window.height().max(1.0);
            let rot = cam.rotation();
            cam.focus += (rot * Vec3::NEG_X * d.x + rot * Vec3::Y * d.y) * per_px;
        } else {
            cam.yaw -= d.x * ORBIT_PER_PX;
            cam.pitch = (cam.pitch - d.y * ORBIT_PER_PX).clamp(-1.5, 1.5);
        }
    }
    if scroll.delta != Vec2::ZERO && !egui.is_pointer_over_area() {
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        cam.distance = (cam.distance * 0.85f32.powf(lines)).clamp(0.25, 12.0);
    }
    editor.cam = cam;
}

/// Ctrl+S saves; Space plays / pauses; Esc and the mouse back button leave
/// (or close the "unsaved changes" question).
fn keys(
    mut editor: ResMut<GripEditor>,
    grips: Res<WeaponGrips>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    time: Res<Time>,
    mut next: ResMut<NextState<AppState>>,
) {
    let typing = egui.wants_any_keyboard_input();
    let back = mouse.just_pressed(MouseButton::Back);
    let esc = keys.just_pressed(KeyCode::Escape) && !typing;
    if back || esc {
        if editor.help_open {
            editor.help_open = false;
        } else if editor.exit_prompt {
            editor.exit_prompt = false;
        } else if editor.request_exit(&grips) {
            next.set(AppState::MainMenu);
        }
    }
    if typing {
        return;
    }
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    if ctrl && keys.just_pressed(KeyCode::KeyS) {
        editor.save(&grips, time.elapsed_secs());
    }
    if keys.just_pressed(KeyCode::Space) {
        editor.playing = !editor.playing;
    }
}

fn drive_camera(
    editor: Res<GripEditor>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerHead>)>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
) {
    let cam = editor.cam;
    player.translation = cam.eye();
    player.rotation = Quat::from_rotation_y(cam.yaw);
    head.rotation = Quat::from_rotation_x(cam.pitch);
}

// --- the soldier ----------------------------------------------------------------------

/// Keep the soldier holding the weapon picked, playing (or paused in) the
/// clip picked, with the model's own rifle shown or not — and stabbing when
/// asked.
#[allow(clippy::too_many_arguments)]
fn drive_soldier(
    mut editor: ResMut<GripEditor>,
    anims: Res<SoldierAnimations>,
    graphs: Res<Assets<AnimationGraph>>,
    clips: Res<Assets<AnimationClip>>,
    mut soldiers: Query<(&mut HeldWeapon, Option<&SoldierRig>, Option<&SoldierAnimationPlayer>), With<StandSoldier>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut visibility: Query<&mut Visibility>,
    time: Res<Time>,
) {
    let Ok((mut held, rig, anim_player)) = soldiers.single_mut() else {
        return;
    };
    // A throw, as a game plays it: the weapon put away, the arm up, the
    // throw — let go of at its release — and the weapon back out.
    const PUT_AWAY_SECS: f32 = 0.45;
    const RAISE_SECS: f32 = 0.2;
    let dt = time.delta_secs();
    let throw = match editor.throw_clock.map(|c| c + dt) {
        Some(c) if c < PUT_AWAY_SECS => Some(None),
        Some(c) if c < PUT_AWAY_SECS + RAISE_SECS => Some(Some(shared::ThrowStage::Raising((c - PUT_AWAY_SECS) / RAISE_SECS))),
        Some(c) if c < PUT_AWAY_SECS + RAISE_SECS + crate::THROW_SECS => {
            let t = (c - PUT_AWAY_SECS - RAISE_SECS) / crate::THROW_SECS;
            Some(Some(shared::ThrowStage::Throwing {
                t,
                released: t * crate::THROW_SECS >= crate::THROW_RELEASE_SECS,
            }))
        }
        _ if editor.throw_ready => Some(Some(shared::ThrowStage::Raising(1.0))),
        _ => None,
    };
    editor.throw_clock = editor
        .throw_clock
        .map(|c| c + dt)
        .filter(|c| *c < PUT_AWAY_SECS + RAISE_SECS + crate::THROW_SECS);
    let stage = throw.flatten();
    held.throwing = stage.map(|s| {
        (editor.throw_kind, !matches!(s, shared::ThrowStage::Throwing { released: true, .. }))
    });
    // A perk drink, likewise: the weapon put away, the drink, the weapon back.
    let drink = match editor.drink_clock.map(|c| c + dt) {
        Some(c) if c < PUT_AWAY_SECS => Some(None),
        Some(c) if c < PUT_AWAY_SECS + crate::DRINK_SECS => Some(Some((c - PUT_AWAY_SECS) / crate::DRINK_SECS)),
        _ => editor.drink_hold_at.map(Some),
    };
    editor.drink_clock = editor.drink_clock.map(|c| c + dt).filter(|c| *c < PUT_AWAY_SECS + crate::DRINK_SECS);
    let drinking = drink.flatten();
    held.drinking = drinking.map(|t| (editor.drink_perk, t));
    let item = (throw.is_none() && drink.is_none()).then(|| editor.weapon.weapon());
    if held.item != item {
        held.item = item;
    }
    if held.pap != editor.pap {
        held.pap = editor.pap;
    }
    let carried: Vec<_> = [Some(editor.weapon), editor.also]
        .into_iter()
        .flatten()
        .map(|k| (k.weapon(), editor.pap))
        .collect();
    if held.carried != carried {
        held.carried = carried;
    }
    held.ik_wanted = if editor.hands { ik_wanted(editor.clip) } else { 0.0 };
    held.pitch = editor.look_deg.to_radians();
    held.stabbing = std::mem::take(&mut editor.play_stab) || (editor.loop_stab && !held.stab_playing());

    if let Some(gun) = rig.and_then(|r| r.baked_gun) {
        if let Ok(mut v) = visibility.get_mut(gun) {
            v.set_if_neq(if editor.show_rifle { Visibility::Inherited } else { Visibility::Hidden });
        }
    }

    let Some(Ok((mut player, mut transitions))) = anim_player.map(|p| players.get_mut(p.0)) else {
        return;
    };
    let mut layer = editor.throw_layer;
    crate::drive_throw_layer(&mut player, &anims, &mut layer, stage, dt, 0.2);
    editor.throw_layer = layer;
    let mut layer = editor.drink_layer;
    crate::drive_drink_layer(&mut player, &anims, &mut layer, drinking, dt, 0.2);
    editor.drink_layer = layer;
    let node = anims.node_for(editor.clip);
    if editor.playing_clip != Some(editor.clip) {
        editor.playing_clip = Some(editor.clip);
        editor.time = 0.0;
        // (Everything else stops first: a transition never fades out a
        // paused clip, which would play on under the new one.)
        player.stop_all();
        // (Every clip loops here, to look at any moment of it.)
        transitions.play(&mut player, node, Duration::ZERO).set_repeat(RepeatAnimation::Forever);
        // Aiming on the move is the legs only — the standing aim's upper body
        // goes over them, held, as in a game.
        if matches!(editor.clip, SoldierAnimState::AimWalk | SoldierAnimState::AimSprint) {
            player
                .start(anims.shot_upper())
                .set_repeat(RepeatAnimation::Never)
                .seek_to(crate::SHOT_CLIP_SECS);
        }
    }
    editor.length = graphs
        .get(anims.graph())
        .and_then(|g| g.get(node))
        .and_then(|n| match &n.node_type {
            AnimationNodeType::Clip(clip) => clips.get(clip).map(AnimationClip::duration),
            _ => None,
        })
        .unwrap_or(0.0);
    let Some(active) = player.animation_mut(node) else { return };
    if editor.playing {
        active.resume().set_speed(editor.speed);
        editor.time = if editor.length > 0.0 { active.seek_time().rem_euclid(editor.length) } else { active.seek_time() };
    } else {
        active.pause().seek_to(editor.time);
    }
}

/// The muzzle (red), scope (white) and left hand's spot (green) on the
/// weapon, and the right-hand bone's axes.
fn draw_markers(
    editor: Res<GripEditor>,
    grips: Res<WeaponGrips>,
    soldiers: Query<(&HeldWeapon, Option<&SoldierRig>), With<StandSoldier>>,
    transforms: Query<&GlobalTransform>,
    mut gizmos: Gizmos<GripGizmos>,
) {
    if !editor.show_markers {
        return;
    }
    let Ok((held, rig)) = soldiers.single() else { return };
    let grip = grips.get(editor.weapon);
    if let Some(at) = held.holder.and_then(|h| transforms.get(h).ok()) {
        let p = |v: [f32; 3]| at.transform_point(Vec3::from_array(v));
        gizmos.sphere(Isometry3d::from_translation(p(grip.muzzle)), 0.015, Color::srgb(1.0, 0.2, 0.2));
        if let Some(scope) = grip.scope {
            gizmos.sphere(Isometry3d::from_translation(p(scope)), 0.015, Color::WHITE);
        }
        if let Some(left) = grip.left_hand {
            let (_, rot, _) = at.to_scale_rotation_translation();
            gizmos.axes(
                Transform::from_translation(p(left.pos)).with_rotation(rot * left.rotation()),
                0.06,
            );
            gizmos.sphere(Isometry3d::from_translation(p(left.pos)), 0.012, Color::srgb(0.3, 1.0, 0.4));
        }
        // The weapon's own frame: barrel along -X, sights up +Y.
        let (_, rot, origin) = at.to_scale_rotation_translation();
        gizmos.arrow(origin, origin + rot * Vec3::NEG_X * 0.15, Color::srgb(1.0, 0.6, 0.1));
        gizmos.arrow(origin, origin + rot * Vec3::Y * 0.08, Color::srgb(0.4, 0.7, 1.0));
    }
    if let Some(hand) = rig.and_then(|r| transforms.get(r.right[2]).ok()) {
        let (_, rot, pos) = hand.to_scale_rotation_translation();
        gizmos.axes(Transform::from_translation(pos).with_rotation(rot), 0.05);
    }
}

// --- the panels -------------------------------------------------------------------------

const DIM: egui::Color32 = egui::Color32::from_gray(140);
const WARN: egui::Color32 = egui::Color32::from_rgb(255, 120, 90);

/// Every control, for the help window: `(keys, what they do)`.
const CONTROLS: &[(&str, &str)] = &[
    ("Left / middle / right drag", "Orbit"),
    ("Shift + drag", "Pan"),
    ("Wheel", "Zoom"),
    ("Space", "Play / pause the clip"),
    ("Ctrl + S", "Save"),
    ("Esc  /  mouse back", "Exit (asks first if anything's unsaved)"),
];

#[allow(clippy::too_many_arguments)]
fn editor_ui(
    mut contexts: EguiContexts,
    mut editor: ResMut<GripEditor>,
    mut grips: ResMut<WeaponGrips>,
    soldiers: Query<(&HeldWeapon, Option<&SoldierRig>), With<StandSoldier>>,
    time: Res<Time>,
    mut next: ResMut<NextState<AppState>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let now = time.elapsed_secs();
    let ed = &mut *editor;
    let soldier = soldiers.single().ok();
    let anim_left = soldier.and_then(|(h, _)| h.anim_left);
    let anim_arm = soldier.and_then(|(h, rig)| h.animated_left_arm(rig?));

    egui::TopBottomPanel::top("weapon_editor_menu").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.strong("WEAPON EDITOR");
            ui.separator();
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                    ed.save(&grips, now);
                }
                if ui.add_enabled(ed.dirty(&grips), egui::Button::new("Revert to the last save")).clicked() {
                    if let Some(saved) = ed.saved.clone() {
                        *grips = saved;
                    }
                    ed.toast("Reverted to the last save", now);
                }
                ui.separator();
                if ui.add(egui::Button::new("Exit").shortcut_text("Esc")).clicked() && ed.request_exit(&grips) {
                    next.set(AppState::MainMenu);
                }
            });
            if ui.button("Help").clicked() {
                ed.help_open = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("EXIT").clicked() && ed.request_exit(&grips) {
                    next.set(AppState::MainMenu);
                }
            });
        });
    });

    egui::TopBottomPanel::bottom("weapon_editor_status").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if let Some((text, until)) = &ed.toast {
                if now < *until {
                    ui.label(text);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ed.dirty(&grips) {
                    ui.colored_label(WARN, "Unsaved changes");
                } else {
                    ui.colored_label(DIM, "Saved");
                }
            });
        });
    });

    egui::SidePanel::right("weapon_editor_panel")
        .default_width(360.0)
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                weapon_and_view(ui, ed);
                ui.separator();
                clip_controls(ui, ed);
                ui.separator();
                let WeaponGrips {
                    sniper,
                    ak74,
                    raygun,
                    knife,
                    arm_down,
                    throw_hold,
                    drink_hold,
                } = &mut *grips;
                throw_controls(ui, ed, throw_hold);
                ui.separator();
                drink_controls(ui, ed, drink_hold);
                ui.separator();
                let grip = match ed.weapon {
                    GripKey::Sniper => sniper,
                    GripKey::Ak74 => ak74,
                    GripKey::RayGun => raygun,
                    GripKey::Knife => knife,
                };
                grip_controls(ui, ed, grip, arm_down, anim_left, anim_arm);
            });
        });

    if ed.exit_prompt {
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("The grips aren't saved.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save & exit").clicked() {
                        ed.save(&grips, now);
                        next.set(AppState::MainMenu);
                    }
                    if ui.button("Exit without saving").clicked() {
                        next.set(AppState::MainMenu);
                    }
                    if ui.button("Cancel").clicked() {
                        ed.exit_prompt = false;
                    }
                });
            });
    }

    let mut open = ed.help_open;
    egui::Window::new("Controls")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Grid::new("weapon_editor_controls").striped(true).show(ui, |ui| {
                for (keys, what) in CONTROLS {
                    ui.strong(*keys);
                    ui.label(*what);
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.colored_label(
                DIM,
                "Saving writes client/data/weapon_grips.ron (and prints it to the console). A dev build reads \
                 it at launch; rebuild the client to ship it.",
            );
        });
    ed.help_open = open;
    Ok(())
}

fn weapon_and_view(ui: &mut egui::Ui, ed: &mut GripEditor) {
    ui.heading("Weapon");
    let was = ed.weapon;
    ui.horizontal_wrapped(|ui| {
        for key in GripKey::ALL {
            ui.selectable_value(&mut ed.weapon, key, key.label());
        }
    });
    // (Never the same one twice — picking the other one swaps them round.)
    if ed.also == Some(ed.weapon) {
        ed.also = Some(was);
    }
    ui.horizontal(|ui| {
        ui.label("Also carrying");
        egui::ComboBox::from_id_salt("weapon_editor_also")
            .selected_text(ed.also.map_or("Nothing", GripKey::label))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut ed.also, None, "Nothing");
                for key in GripKey::ALL.into_iter().filter(|&k| k != ed.weapon) {
                    ui.selectable_value(&mut ed.also, Some(key), key.label());
                }
            });
        if ui
            .add_enabled(ed.also.is_some(), egui::Button::new("Swap"))
            .on_hover_text("Put this one away and get the other out, as other players see it.")
            .clicked()
        {
            if let Some(other) = ed.also.replace(ed.weapon) {
                ed.weapon = other;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Pack-a-Punch camo");
        ui.add(egui::Slider::new(&mut ed.pap, 0..=shared::pap::MAX_LEVEL));
    });
    ui.checkbox(&mut ed.hands, "Hands on the weapon (as in a game)")
        .on_hover_text("Off: the arms as the animation has them, without the grip's hand moves.");
    ui.checkbox(&mut ed.show_rifle, "Show the soldier's own rifle")
        .on_hover_text("The rifle soldier.glb comes with — what the animations were made around.");
    ui.checkbox(&mut ed.show_markers, "Show markers")
        .on_hover_text("Muzzle (red), scope (white), the left hand's spot (green, with its axes), the weapon's barrel (orange) and top (blue), the right hand's axes.");
}

fn clip_controls(ui: &mut egui::Ui, ed: &mut GripEditor) {
    ui.heading("Animation");
    egui::ComboBox::from_id_salt("weapon_editor_clip")
        .selected_text(clip_label(ed.clip))
        .show_ui(ui, |ui| {
            for (state, label) in CLIPS {
                ui.selectable_value(&mut ed.clip, state, label);
            }
        });
    ui.horizontal(|ui| {
        if ui.button(if ed.playing { "Pause" } else { "Play" }).clicked() {
            ed.playing = !ed.playing;
        }
        ui.add(egui::Slider::new(&mut ed.speed, 0.05..=2.0).text("speed"));
    });
    ui.horizontal(|ui| {
        ui.add(egui::Slider::new(&mut ed.look_deg, -89.0..=89.0).text("look up / down").suffix("°"));
        if ui.small_button("Level").clicked() {
            ed.look_deg = 0.0;
        }
    });
    let length = ed.length.max(0.001);
    let mut t = ed.time.clamp(0.0, length);
    if ui
        .add(egui::Slider::new(&mut t, 0.0..=length).text("s").fixed_decimals(2))
        .changed()
    {
        ed.playing = false;
        ed.time = t;
    }
}

/// Three numbers side by side, labelled.
fn vec3_row(ui: &mut egui::Ui, label: &str, v: &mut [f32; 3], speed: f64, suffix: &str, axes: [&str; 3]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        for (value, axis) in v.iter_mut().zip(axes) {
            changed |= ui
                .add(egui::DragValue::new(value).speed(speed).prefix(format!("{axis} ")).suffix(suffix).max_decimals(4))
                .changed();
        }
    });
    changed
}

/// The throw (a lethal or tactical): what's thrown, playing it, and where
/// it sits in the hand — the same for every kind.
fn throw_controls(ui: &mut egui::Ui, ed: &mut GripEditor, hold: &mut HandTarget) {
    ui.heading("Throwing");
    ui.horizontal_wrapped(|ui| {
        for (kind, label) in [
            (LethalKind::Frag, "Frag"),
            (LethalKind::FlashBang, "Flash bang"),
            (LethalKind::MonkeyBomb, "Monkey bomb"),
            (LethalKind::Molotov, "Molotov"),
            (LethalKind::ThrowingKnife, "Throwing knife"),
        ] {
            ui.selectable_value(&mut ed.throw_kind, kind, label);
        }
    });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(ed.throw_clock.is_none(), egui::Button::new("Throw"))
            .on_hover_text("Put the weapon away, bring the arm up and throw — as other players see it.")
            .clicked()
        {
            ed.throw_clock = Some(0.0);
        }
        ui.checkbox(&mut ed.throw_ready, "Hold ready")
            .on_hover_text("The arm up and cocked back, as while a throw's cooked — to place it in the hand by.");
    });
    ui.colored_label(DIM, "In the hand: its middle from the hand bone, in its axes.");
    vec3_row(ui, "Place", &mut hold.pos, 0.001, " m", ["x", "y", "z"]);
    vec3_row(ui, "Turn", &mut hold.turn_deg, 0.25, "°", ["x", "y", "z"]);
}

/// A perk drink: which perk, playing it, holding a moment of it, and where
/// the bottle sits in the hand.
fn drink_controls(ui: &mut egui::Ui, ed: &mut GripEditor, hold: &mut HandTarget) {
    ui.heading("Drinking a perk");
    egui::ComboBox::from_id_salt("weapon_editor_perk")
        .selected_text(ed.drink_perk.label())
        .show_ui(ui, |ui| {
            for perk in Perk::ALL {
                ui.selectable_value(&mut ed.drink_perk, perk, perk.label());
            }
        });
    ui.horizontal(|ui| {
        if ui
            .add_enabled(ed.drink_clock.is_none(), egui::Button::new("Drink"))
            .on_hover_text("Put the weapon away and drink — as other players see it, as long as ours takes.")
            .clicked()
        {
            ed.drink_clock = Some(0.0);
        }
        let mut holding = ed.drink_hold_at.is_some();
        if ui
            .checkbox(&mut holding, "Hold")
            .on_hover_text("Stop on one moment of the drink — to place the bottle in the hand by.")
            .changed()
        {
            ed.drink_hold_at = holding.then_some(0.5);
        }
        if let Some(at) = ed.drink_hold_at.as_mut() {
            ui.add(egui::Slider::new(at, 0.0..=1.0).fixed_decimals(2));
        }
    });
    ui.colored_label(DIM, "In the hand: its middle from the hand bone, in its axes (neck up its +y).");
    vec3_row(ui, "Place", &mut hold.pos, 0.001, " m", ["x", "y", "z"]);
    vec3_row(ui, "Turn", &mut hold.turn_deg, 0.25, "°", ["x", "y", "z"]);
}

/// Where the left arm goes ([`grip_controls`]).
#[derive(Clone, Copy, PartialEq)]
enum LeftArm {
    Animated,
    OnWeapon,
    Down,
}

fn grip_controls(
    ui: &mut egui::Ui,
    ed: &mut GripEditor,
    grip: &mut Grip,
    arm_down: &mut ArmPose,
    anim_left: Option<HandTarget>,
    anim_arm: Option<ArmPose>,
) {
    const XYZ: [&str; 3] = ["x", "y", "z"];

    ui.heading("In the right hand");
    ui.colored_label(DIM, "The weapon's middle from the hand bone, in its axes (the markers' red / green / blue).");
    vec3_row(ui, "Place", &mut grip.offset, 0.001, " m", XYZ);
    vec3_row(ui, "Turn", &mut grip.turn_deg, 0.25, "°", XYZ);
    ui.horizontal(|ui| {
        ui.label("Size");
        ui.add(egui::DragValue::new(&mut grip.scale).speed(0.002).range(0.2..=3.0).max_decimals(3));
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Line up with the soldier's rifle")
            .on_hover_text("Put its middle where the rifle's is, its barrel along the rifle's.")
            .clicked()
        {
            grip.offset = BAKED_GUN_CENTER;
            grip.turn_deg = BAKED_GUN_TURN_DEG;
        }
        if ui.button("Flip front / back").clicked() {
            grip.turn_deg = to_euler_deg(grip.rotation() * Quat::from_rotation_y(PI));
        }
        if ui.button("Flip upside down").clicked() {
            grip.turn_deg = to_euler_deg(grip.rotation() * Quat::from_rotation_x(PI));
        }
        if ui.button("Roll 90°").clicked() {
            grip.turn_deg = to_euler_deg(grip.rotation() * Quat::from_rotation_x(FRAC_PI_2));
        }
    });

    ui.separator();
    ui.heading("Right hand moved");
    ui.colored_label(DIM, "From where the animation has it, weapon and all — e.g. a pistol held out in front.");
    vec3_row(ui, "Move", &mut grip.hand_shift, 0.002, " m", ["right", "up", "fwd"]);
    vec3_row(ui, "Turn", &mut grip.hand_turn_deg, 0.25, "°", ["pitch", "yaw", "roll"]);
    if ui.button("Reset").clicked() {
        grip.hand_shift = [0.0; 3];
        grip.hand_turn_deg = [0.0; 3];
    }

    if ed.weapon == GripKey::Knife {
        ui.separator();
        ui.heading("Stab");
        ui.colored_label(DIM, "The right hand's thrust at its furthest — on top of its move above.");
        vec3_row(ui, "Reach", &mut grip.stab_reach, 0.002, " m", ["right", "up", "fwd"]);
        vec3_row(ui, "Turn", &mut grip.stab_turn_deg, 0.25, "°", ["pitch", "yaw", "roll"]);
        vec3_row(ui, "Lean", &mut grip.stab_lean_deg, 0.25, "°", ["pitch", "yaw", "roll"]);
        ui.colored_label(DIM, "Lean: the chest, into the thrust — yaw left brings the right shoulder forward.");
        ui.horizontal(|ui| {
            if ui.button("Play stab").clicked() {
                ed.play_stab = true;
            }
            ui.checkbox(&mut ed.loop_stab, "Loop");
        });
    }

    ui.separator();
    ui.heading("Left arm");
    let was = if grip.left_arm_down {
        LeftArm::Down
    } else if grip.left_hand.is_some() {
        LeftArm::OnWeapon
    } else {
        LeftArm::Animated
    };
    let mut mode = was;
    ui.horizontal(|ui| {
        ui.radio_value(&mut mode, LeftArm::Animated, "As animated");
        ui.radio_value(&mut mode, LeftArm::OnWeapon, "Hand on the weapon");
        ui.radio_value(&mut mode, LeftArm::Down, "Down by the side");
    });
    if mode != was {
        grip.left_arm_down = mode == LeftArm::Down;
        match mode {
            LeftArm::Animated => grip.left_hand = None,
            LeftArm::OnWeapon if grip.left_hand.is_none() => {
                grip.left_hand = Some(anim_left.unwrap_or(HandTarget { pos: [-0.3, -0.03, 0.0], turn_deg: [0.0; 3] }));
            }
            _ => {}
        }
    }
    match mode {
        LeftArm::Animated => {
            ui.colored_label(DIM, "Where the animation has it.");
        }
        LeftArm::OnWeapon => {
            if let Some(left) = grip.left_hand.as_mut() {
                ui.colored_label(DIM, "The wrist, in the weapon's frame: barrel along -x, sights up +y.");
                vec3_row(ui, "Place", &mut left.pos, 0.001, " m", XYZ);
                vec3_row(ui, "Turn", &mut left.turn_deg, 0.25, "°", XYZ);
            }
            if ui
                .add_enabled(anim_left.is_some(), egui::Button::new("Snap to where the animation has it now"))
                .on_hover_text("Pause on a moment where the hand's right (Idle or Aiming, with the soldier's rifle shown to judge), then snap.")
                .clicked()
            {
                grip.left_hand = anim_left;
            }
        }
        LeftArm::Down => {
            ui.colored_label(
                DIM,
                "As in the \"Idle, no gun\" clip — one pose, shared by every weapon with its left arm down.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(anim_arm.is_some(), egui::Button::new("Take the pose from this frame"))
                    .on_hover_text("The left arm (and collarbone) as the clip playing has it now.")
                    .clicked()
                {
                    if let Some(pose) = anim_arm {
                        *arm_down = pose;
                    }
                }
                if ui
                    .add_enabled(*arm_down != ArmPose::IDLE_NO_GUN, egui::Button::new("Back to \"Idle, no gun\""))
                    .clicked()
                {
                    *arm_down = ArmPose::IDLE_NO_GUN;
                }
            });
        }
    }

    ui.separator();
    ui.heading("Carried");
    ui.colored_label(DIM, "Where it goes while it isn't in hand — from the bone, in its axes.");
    ui.horizontal(|ui| {
        ui.radio_value(&mut grip.stow_on, StowOn::Back, "On the back");
        ui.radio_value(&mut grip.stow_on, StowOn::RightThigh, "On the right thigh");
    });
    vec3_row(ui, "Place", &mut grip.stow_offset, 0.001, " m", XYZ);
    vec3_row(ui, "Turn", &mut grip.stow_turn_deg, 0.25, "°", XYZ);
    ui.horizontal_wrapped(|ui| {
        let carried = euler_deg(grip.stow_turn_deg);
        if ui.button("Flip front / back").clicked() {
            grip.stow_turn_deg = to_euler_deg(carried * Quat::from_rotation_y(PI));
        }
        if ui.button("Flip upside down").clicked() {
            grip.stow_turn_deg = to_euler_deg(carried * Quat::from_rotation_x(PI));
        }
        if ui.button("Roll 90°").clicked() {
            grip.stow_turn_deg = to_euler_deg(carried * Quat::from_rotation_x(FRAC_PI_2));
        }
    });

    ui.separator();
    ui.heading("Muzzle and scope");
    ui.colored_label(DIM, "In the weapon's frame. Shots' tracers start at the muzzle; the sniper glint shows on the scope.");
    vec3_row(ui, "Muzzle", &mut grip.muzzle, 0.001, " m", XYZ);
    let mut scoped = grip.scope.is_some();
    if ui.checkbox(&mut scoped, "Scope (glints)").changed() {
        grip.scope = scoped.then_some([0.0, 0.08, 0.0]);
    }
    if let Some(scope) = grip.scope.as_mut() {
        vec3_row(ui, "Scope", scope, 0.001, " m", XYZ);
    }
}
