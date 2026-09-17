//! Crouch / slide / dive / prone state machine (Call-of-Duty style), and the
//! shadow-only body capsule that follows the same crouch/prone depth.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::keybinds::KeyBindings;
use crate::killcam;
use crate::{GameSounds, Weapon, WeaponSlot};

use super::camera::PlayerHead;
use super::movement::{
    Player, PlayerPhysics, Sprinting, BODY_CAPSULE_HEIGHT, BODY_CAPSULE_RADIUS, EYE_HEIGHT,
    SECONDARY_MOVE_MULT,
};

/// Defaults for the "Slide" / "Dive & prone" panel sections.
pub(crate) const CROUCH_DROP: f32 = 0.8;
pub(crate) const CROUCH_SPEED: f32 = 2.5;
pub(crate) const SLIDE_DUCK_SPEED: f32 = 16.0;
pub(crate) const SLIDE_STAND_SPEED: f32 = 7.0;
pub(crate) const SLIDE_SPEED: f32 = 9.0;
pub(crate) const SLIDE_SPRINT_BONUS: f32 = 4.5;
pub(crate) const SLIDE_FRICTION: f32 = 7.5;
pub(crate) const SLIDE_MIN_SPEED: f32 = 1.6;
pub(crate) const SLIDE_MAX_TIME: f32 = 1.6;
pub(crate) const PRONE_DROP: f32 = 1.35;
pub(crate) const PRONE_SPEED: f32 = 0.7;
pub(crate) const DIVE_SPEED: f32 = 8.5;
pub(crate) const DIVE_JUMP: f32 = 8.5;
pub(crate) const DIVE_TUCK_SPEED: f32 = 25.0;
/// A dolphin dive also counts as "landed" once the tucked camera drops within
/// this of the surface — the belly hits before the standing feet would.
pub(crate) const DIVE_CLEARANCE: f32 = 0.3;

/// What the player's lower body is doing. `Standing` is the normal state;
/// `Crouching` is a slow ducked walk; `Sliding` is a momentum slide that decays
/// to a stop; `Diving` is the airborne half of a dolphin dive; `Prone` is flat
/// on the ground.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stance {
    #[default]
    Standing,
    Crouching,
    Sliding,
    Diving,
    Prone,
}

/// Live crouch / slide / dive / prone state. `drop` is the current camera Y
/// offset from standing (metres, ≤ 0); `crouch_slide` eases it toward the target
/// for the stance and writes it onto the head, so every stance change is a
/// smooth move rather than a jerk.
#[derive(Resource, Default)]
pub(crate) struct Slide {
    pub(crate) stance: Stance,
    pub(crate) drop: f32,
    /// Horizontal slide velocity (m/s); only meaningful while `Sliding`.
    pub(crate) velocity: Vec3,
    /// Seconds the current slide has run.
    timer: f32,
    /// The one-shot slide-sound entity, kept so a cancel can cut it short.
    sound: Option<Entity>,
    /// Set for the frame a slide/crouch/prone swallowed the jump press, so
    /// `jump` doesn't also launch.
    pub(crate) ate_jump: bool,
    /// Stance a `Prone` toggle returns to — `Standing` or `Crouching`.
    prone_from: Stance,
}

/// Panel-adjustable crouch / slide / dive tuning.
#[derive(Resource)]
pub(crate) struct SlideSettings {
    /// How far the camera drops when fully crouched (m).
    pub(crate) crouch_drop: f32,
    /// Move speed while crouch-walking (m/s) — slower than `walk_speed`.
    pub(crate) crouch_speed: f32,
    /// How fast the camera ducks down entering a crouch / slide (1/s).
    pub(crate) duck_speed: f32,
    /// How fast the camera rises back up (1/s) — kept lower so it's smooth.
    pub(crate) stand_speed: f32,
    /// Slide launch speed when starting from a walk (m/s).
    pub(crate) slide_speed: f32,
    /// Added to the launch speed when the slide starts from a sprint (m/s).
    pub(crate) sprint_bonus: f32,
    /// Deceleration that bleeds a slide off (m/s²).
    pub(crate) friction: f32,
    /// The slide ends and the player stands once it drops below this (m/s).
    pub(crate) min_speed: f32,
    /// Hard cap on slide duration regardless of friction (s).
    pub(crate) max_time: f32,
    /// How far the camera drops when prone (m).
    pub(crate) prone_drop: f32,
    /// Move speed while crawling prone (m/s).
    pub(crate) prone_speed: f32,
    /// Forward launch speed of a dolphin dive (m/s); a faster approach carries in.
    pub(crate) dive_speed: f32,
    /// Upward hop of a dolphin dive (m/s).
    pub(crate) dive_jump: f32,
    /// How fast the camera tucks toward prone during a dive (1/s).
    pub(crate) dive_tuck_speed: f32,
}

impl Default for SlideSettings {
    fn default() -> Self {
        Self {
            crouch_drop: CROUCH_DROP,
            crouch_speed: CROUCH_SPEED,
            duck_speed: SLIDE_DUCK_SPEED,
            stand_speed: SLIDE_STAND_SPEED,
            slide_speed: SLIDE_SPEED,
            sprint_bonus: SLIDE_SPRINT_BONUS,
            friction: SLIDE_FRICTION,
            min_speed: SLIDE_MIN_SPEED,
            max_time: SLIDE_MAX_TIME,
            prone_drop: PRONE_DROP,
            prone_speed: PRONE_SPEED,
            dive_speed: DIVE_SPEED,
            dive_jump: DIVE_JUMP,
            dive_tuck_speed: DIVE_TUCK_SPEED,
        }
    }
}

/// The invisible shadow-only body capsule, another direct child of `Player`
/// (a sibling of `PlayerHead`, not under it — it must stay level and shouldn't
/// pitch with the camera). [`crouch_slide`] repositions/reshapes it each frame
/// via [`body_capsule_pose`] to match the current crouch/prone depth.
#[derive(Component)]
pub(crate) struct PlayerBodyCapsule;

/// Wipe crouch / slide state so re-entering the world always starts upright.
pub(crate) fn reset_slide(
    mut slide: ResMut<Slide>,
    mut head: Single<&mut Transform, With<PlayerHead>>,
) {
    *slide = Slide::default();
    head.translation.y = 0.0;
}

/// Local transform (relative to `Player`) for the shadow-only body capsule at
/// a given crouch/prone depth. `drop` is [`Slide::drop`] — `0` standing, easing
/// to `-cfg.crouch_drop` crouched/sliding, then on to `-cfg.prone_drop`
/// prone/diving — so the capsule settles in step with the camera instead of
/// popping between poses.
///
/// Standing → crouched squashes the capsule's height (bottom anchored to the
/// ground, so it reads as bent knees rather than sinking through the floor).
/// Crouched → prone rotates it flat, long axis forward, resting on its belly.
/// Both legs of the blend share the same ground line (`-EYE_HEIGHT` in
/// `Player`-local space, matching the standing pose already spawned in
/// `setup_player`) so nothing ever floats or clips.
pub(crate) fn body_capsule_pose(drop: f32, cfg: &SlideSettings) -> Transform {
    let depth = -drop; // 0 standing .. crouch_drop crouched .. prone_drop prone
    let crouch_t = (depth / cfg.crouch_drop.max(1.0e-4)).clamp(0.0, 1.0);
    let prone_t = ((depth - cfg.crouch_drop) / (cfg.prone_drop - cfg.crouch_drop).max(1.0e-4))
        .clamp(0.0, 1.0);

    // Standing (scale 1, upright) eased toward a squashed crouch as `crouch_t`
    // climbs to 1 — never thinner than twice the radius, so it doesn't invert.
    let crouched_height = (BODY_CAPSULE_HEIGHT - cfg.crouch_drop).max(BODY_CAPSULE_RADIUS * 2.0);
    let height = BODY_CAPSULE_HEIGHT.lerp(crouched_height, crouch_t);
    let upright = Transform {
        translation: Vec3::new(0.0, height / 2.0 - EYE_HEIGHT, 0.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(1.0, height / BODY_CAPSULE_HEIGHT, 1.0),
    };

    // Flat on the ground, long axis along local -Z (forward) so it points the
    // way the player (and thus this capsule's parent) is facing.
    let prone = Transform {
        translation: Vec3::new(0.0, BODY_CAPSULE_RADIUS - EYE_HEIGHT, 0.0),
        rotation: Quat::from_rotation_x(-FRAC_PI_2),
        scale: Vec3::ONE,
    };

    Transform {
        translation: upright.translation.lerp(prone.translation, prone_t),
        rotation: upright.rotation.slerp(prone.rotation, prone_t),
        scale: upright.scale.lerp(prone.scale, prone_t),
    }
}

/// Crouch / slide / dive / prone state machine (Call-of-Duty style).
///
/// Crouch/slide key (`C` by default):
/// * standing + still → **crouch** (slow ducked walk, no sprint);
/// * standing + moving → **slide** along the travel direction — fast, then
///   friction bleeds it off and the player smoothly stands. A slide from a
///   sprint carries extra speed; you can't slide while already crouched.
/// * jump cancels a slide on the spot / stands you up out of a crouch.
///
/// Prone/dive key (`Left Ctrl` by default):
/// * still → toggle **prone** ⇄ whatever you were (standing or crouched);
/// * standing + moving → **dolphin dive**: a forward hop that lands prone, the
///   dive-sound firing on impact. Can't dive while crouched.
#[allow(clippy::too_many_arguments)]
pub(crate) fn crouch_slide(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    cfg: Res<SlideSettings>,
    sounds: Res<GameSounds>,
    weapon: Res<Weapon>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    sprinting: Res<Sprinting>,
    mut slide: ResMut<Slide>,
    player: Single<(&Transform, &mut PlayerPhysics), With<Player>>,
    mut head: Single<&mut Transform, (With<PlayerHead>, Without<Player>)>,
    mut capsule: Single<
        &mut Transform,
        (
            With<PlayerBodyCapsule>,
            Without<Player>,
            Without<PlayerHead>,
        ),
    >,
    mut commands: Commands,
) {
    let dt = time.delta_secs().max(1.0e-5);
    slide.ate_jump = false;
    // The lighter secondary bumps slide-launch and dive speed the same way it
    // bumps walking (see `move_player`).
    let weapon_mult = if weapon.slot == WeaponSlot::Secondary {
        SECONDARY_MOVE_MULT
    } else {
        1.0
    };

    let locked = window.cursor_options.grab_mode != CursorGrabMode::None;
    let crouch_pressed = locked && binds.crouch.just_pressed(&keys, &mouse);
    let prone_pressed = locked && binds.prone.just_pressed(&keys, &mouse);
    let jump_pressed = locked && binds.jump.just_pressed(&keys, &mouse);
    // `toggle_sprint` (runs first) has already flipped sprint on for this press.
    let sprint_pressed = locked && binds.sprint.just_pressed(&keys, &mouse);

    let (transform, mut physics) = player.into_inner();
    let grounded = physics.grounded;
    let planar = Vec3::new(
        physics.horizontal_velocity.x,
        0.0,
        physics.horizontal_velocity.z,
    );
    let speed_now = planar.length();
    let move_held = binds.forward.pressed(&keys, &mouse)
        || binds.back.pressed(&keys, &mouse)
        || binds.left.pressed(&keys, &mouse)
        || binds.right.pressed(&keys, &mouse);
    let moving = speed_now > 0.5 || move_held;
    // Direction we're travelling, falling back to facing.
    let travel_dir = {
        let d = planar.normalize_or_zero();
        if d == Vec3::ZERO {
            let f = *transform.forward();
            Vec3::new(f.x, 0.0, f.z).normalize_or_zero()
        } else {
            d
        }
    };

    match slide.stance {
        Stance::Standing => {
            if grounded && crouch_pressed {
                if moving {
                    let launch = (cfg.slide_speed
                        + if sprinting.0 { cfg.sprint_bonus } else { 0.0 })
                        * weapon_mult;
                    // A slide never slows you down — momentum carries.
                    slide.velocity = travel_dir * launch.max(speed_now);
                    slide.timer = 0.0;
                    slide.stance = Stance::Sliding;
                    if let Some(e) = slide.sound.take() {
                        commands.entity(e).try_despawn();
                    }
                    slide.sound = Some(
                        commands
                            .spawn((
                                AudioPlayer::new(sounds.slide.clone()),
                                PlaybackSettings::DESPAWN,
                            ))
                            .id(),
                    );
                    snd.note(killcam::SND_SLIDE);
                } else {
                    slide.stance = Stance::Crouching;
                }
            } else if grounded && prone_pressed {
                if moving {
                    // Dolphin dive: leap forward, land prone (sound on impact).
                    physics.horizontal_velocity =
                        travel_dir * (cfg.dive_speed * weapon_mult).max(speed_now);
                    physics.vertical_velocity = cfg.dive_jump;
                    physics.grounded = false;
                    slide.stance = Stance::Diving;
                    slide.prone_from = Stance::Standing;
                } else {
                    slide.stance = Stance::Prone;
                    slide.prone_from = Stance::Standing;
                }
            }
        }
        Stance::Crouching => {
            if sprint_pressed {
                // Stand up and take off — sprint is already on from `toggle_sprint`.
                slide.stance = Stance::Standing;
            } else if prone_pressed {
                // No diving out of a crouch — just drop prone.
                slide.stance = Stance::Prone;
                slide.prone_from = Stance::Crouching;
            } else if crouch_pressed || jump_pressed {
                slide.stance = Stance::Standing;
                slide.ate_jump = jump_pressed;
            }
        }
        Stance::Sliding => {
            slide.timer += dt;
            let spd = (slide.velocity.length() - cfg.friction * dt).max(0.0);
            slide.velocity = slide.velocity.normalize_or_zero() * spd;

            let cancelled = jump_pressed;
            if cancelled || spd < cfg.min_speed || slide.timer >= cfg.max_time || !grounded {
                slide.stance = Stance::Standing;
                slide.velocity = Vec3::ZERO;
                if cancelled {
                    slide.ate_jump = true;
                    if let Some(e) = slide.sound.take() {
                        commands.entity(e).try_despawn(); // cut the slide sound short
                    }
                } else {
                    slide.sound = None; // ran its course — let the sound finish
                }
            }
        }
        Stance::Diving => {
            if grounded {
                // Belly hit the ground: settle prone and thump.
                slide.stance = Stance::Prone;
                slide.prone_from = Stance::Standing;
                commands.spawn((
                    AudioPlayer::new(sounds.dive.clone()),
                    PlaybackSettings::DESPAWN,
                ));
                snd.note(killcam::SND_DIVE);
            }
        }
        Stance::Prone => {
            if sprint_pressed {
                // Stand straight up and take off.
                slide.stance = Stance::Standing;
            } else if prone_pressed || jump_pressed || crouch_pressed {
                slide.stance = slide.prone_from;
                slide.ate_jump = jump_pressed;
            }
        }
    }

    // Ease the camera toward the target height for the stance and write it onto
    // the head (which carries the cameras + gun), so it's never a jerk.
    let target_drop = match slide.stance {
        Stance::Standing => 0.0,
        Stance::Crouching | Stance::Sliding => -cfg.crouch_drop,
        Stance::Diving | Stance::Prone => -cfg.prone_drop,
    };
    let rate = if target_drop < slide.drop - 1.0e-4 {
        if slide.stance == Stance::Diving {
            cfg.dive_tuck_speed
        } else {
            cfg.duck_speed
        }
    } else {
        cfg.stand_speed
    };
    slide.drop += (target_drop - slide.drop) * (1.0 - (-rate * dt).exp());
    if slide.drop.abs() < 1.0e-4 {
        slide.drop = 0.0;
    }
    head.translation.y = slide.drop;

    // The (invisible, shadow-only) body capsule follows the same drop, so its
    // shadow reads as crouched / prone instead of always standing tall.
    **capsule = body_capsule_pose(slide.drop, &cfg);
}
