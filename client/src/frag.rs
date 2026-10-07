//! Frags, client side (`Zombies` only — the server owns the flight, the
//! fuse, the blast and the drops; see `server::frags` and `shared::frag`):
//!
//! * **Held** — with the frag as the lethal (`Weapon::lethal`), pressing the
//!   lethal key pulls the pin (its sound for us alone, not positional) and
//!   brings up the throwing arms holding `models/weapons/frag_4k.glb`
//!   ([`HeldFrag`], a child of the arms). There's no cancelling it, and its
//!   fuse runs from the press: releasing throws it with what's left, holding
//!   too long sets it off in the hand (`weapon.rs`, sent by
//!   [`send_throw_requests`]).
//! * **Thrown** — every player sees each [`ThrownFrag`] bounce and roll as
//!   `models/weapons/frag_1k.glb` ([`FragAvatar`]); when it goes off the
//!   server's blast message shows the Bomb Shot explosion with the frag's
//!   own sound, from there (`vfx::explosion`).
//! * **Dropped** — a zombie's dropped frag lies where it fell, outlined like
//!   a dropped molotov ([`DropAvatar`]); the pickup card and key are
//!   `knife_pickup`'s ([`receive_pickups`]).
//!
//! Its look in the hand and in the world is tuned in the debug panel's "Frag
//! (Zombies)" section ([`frag_section`]), which can also show one in the
//! hand and hold the arms up as if the key were held, without pulling a pin.
//!
//! Every entity here is `StateScoped(InGame)` or a child of one (the held
//! frag rides the arms rig), each avatar goes as soon as its server entity
//! does, and the frag count lives on `Weapon` — nothing carries into the
//! next game.

use bevy::prelude::*;
use bevy::render::view::RenderLayers;
use bevy_egui::egui;
use lightyear::prelude::*;
use shared::{FragDrop, ThrownFrag};

use crate::killcam::ActiveKillCam;
use crate::knife_pickup::OutlineWhenLoaded;
use crate::net::GameClient;
use crate::{start_throw_knife_model, AppState, GameSounds, ThrowArmsViewModel, ThrowingKnife, Weapon, VIEW_MODEL_RENDER_LAYER};

/// The one in hand, and the one everyone sees thrown or dropped.
pub(crate) const HELD_MODEL: &str = "models/weapons/frag_4k.glb";
pub(crate) const WORLD_MODEL: &str = "models/weapons/frag_1k.glb";
/// `frag_*.glb` as made: its middle and height (model units — it's ~125
/// tall, standing up `+Y`, well off its origin).
const MODEL_MIDDLE: Vec3 = Vec3::new(-0.743, 62.58, 78.513);
const MODEL_HEIGHT: f32 = 125.05;
/// How long (m) it is in the Mystery Box.
pub(crate) const BOX_LENGTH: f32 = 0.16;

/// The model `height` m tall, its middle on the origin.
pub(crate) fn centered_transform(height: f32) -> Transform {
    let scale = height / MODEL_HEIGHT;
    Transform::from_translation(-MODEL_MIDDLE * scale).with_scale(Vec3::splat(scale))
}

/// The model `height` m tall, standing with its base on the origin.
fn standing_transform(height: f32) -> Transform {
    let mut t = centered_transform(height);
    t.translation.y += height * 0.5;
    t
}

pub(crate) struct FragPlugin;

impl Plugin for FragPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FragSettings>()
            .add_systems(Startup, setup_assets)
            // (The debug poses are the panel's — off again on leaving.)
            .add_systems(OnExit(AppState::InGame), |mut s: ResMut<FragSettings>| {
                s.debug_arms_out = false;
                s.debug_show_in_hand = false;
            })
            .add_systems(
                Update,
                (
                    spawn_held_frag,
                    update_held_frag,
                    send_throw_requests,
                    sync_frag_avatars,
                    sync_drop_avatars,
                    receive_pickups,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The debug panel's frag tuning.
#[derive(Resource, Clone)]
pub(crate) struct FragSettings {
    // --- held in the throwing arms (the arms' local space, like the
    // molotov's: one unit = the arms' scale in metres, ~cm) — the frag's
    // middle is where this puts it ---
    pub(crate) held_translation: Vec3,
    pub(crate) held_yaw: f32,
    pub(crate) held_pitch: f32,
    pub(crate) held_roll: f32,
    /// Model units to arm units (it's ~125 model units tall).
    pub(crate) held_scale: f32,
    /// How tall (m) it is thrown...
    pub(crate) world_height: f32,
    /// ...and dropped, waiting to be picked up.
    pub(crate) drop_height: f32,

    // --- debug poses (never saved; off on leaving the game) ---
    /// Show a frag in the arms' hand whenever they're up.
    pub(crate) debug_show_in_hand: bool,
    /// Hold the arms up as if the lethal key were held — without pulling a
    /// pin or throwing anything (`weapon.rs`' `ThrowingKnife::dry`).
    pub(crate) debug_arms_out: bool,
}

impl Default for FragSettings {
    fn default() -> Self {
        Self {
            held_translation: Vec3::new(-13.0, -8.0, -3.0),
            held_yaw: 135.0,
            held_pitch: 30.0,
            held_roll: 25.0,
            held_scale: 0.09,
            world_height: 0.11,
            drop_height: 0.11,
            debug_show_in_hand: false,
            debug_arms_out: false,
        }
    }
}

impl FragSettings {
    fn held_transform(&self) -> Transform {
        Transform {
            translation: self.held_translation,
            rotation: Quat::from_euler(
                EulerRot::YXZ,
                self.held_yaw.to_radians(),
                self.held_pitch.to_radians(),
                self.held_roll.to_radians(),
            ),
            scale: Vec3::splat(self.held_scale.max(1e-4)),
        }
    }
}

#[derive(Resource)]
struct FragAssets {
    /// Loaded up front (and kept) so the first one picked up or thrown
    /// doesn't hitch.
    held: Handle<Scene>,
    world: Handle<Scene>,
}

fn setup_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(FragAssets {
        held: asset_server.load(GltfAssetLabel::Scene(0).from_asset(HELD_MODEL)),
        world: asset_server.load(GltfAssetLabel::Scene(0).from_asset(WORLD_MODEL)),
    });
}

// --- held --------------------------------------------------------------------

/// The frag in the throwing arms' hand (its pivot: the frag's middle).
#[derive(Component)]
struct HeldFrag;

/// Give each (new) throwing-arms rig its frag, on the view-model layer.
fn spawn_held_frag(
    arms: Query<Entity, Added<ThrowArmsViewModel>>,
    settings: Res<FragSettings>,
    assets: Res<FragAssets>,
    mut commands: Commands,
) {
    for arms in &arms {
        let vm = RenderLayers::layer(VIEW_MODEL_RENDER_LAYER);
        commands
            .spawn((HeldFrag, settings.held_transform(), vm.clone(), Visibility::Hidden, ChildOf(arms)))
            .with_children(|pivot| {
                pivot
                    .spawn((SceneRoot(assets.held.clone()), Transform::from_translation(-MODEL_MIDDLE), vm))
                    .observe(start_throw_knife_model);
            });
    }
}

/// Pose the held frag from the settings every frame and show it while it's
/// in the hand — from the pin-pull until it's thrown — or whenever the
/// debug panel says. Hidden during a kill cam.
fn update_held_frag(
    settings: Res<FragSettings>,
    knife: Res<ThrowingKnife>,
    killcam: Res<ActiveKillCam>,
    mut held: Query<(&mut Transform, &mut Visibility), With<HeldFrag>>,
) {
    let shown = (knife.frag_in_hand() || settings.debug_show_in_hand) && killcam.0.is_none();
    for (mut tf, mut vis) in &mut held {
        tf.set_if_neq(settings.held_transform());
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// Send the frag throw (or cook-off) `weapon_system` filed.
fn send_throw_requests(
    mut knife: ResMut<ThrowingKnife>,
    mut sender: Query<&mut TriggerSender<shared::ThrowFrag>, With<GameClient>>,
) {
    if !knife.has_frag_request() {
        return;
    }
    let Some((origin, dir, fuse, in_hand)) = knife.take_frag_request() else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::ThrowFrag {
            origin: origin.to_array(),
            dir: dir.to_array(),
            fuse,
            in_hand,
        });
    }
}

// --- thrown --------------------------------------------------------------------------

/// Stands in for one server-owned [`ThrownFrag`].
#[derive(Component)]
struct FragAvatar {
    src: Entity,
}

/// The avatar's model (scaled, its middle on the avatar's origin).
#[derive(Component)]
struct FragModel;

/// Keep one avatar per frag, following the interpolated flight; drop it
/// once the server removes it (it's gone off). Hidden during a kill cam.
#[allow(clippy::too_many_arguments)]
fn sync_frag_avatars(
    frags: Query<(Entity, &ThrownFrag), With<Interpolated>>,
    all: Query<&ThrownFrag>,
    mut avatars: Query<(Entity, &FragAvatar, &mut Transform, &mut Visibility), Without<FragModel>>,
    mut models: Query<&mut Transform, (With<FragModel>, Without<FragAvatar>)>,
    killcam: Res<ActiveKillCam>,
    settings: Res<FragSettings>,
    assets: Res<FragAssets>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() { Visibility::Hidden } else { Visibility::Inherited };
    let mut have = Vec::new();
    for (entity, avatar, mut tf, mut vis) in &mut avatars {
        match all.get(avatar.src) {
            Ok(f) => {
                tf.translation = f.pos;
                tf.rotation = f.rot;
                vis.set_if_neq(wanted);
                have.push(avatar.src);
            }
            Err(_) => commands.entity(entity).try_despawn(),
        }
    }
    for mut tf in &mut models {
        tf.set_if_neq(centered_transform(settings.world_height));
    }
    for (src, f) in &frags {
        if have.contains(&src) {
            continue;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            FragAvatar { src },
            Transform::from_translation(f.pos).with_rotation(f.rot),
            Visibility::default(),
            children![(FragModel, SceneRoot(assets.world.clone()), centered_transform(settings.world_height))],
        ));
    }
}

// --- dropped -------------------------------------------------------------------

/// Stands in for one dropped [`FragDrop`], outlined like a dropped molotov.
#[derive(Component)]
struct DropAvatar {
    src: Entity,
}

/// A dropped frag's model (under its [`DropAvatar`]).
#[derive(Component)]
struct DroppedModel;

/// One avatar per dropped frag, standing where it fell; gone with it.
fn sync_drop_avatars(
    drops: Query<(Entity, &FragDrop)>,
    mut avatars: Query<(Entity, &DropAvatar, &mut Visibility)>,
    mut models: Query<&mut Transform, With<DroppedModel>>,
    killcam: Res<ActiveKillCam>,
    settings: Res<FragSettings>,
    assets: Res<FragAssets>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() { Visibility::Hidden } else { Visibility::Inherited };
    for mut tf in &mut models {
        tf.set_if_neq(standing_transform(settings.drop_height));
    }
    let mut have = Vec::new();
    for (entity, avatar, mut vis) in &mut avatars {
        if drops.contains(avatar.src) {
            vis.set_if_neq(wanted);
            have.push(avatar.src);
        } else {
            commands.entity(entity).try_despawn();
        }
    }
    for (src, d) in &drops {
        if have.contains(&src) {
            continue;
        }
        commands.spawn((
            StateScoped(AppState::InGame),
            DropAvatar { src },
            OutlineWhenLoaded,
            Transform::from_translation(d.pos).with_rotation(Quat::from_rotation_y(d.yaw)),
            Visibility::default(),
            children![(DroppedModel, SceneRoot(assets.world.clone()), standing_transform(settings.drop_height))],
        ));
    }
}

/// The server handed us a frag we picked up: it's now the only lethal, and
/// the pickup sound plays — just for us.
fn receive_pickups(
    mut receivers: Query<&mut MessageReceiver<shared::FragPickedUp>>,
    mut weapon: ResMut<Weapon>,
    mut pending: ResMut<crate::knife_pickup::PendingPickup>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            weapon.add_frag();
            pending.0 = None;
            commands.spawn((AudioPlayer::new(sounds.pick_up_equipment.clone()), PlaybackSettings::DESPAWN));
        }
    }
}

// --- debug panel -------------------------------------------------------------------

/// The debug panel's "Frag (Zombies)" section: give yourself some, pose the
/// arms and a frag in them (without pulling a pin — holding the real key
/// would set it off in the hand), and its look in the hand and the world.
pub(crate) fn frag_section(ui: &mut egui::Ui, s: &mut FragSettings, weapon: &mut Weapon) {
    ui.horizontal(|ui| {
        if ui.button("Give me a frag").clicked() {
            weapon.add_frag();
        }
        ui.label(format!("carrying {}", weapon.frags));
    });
    ui.checkbox(&mut s.debug_show_in_hand, "Show a frag in the hand");
    ui.checkbox(
        &mut s.debug_arms_out,
        "Hold the throwing arms up (as if the lethal key were held — no pin pulled)",
    );
    ui.label(format!(
        "Fuse: {:.1} s from the pin coming out. Holding the real lethal key cooks it — too long and it goes off in your hand.",
        shared::frag::FUSE_SECS
    ));
    ui.collapsing("Held frag (4k model)", |ui| {
        ui.label(
            "Positioned in the throwing arms' local space, like the molotov: one unit = the arms' scale in \
             metres (~cm by default). The position is the frag's middle. Tick both boxes above to see it.",
        );
        ui.add(egui::Slider::new(&mut s.held_translation.x, -100.0f32..=100.0).text("x"));
        ui.add(egui::Slider::new(&mut s.held_translation.y, -100.0f32..=100.0).text("y"));
        ui.add(egui::Slider::new(&mut s.held_translation.z, -100.0f32..=100.0).text("z"));
        ui.add(egui::Slider::new(&mut s.held_yaw, -180.0f32..=180.0).text("yaw (°)"));
        ui.add(egui::Slider::new(&mut s.held_pitch, -180.0f32..=180.0).text("pitch (°)"));
        ui.add(egui::Slider::new(&mut s.held_roll, -180.0f32..=180.0).text("roll (°)"));
        ui.add(egui::Slider::new(&mut s.held_scale, 0.005f32..=1.0).text("scale").logarithmic(true));
    });
    ui.collapsing("Thrown / dropped (1k model)", |ui| {
        ui.add(egui::Slider::new(&mut s.world_height, 0.03f32..=0.5).text("thrown height (m)"));
        ui.add(egui::Slider::new(&mut s.drop_height, 0.03f32..=0.5).text("dropped height (m)"));
    });
    if ui.button("Copy settings to console").clicked() {
        info!(
            "frag: held_translation: Vec3::new({:.2}, {:.2}, {:.2}), held_yaw: {:.1}, held_pitch: {:.1}, \
             held_roll: {:.1}, held_scale: {:.4}, world_height: {:.3}, drop_height: {:.3}",
            s.held_translation.x,
            s.held_translation.y,
            s.held_translation.z,
            s.held_yaw,
            s.held_pitch,
            s.held_roll,
            s.held_scale,
            s.world_height,
            s.drop_height,
        );
    }
    if ui.button("Reset frag").clicked() {
        let poses = (s.debug_show_in_hand, s.debug_arms_out);
        *s = FragSettings::default();
        (s.debug_show_in_hand, s.debug_arms_out) = poses;
    }
}
