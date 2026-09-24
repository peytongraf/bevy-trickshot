//! The perk-drinking arms view model (`models/arms_drinking.glb`) — a pair of
//! arms raising a bottle and drinking from it, played once when the player
//! buys a perk. Buying one (`zombies_hud::sync_shroom_perk`) sets
//! [`PerkDrink::requested`]; `weapon::weapon_system` stows the equipped weapon
//! (the throwing knife's quick Hide), then [`play_perk_drink`] shows the arms
//! and plays the drink once, and the weapon is drawn again after. The debug
//! panel's "Drinking arms" section can also show them looping, for tuning.
//!
//! Only part of the glb's single clip is used: frames
//! [`CLIP_START_FRAME`]..[`CLIP_END_FRAME`] (at [`CLIP_FPS`]), played at
//! [`DrinkArmsSettings::speed`]× — the same trim and speed as the original
//! three.js version (`AnimationUtils.subclip(clip, "drink", 160, 410)` at a
//! time scale of 5), so the lead-in and wind-down never play.
//!
//! The bottle (`Object_15`) is tinted the perk's colour, with a faint glow of
//! the same colour; the label (`Object_16`) is left alone.

use std::f32::consts::PI;

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy::scene::SceneInstanceReady;

use crate::death_effect::DeathEffect;
use crate::fall_death::FallDeathState;
use crate::killcam::ActiveKillCam;
use crate::VIEW_MODEL_RENDER_LAYER;

/// First / last frame of the glb's clip that's played, and the frame rate
/// they're counted at (three.js `subclip`'s default).
const CLIP_START_FRAME: f32 = 160.0;
const CLIP_END_FRAME: f32 = 410.0;
const CLIP_FPS: f32 = 30.0;
const CLIP_START: f32 = CLIP_START_FRAME / CLIP_FPS;
const CLIP_END: f32 = CLIP_END_FRAME / CLIP_FPS;

/// The glb's bottle mesh node.
const BOTTLE_NODE: &str = "Object_15";

/// Shroom Tea's bottle colour (sRGB `#6a1fbf`).
pub(crate) const SHROOM_TEA_BOTTLE: Color = Color::srgb(
    0x6a as f32 / 255.0,
    0x1f as f32 / 255.0,
    0xbf as f32 / 255.0,
);

pub(crate) struct DrinkArmsPlugin;

impl Plugin for DrinkArmsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DrinkArmsSettings>()
            .init_resource::<PerkDrink>()
            .add_systems(Update, (play_perk_drink, tint_drink_bottle));
    }
}

/// Where a perk drink is up to — see the module docs.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum DrinkPhase {
    #[default]
    Idle,
    /// The equipped weapon's quick Hide is playing (`weapon_system`).
    Stowing,
    /// The arms are up, playing the drink once ([`play_perk_drink`]).
    Drinking,
    /// The drink's over; `weapon_system` draws the weapon again.
    Done,
}

/// A perk drink's progress. Reset on entering a game (`reset_weapon`) and on
/// respawn (`respawn::reset_on_respawn`).
#[derive(Resource, Default)]
pub(crate) struct PerkDrink {
    /// A perk was just bought; `weapon_system` starts the drink as soon as
    /// nothing else (a throwing-knife sequence) is under way.
    pub(crate) requested: bool,
    pub(crate) phase: DrinkPhase,
    /// Seconds spent in [`DrinkPhase::Stowing`] (see `stow_finished`).
    pub(crate) stow_elapsed: f32,
    /// The drink clip has been started this [`DrinkPhase::Drinking`].
    started: bool,
}

/// The loaded `models/arms_drinking.glb` scene root.
#[derive(Component)]
pub(crate) struct DrinkArmsViewModel;

/// Handles + node index for the arms' single clip — mirrors
/// `ThrowArmsAnimation`.
#[derive(Component)]
pub(crate) struct DrinkArmsAnimation {
    pub(crate) graph: Handle<AnimationGraph>,
    pub(crate) index: AnimationNodeIndex,
}

/// Set by `start_drink_arms_animation` on the descendant entity that carries
/// the arms' `AnimationPlayer`.
#[derive(Component)]
pub(crate) struct DrinkArmsAnimationPlayer;

/// The bottle's own (cloned) material, so tinting it touches nothing else.
#[derive(Component)]
struct DrinkBottle(Handle<StandardMaterial>);

/// Live-tunable drinking arms — the debug panel's "Drinking arms" section.
/// Angles are in degrees (Bevy's `YXZ` order) on top of a fixed half-turn,
/// like `ThrowArmsSettings`.
#[derive(Resource)]
pub(crate) struct DrinkArmsSettings {
    pub(crate) translation: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) roll: f32,
    pub(crate) scale: f32,
    /// Playback speed of the trimmed clip (5 = the three.js version's).
    pub(crate) speed: f32,
    /// The bottle's colour — Shroom Tea's purple, the only perk for now.
    pub(crate) bottle_color: Color,
    /// Strength of the bottle's glow in its own colour (three.js'
    /// `emissiveIntensity`).
    pub(crate) glow: f32,
    /// Debug: show the arms, looping the drink (when not drinking for real).
    pub(crate) show: bool,
}

impl Default for DrinkArmsSettings {
    fn default() -> Self {
        Self {
            translation: Vec3::new(-0.05, 0.01, -0.01),
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            scale: 1.2,
            speed: 5.0,
            bottle_color: SHROOM_TEA_BOTTLE,
            glow: 4.0,
            show: false,
        }
    }
}

impl DrinkArmsSettings {
    pub(crate) fn transform(&self) -> Transform {
        Transform {
            translation: self.translation,
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

/// Once the arms scene has spawned: put it on the view-model layer, give the
/// bottle its own material to tint, and arm the animation player parked on
/// the trimmed clip's first frame.
pub(crate) fn start_drink_arms_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    arms: Query<&DrinkArmsAnimation>,
    names: Query<&Name>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
            // `start_view_model_animation`.
            NoFrustumCulling,
        ));

        // The bottle node's mesh is a child entity of it (one per primitive).
        if names.get(entity).is_ok_and(|n| n.as_str() == BOTTLE_NODE) {
            for mesh in children.iter_descendants(entity) {
                let Ok(mat) = mesh_materials.get(mesh) else { continue };
                if let Some(own) = materials.get(&mat.0).cloned() {
                    let own = materials.add(own);
                    commands
                        .entity(mesh)
                        .insert((MeshMaterial3d(own.clone()), DrinkBottle(own)));
                }
            }
        }

        if let Ok(mut player) = players.get_mut(entity) {
            let active = player.play(anim.index);
            active.set_repeat(RepeatAnimation::Never);
            active.seek_to(CLIP_START);
            active.pause();
            commands.entity(entity).insert((
                AnimationGraphHandle(anim.graph.clone()),
                DrinkArmsAnimationPlayer,
            ));
        }
    }
}

/// Pose the arms from the settings every frame, and run them: during a
/// perk drink, show them and play the trimmed clip once at `speed`, then hide
/// them and hand back to `weapon_system` ([`DrinkPhase::Done`]). Otherwise
/// the debug panel's "show" loops the clip; when neither, they're hidden,
/// parked on the trimmed clip's first frame. Hidden during a death / fall
/// effect or a kill cam.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play_perk_drink(
    settings: Res<DrinkArmsSettings>,
    mut drink: ResMut<PerkDrink>,
    killcam: Res<ActiveKillCam>,
    death: Res<DeathEffect>,
    fall: Res<FallDeathState>,
    arms: Single<(&mut Transform, &mut Visibility), With<DrinkArmsViewModel>>,
    anim: Single<&DrinkArmsAnimation>,
    mut players: Query<&mut AnimationPlayer, With<DrinkArmsAnimationPlayer>>,
) {
    let (mut transform, mut vis) = arms.into_inner();
    *transform = settings.transform();
    let suppressed =
        killcam.0.is_some() || death.is_active() || crate::fall_death::effect_active(fall);
    let drinking = drink.phase == DrinkPhase::Drinking;
    vis.set_if_neq(if !suppressed && (drinking || settings.show) {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });

    // `None` while the arms scene is still loading: the drink then ends
    // straight away rather than leaving the player weaponless.
    let Some(mut player) = players.iter_mut().next() else {
        if drinking {
            drink.phase = DrinkPhase::Done;
        }
        return;
    };
    let active = player.play(anim.index);
    if drinking {
        if !drink.started {
            drink.started = true;
            active.set_speed(settings.speed);
            active.seek_to(CLIP_START);
            active.resume();
        } else if active.seek_time() >= CLIP_END {
            drink.started = false;
            drink.phase = DrinkPhase::Done;
            active.seek_to(CLIP_START);
            active.pause();
        }
        return;
    }
    if !settings.show {
        if !active.is_paused() || active.seek_time() != CLIP_START {
            active.seek_to(CLIP_START);
            active.pause();
        }
        return;
    }
    active.set_speed(settings.speed);
    active.resume();
    let t = active.seek_time();
    if t >= CLIP_END {
        active.seek_to(CLIP_START + (t - CLIP_END) % (CLIP_END - CLIP_START));
    } else if t < CLIP_START {
        active.seek_to(CLIP_START);
    }
}

/// Keep the bottle its perk colour, with a faint glow of the same colour.
fn tint_drink_bottle(
    settings: Res<DrinkArmsSettings>,
    bottles: Query<&DrinkBottle>,
    added: Query<(), Added<DrinkBottle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !settings.is_changed() && added.is_empty() {
        return;
    }
    for bottle in &bottles {
        if let Some(mat) = materials.get_mut(&bottle.0) {
            mat.base_color = settings.bottle_color;
            mat.emissive = settings.bottle_color.to_linear() * settings.glow;
        }
    }
}
