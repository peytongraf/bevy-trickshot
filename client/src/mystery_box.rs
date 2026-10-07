//! The `Zombies` Mystery Box (`shared::mystery_box`, `models/props/mystery_box.glb`).
//! Standing at it shows a card: its cost and whether we can spin it, or —
//! once a spin of ours has settled — the prize to take. The interact key
//! spins it ([`shared::SpinMysteryBox`]) or takes the prize
//! ([`shared::TakeBoxPrize`] → [`shared::BoxPrizeTaken`]): a gun swaps for
//! the weapon in hand (dropped where we stand, like a wall buy), a lethal
//! fills us up with it.
//!
//! Everyone sees a spin, drawn from the lobby's replicated
//! [`Lobby::mystery_box`]: the lid swings open (its clip, scrubbed to the
//! lid's openness so it can run back to shut) on an amber glow fading in,
//! every prize cycles up out of it — slowing — until it settles on the one
//! rolled, which hovers there; taken, it's gone, and left, it sinks back in;
//! then the lid shuts and the glow fades. Its open, spin and close sounds
//! play from the box for the whole lobby ([`BoxSound`]), and only the one
//! who spun gets the prompt to take the prize (the server checks too).
//!
//! Where it stands is the map's layout's (placed in the level editor). The
//! box, its prize display and card are `StateScoped(InGame)` and taken away
//! whenever we're not in a `Zombies` game; the spin itself is server-owned
//! and cleared between games — nothing carries into the next one.

use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::scene::SceneInstanceReady;
use lightyear::prelude::{LocalId, MessageReceiver, TriggerSender};
use shared::mystery_box::{BoxPhase, BoxPrize, COST, HALF_EXTENTS, LID_SECS, SINK_SECS, SPIN_SECS};
use shared::weapon::SlotWeapon;
use shared::Lobby;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::weapon_drops::DropModel;
use crate::zombies_hud::{zombies_game, CARD_RED, MONEY_YELLOW};
use crate::{killcam, menu, AppState, GameSounds, Lethal, Player, Weapon, EYE_HEIGHT, HUD_FONT};

pub(crate) const MYSTERY_BOX_MODEL: &str = "models/props/mystery_box.glb";

/// The glow inside: amber.
pub(crate) const AMBER: Color = Color::srgb(1.0, 0.62, 0.15);
/// The glow light's brightness, fully faded in...
const GLOW_INTENSITY: f32 = 45_000.0;
/// ...and how long (s) it takes to fade in or out.
const GLOW_FADE_SECS: f32 = 0.6;

/// How high (m, the box as made) a prize's middle is at the bottom of the
/// box, and hovering at the top — just clear of the box, under the open lid.
const PRIZE_LOW: f32 = 0.06;
const PRIZE_HIGH: f32 = HALF_EXTENTS.y * 2.0 + 0.22;
/// Seconds into a spin the prizes start coming up (the lid part open).
const RISE_DELAY_SECS: f32 = 0.25;
/// Seconds each prize shows while cycling: this quick at first...
const CYCLE_FAST_SECS: f32 = 0.07;
/// ...slowing by up to this much by the time it settles.
const CYCLE_SLOWDOWN_SECS: f32 = 0.35;
/// How far (m) a hovering prize bobs, and how fast (rad/s).
const BOB: f32 = 0.015;
const BOB_SPEED: f32 = 2.2;

/// The lethals' sizes in the box (m, end to end).
const KNIFE_LENGTH: f32 = 0.34;
const MOLOTOV_LENGTH: f32 = 0.32;

/// Least time (s) between requests, so a double press before the server's
/// answer can't spin twice.
const PRESS_COOLDOWN_SECS: f32 = 0.6;

pub(crate) struct MysteryBoxPlugin;

impl Plugin for MysteryBoxPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(fit_lethal_model)
            .add_systems(OnEnter(AppState::InGame), spawn_box_card)
            .add_systems(
                Update,
                (
                    sync_mystery_box,
                    animate_mystery_box,
                    fade_box_sounds,
                    update_box_card,
                    use_mystery_box.run_if(menu::game_active.and(killcam::no_killcam)),
                    receive_prizes,
                )
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// The box: its lid clip and `AnimationPlayer` (once the scene's spawned
/// one), and what it's drawing of the lobby's spin.
#[derive(Component)]
pub(crate) struct MysteryBox {
    graph: Handle<AnimationGraph>,
    clip: AnimationNodeIndex,
    player: Option<Entity>,
    glow_material: Handle<StandardMaterial>,
    /// How open the lid is (0 shut, 1 open) and how bright the glow (0..1).
    lid: f32,
    /// The game's paused — its effects hold still too.
    paused: bool,
    /// Whether the lid was opening last frame — its open / close sounds
    /// play as that flips.
    was_open: bool,
    glow: f32,
    /// The spin and phase being drawn, and seconds into that phase.
    spin_id: Option<u32>,
    phase: Option<BoxPhase>,
    phase_secs: f32,
    /// The prize showing while cycling, and seconds it's shown.
    cycling: usize,
    cycle_secs: f32,
    cycle_step: u32,
}

impl MysteryBox {
    /// How open it looks (0 shut, 1 open — the glow's fade), for its effects
    /// (`vfx::mystery_box`).
    pub(crate) fn openness(&self) -> f32 {
        self.glow
    }

    /// `dt`, or nothing while the game's paused.
    pub(crate) fn frame_secs(&self, dt: f32) -> f32 {
        if self.paused { 0.0 } else { dt }
    }
}

/// One of the box's sounds, playing from it (a child, so it goes with the
/// box) — its loudness follows our distance ([`fade_box_sounds`]).
#[derive(Component, Clone, Copy)]
enum BoxSound {
    Open,
    Spin,
    Close,
}

/// The glow's light, under the box.
#[derive(Component)]
struct BoxGlowLight;

/// Where the prizes show — at real size, so not under the (scaled) box.
#[derive(Component)]
struct PrizeDisplay;

/// One prize's model, under the [`PrizeDisplay`] — only the one showing
/// is visible.
#[derive(Component)]
struct PrizeModel(BoxPrize);

/// A lethal's scene, under its [`PrizeModel`] — sized by
/// [`fit_lethal_model`] once it's in (a gun's is `weapon_drops`' upright
/// [`DropModel`]).
#[derive(Component)]
struct LethalModel {
    length: f32,
}

/// The box standing on our `Zombies` game's map, if it has one.
fn box_here(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>) -> Option<shared::level::Placement> {
    zombies_game(local, lobbies).and_then(|l| shared::mystery_box::placement(l.map))
}

/// Put the box (and its prize display) on the map while we're in a
/// `Zombies` game on a map with one, and take them away otherwise.
#[allow(clippy::too_many_arguments)]
fn sync_mystery_box(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut boxes: Query<(Entity, &mut Transform), With<MysteryBox>>,
    displays: Query<Entity, With<PrizeDisplay>>,
    mut commands: Commands,
) {
    let Some(at) = box_here(&local, &lobbies) else {
        for e in boxes.iter().map(|(e, _)| e).chain(&displays) {
            commands.entity(e).despawn();
        }
        return;
    };
    if let Some((_, mut t)) = boxes.iter_mut().next() {
        t.set_if_neq(crate::util::placed(at));
        return;
    }
    let (graph, clip) =
        AnimationGraph::from_clip(asset_server.load(GltfAssetLabel::Animation(0).from_asset(MYSTERY_BOX_MODEL)));
    let glow_material = materials.add(StandardMaterial {
        base_color: AMBER.with_alpha(0.0),
        emissive: LinearRgba::BLACK,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    let half = HALF_EXTENTS;
    commands
        .spawn((
            StateScoped(AppState::InGame),
            MysteryBox {
                graph: graphs.add(graph),
                clip,
                player: None,
                glow_material: glow_material.clone(),
                lid: 0.0,
                paused: false,
                was_open: false,
                glow: 0.0,
                spin_id: None,
                phase: None,
                phase_secs: 0.0,
                cycling: 0,
                cycle_secs: 0.0,
                cycle_step: 0,
            },
            crate::util::placed(at),
            Visibility::default(),
        ))
        .with_children(|b| {
            b.spawn(SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(MYSTERY_BOX_MODEL))));
            // Solid — the server's `shared::mystery_box::solid_box`.
            b.spawn((
                bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                Transform::from_translation(Vec3::Y * half.y),
            ));
            // The glow: a light low inside, and a sheet of it just under the
            // top, both faded in with the lid.
            b.spawn((
                BoxGlowLight,
                PointLight {
                    color: AMBER,
                    intensity: 0.0,
                    range: 4.0,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::from_translation(Vec3::Y * half.y * 1.4),
            ));
            b.spawn((
                Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::new(half.x * 0.94, half.z * 0.85)))),
                MeshMaterial3d(glow_material),
                Transform::from_translation(Vec3::Y * half.y * 1.75),
                NotShadowCaster,
            ));
        });
    commands
        .spawn((
            StateScoped(AppState::InGame),
            PrizeDisplay,
            Transform::from_translation(at.pos),
            Visibility::default(),
        ))
        .with_children(|d| {
            for prize in BoxPrize::ALL {
                let mut holder = d.spawn((PrizeModel(prize), Transform::default(), Visibility::Hidden));
                match prize.gun() {
                    Some(gun) => {
                        let weapon = SlotWeapon::Gun(gun);
                        holder.with_child((
                            DropModel {
                                weapon,
                                pap: 0,
                                upright: true,
                            },
                            SceneRoot(
                                asset_server
                                    .load(GltfAssetLabel::Scene(0).from_asset(crate::weapon_drops::model_path(weapon))),
                            ),
                            // (Hidden until it's fitted.)
                            Visibility::Hidden,
                        ));
                    }
                    // The monkey bomb: as made, upright, its middle at the
                    // origin (its skinned model isn't fitted by its shape).
                    None if prize == BoxPrize::MonkeyBomb => {
                        let height = crate::monkey_bomb::BOX_HEIGHT;
                        holder.with_child((
                            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(crate::monkey_bomb::MONKEY_MODEL))),
                            crate::monkey_bomb::centered_transform(height),
                        ));
                    }
                    None => {
                        let (path, length) = match prize {
                            BoxPrize::Molotov => ("models/weapons/molotov_1k.glb", MOLOTOV_LENGTH),
                            BoxPrize::Frag => (crate::frag::WORLD_MODEL, crate::frag::BOX_LENGTH),
                            _ => ("models/weapons/throwing_knife.glb", KNIFE_LENGTH),
                        };
                        holder.with_child((
                            LethalModel { length },
                            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path))),
                            Visibility::Hidden,
                        ));
                    }
                }
            }
        });
}

/// A lethal's scene is in: size it by its shape and stand it with its
/// middle at the origin, its longest way along +X (or up, if that's
/// it — the molotov's bottle).
fn fit_lethal_model(
    trigger: Trigger<SceneInstanceReady>,
    models: Query<&LethalModel>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    mesh_entities: Query<&Mesh3d, Without<SkinnedMesh>>,
    meshes: Res<Assets<Mesh>>,
    mut commands: Commands,
) {
    let root = trigger.target();
    let Ok(model) = models.get(root) else { return };
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for entity in children.iter_descendants(root) {
        let Ok(mesh3d) = mesh_entities.get(entity) else { continue };
        let Some(positions) = meshes
            .get(&mesh3d.0)
            .and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION))
            .and_then(|a| a.as_float3())
        else {
            continue;
        };
        let m = crate::weapon_drops::root_from(entity, root, &parents, &transforms);
        for p in positions {
            let p = m.transform_point3(Vec3::from_array(*p));
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    let size = hi - lo;
    let long = size.max_element();
    if !long.is_finite() || long <= 1e-6 {
        warn!("mystery box: a lethal with no meshes to fit");
        return;
    }
    let scale = model.length / long;
    let turn = if size.z >= size.x && size.z >= size.y {
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    } else {
        Quat::IDENTITY
    };
    let center = (lo + hi) * 0.5;
    commands.entity(root).insert((
        Transform {
            translation: -(turn * (center * scale)),
            rotation: turn,
            scale: Vec3::splat(scale),
        },
        Visibility::Inherited,
    ));
}

/// The next prize to show while cycling — never the same one twice running.
fn next_cycled(current: usize, spin: u32, step: u32) -> usize {
    let hop = 1 + (shared::bots::rand01(((spin as u64) << 32) | step as u64) * 3.999) as usize;
    (current + hop) % BoxPrize::ALL.len()
}

/// Draw the lobby's spin: the lid open or shutting (with its sounds), the
/// glow, and the prize cycling up, hovering, sinking or gone.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn animate_mystery_box(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Res<GameSounds>,
    mut boxes: Query<(Entity, &mut MysteryBox)>,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    mut lights: Query<&mut PointLight, With<BoxGlowLight>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut display: Query<&mut Transform, With<PrizeDisplay>>,
    mut prizes: Query<(&PrizeModel, &mut Visibility)>,
    mut commands: Commands,
) {
    let Some(lobby) = zombies_game(&local, &lobbies) else { return };
    let Some(at) = shared::mystery_box::placement(lobby.map) else { return };
    let Some((box_e, mut mbox)) = boxes.iter_mut().next() else { return };
    let dt = if lobby.paused { 0.0 } else { time.delta_secs() };
    let spin = lobby.mystery_box;
    mbox.paused = lobby.paused;

    // A new spin, or the next phase of it: start its clock.
    if spin.map(|s| s.id) != mbox.spin_id {
        mbox.spin_id = spin.map(|s| s.id);
        mbox.cycling = 0;
        mbox.cycle_secs = 0.0;
        mbox.cycle_step = 0;
    }
    if spin.map(|s| s.phase) != mbox.phase {
        mbox.phase = spin.map(|s| s.phase);
        mbox.phase_secs = 0.0;
    }
    mbox.phase_secs += dt;
    let t = mbox.phase_secs;

    // The lid: open through the spin and the offer; shut once the prize is
    // taken, or once it's sunk back in.
    let open = match spin {
        Some(s) => match s.phase {
            BoxPhase::Spinning | BoxPhase::Offering => true,
            BoxPhase::Closing => !s.taken && t < SINK_SECS,
        },
        None => false,
    };
    // Its sounds, from the box: the lid opening and the spin's music
    // together, then the lid shutting.
    if open != mbox.was_open {
        mbox.was_open = open;
        let clips = if open {
            vec![(BoxSound::Open, sounds.mystery_box_open.clone()), (BoxSound::Spin, sounds.mystery_box_spin.clone())]
        } else {
            vec![(BoxSound::Close, sounds.mystery_box_close.clone())]
        };
        for (kind, clip) in clips {
            commands.entity(box_e).with_child((
                kind,
                // Volume is ours to set (`fade_box_sounds`), not the
                // one-shot volume pass's.
                crate::RemoteSoundEmitter,
                AudioPlayer::new(clip),
                Transform::from_translation(Vec3::Y * HALF_EXTENTS.y),
                // Starts silent; the real volume is set from the next frame.
                crate::positional_playback(bevy::audio::Volume::Linear(0.0)),
            ));
        }
    }
    let step = dt / LID_SECS;
    mbox.lid = if open { (mbox.lid + step).min(1.0) } else { (mbox.lid - step).max(0.0) };
    let fade = dt / GLOW_FADE_SECS;
    mbox.glow = if open { (mbox.glow + fade).min(1.0) } else { (mbox.glow - fade).max(0.0) };

    // The clip, scrubbed to the lid: found and held paused the first time.
    if mbox.player.is_none() {
        if let Some(found) = children.iter_descendants(box_e).find(|&e| players.contains(e)) {
            let mut player = players.get_mut(found).unwrap();
            player.start(mbox.clip).pause();
            commands.entity(found).insert(AnimationGraphHandle(mbox.graph.clone()));
            mbox.player = Some(found);
        }
    }
    if let Some(mut player) = mbox.player.and_then(|p| players.get_mut(p).ok()) {
        if let Some(clip) = player.animation_mut(mbox.clip) {
            clip.seek_to(mbox.lid * LID_SECS);
        }
    }

    // The glow.
    for mut light in &mut lights {
        light.intensity = GLOW_INTENSITY * mbox.glow * at.scale.max(0.2);
    }
    if let Some(m) = materials.get_mut(&mbox.glow_material) {
        let g = mbox.glow;
        m.base_color = AMBER.with_alpha(0.55 * g);
        let c = AMBER.to_linear();
        m.emissive = LinearRgba::rgb(c.red, c.green, c.blue) * (3.0 * g);
    }

    // The prize: which one shows, and how high.
    let height = |k: f32| PRIZE_LOW + (PRIZE_HIGH - PRIZE_LOW) * k;
    let showing: Option<(BoxPrize, f32)> = spin.and_then(|s| match s.phase {
        BoxPhase::Spinning => {
            if t < RISE_DELAY_SECS {
                return None;
            }
            let k = ((t - RISE_DELAY_SECS) / (SPIN_SECS - RISE_DELAY_SECS)).clamp(0.0, 1.0);
            mbox.cycle_secs += dt;
            if mbox.cycle_secs >= CYCLE_FAST_SECS + CYCLE_SLOWDOWN_SECS * k * k {
                mbox.cycle_secs = 0.0;
                mbox.cycle_step += 1;
                mbox.cycling = next_cycled(mbox.cycling, s.id, mbox.cycle_step);
            }
            Some((BoxPrize::ALL[mbox.cycling], height(1.0 - (1.0 - k) * (1.0 - k))))
        }
        BoxPhase::Offering => Some((s.prize, height(1.0) + BOB * (t * BOB_SPEED).sin())),
        BoxPhase::Closing if !s.taken && t < SINK_SECS => {
            let k = t / SINK_SECS;
            Some((s.prize, height(1.0 - k * k)))
        }
        BoxPhase::Closing => None,
    });
    for (model, mut vis) in &mut prizes {
        let shown = showing.is_some_and(|(p, _)| p == model.0);
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
    if let (Some((_, y)), Ok(mut d)) = (showing, display.single_mut()) {
        let scale = at.scale.max(0.01);
        d.translation = at.pos + Vec3::Y * y * scale;
        d.rotation = at.rotation();
    }
}

/// Keep each box sound's loudness matched to how far we are from the box.
fn fade_box_sounds(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    sound_vol: Res<crate::SoundVolumes>,
    remote: Res<crate::RemoteSoundSettings>,
    global_volume: Res<GlobalVolume>,
    mut sounds: Query<(&BoxSound, &GlobalTransform, &mut SpatialAudioSink)>,
) {
    let Ok(ear) = listener.single() else { return };
    let ear = ear.translation();
    for (kind, gt, mut sink) in &mut sounds {
        let volume = match kind {
            BoxSound::Open => sound_vol.mystery_box_open,
            BoxSound::Spin => sound_vol.mystery_box_spin,
            BoxSound::Close => sound_vol.mystery_box_close,
        };
        let loudness = volume * crate::distance_falloff(ear.distance(gt.translation()), &remote);
        sink.set_volume(bevy::audio::Volume::Linear(loudness.max(0.0)) * global_volume.volume);
    }
}

/// What the interact key does at the box, standing at it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoxUse {
    Spin,
    TooPoor,
    /// Our spin's prize, on offer.
    Take(BoxPrize),
}

/// What we can do at the box, if we're standing at it in our `Zombies`
/// game — nothing while someone else's spin is going.
fn at_box(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>, player: &Transform) -> Option<BoxUse> {
    let lobby = zombies_game(local, lobbies)?;
    let at = shared::mystery_box::placement(lobby.map)?;
    let me = local.iter().next()?.0;
    let feet = player.translation - Vec3::Y * EYE_HEIGHT;
    if !shared::mystery_box::in_range_of(at, feet, 0.0) {
        return None;
    }
    match lobby.mystery_box {
        None => {
            let points = lobby.members.iter().find(|m| m.peer == me)?.score;
            Some(if points >= COST { BoxUse::Spin } else { BoxUse::TooPoor })
        }
        Some(s) if s.user == me && s.phase == BoxPhase::Offering => Some(BoxUse::Take(s.prize)),
        Some(_) => None,
    }
}

/// Whether the interact key's the box's — not a dropped weapon's
/// (`knife_pickup`).
pub(crate) fn usable_in_reach(
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&Lobby>,
    player: &Transform,
) -> bool {
    matches!(at_box(local, lobbies, player), Some(BoxUse::Spin | BoxUse::Take(_)))
}

// --- the card ----------------------------------------------------------------

#[derive(Component)]
struct BoxCard;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum BoxCardText {
    Title,
    Detail,
    Action,
}

fn spawn_box_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let text = |size: f32| TextFont {
        font: font.clone(),
        font_size: size,
        ..default()
    };
    commands
        .spawn((
            StateScoped(AppState::InGame),
            GlobalZIndex(5),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(72.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                BoxCard,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(2.0)),
                    padding: UiRect::axes(Val::Px(22.0), Val::Px(8.0)),
                    row_gap: Val::Px(2.0),
                    min_width: Val::Px(260.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.88)),
                BorderColor(AMBER),
                BorderRadius::all(Val::Px(4.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((BoxCardText::Title, Text::new(""), text(30.0), TextColor(Color::WHITE)));
                card.spawn((BoxCardText::Detail, Text::new(""), text(20.0), TextColor(MONEY_YELLOW)));
                card.spawn((BoxCardText::Action, Text::new(""), text(20.0), TextColor(Color::WHITE)));
            });
        });
}

/// Show the card while we're at the box with something to do there (hidden
/// behind menus, during a kill cam and while dead, like the rest of the
/// HUD).
#[allow(clippy::too_many_arguments)]
fn update_box_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    card: Single<(&mut Visibility, &mut BorderColor), With<BoxCard>>,
    mut texts: Query<(&BoxCardText, &mut Text, &mut TextColor)>,
) {
    let (mut vis, mut border) = card.into_inner();
    let here = (!menu.is_open() && active_killcam.0.is_none() && !death.is_active())
        .then(|| at_box(&local, &lobbies, &player))
        .flatten();
    let Some(what) = here else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    let key = binds.interact.label().to_uppercase();
    let (title, detail, detail_color, action, edge) = match what {
        BoxUse::Spin => (
            "MYSTERY BOX".to_string(),
            format!("COST  {COST}"),
            MONEY_YELLOW,
            format!("PRESS {key} TO SPIN"),
            AMBER,
        ),
        BoxUse::TooPoor => (
            "MYSTERY BOX".to_string(),
            format!("COST  {COST}"),
            CARD_RED,
            "NOT ENOUGH POINTS".to_string(),
            CARD_RED,
        ),
        BoxUse::Take(prize) => (
            prize.label().to_string(),
            prize.gun().map_or("LETHAL — FULL LOAD", |g| g.class_label()).to_string(),
            Color::srgba(1.0, 1.0, 1.0, 0.55),
            format!("PRESS {key} TO TAKE"),
            AMBER,
        ),
    };
    border.set_if_neq(BorderColor(edge));
    for (which, mut text, mut color) in &mut texts {
        let (wanted, tint) = match which {
            BoxCardText::Title => (title.clone(), Color::WHITE),
            BoxCardText::Detail => (detail.clone(), detail_color),
            BoxCardText::Action => (action.clone(), if what == BoxUse::TooPoor { CARD_RED } else { Color::WHITE }),
        };
        if text.0 != wanted {
            text.0 = wanted;
        }
        color.set_if_neq(TextColor(tint));
    }
}

/// The interact key at the box, hands free: spin it (saying which lethal
/// we're full of, so it won't land on that), or take our prize — handing
/// over the weapon in hand's rounds for the drop, or how many of the other
/// lethal we carry.
#[allow(clippy::too_many_arguments)]
fn use_mystery_box(
    time: Res<Time>,
    mut last_press: Local<Option<f32>>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<Weapon>,
    hands: crate::weapons::Hands,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    player: Single<&Transform, With<Player>>,
    mut spin_sender: Query<&mut TriggerSender<shared::SpinMysteryBox>, With<GameClient>>,
    mut take_sender: Query<&mut TriggerSender<shared::TakeBoxPrize>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) || !hands.free() {
        return;
    }
    let now = time.elapsed_secs();
    if last_press.is_some_and(|t| now - t < PRESS_COOLDOWN_SECS) {
        return;
    }
    match at_box(&local, &lobbies, &player) {
        Some(BoxUse::Spin) => {
            if let Ok(mut s) = spin_sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::SpinMysteryBox {
                    knives_full: weapon.lethal_full(Lethal::ThrowingKnife),
                    molotovs_full: weapon.lethal_full(Lethal::Molotov),
                    monkeys_full: weapon.lethal_full(Lethal::MonkeyBomb),
                    frags_full: weapon.lethal_full(Lethal::Frag),
                });
                *last_press = Some(now);
            }
        }
        Some(BoxUse::Take(prize)) => {
            let (mag, reserve) = weapon.held_ammo();
            if let Ok(mut s) = take_sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::TakeBoxPrize {
                    slot: weapon.held as u8,
                    mag,
                    reserve,
                    dropping: match prize {
                        BoxPrize::ThrowingKnife => weapon.carried_other_than(Lethal::ThrowingKnife),
                        BoxPrize::Molotov => weapon.carried_other_than(Lethal::Molotov),
                        BoxPrize::MonkeyBomb => weapon.carried_other_than(Lethal::MonkeyBomb),
                        BoxPrize::Frag => weapon.carried_other_than(Lethal::Frag),
                        _ => None,
                    },
                });
                *last_press = Some(now);
            }
        }
        _ => {}
    }
}

/// The server handed us our prize: a gun's in our hands (full), drawn, or
/// we're full of the lethal — and the pickup sound, just for us.
fn receive_prizes(
    mut receivers: Query<&mut MessageReceiver<shared::BoxPrizeTaken>>,
    mut weapon: ResMut<Weapon>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for got in rx.receive() {
            match got.prize {
                BoxPrize::ThrowingKnife => weapon.fill_lethal(Lethal::ThrowingKnife),
                BoxPrize::Molotov => weapon.fill_lethal(Lethal::Molotov),
                BoxPrize::MonkeyBomb => weapon.fill_lethal(Lethal::MonkeyBomb),
                BoxPrize::Frag => weapon.fill_lethal(Lethal::Frag),
                gun => {
                    if let Some(gun) = gun.gun() {
                        weapon.take_into_slot(got.slot as usize, SlotWeapon::Gun(gun), None);
                    }
                }
            }
            commands.spawn((AudioPlayer::new(sounds.pick_up_equipment.clone()), PlaybackSettings::DESPAWN));
        }
    }
}
