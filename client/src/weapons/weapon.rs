//! Weapon state machine: ammo, the primary's (sniper's or AK-74's)
//! fire/reload/rechamber queue, weapon swap, the throwing-knife throw/melee
//! mechanic, and the quick-melee key.

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use crate::keybinds::KeyBindings;
use crate::killcam;
use crate::player::WorldModelCamera;
use crate::settings::Settings;
use crate::util::{rand01, rand_roll};
use crate::{BulletImpact, FireTracer, GameSounds, GroundImpact, MuzzleFlashState, SmokeEmission};
use shared::ballistics::ground_impact;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use shared::weapon::WeaponId;

use super::ads::{noscope_spread_angle, Ads, NoScopeSpread};
use super::ak::{ak_locomotion, AkLoco, AkMotion, AkSettings, AK_MAG_SIZE, AK_ZOMBIES_TOTAL_MAGS};
use super::drink_arms::{DrinkPhase, PerkDrink};
use super::knife_view_model::{
    KnifeAnimation, KnifeAnimationPlayer, KnifeViewModel, KNIFE_SEGMENTS, KNIFE_SEG_ADJUST_GRIP,
    KNIFE_SEG_HIDE, KNIFE_SEG_SHOW, KNIFE_SLICE_SEGMENTS,
};
use super::recoil::{Shake, ShakeSettings};
use super::throw_arms::{
    park_throw_arms, play_throw, ThrowArmsAnimation, ThrowArmsAnimationPlayer, ThrowArmsSettings,
};
use super::view_model::{
    play_segment, play_segment_at, AnimationSegment, AnimationSettings, PrimaryRig, SegAct,
    SniperAnimationPlayer, ViewModel, ViewModelAnimation,
};

/// The sniper's magazine capacity, and the total number of magazines the
/// player carries in `FreeForAll` (current mag + reserve = `MAG_SIZE *
/// TOTAL_MAGS`). (The AK-74's are `ak::AK_MAG_SIZE` / `AK_ZOMBIES_TOTAL_MAGS`.)
pub(crate) const MAG_SIZE: u32 = 5;
pub(crate) const TOTAL_MAGS: u32 = 6;
/// Reserve rounds in `Freestyle` — effectively unlimited for practice.
const FREESTYLE_RESERVE: u32 = 1000;

/// Magazines' worth of sniper rounds (loaded mag included) a `Zombies` player
/// carries at most — topped back up at the ammo crate (`ammo_crate`).
const ZOMBIES_TOTAL_MAGS: u32 = 8;

/// Extra magazines' worth of sniper rounds a `Zombies` player can carry for
/// every Pack-a-Punch level on the sniper (`shared::pap`).
pub(crate) const PAP_EXTRA_MAGS_PER_LEVEL: u32 = 2;

/// `primary`'s magazine capacity.
pub(crate) fn mag_size(primary: WeaponId) -> u32 {
    match primary {
        WeaponId::Ak74 => AK_MAG_SIZE,
        _ => MAG_SIZE,
    }
}

/// Reserve rounds (beyond the loaded mag) a fresh life starts with in `mode`
/// with `primary` — also the most it holds.
fn starting_reserve(mode: shared::GameMode, primary: WeaponId) -> u32 {
    let mag = mag_size(primary);
    match mode {
        shared::GameMode::Freestyle => FREESTYLE_RESERVE,
        shared::GameMode::FreeForAll => mag * (TOTAL_MAGS - 1),
        shared::GameMode::Zombies if primary == WeaponId::Ak74 => mag * (AK_ZOMBIES_TOTAL_MAGS - 1),
        shared::GameMode::Zombies => mag * (ZOMBIES_TOTAL_MAGS - 1),
    }
}

/// A weapon action (fire / swap-to-secondary) cut a busy queue short — stash
/// whatever bolt-cycle / reload work is still outstanding as
/// `weapon.interrupted` so it forces its way back in (from the top) once the
/// primary is drawn again.
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
    while busy.remaining.first().is_some_and(|seg| seg.act == SegAct::Shoot) {
        busy.remaining.remove(0);
    }
    if let Some(seg) = busy.remaining.first().copied() {
        if matches!(seg.act, SegAct::Rechamber | SegAct::Reload | SegAct::ReloadEmpty) {
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

/// Every [`LocalShot`] kicks up dust where it lands and spawns the shooter's
/// own tracer instantly, rather than waiting on the server (other players'
/// tracers come off the server's authoritative `ShotResolved`,
/// `net::receive_shots`). The tracer ends at the first solid surface it meets
/// — the map's own colliders, the same model the server stops the bullet on —
/// or the flat ground plane, else at a max-range whiff; the server owns bot /
/// player hits, so there's no local impact point to use for those.
pub(crate) fn resolve_local_shot(
    rapier: ReadRapierContext,
    mut shots: EventReader<LocalShot>,
    mut ground_hit: ResMut<killcam::ReplayGroundImpact>,
    mut tracer_rec: ResMut<killcam::ReplayTracer>,
    mut impacts: EventWriter<GroundImpact>,
    mut holes: EventWriter<BulletImpact>,
    mut tracers: EventWriter<FireTracer>,
) {
    for shot in shots.read() {
        let max_range = WeaponId::Sniper.spec().max_range;
        let aim = shot.dir.normalize_or_zero();
        // Each candidate surface carries its normal (the collider's own, or
        // straight up for the flat plane) for the bullet hole.
        let wall_hit = rapier.single().ok().and_then(|r| {
            r.cast_ray_and_get_normal(shot.origin, aim, max_range, true, QueryFilter::default())
                .map(|(_, hit)| {
                    // Face the shooter, whichever way the collider's winding goes.
                    let n = if hit.normal.dot(aim) > 0.0 { -hit.normal } else { hit.normal };
                    (shot.origin + aim * hit.time_of_impact, n)
                })
        });
        let flat_hit = ground_impact(shot.origin, shot.dir).map(|p| (p, Vec3::Y));
        let surface = match (wall_hit, flat_hit) {
            (Some(w), Some(g)) => Some(if shot.origin.distance(w.0) <= shot.origin.distance(g.0) {
                w
            } else {
                g
            }),
            (w, g) => w.or(g),
        };
        let ground_pt = surface.map(|(p, _)| p);
        if let Some((p, normal)) = surface {
            impacts.write(GroundImpact(p));
            holes.write(BulletImpact { point: p, normal });
            // Stamp it onto this tick's `PlayerInput` too, so the kill cam can
            // replay the burst for the other players watching.
            ground_hit.0 = Some(p);
        }

        let end = ground_pt.unwrap_or_else(|| shot.origin + aim * max_range);
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
    /// Throwing knives left this life. One is used when a knife actually
    /// leaves the hand (a cancelled throw costs nothing); at 0 the
    /// throwing-knife key does nothing.
    pub(crate) throwing_knives: u32,
    /// Molotovs carried (`Zombies` only — picked up from dropped ones).
    pub(crate) molotovs: u32,
    /// Which lethal the lethal key throws — the only kind carried: picking up
    /// the other kind swaps to it (the old ones are dropped, see
    /// `knife_pickup`). Stays put when it runs out.
    pub(crate) lethal: Lethal,
    /// Set by a fresh loadout (`Default` / [`Weapon::refill_ammo`]):
    /// [`apply_loadout`] fills `reserve` and `throwing_knives` for the
    /// lobby's `GameMode` as soon as that's known, then clears it.
    loadout_pending: bool,
    pub(crate) busy: Option<WeaponBusy>,
    /// The slot currently equipped (switched the instant the swap key is
    /// pressed; the Hide / Show animation then plays out via `busy`).
    pub(crate) slot: WeaponSlot,
    /// A reload / rechamber cut short by a weapon switch. Replayed from the top —
    /// animation and sound — once the sniper is next drawn.
    interrupted: Option<WeaponBusy>,
    /// Our primary's Pack-a-Punch level — kept in step with the lobby by
    /// `pap::sync_pap_levels` (0 outside a `Zombies` game). Each level lets
    /// it carry more ([`Weapon::max_reserve`]).
    pub(crate) pap_level: u8,
    /// Which primary we carry — the sniper, or the AK-74 in a `Zombies` game
    /// we picked it for (`ak::sync_primary_model`, which also swaps the
    /// view model to match). The ammo counts are this weapon's.
    pub(crate) primary: WeaponId,
    /// Full-auto: when (`Time::elapsed_secs`) the next round may go.
    next_shot_at: f32,
    /// The AK's walk / jump animation state (`ak::ak_locomotion`).
    ak_loco: AkLoco,
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
            // Both filled per game mode by `apply_loadout`.
            reserve: 0,
            throwing_knives: 0,
            molotovs: 0,
            lethal: Lethal::ThrowingKnife,
            loadout_pending: true,
            busy: None,
            slot: WeaponSlot::Primary,
            interrupted: None,
            pap_level: 0,
            primary: WeaponId::Sniper,
            next_shot_at: 0.0,
            ak_loco: AkLoco::Idle,
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
    /// Whether the sniper holds every round it can in `mode` (mag + reserve).
    pub(crate) fn ammo_full(&self, mode: shared::GameMode) -> bool {
        self.mag + self.reserve >= self.mag_size() + self.max_reserve(mode)
    }

    /// The AK's walk / jump animation state (the debug panel shows it).
    pub(crate) fn ak_loco(&self) -> AkLoco {
        self.ak_loco
    }

    /// The primary's magazine capacity.
    pub(crate) fn mag_size(&self) -> u32 {
        mag_size(self.primary)
    }

    /// Carry `primary` from now on: a full mag of it, and its reserve filled
    /// for the game mode as soon as that's known (`apply_loadout`). Anything
    /// the old one was doing is dropped.
    pub(crate) fn set_primary(&mut self, primary: WeaponId) {
        self.primary = primary;
        self.mag = self.mag_size();
        self.busy = None;
        self.interrupted = None;
        self.next_shot_at = 0.0;
        self.ak_loco = AkLoco::Idle;
        self.loadout_pending = true;
    }

    /// The most reserve rounds the primary holds in `mode` — more for every
    /// Pack-a-Punch level in `Zombies`.
    fn max_reserve(&self, mode: shared::GameMode) -> u32 {
        let packed = if mode == shared::GameMode::Zombies {
            self.mag_size() * PAP_EXTRA_MAGS_PER_LEVEL * self.pap_level as u32
        } else {
            0
        };
        starting_reserve(mode, self.primary) + packed
    }

    /// Top the sniper up to every round it can hold in `mode` (the ammo
    /// crate). The extra goes into the reserve — the mag and chamber are
    /// left alone, so a reload loads it like any other.
    pub(crate) fn fill_ammo(&mut self, mode: shared::GameMode) {
        self.reserve = (self.mag_size() + self.max_reserve(mode)).saturating_sub(self.mag);
    }

    /// Top the sniper right up — a full mag *and* a full reserve for its
    /// capacity in `mode` (a Pack-a-Punch, Cold War style).
    pub(crate) fn fill_mag_and_reserve(&mut self, mode: shared::GameMode) {
        self.mag = self.mag_size();
        self.reserve = self.max_reserve(mode);
    }

    /// How many of the current lethal are left.
    pub(crate) fn lethal_count(&self) -> u32 {
        match self.lethal {
            Lethal::ThrowingKnife => self.throwing_knives,
            Lethal::Molotov => self.molotovs,
        }
    }

    /// One `kind` just left the hand.
    fn use_lethal(&mut self, kind: Lethal) {
        match kind {
            Lethal::ThrowingKnife => self.throwing_knives = self.throwing_knives.saturating_sub(1),
            Lethal::Molotov => self.molotovs = self.molotovs.saturating_sub(1),
        }
    }

    /// The kind of lethal actually carried — `None` once the current one has
    /// run out (so either kind is picked up without swapping).
    pub(crate) fn carried_lethal(&self) -> Option<Lethal> {
        (self.lethal_count() > 0).then_some(self.lethal)
    }

    /// Whether as many of `kind` are carried as can be (Freestyle's
    /// bottomless knives count as full).
    pub(crate) fn lethal_full(&self, kind: Lethal) -> bool {
        match kind {
            Lethal::ThrowingKnife => self.throwing_knives >= shared::throwing_knife::MAX_CARRIED,
            Lethal::Molotov => self.molotovs >= shared::molotov::MAX_MOLOTOVS,
        }
    }

    /// A molotov was picked up: one more (up to the most that can be
    /// carried), and it's now the only lethal — any knives were dropped.
    pub(crate) fn add_molotov(&mut self) {
        if self.lethal != Lethal::Molotov {
            self.throwing_knives = 0;
        }
        self.molotovs = (self.molotovs + 1).min(shared::molotov::MAX_MOLOTOVS);
        self.lethal = Lethal::Molotov;
    }

    /// A throwing knife was picked up: one more (up to the most that can be
    /// carried), and it's now the only lethal — any molotovs were dropped.
    pub(crate) fn add_throwing_knife(&mut self) {
        if self.lethal != Lethal::ThrowingKnife {
            self.molotovs = 0;
        }
        if self.throwing_knives < shared::throwing_knife::MAX_CARRIED {
            self.throwing_knives += 1;
        }
        self.lethal = Lethal::ThrowingKnife;
    }

    pub(crate) fn refill_ammo(&mut self) {
        self.mag = self.mag_size();
        self.reserve = 0;
        self.loadout_pending = true;
    }
}

/// Fill `Weapon::reserve` and `Weapon::throwing_knives` for a fresh loadout
/// (game start or respawn) — both depend on the lobby's `GameMode`
/// ([`starting_reserve`], `shared::throwing_knife::starting_knives`), which
/// isn't known where the loadout is reset, so they're applied here once the
/// local player's started lobby is found.
pub(crate) fn apply_loadout(
    mut weapon: ResMut<Weapon>,
    local: Query<&lightyear::prelude::LocalId, With<crate::net::GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    if !weapon.loadout_pending {
        return;
    }
    let Some(me) = local.iter().next().map(|l| l.0) else {
        return;
    };
    let Some(lobby) = lobbies.iter().find(|l| l.started && l.has(me)) else {
        return;
    };
    weapon.reserve = starting_reserve(lobby.mode, weapon.primary);
    weapon.throwing_knives = shared::throwing_knife::starting_knives(lobby.mode);
    weapon.molotovs = 0;
    weapon.lethal = Lethal::ThrowingKnife;
    weapon.loadout_pending = false;
}

/// The lethal equipment the lethal key throws — see [`Weapon::lethal`].
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Lethal {
    #[default]
    ThrowingKnife,
    Molotov,
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
    /// What this sequence is throwing — [`Weapon::lethal`] at the press, so
    /// picking something up mid-throw can't swap it.
    pub(crate) kind: Lethal,
    /// The throw clip has started this round, so the knife model in the
    /// arms' hand (`throw_arms::ThrowKnifeModel`) is gone. Cleared on the
    /// next press.
    thrown: bool,
    /// Last frame's `ThrowArmsSettings::debug_hold_key`, so turning it on
    /// counts as a single key *press* (not a re-press every time the sequence
    /// returns to idle while it's still on).
    debug_hold_prev: bool,
    /// The throw request for the server has been filed this round (at
    /// `ThrowArmsSettings::throw_release_secs` into the throw clip).
    throw_sent: bool,
    /// The filed throw request — the camera's `(eye, forward)` at the release
    /// point — until `thrown_knife::send_throw_requests` sends it.
    pending_throw: Option<(Vec3, Vec3)>,
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

    /// Take the filed throw request, if any (`(eye position, aim direction)`).
    pub(crate) fn take_throw_request(&mut self) -> Option<(Vec3, Vec3)> {
        self.pending_throw.take()
    }

    /// Whether a molotov throw request is waiting to be sent (the throwing
    /// knife's are sent by `thrown_knife::send_throw_requests`, a molotov's
    /// by `molotov::send_throw_requests`).
    pub(crate) fn has_molotov_request(&self) -> bool {
        self.pending_throw.is_some() && self.kind == Lethal::Molotov
    }

    /// Whether a throwing-knife throw request is waiting to be sent.
    pub(crate) fn has_throw_request(&self) -> bool {
        self.pending_throw.is_some() && self.kind == Lethal::ThrowingKnife
    }

    /// Whether the throwing-knife reticle should be up instead of the sniper's
    /// centre dot: from the key press until the arms have finished sliding
    /// away again (`active` alone drops the moment the throw ends, while the
    /// arms are still on screen).
    pub(crate) fn crosshair_up(&self) -> bool {
        self.active || self.slide > 0.0
    }

    /// Whether the knife model should be showing in the arms' hand: from the
    /// press until the throw animation starts (a cancelled throw keeps it as
    /// the arms slide away).
    pub(crate) fn knife_in_hand(&self) -> bool {
        self.kind == Lethal::ThrowingKnife && self.phase != ThrowPhase::Idle && !self.thrown
    }

    /// Whether a molotov's rag is lit in the hand: from the press until it's
    /// thrown or the throw's cancelled (the light sound plays meanwhile).
    pub(crate) fn molotov_lit(&self) -> bool {
        self.kind == Lethal::Molotov && self.active && !self.thrown
    }

    /// [`Self::knife_in_hand`] for the molotov.
    pub(crate) fn molotov_in_hand(&self) -> bool {
        self.kind == Lethal::Molotov && self.phase != ThrowPhase::Idle && !self.thrown
    }
}

/// Where a quick melee (the melee key with the sniper out) is up to. Like
/// the throwing knife it's an overlay, not a real weapon switch —
/// `Weapon::slot` stays `Primary` throughout: the sniper's Hide plays sped up
/// (`ThrowArmsSettings::weapon_hide_speed`, the throwing knife's quick stow),
/// the knife's Show plays at that same speed, one slice (the stab — filed for
/// the server the moment it starts), the knife's Hide sped up, and then the
/// sniper's normal Show, resuming any reload / rechamber the press cut short.
/// With the knife already out, the melee key is just a normal stab and none
/// of this runs.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum MeleePhase {
    #[default]
    Idle,
    /// The sniper's quick Hide.
    Stowing,
    /// The knife's quick Show.
    Drawing,
    /// The slice.
    Stabbing,
    /// The knife's quick Hide; the sniper comes back once it's done.
    Holstering,
}

#[derive(Resource, Default)]
pub(crate) struct QuickMelee {
    pub(crate) phase: MeleePhase,
    /// Seconds in the current phase — each phase also ends once its clip
    /// should have finished, so a clip parked by a kill cam can't wedge it.
    elapsed: f32,
}

/// Whether `seg` has played out on `player` (at `speed`), or — however the
/// clip reports — has had long enough to (`elapsed` seconds in).
fn segment_done(
    player: &AnimationPlayer,
    node: AnimationNodeIndex,
    seg: AnimationSegment,
    elapsed: f32,
    speed: f32,
) -> bool {
    let finished = player
        .animation(node)
        .is_none_or(|a| a.is_finished() || a.seek_time() >= seg.end_secs());
    finished || elapsed > (seg.end_secs() - seg.start_secs()) / speed.max(0.01) + 0.25
}

/// Re-entering the world always starts on the sniper, model shown, animation
/// parked at rest — so quitting mid-swap can't leave the next game weaponless.
pub(crate) fn reset_weapon(
    mut weapon: ResMut<Weapon>,
    mut melee: ResMut<QuickMelee>,
    mut knife: ResMut<ThrowingKnife>,
    mut drink: ResMut<PerkDrink>,
    mut view_model: Query<(&ViewModelAnimation, &mut Visibility), With<ViewModel>>,
    mut players: Query<&mut AnimationPlayer, With<SniperAnimationPlayer>>,
) {
    *weapon = Weapon::default();
    *melee = QuickMelee::default();
    *knife = ThrowingKnife::default();
    *drink = PerkDrink::default();
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

/// Start the quick stow shared by the throwing knife and perk drinking: play
/// the equipped weapon's Hide at `hide_speed`, cutting a primary reload /
/// rechamber short (it resumes when the weapon's drawn again). First settles a
/// half-finished swap so "the weapon that was out" is unambiguous, and so
/// there's nothing visible to play a Hide on: a primary Hide in progress (slot
/// already `Secondary`) just finishes instantly, and a knife Hide in progress
/// hands the slot straight back to the primary with both models already away.
/// Returns `true` if that left nothing on screen (no Hide to wait for).
#[allow(clippy::too_many_arguments)]
fn stow_equipped(
    weapon: &mut Weapon,
    knife_state: &mut KnifeAnimState,
    rig: &mut PrimaryRig,
    knife_player: &mut AnimationPlayer,
    knife_node: AnimationNodeIndex,
    view_model_vis: &mut Visibility,
    knife_vis: &mut Visibility,
    action_sounds: &Query<Entity, With<WeaponActionSound>>,
    commands: &mut Commands,
    hide_speed: f32,
) -> bool {
    let mut nothing_shown = false;
    if weapon.slot == WeaponSlot::Secondary
        && weapon
            .busy
            .as_ref()
            .is_some_and(|b| b.on_finish == WeaponFinish::Holster)
    {
        weapon.busy = None;
        for e in action_sounds {
            commands.entity(e).try_despawn();
        }
        rig.park();
        *view_model_vis = Visibility::Hidden;
        nothing_shown = true;
    }
    if let Some(busy) = knife_state.busy.take() {
        if busy.on_finish == KnifeFinish::Hidden {
            weapon.slot = WeaponSlot::Primary;
            *knife_vis = Visibility::Hidden;
            nothing_shown = true;
        }
        if let Some(active_anim) = knife_player.animation_mut(knife_node) {
            active_anim.seek_to(0.0);
            active_anim.pause();
        }
    }
    if nothing_shown {
        return true;
    }
    match weapon.slot {
        WeaponSlot::Primary => {
            if let Some(busy) = weapon.busy.take() {
                for e in action_sounds {
                    commands.entity(e).try_despawn();
                }
                stash_interrupted(weapon, busy);
            }
            let hide = rig.seg(SegAct::Hide);
            rig.play_at(hide, hide_speed);
        }
        WeaponSlot::Secondary => {
            play_segment(knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_HIDE]);
            if let Some(active_anim) = knife_player.animation_mut(knife_node) {
                active_anim.set_speed(hide_speed);
            }
        }
    }
    false
}

/// Whether [`stow_equipped`]'s sped-up Hide is done — and if so, drop the
/// model and park its animation. `elapsed` is the time spent stowing, so a
/// Hide clip that never reports done (e.g. reset by a kill cam) can't wedge
/// the sequence.
#[allow(clippy::too_many_arguments)]
fn stow_finished(
    weapon: &Weapon,
    rig: &mut PrimaryRig,
    knife_player: &mut AnimationPlayer,
    knife_node: AnimationNodeIndex,
    view_model_vis: &mut Visibility,
    knife_vis: &mut Visibility,
    elapsed: f32,
    hide_speed: f32,
) -> bool {
    let (seg, done) = match weapon.slot {
        WeaponSlot::Primary => {
            let hide = rig.seg(SegAct::Hide);
            (hide, rig.reached(hide, hide.end_secs()))
        }
        WeaponSlot::Secondary => (
            KNIFE_SEGMENTS[KNIFE_SEG_HIDE],
            knife_player.animation(knife_node).is_none_or(|a| {
                a.is_finished() || a.seek_time() >= KNIFE_SEGMENTS[KNIFE_SEG_HIDE].end_secs()
            }),
        ),
    };
    let expected = (seg.end_secs() - seg.start_secs()) / hide_speed.max(0.01);
    if !done && elapsed <= expected + 0.25 {
        return false;
    }
    match weapon.slot {
        WeaponSlot::Primary => {
            rig.park();
            *view_model_vis = Visibility::Hidden;
        }
        WeaponSlot::Secondary => {
            if let Some(active_anim) = knife_player.animation_mut(knife_node) {
                active_anim.seek_to(0.0);
                active_anim.pause();
            }
            *knife_vis = Visibility::Hidden;
        }
    }
    true
}

/// Draw whichever weapon was out before the throwing / drinking arms came up — the
/// primary (its normal Show, resuming any interrupted reload once that
/// finishes) or the knife (its own Show, then idle-out).
#[allow(clippy::too_many_arguments)]
fn redraw_active_weapon(
    weapon: &mut Weapon,
    knife_state: &mut KnifeAnimState,
    rig: &mut PrimaryRig,
    knife_player: &mut AnimationPlayer,
    knife_node: AnimationNodeIndex,
    view_model_vis: &mut Visibility,
    knife_vis: &mut Visibility,
    swap_speed: f32,
    primary_swap: f32,
) {
    match weapon.slot {
        WeaponSlot::Primary => {
            *view_model_vis = Visibility::Inherited;
            let show = rig.seg(SegAct::Show);
            rig.play_at(show, primary_swap);
            weapon.busy = Some(WeaponBusy {
                remaining: vec![show],
                seg_end: show.end_secs(),
                on_finish: WeaponFinish::Draw,
            });
        }
        WeaponSlot::Secondary => {
            *knife_vis = Visibility::Inherited;
            play_segment_at(knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_SHOW], swap_speed);
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

/// The speeds and sounds a primary segment starts with.
struct SegCtx<'a> {
    sounds: &'a GameSounds,
    /// The panel's sniper rechamber speed (`AnimationSettings`).
    rechamber_anim: f32,
    /// Nitro Brew's (1 without it).
    reload_speed: f32,
    rechamber_speed: f32,
    ak: &'a AkSettings,
}

/// Start `seg` on the primary, with its sound (heard by us, and — via the
/// recorded sound bits — by the lobby and in replays) and speed: a sniper
/// rechamber or reload, or the AK's fast or full reload. Anything else just
/// plays.
fn begin_segment(
    rig: &mut PrimaryRig,
    seg: AnimationSegment,
    ctx: &SegCtx,
    commands: &mut Commands,
    snd: &mut killcam::ReplaySoundBits,
) {
    let ak = rig.weapon == WeaponId::Ak74;
    let (sound, bit, sound_speed, anim_speed) = match seg.act {
        SegAct::Rechamber => (
            &ctx.sounds.rechamber,
            killcam::SND_RECHAMBER,
            ctx.rechamber_speed,
            ctx.rechamber_anim * ctx.rechamber_speed,
        ),
        SegAct::Reload if ak => (
            &ctx.sounds.ak_reload_fast,
            killcam::SND_AK_RELOAD_FAST,
            ctx.reload_speed * ctx.ak.reload_speed,
            ctx.reload_speed * ctx.ak.reload_speed,
        ),
        SegAct::Reload => (&ctx.sounds.reload, killcam::SND_RELOAD, ctx.reload_speed, ctx.reload_speed),
        SegAct::ReloadEmpty => (
            &ctx.sounds.ak_reload,
            killcam::SND_AK_RELOAD,
            ctx.reload_speed * ctx.ak.reload_speed,
            ctx.reload_speed * ctx.ak.reload_speed,
        ),
        _ => {
            rig.play(seg);
            return;
        }
    };
    commands.spawn((
        AudioPlayer::new(sound.clone()),
        PlaybackSettings::DESPAWN.with_speed(sound_speed),
        WeaponActionSound,
    ));
    snd.note(bit);
    rig.play_at(seg, anim_speed);
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
            (&mut AnimationPlayer, Option<&mut AnimationTransitions>),
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
    (mut knife, mut knife_state, mut pending_melee, mut drink, mut melee): (
        ResMut<ThrowingKnife>,
        ResMut<KnifeAnimState>,
        ResMut<PendingMelee>,
        ResMut<PerkDrink>,
        ResMut<QuickMelee>,
    ),
    (mut pending_shot, mut shake, mut muzzle, mut smoke): (
        ResMut<PendingShot>,
        ResMut<Shake>,
        ResMut<MuzzleFlashState>,
        ResMut<SmokeEmission>,
    ),
    mut shots: EventWriter<LocalShot>,
    mut snd: ResMut<killcam::ReplaySoundBits>,
    (shake_cfg, sounds, anim, ads, spread_cfg, settings, time, arms_settings, nitro): (
        Res<ShakeSettings>,
        Res<GameSounds>,
        Res<AnimationSettings>,
        Res<Ads>,
        Res<NoScopeSpread>,
        Res<Settings>,
        Res<Time>,
        Res<ThrowArmsSettings>,
        Res<crate::zombies_hud::NitroBrew>,
    ),
    (physics, ak_cfg): (
        Query<&crate::player::PlayerPhysics, With<crate::player::Player>>,
        Res<AkSettings>,
    ),
    mut commands: Commands,
) {
    // The view model's still switching over to our primary
    // (`ak::sync_primary_model`).
    if view_model.weapon != weapon.primary {
        return;
    }
    // Nitro Brew's speed-ups (all `1.0` without it). The sped-up Hide for the
    // throwing / drinking arms gets the swap speed-up on top.
    let (reload_speed, rechamber_speed, swap_speed) = (nitro.reload(), nitro.rechamber(), nitro.swap());
    // The primary's own draw / hide (the AK's panel speed on top).
    let primary_swap = if weapon.primary == WeaponId::Ak74 {
        swap_speed * ak_cfg.draw_speed
    } else {
        swap_speed
    };
    let stow_speed = arms_settings.weapon_hide_speed * swap_speed;
    let Some((player, transitions)) = players.iter_mut().next() else {
        return;
    };
    let mut rig = PrimaryRig::new(player, transitions, &view_model, ak_cfg.blend_secs);
    let seg_ctx = SegCtx {
        sounds: &sounds,
        rechamber_anim: anim.rechamber_speed,
        reload_speed,
        rechamber_speed,
        ak: &ak_cfg,
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
    // The debug panel's "hold the key for me" toggle acts as the throwing-knife
    // key being down (its rising edge is the press; needs no cursor lock,
    // since the panel needs a free cursor).
    let debug_hold = arms_settings.debug_hold_key;
    let debug_press = debug_hold && !knife.debug_hold_prev;
    if knife.debug_hold_prev != debug_hold {
        knife.debug_hold_prev = debug_hold;
    }
    let key_held = debug_hold || binds.lethal.pressed(&keys, &mouse);
    let melee_pressed = locked && binds.melee.just_pressed(&keys, &mouse);
    let melee_idle = melee.phase == MeleePhase::Idle;

    // Throwing knife — see [`ThrowPhase`]. Press: settle any half-finished
    // swap, then play the equipped weapon's Hide (sped up), which also cuts a
    // sniper reload / rechamber short for a "silent" quickscope. Once it's
    // done the arms slide up (`throw_arms::slide_throw_arms`). Release (with
    // the arms fully in): the throw clip plays once. When it ends the arms
    // slide away and the weapon that was out is drawn again with its normal
    // Show, resuming whatever the press interrupted. Swap-weapon while the
    // arms are out cancels instead: no clip, the arms slide away and the
    // same weapon comes back.
    // A perk just bought: stow the weapon the same quick way as the throwing
    // knife does, then the drinking arms play their clip once
    // (`drink_arms::play_perk_drink`) and hand back with `DrinkPhase::Done`,
    // when the weapon that was out is drawn again. Waits for a throwing-knife
    // sequence already under way to finish first.
    if drink.requested.is_some()
        && drink.phase == DrinkPhase::Idle
        && knife.phase == ThrowPhase::Idle
        && melee_idle
    {
        drink.perk = drink.requested.take();
        drink.stow_elapsed = 0.0;
        let nothing_shown = stow_equipped(
            &mut weapon,
            &mut knife_state,
            &mut rig,
            &mut knife_player,
            knife_node,
            &mut view_model_vis,
            &mut knife_vis,
            &action_sounds,
            &mut commands,
            stow_speed,
        );
        drink.phase = if nothing_shown {
            DrinkPhase::Drinking
        } else {
            DrinkPhase::Stowing
        };
        return;
    }
    match drink.phase {
        DrinkPhase::Idle => {}
        DrinkPhase::Stowing => {
            drink.stow_elapsed += time.delta_secs();
            if stow_finished(
                &weapon,
                &mut rig,
                &mut knife_player,
                knife_node,
                &mut view_model_vis,
                &mut knife_vis,
                drink.stow_elapsed,
                stow_speed,
            ) {
                drink.phase = DrinkPhase::Drinking;
            }
            return;
        }
        // The drinking arms own this phase; nothing else is accepted.
        DrinkPhase::Drinking => return,
        DrinkPhase::Done => {
            drink.phase = DrinkPhase::Idle;
            redraw_active_weapon(
                &mut weapon,
                &mut knife_state,
                &mut rig,
                &mut knife_player,
                knife_node,
                &mut view_model_vis,
                &mut knife_vis,
                swap_speed,
                primary_swap,
            );
            return;
        }
    }

    if (debug_press || (locked && binds.lethal.just_pressed(&keys, &mouse)))
        && knife.phase == ThrowPhase::Idle
        && drink.requested.is_none()
        && melee_idle
        && weapon.lethal_count() > 0
    {
        knife.active = true;
        knife.kind = weapon.lethal;
        knife.thrown = false;
        knife.throw_sent = false;
        knife.pending_throw = None;
        knife.stow_elapsed = 0.0;
        if let Some(arms) = arms_player.as_mut() {
            park_throw_arms(arms, arms_node);
        }
        let nothing_shown = stow_equipped(
            &mut weapon,
            &mut knife_state,
            &mut rig,
            &mut knife_player,
            knife_node,
            &mut view_model_vis,
            &mut knife_vis,
            &action_sounds,
            &mut commands,
            stow_speed,
        );
        knife.phase = if nothing_shown {
            ThrowPhase::Held
        } else {
            ThrowPhase::Stowing
        };
        return;
    }
    match knife.phase {
        ThrowPhase::Idle => {}
        ThrowPhase::Stowing => {
            // Wait out the sped-up Hide (a swap press is ignored — it only
            // lasts a moment), then drop the model and bring the arms in.
            knife.stow_elapsed += time.delta_secs();
            if stow_finished(
                &weapon,
                &mut rig,
                &mut knife_player,
                knife_node,
                &mut view_model_vis,
                &mut knife_vis,
                knife.stow_elapsed,
                stow_speed,
            ) {
                knife.phase = ThrowPhase::Held;
            }
            return;
        }
        ThrowPhase::Held => {
            if binds.swap_weapon.just_pressed(&keys, &mouse) {
                // Cancelled: no throw animation.
                knife.active = false;
                knife.phase = ThrowPhase::Returning;
            } else if !key_held && knife.slide >= 1.0 {
                // Released with the arms fully in place: throw.
                knife.phase = ThrowPhase::Throwing;
                knife.thrown = true;
                // The throw sound the moment it's released: heard locally
                // here, and (via the recorded sound bits) positionally by the
                // rest of the lobby and in kill-cam replays. (The molotov
                // has no sound of its own yet.)
                if knife.kind == Lethal::ThrowingKnife {
                    commands.spawn((
                        AudioPlayer::new(sounds.knife_throw.clone()),
                        PlaybackSettings::DESPAWN,
                    ));
                    snd.note(killcam::SND_THROW);
                }
                if let Some(arms) = arms_player.as_mut() {
                    play_throw(arms, arms_node);
                }
            }
            return;
        }
        ThrowPhase::Throwing => {
            // Throw clip playing out — nothing else is accepted until it
            // finishes (a swap press is ignored; the knife is already gone).
            let clip = arms_player.as_ref().and_then(|p| p.animation(arms_node));
            let done = clip.is_none_or(|a| a.is_finished());
            // The knife leaves the hand part-way through the clip: file the
            // throw request (eye + aim right now) for the server. Sent by
            // `thrown_knife::send_throw_requests`; if the clip ends first, it
            // goes now rather than never.
            if !knife.throw_sent
                && (done || clip.is_some_and(|a| a.seek_time() >= arms_settings.throw_release_secs))
            {
                knife.throw_sent = true;
                let kind = knife.kind;
                weapon.use_lethal(kind);
                if let Ok(cam) = cam.single() {
                    knife.pending_throw = Some((cam.translation(), cam.forward().as_vec3()));
                }
            }
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
                    &mut rig,
                    &mut knife_player,
                    knife_node,
                    &mut view_model_vis,
                    &mut knife_vis,
                    swap_speed,
                    primary_swap,
                );
            }
            return;
        }
    }

    // Quick melee — see [`MeleePhase`]. Only from the sniper (with the knife
    // out the key is a normal stab, below), and not while the knife is still
    // mid-swap. Like the throwing knife, it cuts a reload / rechamber short.
    if melee_idle
        && melee_pressed
        && weapon.slot == WeaponSlot::Primary
        && knife_state.busy.is_none()
    {
        melee.elapsed = 0.0;
        stow_equipped(
            &mut weapon,
            &mut knife_state,
            &mut rig,
            &mut knife_player,
            knife_node,
            &mut view_model_vis,
            &mut knife_vis,
            &action_sounds,
            &mut commands,
            stow_speed,
        );
        melee.phase = MeleePhase::Stowing;
        return;
    }
    let dt = time.delta_secs();
    match melee.phase {
        MeleePhase::Idle => {}
        MeleePhase::Stowing => {
            melee.elapsed += dt;
            if stow_finished(
                &weapon,
                &mut rig,
                &mut knife_player,
                knife_node,
                &mut view_model_vis,
                &mut knife_vis,
                melee.elapsed,
                stow_speed,
            ) {
                **knife_vis = Visibility::Inherited;
                play_segment_at(&mut knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_SHOW], stow_speed);
                melee.phase = MeleePhase::Drawing;
                melee.elapsed = 0.0;
            }
            return;
        }
        MeleePhase::Drawing => {
            melee.elapsed += dt;
            if segment_done(&knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_SHOW], melee.elapsed, stow_speed) {
                start_knife_slice(&mut knife_state, &mut knife_player, knife_node);
                request_stab(&cam, &mut pending_melee);
                melee.phase = MeleePhase::Stabbing;
                melee.elapsed = 0.0;
            }
            return;
        }
        MeleePhase::Stabbing => {
            melee.elapsed += dt;
            // (No slice left in `knife_state` — e.g. a kill cam reset it —
            // counts as done.)
            let done = knife_state
                .busy
                .as_ref()
                .and_then(|b| b.remaining.first().copied())
                .is_none_or(|seg| segment_done(&knife_player, knife_node, seg, melee.elapsed, 1.0));
            if done {
                knife_state.busy = None;
                play_segment_at(&mut knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_HIDE], stow_speed);
                melee.phase = MeleePhase::Holstering;
                melee.elapsed = 0.0;
            }
            return;
        }
        MeleePhase::Holstering => {
            melee.elapsed += dt;
            if segment_done(&knife_player, knife_node, KNIFE_SEGMENTS[KNIFE_SEG_HIDE], melee.elapsed, stow_speed) {
                if let Some(active) = knife_player.animation_mut(knife_node) {
                    active.seek_to(0.0);
                    active.pause();
                }
                **knife_vis = Visibility::Hidden;
                melee.phase = MeleePhase::Idle;
                // `slot` is still `Primary`: the sniper's Show, resuming
                // whatever the press cut short.
                redraw_active_weapon(
                    &mut weapon,
                    &mut knife_state,
                    &mut rig,
                    &mut knife_player,
                    knife_node,
                    &mut view_model_vis,
                    &mut knife_vis,
                    swap_speed,
                    primary_swap,
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
                let hide = rig.seg(SegAct::Hide);
                rig.play_at(hide, primary_swap);
                weapon.busy = Some(WeaponBusy {
                    remaining: vec![hide],
                    seg_end: hide.end_secs(),
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
                play_segment_at(
                    &mut knife_player,
                    knife_node,
                    KNIFE_SEGMENTS[KNIFE_SEG_HIDE],
                    swap_speed,
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
            let done = busy
                .remaining
                .first()
                .is_none_or(|&seg| rig.reached(seg, busy.seg_end));
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
            // A rechamber, or an auto-reload rolling straight out of a shot.
            Ok(next) => begin_segment(&mut rig, next, &seg_ctx, &mut commands, &mut snd),
            Err(on_finish) => {
                // Whole queue played out: park at the rest pose (the AK's
                // walk / idle picks up from here), apply effect.
                rig.park();
                weapon.busy = None;
                match on_finish {
                    WeaponFinish::Nothing => {}
                    WeaponFinish::Reload => {
                        let moved = weapon.mag_size().saturating_sub(weapon.mag).min(weapon.reserve);
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
                        // Just the local player hears the equip sound.
                        commands.spawn((
                            AudioPlayer::new(sounds.knife_equip.clone()),
                            PlaybackSettings::DESPAWN,
                        ));
                        play_segment_at(
                            &mut knife_player,
                            knife_node,
                            KNIFE_SEGMENTS[KNIFE_SEG_SHOW],
                            swap_speed,
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
                            begin_segment(&mut rig, resumed.remaining[0], &seg_ctx, &mut commands, &mut snd);
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
            && (binds.fire.just_pressed(&keys, &mouse) || melee_pressed)
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
                    // Knife fully hidden — hand back off to the sniper. Just the
                    // local player hears the equip sound.
                    **knife_vis = Visibility::Hidden;
                    weapon.slot = WeaponSlot::Primary;
                    **view_model_vis = Visibility::Inherited;
                    commands.spawn((
                        AudioPlayer::new(sounds.sniper_equip.clone()),
                        PlaybackSettings::DESPAWN,
                    ));
                    let show = rig.seg(SegAct::Show);
                    rig.play_at(show, primary_swap);
                    weapon.busy = Some(WeaponBusy {
                        remaining: vec![show],
                        seg_end: show.end_secs(),
                        on_finish: WeaponFinish::Draw,
                    });
                }
            }
        }
        return;
    }

    // The AK, out and free: its idle / jump clips — unless a shot's
    // still playing out.
    if weapon.slot == WeaponSlot::Primary && rig.weapon == WeaponId::Ak74 {
        let shot = rig.seg(SegAct::Shoot);
        let shooting = rig.is_main(shot) && !rig.reached(shot, shot.end_secs());
        if !shooting {
            let (grounded, rising) = physics
                .single()
                .map_or((true, false), |p| (p.grounded, p.vertical_velocity > 0.0));
            let motion = AkMotion { grounded, rising };
            ak_locomotion(&mut rig, &mut weapon.ak_loco, motion, &ak_cfg);
        }
    }

    // Idle: only take input while the cursor is captured (i.e. in-game).
    if !locked {
        return;
    }
    if weapon.slot == WeaponSlot::Secondary {
        // Knife idle-out, waiting on a left click (or the melee key) — one of
        // the four slices, picked at random each time.
        if binds.fire.just_pressed(&keys, &mouse) || melee_pressed {
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
    // Only `WeaponSlot::Primary` is left.

    // The AK-74: full-auto — a round every `fire_interval` while the trigger's
    // held, each restarting its Shot clip. No bolt to work: the last round
    // out goes straight into the full reload (auto-reload on).
    if rig.weapon == WeaponId::Ak74 {
        let now = time.elapsed_secs();
        if binds.fire.pressed(&keys, &mouse) && weapon.mag > 0 && now >= weapon.next_shot_at {
            // Held: keep the cadence even across uneven frames (the next
            // round's due one interval after this one was); a fresh pull
            // after a pause counts from now — one round, not a catch-up.
            let interval = ak_cfg.fire_interval.max(0.02);
            let due = if now - weapon.next_shot_at < interval {
                weapon.next_shot_at
            } else {
                now
            };
            weapon.next_shot_at = due + interval;
            weapon.mag -= 1;
            shake.trauma = (shake.trauma + ak_cfg.trauma_per_shot).min(1.0);
            shake.recoil = ak_cfg.recoil_kick;
            muzzle.shots = muzzle.shots.wrapping_add(1);
            muzzle.roll = rand_roll(muzzle.shots);
            muzzle.intensity = 1.0;
            commands.spawn((
                AudioPlayer::new(sounds.ak_shot.clone()),
                PlaybackSettings::DESPAWN,
            ));
            snd.note(killcam::SND_AK_SHOT);
            let cam_gt = cam.single().ok();
            let dir = cam_gt
                .map(|cam| {
                    let max = ak_cfg.spread_rad(ads.t);
                    let yaw = (rand01(muzzle.shots.wrapping_mul(0x9E37_79B9)) * 2.0 - 1.0) * max;
                    let pitch = (rand01(muzzle.shots.wrapping_mul(0x85EB_CA6B) ^ 0xDEAD_BEEF) * 2.0 - 1.0) * max;
                    cam.rotation() * Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z
                })
                .unwrap_or(Vec3::NEG_Z);
            pending_shot.0 = Some(dir);
            if let Some(cam) = cam_gt {
                shots.write(LocalShot {
                    origin: cam.translation(),
                    dir,
                });
            }
            let shot = rig.seg(SegAct::Shoot);
            rig.play_with(
                shot,
                ak_cfg.shot_speed,
                RepeatAnimation::Never,
                std::time::Duration::from_secs_f32(ak_cfg.shot_blend_secs.max(0.0)),
            );
            if weapon.mag == 0 && settings.auto_reload && weapon.reserve > 0 {
                weapon.busy = Some(WeaponBusy {
                    remaining: vec![shot, rig.seg(SegAct::ReloadEmpty)],
                    seg_end: shot.end_secs(),
                    on_finish: WeaponFinish::Reload,
                });
            }
        } else if binds.fire.just_pressed(&keys, &mouse) && weapon.mag == 0 {
            commands.spawn((
                AudioPlayer::new(sounds.out_of_ammo.clone()),
                PlaybackSettings::DESPAWN,
            ));
        } else if binds.reload.just_pressed(&keys, &mouse)
            && weapon.reserve > 0
            && weapon.mag < weapon.mag_size()
        {
            // From empty: a fresh mag and a round chambered; otherwise just
            // the mag swap.
            let seg = rig.seg(if weapon.mag == 0 { SegAct::ReloadEmpty } else { SegAct::Reload });
            begin_segment(&mut rig, seg, &seg_ctx, &mut commands, &mut snd);
            weapon.busy = Some(WeaponBusy {
                remaining: vec![seg],
                seg_end: seg.end_secs(),
                on_finish: WeaponFinish::Reload,
            });
        }
        return;
    }

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
        let shoot = rig.seg(SegAct::Shoot);
        rig.play(shoot);

        // Bolt-action cycle. After a shot that leaves rounds in the mag, work
        // the bolt (Rechamber) to eject the spent case and feed the next round.
        // The shot that empties the mag leaves the spent case sitting in the
        // chamber — the Reload clip only swaps the magazine, it never touches
        // the bolt — so with auto-reload on we go Shoot → Reload → Rechamber,
        // working the bolt only once the fresh mag is seated (that one motion
        // both ejects the old case and feeds the first round of the new mag).
        // With auto-reload off the sniper just holds on the fired pose until a
        // manual reload.
        let (rechamber, reload) = (rig.seg(SegAct::Rechamber), rig.seg(SegAct::Reload));
        let (remaining, on_finish) = if weapon.mag > 0 {
            (vec![shoot, rechamber], WeaponFinish::Nothing)
        } else if settings.auto_reload && weapon.reserve > 0 {
            (vec![shoot, reload, rechamber], WeaponFinish::Reload)
        } else {
            (vec![shoot], WeaponFinish::Nothing)
        };
        weapon.busy = Some(WeaponBusy {
            remaining,
            seg_end: shoot.end_secs(),
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
        let reload = rig.seg(SegAct::Reload);
        begin_segment(&mut rig, reload, &seg_ctx, &mut commands, &mut snd);
        // `mag == 0` only when the last round was fired and never rechambered
        // (there's no separate "round chambered" flag — an empty mag is the
        // one moment the chamber is guaranteed empty too), so the bolt still
        // needs working after the fresh mag goes in. Reloading with a round
        // already chambered (mag > 0, the TEMP full-mag case above) is a
        // tactical swap — the chamber's already loaded, so no bolt work.
        let mut remaining = vec![reload];
        if weapon.mag == 0 {
            remaining.push(rig.seg(SegAct::Rechamber));
        }
        weapon.busy = Some(WeaponBusy {
            remaining,
            seg_end: reload.end_secs(),
            on_finish: WeaponFinish::Reload,
        });
    }
}
