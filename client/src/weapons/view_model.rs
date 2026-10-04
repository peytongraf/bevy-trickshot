//! The primary weapon's view-model rig: the sniper's baked animation clip
//! sliced into named segments (the AK-74's are whole clips — `ak`), the
//! hip/ADS pose blend, the one-time setup that arms its `AnimationPlayer`
//! once the scene loads, and [`PrimaryRig`], which every animation the
//! weapon system plays on the primary goes through.

use std::f32::consts::PI;
use std::time::Duration;

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::RepeatAnimation;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::view::{NoFrustumCulling, RenderLayers};
use bevy::scene::SceneInstanceReady;

use crate::VIEW_MODEL_RENDER_LAYER;
use shared::weapon::WeaponId;

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
    AnimationSegment::new("Shoot", 0.0, 9.0, ANIM_FPS).with_act(SegAct::Shoot),
    AnimationSegment::new("Rechamber", 9.0, 48.0, ANIM_FPS).with_act(SegAct::Rechamber),
    AnimationSegment::new("Reload", 48.0, 92.0, ANIM_FPS).with_act(SegAct::Reload),
    AnimationSegment::new("Hide", 92.0, 101.0, ANIM_FPS).with_act(SegAct::Hide),
    AnimationSegment::new("Show", 101.0, 113.0, ANIM_FPS).with_act(SegAct::Show),
    AnimationSegment::new("Adjust Grip", 113.0, 132.0, ANIM_FPS),
    AnimationSegment::new("Melee", 132.0, 156.0, ANIM_FPS),
];

/// What a primary-weapon segment does, whichever weapon's it is — the
/// weapon system keys its sounds, speeds and interrupt-and-resume off this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SegAct {
    /// Anything else (the knife's, the AK's idle / walk / jump clips...).
    Other,
    Shoot,
    /// The sniper's bolt cycle.
    Rechamber,
    /// A reload with a round still chambered (the sniper's mag swap — then a
    /// `Rechamber` if the mag was empty — or the AK's fast reload).
    Reload,
    /// The AK's empty-mag reload: a fresh mag *and* a round chambered.
    ReloadEmpty,
    Hide,
    Show,
}

#[derive(Clone, Copy)]
pub(crate) struct AnimationSegment {
    /// What it's called (in Blender / the glb) — for reading the code by.
    #[allow(dead_code)]
    pub(crate) name: &'static str,
    /// Which clip of its model it's in (an index into
    /// [`ViewModelAnimation::nodes`]) — always 0 for the one-clip sniper and
    /// knife.
    pub(crate) clip: usize,
    pub(crate) act: SegAct,
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
            clip: 0,
            act: SegAct::Other,
            start_frame,
            end_frame,
            fps,
        }
    }

    /// A whole clip `secs` long, clip `clip` of its model.
    pub(crate) const fn clip(name: &'static str, clip: usize, secs: f32) -> Self {
        Self {
            name,
            clip,
            act: SegAct::Other,
            start_frame: 0.0,
            end_frame: secs,
            fps: 1.0,
        }
    }

    pub(crate) const fn with_act(mut self, act: SegAct) -> Self {
        self.act = act;
        self
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

/// [`play_segment`] at `speed`× (Nitro Brew's reload / swap speed-ups).
pub(crate) fn play_segment_at(
    player: &mut AnimationPlayer,
    node: AnimationNodeIndex,
    seg: AnimationSegment,
    speed: f32,
) {
    play_segment(player, node, seg);
    if let Some(active) = player.animation_mut(node) {
        active.set_speed(speed);
    }
}

/// The primary weapon's view-model scene root (the sniper's, or the
/// AK-74's — `ak::sync_primary_model` swaps the scene in place).
#[derive(Component)]
pub(crate) struct ViewModel;

/// Which primary the view model is wearing, and its animation graph: the
/// sniper's single clip, or one node per AK-74 clip.
#[derive(Component)]
pub(crate) struct ViewModelAnimation {
    pub(crate) graph: Handle<AnimationGraph>,
    /// The first clip's node (the sniper's only one).
    pub(crate) index: AnimationNodeIndex,
    /// Every clip's node, by [`AnimationSegment::clip`].
    pub(crate) nodes: Vec<AnimationNodeIndex>,
    pub(crate) weapon: WeaponId,
}

/// Plays the primary weapon's animations, whichever it is: the sniper's
/// segments of its one clip, seeked straight to (as ever), or the AK-74's
/// separate clips, cross-faded through its [`AnimationTransitions`] so they
/// flow into each other.
pub(crate) struct PrimaryRig<'a> {
    pub(crate) player: Mut<'a, AnimationPlayer>,
    transitions: Option<Mut<'a, AnimationTransitions>>,
    nodes: &'a [AnimationNodeIndex],
    pub(crate) weapon: WeaponId,
    blend: Duration,
}

impl<'a> PrimaryRig<'a> {
    pub(crate) fn new(
        player: Mut<'a, AnimationPlayer>,
        transitions: Option<Mut<'a, AnimationTransitions>>,
        anim: &'a ViewModelAnimation,
        blend_secs: f32,
    ) -> Self {
        Self {
            player,
            transitions,
            nodes: &anim.nodes,
            weapon: anim.weapon,
            blend: Duration::from_secs_f32(blend_secs.max(0.0)),
        }
    }

    fn node(&self, seg: AnimationSegment) -> AnimationNodeIndex {
        self.nodes.get(seg.clip).or(self.nodes.first()).copied().unwrap_or_default()
    }

    /// This weapon's segment for `act`.
    pub(crate) fn seg(&self, act: SegAct) -> AnimationSegment {
        match self.weapon {
            WeaponId::Ak74 => super::ak::ak_seg(act),
            WeaponId::RayGun => super::raygun::raygun_seg(act),
            _ => sniper_seg(act),
        }
    }

    /// Play `seg` once from its start at normal speed.
    pub(crate) fn play(&mut self, seg: AnimationSegment) {
        self.play_at(seg, 1.0);
    }

    /// Play `seg` once from its start at `speed`×.
    pub(crate) fn play_at(&mut self, seg: AnimationSegment, speed: f32) {
        self.play_with(seg, speed, RepeatAnimation::Never, self.blend);
    }

    /// Play `seg` from its start at `speed`×, repeating as `repeat` says,
    /// cross-fading over `blend` (AK) — the sniper just seeks.
    pub(crate) fn play_with(&mut self, seg: AnimationSegment, speed: f32, repeat: RepeatAnimation, blend: Duration) {
        let node = self.node(seg);
        let Self { player, transitions, .. } = self;
        match transitions {
            Some(t) => {
                let active = t.play(player, node, blend);
                active.set_repeat(repeat);
                active.set_speed(speed);
                active.seek_to(seg.start_secs());
                active.resume();
                active.set_weight(1.0);
            }
            None => {
                play_segment_at(player, node, seg, speed);
                if let Some(active) = player.animation_mut(node) {
                    active.set_repeat(repeat);
                }
            }
        }
    }

    pub(crate) fn set_speed(&mut self, seg: AnimationSegment, speed: f32) {
        let node = self.node(seg);
        if let Some(active) = self.player.animation_mut(node) {
            active.set_speed(speed);
        }
    }

    /// Whether `seg` has played to `end` (clip seconds) — or isn't playing.
    pub(crate) fn reached(&self, seg: AnimationSegment, end: f32) -> bool {
        self.player
            .animation(self.node(seg))
            .is_none_or(|a| a.is_finished() || a.seek_time() >= end)
    }

    /// Whether `seg` is the clip currently playing (the AK's main
    /// animation). Always true for the one-clip sniper.
    pub(crate) fn is_main(&self, seg: AnimationSegment) -> bool {
        let node = self.node(seg);
        self.transitions
            .as_ref()
            .is_none_or(|t| t.get_main_animation() == Some(node))
    }

    /// Back to rest with nothing playing — the sniper parks on its first
    /// frame. The AK's locomotion (`ak::ak_locomotion`) takes over from
    /// whatever it was last playing instead, so this leaves it be.
    pub(crate) fn park(&mut self) {
        if self.transitions.is_some() {
            return;
        }
        let node = self.node(SEGMENTS[0]);
        if let Some(active) = self.player.animation_mut(node) {
            active.seek_to(0.0);
            active.pause();
        }
    }
}

/// The sniper's segment for `act`.
fn sniper_seg(act: SegAct) -> AnimationSegment {
    SEGMENTS[match act {
        SegAct::Rechamber => SEG_RECHAMBER,
        SegAct::Reload | SegAct::ReloadEmpty => SEG_RELOAD,
        SegAct::Hide => SEG_HIDE,
        SegAct::Show => SEG_SHOW,
        SegAct::Shoot | SegAct::Other => SEG_SHOOT,
    }]
}

/// Set by `start_view_model_animation` on the descendant entity that carries
/// the primary weapon's `AnimationPlayer` (the sniper's or the AK-74's — the
/// name predates the AK). Remote players and bots have their own
/// `AnimationPlayer`s (see [`crate::SoldierAnimationPlayer`]), so anywhere that used to assume "the"
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

/// Once the primary's scene has spawned, drop every entity it created onto
/// the view-model render layer and arm the animation player: the sniper's
/// parked (paused) on its first frame, the AK-74's looping its idle, with
/// [`AnimationTransitions`] to cross-fade every clip after.
pub(crate) fn start_view_model_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    view_models: Query<&ViewModelAnimation>,
    mut players: Query<&mut AnimationPlayer>,
    names: Query<&Name>,
    meshes: Query<(), With<Mesh3d>>,
    transforms: Query<&Transform>,
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
        // `update_scope` can fade the sight picture in while scoping. (The
        // AK-74 has iron sights — no scope.)
        if anim.weapon == WeaponId::Sniper && is_mesh && name_has("lens") {
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

        // The AK's spent shell (`ak::pin_ak_shell` holds it at rest between
        // shots).
        if anim.weapon == WeaponId::Ak74 && name_has(super::ak::AK_SHELL_NODE) {
            if let Ok(tf) = transforms.get(entity) {
                commands.entity(entity).insert(super::ak::AkShell { rest: *tf });
            }
        }

        if let Ok(mut player) = players.get_mut(entity) {
            if anim.weapon != WeaponId::Sniper {
                let mut transitions = AnimationTransitions::new();
                let idle = if anim.weapon == WeaponId::RayGun {
                    super::raygun::raygun_idle_seg()
                } else {
                    super::ak::ak_idle_seg()
                };
                let node = anim.nodes.get(idle.clip).copied().unwrap_or(anim.index);
                transitions
                    .play(&mut player, node, Duration::ZERO)
                    .set_repeat(RepeatAnimation::Forever);
                commands.entity(entity).insert(transitions);
            } else {
                let active = player.play(anim.index);
                active.set_repeat(RepeatAnimation::Never);
                active.seek_to(0.0);
                active.pause();
            }
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
