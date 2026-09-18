//! The throwing-knife's melee view-model rig — the visual sniper-mirroring
//! knife shown while `WeaponSlot::Secondary` is equipped (distinct from the
//! throwing-knife *throw* mechanic in `weapon.rs`).

use std::f32::consts::PI;

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy::scene::SceneInstanceReady;

use crate::VIEW_MODEL_RENDER_LAYER;

use super::view_model::AnimationSegment;

/// Frame rate `knife.glb`'s own `allanims` clip was baked at — a different
/// Blender scene, and *not* 24 like the sniper's: the exported clip's raw
/// keyframe spacing is a consistent 1/30 s and its accessor caps out at
/// exactly 4.0 s, which only lines up with the given frame numbers (a
/// 108-frame "Show" ending at "the end" of the clip) at 30 fps — at 24 fps
/// frame 108 alone would land past the clip's actual 4.0 s of data.
pub(crate) const KNIFE_ANIM_FPS: f32 = 30.0;

/// Indices into `KNIFE_SEGMENTS`.
pub(crate) const KNIFE_SEG_ADJUST_GRIP: usize = 0;
pub(crate) const KNIFE_SEG_SLICE_1: usize = 1;
pub(crate) const KNIFE_SEG_SLICE_2: usize = 2;
pub(crate) const KNIFE_SEG_SLICE_3: usize = 3;
pub(crate) const KNIFE_SEG_SLICE_4: usize = 4;
pub(crate) const KNIFE_SEG_HIDE: usize = 5;
pub(crate) const KNIFE_SEG_SHOW: usize = 6;

/// One of `KNIFE_SEG_SLICE_1..=4`, picked at random by `weapon_system` each
/// time the knife attacks — see `KnifeAnimState::swings`.
pub(crate) const KNIFE_SLICE_SEGMENTS: [usize; 4] = [
    KNIFE_SEG_SLICE_1,
    KNIFE_SEG_SLICE_2,
    KNIFE_SEG_SLICE_3,
    KNIFE_SEG_SLICE_4,
];

/// `models/knife.glb`'s own single baked clip, sliced the same way as
/// `SEGMENTS` — frame ranges lifted from a known-good cut of this same
/// animation (a separate three.js project's `AnimationUtils.subclip(fullClip,
/// name, startFrame, endFrame)` calls, which default to 30 fps — matching
/// `KNIFE_ANIM_FPS`): `idle` 0-40, `hit1` 40-60, `hit2` 60-80,
/// `backwardsHit1` 80-100, `hit4backwardsHit2` 100-121, `hide` 121-128,
/// `appear` 128-142, renamed below to match this file's own naming
/// (`Adjust Grip` / `Slice 1..4` / `Hide` / `Show`). This particular export's
/// baked clip only actually runs to frame 135 (4.5 s — confirmed directly
/// from every channel's raw keyframe times, all spaced an exact 1/30 s
/// apart), 7 frames short of that reference's 142, so `Show` is clamped to
/// 135 rather than running past the end of the data. `Show` plays once
/// whenever the knife is drawn; `Adjust Grip` is a purely cosmetic idle
/// fidget that plays on its own every few seconds while the knife is out and
/// otherwise idle, and is cut short the instant an attack comes in (see
/// `weapon_system`'s `KnifeAnimState::next_adjust_in`); each of the four
/// slices plays once, picked at random, on a left click while the knife is
/// out. Each segment starts exactly
/// where the previous one's pose ends (baked that way in Blender, the same
/// as `SEGMENTS`) — `play_segment` is a hard cut with no blending, for the
/// sniper as much as the knife, so getting these boundaries right *is* what
/// makes the flow from one animation into the next look smooth.
pub(crate) const KNIFE_SEGMENTS: [AnimationSegment; 7] = [
    AnimationSegment::new("Adjust Grip", 0.0, 40.0, KNIFE_ANIM_FPS),
    AnimationSegment::new("Slice 1", 40.0, 60.0, KNIFE_ANIM_FPS),
    AnimationSegment::new("Slice 2", 60.0, 80.0, KNIFE_ANIM_FPS),
    AnimationSegment::new("Slice 3", 80.0, 100.0, KNIFE_ANIM_FPS),
    AnimationSegment::new("Slice 4", 100.0, 121.0, KNIFE_ANIM_FPS),
    AnimationSegment::new("Hide", 121.0, 128.0, KNIFE_ANIM_FPS),
    AnimationSegment::new("Show", 128.0, 135.0, KNIFE_ANIM_FPS),
];

/// The loaded `models/knife.glb` scene root — the melee weapon in
/// `WeaponSlot::Secondary` (separate from `ThrowingKnife`, the lethal).
/// Mirrors [`ViewModel`], kept as its own component (not reused) so systems
/// that expect exactly one sniper view model — `apply_ads`, `weapon_sway`,
/// etc. — aren't broken by a second `ViewModel`-tagged entity existing.
#[derive(Component)]
pub(crate) struct KnifeViewModel;

/// Handles + node index for the knife's single animation clip — mirrors
/// [`ViewModelAnimation`].
#[derive(Component)]
pub(crate) struct KnifeAnimation {
    pub(crate) graph: Handle<AnimationGraph>,
    pub(crate) index: AnimationNodeIndex,
}

/// Set by `start_knife_animation` on the descendant entity that carries the
/// knife's `AnimationPlayer` — mirrors [`SniperAnimationPlayer`].
#[derive(Component)]
pub(crate) struct KnifeAnimationPlayer;

/// Live-tunable placement of the knife view model — debug panel's "Knife"
/// section, applied every frame by `apply_knife_transform`. Position and
/// scale only (no hip/ADS blend like `ViewModelPoses`: the knife never aims
/// down sight). Rotation isn't exposed here — `apply_knife_transform` fixes
/// it at `yaw: PI` to match `ViewModelPoses::hip`'s own convention, since
/// it's the same view-model rig facing the same way.
#[derive(Resource)]
pub(crate) struct KnifeViewModelSettings {
    pub(crate) translation: Vec3,
    pub(crate) scale: f32,
}

impl Default for KnifeViewModelSettings {
    fn default() -> Self {
        Self {
            // Dialed in against a bot for scale (see the "Knife" debug-panel
            // section).
            translation: Vec3::new(0.0, -0.13, -0.5),
            scale: 0.01,
        }
    }
}

/// Once the knife scene has spawned, drop every entity it created onto the
/// view-model render layer and arm its animation player, parked (paused) on
/// the first frame — mirrors `start_view_model_animation`, minus the
/// scope-lens handling the knife has no equivalent of.
pub(crate) fn start_knife_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    knives: Query<&KnifeAnimation>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let root = trigger.target();
    let Ok(anim) = knives.get(root) else {
        return;
    };

    for entity in children.iter_descendants(root) {
        commands.entity(entity).insert((
            RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
            NoFrustumCulling,
        ));

        if let Ok(mut player) = players.get_mut(entity) {
            let active = player.play(anim.index);
            active.set_repeat(RepeatAnimation::Never);
            active.seek_to(0.0);
            active.pause();
            commands.entity(entity).insert((
                AnimationGraphHandle(anim.graph.clone()),
                KnifeAnimationPlayer,
            ));
        }
    }
}

/// Push `KnifeViewModelSettings` onto the knife view model's `Transform`
/// every frame — mirrors `apply_shipment_transform`'s reasoning: cheap, and
/// unconditional so a value tweaked while the knife is hidden still takes
/// effect the instant it's next drawn. Unlike `apply_ads`, there's no
/// hip/ADS blend to compute — the knife never aims — so this just pushes
/// the one pose straight through.
pub(crate) fn apply_knife_transform(
    settings: Res<KnifeViewModelSettings>,
    mut knife: Single<&mut Transform, With<KnifeViewModel>>,
) {
    knife.translation = settings.translation;
    knife.rotation = Quat::from_euler(EulerRot::YXZ, PI, 0.0, 0.0);
    knife.scale = Vec3::splat(settings.scale);
}
