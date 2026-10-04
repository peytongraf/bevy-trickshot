//! The muzzle-flash sprite: a quad that snaps to full brightness on a shot
//! and decays over `MUZZLE_FLASH_TIME`, with a fresh random roll each time.

use bevy::prelude::*;

/// Seconds for the muzzle flash to go from full to gone (it pops on instantly).
pub(crate) const MUZZLE_FLASH_TIME: f32 = 0.06;

/// The muzzle-flash sprite quad.
#[derive(Component)]
pub(crate) struct MuzzleFlash;

/// Panel-adjustable placement of the sniper's muzzle-flash sprite, relative to
/// the camera rig at the hip pose (it follows the gun from there — see
/// [`update_muzzle_flash`]). Pitch and yaw follow the gun's tilt; roll is
/// randomised per shot.
#[derive(Resource)]
pub(crate) struct MuzzleFlashSettings {
    pub(crate) translation: Vec3,
    pub(crate) size: Vec2,
}

impl Default for MuzzleFlashSettings {
    fn default() -> Self {
        Self {
            translation: Vec3::new(0.15, -0.07, -1.42),
            size: Vec2::new(0.9, 0.8),
        }
    }
}

/// Live state of the flash: `intensity` snaps to 1 on a shot and decays to 0
/// over `MUZZLE_FLASH_TIME`; `roll` is a fresh random angle per shot.
#[derive(Resource, Default)]
pub(crate) struct MuzzleFlashState {
    pub(crate) intensity: f32,
    pub(crate) roll: f32,
    pub(crate) shots: u32,
}

/// Where the primary's barrel ends this frame, in the camera rig's space
/// (`CameraShake`, which the view model and this sprite hang off) — the
/// muzzle flash sits on it, and the local player's tracers start where it
/// shows on screen (`weapons::resolve_local_shot`). Kept by
/// [`update_muzzle_flash`].
#[derive(Resource, Default)]
pub(crate) struct MuzzlePoint(pub(crate) Option<Vec3>);

/// Decay the muzzle flash and push its state onto the sprite: full alpha the
/// frame a shot fires, then a quick fade; a fresh random roll each shot.
/// Each primary's muzzle (`MuzzleFlashSettings` for the sniper,
/// `AkSettings` for the AK-74) is tuned against its hip pose and carried from
/// there to wherever the view model actually is this frame, so it stays on
/// the barrel at any ADS amount, mid-transition, and through bob / sway /
/// recoil.
pub(crate) fn update_muzzle_flash(
    time: Res<Time>,
    settings: Res<MuzzleFlashSettings>,
    (weapon, ak, poses, ray): (
        Res<crate::Weapon>,
        Res<crate::AkSettings>,
        Res<crate::ViewModelPoses>,
        Res<crate::RayGunSettings>,
    ),
    view_model: Single<&Transform, (With<crate::weapons::ViewModel>, Without<MuzzleFlash>)>,
    mut state: ResMut<MuzzleFlashState>,
    mut point: ResMut<MuzzlePoint>,
    flash: Single<
        (
            &mut Transform,
            &MeshMaterial3d<StandardMaterial>,
            &mut Visibility,
        ),
        With<MuzzleFlash>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    state.intensity = (state.intensity - time.delta_secs() / MUZZLE_FLASH_TIME).max(0.0);

    let (mut transform, material, mut visibility) = flash.into_inner();

    *visibility = if state.intensity > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };

    let raygun = weapon.primary == shared::weapon::WeaponId::RayGun;
    let (hip, muzzle, size) = match weapon.primary {
        shared::weapon::WeaponId::Ak74 => (ak.hip.transform(), ak.muzzle_translation, ak.muzzle_size),
        shared::weapon::WeaponId::RayGun => (ray.hip.transform(), ray.muzzle_translation, ray.muzzle_size),
        _ => (poses.hip.transform(), settings.translation, settings.size),
    };
    // Both are children of the same rig: hip pose → the pose right now.
    let rel = view_model.compute_affine() * hip.compute_affine().inverse();
    let turn = view_model.rotation * hip.rotation.inverse();
    let translation = rel.transform_point3(muzzle);
    point.0 = Some(translation);
    *transform = Transform {
        translation,
        rotation: turn * Quat::from_rotation_z(state.roll),
        scale: Vec3::new(size.x.max(1.0e-4), size.y.max(1.0e-4), 1.0),
    };

    if let Some(material) = materials.get_mut(&material.0) {
        // (The Ray Gun's flash is its bolt's green.)
        material.base_color = if raygun {
            Color::srgba(0.35, 1.0, 0.35, state.intensity)
        } else {
            Color::srgba(1.0, 1.0, 1.0, state.intensity)
        };
    }
}
