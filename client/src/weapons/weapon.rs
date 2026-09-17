//! Weapon state machine: ammo, the sniper's fire/reload/rechamber queue, weapon
//! swap, and the throwing-knife throw/melee mechanic.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::keybinds::KeyBindings;
use crate::killcam;
use crate::player::WorldModelCamera;
use crate::settings::Settings;
use crate::util::{rand01, rand_roll};
use crate::{practice, GameSounds, MuzzleFlashState, SmokeEmission};

use super::ads::{noscope_spread_angle, Ads, NoScopeSpread};
use super::knife_view_model::{
    KnifeAnimation, KnifeAnimationPlayer, KnifeViewModel, KNIFE_SEGMENTS, KNIFE_SEG_ADJUST_GRIP,
    KNIFE_SEG_HIDE, KNIFE_SEG_SHOW, KNIFE_SLICE_SEGMENTS,
};
use super::recoil::{Shake, ShakeSettings};
use super::view_model::{
    play_segment, AnimationSegment, AnimationSettings, SniperAnimationPlayer, ViewModel,
    ViewModelAnimation, SEGMENTS, SEG_HIDE, SEG_RECHAMBER, SEG_RELOAD, SEG_SHOOT, SEG_SHOW,
};

/// Magazine capacity and the total number of magazines the player carries
/// (current mag + reserve = `MAG_SIZE * TOTAL_MAGS`).
pub(crate) const MAG_SIZE: u32 = 5;
#[allow(dead_code)] // TEMP: unused while reserve is hard-coded for reload testing
pub(crate) const TOTAL_MAGS: u32 = 6;

/// A weapon action (fire / swap-to-secondary) cut a busy queue short — stash
/// whatever bolt-cycle work is still outstanding as `weapon.interrupted` so it
/// forces its way back in (from the top) once the sniper is drawn again.
///
/// The already-fired `Shoot` segment itself never needs replaying, only
/// whatever comes after it, so that's dropped off the front first — otherwise
/// interrupting *while `Shoot` is still playing* (e.g. holding the throwing
/// knife the instant a shot goes off) would see `Shoot` still sitting at
/// `remaining[0]`, fail the "is it a Rechamber/Reload" check below, and the
/// queue — Rechamber and all — would just be dropped, leaving the chamber
/// permanently uncycled. Once `Shoot` is stripped, a queue fronted by
/// `Rechamber` or `Reload` (which may itself chain into a trailing
/// `Rechamber`) is preserved; anything else (nothing left, or a mid-play
/// Hide/Show) is simply abandoned, same as before.
pub(crate) fn stash_interrupted(weapon: &mut Weapon, mut busy: WeaponBusy) {
    while busy
        .remaining
        .first()
        .is_some_and(|seg| seg.name == SEGMENTS[SEG_SHOOT].name)
    {
        busy.remaining.remove(0);
    }
    if let Some(seg) = busy.remaining.first().copied() {
        if seg.name == SEGMENTS[SEG_RECHAMBER].name || seg.name == SEGMENTS[SEG_RELOAD].name {
            busy.seg_end = seg.end_secs();
            weapon.interrupted = Some(busy);
        }
    }
}

/// Set by `weapon_system` on the frame the trigger is pulled; consumed by
/// `net::write_input`, which turns it into the tick's fire request. Carries the
/// world-space shot direction — already thrown off by no-scope inaccuracy (see
/// [`NoScopeSpread`]) — so the local hit resolution and the server agree.
#[derive(Resource, Default)]
pub(crate) struct PendingShot(pub Option<Vec3>);

/// Which weapon slot is up. The knife has no model yet, so `Secondary` just
/// means "sniper hidden, hands empty" (plus a small movement-speed bump).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum WeaponSlot {
    #[default]
    Primary,
    Secondary,
}

/// Ammo counts and the animation the weapon is mid-way through, if any. While
/// `busy` is `Some` neither firing nor reloading is accepted.
#[derive(Resource)]
pub(crate) struct Weapon {
    /// Rounds in the current magazine.
    pub(crate) mag: u32,
    /// Rounds not in the magazine.
    pub(crate) reserve: u32,
    pub(crate) busy: Option<WeaponBusy>,
    /// The slot currently equipped (switched the instant the swap key is
    /// pressed; the Hide / Show animation then plays out via `busy`).
    pub(crate) slot: WeaponSlot,
    /// A reload / rechamber cut short by a weapon switch. Replayed from the top —
    /// animation and sound — once the sniper is next drawn.
    interrupted: Option<WeaponBusy>,
}

pub(crate) struct WeaponBusy {
    /// Segments still to play; `remaining[0]` is the one playing now.
    remaining: Vec<AnimationSegment>,
    /// Clip time (seconds) the current segment ends at.
    seg_end: f32,
    /// What to apply once the whole queue has played out.
    pub(crate) on_finish: WeaponFinish,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WeaponFinish {
    Nothing,
    Reload,
    /// Hide finished — the sniper is now stowed; drop its model.
    Holster,
    /// Show finished — the sniper is back out; resume any interrupted action.
    Draw,
}

/// Tags the one-shot rechamber / reload audio entities so a weapon switch can
/// cut them off (they otherwise self-despawn when the clip ends).
#[derive(Component)]
pub(crate) struct WeaponActionSound;

/// The knife's own animation state — mirrors `Weapon::busy`/`WeaponBusy`, but
/// kept entirely separate rather than reused: the sniper and the knife are
/// never both mid-animation at once (only one is ever drawn), but they
/// animate through two different `AnimationPlayer`s, and the knife's own
/// state machine is much simpler (no reload/rechamber interrupt-and-resume
/// queue to model). `None` means idle — fully hidden while
/// `WeaponSlot::Primary`, fully drawn and waiting on a left click while
/// `WeaponSlot::Secondary`.
#[derive(Resource, Default)]
pub(crate) struct KnifeAnimState {
    busy: Option<KnifeBusy>,
    /// Bumped on every slice attack, seeds which of `KNIFE_SLICE_SEGMENTS`
    /// plays — see `weapon_system`.
    swings: u32,
}

pub(crate) struct KnifeBusy {
    /// Segments still to play; `remaining[0]` is the one playing now — e.g.
    /// `[Show, Adjust Grip]` when the knife is being drawn.
    remaining: Vec<AnimationSegment>,
    /// Clip time (seconds) the current segment ends at.
    seg_end: f32,
    on_finish: KnifeFinish,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum KnifeFinish {
    /// A slice (or the Show → Adjust Grip draw sequence) finished — nothing
    /// special, just back to idle-out.
    Nothing,
    /// Hide finished — the knife is now stowed. `weapon_system` flips
    /// `Weapon::slot` back to `Primary` and plays the sniper's own Show
    /// right here, deferred from whenever the swap key was actually
    /// pressed — mirrors `WeaponFinish::Holster` kicking the knife's Show
    /// off the moment the sniper finishes *its* Hide.
    Hidden,
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            mag: MAG_SIZE,
            reserve: 1000, // TEMP: high reserve for reload-sound testing
            busy: None,
            slot: WeaponSlot::Primary,
            interrupted: None,
        }
    }
}

impl Weapon {
    /// Top ammo back up to a fresh loadout without touching which weapon is
    /// drawn or any animation in progress — a `FreeForAll` respawn hands the
    /// player a new life, but `stop_killcam` deliberately restores the drawn
    /// weapon/slot from the moment of death, so only the ammo counts (not the
    /// rest of `Weapon`) should reset here; otherwise the empty mag from the
    /// old life carried straight into the new one.
    pub(crate) fn refill_ammo(&mut self) {
        let fresh = Self::default();
        self.mag = fresh.mag;
        self.reserve = fresh.reserve;
    }
}

/// Hold-to-snap-out state for the throwing-knife key. Independent of
/// [`WeaponSlot`] — it's an instant overlay on whatever's currently equipped
/// (snap the sniper away with no Hide animation, so a shot's rechamber can be
/// cut off for a "silent" quickscope), not a real weapon switch.
#[derive(Resource, Default)]
pub(crate) struct ThrowingKnife {
    pub(crate) active: bool,
}

/// Re-entering the world always starts on the sniper, model shown, animation
/// parked at rest — so quitting mid-swap can't leave the next game weaponless.
pub(crate) fn reset_weapon(
    mut weapon: ResMut<Weapon>,
    mut knife: ResMut<ThrowingKnife>,
    mut view_model: Query<(&ViewModelAnimation, &mut Visibility), With<ViewModel>>,
    mut players: Query<&mut AnimationPlayer, With<SniperAnimationPlayer>>,
) {
    *weapon = Weapon::default();
    *knife = ThrowingKnife::default();
    if let Ok((vm, mut vis)) = view_model.single_mut() {
        *vis = Visibility::Inherited;
        if let Some(mut player) = players.iter_mut().next() {
            if let Some(active) = player.animation_mut(vm.index) {
                active.seek_to(0.0);
                active.pause();
            }
        }
    }
}

/// Fire, reload and weapon-swap (bindings). Firing spends a round and plays
/// Shoot → Rechamber to cycle the bolt. The shot that empties the mag leaves
/// the spent case sitting in the chamber (nothing left in the mag to cycle
/// into it) and, since the Reload clip only swaps the magazine and never
/// touches the bolt, that reload always ends with a Rechamber to load the
/// first round of the fresh mag — either right away (auto-reload) or once a
/// manual reload is pressed. Reloading with a round already chambered (mag
/// still has rounds) skips that trailing Rechamber; refilling the mag/reserve
/// counters happens once the whole queue finishes. Swapping to the secondary
/// plays Hide in full then drops the sniper model;
/// swapping back plays Show in full and restarts any reload / rechamber the swap
/// cut short. Nothing new is accepted while an animation is mid-play (except the
/// swap key), so shots are impossible until a reload finishes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn weapon_system(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    window: Single<&Window, With<PrimaryWindow>>,
    (view_model, mut view_model_vis, knife_anim, mut knife_vis): (
        Single<&ViewModelAnimation>,
        Single<&mut Visibility, (With<ViewModel>, Without<KnifeViewModel>)>,
        Single<&KnifeAnimation>,
        Single<&mut Visibility, (With<KnifeViewModel>, Without<ViewModel>)>,
    ),
    (cam, action_sounds, mut players, mut knife_players): (
        Query<&GlobalTransform, With<WorldModelCamera>>,
        Query<Entity, With<WeaponActionSound>>,
        Query<&mut AnimationPlayer, (With<SniperAnimationPlayer>, Without<KnifeAnimationPlayer>)>,
        Query<&mut AnimationPlayer, (With<KnifeAnimationPlayer>, Without<SniperAnimationPlayer>)>,
    ),
    mut weapon: ResMut<Weapon>,
    (mut knife, mut knife_state): (ResMut<ThrowingKnife>, ResMut<KnifeAnimState>),
    mut pending_shot: ResMut<PendingShot>,
    mut shake: ResMut<Shake>,
    mut muzzle: ResMut<MuzzleFlashState>,
    mut smoke: ResMut<SmokeEmission>,
    mut shots: EventWriter<practice::LocalShot>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    (shake_cfg, sounds, anim, ads, spread_cfg, settings): (
        Res<ShakeSettings>,
        Res<GameSounds>,
        Res<AnimationSettings>,
        Res<Ads>,
        Res<NoScopeSpread>,
        Res<Settings>,
    ),
    mut commands: Commands,
) {
    let node = view_model.index;
    let Some(mut player) = players.iter_mut().next() else {
        return;
    };
    let knife_node = knife_anim.index;
    let Some(mut knife_player) = knife_players.iter_mut().next() else {
        return;
    };
    let locked = window.cursor_options.grab_mode != CursorGrabMode::None;

    // Throwing knife — a separate, instant hide/show layered over whatever's
    // equipped (doesn't touch `weapon.slot`). Pressing it snaps the sniper away
    // with no Hide animation, so a shot's rechamber can be cut off for a
    // "silent" quickscope; releasing it (or pressing swap-weapon while it's
    // held) draws the sniper back out with the normal Show animation and
    // resumes whatever the hold interrupted.
    if locked && binds.throwing_knife.just_pressed(&keys, &mouse) && !knife.active {
        knife.active = true;
        if weapon.slot == WeaponSlot::Primary {
            if let Some(busy) = weapon.busy.take() {
                for e in &action_sounds {
                    commands.entity(e).try_despawn();
                }
                stash_interrupted(&mut weapon, busy);
            }
            if let Some(active_anim) = player.animation_mut(node) {
                active_anim.seek_to(0.0);
                active_anim.pause();
            }
            **view_model_vis = Visibility::Hidden;
        }
        return;
    }
    if knife.active {
        let release = !binds.throwing_knife.pressed(&keys, &mouse)
            || binds.swap_weapon.just_pressed(&keys, &mouse);
        if release {
            knife.active = false;
            if weapon.slot == WeaponSlot::Primary {
                **view_model_vis = Visibility::Inherited;
                play_segment(&mut player, node, SEGMENTS[SEG_SHOW]);
                weapon.busy = Some(WeaponBusy {
                    remaining: vec![SEGMENTS[SEG_SHOW]],
                    seg_end: SEGMENTS[SEG_SHOW].end_secs(),
                    on_finish: WeaponFinish::Draw,
                });
            }
        }
        return;
    }

    // Weapon swap — accepted even mid-action, so it can cut a reload / rechamber
    // short.
    if locked && binds.swap_weapon.just_pressed(&keys, &mouse) {
        match weapon.slot {
            WeaponSlot::Primary => {
                // Stow the sniper. Cancel whatever it was doing: silence the
                // reload / rechamber audio, and remember a reload / rechamber so
                // it can be replayed from the top when the sniper is drawn again.
                if let Some(busy) = weapon.busy.take() {
                    for e in &action_sounds {
                        commands.entity(e).try_despawn();
                    }
                    stash_interrupted(&mut weapon, busy);
                }
                weapon.slot = WeaponSlot::Secondary;
                play_segment(&mut player, node, SEGMENTS[SEG_HIDE]);
                weapon.busy = Some(WeaponBusy {
                    remaining: vec![SEGMENTS[SEG_HIDE]],
                    seg_end: SEGMENTS[SEG_HIDE].end_secs(),
                    on_finish: WeaponFinish::Holster,
                });
            }
            WeaponSlot::Secondary => {
                // Stow the knife first (cutting short whatever it was doing —
                // showing, adjusting grip, or mid-slice — same "accepted
                // even mid-action" policy as the sniper above). `weapon.slot`
                // only actually flips back to `Primary`, and the sniper's
                // own Show plays, once the knife's Hide finishes — see the
                // knife-busy advance block below.
                play_segment(
                    &mut knife_player,
                    knife_node,
                    KNIFE_SEGMENTS[KNIFE_SEG_HIDE],
                );
                knife_state.busy = Some(KnifeBusy {
                    remaining: vec![KNIFE_SEGMENTS[KNIFE_SEG_HIDE]],
                    seg_end: KNIFE_SEGMENTS[KNIFE_SEG_HIDE].end_secs(),
                    on_finish: KnifeFinish::Hidden,
                });
            }
        }
        return;
    }

    // Advance an in-progress action.
    if weapon.busy.is_some() {
        let next_or_finish = {
            let busy = weapon.busy.as_mut().unwrap();
            let done = player
                .animation(node)
                .is_none_or(|a| a.is_finished() || a.seek_time() >= busy.seg_end);
            if !done {
                return;
            }
            busy.remaining.remove(0);
            match busy.remaining.first().copied() {
                Some(next) => {
                    busy.seg_end = next.end_secs();
                    Ok(next)
                }
                None => Err(busy.on_finish),
            }
        };

        match next_or_finish {
            Ok(next) => {
                play_segment(&mut player, node, next);
                if next.name == SEGMENTS[SEG_RECHAMBER].name {
                    commands.spawn((
                        AudioPlayer::new(sounds.rechamber.clone()),
                        PlaybackSettings::DESPAWN,
                        WeaponActionSound,
                    ));
                    snd.note(killcam::SND_RECHAMBER);
                    if let Some(active) = player.animation_mut(node) {
                        active.set_speed(anim.rechamber_speed);
                    }
                } else if next.name == SEGMENTS[SEG_RELOAD].name {
                    // Auto-reload rolling straight out of the Shoot segment.
                    commands.spawn((
                        AudioPlayer::new(sounds.reload.clone()),
                        PlaybackSettings::DESPAWN,
                        WeaponActionSound,
                    ));
                    snd.note(killcam::SND_RELOAD);
                }
            }
            Err(on_finish) => {
                // Whole queue played out: park at the rest pose, apply effect.
                if let Some(active) = player.animation_mut(node) {
                    active.seek_to(0.0);
                    active.pause();
                }
                weapon.busy = None;
                match on_finish {
                    WeaponFinish::Nothing => {}
                    WeaponFinish::Reload => {
                        let moved = (MAG_SIZE - weapon.mag).min(weapon.reserve);
                        weapon.mag += moved;
                        weapon.reserve -= moved;
                    }
                    WeaponFinish::Holster => {
                        // Sniper fully hidden — the knife takes over: show it,
                        // then (once that finishes) settle into an adjusted
                        // grip, then idle-out waiting for a slice input.
                        **view_model_vis = Visibility::Hidden;
                        **knife_vis = Visibility::Inherited;
                        play_segment(
                            &mut knife_player,
                            knife_node,
                            KNIFE_SEGMENTS[KNIFE_SEG_SHOW],
                        );
                        knife_state.busy = Some(KnifeBusy {
                            remaining: vec![
                                KNIFE_SEGMENTS[KNIFE_SEG_SHOW],
                                KNIFE_SEGMENTS[KNIFE_SEG_ADJUST_GRIP],
                            ],
                            seg_end: KNIFE_SEGMENTS[KNIFE_SEG_SHOW].end_secs(),
                            on_finish: KnifeFinish::Nothing,
                        });
                    }
                    WeaponFinish::Draw => {
                        // Sniper back out: restart whatever the swap interrupted,
                        // animation and sound from the top.
                        if let Some(resumed) = weapon.interrupted.take() {
                            let seg = resumed.remaining[0];
                            play_segment(&mut player, node, seg);
                            if seg.name == SEGMENTS[SEG_RECHAMBER].name {
                                commands.spawn((
                                    AudioPlayer::new(sounds.rechamber.clone()),
                                    PlaybackSettings::DESPAWN,
                                    WeaponActionSound,
                                ));
                                snd.note(killcam::SND_RECHAMBER);
                                if let Some(active) = player.animation_mut(node) {
                                    active.set_speed(anim.rechamber_speed);
                                }
                            } else if seg.name == SEGMENTS[SEG_RELOAD].name {
                                commands.spawn((
                                    AudioPlayer::new(sounds.reload.clone()),
                                    PlaybackSettings::DESPAWN,
                                    WeaponActionSound,
                                ));
                                snd.note(killcam::SND_RELOAD);
                            }
                            weapon.busy = Some(resumed);
                        }
                    }
                }
            }
        }
        return;
    }

    // Advance an in-progress knife action (Show → Adjust Grip on draw, or a
    // single Hide / slice segment) — mirrors the sniper's own advance block
    // above, just without a reload/rechamber-style interrupt-and-resume
    // queue (the knife never needs one: swapping away just cuts straight to
    // Hide, nothing to resume later).
    if knife_state.busy.is_some() {
        let next_or_finish = {
            let busy = knife_state.busy.as_mut().unwrap();
            let done = knife_player
                .animation(knife_node)
                .is_none_or(|a| a.is_finished() || a.seek_time() >= busy.seg_end);
            if !done {
                return;
            }
            busy.remaining.remove(0);
            match busy.remaining.first().copied() {
                Some(next) => {
                    busy.seg_end = next.end_secs();
                    Ok(next)
                }
                None => Err(busy.on_finish),
            }
        };
        match next_or_finish {
            Ok(next) => play_segment(&mut knife_player, knife_node, next),
            Err(on_finish) => {
                if let Some(active) = knife_player.animation_mut(knife_node) {
                    active.seek_to(0.0);
                    active.pause();
                }
                knife_state.busy = None;
                if on_finish == KnifeFinish::Hidden {
                    // Knife fully hidden — hand back off to the sniper.
                    **knife_vis = Visibility::Hidden;
                    weapon.slot = WeaponSlot::Primary;
                    **view_model_vis = Visibility::Inherited;
                    play_segment(&mut player, node, SEGMENTS[SEG_SHOW]);
                    weapon.busy = Some(WeaponBusy {
                        remaining: vec![SEGMENTS[SEG_SHOW]],
                        seg_end: SEGMENTS[SEG_SHOW].end_secs(),
                        on_finish: WeaponFinish::Draw,
                    });
                }
            }
        }
        return;
    }

    // Idle: only take input while the cursor is captured (i.e. in-game).
    if !locked {
        return;
    }
    if weapon.slot == WeaponSlot::Secondary {
        // Knife idle-out, waiting on a left click — one of the four slices,
        // picked at random each time.
        if binds.fire.just_pressed(&keys, &mouse) {
            knife_state.swings = knife_state.swings.wrapping_add(1);
            let pick = (rand01(knife_state.swings.wrapping_mul(0xA511_E9B3))
                * KNIFE_SLICE_SEGMENTS.len() as f32) as usize;
            let seg =
                KNIFE_SEGMENTS[KNIFE_SLICE_SEGMENTS[pick.min(KNIFE_SLICE_SEGMENTS.len() - 1)]];
            play_segment(&mut knife_player, knife_node, seg);
            knife_state.busy = Some(KnifeBusy {
                remaining: vec![seg],
                seg_end: seg.end_secs(),
                on_finish: KnifeFinish::Nothing,
            });
        }
        return;
    }
    // Only `WeaponSlot::Primary` (the sniper) is left.

    if binds.fire.just_pressed(&keys, &mouse) && weapon.mag > 0 {
        weapon.mag -= 1;
        shake.trauma = (shake.trauma + shake_cfg.trauma_per_shot).min(1.0);
        shake.recoil = shake_cfg.recoil_kick;
        muzzle.shots = muzzle.shots.wrapping_add(1);
        muzzle.roll = rand_roll(muzzle.shots);
        muzzle.intensity = 1.0;
        smoke.0 = Some(0.0);
        commands.spawn((
            AudioPlayer::new(sounds.shot.clone()),
            PlaybackSettings::DESPAWN,
        ));
        snd.note(killcam::SND_SHOT);

        // Aim direction, thrown off the crosshair by no-scope inaccuracy: a
        // random up/down + left/right angle, each up to `noscope_spread_angle`
        // (wide at the hip, zero at full ADS). `write_input` and
        // `resolve_local_shot` both take this exact ray so local feedback and
        // the server hit agree.
        let cam_gt = cam.single().ok();
        let dir = cam_gt
            .map(|cam| {
                let max = noscope_spread_angle(&spread_cfg, ads.t);
                let yaw = (rand01(muzzle.shots.wrapping_mul(0x9E37_79B9)) * 2.0 - 1.0) * max;
                let pitch = (rand01(muzzle.shots.wrapping_mul(0x85EB_CA6B) ^ 0xDEAD_BEEF) * 2.0
                    - 1.0)
                    * max;
                cam.rotation() * Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z
            })
            .unwrap_or(Vec3::NEG_Z);
        pending_shot.0 = Some(dir); // net::write_input turns this into a fire request

        // Hand the shot ray to `practice::resolve_local_shot`: it kicks up the
        // ground dust locally (instant, and the only path in solo Practice) and,
        // in Practice, resolves the hit + scoring against the offline bots. In a
        // real game the server also broadcasts this shot; `net::receive_shots`
        // drops the echo for our own peer so nothing double-spawns.
        if let Some(cam) = cam_gt {
            shots.write(practice::LocalShot {
                origin: cam.translation(),
                dir,
            });
        }
        play_segment(&mut player, node, SEGMENTS[SEG_SHOOT]);

        // Bolt-action cycle. After a shot that leaves rounds in the mag, work
        // the bolt (Rechamber) to eject the spent case and feed the next round.
        // The shot that empties the mag leaves the spent case sitting in the
        // chamber — the Reload clip only swaps the magazine, it never touches
        // the bolt — so with auto-reload on we go Shoot → Reload → Rechamber,
        // working the bolt only once the fresh mag is seated (that one motion
        // both ejects the old case and feeds the first round of the new mag).
        // With auto-reload off the sniper just holds on the fired pose until a
        // manual reload.
        let (remaining, on_finish) = if weapon.mag > 0 {
            (
                vec![SEGMENTS[SEG_SHOOT], SEGMENTS[SEG_RECHAMBER]],
                WeaponFinish::Nothing,
            )
        } else if settings.auto_reload && weapon.reserve > 0 {
            (
                vec![
                    SEGMENTS[SEG_SHOOT],
                    SEGMENTS[SEG_RELOAD],
                    SEGMENTS[SEG_RECHAMBER],
                ],
                WeaponFinish::Reload,
            )
        } else {
            (vec![SEGMENTS[SEG_SHOOT]], WeaponFinish::Nothing)
        };
        weapon.busy = Some(WeaponBusy {
            remaining,
            seg_end: SEGMENTS[SEG_SHOOT].end_secs(),
            on_finish,
        });
    } else if binds.fire.just_pressed(&keys, &mouse) {
        // Trigger pulled on an empty mag — click, no bang.
        commands.spawn((
            AudioPlayer::new(sounds.out_of_ammo.clone()),
            PlaybackSettings::DESPAWN,
        ));
    } else if binds.reload.just_pressed(&keys, &mouse) && weapon.reserve > 0 {
        // TEMP: reload allowed even with a full mag, for reload-sound testing
        commands.spawn((
            AudioPlayer::new(sounds.reload.clone()),
            PlaybackSettings::DESPAWN,
            WeaponActionSound,
        ));
        snd.note(killcam::SND_RELOAD);
        play_segment(&mut player, node, SEGMENTS[SEG_RELOAD]);
        // `mag == 0` only when the last round was fired and never rechambered
        // (there's no separate "round chambered" flag — an empty mag is the
        // one moment the chamber is guaranteed empty too), so the bolt still
        // needs working after the fresh mag goes in. Reloading with a round
        // already chambered (mag > 0, the TEMP full-mag case above) is a
        // tactical swap — the chamber's already loaded, so no bolt work.
        let mut remaining = vec![SEGMENTS[SEG_RELOAD]];
        if weapon.mag == 0 {
            remaining.push(SEGMENTS[SEG_RECHAMBER]);
        }
        weapon.busy = Some(WeaponBusy {
            remaining,
            seg_end: SEGMENTS[SEG_RELOAD].end_secs(),
            on_finish: WeaponFinish::Reload,
        });
    }
}
