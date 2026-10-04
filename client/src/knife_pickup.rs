//! Stopped throwing knives: a blue outline so they're easy to spot where they
//! landed, and picking them up (one more knife, and the pickup sound for the
//! picker alone). Dropped molotovs (`molotov.rs`) share the outline
//! ([`OutlineWhenLoaded`]) and the pickup rules. Dropped weapons
//! (`weapon_drops.rs`) share the outline too, tinted by their Pack-a-Punch
//! level ([`OutlineTint`]), and this card: the interact key swaps the weapon
//! in hand for the nearest one (unless a wall buy we can buy is in reach —
//! that key's the sign's then).
//!
//! A player carries one kind of lethal at a time, like Call of Duty. Walking
//! into the kind already carried (or anything, carrying none) picks it up
//! automatically, up to the most that can be carried. The other kind shows a
//! card with the interact key, which swaps to it — the server drops what was
//! carried around the player.
//!
//! The server keeps a stopped knife around for
//! `shared::throwing_knife::REST_LINGER_SECS` and owns the pickup
//! ([`shared::PickUpKnife`] → [`shared::KnifePickedUp`]); the range check is
//! shared (`shared::throwing_knife::in_pickup_range`) so the card and the
//! server agree.
//!
//! The outline: each knife avatar's meshes get a child twin — the same mesh
//! with smoothed normals ([`outline_mesh`]) — drawn with
//! [`KnifeOutlineMaterial`] (`assets/shaders/knife_outline.wgsl`), an
//! inverted hull. The twins are children of the avatar (`thrown_knife.rs`),
//! so they go with it and hide with it during a kill cam; the card is
//! `StateScoped` — nothing to reset per game.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, NotShadowCaster};
use bevy::prelude::*;
use bevy::render::mesh::{MeshVertexBufferLayoutRef, VertexAttributeValues};
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderRef, SpecializedMeshPipelineError,
};
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::render::view::NoFrustumCulling;
use lightyear::prelude::*;
use shared::{MolotovDrop, ThrownKnife, WeaponDrop};

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::thrown_knife::KnifeAvatar;
use crate::{killcam, menu, AppState, GameSounds, Lethal, Player, Weapon, EYE_HEIGHT, HUD_FONT};

const SHADER_ASSET_PATH: &str = "shaders/knife_outline.wgsl";

/// The outline's colour (sRGB).
pub(crate) const OUTLINE_BLUE: Color = Color::srgb(0.2, 0.6, 1.0);
/// Outline thickness (m) per metre from the camera — about 2 px at 1080p.
const OUTLINE_WIDTH_PER_M: f32 = 0.004;
/// Least outline thickness (m), up close.
const OUTLINE_MIN_WIDTH: f32 = 0.006;

use params::OutlineParams;

// Own module only so the allow covers the unused per-field layout checks
// `ShaderType`'s derive generates next to the struct (as in `shroom_xray`).
#[allow(dead_code)]
mod params {
    use bevy::prelude::*;
    use bevy::render::render_resource::ShaderType;

    /// Mirrors `OutlineParams` in the shader.
    #[derive(Clone, Copy, Default, ShaderType)]
    pub(crate) struct OutlineParams {
        pub(crate) color: Vec4,
        pub(crate) width: f32,
        pub(crate) min_width: f32,
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub(crate) struct KnifeOutlineMaterial {
    #[uniform(0)]
    params: OutlineParams,
}

impl Material for KnifeOutlineMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    /// Inverted hull: only the back faces of the pushed-out copy draw, so the
    /// real knife sits inside a rim of them.
    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        Ok(())
    }
}

/// An outline material per colour, and each mesh's smoothed-normal copy
/// (built on first use — the models never change).
#[derive(Resource, Default)]
struct OutlineAssets {
    materials: HashMap<[u8; 4], Handle<KnifeOutlineMaterial>>,
    meshes: HashMap<AssetId<Mesh>, Handle<Mesh>>,
}

/// On an [`OutlineWhenLoaded`] avatar: its outline's colour, if not
/// [`OUTLINE_BLUE`].
#[derive(Component, Clone, Copy)]
pub(crate) struct OutlineTint(pub(crate) Color);

/// On a knife avatar once its meshes have their outline twins.
#[derive(Component)]
struct KnifeOutlined;

/// Outlined like a stopped knife as soon as its model has loaded — a
/// dropped molotov's avatar (`molotov.rs`).
#[derive(Component)]
pub(crate) struct OutlineWhenLoaded;

/// A twin mesh drawing the outline.
#[derive(Component)]
struct KnifeOutlineTwin;

/// The "throwing knife — press F" card.
#[derive(Component)]
struct PickupCard;

/// The card's action text (it names the interact key, which can be rebound).
#[derive(Component)]
struct PickupCardAction;

/// The card's heading — what's in reach.
#[derive(Component)]
struct PickupCardTitle;

/// The card's border and action strip, in the colour of what's in reach.
#[derive(Component)]
struct PickupCardColor;

/// When (`Time::elapsed_secs`) a pickup request last went out that the
/// server hasn't answered yet — none is sent while one's in flight, so
/// walking over a pile doesn't grab more than can be carried. Cleared by the
/// answer (here and in `molotov.rs`), or given up on after
/// [`PICKUP_TIMEOUT_SECS`].
#[derive(Resource, Default)]
pub(crate) struct PendingPickup(pub(crate) Option<f32>);

/// A pickup request unanswered this long (s) is given up on.
const PICKUP_TIMEOUT_SECS: f32 = 1.0;

impl PendingPickup {
    fn waiting(&self, now: f32) -> bool {
        self.0.is_some_and(|t| now - t < PICKUP_TIMEOUT_SECS)
    }
}

fn reset_pending_pickup(mut pending: ResMut<PendingPickup>) {
    pending.0 = None;
}

pub(crate) struct KnifePickupPlugin;

impl Plugin for KnifePickupPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<KnifeOutlineMaterial> {
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .init_resource::<OutlineAssets>()
        .init_resource::<PendingPickup>()
        .add_systems(OnEnter(AppState::InGame), (spawn_pickup_card, reset_pending_pickup))
        .add_systems(OnExit(AppState::InGame), reset_pending_pickup)
        .add_systems(
            Update,
            (
                outline_resting_knives,
                update_pickup_card,
                (auto_pick_up, swap_lethal).run_if(menu::game_active.and(killcam::no_killcam)),
                receive_pickups,
                receive_weapon_pickups,
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}

/// `mesh` with its normals averaged across every vertex sharing a position,
/// so pushing each vertex out along its normal keeps hard edges closed. Just
/// positions, normals and indices — all the outline needs.
fn outline_mesh(mesh: &Mesh) -> Option<Mesh> {
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3()?;
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        return None;
    };
    let key = |p: [f32; 3]| p.map(|c| (c * 1.0e4).round() as i32);
    let mut summed: HashMap<[i32; 3], Vec3> = HashMap::new();
    for (p, n) in positions.iter().zip(normals) {
        *summed.entry(key(*p)).or_default() += Vec3::from_array(*n);
    }
    let smooth: Vec<[f32; 3]> = positions
        .iter()
        .zip(normals)
        .map(|(p, n)| {
            summed[&key(*p)]
                .try_normalize()
                .unwrap_or(Vec3::from_array(*n))
                .to_array()
        })
        .collect();
    let mut out = Mesh::new(mesh.primitive_topology(), RenderAssetUsages::RENDER_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.to_vec());
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, smooth);
    // A skinned mesh's twin is skinned the same way.
    for attribute in [Mesh::ATTRIBUTE_JOINT_INDEX, Mesh::ATTRIBUTE_JOINT_WEIGHT] {
        if let Some(values) = mesh.attribute(attribute) {
            out.insert_attribute(attribute, values.clone());
        }
    }
    if let Some(indices) = mesh.indices() {
        out.insert_indices(indices.clone());
    }
    Some(out)
}

/// Give every knife avatar whose knife has stopped (and whose model has
/// loaded) its outline — and every [`OutlineWhenLoaded`] avatar, once its
/// model has.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn outline_resting_knives(
    knives: Query<&ThrownKnife>,
    avatars: Query<
        (Entity, Option<&KnifeAvatar>, Option<&OutlineTint>),
        (Without<KnifeOutlined>, Or<(With<KnifeAvatar>, With<OutlineWhenLoaded>)>),
    >,
    children: Query<&Children>,
    mesh_entities: Query<(&Mesh3d, Option<&SkinnedMesh>), Without<KnifeOutlineTwin>>,
    mut outline: ResMut<OutlineAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<KnifeOutlineMaterial>>,
    mut commands: Commands,
) {
    for (root, avatar, tint) in &avatars {
        if avatar.is_some_and(|a| !knives.get(a.src).is_ok_and(|k| k.resting)) {
            continue;
        }
        let color = tint.map_or(OUTLINE_BLUE, |t| t.0);
        let material = outline
            .materials
            .entry(color.to_srgba().to_u8_array())
            .or_insert_with(|| {
                let lin = color.to_linear();
                materials.add(KnifeOutlineMaterial {
                    params: OutlineParams {
                        color: Vec4::new(lin.red, lin.green, lin.blue, 1.0),
                        width: OUTLINE_WIDTH_PER_M,
                        min_width: OUTLINE_MIN_WIDTH,
                    },
                })
            })
            .clone();
        let mut twinned = false;
        for entity in children.iter_descendants(root) {
            let Ok((mesh, skinned)) = mesh_entities.get(entity) else {
                continue;
            };
            let twin_mesh = match outline.meshes.get(&mesh.0.id()) {
                Some(h) => h.clone(),
                None => {
                    let Some(built) = meshes.get(&mesh.0).and_then(outline_mesh) else {
                        continue;
                    };
                    let h = meshes.add(built);
                    outline.meshes.insert(mesh.0.id(), h.clone());
                    h
                }
            };
            // A child of the mesh it outlines, so it inherits its transform.
            let mut twin = commands.spawn((
                KnifeOutlineTwin,
                Mesh3d(twin_mesh),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Inherited,
                NotShadowCaster,
                ChildOf(entity),
            ));
            // (Bent by the same bones — and culled by its posed bounds, not
            // its rest pose's, like its mesh.)
            if let Some(skinned) = skinned {
                twin.insert((skinned.clone(), NoFrustumCulling));
            }
            twinned = true;
        }
        // (Model not in yet: try again next frame.)
        if twinned {
            commands.entity(root).insert(KnifeOutlined);
        }
    }
}

/// The kind of the nearest stopped knife or dropped molotov in pickup range
/// of the local player that `wanted` accepts, if any.
fn pickup_in_reach<'a>(
    player: &Transform,
    knives: impl Iterator<Item = &'a ThrownKnife>,
    drops: impl Iterator<Item = &'a MolotovDrop>,
    wanted: impl Fn(Lethal) -> bool,
) -> Option<Lethal> {
    lethal_in_reach(player, knives, drops, wanted).map(|(what, _)| what)
}

/// [`pickup_in_reach`], with how far (squared, from the eye) it is.
fn lethal_in_reach<'a>(
    player: &Transform,
    knives: impl Iterator<Item = &'a ThrownKnife>,
    drops: impl Iterator<Item = &'a MolotovDrop>,
    wanted: impl Fn(Lethal) -> bool,
) -> Option<(Lethal, f32)> {
    let eye = player.translation;
    let feet = eye - Vec3::Y * EYE_HEIGHT;
    let in_range = |p: Vec3| shared::throwing_knife::in_pickup_range(feet, eye, p);
    knives
        .filter(|k| k.resting && in_range(k.pos))
        .map(|k| (Lethal::ThrowingKnife, k.pos))
        .chain(drops.filter(|d| in_range(d.pos)).map(|d| (Lethal::Molotov, d.pos)))
        .filter(|(what, _)| wanted(*what))
        .map(|(what, pos)| (what, pos.distance_squared(eye)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The nearest dropped weapon in pickup range of the local player, with how
/// far (squared, from the eye) it is.
fn weapon_in_reach<'a>(player: &Transform, drops: impl Iterator<Item = &'a WeaponDrop>) -> Option<(WeaponDrop, f32)> {
    let eye = player.translation;
    let feet = eye - Vec3::Y * EYE_HEIGHT;
    drops
        .filter(|d| shared::throwing_knife::in_pickup_range(feet, eye, d.pos))
        .map(|d| (*d, d.pos.distance_squared(eye)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// What the card's about (and the interact key would take).
enum Offer {
    Lethal(Lethal),
    Weapon(WeaponDrop),
}

/// The nearer of the lethal `lethal` and a dropped weapon in reach — the
/// weapon left out while a wall buy we can buy is in reach (the key's the
/// sign's then).
fn offer(lethal: Option<(Lethal, f32)>, weapon: Option<(WeaponDrop, f32)>, wall_buy_here: bool) -> Option<Offer> {
    let weapon = weapon.filter(|_| !wall_buy_here);
    match (lethal, weapon) {
        (Some((l, dl)), Some((w, dw))) => Some(if dw < dl { Offer::Weapon(w) } else { Offer::Lethal(l) }),
        (Some((l, _)), None) => Some(Offer::Lethal(l)),
        (None, Some((w, _))) => Some(Offer::Weapon(w)),
        (None, None) => None,
    }
}

/// Whether walking into a `kind` picks it up without the interact key: it's
/// the kind carried, or none is.
fn auto_kind(weapon: &Weapon, kind: Lethal) -> bool {
    weapon.carried_lethal().is_none_or(|c| c == kind)
}

/// Ask the server for the nearest `kind` in reach, dropping `drop` of the
/// other kind.
fn request_pickup(
    kind: Lethal,
    drop: u32,
    knife_sender: &mut Query<&mut TriggerSender<shared::PickUpKnife>, With<GameClient>>,
    molotov_sender: &mut Query<&mut TriggerSender<shared::PickUpMolotov>, With<GameClient>>,
) {
    match kind {
        Lethal::ThrowingKnife => {
            if let Ok(mut s) = knife_sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::PickUpKnife { drop_molotovs: drop });
            }
        }
        Lethal::Molotov => {
            if let Ok(mut s) = molotov_sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::PickUpMolotov { drop_knives: drop });
            }
        }
    }
}

fn spawn_pickup_card(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font = asset_server.load(HUD_FONT);
    let heading = |size: f32| TextFont {
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
                top: Val::Percent(57.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                PickupCard,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.03, 0.03, 0.05, 0.85)),
                BorderColor(OUTLINE_BLUE),
                PickupCardColor,
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((
                    PickupCardTitle,
                    Text::new("THROWING KNIFE"),
                    heading(32.0),
                    TextColor(OUTLINE_BLUE),
                ));
                card.spawn((
                    Node {
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(OUTLINE_BLUE.with_alpha(0.25)),
                    PickupCardColor,
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_child((PickupCardAction, Text::new(""), heading(22.0), TextColor::WHITE));
            });
        });
}

/// Show the card while a stopped knife or dropped molotov is in reach that
/// isn't picked up automatically — the other kind of lethal (to swap to), or
/// the carried kind while full — or a dropped weapon, whichever's nearer
/// (hidden behind menus, during a kill cam and while dead, like the rest of
/// the HUD).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_pickup_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    weapon: Res<Weapon>,
    player: Single<&Transform, With<Player>>,
    knives: Query<&ThrownKnife, With<Interpolated>>,
    drops: Query<&MolotovDrop>,
    weapon_drops: Query<&WeaponDrop>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut card: Single<&mut Visibility, With<PickupCard>>,
    mut action: Single<&mut Text, (With<PickupCardAction>, Without<PickupCardTitle>)>,
    title: Single<(&mut Text, &mut TextColor), (With<PickupCardTitle>, Without<PickupCardAction>)>,
    mut colors: Query<(Option<&mut BorderColor>, Option<&mut BackgroundColor>), With<PickupCardColor>>,
) {
    let shown = !menu.is_open() && active_killcam.0.is_none() && !death.is_active();
    let what = shown
        .then(|| {
            let lethal = lethal_in_reach(&player, knives.iter(), drops.iter(), |k| {
                !auto_kind(&weapon, k) || weapon.lethal_full(k)
            });
            let wall = crate::wall_buys::buyable_in_reach(&local, &lobbies, &weapon, &player);
            offer(lethal, weapon_in_reach(&player, weapon_drops.iter()), wall)
        })
        .flatten();
    card.set_if_neq(if what.is_some() { Visibility::Inherited } else { Visibility::Hidden });
    let Some(what) = what else {
        return;
    };
    let press = format!("PRESS {} TO SWAP", binds.interact.label().to_uppercase());
    let (heading, wanted, color) = match what {
        Offer::Lethal(kind) => (
            match kind {
                Lethal::ThrowingKnife => "THROWING KNIFE".to_string(),
                Lethal::Molotov => "MOLOTOV".to_string(),
            },
            if auto_kind(&weapon, kind) { "CARRYING THE MOST".to_string() } else { press },
            OUTLINE_BLUE,
        ),
        Offer::Weapon(drop) => (
            match drop.pap {
                0 => drop.weapon.label().to_string(),
                level => format!("{}  {}", drop.weapon.label(), shared::pap::numeral(level)),
            },
            if weapon.carries(drop.weapon) { "ALREADY CARRYING".to_string() } else { press },
            crate::weapon_drops::outline_color(drop.pap),
        ),
    };
    let (mut title_text, mut title_color) = title.into_inner();
    if title_text.0 != heading {
        title_text.0 = heading;
    }
    title_color.set_if_neq(TextColor(color));
    if action.0 != wanted {
        action.0 = wanted;
    }
    for (border, background) in &mut colors {
        if let Some(mut border) = border {
            border.set_if_neq(BorderColor(color));
        }
        if let Some(mut background) = background {
            background.set_if_neq(BackgroundColor(color.with_alpha(0.25)));
        }
    }
}

/// Walking into the kind of lethal carried (or anything, carrying none):
/// ask the server for it, unless already full or a pickup's in flight. The
/// server checks the range again and picks the nearest of that kind itself.
#[allow(clippy::too_many_arguments)]
fn auto_pick_up(
    time: Res<Time>,
    weapon: Res<Weapon>,
    mut pending: ResMut<PendingPickup>,
    death: Res<crate::death_effect::DeathEffect>,
    player: Single<&Transform, With<Player>>,
    knives: Query<&ThrownKnife, With<Interpolated>>,
    drops: Query<&MolotovDrop>,
    mut knife_sender: Query<&mut TriggerSender<shared::PickUpKnife>, With<GameClient>>,
    mut molotov_sender: Query<&mut TriggerSender<shared::PickUpMolotov>, With<GameClient>>,
) {
    let now = time.elapsed_secs();
    if pending.waiting(now) || death.is_active() {
        return;
    }
    let Some(kind) = pickup_in_reach(&player, knives.iter(), drops.iter(), |k| {
        auto_kind(&weapon, k) && !weapon.lethal_full(k)
    }) else {
        return;
    };
    request_pickup(kind, 0, &mut knife_sender, &mut molotov_sender);
    pending.0 = Some(now);
}

/// The interact key with the other kind of lethal in reach: swap to it —
/// the server drops every one of the current kind around the player. Or,
/// with a dropped weapon nearer (and our hands free), swap the one in hand
/// for it — the server drops ours where we stand.
#[allow(clippy::too_many_arguments)]
fn swap_lethal(
    time: Res<Time>,
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    weapon: Res<Weapon>,
    hands: crate::weapons::Hands,
    mut pending: ResMut<PendingPickup>,
    player: Single<&Transform, With<Player>>,
    knives: Query<&ThrownKnife, With<Interpolated>>,
    drops: Query<&MolotovDrop>,
    (weapon_drops, local, lobbies): (Query<&WeaponDrop>, Query<&LocalId, With<GameClient>>, Query<&shared::Lobby>),
    mut knife_sender: Query<&mut TriggerSender<shared::PickUpKnife>, With<GameClient>>,
    mut molotov_sender: Query<&mut TriggerSender<shared::PickUpMolotov>, With<GameClient>>,
    mut weapon_sender: Query<&mut TriggerSender<shared::PickUpWeapon>, With<GameClient>>,
) {
    let now = time.elapsed_secs();
    if !binds.interact.just_pressed(&keys, &mouse) || pending.waiting(now) {
        return;
    }
    let lethal = lethal_in_reach(&player, knives.iter(), drops.iter(), |k| !auto_kind(&weapon, k));
    let wall = crate::wall_buys::buyable_in_reach(&local, &lobbies, &weapon, &player);
    match offer(lethal, weapon_in_reach(&player, weapon_drops.iter()), wall) {
        Some(Offer::Lethal(kind)) => {
            request_pickup(kind, weapon.lethal_count(), &mut knife_sender, &mut molotov_sender);
            pending.0 = Some(now);
        }
        Some(Offer::Weapon(drop)) if !weapon.carries(drop.weapon) && hands.free() => {
            let (mag, reserve) = weapon.held_ammo();
            if let Ok(mut s) = weapon_sender.single_mut() {
                s.trigger::<shared::LobbyChannel>(shared::PickUpWeapon {
                    slot: weapon.held as u8,
                    mag,
                    reserve,
                });
                pending.0 = Some(now);
            }
        }
        _ => {}
    }
}

/// The server handed us a knife we picked up: count it (it's now the only
/// lethal), and play the pickup sound — just for us, not positional.
fn receive_pickups(
    mut receivers: Query<&mut MessageReceiver<shared::KnifePickedUp>>,
    mut weapon: ResMut<Weapon>,
    mut pending: ResMut<PendingPickup>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            weapon.add_throwing_knife();
            pending.0 = None;
            commands.spawn((
                AudioPlayer::new(sounds.pick_up_equipment.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}

/// The server handed us a dropped weapon we picked up: it's in our hands now
/// (with the rounds it was dropped with), drawn; the pickup sound, just for
/// us.
fn receive_weapon_pickups(
    mut receivers: Query<&mut MessageReceiver<shared::WeaponPickedUp>>,
    mut weapon: ResMut<Weapon>,
    mut pending: ResMut<PendingPickup>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for got in rx.receive() {
            let ammo = got.weapon.gun().map(|_| (got.mag, got.reserve));
            weapon.take_into_slot(got.slot as usize, got.weapon, ammo);
            pending.0 = None;
            commands.spawn((
                AudioPlayer::new(sounds.pick_up_equipment.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}
