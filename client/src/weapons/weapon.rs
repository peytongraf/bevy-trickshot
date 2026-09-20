//! Weapon state machine: ammo, the sniper's fire/reload/rechamber queue, weapon
//! swap, and the throwing-knife throw/melee mechanic.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::keybinds::KeyBindings;
use crate::killcam;
use crate::player::WorldModelCamera;
use crate::settings::Settings;
use crate::util::{rand01, rand_roll};
use crate::{FireTracer, GameSounds, GroundImpact, MuzzleFlashState, SmokeEmission};
use shared::ballistics::ground_impact;
use shared::weapon::WeaponId;

use super::ads::{noscope_spread_angle, Ads, NoScopeSpread};
use super::knife_view_model::{
    KnifeAnimation, KnifeAnimationPlayer, KnifeViewModel, KNIFE_SEGMENTS, KNIFE_SEG_ADJUST_GRIP,
    KNIFE_SEG_HIDE, KNIFE_SEG_SHOW, KNIFE_SLICE_SEGMENTS,
};
use super::recoil::{Shake, ShakeSettings};
use super::throw_arms::{
    park_throw_arms, play_throw, ThrowArmsAnimation, ThrowArmsAnimationPlayer, ThrowArmsSettings,
};
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

/// Set by `weapon_system` on the frame a knife stab starts; consumed by
/// `net::write_input`, which turns it into the tick's melee request. Carries
/// the camera's `(eye position, forward direction)` — the server resolves the
/// stab with `shared::melee::resolve_melee` from exactly that ray.
#[derive(Resource, Default)]
pub(crate) struct PendingMelee(pub Option<(Vec3, Vec3)>);

/// The local player fired: the ray to draw the shooter's own tracer / ground
/// dust along this frame (see [`resolve_local_shot`]). The server still decides
/// what the shot actually hit.
#[derive(Event)]
pub(crate) struct LocalShot {
    pub(crate) origin: Vec3,
    pub(crate) dir: Vec3,
}

/// Every [`LocalShot`] kicks up ground dust where it lands and spawns the
/// shooter's own tracer instantly, rather than waiting on the server (other
/// players' tracers come off the server's authoritative `ShotResolved`,
/// `net::receive_shots`). The tracer ends at the ground point below, else at a
/// max-range whiff — the server owns bot / player hits, so there's no local
/// impact point to use.
pub(crate) fn resolve_local_shot(
    mut shots: EventReader<LocalShot>,
    mut ground_hit: ResMut<killcam::ReplayGroundImpact>,
    mut tracer_rec: ResMut<killcam::ReplayTracer>,
    mut impacts: EventWriter<GroundImpact>,
    mut tracers: EventWriter<FireTracer>,
) {
    for shot in shots.read() {
        let ground_pt = ground_impact(shot.origin, shot.dir);
        if let Some(p) = ground_pt {
            impacts.write(GroundImpact(p));
            // Stamp it onto this tick's `PlayerInput` too, so the kill cam can
            // replay the burst for the other players watching.
            ground_hit.0 = Some(p);
        }

        let end = ground_pt.unwrap_or_else(|| {
            shot.origin + shot.dir.normalize_or_zero() * WeaponId::Sniper.spec().max_range
        });
        tracers.write(FireTracer { start: shot.origin, end });
        // Stamp it onto this tick's `PlayerInput` too, so the kill cam re-draws
        // the tracer along its true path instead of leaving the live one
        // hanging in the world.
        tracer_rec.0 = Some((shot.origin, end));
    }
}

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
#[derive(Resource)]
pub(crate) struct KnifeAnimState {
    busy: Option<KnifeBusy>,
    /// Bumped on every slice attack, seeds which of `KNIFE_SLICE_SEGMENTS`
    /// plays — see `weapon_system`.
    swings: u32,
    /// Bumped every time [`KNIFE_ADJUST_MIN_SECS`..`KNIFE_ADJUST_MAX_SECS`] is
    /// re-rolled — a separate counter from `swings` so the idle-fidget timing
    /// and the slice picks don't share a seed sequence.
    adjust_rolls: u32,
    /// Seconds of no attack left before the next idle "Adjust Grip" fidget
    /// plays while the knife is out and idle — see `weapon_system`'s idle
    /// branch. Re-rolled to a fresh span whenever it fires, the knife is
    /// freshly drawn, or the player attacks, so it never fires mid-swing and
    /// always waits out a full fresh idle stretch afterwards.
    next_adjust_in: f32,
}

impl Default for KnifeAnimState {
    fn default() -> Self {
        Self {
            busy: None,
            swings: 0,
            adjust_rolls: 0,
            next_adjust_in: roll_knife_adjust_delay(0),
        }
    }
}

/// A random span in [`KNIFE_ADJUST_MIN_SECS`, `KNIFE_ADJUST_MAX_SECS`) until
/// the next idle "Adjust Grip" fidget — see [`KnifeAnimState::next_adjust_in`].
fn roll_knife_adjust_delay(seed: u32) -> f32 {
    KNIFE_ADJUST_MIN_SECS
        + rand01(seed.wrapping_mul(0xB529_7A4D)) * (KNIFE_ADJUST_MAX_SECS - KNIFE_ADJUST_MIN_SECS)
}

const KNIFE_ADJUST_MIN_SECS: f32 = 3.0;
const KNIFE_ADJUST_MAX_SECS: f32 = 10.0;

pub(crate) struct KnifeBusy {
    /// Segments still to play; `remaining[0]` is the one playing now.
    remaining: Vec<AnimationSegment>,
    /// Clip time (seconds) the current segment ends at.
    seg_end: f32,
    on_finish: KnifeFinish,
    /// Whether an attack can cut this short instead of waiting for it to
    /// finish naturally — set only for the randomly-triggered idle "Adjust
    /// Grip" fidget. The draw (Show) and hide sequences, and a slice already
    /// in progress, are never interrupted.
    interruptible: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum KnifeFinish {
    /// A slice, the draw (Show), or the idle "Adjust Grip" fidget finished —
    /// nothing special, just back to idle-out.
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

/// Where the throwing-knife key's sequence is up to. Independent of
/// [`WeaponSlot`] — it's an overlay on whatever's currently equipped, not a
/// real weapon switch. Pressing it plays the equipped weapon's own Hide
/// (faster than normal, `ThrowArmsSettings::weapon_hide_speed`), *then* the
/// throwing arms (`throw_arms.rs`) slide up; releasing plays their throw clip
/// once, they slide back down, and only once they're fully away does the
/// weapon that was out get its normal Show again. Swapping weapons while the
/// arms are held out cancels the throw: no clip, just the arms away and the
/// old weapon back.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum ThrowPhase {
    #[default]
    Idle,
    /// The equipped weapon's Hide is playing.
    Stowing,
    /// Arms sliding in / held out, waiting on the key's release.
    Held,
    /// The throw clip is playing.
    Throwing,
    /// Arms sliding back down; the weapon returns once they're out of view.
    Returning,
}

#[derive(Resource, Default)]
pub(crate) struct ThrowingKnife {
    /// The throwing knife is "up" for the crosshair / kill-cam recorder: from
    /// the key press until the arms start sliding away.
    pub(crate) active: bool,
    phase: ThrowPhase,
    /// Seconds spent in [`ThrowPhase::Stowing`], so a Hide clip that never
    /// reports done (e.g. reset by a kill cam) can't wedge the sequence.
    stow_elapsed: f32,
    /// How far the arms have slid into view: `0` hidden below the screen, `1`
    /// in place. Advanced by `throw_arms::slide_throw_arms`.
    pub(crate) slide: f32,
}

impl ThrowingKnife {
    /// Whether the arms should be sliding into (or holding) view.
    pub(crate) fn arms_out(&self) -> bool {
        matches!(self.phase, ThrowPhase::Held | ThrowPhase::Throwing)
    }
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

/// Start a random one of the four slice segments, replacing whatever the
/// knife's animation state currently holds. Used both from idle (a plain
/// left click) and to cut a mid-play idle "Adjust Grip" fidget short the
/// instant an attack comes in, instead of waiting for it to finish.
fn start_knife_slice(
    knife_state: &mut KnifeAnimState,
    knife_player: &mut AnimationPlayer,
    knife_node: AnimationNodeIndex,
) {
    knife_state.swings = knife_state.swings.wrapping_add(1);
    let pick = (rand01(knife_state.swings.wrapping_mul(0xA511_E9B3))
        * KNIFE_SLICE_SEGMENTS.len() as f32) as usize;
    let seg = KNIFE_SEGMENTS[KNIFE_SLICE_SEGMENTS[pick.min(KNIFE_SLICE_SEGMENTS.len() - 1)]];
    play_segment(knife_player, knife_node, seg);
    knife_state.busy = Some(KnifeBusy {
        remaining: vec![seg],
        seg_end: seg.end_secs(),
        on_finish: KnifeFinish::Nothing,
        interruptible: false,
    });
    // An attack always pushes the idle fidget back out to a fresh span, so it
    // never fires right on the heels of a swing.
    knife_state.adjust_rolls = knife_state.adjust_rolls.wrapping_add(1);
    knife_state.next_adjust_in = roll_knife_adjust_delay(knife_state.adjust_rolls);
}

/// A knife attack was just started: request the stab from wherever the
/// camera is looking. The kill (or whiff) is decided by the server, not here;
/// this only files the request, alongside the slice animation the caller
/// starts.
fn request_stab(cam: &Query<&GlobalTransform, With<WorldModelCamera>>, pending: &mut PendingMelee) {
    let Ok(cam) = cam.single() else {
        return;
    };
    pending.0 = Some((cam.translation(), cam.forward().as_vec3()));
}

/// Draw whichever weapon was out before the throwing arms came up — the
/// sniper (its normal Show, resuming any interrupted reload once that
/// finishes) or the knife (its own Show, then idle-out).
#[allow(clippy::too_many_arguments)]
fn redraw_active_weapon(
    weapon: &mut Weapon,
    knife_state: &mut KnifeAnimState,
    player: &mut AnimationPlayer,
    node: AnimationNodeIndex,
    knife_player: &mut AnimationPlayer,
    knife_node: AnimationNodeIndex,
    view_model_vis: &mut Visibility,
    knife_vis: &mut Visibility,
) {
    match weapon.slot {
        WeaponSlot::Primary => {
            *view_model_vis = Visibility::Inherited;
            play_segment(player, node, SEGMENTS[SEG_SHOW]);
            weapon.busy = Some(WeaponBusy {
                remaining: vec![SEGMENTS[SEG_SHOW]],
                seg_end: SEGMENTS[SEG_SHOW].end_secs(),
                on_finish: WeaponFinish::Draw,
            });
        }
        WeaponSlot::Secondary => {
            *knife_vis = Visibility::Inherited;
            play_segment(knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_SHOW]);
            knife_state.busy = Some(KnifeBusy {
                remaining: vec![KNIFE_SEGMENTS[KNIFE_SEG_SHOW]],
                seg_end: KNIFE_SEGMENTS[KNIFE_SEG_SHOW].end_secs(),
                on_finish: KnifeFinish::Nothing,
                interruptible: false,
            });
            knife_state.adjust_rolls = knife_state.adjust_rolls.wrapping_add(1);
            knife_state.next_adjust_in = roll_knife_adjust_delay(knife_state.adjust_rolls);
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
    (view_model, mut view_model_vis, knife_anim, mut knife_vis, arms_anim): (
        Single<&ViewModelAnimation>,
        Single<&mut Visibility, (With<ViewModel>, Without<KnifeViewModel>)>,
        Single<&KnifeAnimation>,
        Single<&mut Visibility, (With<KnifeViewModel>, Without<ViewModel>)>,
        Single<&ThrowArmsAnimation>,
    ),
    (cam, action_sounds, mut players, mut knife_players, mut arms_players): (
        Query<&GlobalTransform, With<WorldModelCamera>>,
        Query<Entity, With<WeaponActionSound>>,
        Query<
            &mut AnimationPlayer,
            (
                With<SniperAnimationPlayer>,
                Without<KnifeAnimationPlayer>,
                Without<ThrowArmsAnimationPlayer>,
            ),
        >,
        Query<
            &mut AnimationPlayer,
            (
                With<KnifeAnimationPlayer>,
                Without<SniperAnimationPlayer>,
                Without<ThrowArmsAnimationPlayer>,
            ),
        >,
        Query<
            &mut AnimationPlayer,
            (
                With<ThrowArmsAnimationPlayer>,
                Without<SniperAnimationPlayer>,
                Without<KnifeAnimationPlayer>,
            ),
        >,
    ),
    mut weapon: ResMut<Weapon>,
    (mut knife, mut knife_state, mut pending_melee): (
        ResMut<ThrowingKnife>,
        ResMut<KnifeAnimState>,
        ResMut<PendingMelee>,
    ),
    mut pending_shot: ResMut<PendingShot>,
    mut shake: ResMut<Shake>,
    mut muzzle: ResMut<MuzzleFlashState>,
    mut smoke: ResMut<SmokeEmission>,
    mut shots: EventWriter<LocalShot>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    (shake_cfg, sounds, anim, ads, spread_cfg, settings, time, arms_settings): (
        Res<ShakeSettings>,
        Res<GameSounds>,
        Res<AnimationSettings>,
        Res<Ads>,
        Res<NoScopeSpread>,
        Res<Settings>,
        Res<Time>,
        Res<ThrowArmsSettings>,
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
    // `None` if the arms scene hasn't finished loading — the throwing arms
    // then just never show anything, instead of taking the whole weapon
    // system down with them.
    let mut arms_player = arms_players.iter_mut().next();
    let arms_node = arms_anim.index;

    // Throwing knife — see [`ThrowPhase`]. Press: settle any half-finished
    // swap, then play the equipped weapon's Hide (sped up), which also cuts a
    // sniper reload / rechamber short for a "silent" quickscope. Once it's
    // done the arms slide up (`throw_arms::slide_throw_arms`). Release (with
    // the arms fully in): the throw clip plays once. When it ends the arms
    // slide away and the weapon that was out is drawn again with its normal
    // Show, resuming whatever the press interrupted. Swap-weapon while the
    // arms are out cancels instead: no clip, the arms slide away and the
    // same weapon comes back.
    if locked && binds.throwing_knife.just_pressed(&keys, &mouse) && knife.phase == ThrowPhase::Idle
    {
        knife.active = true;
        knife.stow_elapsed = 0.0;
        if let Some(arms) = arms_player.as_mut() {
            park_throw_arms(arms, arms_node);
        }
        // Settle a half-finished swap so "the weapon that was out" is
        // unambiguous, and so there's nothing visible to play a Hide on: a
        // sniper Hide in progress (slot already `Secondary`) just finishes
        // instantly, and a knife Hide in progress hands the slot straight
        // back to the sniper with both models already away.
        let mut nothing_shown = false;
        if weapon.slot == WeaponSlot::Secondary
            && weapon
                .busy
                .as_ref()
                .is_some_and(|b| b.on_finish == WeaponFinish::Holster)
        {
            weapon.busy = None;
            for e in &action_sounds {
                commands.entity(e).try_despawn();
            }
            if let Some(active_anim) = player.animation_mut(node) {
                active_anim.seek_to(0.0);
                active_anim.pause();
            }
            **view_model_vis = Visibility::Hidden;
            nothing_shown = true;
        }
        if let Some(busy) = knife_state.busy.take() {
            if busy.on_finish == KnifeFinish::Hidden {
                weapon.slot = WeaponSlot::Primary;
                **knife_vis = Visibility::Hidden;
                nothing_shown = true;
            }
            if let Some(active_anim) = knife_player.animation_mut(knife_node) {
                active_anim.seek_to(0.0);
                active_anim.pause();
            }
        }
        if nothing_shown {
            knife.phase = ThrowPhase::Held;
            return;
        }
        knife.phase = ThrowPhase::Stowing;
        match weapon.slot {
            WeaponSlot::Primary => {
                if let Some(busy) = weapon.busy.take() {
                    for e in &action_sounds {
                        commands.entity(e).try_despawn();
                    }
                    stash_interrupted(&mut weapon, busy);
                }
                play_segment(&mut player, node, SEGMENTS[SEG_HIDE]);
                if let Some(active_anim) = player.animation_mut(node) {
                    active_anim.set_speed(arms_settings.weapon_hide_speed);
                }
            }
            WeaponSlot::Secondary => {
                play_segment(&mut knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_HIDE]);
                if let Some(active_anim) = knife_player.animation_mut(knife_node) {
                    active_anim.set_speed(arms_settings.weapon_hide_speed);
                }
            }
        }
        return;
    }
    match knife.phase {
        ThrowPhase::Idle => {}
        ThrowPhase::Stowing => {
            // Wait out the sped-up Hide (a swap press is ignored — it only
            // lasts a moment), then drop the model and bring the arms in.
            knife.stow_elapsed += time.delta_secs();
            let (seg, done) = match weapon.slot {
                WeaponSlot::Primary => (
                    SEGMENTS[SEG_HIDE],
                    player
                        .animation(node)
                        .is_none_or(|a| a.is_finished() || a.seek_time() >= SEGMENTS[SEG_HIDE].end_secs()),
                ),
                WeaponSlot::Secondary => (
                    KNIFE_SEGMENTS[KNIFE_SEG_HIDE],
                    knife_player.animation(knife_node).is_none_or(|a| {
                        a.is_finished() || a.seek_time() >= KNIFE_SEGMENTS[KNIFE_SEG_HIDE].end_secs()
                    }),
                ),
            };
            let expected = (seg.end_secs() - seg.start_secs()) / arms_settings.weapon_hide_speed.max(0.01);
            if done || knife.stow_elapsed > expected + 0.25 {
                match weapon.slot {
                    WeaponSlot::Primary => {
                        if let Some(active_anim) = player.animation_mut(node) {
                            active_anim.seek_to(0.0);
                            active_anim.pause();
                        }
                        **view_model_vis = Visibility::Hidden;
                    }
                    WeaponSlot::Secondary => {
                        if let Some(active_anim) = knife_player.animation_mut(knife_node) {
                            active_anim.seek_to(0.0);
                            active_anim.pause();
                        }
                        **knife_vis = Visibility::Hidden;
                    }
                }
                knife.phase = ThrowPhase::Held;
            }
            return;
        }
        ThrowPhase::Held => {
            if binds.swap_weapon.just_pressed(&keys, &mouse) {
                // Cancelled: no throw animation.
                knife.active = false;
                knife.phase = ThrowPhase::Returning;
            } else if !binds.throwing_knife.pressed(&keys, &mouse) && knife.slide >= 1.0 {
                // Released with the arms fully in place: throw.
                knife.phase = ThrowPhase::Throwing;
                if let Some(arms) = arms_player.as_mut() {
                    play_throw(arms, arms_node);
                }
            }
            return;
        }
        ThrowPhase::Throwing => {
            // Throw clip playing out — nothing else is accepted until it
            // finishes (a swap press is ignored; the knife is already gone).
            let done = arms_player
                .as_ref()
                .and_then(|p| p.animation(arms_node))
                .is_none_or(|a| a.is_finished());
            if done {
                // The arms stay on the clip's last frame as they slide away;
                // the next press re-parks them.
                knife.active = false;
                knife.phase = ThrowPhase::Returning;
            }
            return;
        }
        ThrowPhase::Returning => {
            // Only once the arms are fully out of view does the weapon come
            // back.
            if knife.slide <= 0.0 {
                knife.phase = ThrowPhase::Idle;
                redraw_active_weapon(
                    &mut weapon,
                    &mut knife_state,
                    &mut player,
                    node,
                    &mut knife_player,
                    knife_node,
                    &mut view_model_vis,
                    &mut knife_vis,
                );
            }
            return;
        }
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
                    interruptible: false,
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
                        // then straight to idle-out waiting for a slice input.
                        // The "Adjust Grip" fidget no longer auto-plays here —
                        // it's a random idle-only animation now (see the
                        // `WeaponSlot::Secondary` idle branch below) so a
                        // fresh draw never blocks an immediate attack.
                        **view_model_vis = Visibility::Hidden;
                        **knife_vis = Visibility::Inherited;
                        play_segment(
                            &mut knife_player,
                            knife_node,
                            KNIFE_SEGMENTS[KNIFE_SEG_SHOW],
                        );
                        knife_state.busy = Some(KnifeBusy {
                            remaining: vec![KNIFE_SEGMENTS[KNIFE_SEG_SHOW]],
                            seg_end: KNIFE_SEGMENTS[KNIFE_SEG_SHOW].end_secs(),
                            on_finish: KnifeFinish::Nothing,
                            interruptible: false,
                        });
                        knife_state.adjust_rolls = knife_state.adjust_rolls.wrapping_add(1);
                        knife_state.next_adjust_in =
                            roll_knife_adjust_delay(knife_state.adjust_rolls);
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

    // Advance an in-progress knife action (a single Show / Hide / slice / idle
    // "Adjust Grip" segment) — mirrors the sniper's own advance block above,
    // just without a reload/rechamber-style interrupt-and-resume queue (the
    // knife never needs one: swapping away just cuts straight to Hide,
    // nothing to resume later).
    if knife_state.busy.is_some() {
        // An attack always wins over the idle "Adjust Grip" fidget — cut it
        // short and swing immediately instead of waiting for it to finish;
        // landing (or missing) a hit matters more than a cosmetic idle
        // animation. Show/Hide/an in-progress slice are never interruptible.
        if knife_state
            .busy
            .as_ref()
            .is_some_and(|b| b.interruptible)
            && binds.fire.just_pressed(&keys, &mouse)
        {
            start_knife_slice(&mut knife_state, &mut knife_player, knife_node);
            request_stab(&cam, &mut pending_melee);
            return;
        }
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
            start_knife_slice(&mut knife_state, &mut knife_player, knife_node);
            request_stab(&cam, &mut pending_melee);
            return;
        }
        // No attack this frame — count down toward the next idle "Adjust
        // Grip" fidget (a cosmetic hands-resettle animation; see
        // `KnifeAnimState::next_adjust_in`).
        knife_state.next_adjust_in -= time.delta_secs();
        if knife_state.next_adjust_in <= 0.0 {
            let seg = KNIFE_SEGMENTS[KNIFE_SEG_ADJUST_GRIP];
            play_segment(&mut knife_player, knife_node, seg);
            knife_state.busy = Some(KnifeBusy {
                remaining: vec![seg],
                seg_end: seg.end_secs(),
                on_finish: KnifeFinish::Nothing,
                interruptible: true,
            });
            // Roll the next span now, not when this one finishes — otherwise
            // `next_adjust_in` sits at/below zero for the whole time this
            // fidget is playing and fires again immediately the instant it
            // ends (or every frame, since it's re-checked each frame while
            // idle) instead of waiting out a fresh span.
            knife_state.adjust_rolls = knife_state.adjust_rolls.wrapping_add(1);
            knife_state.next_adjust_in = roll_knife_adjust_delay(knife_state.adjust_rolls);
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

        // Hand the shot ray to `resolve_local_shot`: it kicks up the ground
        // dust and spawns our tracer instantly. The server also broadcasts this
        // shot; `net::receive_shots` drops the echo for our own peer so nothing
        // double-spawns.
        if let Some(cam) = cam_gt {
            shots.write(LocalShot {
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
