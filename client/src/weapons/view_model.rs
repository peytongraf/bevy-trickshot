//! The sniper's view-model rig: its baked animation clip sliced into named
//! segments, the hip/ADS pose blend, and the one-time setup that arms its
//! `AnimationPlayer` once the scene loads.

use std::f32::consts::PI;

use bevy::animation::RepeatAnimation;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy::scene::SceneInstanceReady;

use crate::VIEW_MODEL_RENDER_LAYER;

use super::scope::{LensSettings, ScopeLens, ScopeRenderTarget};

/// Frame rate `sniper.glb`'s `allanims` clip was baked at in Blender. If the
/// segment cuts below look off, check the console on startup: the game logs
/// the clip's real duration and the fps a 156-frame timeline would imply.
pub(crate) const ANIM_FPS: f32 = 24.0;

/// Indices into `SEGMENTS`.
pub(crate) const SEG_SHOOT: usize = 0;
pub(crate) const SEG_RECHAMBER: usize = 1;
pub(crate) const SEG_RELOAD: usize = 2;
pub(crate) const SEG_HIDE: usize = 3;
pub(crate) const SEG_SHOW: usize = 4;

/// The individual animations packed into the single `allanims` clip, as
/// `[start, end)` frame ranges lifted straight from the Blender timeline.
/// Adjust these until every section is exactly right, then rebuild.
pub(crate) const SEGMENTS: [AnimationSegment; 7] = [
    AnimationSegment::new("Shoot", 0.0, 9.0, ANIM_FPS),
    AnimationSegment::new("Rechamber", 9.0, 48.0, ANIM_FPS),
    AnimationSegment::new("Reload", 48.0, 92.0, ANIM_FPS),
    AnimationSegment::new("Hide", 92.0, 101.0, ANIM_FPS),
    AnimationSegment::new("Show", 101.0, 113.0, ANIM_FPS),
    AnimationSegment::new("Adjust Grip", 113.0, 132.0, ANIM_FPS),
    AnimationSegment::new("Melee", 132.0, 156.0, ANIM_FPS),
];

#[derive(Clone, Copy)]
pub(crate) struct AnimationSegment {
    pub(crate) name: &'static str,
    start_frame: f32,
    end_frame: f32,
    /// Frames/second the source clip was exported at. Not a shared global —
    /// `sniper.glb` (`ANIM_FPS`, 24) and `knife.glb` (`KNIFE_ANIM_FPS`, 30)
    /// were authored at different Blender scene rates, so each segment
    /// carries its own instead of assuming one constant for every clip.
    fps: f32,
}

impl AnimationSegment {
    pub(crate) const fn new(
        name: &'static str,
        start_frame: f32,
        end_frame: f32,
        fps: f32,
    ) -> Self {
        Self {
            name,
            start_frame,
            end_frame,
            fps,
        }
    }

    pub(crate) fn start_secs(&self) -> f32 {
        self.start_frame / self.fps
    }

    pub(crate) fn end_secs(&self) -> f32 {
        self.end_frame / self.fps
    }
}

/// Play `seg` from its start frame at normal speed, not looping.
pub(crate) fn play_segment(
    player: &mut AnimationPlayer,
    node: AnimationNodeIndex,
    seg: AnimationSegment,
) {
    let active = player.play(node);
    active.set_repeat(RepeatAnimation::Never);
    active.set_speed(1.0);
    active.replay();
    active.seek_to(seg.start_secs());
    active.resume();
}

/// The loaded sniper scene root.
#[derive(Component)]
pub(crate) struct ViewModel;

/// Handles + node index for the sniper's single animation clip.
#[derive(Component)]
pub(crate) struct ViewModelAnimation {
    pub(crate) graph: Handle<AnimationGraph>,
    pub(crate) index: AnimationNodeIndex,
}

/// Set by `start_view_model_animation` on the descendant entity that carries
/// the sniper's `AnimationPlayer`. Bots have their own `AnimationPlayer`s now
/// (see [`BotAnimationPlayer`]), so anywhere that used to assume "the"
/// `AnimationPlayer` in the world was the sniper's needs to filter on this.
#[derive(Component)]
pub(crate) struct SniperAnimationPlayer;

/// Panel-adjustable playback speeds for the view-model animation segments
/// ("Animations" panel section). `1.0` is the clip's authored speed.
#[derive(Resource)]
pub(crate) struct AnimationSettings {
    /// Speed multiplier for the Rechamber segment played after a shot.
    pub(crate) rechamber_speed: f32,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            rechamber_speed: 1.7,
        }
    }
}

/// A local pose for the view model (hip or ADS).
#[derive(Clone, Copy)]
pub(crate) struct ViewModelOffset {
    pub(crate) translation: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) scale: f32,
}

impl ViewModelOffset {
    pub(crate) fn transform(&self) -> Transform {
        Transform {
            translation: self.translation,
            rotation: Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0),
            scale: Vec3::splat(self.scale),
        }
    }
}

/// Interpolate between two poses and build the resulting transform.
pub(crate) fn lerp_pose(a: &ViewModelOffset, b: &ViewModelOffset, t: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation: Quat::from_euler(
            EulerRot::YXZ,
            a.yaw.lerp(b.yaw, t),
            a.pitch.lerp(b.pitch, t),
            0.0,
        ),
        scale: Vec3::splat(a.scale.lerp(b.scale, t)),
    }
}

/// The two view-model poses ADS blends between.
#[derive(Resource)]
pub(crate) struct ViewModelPoses {
    pub(crate) hip: ViewModelOffset,
    pub(crate) ads: ViewModelOffset,
}

impl Default for ViewModelPoses {
    fn default() -> Self {
        Self {
            hip: ViewModelOffset {
                translation: Vec3::new(0.13, -0.24, -0.4),
                yaw: PI,
                pitch: 0.0,
                scale: 0.01,
            },
            // Gun pulled onto the camera axis so the sight lines up dead centre.
            // Tuned in-game with the ADS panel sliders.
            ads: ViewModelOffset {
                translation: Vec3::new(-0.00008, -0.1721, -0.18),
                yaw: PI,
                pitch: 0.0,
                scale: 0.01,
            },
        }
    }
}

/// Once the sniper scene has spawned, drop every entity it created onto the
/// view-model render layer and arm the animation player, parked (paused) on the
/// first frame until `L` is pressed.
pub(crate) fn start_view_model_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    view_models: Query<&ViewModelAnimation>,
    mut players: Query<&mut AnimationPlayer>,
    names: Query<&Name>,
    meshes: Query<(), With<Mesh3d>>,
    scope_rt: Res<ScopeRenderTarget>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let root = trigger.target();
    let Ok(anim) = view_models.get(root) else {
        return;
    };

    let mut lens_count = 0;
    // Starting look only — `update_scope` rewrites the lens material from the
    // live `LensSettings` every frame.
    let lens_cfg = LensSettings::default();

    for entity in children.iter_descendants(root) {
        commands.entity(entity).insert((
            RenderLayers::layer(VIEW_MODEL_RENDER_LAYER),
            // A skinned mesh is frustum-culled against its *rest-pose* AABB, not
            // the animated one. The re-exported glb's arms rest pose sits behind
            // the near plane, so the arms were being culled from the view-model
            // camera while still showing up in the (light-frustum) shadow pass.
            // The view model is glued to the camera anyway, so culling it per
            // frame buys nothing — just switch it off.
            NoFrustumCulling,
        ));

        let lower_name = names
            .get(entity)
            .ok()
            .map(|n| n.as_str().to_ascii_lowercase());
        let name_has = |needle: &str| lower_name.as_deref().is_some_and(|n| n.contains(needle));
        let is_mesh = meshes.contains(entity);

        // The scope's rear lens ("lens_lens_0" in Blender): a reflective glass
        // disc at the hip, carrying the scope render target on `emissive` so
        // `update_scope` can fade the sight picture in while scoping.
        if is_mesh && name_has("lens") {
            commands.entity(entity).insert((
                ScopeLens,
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(lens_cfg.tint[0], lens_cfg.tint[1], lens_cfg.tint[2]),
                    emissive_texture: Some(scope_rt.0.clone()),
                    emissive: LinearRgba::BLACK,
                    // The render target samples V-flipped on the lens; undo it.
                    uv_transform: Affine2::from_scale_angle_translation(
                        Vec2::new(1.0, -1.0),
                        0.0,
                        Vec2::new(0.0, 1.0),
                    ),
                    perceptual_roughness: lens_cfg.roughness,
                    metallic: lens_cfg.metallic,
                    reflectance: lens_cfg.reflectance,
                    alpha_mode: AlphaMode::Blend,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })),
                Visibility::Hidden,
            ));
            lens_count += 1;
        }

        if let Ok(mut player) = players.get_mut(entity) {
            let active = player.play(anim.index);
            active.set_repeat(RepeatAnimation::Never);
            active.seek_to(0.0);
            active.pause();
            commands.entity(entity).insert((
                AnimationGraphHandle(anim.graph.clone()),
                // Bots now carry their own `AnimationPlayer`s too, so anything
                // that used to grab "the" `AnimationPlayer` unfiltered needs
                // this to pick the sniper's back out.
                SniperAnimationPlayer,
            ));
        }
    }

    info!("view model ready: tagged {lens_count} scope-lens mesh(es)");
}
