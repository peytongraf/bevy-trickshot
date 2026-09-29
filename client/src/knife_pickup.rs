//! Stopped throwing knives: a blue outline so they're easy to spot where they
//! landed, and — standing close enough to one — a card saying it's a
//! throwing knife with the interact key to pick it up (one more knife, and
//! the pickup sound for the picker alone).
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
use lightyear::prelude::*;
use shared::ThrownKnife;

use crate::keybinds::KeyBindings;
use crate::net::GameClient;
use crate::thrown_knife::KnifeAvatar;
use crate::{killcam, menu, AppState, GameSounds, Player, Weapon, EYE_HEIGHT, HUD_FONT};

const SHADER_ASSET_PATH: &str = "shaders/knife_outline.wgsl";

/// The outline's colour (sRGB).
const OUTLINE_BLUE: Color = Color::srgb(0.2, 0.6, 1.0);
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

/// The one outline material, and each knife mesh's smoothed-normal copy
/// (built on first use — the knife model never changes).
#[derive(Resource, Default)]
struct OutlineAssets {
    material: Option<Handle<KnifeOutlineMaterial>>,
    meshes: HashMap<AssetId<Mesh>, Handle<Mesh>>,
}

/// On a knife avatar once its meshes have their outline twins.
#[derive(Component)]
struct KnifeOutlined;

/// A twin mesh drawing the outline.
#[derive(Component)]
struct KnifeOutlineTwin;

/// The "throwing knife — press F" card.
#[derive(Component)]
struct PickupCard;

/// The card's action text (it names the interact key, which can be rebound).
#[derive(Component)]
struct PickupCardAction;

pub(crate) struct KnifePickupPlugin;

impl Plugin for KnifePickupPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<KnifeOutlineMaterial> {
            prepass_enabled: false,
            shadows_enabled: false,
            ..default()
        })
        .init_resource::<OutlineAssets>()
        .add_systems(OnEnter(AppState::InGame), spawn_pickup_card)
        .add_systems(
            Update,
            (
                outline_resting_knives,
                update_pickup_card,
                pick_up_knife.run_if(menu::game_active.and(killcam::no_killcam)),
                receive_pickups,
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
    if let Some(indices) = mesh.indices() {
        out.insert_indices(indices.clone());
    }
    Some(out)
}

/// Give every knife avatar whose knife has stopped (and whose model has
/// loaded) its outline.
#[allow(clippy::too_many_arguments)]
fn outline_resting_knives(
    knives: Query<&ThrownKnife>,
    avatars: Query<(Entity, &KnifeAvatar), Without<KnifeOutlined>>,
    children: Query<&Children>,
    mesh_entities: Query<&Mesh3d, Without<KnifeOutlineTwin>>,
    mut outline: ResMut<OutlineAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<KnifeOutlineMaterial>>,
    mut commands: Commands,
) {
    for (root, avatar) in &avatars {
        if !knives.get(avatar.src).is_ok_and(|k| k.resting) {
            continue;
        }
        let material = outline
            .material
            .get_or_insert_with(|| {
                let lin = OUTLINE_BLUE.to_linear();
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
            let Ok(mesh) = mesh_entities.get(entity) else {
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
            commands.spawn((
                KnifeOutlineTwin,
                Mesh3d(twin_mesh),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Inherited,
                NotShadowCaster,
                ChildOf(entity),
            ));
            twinned = true;
        }
        // (Model not in yet: try again next frame.)
        if twinned {
            commands.entity(root).insert(KnifeOutlined);
        }
    }
}

/// The nearest stopped knife in pickup range of the local player, if any.
fn knife_in_reach<'a>(
    player: &Transform,
    knives: impl Iterator<Item = &'a ThrownKnife>,
) -> Option<&'a ThrownKnife> {
    let eye = player.translation;
    let feet = eye - Vec3::Y * EYE_HEIGHT;
    knives
        .filter(|k| k.resting && shared::throwing_knife::in_pickup_range(feet, eye, k.pos))
        .min_by(|a, b| a.pos.distance_squared(eye).total_cmp(&b.pos.distance_squared(eye)))
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
                BorderRadius::all(Val::Px(6.0)),
                Visibility::Hidden,
            ))
            .with_children(|card| {
                card.spawn((Text::new("THROWING KNIFE"), heading(32.0), TextColor(OUTLINE_BLUE)));
                card.spawn((
                    Node {
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(OUTLINE_BLUE.with_alpha(0.25)),
                    BorderRadius::all(Val::Px(4.0)),
                ))
                .with_child((PickupCardAction, Text::new(""), heading(22.0), TextColor::WHITE));
            });
        });
}

/// Show the card while a stopped knife is in reach (hidden behind menus,
/// during a kill cam and while dead, like the rest of the HUD).
#[allow(clippy::too_many_arguments)]
fn update_pickup_card(
    menu: Res<menu::Menu>,
    active_killcam: Res<killcam::ActiveKillCam>,
    death: Res<crate::death_effect::DeathEffect>,
    binds: Res<KeyBindings>,
    player: Single<&Transform, With<Player>>,
    knives: Query<&ThrownKnife, With<Interpolated>>,
    mut card: Single<&mut Visibility, With<PickupCard>>,
    mut action: Single<&mut Text, With<PickupCardAction>>,
) {
    let show = !menu.is_open()
        && active_killcam.0.is_none()
        && !death.is_active()
        && knife_in_reach(&player, knives.iter()).is_some();
    card.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
    if show {
        let wanted = format!("PRESS {} TO EQUIP", binds.interact.label().to_uppercase());
        if action.0 != wanted {
            action.0 = wanted;
        }
    }
}

/// The interact key with a stopped knife in reach: ask the server for it (it
/// checks the range again and picks the nearest itself).
fn pick_up_knife(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    player: Single<&Transform, With<Player>>,
    knives: Query<&ThrownKnife, With<Interpolated>>,
    mut sender: Query<&mut TriggerSender<shared::PickUpKnife>, With<GameClient>>,
) {
    if !binds.interact.just_pressed(&keys, &mouse) {
        return;
    }
    if knife_in_reach(&player, knives.iter()).is_none() {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::PickUpKnife);
    }
}

/// The server handed us a knife we picked up: count it, and play the pickup
/// sound — just for us, not positional.
fn receive_pickups(
    mut receivers: Query<&mut MessageReceiver<shared::KnifePickedUp>>,
    mut weapon: ResMut<Weapon>,
    sounds: Res<GameSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for _ in rx.receive() {
            weapon.throwing_knives += 1;
            commands.spawn((
                AudioPlayer::new(sounds.pick_up_equipment.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}
