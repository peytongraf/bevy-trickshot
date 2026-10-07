//! Monkey bombs, client side (`Zombies` only — the server owns the flight,
//! the fuse, the lure and the drops; see `server::monkey_bombs` and
//! `shared::monkey_bomb`):
//!
//! * **Held** — with the monkey bomb as the tactical (`Weapon::tactical`),
//!   holding the tactical key brings up the throwing arms holding
//!   `models/weapons/monkey_bomb.glb` ([`HeldMonkey`], a child of the arms),
//!   still — its clip doesn't play in the hand. Releasing primes it: the
//!   prime sound plays for us alone and we keep hold of it until that's done
//!   (`weapon.rs`'s `ThrowPhase::Priming`), then it's thrown
//!   ([`send_throw_requests`]).
//! * **Thrown / landed** — every player sees each [`ThrownMonkey`] fly
//!   ([`MonkeyAvatar`]) and hears it thrown; once it's down it plays its
//!   clip (spinning, bouncing, clapping its cymbals) on a loop, and its land
//!   sound, its song and then its "bye bye zombies" play from it, for the
//!   whole lobby ([`BoxSound`]-style distance fade, [`fade_monkey_sounds`]).
//!   The explosion is the server's Bomb Shot blast (`vfx::explosion`).
//! * **Dropped** — a zombie's dropped monkey bomb stands where it fell,
//!   outlined like a dropped molotov ([`DropAvatar`]); the pickup card and
//!   key are `knife_pickup`'s ([`receive_pickups`]).
//!
//! Its look in the hand and in the world is tuned in the debug panel's
//! "Monkey bomb (Zombies)" section ([`monkey_section`]).
//!
//! Every entity here is `StateScoped(InGame)` or a child of one (the held
//! monkey rides the arms rig), each avatar and its sounds go as soon as its
//! server entity does, and the monkey count lives on `Weapon` — nothing
//! carries into the next game.

use bevy::prelude::*;
use bevy::render::view::RenderLayers;
use bevy_egui::egui;
use lightyear::prelude::*;
use shared::monkey_bomb::{
    FUSE_SECS, LAND_SECS, MAX_FUSE_SECS, MIN_FUSE_SECS, PRIME_SECS, PRIME_SOUND_SECS, SONG_DELAY_SECS, VOX_SECS,
};
use shared::{MonkeyDrop, ThrownMonkey};

use crate::killcam::ActiveKillCam;
use crate::knife_pickup::OutlineWhenLoaded;
use crate::net::GameClient;
use crate::{
    start_throw_knife_model, AppState, GameSounds, ThrowArmsViewModel, ThrowingKnife, Weapon, VIEW_MODEL_RENDER_LAYER,
};

pub(crate) const MONKEY_MODEL: &str = "models/weapons/monkey_bomb.glb";
/// `monkey_bomb.glb` as made: how tall it is, and how far its base sits above
/// its origin (model units).
const MODEL_HEIGHT: f32 = 0.497;
const MODEL_BASE: f32 = 0.006;
/// How tall (m) it stands in the Mystery Box.
pub(crate) const BOX_HEIGHT: f32 = 0.3;

/// The model standing `height` m tall, its base on the origin.
pub(crate) fn standing_transform(height: f32) -> Transform {
    let scale = height / MODEL_HEIGHT;
    Transform::from_translation(Vec3::NEG_Y * MODEL_BASE * scale).with_scale(Vec3::splat(scale))
}

/// The model standing `height` m tall, its middle on the origin.
pub(crate) fn centered_transform(height: f32) -> Transform {
    let mut t = standing_transform(height);
    t.translation.y -= height * 0.5;
    t
}

pub(crate) struct MonkeyBombPlugin;

impl Plugin for MonkeyBombPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MonkeyBombSettings>()
            .add_systems(Startup, setup_assets)
            .add_systems(
                Update,
                (
                    spawn_held_monkey,
                    update_held_monkey,
                    send_throw_requests,
                    sync_monkey_avatars,
                    play_landed_monkeys,
                    fade_monkey_sounds,
                    sync_drop_avatars,
                    receive_pickups,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The debug panel's monkey bomb tuning.
#[derive(Resource, Clone)]
pub(crate) struct MonkeyBombSettings {
    // --- held in the throwing arms (the arms' local space, like the
    // molotov's: one unit = the arms' scale in metres) ---
    pub(crate) held_translation: Vec3,
    pub(crate) held_yaw: f32,
    pub(crate) held_pitch: f32,
    pub(crate) held_roll: f32,
    pub(crate) held_scale: f32,
    /// How tall (m) it stands thrown and landed...
    pub(crate) world_height: f32,
    /// ...and dropped, waiting to be picked up.
    pub(crate) drop_height: f32,
    /// Its clip's speed once it's landed (1 = as made).
    pub(crate) anim_speed: f32,

    // --- timing ---
    /// How long (s) it's held, primed, after the tactical key's released,
    /// before it's thrown (`weapon.rs`' `ThrowPhase::Priming`).
    pub(crate) prime_secs: f32,
    /// How long (s) after it lands its song starts (its land sound plays at
    /// once).
    pub(crate) song_delay_secs: f32,
    /// How long (s) before it blows up its "bye bye zombies" starts (over
    /// the song, which plays on till the end).
    pub(crate) vox_lead_secs: f32,
    /// How long (s) from landing to blowing up — the server's, for every
    /// monkey thrown in our lobby from now on (`SetMonkeyFuse`).
    pub(crate) fuse_secs: f32,
}

impl Default for MonkeyBombSettings {
    fn default() -> Self {
        Self {
            held_translation: Vec3::new(-15.0, -18.0, 0.0),
            held_yaw: 70.0,
            held_pitch: 20.0,
            held_roll: -5.0,
            held_scale: 70.0,
            world_height: 0.6,
            drop_height: 0.3,
            anim_speed: 1.0,
            prime_secs: PRIME_SECS,
            song_delay_secs: SONG_DELAY_SECS,
            vox_lead_secs: VOX_SECS,
            fuse_secs: FUSE_SECS,
        }
    }
}

impl MonkeyBombSettings {
    fn held_transform(&self) -> Transform {
        Transform {
            translation: self.held_translation,
            rotation: Quat::from_euler(
                EulerRot::YXZ,
                self.held_yaw.to_radians(),
                self.held_pitch.to_radians(),
                self.held_roll.to_radians(),
            ),
            scale: Vec3::splat(self.held_scale),
        }
    }
}

#[derive(Resource)]
struct MonkeyAssets {
    /// Loaded up front (and kept) so the first one picked up or thrown
    /// doesn't hitch.
    model: Handle<Scene>,
    graph: Handle<AnimationGraph>,
    clip: AnimationNodeIndex,
}

fn setup_assets(mut commands: Commands, asset_server: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    let (graph, clip) = AnimationGraph::from_clip(asset_server.load(GltfAssetLabel::Animation(0).from_asset(MONKEY_MODEL)));
    commands.insert_resource(MonkeyAssets {
        model: asset_server.load(GltfAssetLabel::Scene(0).from_asset(MONKEY_MODEL)),
        graph: graphs.add(graph),
        clip,
    });
}

// --- held --------------------------------------------------------------------

/// The monkey bomb in the throwing arms' hand — shown only while
/// `ThrowingKnife::monkey_in_hand`. Its clip never plays here.
#[derive(Component)]
struct HeldMonkey;

/// Give each (new) throwing-arms rig its monkey bomb, on the view-model layer.
fn spawn_held_monkey(
    arms: Query<Entity, Added<ThrowArmsViewModel>>,
    settings: Res<MonkeyBombSettings>,
    assets: Res<MonkeyAssets>,
    mut commands: Commands,
) {
    for arms in &arms {
        let vm = RenderLayers::layer(VIEW_MODEL_RENDER_LAYER);
        commands
            .spawn((HeldMonkey, settings.held_transform(), vm.clone(), Visibility::Hidden, ChildOf(arms)))
            .with_children(|pivot| {
                pivot
                    .spawn((SceneRoot(assets.model.clone()), Transform::default(), vm))
                    .observe(start_throw_knife_model);
            });
    }
}

/// Pose the held monkey from the settings every frame and show it only
/// while it's in the hand (from the press, through its prime, until the
/// throw). Hidden during a kill cam.
fn update_held_monkey(
    settings: Res<MonkeyBombSettings>,
    knife: Res<ThrowingKnife>,
    killcam: Res<ActiveKillCam>,
    mut held: Query<(&mut Transform, &mut Visibility), With<HeldMonkey>>,
) {
    let shown = knife.monkey_in_hand() && killcam.0.is_none();
    for (mut tf, mut vis) in &mut held {
        tf.set_if_neq(settings.held_transform());
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// Send the monkey bomb throw `weapon_system` filed when it left the hand.
fn send_throw_requests(
    mut knife: ResMut<ThrowingKnife>,
    mut sender: Query<&mut TriggerSender<shared::ThrowMonkey>, With<GameClient>>,
) {
    if !knife.has_monkey_request() {
        return;
    }
    let Some((origin, dir)) = knife.take_throw_request() else {
        return;
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::ThrowMonkey {
            origin: origin.to_array(),
            dir: dir.to_array(),
        });
    }
}

// --- thrown and landed -----------------------------------------------------------

/// Stands in for one server-owned [`ThrownMonkey`]: flying, then once it's
/// landed, playing its clip and its sounds.
#[derive(Component)]
struct MonkeyAvatar {
    src: Entity,
    /// Seconds since we saw it land (not counting pauses), once it has.
    landed: Option<f32>,
    /// Its song, and its "bye bye", have started.
    song: bool,
    vox: bool,
    /// The scene's `AnimationPlayer`, once its clip's going.
    player: Option<Entity>,
}

/// The avatar's model (scaled, its base on the avatar's origin).
#[derive(Component)]
struct MonkeyModel;

/// One of a monkey's sounds, playing from it (a child of its avatar, so it
/// goes with it) — its loudness follows our distance.
#[derive(Component, Clone, Copy)]
enum MonkeySound {
    Throw,
    Land,
    Song,
    Vox,
}

fn monkey_sound(kind: MonkeySound, clip: Handle<AudioSource>) -> impl Bundle {
    (
        kind,
        // Volume is ours to set (`fade_monkey_sounds`), not the one-shot
        // volume pass's.
        crate::RemoteSoundEmitter,
        AudioPlayer::new(clip),
        Transform::from_translation(Vec3::Y * 0.15),
        // Starts silent; the real volume is set from the next frame.
        crate::positional_playback(bevy::audio::Volume::Linear(0.0)),
    )
}

/// Keep one avatar per monkey bomb, following the interpolated flight; drop
/// it once the server removes it (it's blown up). Its throw sound plays from
/// it as it appears. Hidden during a kill cam.
#[allow(clippy::too_many_arguments)]
fn sync_monkey_avatars(
    monkeys: Query<(Entity, &ThrownMonkey), With<Interpolated>>,
    all: Query<&ThrownMonkey>,
    mut avatars: Query<(Entity, &MonkeyAvatar, &mut Transform, &mut Visibility), Without<MonkeyModel>>,
    mut models: Query<&mut Transform, With<MonkeyModel>>,
    killcam: Res<ActiveKillCam>,
    settings: Res<MonkeyBombSettings>,
    assets: Res<MonkeyAssets>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    let wanted = if killcam.0.is_some() { Visibility::Hidden } else { Visibility::Inherited };
    let mut have = Vec::new();
    for (entity, avatar, mut tf, mut vis) in &mut avatars {
        match all.get(avatar.src) {
            Ok(m) => {
                tf.translation = m.pos;
                tf.rotation = m.rot;
                vis.set_if_neq(wanted);
                have.push(avatar.src);
            }
            Err(_) => commands.entity(entity).try_despawn(),
        }
    }
    for mut tf in &mut models {
        tf.set_if_neq(standing_transform(settings.world_height));
    }
    for (src, m) in &monkeys {
        if have.contains(&src) {
            continue;
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                MonkeyAvatar {
                    src,
                    landed: None,
                    song: false,
                    vox: false,
                    player: None,
                },
                Transform::from_translation(m.pos).with_rotation(m.rot),
                Visibility::default(),
            ))
            .with_children(|a| {
                a.spawn((MonkeyModel, SceneRoot(assets.model.clone()), standing_transform(settings.world_height)));
                // (One that's already down — joined late — makes no throw.)
                if !m.landed {
                    a.spawn(monkey_sound(MonkeySound::Throw, sounds.monkey_bomb_throw.clone()));
                }
            });
    }
}

/// A monkey that's landed: its clip on a loop (spinning, bouncing,
/// clapping), and its land sound, then its song, then its "bye bye zombies"
/// just before its fuse runs out ([`ThrownMonkey::fuse`]), over the song,
/// which plays on until the server blows it up (and the avatar, its sounds
/// with it, goes).
#[allow(clippy::too_many_arguments)]
fn play_landed_monkeys(
    time: Res<Time>,
    paused: Res<crate::pause::GamePaused>,
    all: Query<&ThrownMonkey>,
    settings: Res<MonkeyBombSettings>,
    assets: Res<MonkeyAssets>,
    sounds: Res<GameSounds>,
    mut avatars: Query<(Entity, &mut MonkeyAvatar)>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    mut commands: Commands,
) {
    let dt = if paused.0 { 0.0 } else { time.delta_secs() };
    for (entity, mut avatar) in &mut avatars {
        let Ok(m) = all.get(avatar.src) else { continue };
        if !m.landed {
            continue;
        }
        let Some(secs) = avatar.landed else {
            avatar.landed = Some(0.0);
            commands
                .entity(entity)
                .with_child(monkey_sound(MonkeySound::Land, sounds.monkey_bomb_land.clone()));
            continue;
        };
        let secs = secs + dt;
        avatar.landed = Some(secs);
        let vox_at = (m.fuse - settings.vox_lead_secs).max(0.0);
        if !avatar.song && secs >= settings.song_delay_secs && secs < vox_at {
            avatar.song = true;
            commands
                .entity(entity)
                .with_child(monkey_sound(MonkeySound::Song, sounds.monkey_bomb_song.clone()));
        }
        if !avatar.vox && secs >= vox_at {
            avatar.vox = true;
            commands
                .entity(entity)
                .with_child(monkey_sound(MonkeySound::Vox, sounds.monkey_bomb_explode_vox.clone()));
        }
        // Its clip, once the scene's spawned its `AnimationPlayer`.
        match avatar.player.and_then(|p| players.get_mut(p).ok()) {
            Some(mut player) => {
                if let Some(clip) = player.animation_mut(assets.clip) {
                    clip.set_speed(settings.anim_speed.max(0.01));
                }
            }
            None => {
                if let Some(found) = children.iter_descendants(entity).find(|&e| players.contains(e)) {
                    let mut player = players.get_mut(found).unwrap();
                    player.play(assets.clip).repeat().set_speed(settings.anim_speed.max(0.01));
                    commands.entity(found).insert(AnimationGraphHandle(assets.graph.clone()));
                    avatar.player = Some(found);
                }
            }
        }
    }
}

/// Keep each monkey sound's loudness matched to how far we are from it.
fn fade_monkey_sounds(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    sound_vol: Res<crate::SoundVolumes>,
    remote: Res<crate::RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut sounds: Query<(&MonkeySound, &GlobalTransform, &mut SpatialAudioSink)>,
) {
    let Ok(ear) = listener.single() else { return };
    let ear = ear.translation();
    for (kind, gt, mut sink) in &mut sounds {
        let volume = match kind {
            MonkeySound::Throw => sound_vol.monkey_bomb_throw,
            MonkeySound::Land => sound_vol.monkey_bomb_land,
            MonkeySound::Song => sound_vol.monkey_bomb_song,
            MonkeySound::Vox => sound_vol.monkey_bomb_explode_vox,
        };
        let loudness = volume * crate::distance_falloff(ear.distance(gt.translation()), &remote);
        sink.set_volume(bevy::audio::Volume::Linear(loudness.max(0.0)) * global_volume.volume);
    }
}

// --- dropped -------------------------------------------------------------------

/// Stands in for one dropped [`MonkeyDrop`], outlined like a dropped molotov.
#[derive(Component)]
struct DropAvatar {
    src: Entity,
}

/// A dropped monkey's model (under its [`DropAvatar`]).
#[derive(Component)]
struct DroppedModel;

/// One avatar per dropped monkey bomb, standing where it fell; gone with it.
fn sync_drop_avatars(
    drops: Query<(Entity, &MonkeyDrop)>,
    mut avatars: Query<(Entity, &DropAvatar, &mut Visibility)>,
    mut models: Query<&mut Transform, With<DroppedModel>>,
    killcam: Res<ActiveKillCam>,
    settings: Res<MonkeyBombSettings>,
    assets: Res<MonkeyAssets>,
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
            children![(DroppedModel, SceneRoot(assets.model.clone()), standing_transform(settings.drop_height))],
        ));
    }
}

/// The server handed us a monkey bomb we picked up: it's now the only
/// tactical, and the pickup sound plays — just for us.
fn receive_pickups(
    mut receivers: Query<&mut MessageReceiver<shared::MonkeyPickedUp>>,
    mut weapon: ResMut<Weapon>,
    mut pending: ResMut<crate::knife_pickup::PendingPickup>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            weapon.add_monkey_bomb();
            pending.0 = None;
            commands.spawn((AudioPlayer::new(sounds.pick_up_equipment.clone()), PlaybackSettings::DESPAWN));
        }
    }
}

// --- debug panel -------------------------------------------------------------------

/// The debug panel's "Monkey bomb (Zombies)" section: give yourself some,
/// hold the tactical key without holding it (to pose the one in hand), and
/// its look in the hand and in the world. `hold_key` is
/// `ThrowArmsSettings::debug_hold_tactical` (it's a tactical, on that key).
pub(crate) fn monkey_section(
    ui: &mut egui::Ui,
    s: &mut MonkeyBombSettings,
    weapon: &mut Weapon,
    hold_key: &mut bool,
    fuse_tx: &mut Query<&mut TriggerSender<shared::SetMonkeyFuse>, With<GameClient>>,
) {
    ui.horizontal(|ui| {
        if ui.button("Give me a monkey bomb").clicked() {
            weapon.add_monkey_bomb();
        }
        ui.label(format!("carrying {}", weapon.monkey_bombs));
    });
    ui.checkbox(hold_key, "Hold tactical key (as if held — untick to throw)");
    ui.collapsing("Held monkey bomb", |ui| {
        ui.label(
            "Positioned in the throwing arms' local space, like the molotov: one unit = the \
             arms' scale in metres (~cm by default). Tick \"Hold tactical key\" with it as your \
             tactical to see it in the hand.",
        );
        ui.add(egui::Slider::new(&mut s.held_translation.x, -100.0f32..=100.0).text("x"));
        ui.add(egui::Slider::new(&mut s.held_translation.y, -100.0f32..=100.0).text("y"));
        ui.add(egui::Slider::new(&mut s.held_translation.z, -100.0f32..=100.0).text("z"));
        ui.add(egui::Slider::new(&mut s.held_yaw, -180.0f32..=180.0).text("yaw (°)"));
        ui.add(egui::Slider::new(&mut s.held_pitch, -180.0f32..=180.0).text("pitch (°)"));
        ui.add(egui::Slider::new(&mut s.held_roll, -180.0f32..=180.0).text("roll (°)"));
        ui.add(egui::Slider::new(&mut s.held_scale, 1.0f32..=200.0).text("scale").logarithmic(true));
    });
    ui.collapsing("Thrown / landed / dropped", |ui| {
        ui.add(egui::Slider::new(&mut s.world_height, 0.05f32..=1.5).text("thrown / landed height (m)"));
        ui.add(egui::Slider::new(&mut s.drop_height, 0.05f32..=1.5).text("dropped height (m)"));
        ui.add(egui::Slider::new(&mut s.anim_speed, 0.1f32..=3.0).text("clip speed once landed"));
    });
    ui.collapsing("Timing", |ui| {
        ui.label(format!(
            "Sounds as made: prime {PRIME_SOUND_SECS:.2} s, land {LAND_SECS:.2} s, song {:.2} s, \
             bye bye {VOX_SECS:.2} s.",
            shared::monkey_bomb::SONG_SECS
        ));
        ui.add(egui::Slider::new(&mut s.prime_secs, 0.0f32..=8.0).text("held after release, until thrown (s)"));
        ui.add(egui::Slider::new(&mut s.song_delay_secs, 0.0f32..=3.0).text("landing → song starts (s)"));
        let fuse = ui.add(
            egui::Slider::new(&mut s.fuse_secs, MIN_FUSE_SECS..=MAX_FUSE_SECS).text("landing → explosion (s, server)"),
        );
        if fuse.changed() {
            if let Ok(mut tx) = fuse_tx.single_mut() {
                tx.trigger::<shared::LobbyChannel>(shared::SetMonkeyFuse { secs: s.fuse_secs });
            }
        }
        ui.add(egui::Slider::new(&mut s.vox_lead_secs, 0.0f32..=4.0).text("\"bye bye\" starts this long before (s)"));
        ui.label("The explosion time is your lobby's on the server — it applies to monkeys thrown after you change it.");
    });
    if ui.button("Copy settings to console").clicked() {
        info!(
            "monkey bomb: held_translation: Vec3::new({:.2}, {:.2}, {:.2}), held_yaw: {:.1}, held_pitch: {:.1}, \
             held_roll: {:.1}, held_scale: {:.2}, world_height: {:.3}, drop_height: {:.3}, anim_speed: {:.2}, \
             prime_secs: {:.2}, song_delay_secs: {:.2}, vox_lead_secs: {:.2}, fuse_secs: {:.2}",
            s.held_translation.x,
            s.held_translation.y,
            s.held_translation.z,
            s.held_yaw,
            s.held_pitch,
            s.held_roll,
            s.held_scale,
            s.world_height,
            s.drop_height,
            s.anim_speed,
            s.prime_secs,
            s.song_delay_secs,
            s.vox_lead_secs,
            s.fuse_secs,
        );
    }
    if ui.button("Reset monkey bomb").clicked() {
        *s = MonkeyBombSettings::default();
    }
}
