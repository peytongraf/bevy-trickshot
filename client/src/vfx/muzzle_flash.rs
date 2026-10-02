//! The muzzle-flash sprite: a quad that snaps to full brightness on a shot
//! and decays over `MUZZLE_FLASH_TIME`, with a fresh random roll each time.

use bevy::prelude::*;

/// Seconds for the muzzle flash to go from full to gone (it pops on instantly).
pub(crate) const MUZZLE_FLASH_TIME: f32 = 0.06;

/// The muzzle-flash sprite quad.
#[derive(Component)]
pub(crate) struct MuzzleFlash;

/// Panel-adjustable placement of the muzzle-flash sprite, relative to the camera
/// rig. Pitch and yaw are fixed at 0 (the sprite always faces the camera); roll
/// is randomised per shot.
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

/// Decay the muzzle flash and push its state onto the sprite: full alpha the
/// frame a shot fires, then a quick fade; a fresh random roll each shot.
pub(crate) fn update_muzzle_flash(
    time: Res<Time>,
    settings: Res<MuzzleFlashSettings>,
    // The AK-74's sits at its own muzzle (`AkSettings`), and rides with the
    // view model as it aims in / bobs / sways / kicks.
    (weapon, ak): (Res<crate::Weapon>, Res<crate::AkSettings>),
    view_model: Single<&Transform, (With<crate::weapons::ViewModel>, Without<MuzzleFlash>)>,
    mut state: ResMut<MuzzleFlashState>,
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

    let roll = Quat::from_rotation_z(state.roll);
    let (translation, rotation, size) = if weapon.primary == shared::weapon::WeaponId::Ak74 {
        // `muzzle_translation` is tuned against the hip pose: carry it from
        // there to wherever the view model actually is this frame (both are
        // children of the same rig), so it stays on the barrel at any ADS
        // amount, mid-transition, and through bob / sway / recoil.
        let hip = ak.hip.transform();
        let rel = view_model.compute_affine() * hip.compute_affine().inverse();
        let turn = view_model.rotation * hip.rotation.inverse();
        (rel.transform_point3(ak.muzzle_translation), turn * roll, ak.muzzle_size)
    } else {
        (settings.translation, roll, settings.size)
    };
    *transform = Transform {
        translation,
        rotation,
        scale: Vec3::new(size.x.max(1.0e-4), size.y.max(1.0e-4), 1.0),
    };

    if let Some(material) = materials.get_mut(&material.0) {
        material.base_color = Color::srgba(1.0, 1.0, 1.0, state.intensity);
    }
}
