//! The `Zombies` Pack-a-Punch machine (`models/props/pap_machine.glb`) on a map
//! that has one (`shared::pap::machine_pos`, from its level layout), glowing
//! blue once the power's on (and solid, like the perk machines), and what
//! being packed looks like: each packed weapon's body — ours, and the gun in
//! other players' hands — wears that level's camo
//! (`textures/camos/pap_<level>.webp`), scrolling across it. Buying happens
//! in `pap_menu`.
//!
//! The machine is `StateScoped(InGame)` and taken away whenever we're not in
//! such a game; our levels are read off the replicated lobby every frame
//! (nothing packed outside a `Zombies` game), so nothing carries into the
//! next one.

use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use lightyear::prelude::LocalId;
use shared::pap::{PapLevels, PapWeapon};
use shared::weapon::SlotWeapon;
use shared::{GameMode, Lobby};

use crate::net::GameClient;
use crate::weapons::{KnifeViewModel, ViewModel, Weapon};
use crate::zombies_hud::{zombies_game, PerkMachineSettings};
use crate::AppState;

pub(crate) const PAP_MODEL: &str = "models/props/pap_machine.glb";
/// Half the model's height (m) at scale 1 — its origin is at its middle.
const PAP_HALF_HEIGHT: f32 = 2.0;
/// The machine's glow.
pub(crate) const PAP_BLUE: Color = Color::srgb(0.25, 0.55, 1.0);

/// The parts of `models/weapons/sniper.glb` that are the gun (not the arms) and
/// wear the camo — matched against glTF node / mesh names.
const SNIPER_CAMO_PARTS: [&str; 3] = ["scope_sniper_0", "base_sniper_0", "mag_sniper_0"];
/// ...of `models/weapons/ak_74.glb` (the AK-74 itself, not the arms)...
const AK_CAMO_PARTS: [&str; 1] = ["Object_71"];
/// ...of `models/weapons/knife.glb`...
const KNIFE_CAMO_PARTS: [&str; 1] = ["knife_knife_0"];
/// ...and of a remote player's `models/characters/soldier.glb` (just the gun).
const AVATAR_CAMO_PARTS: [&str; 1] = ["Object_10"];

pub(crate) struct PapPlugin;

impl Plugin for PapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PapSettings>()
            .init_resource::<PapCamo>()
            .add_observer(tag_camo_parts)
            .add_systems(Update, sync_pap_machine.run_if(in_state(AppState::InGame)))
            // Not gated on `InGame`: it's what takes the camo (and the extra
            // ammo) back *off* once the game is left.
            .add_systems(Update, (sync_pap_levels, apply_pap_camo));
    }
}

/// Panel-tunable Pack-a-Punch look. Where it stands is the map's layout's
/// (`shared::pap::machine_pos`).
#[derive(Resource, Clone)]
pub(crate) struct PapSettings {
    /// 1 = the model as it comes: 2.5 m wide, 4 m tall, 1.2 m deep.
    pub(crate) scale: f32,
    /// Where its blue light sits (m) from the ground under its middle, in
    /// the machine's own frame. Intensity / range / softness are the perk
    /// machines' ("Perk machines" → Light).
    pub(crate) light_offset: Vec3,
    /// Camo scroll (texture widths per second, along u and v).
    pub(crate) camo_scroll: Vec2,
    /// How many times the camo repeats across the weapon's texture space.
    pub(crate) camo_tiling: f32,
    /// How much the camo glows by itself (0 = only lit by the scene).
    pub(crate) camo_glow: f32,
}

impl Default for PapSettings {
    fn default() -> Self {
        Self {
            // A 2 m tall machine.
            scale: 0.5,
            light_offset: Vec3::new(0.0, 1.6, 0.9),
            camo_scroll: Vec2::new(0.08, 0.03),
            camo_tiling: 2.0,
            camo_glow: 1.5,
        }
    }
}

impl PapSettings {

    /// Half the solid box's width, height and depth (m) at this scale — at
    /// the default it's `shared::pap::MACHINE_HALF_EXTENTS`, the server's.
    fn half_extents(&self) -> Vec3 {
        Vec3::new(1.25, PAP_HALF_HEIGHT, 0.6) * self.scale.max(1e-4)
    }

    /// The model, from the machine's root (the ground under its middle).
    pub(crate) fn model_transform(&self) -> Transform {
        let scale = self.scale.max(1e-4);
        Transform::from_translation(Vec3::Y * PAP_HALF_HEIGHT * scale).with_scale(Vec3::splat(scale))
    }
}

/// The machine's root standing at `at`.
pub(crate) fn root_transform(at: shared::level::Placement) -> Transform {
    Transform::from_translation(at.pos).with_rotation(at.rotation())
}

#[derive(Component)]
struct PapMachine;

#[derive(Component)]
struct PapModel;

#[derive(Component)]
struct PapLight;

#[derive(Component)]
struct PapCollider;

/// Put the machine on the map while we're in a `Zombies` game on a map with
/// one (and take it away otherwise), where its layout says, its light faded
/// in with the power like the perk machines'.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_pap_machine(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    settings: Res<PapSettings>,
    machines_cfg: Res<PerkMachineSettings>,
    map_lights: Res<crate::power::MapLightSettings>,
    time: Res<Time>,
    asset_server: Res<AssetServer>,
    mut machines: Query<
        (Entity, &mut Transform),
        (With<PapMachine>, Without<PapModel>, Without<PapLight>, Without<PapCollider>),
    >,
    mut models: Query<&mut Transform, (With<PapModel>, Without<PapMachine>, Without<PapLight>, Without<PapCollider>)>,
    mut lights: Query<
        (&mut Transform, &mut PointLight),
        (With<PapLight>, Without<PapMachine>, Without<PapModel>, Without<PapCollider>),
    >,
    mut colliders: Query<
        (&mut Transform, &mut bevy_rapier3d::prelude::Collider),
        (With<PapCollider>, Without<PapMachine>, Without<PapModel>, Without<PapLight>),
    >,
    // How far the light's faded in (0..=1). Back to 0 whenever there's no
    // machine.
    mut power: Local<f32>,
    mut commands: Commands,
) {
    let found = zombies_game(&local, &lobbies).and_then(|l| shared::level::layout(l.map).pack_a_punch.map(|at| (l, at)));
    let Some((lobby, at)) = found else {
        for (e, _) in &machines {
            commands.entity(e).despawn();
        }
        *power = 0.0;
        return;
    };
    if machines.is_empty() {
        commands
            .spawn((
                StateScoped(AppState::InGame),
                PapMachine,
                crate::power::PoweredHum(Vec3::Y * 1.0),
                root_transform(at),
                Visibility::default(),
            ))
            .with_children(|m| {
                m.spawn((
                    PapModel,
                    SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(PAP_MODEL))),
                    settings.model_transform(),
                ));
                let half = settings.half_extents();
                m.spawn((
                    PapCollider,
                    bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z),
                    Transform::from_xyz(0.0, half.y, 0.0),
                ));
                m.spawn((
                    PapLight,
                    // (Dark until the power fades it in, below.)
                    PointLight {
                        intensity: 0.0,
                        shadows_enabled: false,
                        ..machines_cfg.light.point_light(PAP_BLUE)
                    },
                    Transform::from_translation(settings.light_offset),
                ));
            });
        return;
    }

    let target = if shared::power::has_power(lobby.map, lobby.power_on) || map_lights.force_on {
        1.0
    } else {
        0.0
    };
    *power = if map_lights.fade_secs <= 0.0 {
        target
    } else {
        let step = time.delta_secs() / map_lights.fade_secs;
        *power + (target - *power).clamp(-step, step)
    };
    let fade = *power * *power * (3.0 - 2.0 * *power);

    for (_, mut t) in &mut machines {
        t.set_if_neq(root_transform(at));
    }
    for mut t in &mut models {
        t.set_if_neq(settings.model_transform());
    }
    if settings.is_changed() {
        let half = settings.half_extents();
        for (mut t, mut c) in &mut colliders {
            t.set_if_neq(Transform::from_xyz(0.0, half.y, 0.0));
            *c = bevy_rapier3d::prelude::Collider::cuboid(half.x, half.y, half.z);
        }
    }
    for (mut t, mut l) in &mut lights {
        t.set_if_neq(Transform::from_translation(settings.light_offset));
        let mut lit = machines_cfg.light.point_light(PAP_BLUE);
        lit.intensity *= fade;
        lit.shadows_enabled &= fade > 0.0;
        if l.intensity != lit.intensity
            || l.range != lit.range
            || l.radius != lit.radius
            || l.shadows_enabled != lit.shadows_enabled
        {
            *l = lit;
        }
    }
}

/// Our Pack-a-Punch levels in a running `Zombies` game (nothing packed
/// otherwise).
pub(crate) fn my_pap_levels(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>) -> PapLevels {
    let me = local.iter().next().map(|l| l.0);
    zombies_game(local, lobbies)
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map_or_else(PapLevels::default, |m| m.pap)
}

/// Keep the gun in hand's ammo limit in step with its level; a new level on
/// the same gun is our purchase going through, so the mag and reserve are
/// both filled to the new most, Cold War style. (A different gun — swapped
/// to, bought or picked up — just brings its own level.)
fn sync_pap_levels(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut weapon: ResMut<Weapon>,
    mut last_gun: Local<Option<shared::weapon::WeaponId>>,
) {
    let level = my_pap_levels(&local, &lobbies).get(PapWeapon::gun(weapon.primary));
    let same_gun = last_gun.replace(weapon.primary) == Some(weapon.primary);
    if weapon.pap_level == level {
        return;
    }
    let raised = level > weapon.pap_level;
    weapon.pap_level = level;
    if raised && same_gun {
        weapon.fill_mag_and_reserve(GameMode::Zombies);
    }
}

/// One material per level, wearing that level's camo — every packed part
/// (ours and other players') at a level shares it, so they all scroll
/// together.
#[derive(Resource)]
struct PapCamo {
    levels: [Handle<StandardMaterial>; shared::pap::MAX_LEVEL as usize],
}

impl FromWorld for PapCamo {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        // Repeat, not clamp: it tiles and scrolls.
        let textures = [1, 2, 3].map(|level| -> Handle<Image> {
            asset_server.load_with_settings(
                format!("textures/camos/pap_{level}.webp"),
                |s: &mut ImageLoaderSettings| {
                    s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::linear()
                    });
                },
            )
        });
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        let levels = textures.map(|tex| {
            materials.add(StandardMaterial {
                base_color_texture: Some(tex.clone()),
                emissive_texture: Some(tex),
                perceptual_roughness: 0.35,
                metallic: 0.2,
                ..default()
            })
        });
        Self { levels }
    }
}

/// The glTF names of `weapon`'s view model parts that wear the camo — the
/// gun itself, not the arms.
pub(crate) fn camo_parts(weapon: SlotWeapon) -> &'static [&'static str] {
    match weapon {
        SlotWeapon::Gun(shared::weapon::WeaponId::Ak74) => &AK_CAMO_PARTS,
        SlotWeapon::Gun(_) => &SNIPER_CAMO_PARTS,
        SlotWeapon::Knife => &KNIFE_CAMO_PARTS,
    }
}

/// Whether the entity named `name` is one of `parts` (a glTF node or mesh
/// name, or a mesh primitive under one — `"name.0"`).
pub(crate) fn is_camo_part(parts: &[&str], name: &str) -> bool {
    parts
        .iter()
        .any(|p| name == *p || name.strip_prefix(p).is_some_and(|r| r.starts_with('.')))
}

/// Whose level a camo part follows.
#[derive(Clone, Copy)]
enum CamoOwner {
    /// Our own view model's gun or knife.
    Mine(PapWeapon),
    /// The gun on this remote player avatar (`net::RemoteAvatar`) — its
    /// player's level for the gun they last took.
    Avatar(Entity),
    /// A dropped weapon (`weapon_drops`) — its own level.
    Fixed(u8),
}

/// On a weapon mesh that wears the camo when packed: whose level it follows,
/// and the material it came with (worn again while unpacked).
#[derive(Component)]
struct PapCamoPart {
    owner: CamoOwner,
    original: Handle<StandardMaterial>,
}

/// Once the sniper or knife view model's scene, or a remote player's
/// soldier, has spawned, find its camo parts (by glTF name — the mesh
/// entity's own, or its node's) and remember their own materials.
#[allow(clippy::too_many_arguments)]
fn tag_camo_parts(
    trigger: Trigger<SceneInstanceReady>,
    primary: Query<&crate::weapons::ViewModelAnimation, With<ViewModel>>,
    knife: Query<(), With<KnifeViewModel>>,
    avatars: Query<(), (With<crate::net::RemoteAvatar>, With<crate::SoldierVisual>)>,
    drops: Query<&crate::weapon_drops::DropModel>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    mats: Query<&MeshMaterial3d<StandardMaterial>>,
    mut commands: Commands,
) {
    let root = trigger.target();
    let (owner, parts): (_, &[&str]) = if let Ok(anim) = primary.get(root) {
        (CamoOwner::Mine(PapWeapon::gun(anim.weapon)), camo_parts(SlotWeapon::Gun(anim.weapon)))
    } else if let Ok(drop) = drops.get(root) {
        (CamoOwner::Fixed(drop.pap), camo_parts(drop.weapon))
    } else if knife.contains(root) {
        (CamoOwner::Mine(PapWeapon::Knife), &KNIFE_CAMO_PARTS)
    } else if avatars.contains(root) {
        (CamoOwner::Avatar(root), &AVATAR_CAMO_PARTS)
    } else {
        return;
    };
    let is_part = |e: Entity| names.get(e).is_ok_and(|n| is_camo_part(parts, n.as_str()));
    let mut tagged = 0;
    for entity in children.iter_descendants(root) {
        let Ok(mat) = mats.get(entity) else {
            continue;
        };
        if is_part(entity) || parents.get(entity).is_ok_and(|p| is_part(p.parent())) {
            commands.entity(entity).insert(PapCamoPart {
                owner,
                original: mat.0.clone(),
            });
            tagged += 1;
        }
    }
    if matches!(owner, CamoOwner::Mine(_)) || tagged == 0 {
        info!("pap camo: tagged {tagged} mesh(es) on {parts:?}");
    }
}

/// Dress each packed weapon's parts in its level's camo, scrolling, and put
/// their own materials back while unpacked.
#[allow(clippy::too_many_arguments)]
fn apply_pap_camo(
    time: Res<Time>,
    settings: Res<PapSettings>,
    camo: Res<PapCamo>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<&crate::net::RemoteAvatar>,
    ids: Query<&shared::PlayerId>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut parts: Query<(&PapCamoPart, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let mine = my_pap_levels(&local, &lobbies);
    let lobby = zombies_game(&local, &lobbies);
    let level_of = |owner: CamoOwner| match owner {
        CamoOwner::Mine(weapon) => mine.get(weapon),
        CamoOwner::Avatar(avatar) => avatars
            .get(avatar)
            .ok()
            .and_then(|a| ids.get(a.src).ok())
            .zip(lobby)
            .and_then(|(id, l)| l.members.iter().find(|m| m.peer == id.0))
            .map_or(0, |m| m.pap.get(PapWeapon::gun(m.primary))),
        CamoOwner::Fixed(level) => level,
    };
    let mut in_use = false;
    for (part, mut mat) in &mut parts {
        let level = level_of(part.owner).min(shared::pap::MAX_LEVEL);
        let wanted = if level == 0 {
            &part.original
        } else {
            in_use = true;
            &camo.levels[level as usize - 1]
        };
        if mat.0 != *wanted {
            mat.0 = wanted.clone();
        }
    }
    if !in_use {
        return;
    }

    let offset = (settings.camo_scroll * time.elapsed_secs()).fract_gl();
    let uv = Affine2::from_scale_angle_translation(Vec2::splat(settings.camo_tiling.max(0.01)), 0.0, offset);
    let glow = settings.camo_glow.max(0.0);
    for handle in &camo.levels {
        if let Some(m) = materials.get_mut(handle) {
            m.uv_transform = uv;
            m.emissive = LinearRgba::rgb(glow, glow, glow);
        }
    }
}
