//! The throwing-knife's arms view model (`models/arms_throwing.glb`) — just a
//! pair of arms with one baked throwing animation, shown while the
//! throwing-knife key is held and played once when it's released. The
//! hold / release / cancel state machine lives in `weapon.rs`
//! (`weapon_system`, [`super::ThrowingKnife`]); this module owns the rig, its
//! placement, and its visibility.
//!
//! The throwing knife itself (`models/throwing_knife.glb`) is a child of the
//! arms' root, so it follows the arms wherever they go (slide, and any sway
//! added later) and only needs positioning once, in the arms' local space. It
//! shows from the press until the throw clip starts.
//!
//! The clip has no hide / show animation of its own, so [`slide_throw_arms`]
//! supplies one: the arms slide up into place from below the screen once the
//! equipped weapon has finished hiding, and back down out of view after the
//! throw (the weapon only returns once they're gone).

use std::f32::consts::PI;

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy::scene::SceneInstanceReady;

use crate::death_effect::DeathEffect;
use crate::fall_death::FallDeathState;
use crate::killcam::ActiveKillCam;
use crate::VIEW_MODEL_RENDER_LAYER;

use super::weapon::ThrowingKnife;

/// The loaded `models/arms_throwing.glb` scene root.
#[derive(Component)]
pub(crate) struct ThrowArmsViewModel;

/// Handles + node index for the arms' single `throw` clip — mirrors
/// `KnifeAnimation`.
#[derive(Component)]
pub(crate) struct ThrowArmsAnimation {
    pub(crate) graph: Handle<AnimationGraph>,
    pub(crate) index: AnimationNodeIndex,
}

/// Set by `start_throw_arms_animation` on the descendant entity that carries
/// the arms' `AnimationPlayer` — mirrors `KnifeAnimationPlayer`.
#[derive(Component)]
pub(crate) struct ThrowArmsAnimationPlayer;

/// Live-tunable placement of the arms while they're held out — the debug
/// panel's "Throwing arms" section, applied every frame by
/// `apply_throw_arms_transform`. There's no hip/ADS blend: the arms never aim.
/// `translation` is the resting (fully shown) position; hiding slides them
/// `hide_drop` metres straight down from it, at `slide_speed`. (Those two and
/// `weapon_hide_speed` are edited in the debug panel's "Throwing knife"
/// section rather than "Throwing arms".)
/// Angles are in degrees on `yaw` / `pitch` / `roll` (Bevy's `YXZ` order), on
/// top of a fixed half-turn like `KnifeViewModelSettings` — the glb's arms
/// reach out along +Z, and the view-model rig looks down -Z.
#[derive(Resource)]
pub(crate) struct ThrowArmsSettings {
    pub(crate) translation: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) roll: f32,
    pub(crate) scale: f32,
    /// How far below `translation` (m) the arms sit when hidden.
    pub(crate) hide_drop: f32,
    /// Show / hide speed, in full slides per second (6 = a sixth-of-a-second
    /// slide).
    pub(crate) slide_speed: f32,
    /// How many times faster than normal the equipped weapon's own Hide
    /// animation plays when it's being stowed for the throwing arms.
    pub(crate) weapon_hide_speed: f32,
    /// Seconds into the throw clip at which the knife leaves the hand: the
    /// throw request goes to the server then (aim taken at that moment). The
    /// clip is ~0.67 s long.
    pub(crate) throw_release_secs: f32,
    /// Debug toggle (the "Throwing knife model" panel section): acts as
    /// holding the throwing-knife key without holding it, so the held pose can
    /// be tuned with the cursor free. Turning it on starts the sequence, like a
    /// press; turning it off is a release. Never saved; not reset by the
    /// panel's reset buttons.
    pub(crate) debug_hold_key: bool,
}

impl Default for ThrowArmsSettings {
    fn default() -> Self {
        Self {
            translation: Vec3::new(0.0, -0.05, -0.55),
            yaw: 0.0,
            pitch: -10.0,
            roll: 0.0,
            scale: 0.01,
            // A starting guess — tune from the debug panel.
            hide_drop: 0.8,
            slide_speed: 6.0,
            weapon_hide_speed: 6.0,
            throw_release_secs: 0.25,
            debug_hold_key: false,
        }
    }
}

impl ThrowArmsSettings {
    /// The arms' pose at slide progress `slide` (`0` hidden .. `1` shown),
    /// eased so they settle into place instead of stopping dead.
    pub(crate) fn transform(&self, slide: f32) -> Transform {
        let eased = slide * slide * (3.0 - 2.0 * slide);
        Transform {
            translation: self.translation - Vec3::Y * self.hide_drop * (1.0 - eased),
            rotation: Quat::from_euler(
                EulerRot::YXZ,
                PI + self.yaw.to_radians(),
                self.pitch.to_radians(),
                self.roll.to_radians(),
            ),
            scale: Vec3::splat(self.scale),
        }
    }
}

/// Once the arms scene has spawned, drop every entity it created onto the
/// view-model render layer and arm its animation player, parked (paused) on
/// the first frame — mirrors `start_knife_animation`.
pub(crate) fn start_throw_arms_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    arms: Query<&ThrowArmsAnimation>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let root = trigger.target();
    let Ok(anim) = arms.get(root) else {
        return;
    };

    for entity in children.iter_descendants(root) {
        commands.entity(entity).insert((
            RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
            // Skinned meshes cull against their rest-pose AABB — see
            // `start_view_model_animation`; the arms are glued to the camera
            // anyway.
            NoFrustumCulling,
        ));

        if let Ok(mut player) = players.get_mut(entity) {
            let active = player.play(anim.index);
            active.set_repeat(RepeatAnimation::Never);
            active.seek_to(0.0);
            active.pause();
            commands.entity(entity).insert((
                AnimationGraphHandle(anim.graph.clone()),
                ThrowArmsAnimationPlayer,
            ));
        }
    }
}

/// Park the arms on the clip's first frame — the pose they're held out in
/// while the key is down.
pub(crate) fn park_throw_arms(player: &mut AnimationPlayer, node: AnimationNodeIndex) {
    let active = player.play(node);
    active.set_repeat(RepeatAnimation::Never);
    active.set_speed(1.0);
    active.replay();
    active.seek_to(0.0);
    active.pause();
}

/// Play the throw clip once from the top.
pub(crate) fn play_throw(player: &mut AnimationPlayer, node: AnimationNodeIndex) {
    let active = player.play(node);
    active.set_repeat(RepeatAnimation::Never);
    active.set_speed(1.0);
    active.replay();
    active.seek_to(0.0);
    active.resume();
}

/// Push `ThrowArmsSettings` onto the arms' `Transform` every frame, same
/// reasoning as `apply_knife_transform`.
pub(crate) fn apply_throw_arms_transform(
    settings: Res<ThrowArmsSettings>,
    knife: Res<ThrowingKnife>,
    mut arms: Single<&mut Transform, With<ThrowArmsViewModel>>,
) {
    **arms = settings.transform(knife.slide);
}

/// Slide the arms up into view once the throwing knife's arms are due out
/// ([`ThrowingKnife::arms_out`] — after the weapon has finished hiding) and
/// back down when they're not, hiding the entity entirely once fully down.
/// During a kill cam or a death effect, when the live view model is hidden or
/// replaced, they snap away instead of sliding.
pub(crate) fn slide_throw_arms(
    time: Res<Time>,
    settings: Res<ThrowArmsSettings>,
    mut knife: ResMut<ThrowingKnife>,
    killcam: Res<ActiveKillCam>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    mut arms: Single<&mut Visibility, With<ThrowArmsViewModel>>,
) {
    let suppressed =
        killcam.0.is_some() || death.is_active() || crate::fall_death::effect_active(fall);
    let step = settings.slide_speed * time.delta_secs();
    let slide = if suppressed {
        0.0
    } else if knife.arms_out() {
        (knife.slide + step).min(1.0)
    } else {
        (knife.slide - step).max(0.0)
    };
    if slide != knife.slide {
        knife.slide = slide;
    }
    let wanted = if slide > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    arms.set_if_neq(wanted);
}

/// The throwing knife model held in the arms' hand
/// (`models/throwing_knife.glb`) — a child of [`ThrowArmsViewModel`].
#[derive(Component)]
pub(crate) struct ThrowKnifeModel;

/// Live-tunable placement of the held knife, in the *arms'* local space (so
/// it stays put in the hand however the arms move) — the debug panel's
/// "Throwing knife model" section. That space carries the arms' own
/// `ThrowArmsSettings::scale`, so one unit here is `scale` metres: with the
/// default 0.01, a 20 cm knife is ~20 units long. Angles in degrees, `YXZ`.
#[derive(Resource)]
pub(crate) struct ThrowKnifeModelSettings {
    pub(crate) translation: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) roll: f32,
    pub(crate) scale: f32,
}

impl Default for ThrowKnifeModelSettings {
    fn default() -> Self {
        Self {
            // Dialled in between the fingers from the debug panel.
            translation: Vec3::new(-10.0, -5.0, 0.0),
            yaw: -55.0,
            pitch: -15.0,
            roll: 60.0,
            scale: 7.0,
        }
    }
}

impl ThrowKnifeModelSettings {
    pub(crate) fn transform(&self) -> Transform {
        Transform {
            translation: self.translation,
            rotation: Quat::from_euler(
                EulerRot::YXZ,
                self.yaw.to_radians(),
                self.pitch.to_radians(),
                self.roll.to_radians(),
            ),
            scale: Vec3::splat(self.scale),
        }
    }
}

/// Once the knife scene has spawned, put every entity it created on the
/// view-model render layer (it doesn't inherit the root's).
pub(crate) fn start_throw_knife_model(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
) {
    for entity in children.iter_descendants(trigger.target()) {
        commands.entity(entity).insert((
            RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
            NoFrustumCulling,
        ));
    }
}

/// Push `ThrowKnifeModelSettings` onto the held knife every frame and show it
/// only while [`ThrowingKnife::knife_in_hand`] — it vanishes the moment the
/// throw clip starts. (It's also hidden whenever the arms are, through
/// inheritance.)
pub(crate) fn update_throw_knife_model(
    settings: Res<ThrowKnifeModelSettings>,
    knife: Res<ThrowingKnife>,
    mut model: Single<(&mut Transform, &mut Visibility), With<ThrowKnifeModel>>,
) {
    let (tf, vis) = &mut *model;
    **tf = settings.transform();
    vis.set_if_neq(if knife.knife_in_hand() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
}
