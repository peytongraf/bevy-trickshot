//! `Zombies` teammate bots: lobby members on the players' side (a fake
//! [`ally_peer`](shared::bot_players::ally_peer) each, added like a
//! `FreeForAll` bot — `lobby::add_bots`), up to a team of four. Each is a
//! player entity driven here through the same input path a real client
//! uses, so everything else treats it as a player: zombies go after it,
//! it scores, goes down, is revived and comes back next round like anyone.
//!
//! What it does, most pressing first ([`drive_allies`]):
//!
//! * **Down**, it crawls toward whoever's standing; **dead** (bled out), it
//!   waits for the next round, then comes back beside someone.
//! * **A teammate down** (and it's safe enough), it goes and revives them.
//! * **Zombies** in view, it shoots them — its reactions, aim and trigger
//!   finger as good as its difficulty ([`BotDifficulty::skill`],
//!   [`BotDifficulty::ally`]) — backing off one that gets close, knifing
//!   one on top of it, reloading and swapping guns as they run dry, and
//!   throwing its lethals and tacticals at a crowd.
//! * **Errands**, thought over every few seconds when nothing's on it
//!   ([`think`]): perks (the sensible ones first), the power, Pack-a-Punch
//!   once it's rich, the Mystery Box now and then (keeping the prize only
//!   if it's better), an AK-74 off the wall to start, ammo when low, boards
//!   back on a nearby window, and equipment lying about — only what it has
//!   none of, that's lain a while with no player nearer. Always keeping a
//!   few points back. Doors when a player's been waiting at one they can't
//!   afford, or once nobody's opened one for a while (its own patience,
//!   [`AllyBrain::door_patience`]) — never every door. A power-up dropped,
//!   it goes for — at once, or after giving the players a chance at it.
//!   Whether it does a thing first or holds off to let a player is up to
//!   chance each time ([`AllyBrain::ready`]).
//! * **Otherwise** it keeps up with its human teammates, each bot at its own
//!   spot round them ([`AllyBrain::slot_angle`]), moving about now and then.
//!
//! It never walks through anyone: it steers round them, and anyone it's
//! pressed into pushes it back out ([`spacing`]).
//!
//! Every purchase and pickup is the real one: it fires the same handler a
//! client's message does ([`RemoteTrigger`]) under its own peer, so the
//! rules — standing at it, the points, the power — and the quotes that go
//! with them are a player's. Everything a player's avatar shows, it shows
//! too ([`Act`]): the gun it's holding and carrying, putting one away and
//! getting another out, drinking a perk, throwing, reloading, aiming,
//! stabbing, lying down when it's down.
//!
//! Its state lives on its player entity, which goes with the game.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::*;

use shared::bot_players::{AllySkill, BotDifficulty, BotSkill};
use shared::bots::rand01;
use shared::lethal::LethalKind;
use shared::map::CollisionWorld;
use shared::perks::Perk;
use shared::weapon::{SlotWeapon, WeaponId};
use shared::{
    DrinkProgress, GameMode, Lobby, PlayerId, PlayerInput, PlayerPose, SwapProgress, ThrowProgress, ThrowStage,
};

use crate::ai::{
    ease, forward, look_angles, move_bot, shortest_angle, AIM_TOLERANCE_DEG, DETOUR_SECS, MANTLE_BASE_SECS,
    MANTLE_SECS_PER_M, MANTLE_START_DIST, REPATH_SECS, SPRINT_SPEED, STUCK_SECS, UNSTICK_PROGRESS, UNSTICK_SECS,
    VOID_Y, WALK_SPEED,
};
use crate::collision::MapColliders;
use crate::lobby::LobbyPlayer;
use crate::nav::{NavGraph, NavGraphs, Waypoint, BODY_RADIUS, CHEST_HEIGHT, STEP_HEIGHT};
use crate::pvp::PlayerCombat;
use crate::quotes::SayQuote;
use crate::sim::EYE_HEIGHT;

pub struct AllyPlugin;

impl Plugin for AllyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TeamDoors>()
            .add_systems(FixedUpdate, drive_allies.before(crate::sim::apply_client_pose));
    }
}

// --- how long things take (a player's, as their avatar shows them) ----------

/// Putting the weapon in hand away, and getting one out (s).
const STOW_SECS: f32 = 0.4;
const DRAW_SECS: f32 = 0.4;
/// A perk's drink (s) — the first-person one's.
const DRINK_SECS: f32 = 1.667;
/// A throw: the arm coming up, the throw, and the moment it's let go (s).
const RAISE_SECS: f32 = 0.25;
const THROW_SECS: f32 = 0.667;
const THROW_RELEASE_SECS: f32 = 0.25;
/// A knife stab, and when (s into it) it lands — `shared::melee`'s.
const STAB_SECS: f32 = 0.67;

// --- how it plays ----------------------------------------------------------------

/// Teammates nearer than this (m) are close enough; farther, it catches up.
const FOLLOW_NEAR: f32 = 3.0;
const FOLLOW_FAR: f32 = 7.0;
/// Farther than this (m) from the goal, it runs (if its difficulty does).
const RUN_FROM: f32 = 12.0;
/// A zombie this close (m) and in the way is knifed.
const STAB_REACH: f32 = 1.7;
/// How close (m) another body can be before it's steered round...
const AVOID_RADIUS: f32 = 1.4;
/// ...and how close before it's pushed back out of them.
const MIN_SPACING: f32 = 0.8;
/// The most it's pushed out at (m/s).
const PUSH_MAX_SPEED: f32 = 3.0;
/// The chance it goes for a power-up the moment it drops (else it gives the
/// players a few seconds' chance at it first).
const POWER_UP_AT_ONCE: f32 = 0.3;
/// The chance it does a thing it's worked out it wants at once (else it
/// holds off a while — [`AllyBrain::ready`]).
const ERRAND_AT_ONCE: f32 = 0.45;
/// Equipment lying this long (s) with no player nearer is fair game.
const DROP_WAIT_SECS: f32 = 6.0;
/// An errand farther than this (m, walking) isn't worth the trip.
const ERRAND_MAX_DIST: f32 = 70.0;
/// Rounds (at least) between two Mystery Box spins by one bot.
const BOX_EVERY_ROUNDS: u32 = 2;
/// A crowd worth a frag / molotov: this many zombies within...
const CROWD: usize = 4;
/// ...this far (m) of one of them.
const CROWD_RADIUS: f32 = 3.5;
/// How hard (s) an errand's tried before it's given up on.
const ERRAND_GIVE_UP_SECS: f32 = 25.0;

/// A gun's numbers, as the bot fires it: what it holds, how long a reload
/// takes and how soon the next round can go (s) — a `Zombies` player's
/// (the client's `weapon::mag_size` / `starting_reserve`, and its reload
/// clips' lengths).
struct GunStats {
    mag: u32,
    reserve: u32,
    reload_secs: f32,
    interval: f32,
}

fn gun_stats(gun: WeaponId) -> GunStats {
    match gun {
        // (A reload from empty — the long one.)
        WeaponId::Ak74 => GunStats { mag: 30, reserve: 30 * 5, reload_secs: 2.83, interval: 0.1 },
        WeaponId::RayGun => GunStats { mag: 20, reserve: 20 * 8, reload_secs: 2.5, interval: 0.33 },
        // Semi-auto: as fast as a player taps it.
        WeaponId::M17 => GunStats { mag: 17, reserve: 17 * 6, reload_secs: 3.33, interval: 0.2 },
        // A bolt-action: the shot, then the rechamber.
        WeaponId::Sniper | WeaponId::Marksman => GunStats { mag: 5, reserve: 5 * 7, reload_secs: 1.83, interval: 1.2 },
    }
}

/// How much better a gun is to have — for whether a new one's worth taking.
fn gun_rank(weapon: SlotWeapon) -> u32 {
    match weapon {
        SlotWeapon::Knife => 0,
        SlotWeapon::Gun(WeaponId::M17) => 1,
        SlotWeapon::Gun(WeaponId::Sniper | WeaponId::Marksman) => 2,
        SlotWeapon::Gun(WeaponId::Ak74) => 3,
        SlotWeapon::Gun(WeaponId::RayGun) => 4,
    }
}

/// The order it buys perks in — the ones that keep a team alive first.
fn perk_priority(perk: Perk) -> u32 {
    match perk {
        Perk::Juggernog => 0,
        Perk::QuickRevive => 1,
        Perk::SpeedCola => 2,
        Perk::DoubleTap => 3,
        Perk::StaminUp => 4,
        Perk::DeadshotDaiquiri => 5,
        _ => 6,
    }
}

// --- the brain ------------------------------------------------------------------

/// Rounds in a gun ([`AllyBrain::ammo`]).
#[derive(Clone, Copy, Debug, Default)]
struct Ammo {
    mag: u32,
    reserve: u32,
}

impl Ammo {
    fn full(weapon: SlotWeapon) -> Self {
        match weapon.gun() {
            Some(gun) => {
                let s = gun_stats(gun);
                Ammo { mag: s.mag, reserve: s.reserve }
            }
            None => Ammo::default(),
        }
    }
}

/// Something it's doing with its hands — shown as a player's would be
/// ([`act_look`]).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Ready,
    /// The weapon in hand away, slot `to`'s out.
    Swap { to: usize, since: f32 },
    /// The weapon away, `perk` drunk, the weapon back.
    Drink { perk: Perk, since: f32 },
    /// The weapon away, `kind` thrown at `at`, the weapon back.
    Throw { kind: LethalKind, at: Vec3, since: f32, sent: bool },
}

/// Somewhere it's going to do something ([`think`]).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Errand {
    Perk(Perk),
    Power,
    Pap,
    Door(u8),
    Box,
    WallBuy(WeaponId),
    Ammo(Vec3),
    Repair(u8),
    PickUp(Entity, LethalKind, Vec3),
    /// A power-up lying about — walked onto, which grabs it.
    PowerUp(Entity),
}

/// An errand under way: what, where it's walking to, when it started, and
/// when (if) it last asked for it.
#[derive(Clone, Copy, Debug)]
struct Doing {
    errand: Errand,
    spot: Vec3,
    since: f32,
    asked_at: Option<f32>,
    /// Points before it asked — paid means it went through.
    points_before: u32,
}

/// A climb up a ledge on the route.
#[derive(Clone, Copy, Debug)]
struct Climb {
    start: f32,
    secs: f32,
    from: Vec3,
    to: Vec3,
}

/// A `Zombies` teammate bot's mind and body (see the module docs).
#[derive(Component)]
pub struct AllyBrain {
    difficulty: BotDifficulty,
    rng: u64,
    feet: Vec3,
    vertical_velocity: f32,
    yaw: f32,
    pitch: f32,
    climb: Option<Climb>,
    path: Vec<Waypoint>,
    path_index: usize,
    path_goal: Vec3,
    repath_at: f32,
    blocked_for: f32,
    detour_until: f32,
    detour_dir: Vec3,
    stall_from: Vec3,
    stall_secs: f32,
    stride: f32,
    // Fighting.
    target: Option<PeerId>,
    seen_for: f32,
    next_fire_at: f32,
    ads: f32,
    reload_until: Option<f32>,
    stab_since: Option<f32>,
    stab_sent: bool,
    // What it carries.
    ammo: [Ammo; 2],
    weapons: [SlotWeapon; 2],
    held: usize,
    lethal: Option<(LethalKind, u32)>,
    tactical: Option<(LethalKind, u32)>,
    act: Act,
    // Deciding.
    doing: Option<Doing>,
    think_at: f32,
    box_round: Option<u32>,
    /// Errands that didn't work, and until when not to try them again.
    gave_up: Vec<(Errand, f32)>,
    /// How long (s) a player's been standing at each shut door.
    waited_at_door: HashMap<u8, f32>,
    low_ammo_said: bool,
    /// Where round the player it's keeping up with it stands — this way
    /// (radians) and this far (m), picked afresh at `slot_until`.
    slot_angle: f32,
    slot_dist: f32,
    slot_until: f32,
    /// How long (s) it'll wait for anyone to open a door before it does...
    door_patience: f32,
    /// ...since the doors last changed (`Lobby::doors_open`, and when — `<
    /// 0` before it's looked).
    doors_seen: u32,
    doors_changed_at: f32,
    /// How long (s) a player stands at a shut door they can't afford before
    /// it pays for it.
    door_wait: f32,
    /// Errands it's wanted, and from when it'll do them ([`Self::ready`]).
    hesitate: Vec<(Errand, f32)>,
    /// Power-ups it's seen dropped, and from when it'll go for each.
    power_ups: Vec<(Entity, f32)>,
    /// How long (s) the current target has to be in view before it fires.
    reaction: f32,
    // Up / down / out.
    was_alive: bool,
    perks_seen: usize,
}

impl AllyBrain {
    pub fn new(difficulty: BotDifficulty, feet: Vec3, yaw: f32, seed: u64) -> Self {
        Self {
            difficulty,
            rng: seed,
            feet,
            vertical_velocity: 0.0,
            yaw,
            pitch: 0.0,
            climb: None,
            path: Vec::new(),
            path_index: 0,
            path_goal: feet,
            repath_at: 0.0,
            blocked_for: 0.0,
            detour_until: 0.0,
            detour_dir: Vec3::ZERO,
            stall_from: feet,
            stall_secs: 0.0,
            stride: 0.0,
            target: None,
            seen_for: 0.0,
            next_fire_at: 0.0,
            ads: 0.0,
            reload_until: None,
            stab_since: None,
            stab_sent: false,
            ammo: [Ammo::default(); 2],
            weapons: [SlotWeapon::Knife; 2],
            held: 0,
            lethal: None,
            tactical: None,
            act: Act::Ready,
            doing: None,
            think_at: 2.0,
            box_round: None,
            gave_up: Vec::new(),
            waited_at_door: HashMap::new(),
            low_ammo_said: false,
            slot_angle: rand01(seed ^ 0x51) * core::f32::consts::TAU,
            slot_dist: 2.0 + rand01(seed ^ 0x52) * 3.0,
            slot_until: 0.0,
            door_patience: 45.0 + rand01(seed ^ 0x53) * 105.0,
            doors_seen: 0,
            doors_changed_at: -1.0,
            door_wait: 3.0 + rand01(seed ^ 0x54) * 6.0,
            hesitate: Vec::new(),
            power_ups: Vec::new(),
            reaction: difficulty.skill().reaction_secs,
            was_alive: true,
            perks_seen: 0,
        }
    }

    /// Whether to do `errand` now: the first time it's wanted, it's decided
    /// whether to go at once ([`ERRAND_AT_ONCE`]) or hold off a while —
    /// leaving a player the chance — and it's done from then on.
    fn ready(&mut self, errand: Errand, now: f32) -> bool {
        if let Some(&(_, at)) = self.hesitate.iter().find(|(e, _)| *e == errand) {
            return now >= at;
        }
        let at = if self.roll() < ERRAND_AT_ONCE { now } else { now + 8.0 + self.roll() * 35.0 };
        self.hesitate.push((errand, at));
        now >= at
    }

    fn roll(&mut self) -> f32 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        rand01(self.rng)
    }

    fn skill(&self) -> BotSkill {
        self.difficulty.skill()
    }

    fn ally(&self) -> AllySkill {
        self.difficulty.ally()
    }

    fn eye(&self) -> Vec3 {
        self.feet + Vec3::Y * EYE_HEIGHT
    }

    fn held_weapon(&self) -> SlotWeapon {
        self.weapons[self.held]
    }

    /// Keep what it carries in step with its lobby's say (a box prize, a
    /// wall buy or a respawn changed it): a new gun comes full; it holds a
    /// gun over the knife.
    fn sync_weapons(&mut self, weapons: [SlotWeapon; 2]) {
        if weapons == self.weapons {
            return;
        }
        for i in 0..2 {
            if weapons[i] != self.weapons[i] {
                self.ammo[i] = Ammo::full(weapons[i]);
            }
        }
        self.weapons = weapons;
        if self.weapons[self.held].gun().is_none() {
            if let Some(i) = (0..2).find(|&i| self.weapons[i].gun().is_some()) {
                self.held = i;
            }
        }
    }

    fn gave_up_on(&self, errand: Errand, now: f32) -> bool {
        self.gave_up.iter().any(|(e, until)| *e == errand && now < *until)
    }

    fn give_up(&mut self, now: f32) {
        if let Some(doing) = self.doing.take() {
            self.gave_up.push((doing.errand, now + 20.0));
        }
        self.gave_up.retain(|(_, until)| *until > now);
    }
}

/// When (`Time::elapsed_secs`) the team's bots last went to open a door, by
/// lobby — so two don't open doors at once.
#[derive(Resource, Default)]
struct TeamDoors(HashMap<Entity, f32>);

/// One other player (or zombie), as this tick's sees them.
#[derive(Clone, Copy)]
struct Body {
    peer: PeerId,
    lobby: Entity,
    eye: Vec3,
    feet: Vec3,
    alive: bool,
    down: bool,
    zombie: bool,
    bot: bool,
}

/// A piece of equipment lying about.
#[derive(Clone, Copy)]
struct Lying {
    entity: Entity,
    lobby: Entity,
    kind: LethalKind,
    pos: Vec3,
    age: f32,
}

/// What its hands look like during `act`, `t` seconds in: the weapon held
/// (by slot), and the swap / throw / drink their avatar plays — and whether
/// it's done.
struct ActLook {
    held: Option<usize>,
    swap: Option<SwapProgress>,
    throw: Option<ThrowProgress>,
    drink: Option<DrinkProgress>,
    done: bool,
}

fn act_look(act: Act, held: usize, now: f32) -> ActLook {
    let stowing = |t: f32| Some(SwapProgress::Stowing((t / STOW_SECS).clamp(0.0, 1.0)));
    let drawing = |t: f32| Some(SwapProgress::Drawing((t / DRAW_SECS).clamp(0.0, 1.0)));
    let mut look = ActLook { held: Some(held), swap: None, throw: None, drink: None, done: false };
    match act {
        Act::Ready => {}
        Act::Swap { to, since } => {
            let t = now - since;
            if t < STOW_SECS {
                look.swap = stowing(t);
            } else if t < STOW_SECS + DRAW_SECS {
                look.held = Some(to);
                look.swap = drawing(t - STOW_SECS);
            } else {
                look.held = Some(to);
                look.done = true;
            }
        }
        Act::Drink { perk, since } => {
            let t = now - since;
            if t < STOW_SECS {
                look.swap = stowing(t);
            } else if t < STOW_SECS + DRINK_SECS {
                look.held = None;
                look.drink = Some(DrinkProgress { perk, t: (t - STOW_SECS) / DRINK_SECS });
            } else if t < STOW_SECS + DRINK_SECS + DRAW_SECS {
                look.swap = drawing(t - STOW_SECS - DRINK_SECS);
            } else {
                look.done = true;
            }
        }
        Act::Throw { kind, since, .. } => {
            let t = now - since;
            let (raise, throw) = (STOW_SECS + RAISE_SECS, STOW_SECS + RAISE_SECS + THROW_SECS);
            if t < STOW_SECS {
                look.swap = stowing(t);
            } else if t < raise {
                look.held = None;
                look.throw = Some(ThrowProgress { kind, stage: ThrowStage::Raising((t - STOW_SECS) / RAISE_SECS) });
            } else if t < throw {
                look.held = None;
                let into = t - raise;
                look.throw = Some(ThrowProgress {
                    kind,
                    stage: ThrowStage::Throwing { t: into / THROW_SECS, released: into >= THROW_RELEASE_SECS },
                });
            } else if t < throw + DRAW_SECS {
                look.swap = drawing(t - throw);
            } else {
                look.done = true;
            }
        }
    }
    look
}

/// Drive every `Zombies` teammate bot one tick (see the module docs).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive_allies(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    navs: Res<NavGraphs>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<(Entity, &Lobby)>,
    everyone: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &PlayerCombat)>,
    mut allies: Query<(&PlayerId, &LobbyPlayer, &PlayerCombat, &mut AllyBrain, &mut ActionState<PlayerInput>)>,
    drops: (
        Query<(Entity, &shared::FragDrop, &crate::frags::DropSim)>,
        Query<(Entity, &shared::MolotovDrop, &crate::molotovs::DropSim)>,
        Query<(Entity, &shared::MonkeyDrop, &crate::monkey_bombs::DropSim)>,
        Query<(Entity, &shared::FlashBangDrop, &crate::flash_bangs::DropSim)>,
        Query<(Entity, &crate::knives::KnifeSim)>,
        Query<(Entity, &shared::PowerUpDrop, &crate::power_ups::DropSim)>,
    ),
    mut team_doors: ResMut<TeamDoors>,
    mut quotes: EventWriter<SayQuote>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    if dt <= 0.0 {
        return;
    }
    let bodies: Vec<Body> = everyone
        .iter()
        .map(|(id, pose, lp, c)| Body {
            peer: id.0,
            lobby: lp.lobby,
            eye: pose.translation,
            feet: pose.translation - Vec3::Y * EYE_HEIGHT,
            alive: c.alive,
            down: c.down.is_some(),
            zombie: pose.zombie.is_zombie(),
            bot: shared::bot_players::is_ally_peer(id.0),
        })
        .collect();
    let mut lying: Vec<Lying> = Vec::new();
    let (frags, molotovs, monkeys, flashes, knives, power_ups) = &drops;
    // Power-ups lying about: where, by lobby, and how long they've been there.
    let power_ups: Vec<(Entity, Entity, Vec3, f32)> = power_ups.iter().map(|(e, d, s)| (e, s.lobby, d.pos, s.age)).collect();
    lying.extend(frags.iter().map(|(e, d, s)| Lying { entity: e, lobby: s.lobby, kind: LethalKind::Frag, pos: d.pos, age: s.age }));
    lying.extend(molotovs.iter().map(|(e, d, s)| Lying { entity: e, lobby: s.lobby, kind: LethalKind::Molotov, pos: d.pos, age: s.age }));
    lying.extend(monkeys.iter().map(|(e, d, s)| Lying { entity: e, lobby: s.lobby, kind: LethalKind::MonkeyBomb, pos: d.pos, age: s.age }));
    lying.extend(flashes.iter().map(|(e, d, s)| Lying { entity: e, lobby: s.lobby, kind: LethalKind::FlashBang, pos: d.pos, age: s.age }));
    lying.extend(knives.iter().filter(|(_, k)| k.body.resting).map(|(e, k)| Lying {
        entity: e,
        lobby: k.lobby,
        kind: LethalKind::ThrowingKnife,
        pos: k.body.pos,
        age: k.body.age,
    }));

    for (id, lp, combat, mut brain_ref, mut action) in &mut allies {
        let brain = &mut *brain_ref;
        let me = id.0;
        let Ok((lobby_e, lobby)) = lobbies.get(lp.lobby) else { continue };
        let still = |brain: &AllyBrain| PlayerInput {
            translation: brain.eye().to_array(),
            yaw: brain.yaw,
            pitch: brain.pitch,
            held: brain.weapons[brain.held].gun().map(SlotWeapon::Gun),
            weapon: brain.weapons[brain.held].gun().unwrap_or_default().as_u8(),
            ..default()
        };
        if !lobby.started || lobby.mode != GameMode::Zombies || lobby.paused || endings.is_frozen(lobby_e) {
            action.0 = still(brain);
            continue;
        }
        let Some(member) = lobby.members.iter().find(|m| m.peer == me) else { continue };
        let world = &colliders.for_lobby(lobby);
        let nav = navs.graph(lobby.map);
        let closed = shared::doors::closed_mask(lobby.map, lobby.doors_open);
        brain.sync_weapons(member.weapons);

        // Down (which isn't "alive" either — `PlayerCombat::go_down`):
        // no weapon, lying there, crawling toward whoever's standing.
        if combat.down.is_some() {
            brain.doing = None;
            brain.act = Act::Ready;
            brain.reload_until = None;
            let to = bodies
                .iter()
                .filter(|b| b.lobby == lobby_e && b.peer != me && !b.zombie && b.alive && !b.down)
                .min_by(|a, b| a.feet.distance_squared(brain.feet).total_cmp(&b.feet.distance_squared(brain.feet)))
                .map(|b| b.feet);
            let mut wish = Vec3::ZERO;
            if let Some(to) = to.filter(|t| t.distance(brain.feet) > 1.2) {
                wish = walk(brain, world, nav, to, closed, now, dt);
                // (No climbing ledges lying down.)
                brain.climb = None;
                brain.yaw = look_angles(wish.with_y(0.0).normalize_or(Vec3::NEG_Z)).0;
            }
            move_bot(world, &mut brain.feet, &mut brain.vertical_velocity, wish, shared::revive::DOWNED_CRAWL_SPEED, dt);
            brain.pitch = 0.0;
            action.0 = PlayerInput {
                translation: brain.eye().to_array(),
                yaw: brain.yaw,
                pitch: 0.0,
                prone: true,
                crouch_drop: -1.3,
                held: None,
                ..default()
            };
            continue;
        }

        // Out (bled out): wait for the round that brings it back — beside
        // someone still standing (the message a player's told where to
        // stand by goes nowhere).
        if !combat.alive {
            brain.was_alive = false;
            brain.doing = None;
            brain.act = Act::Ready;
            action.0 = still(brain);
            continue;
        }
        if !brain.was_alive {
            brain.was_alive = true;
            let near = bodies
                .iter()
                .find(|b| b.lobby == lobby_e && b.peer != me && !b.zombie && b.alive && !b.down)
                .map(|b| b.feet);
            if let Some(spot) = near.and_then(|n| nav.random_spot_near(n, 1.5, 5.0, now.to_bits() as u64 ^ me.to_bits())) {
                brain.feet = spot;
            }
            brain.vertical_velocity = 0.0;
            brain.path.clear();
            brain.climb = None;
            brain.ammo = [Ammo::full(brain.weapons[0]), Ammo::full(brain.weapons[1])];
        }

        let skill = brain.skill();
        let ally = brain.ally();
        let eye = brain.eye();
        let team = |b: &&Body| b.lobby == lobby_e && b.peer != me && !b.zombie && b.alive;
        let zombies: Vec<&Body> = bodies.iter().filter(|b| b.lobby == lobby_e && b.zombie && b.alive).collect();
        let nearest_zombie = zombies
            .iter()
            .map(|z| (z, z.feet.distance(brain.feet)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let pressed = nearest_zombie.is_some_and(|(_, d)| d < 6.0);

        // A perk bought (its drink) — whoever's errand it was.
        if member.perks.len() > brain.perks_seen {
            if let Some(&perk) = member.perks.last() {
                if brain.act == Act::Ready {
                    brain.act = Act::Drink { perk, since: now };
                }
            }
        }
        brain.perks_seen = member.perks.len();

        // --- who to shoot ----------------------------------------------------
        // (The better ones go for the head.)
        let heads = matches!(brain.difficulty, BotDifficulty::Hardened | BotDifficulty::Veteran);
        let aim_at = |z: &Body| {
            if heads {
                z.eye - Vec3::Y * 0.1
            } else {
                z.eye - Vec3::Y * 0.45
            }
        };
        let visible = |z: &Body| {
            let to = aim_at(z) - eye;
            to.length() <= skill.sight_range && !world.segment_blocked(eye, aim_at(z))
        };
        let target = zombies
            .iter()
            .filter(|z| visible(z))
            .min_by(|a, b| a.feet.distance_squared(brain.feet).total_cmp(&b.feet.distance_squared(brain.feet)))
            .copied();
        if target.map(|t| t.peer) != brain.target {
            brain.target = target.map(|t| t.peer);
            brain.seen_for = 0.0;
            // (Quicker to some than others.)
            brain.reaction = skill.reaction_secs * (0.7 + brain.roll() * 0.6);
        } else if target.is_some() {
            brain.seen_for += dt;
        }
        let engaged = target.is_some() && brain.seen_for >= brain.reaction;

        // The doors last changed (anyone opening one) — its patience with them
        // starts over.
        if brain.doors_changed_at < 0.0 || brain.doors_seen != lobby.doors_open {
            brain.doors_seen = lobby.doors_open;
            brain.doors_changed_at = now;
            brain.door_patience = 45.0 + brain.roll() * 105.0;
        }
        // Power-ups: each one new, whether to go at once or give the players
        // a moment first.
        brain.power_ups.retain(|(e, _)| power_ups.iter().any(|(p, ..)| p == e));
        for &(e, lobby_of, ..) in &power_ups {
            if lobby_of == lobby_e && !brain.power_ups.iter().any(|(p, _)| *p == e) {
                let at = if brain.roll() < POWER_UP_AT_ONCE { now } else { now + 4.0 + brain.roll() * 14.0 };
                brain.power_ups.push((e, at));
            }
        }
        if let Some(Doing { errand: Errand::PowerUp(e), .. }) = brain.doing {
            if !power_ups.iter().any(|(p, ..)| *p == e) {
                brain.doing = None; // (Taken — by it, or someone.)
            }
        }

        // --- what to do ------------------------------------------------------
        // A teammate down, and nothing right on top of it: go and get them up.
        // (A downed player isn't "alive" — `PlayerCombat::go_down`.)
        let downed = bodies
            .iter()
            .filter(|b| b.lobby == lobby_e && b.peer != me && !b.zombie && b.down)
            .min_by(|a, b| a.feet.distance_squared(brain.feet).total_cmp(&b.feet.distance_squared(brain.feet)))
            .copied();
        let reviving = downed.filter(|d| {
            let crowded = zombies.iter().filter(|z| z.feet.distance(d.feet) < 2.5).count();
            crowded == 0 || brain.difficulty == BotDifficulty::Veteran
        });

        // A power-up it's due to go for (or that's running out) within reach.
        if brain.doing.is_none() && reviving.is_none() {
            let due = power_ups
                .iter()
                .filter(|(e, lobby_of, pos, age)| {
                    *lobby_of == lobby_e
                        && pos.distance(brain.feet) < 30.0
                        && (brain.power_ups.iter().any(|(p, at)| p == e && now >= *at)
                            || *age > shared::power_ups::DROP_LIFETIME_SECS - 9.0)
                })
                .min_by(|a, b| a.2.distance_squared(brain.feet).total_cmp(&b.2.distance_squared(brain.feet)));
            if let Some(&(e, _, pos, _)) = due {
                brain.doing = Some(Doing { errand: Errand::PowerUp(e), spot: pos, since: now, asked_at: None, points_before: member.score });
                brain.repath_at = now;
            }
        }

        // Think over what to spend on, every so often, when nothing's on it.
        if brain.doing.is_none() && reviving.is_none() && !pressed && now >= brain.think_at && brain.act == Act::Ready {
            brain.think_at = now + ally.think_secs * (0.8 + brain.roll() * 0.4);
            let view = View { lobby_e, lobby, me, member, bodies: &bodies, lying: &lying, team_doors: &team_doors };
            if let Some((errand, spot)) = think(brain, &view, world, nav, closed, now) {
                brain.doing = Some(Doing { errand, spot, since: now, asked_at: None, points_before: member.score });
                brain.path.clear();
                brain.repath_at = now;
            }
        }
        // (Players waiting at shut doors, for the next think.)
        for (i, door) in shared::doors::doors(lobby.map).iter().enumerate() {
            if shared::doors::is_open(lobby.doors_open, i) {
                brain.waited_at_door.remove(&(i as u8));
                continue;
            }
            let waiting = bodies.iter().any(|b| team(&b) && !b.bot && door.in_range(b.feet, 1.5));
            let t = brain.waited_at_door.entry(i as u8).or_default();
            *t = if waiting { *t + dt } else { 0.0 };
        }

        // An errand under way: there yet? Then ask — and see whether it went.
        let mut goal: Option<Vec3> = None;
        let mut reviving_now = false;
        if let Some(d) = reviving {
            if d.feet.distance(brain.feet) <= shared::revive::REVIVE_RANGE * 0.7 {
                reviving_now = true;
            } else {
                goal = Some(d.feet);
            }
        } else if let Some(mut doing) = brain.doing {
            if now - doing.since > ERRAND_GIVE_UP_SECS {
                brain.give_up(now);
            } else if let Some(asked) = doing.asked_at {
                // (A moment for the server to have had its say.)
                if now - asked > 0.4 {
                    let view = View { lobby_e, lobby, me, member, bodies: &bodies, lying: &lying, team_doors: &team_doors };
                    match errand_result(brain, &view, doing, now, &mut commands) {
                        Some(true) => {
                            brain.doing = None;
                            // (Wanted again — another level, say — it's
                            // decided afresh.)
                            brain.hesitate.retain(|(e, _)| *e != doing.errand);
                        }
                        Some(false) => brain.give_up(now),
                        None => {}
                    }
                }
            } else if at_errand(doing.errand, lobby, brain.feet, &lying) {
                ask(doing.errand, brain, member, me, &mut commands);
                doing.asked_at = Some(now);
                doing.points_before = member.score;
                brain.doing = Some(doing);
                if let Errand::Door(_) = doing.errand {
                    team_doors.0.insert(lobby_e, now);
                }
            } else {
                goal = Some(doing.spot);
            }
            if pressed && brain.doing.is_some_and(|d| d.asked_at.is_none()) {
                // (Zombies on it: fight first, the errand can wait.)
                goal = None;
            }
        }
        if goal.is_none() && !reviving_now && brain.doing.is_none() {
            // Keep up with the team (its players first).
            let lead = bodies
                .iter()
                .filter(|b| team(b) && !b.down)
                .min_by(|a, b| {
                    (a.bot, a.feet.distance_squared(brain.feet)).partial_cmp(&(b.bot, b.feet.distance_squared(brain.feet))).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();
            if let Some(lead) = lead {
                // Its own spot round them, moved about now and then.
                if now >= brain.slot_until {
                    brain.slot_angle = brain.roll() * core::f32::consts::TAU;
                    brain.slot_dist = FOLLOW_NEAR * 0.7 + brain.roll() * (FOLLOW_FAR - FOLLOW_NEAR);
                    brain.slot_until = now + 6.0 + brain.roll() * 8.0;
                }
                let around = lead.feet + Vec3::new(brain.slot_angle.cos(), 0.0, brain.slot_angle.sin()) * brain.slot_dist;
                let spot = nav
                    .nearest_spot(around)
                    .filter(|s| (s.y - lead.feet.y).abs() < 1.5 && nav.connected(*s, lead.feet))
                    .unwrap_or(lead.feet);
                let d = lead.feet.distance(brain.feet);
                if d > FOLLOW_FAR || (spot.distance(brain.feet) > 1.2 && !engaged) {
                    goal = Some(spot);
                }
            }
        }

        // --- hands: reload, swap to a gun with rounds, throw at a crowd -------
        let held_gun = brain.held_weapon().gun();
        if let Some(until) = brain.reload_until {
            if now >= until {
                let a = &mut brain.ammo[brain.held];
                let stats = held_gun.map(gun_stats);
                if let Some(s) = stats {
                    let take = (s.mag - a.mag).min(a.reserve);
                    a.mag += take;
                    a.reserve -= take;
                }
                brain.reload_until = None;
            }
        }
        let mut sound_bits = 0u32;
        if brain.act == Act::Ready && brain.reload_until.is_none() {
            let a = brain.ammo[brain.held];
            let other = 1 - brain.held;
            let other_rounds = brain.weapons[other].gun().map(|_| brain.ammo[other].mag + brain.ammo[other].reserve);
            if a.mag == 0 && a.reserve == 0 && other_rounds.is_some_and(|r| r > 0) {
                brain.act = Act::Swap { to: other, since: now };
            } else if a.mag == 0 && a.reserve > 0 && held_gun.is_some() {
                brain.reload_until = Some(now + held_gun.map_or(2.5, |g| gun_stats(g).reload_secs));
                sound_bits |= SND_RELOAD;
            } else if !engaged {
                // Topped up between fights, like anyone.
                let stats = held_gun.map(gun_stats);
                if stats.is_some_and(|s| a.mag < s.mag / 2 && a.reserve > 0) && !pressed {
                    brain.reload_until = Some(now + held_gun.map_or(2.5, |g| gun_stats(g).reload_secs));
                    sound_bits |= SND_RELOAD;
                }
            }
            // Low / out of rounds altogether: says so.
            let total: u32 = (0..2).filter(|&i| brain.weapons[i].gun().is_some()).map(|i| brain.ammo[i].mag + brain.ammo[i].reserve).sum();
            let low = held_gun.map_or(0, |g| gun_stats(g).mag);
            if total <= low && !brain.low_ammo_said {
                brain.low_ammo_said = true;
                let quote = if total == 0 { shared::quotes::Quote::OutOfAmmo } else { shared::quotes::Quote::LowAmmo };
                quotes.write(SayQuote::by(lobby_e, me, quote));
            } else if total > low {
                brain.low_ammo_said = false;
            }
            // A crowd: a frag or molotov into it; a monkey bomb or flash bang
            // when it's on top of us.
            if ally.throws && brain.stab_since.is_none() {
                if let Some((kind, at)) = pick_throw(brain, &zombies, eye, world) {
                    brain.act = Act::Throw { kind, at, since: now, sent: false };
                }
            }
        }
        let look = act_look(brain.act, brain.held, now);
        if let Act::Throw { kind, at, since, sent: false } = brain.act {
            if now - since >= STOW_SECS + RAISE_SECS + THROW_RELEASE_SECS {
                throw_it(brain, me, kind, at, &mut commands);
                brain.act = Act::Throw { kind, at, since, sent: true };
            }
        }
        if look.done {
            if let Act::Swap { to, .. } = brain.act {
                brain.held = to;
            }
            brain.act = Act::Ready;
        }
        let hands_free = brain.act == Act::Ready && brain.reload_until.is_none();

        // --- moving --------------------------------------------------------------
        let mut wish = Vec3::ZERO;
        let mut speed = WALK_SPEED;
        let jumping;
        if let Some(run) = brain.climb {
            let t = ((now - run.start) / run.secs.max(1e-3)).clamp(0.0, 1.0);
            let rise = ease((t / 0.6).clamp(0.0, 1.0));
            let advance = ease(((t - 0.4) / 0.6).clamp(0.0, 1.0));
            brain.feet = Vec3::new(
                run.from.x.lerp(run.to.x, advance),
                run.from.y.lerp(run.to.y, rise),
                run.from.z.lerp(run.to.z, advance),
            );
            jumping = true;
            if t >= 1.0 {
                brain.climb = None;
                brain.vertical_velocity = 0.0;
                brain.stall_from = brain.feet;
            }
        } else {
            // Backing off a zombie that's close; knifing one right on it.
            let too_close = nearest_zombie.filter(|(_, d)| *d < ally.keep_away);
            if let Some((z, d)) = too_close {
                let away = (brain.feet - z.feet).with_y(0.0).normalize_or(Vec3::Z);
                wish = away;
                if d < STAB_REACH && brain.stab_since.is_none() && brain.act == Act::Ready {
                    let dry = brain.ammo[brain.held].mag == 0 || brain.reload_until.is_some();
                    if dry || brain.roll() < 0.02 {
                        brain.stab_since = Some(now);
                        brain.stab_sent = false;
                    }
                }
            } else if let Some(g) = goal {
                wish = walk(brain, world, nav, g, closed, now, dt);
                if g.distance(brain.feet) > RUN_FROM && skill.sprints && !engaged {
                    speed = SPRINT_SPEED;
                }
            }
            if brain.detour_until > now {
                wish = brain.detour_dir;
            }
            // (Slower while shooting, as a player aiming is.)
            if engaged && hands_free {
                speed = speed.min(WALK_SPEED * 0.5);
            }
            // Round anyone in the way, and out of anyone it's pressed into.
            let (steer, push) = spacing(brain.feet, me, lobby_e, &bodies, dt);
            if wish != Vec3::ZERO {
                wish = (wish + steer).with_y(0.0).normalize_or(wish);
            }
            let velocity = wish * speed + push;
            let move_speed = velocity.length();
            let move_dir = if move_speed > 1e-4 { velocity / move_speed } else { Vec3::ZERO };
            let moved = move_bot(world, &mut brain.feet, &mut brain.vertical_velocity, move_dir, move_speed, dt);
            jumping = !moved.grounded;
            unstick(brain, nav, wish, moved.blocked, now, dt);
            if moved.grounded && wish != Vec3::ZERO {
                brain.stride += speed * dt;
                if brain.stride >= 2.2 {
                    brain.stride -= 2.2;
                    sound_bits |= SND_FOOTSTEP;
                }
            }
        }
        if brain.feet.y < VOID_Y {
            if let Some(spot) = bodies.iter().find(|b| team(b)).and_then(|b| nav.nearest_spot(b.feet)) {
                brain.feet = spot;
            }
            brain.vertical_velocity = 0.0;
        }

        // --- aiming and firing ------------------------------------------------
        let eye = brain.eye();
        let (want_yaw, want_pitch) = match target.filter(|_| engaged) {
            Some(t) => look_angles((aim_at(t) - eye).normalize_or(Vec3::NEG_Z)),
            None if wish != Vec3::ZERO => (look_angles(wish).0, 0.0),
            None => match downed.filter(|_| reviving_now) {
                Some(d) => (look_angles((d.feet - brain.feet).with_y(0.0).normalize_or(Vec3::NEG_Z)).0, -0.5),
                None => (brain.yaw, 0.0),
            },
        };
        let max_turn = skill.turn_deg_per_sec.to_radians() * dt;
        brain.yaw += shortest_angle(brain.yaw, want_yaw).clamp(-max_turn, max_turn);
        brain.yaw = shortest_angle(0.0, brain.yaw);
        brain.pitch += (want_pitch - brain.pitch).clamp(-max_turn, max_turn);
        let aim_error = shortest_angle(brain.yaw, want_yaw).abs().max((brain.pitch - want_pitch).abs());

        let mut fire = false;
        let mut fire_dir = forward(brain.yaw, brain.pitch);
        if let Some(gun) = held_gun.filter(|_| engaged && hands_free && brain.stab_since.is_none()) {
            if now >= brain.next_fire_at
                && brain.ammo[brain.held].mag > 0
                && aim_error <= (AIM_TOLERANCE_DEG * 2.0).to_radians()
                && !lobby.bots_hold_fire()
            {
                let err = skill.aim_error_deg.to_radians() * 0.6;
                fire_dir = forward(brain.yaw + (brain.roll() * 2.0 - 1.0) * err, brain.pitch + (brain.roll() * 2.0 - 1.0) * err);
                fire = true;
                brain.ammo[brain.held].mag -= 1;
                brain.next_fire_at = now + gun_stats(gun).interval / ally.trigger_speed.max(0.1) * (0.9 + brain.roll() * 0.2);
                sound_bits |= SND_SHOT;
            }
        }
        // Scoped in while it's fighting (a player's 0.4 s).
        let ads_goal = if engaged && hands_free { 1.0 } else { 0.0 };
        let step = dt / 0.4;
        brain.ads = if ads_goal > brain.ads { (brain.ads + step).min(1.0) } else { (brain.ads - step).max(0.0) };

        // A knife stab: shown at once, landing as the swing gets there.
        let mut melee = false;
        let mut stabbing = false;
        if let Some(since) = brain.stab_since {
            stabbing = now - since < STAB_SECS;
            if !brain.stab_sent && now - since >= shared::melee::KNIFE_HIT_SECS {
                brain.stab_sent = true;
                melee = true;
                if let Some((z, _)) = nearest_zombie {
                    fire_dir = (z.eye - Vec3::Y * 0.5 - eye).normalize_or(fire_dir);
                }
            }
            if !stabbing {
                brain.stab_since = None;
            }
        }

        let held = look.held.map(|slot| brain.weapons[slot]).filter(|w| w.gun().is_some());
        action.0 = PlayerInput {
            translation: eye.to_array(),
            yaw: brain.yaw,
            pitch: brain.pitch,
            fire,
            fire_origin: eye.to_array(),
            fire_dir: fire_dir.to_array(),
            melee,
            stabbing,
            weapon: held_gun.unwrap_or_default().as_u8(),
            sound_bits,
            ads_t: brain.ads,
            reloading: brain.reload_until.is_some(),
            jumping,
            revive: reviving_now,
            crouching: reviving_now,
            held,
            swap: look.swap,
            throw: look.throw,
            drink: look.drink,
            ..default()
        };
    }
}

/// One-shot sound bits a bot's input carries (`client::killcam::SND_*`).
const SND_SHOT: u32 = 1 << 0;
const SND_RELOAD: u32 = 1 << 1;
const SND_FOOTSTEP: u32 = 1 << 7;

/// Keeping clear of the rest of the team at `feet`: a steer away from
/// anyone close (`AVOID_RADIUS`), and a push (a velocity) out of anyone it's
/// pressed into (`MIN_SPACING`) — all of it from a player, who won't move
/// for it; half from another bot, which takes the other half.
fn spacing(feet: Vec3, me: PeerId, lobby: Entity, bodies: &[Body], dt: f32) -> (Vec3, Vec3) {
    let (mut steer, mut push) = (Vec3::ZERO, Vec3::ZERO);
    for b in bodies.iter().filter(|b| b.lobby == lobby && b.peer != me && !b.zombie && (b.alive || b.down)) {
        if (b.feet.y - feet.y).abs() > 1.5 {
            continue;
        }
        let away = (feet - b.feet).with_y(0.0);
        let d = away.length();
        if d >= AVOID_RADIUS {
            continue;
        }
        // (Right on top of each other: apart any old way, each its own.)
        let dir = away.try_normalize().unwrap_or_else(|| {
            let a = (me.to_bits() % 628) as f32 / 100.0;
            Vec3::new(a.cos(), 0.0, a.sin())
        });
        steer += dir * (1.0 - d / AVOID_RADIUS);
        if d < MIN_SPACING {
            push += dir * (MIN_SPACING - d) * if b.bot { 0.5 } else { 1.0 };
        }
    }
    let len = push.length();
    let push = if len > 1e-4 { push / len * (len / dt.max(1e-3)).min(PUSH_MAX_SPEED) } else { Vec3::ZERO };
    (steer * 0.8, push)
}

/// The way to walk toward `goal` this tick, along a route around the map
/// (never through a shut door), climbing any ledge on it.
fn walk(
    brain: &mut AllyBrain,
    world: &dyn CollisionWorld,
    nav: &NavGraph,
    goal: Vec3,
    closed: u32,
    now: f32,
    _dt: f32,
) -> Vec3 {
    if now >= brain.repath_at || brain.path_goal.distance(goal) > 2.5 {
        brain.repath_at = now + REPATH_SECS + brain.roll() * 0.6;
        brain.path_goal = goal;
        brain.path = nav.find_path_doors(world, brain.feet, goal, true, closed).unwrap_or_default();
        brain.path_index = 0;
    }
    let feet = brain.feet;
    let mut index = brain.path_index;
    let mut wish = (goal - feet).with_y(0.0).normalize_or_zero();
    if let Some(wp) = NavGraph::next_waypoint(world, &brain.path, &mut index, feet) {
        let foot = index.checked_sub(1).and_then(|i| brain.path.get(i)).map(|w| w.at);
        if let Some(foot) = foot.filter(|f| {
            wp.mantle && Vec3::new(f.x - feet.x, 0.0, f.z - feet.z).length() < MANTLE_START_DIST && (f.y - feet.y).abs() < STEP_HEIGHT
        }) {
            let low = feet + Vec3::Y * CHEST_HEIGHT;
            let high = Vec3::new(feet.x, wp.at.y + CHEST_HEIGHT, feet.z);
            let from = if world.sweep_sphere(low, high, BODY_RADIUS).is_none() { feet } else { foot };
            let climb = (wp.at.y - from.y).max(0.0);
            brain.climb = Some(Climb { start: now, secs: MANTLE_BASE_SECS + MANTLE_SECS_PER_M * climb, from, to: wp.at });
            brain.yaw = look_angles(Vec3::new(wp.at.x - feet.x, 0.0, wp.at.z - feet.z).normalize_or(Vec3::NEG_Z)).0;
            index += 1;
            wish = Vec3::ZERO;
        } else {
            wish = Vec3::new(wp.at.x - feet.x, 0.0, wp.at.z - feet.z).normalize_or_zero();
        }
    }
    brain.path_index = index;
    wish
}

/// Stuck against something while trying to move: a short detour; going
/// nowhere for seconds on end: back onto the nearest walkable spot.
fn unstick(brain: &mut AllyBrain, nav: &NavGraph, wish: Vec3, blocked: bool, now: f32, dt: f32) {
    if wish != Vec3::ZERO && blocked {
        brain.blocked_for += dt;
    } else {
        brain.blocked_for = 0.0;
    }
    if brain.blocked_for > STUCK_SECS {
        brain.blocked_for = 0.0;
        let turn = (brain.roll() - 0.5) * core::f32::consts::PI * 1.5;
        brain.detour_dir = Quat::from_rotation_y(turn) * if wish != Vec3::ZERO { wish } else { Vec3::NEG_Z };
        brain.detour_until = now + DETOUR_SECS;
        brain.repath_at = now + DETOUR_SECS;
    }
    if wish == Vec3::ZERO || brain.feet.distance(brain.stall_from) > UNSTICK_PROGRESS {
        brain.stall_from = brain.feet;
        brain.stall_secs = 0.0;
    } else if blocked {
        brain.stall_secs += dt;
        if brain.stall_secs > UNSTICK_SECS {
            if let Some(spot) = nav.nearest_spot(brain.feet) {
                brain.feet = spot;
                brain.vertical_velocity = 0.0;
                brain.path.clear();
                brain.repath_at = now;
            }
            brain.stall_from = brain.feet;
            brain.stall_secs = 0.0;
        }
    }
}

/// What a bot's deciding with ([`think`]).
struct View<'a> {
    lobby_e: Entity,
    lobby: &'a Lobby,
    me: PeerId,
    member: &'a shared::LobbyMember,
    bodies: &'a [Body],
    lying: &'a [Lying],
    team_doors: &'a TeamDoors,
}

/// How far (m) it'd walk to `to`, if it can get there at all.
fn walk_dist(world: &dyn CollisionWorld, nav: &NavGraph, from: Vec3, to: Vec3, closed: u32) -> Option<f32> {
    let path = nav.find_path_doors(world, from, to, true, closed)?;
    Some(path.windows(2).map(|w| w[0].at.distance(w[1].at)).sum())
}

/// What's worth doing with its points (and where), if anything — see the
/// module docs.
fn think(
    brain: &mut AllyBrain,
    v: &View,
    world: &dyn CollisionWorld,
    nav: &NavGraph,
    closed: u32,
    now: f32,
) -> Option<(Errand, Vec3)> {
    let map = v.lobby.map;
    let points = v.member.score;
    let reserve = brain.ally().reserve_points;
    let feet = brain.feet;
    let near = |to: Vec3| walk_dist(world, nav, feet, to, closed).filter(|d| *d <= ERRAND_MAX_DIST);
    let ok = |brain: &AllyBrain, e: Errand| !brain.gave_up_on(e, now);
    let players: Vec<&Body> = v
        .bodies
        .iter()
        .filter(|b| b.lobby == v.lobby_e && !b.zombie && !b.bot && b.peer != v.me && b.alive)
        .collect();

    // A player's been stood at a shut door they can't afford: get it.
    for (&i, &waited) in &brain.waited_at_door {
        let Ok(door) = shared::doors::can_open(map, i as usize, v.lobby.doors_open, v.lobby.power_on) else { continue };
        let broke = players
            .iter()
            .filter(|p| door.in_range(p.feet, 1.5))
            .all(|p| v.lobby.members.iter().find(|m| m.peer == p.peer).is_none_or(|m| m.score < door.cost));
        if waited >= brain.door_wait && broke && points >= door.cost + reserve / 2 && ok(brain, Errand::Door(i)) {
            if near(door.at.pos).is_some() {
                return Some((Errand::Door(i), door.at.pos));
            }
        }
    }

    // Equipment lying about it has none of, that's lain a while with no
    // player nearer it.
    for l in v.lying.iter().filter(|l| l.lobby == v.lobby_e && l.age >= DROP_WAIT_SECS) {
        let carried = if l.kind.is_tactical() { brain.tactical } else { brain.lethal };
        if carried.is_some() {
            continue;
        }
        let mine = l.pos.distance(feet);
        if mine > 15.0 || players.iter().any(|p| p.feet.distance(l.pos) < mine) {
            continue;
        }
        let errand = Errand::PickUp(l.entity, l.kind, l.pos);
        if ok(brain, errand) && near(l.pos).is_some() && brain.ready(errand, now) {
            return Some((errand, l.pos));
        }
    }

    // The gun in hand: an AK-74 off the wall, early on, over the sniper.
    let carrying = |w: SlotWeapon| v.member.weapons.contains(&w);
    if !carrying(SlotWeapon::Gun(WeaponId::Ak74)) && !carrying(SlotWeapon::Gun(WeaponId::RayGun)) {
        let cost = shared::weapon::wall_buy_cost(WeaponId::Ak74);
        if let Some(at) = shared::level::layout(map).wall_buy(WeaponId::Ak74) {
            let spot = shared::wall_buy::use_spot(at);
            if points >= cost + reserve / 2 && ok(brain, Errand::WallBuy(WeaponId::Ak74)) && near(spot).is_some() && brain.ready(Errand::WallBuy(WeaponId::Ak74), now) {
                return Some((Errand::WallBuy(WeaponId::Ak74), spot));
            }
        }
    }

    // Perks, the ones that keep a team alive first.
    if shared::perks::machines_on(v.lobby) {
        let mut perks: Vec<Perk> = v
            .lobby
            .perk_set
            .machine_perks()
            .filter(|p| p.has_machine() && !v.member.perks.contains(p))
            .collect();
        perks.sort_by_key(|p| perk_priority(*p));
        for perk in perks {
            if points < perk.cost() + reserve / 2 {
                continue;
            }
            let at = perk.machine_pos(map);
            if ok(brain, Errand::Perk(perk)) && near(at).is_some() && brain.ready(Errand::Perk(perk), now) {
                return Some((Errand::Perk(perk), at));
            }
            break;
        }
    }

    // The power, from round 2.
    if !v.lobby.power_on && v.lobby.round >= 2 && points >= shared::power::POWER_COST + reserve {
        if let Some(at) = shared::power::switch_pos(map) {
            if ok(brain, Errand::Power) && near(at).is_some() && brain.ready(Errand::Power, now) {
                return Some((Errand::Power, at));
            }
        }
    }

    // Ammo, running low (at a crate).
    let total: u32 = (0..2).filter(|&i| brain.weapons[i].gun().is_some()).map(|i| brain.ammo[i].mag + brain.ammo[i].reserve).sum();
    let low = brain.held_weapon().gun().map_or(0, |g| gun_stats(g).mag * 2);
    if total <= low && points >= shared::ammo::AMMO_COST {
        let crate_at = shared::level::layout(map)
            .ammo_crates
            .iter()
            .map(|c| c.pos)
            .min_by(|a, b| a.distance_squared(feet).total_cmp(&b.distance_squared(feet)));
        if let Some(at) = crate_at {
            if ok(brain, Errand::Ammo(at)) && near(at).is_some() {
                return Some((Errand::Ammo(at), at));
            }
        }
    }

    // Pack-a-Punch, once it's got plenty.
    if shared::power::has_power(map, v.lobby.power_on) {
        if let (Some(at), Some(gun)) = (shared::pap::machine_pos(map), brain.held_weapon().gun()) {
            let weapon = shared::pap::PapWeapon::gun(gun);
            let level = v.member.pap.get(weapon);
            let cost = shared::pap::cost_to(level, level + 1);
            if level < shared::pap::MAX_LEVEL && points >= cost + reserve && ok(brain, Errand::Pap) && near(at).is_some() && brain.ready(Errand::Pap, now) {
                return Some((Errand::Pap, at));
            }
        }
    }

    // The Mystery Box, now and then, for something better than it's got.
    let worst = brain.weapons.iter().map(|w| gun_rank(*w)).min().unwrap_or(0);
    let box_due = brain.box_round.is_none_or(|r| v.lobby.round >= r + BOX_EVERY_ROUNDS);
    if worst < gun_rank(SlotWeapon::Gun(WeaponId::RayGun))
        && box_due
        && v.lobby.mystery_box.is_none()
        && points >= shared::mystery_box::COST + reserve
    {
        if let Some(at) = shared::mystery_box::placement(map) {
            let spot = shared::mystery_box::use_spot(at);
            if ok(brain, Errand::Box) && near(spot).is_some() && brain.ready(Errand::Box, now) {
                brain.box_round = Some(v.lobby.round);
                return Some((Errand::Box, spot));
            }
        }
    }

    // Nobody's opened a door for longer than it'll wait: the cheapest in
    // reach it can afford (unless another of the team's bots just went for
    // one).
    let bot_went = v.team_doors.0.get(&v.lobby_e).is_some_and(|t| now - t < 20.0);
    if now - brain.doors_changed_at >= brain.door_patience && !bot_went {
        let pick = shared::doors::doors(map)
            .iter()
            .enumerate()
            .filter(|(i, _)| shared::doors::can_open(map, *i, v.lobby.doors_open, v.lobby.power_on).is_ok())
            .filter(|(_, d)| points >= d.cost + reserve / 3)
            .filter_map(|(i, d)| near(d.at.pos).map(|dist| (i, d, dist)))
            .min_by(|a, b| (a.1.cost, a.2 as u32).cmp(&(b.1.cost, b.2 as u32)));
        if let Some((i, d, _)) = pick {
            if ok(brain, Errand::Door(i as u8)) {
                return Some((Errand::Door(i as u8), d.at.pos));
            }
        }
    }

    // Boards back on a window close by, with nothing about.
    for (i, w) in shared::windows::windows(map).iter().enumerate() {
        if shared::windows::boards_left(&v.lobby.windows_torn, i) >= shared::windows::BOARDS {
            continue;
        }
        let spot = w.landing();
        if spot.distance(feet) < 8.0 && ok(brain, Errand::Repair(i as u8)) && near(spot).is_some() {
            return Some((Errand::Repair(i as u8), spot));
        }
    }
    None
}

/// Whether it's standing where `errand`'s done (the same check the server
/// makes, a little tighter).
fn at_errand(errand: Errand, lobby: &Lobby, feet: Vec3, lying: &[Lying]) -> bool {
    let map = lobby.map;
    match errand {
        Errand::Perk(perk) => shared::perks::in_range(perk, map, feet, 0.0),
        Errand::Power => shared::power::in_range(map, feet, 0.0),
        Errand::Pap => shared::pap::in_range(map, feet, 0.0),
        Errand::Door(i) => shared::doors::doors(map).get(i as usize).is_some_and(|d| d.in_range(feet, 0.0)),
        Errand::Box => shared::mystery_box::in_range(map, feet, 0.0),
        Errand::WallBuy(gun) => shared::wall_buy::in_range(map, gun, feet, 0.0),
        Errand::Ammo(at) => at.distance(feet) < 2.0,
        Errand::Repair(i) => shared::windows::windows(map).get(i as usize).is_some_and(|w| w.in_repair_range(feet, 0.0)),
        Errand::PickUp(entity, _, pos) => {
            lying.iter().any(|l| l.entity == entity)
                && shared::throwing_knife::in_server_pickup_range(feet, feet + Vec3::Y * EYE_HEIGHT, pos)
        }
        // (Walking onto it is grabbing it — see `drive_allies`.)
        Errand::PowerUp(_) => false,
    }
}

/// Ask for `errand` — the very handler a client's message would reach.
fn ask(errand: Errand, brain: &AllyBrain, member: &shared::LobbyMember, me: PeerId, commands: &mut Commands) {
    match errand {
        Errand::Perk(perk) => from(commands, me, shared::BuyPerk { perk, wunderfizz: false }),
        Errand::Power => from(commands, me, shared::TurnOnPower),
        Errand::Pap => {
            if let Some(gun) = brain.held_weapon().gun() {
                let weapon = shared::pap::PapWeapon::gun(gun);
                let level = member.pap.get(weapon) + 1;
                from(commands, me, shared::BuyPap { weapon, level });
            }
        }
        Errand::Door(i) => from(commands, me, shared::BuyDoor(i)),
        Errand::Box => from(
            commands,
            me,
            shared::SpinMysteryBox {
                knives_full: brain.lethal.is_some(),
                molotovs_full: brain.lethal.is_some(),
                monkeys_full: brain.tactical.is_some(),
                frags_full: brain.lethal.is_some(),
                flashes_full: brain.tactical.is_some(),
            },
        ),
        Errand::WallBuy(weapon) => {
            // (Into the knife's slot if it has one, else the worse gun's.)
            let into = (0..2).min_by_key(|&i| gun_rank(brain.weapons[i])).unwrap_or(0);
            let a = brain.ammo[into];
            from(commands, me, shared::BuyWallWeapon { weapon, slot: into as u8, mag: a.mag, reserve: a.reserve });
        }
        Errand::Ammo(_) => from(commands, me, shared::BuyAmmo),
        Errand::Repair(i) => from(commands, me, shared::RepairBarrier(i)),
        Errand::PickUp(_, kind, _) => match kind {
            LethalKind::Frag => from(commands, me, shared::PickUpFrag { dropping: None }),
            LethalKind::Molotov => from(commands, me, shared::PickUpMolotov { dropping: None }),
            LethalKind::MonkeyBomb => from(commands, me, shared::PickUpMonkey { dropping: None }),
            LethalKind::FlashBang => from(commands, me, shared::PickUpFlashBang { dropping: None }),
            LethalKind::ThrowingKnife => from(commands, me, shared::PickUpKnife { dropping: None }),
        },
        Errand::PowerUp(_) => {}
    }
}

/// Fire the handler a client's `trigger` message would reach, as `me`.
fn from<M: Message>(commands: &mut Commands, me: PeerId, trigger: M) {
    commands.trigger(RemoteTrigger { trigger, from: me });
}

/// How an errand it asked for went: `Some(true)` done (anything to show
/// for it shown), `Some(false)` it didn't, `None` not over yet.
fn errand_result(brain: &mut AllyBrain, v: &View, doing: Doing, now: f32, commands: &mut Commands) -> Option<bool> {
    let paid = v.member.score < doing.points_before;
    match doing.errand {
        // (Its drink plays once the perk's its — see `drive_allies`.)
        Errand::Perk(perk) => Some(v.member.perks.contains(&perk)),
        Errand::Power => Some(v.lobby.power_on),
        Errand::Door(i) => Some(shared::doors::is_open(v.lobby.doors_open, i as usize)),
        Errand::Pap => {
            if paid {
                // The gun goes in and comes back out packed (and full).
                brain.ammo[brain.held] = Ammo::full(brain.held_weapon());
                brain.act = Act::Swap { to: brain.held, since: now };
            }
            Some(paid)
        }
        Errand::Ammo(_) => {
            if paid {
                for i in 0..2 {
                    brain.ammo[i] = Ammo::full(brain.weapons[i]);
                }
            }
            Some(paid)
        }
        Errand::WallBuy(gun) => {
            let got = v.member.weapons.contains(&SlotWeapon::Gun(gun));
            if got {
                let slot = (0..2).find(|&i| v.member.weapons[i] == SlotWeapon::Gun(gun)).unwrap_or(0);
                brain.sync_weapons(v.member.weapons);
                brain.act = Act::Swap { to: slot, since: now };
            }
            Some(got)
        }
        Errand::Repair(i) => {
            // Board by board until it's whole (or a zombie's about).
            let whole = shared::windows::boards_left(&v.lobby.windows_torn, i as usize) >= shared::windows::BOARDS;
            if whole {
                return Some(true);
            }
            if now - doing.asked_at.unwrap_or(now) >= shared::windows::REPAIR_SECS {
                if let Some(d) = brain.doing.as_mut() {
                    d.asked_at = None;
                }
            }
            None
        }
        Errand::PowerUp(_) => Some(true),
        Errand::PickUp(entity, kind, _) => {
            let gone = !v.lying.iter().any(|l| l.entity == entity);
            if gone {
                let carried = if kind.is_tactical() { &mut brain.tactical } else { &mut brain.lethal };
                *carried = Some((kind, kind.max_carried().min(match kind {
                    LethalKind::ThrowingKnife => 3,
                    _ => 1,
                })));
            }
            Some(gone)
        }
        Errand::Box => {
            let Some(spin) = v.lobby.mystery_box.filter(|s| s.user == v.me) else {
                // Never spun (the box was busy, or it was short): give up.
                return Some(false);
            };
            match spin.phase {
                shared::mystery_box::BoxPhase::Spinning => None,
                shared::mystery_box::BoxPhase::Offering => {
                    let better_gun = spin
                        .prize
                        .gun()
                        .filter(|g| gun_rank(SlotWeapon::Gun(*g)) > brain.weapons.iter().map(|w| gun_rank(*w)).min().unwrap_or(0))
                        .filter(|g| !v.member.weapons.contains(&SlotWeapon::Gun(*g)));
                    let wanted_lethal = spin.prize.lethal().filter(|k| {
                        if k.is_tactical() { brain.tactical.is_none() } else { brain.lethal.is_none() }
                    });
                    if better_gun.is_some() || wanted_lethal.is_some() {
                        // (A gun into the knife's slot if it has one, else
                        // the worse gun's — out and in hand.)
                        let into = (0..2).min_by_key(|&i| gun_rank(brain.weapons[i])).unwrap_or(0);
                        let a = brain.ammo[into];
                        from(
                            commands,
                            v.me,
                            shared::TakeBoxPrize { slot: into as u8, mag: a.mag, reserve: a.reserve, dropping: None },
                        );
                        if better_gun.is_some() {
                            brain.act = Act::Swap { to: into, since: now };
                        }
                        if let Some(kind) = wanted_lethal {
                            let carried = if kind.is_tactical() { &mut brain.tactical } else { &mut brain.lethal };
                            // (Just the one, as for a player.)
                            *carried = Some((kind, 1));
                        }
                        Some(true)
                    } else {
                        // Not worth it: leave it.
                        Some(true)
                    }
                }
                shared::mystery_box::BoxPhase::Closing => Some(true),
            }
        }
    }
}

/// Pick something worth throwing and where: a frag / molotov into a crowd,
/// a monkey bomb or flash bang when they're on top of it.
fn pick_throw(brain: &AllyBrain, zombies: &[&Body], eye: Vec3, world: &dyn CollisionWorld) -> Option<(LethalKind, Vec3)> {
    let feet = brain.feet;
    let close = zombies.iter().filter(|z| z.feet.distance(feet) < 4.0).count();
    if let Some((kind, n)) = brain.tactical.filter(|(_, n)| *n > 0) {
        if close >= CROWD + 2 {
            let at = match kind {
                // (Thrown away from the team, to draw them off.)
                LethalKind::MonkeyBomb => feet + Quat::from_rotation_y(brain.yaw) * Vec3::NEG_Z * 10.0,
                _ => zombies.iter().min_by(|a, b| a.feet.distance_squared(feet).total_cmp(&b.feet.distance_squared(feet)))?.feet,
            };
            let _ = n;
            return Some((kind, at));
        }
    }
    let (kind, _) = brain.lethal.filter(|(k, n)| *n > 0 && *k != LethalKind::ThrowingKnife)?;
    // The middle of a crowd, a safe way off and in view.
    zombies
        .iter()
        .filter(|z| (6.0..18.0).contains(&z.feet.distance(feet)) && !world.segment_blocked(eye, z.eye))
        .find(|z| zombies.iter().filter(|o| o.feet.distance(z.feet) < CROWD_RADIUS).count() >= CROWD)
        .map(|z| (kind, z.feet))
}

/// Let `kind` go toward `at` — the same throw a client's request makes.
fn throw_it(brain: &mut AllyBrain, me: PeerId, kind: LethalKind, at: Vec3, commands: &mut Commands) {
    let eye = brain.eye();
    let flat = (at - eye).with_y(0.0);
    let dist = flat.length().max(0.1);
    // (Lobbed a little higher the farther it's going.)
    let dir = (flat / dist + Vec3::Y * (0.15 + dist * 0.02)).normalize();
    let (origin, dir) = (eye.to_array(), dir.to_array());
    match kind {
        LethalKind::Frag => from(commands, me, shared::ThrowFrag { origin, dir, fuse: shared::frag::FUSE_SECS, in_hand: false }),
        LethalKind::Molotov => from(commands, me, shared::ThrowMolotov { origin, dir }),
        LethalKind::MonkeyBomb => from(commands, me, shared::ThrowMonkey { origin, dir }),
        LethalKind::FlashBang => from(commands, me, shared::ThrowFlashBang { origin, dir }),
        // (Not thrown — `pick_throw` never picks one.)
        LethalKind::ThrowingKnife => {}
    }
    let carried = if kind.is_tactical() { &mut brain.tactical } else { &mut brain.lethal };
    if let Some((k, n)) = *carried {
        *carried = (n > 1).then_some((k, n - 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::bot_players::{ally_peer, bot_peer};
    use shared::{MapId, ZombieAnim};

    fn lobby() -> Lobby {
        Lobby {
            name: "test".into(),
            leader: PeerId::Netcode(1),
            mode: GameMode::Zombies,
            map: MapId::BreakPoint,
            started: true,
            time_limit_secs: 300,
            time_left_secs: 300,
            kill_limit: 30,
            end_cam: shared::EndCam::default(),
            round: 1,
            enemies_left: 0,
            enemies_active: 0,
            paused: false,
            bots_passive: false,
            bots_frozen: false,
            exfil: Default::default(),
            power_up_test: false,
            molotov_test: false,
            active_power_ups: Vec::new(),
            bomb_test: false,
            start_round: 1,
            start_points: 0,
            perk_set: Default::default(),
            perks_need_power: false,
            full_power: false,
            snd_rounds_to_win: 0,
            snd: Default::default(),
            countdown_secs: 0,
            countdown_left: 0,
            power_on: false,
            doors_open: 0,
            windows_torn: Vec::new(),
            rampage: false,
            carpenter: false,
            mystery_box: None,
            members: vec![member(ally_peer(0), true), member(PeerId::Netcode(1), false)],
        }
    }

    fn member(peer: PeerId, bot: bool) -> shared::LobbyMember {
        shared::LobbyMember {
            peer,
            name: "m".into(),
            score: 0,
            loaded: true,
            assets: true,
            bot: bot.then_some(BotDifficulty::Veteran),
            kills: 0,
            perks: Vec::new(),
            armor: Default::default(),
            field_upgrade: Default::default(),
            pap: Default::default(),
            loadout: Default::default(),
            operator: Default::default(),
            primary: Default::default(),
            weapons: SlotWeapon::starting(WeaponId::Sniper),
            critical_kills: 0,
            revives: 0,
            downs: 0,
            deaths: 0,
            team: Default::default(),
            plants: 0,
            defuses: 0,
        }
    }

    /// A started `Zombies` lobby on Break Point's open yard: a veteran
    /// teammate bot at `bot_feet`, and its human teammate at `player_feet`.
    fn world(bot_feet: Vec3, player_feet: Vec3) -> (App, Entity, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(core::time::Duration::from_secs_f64(1.0 / 64.0)));
        app.insert_resource(MapColliders::load());
        app.insert_resource(NavGraphs::build(&MapColliders::load()));
        app.init_resource::<crate::killcam::EndingLobbies>();
        app.init_resource::<TeamDoors>();
        app.add_event::<SayQuote>();
        // (Each bot's input published as its pose, as `sim::apply_client_pose`
        // does — what the others see of it.)
        app.add_systems(Update, (drive_allies, publish_poses).chain());
        let lobby = app.world_mut().spawn(lobby()).id();
        let bot = app
            .world_mut()
            .spawn((
                PlayerId(ally_peer(0)),
                LobbyPlayer { lobby },
                PlayerCombat::default(),
                PlayerPose { translation: bot_feet + Vec3::Y * EYE_HEIGHT, ..default() },
                AllyBrain::new(BotDifficulty::Veteran, bot_feet, 0.0, 7),
                ActionState::<PlayerInput>::default(),
            ))
            .id();
        let player = app
            .world_mut()
            .spawn((
                PlayerId(PeerId::Netcode(1)),
                LobbyPlayer { lobby },
                PlayerCombat::default(),
                PlayerPose { translation: player_feet + Vec3::Y * EYE_HEIGHT, ..default() },
            ))
            .id();
        (app, lobby, bot, player)
    }

    fn publish_poses(mut q: Query<(&ActionState<PlayerInput>, &mut PlayerPose)>) {
        for (input, mut pose) in &mut q {
            pose.translation = Vec3::from_array(input.0.translation);
        }
    }

    /// Another veteran teammate bot (`n`th), at `feet`.
    fn add_bot(app: &mut App, lobby: Entity, n: u64, feet: Vec3) -> Entity {
        let peer = ally_peer(n);
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().members.push(member(peer, true));
        app.world_mut()
            .spawn((
                PlayerId(peer),
                LobbyPlayer { lobby },
                PlayerCombat::default(),
                PlayerPose { translation: feet + Vec3::Y * EYE_HEIGHT, ..default() },
                AllyBrain::new(BotDifficulty::Veteran, feet, 0.0, 1000 + n * 77),
                ActionState::<PlayerInput>::default(),
            ))
            .id()
    }

    fn input(app: &App, bot: Entity) -> PlayerInput {
        app.world().get::<ActionState<PlayerInput>>(bot).unwrap().0.clone()
    }

    fn run(app: &mut App, secs: f32, mut each: impl FnMut(&App)) {
        for _ in 0..(secs * 64.0) as usize {
            app.update();
            each(app);
        }
    }

    #[test]
    fn a_teammate_bot_shoots_a_zombie_it_can_see() {
        let (mut app, lobby, bot, _) = world(Vec3::new(-20.0, 0.0, -40.0), Vec3::new(-22.0, 0.0, -40.0));
        app.world_mut().spawn((
            PlayerId(bot_peer(1)),
            LobbyPlayer { lobby },
            PlayerCombat::default(),
            PlayerPose { translation: Vec3::new(0.0, EYE_HEIGHT, -40.0), zombie: ZombieAnim::Walk, ..default() },
        ));
        let mut shots = 0;
        run(&mut app, 4.0, |app| {
            let i = input(app, bot);
            if i.fire {
                shots += 1;
                assert!(i.fire_dir[0] > 0.9, "shot went {:?}", i.fire_dir);
                assert_eq!(i.held, Some(SlotWeapon::Gun(WeaponId::Sniper)));
            }
        });
        assert!(shots >= 2, "fired {shots} times in 4 s");
        // (Rounds came out of the magazine — or it's reloading.)
        let b = app.world().get::<AllyBrain>(bot).unwrap();
        assert!(b.ammo[0].mag < 5 || b.reload_until.is_some());
    }

    #[test]
    fn a_teammate_bot_keeps_up_with_its_player() {
        let (mut app, _, bot, _) = world(Vec3::new(-30.0, 0.0, -40.0), Vec3::new(-10.0, 0.0, -40.0));
        run(&mut app, 6.0, |_| {});
        let feet = app.world().get::<AllyBrain>(bot).unwrap().feet;
        assert!(feet.distance(Vec3::new(-10.0, 0.0, -40.0)) <= FOLLOW_FAR + 0.5, "still at {feet}");
    }

    #[test]
    fn a_teammate_bot_goes_to_a_downed_player_and_revives_them() {
        let (mut app, _, bot, player) = world(Vec3::new(-30.0, 0.0, -40.0), Vec3::new(-24.0, 0.0, -40.0));
        app.world_mut().get_mut::<PlayerCombat>(player).unwrap().go_down(Vec::new(), false);
        let mut reviving = false;
        run(&mut app, 5.0, |app| reviving |= input(app, bot).revive);
        assert!(reviving, "never held revive");
    }

    #[test]
    fn a_downed_teammate_bot_lies_down_and_gets_up_where_it_was() {
        let (mut app, _, bot, _) = world(Vec3::new(-30.0, 0.0, -40.0), Vec3::new(-29.0, 0.0, -40.0));
        run(&mut app, 0.5, |_| {});
        app.world_mut().get_mut::<PlayerCombat>(bot).unwrap().go_down(Vec::new(), false);
        run(&mut app, 0.5, |app| {
            let i = input(app, bot);
            assert!(i.prone && i.held.is_none(), "down but not lying down: {i:?}");
        });
        let before = app.world().get::<AllyBrain>(bot).unwrap().feet;
        app.world_mut().get_mut::<PlayerCombat>(bot).unwrap().revive();
        app.update();
        let after = app.world().get::<AllyBrain>(bot).unwrap().feet;
        assert!(after.distance(before) < 0.5, "jumped from {before} to {after}");
        assert!(!input(&app, bot).prone);
    }

    #[test]
    fn teammate_bots_spread_round_their_player_and_never_stand_in_each_other() {
        let player = Vec3::new(-20.0, 0.0, -40.0);
        let (mut app, lobby, first, _) = world(Vec3::new(-30.0, 0.0, -40.0), player);
        let bots = [first, add_bot(&mut app, lobby, 1, Vec3::new(-30.5, 0.0, -40.0)), add_bot(&mut app, lobby, 2, Vec3::new(-30.0, 0.0, -40.5))];
        let feet = |app: &App, e: Entity| app.world().get::<AllyBrain>(e).unwrap().feet;
        let mut worst = f32::MAX;
        // (Settled in after a few seconds, then watched while they move about.)
        run(&mut app, 4.0, |_| {});
        run(&mut app, 12.0, |app| {
            for (i, a) in bots.iter().enumerate() {
                let fa = feet(app, *a);
                worst = worst.min(fa.with_y(0.0).distance(player.with_y(0.0)));
                for b in &bots[i + 1..] {
                    worst = worst.min(fa.with_y(0.0).distance(feet(app, *b).with_y(0.0)));
                }
            }
        });
        assert!(worst >= MIN_SPACING * 0.75, "two of them were {worst} m apart");
        // Not all on one spot: each round the player its own way.
        let spots: Vec<Vec3> = bots.iter().map(|b| feet(&app, *b)).collect();
        assert!(spots[0].distance(spots[1]) > 1.0 || spots[1].distance(spots[2]) > 1.0, "{spots:?}");
    }

    #[test]
    fn a_teammate_bot_goes_for_a_power_up_before_it_runs_out() {
        let (mut app, lobby, bot, _) = world(Vec3::new(-30.0, 0.0, -40.0), Vec3::new(-31.0, 0.0, -40.0));
        let at = Vec3::new(-22.0, 0.0, -40.0);
        app.world_mut().spawn((
            shared::PowerUpDrop { kind: shared::power_ups::PowerUp::ALL[0], pos: at, blinking: false },
            crate::power_ups::DropSim { lobby, age: 0.0 },
        ));
        let mut reached = false;
        run(&mut app, shared::power_ups::DROP_LIFETIME_SECS - 2.0, |app| {
            let f = app.world().get::<AllyBrain>(bot).unwrap().feet;
            reached |= f.with_y(0.0).distance(at.with_y(0.0)) < shared::power_ups::PICKUP_RADIUS;
        });
        assert!(reached, "never got to it");
    }

    #[test]
    fn bots_sometimes_go_at_once_and_sometimes_hold_off_and_stick_to_it() {
        let (mut now_, mut later) = (0, 0);
        for seed in 0..200 {
            let mut b = AllyBrain::new(BotDifficulty::Regular, Vec3::ZERO, 0.0, seed);
            if b.ready(Errand::Power, 10.0) {
                now_ += 1;
            } else {
                later += 1;
                // (Still waiting a moment later, done once its time's come.)
                assert!(!b.ready(Errand::Power, 10.5));
                assert!(b.ready(Errand::Power, 60.0));
            }
        }
        assert!(now_ > 40 && later > 40, "{now_} at once, {later} later");
    }

    #[test]
    fn a_drink_puts_the_gun_away_drinks_for_as_long_as_a_players_and_gets_it_back() {
        let act = Act::Drink { perk: Perk::Juggernog, since: 0.0 };
        let at = |t: f32| act_look(act, 0, t);
        assert!(matches!(at(0.1).swap, Some(SwapProgress::Stowing(_))));
        let mid = at(STOW_SECS + DRINK_SECS * 0.5);
        assert_eq!(mid.held, None);
        assert!(mid.drink.is_some_and(|d| (d.t - 0.5).abs() < 1e-3 && d.perk == Perk::Juggernog));
        assert!(matches!(at(STOW_SECS + DRINK_SECS + 0.1).swap, Some(SwapProgress::Drawing(_))));
        assert!(at(STOW_SECS + DRINK_SECS + DRAW_SECS + 0.01).done);
    }

    #[test]
    fn a_throw_lets_go_on_the_first_person_throws_release() {
        let act = Act::Throw { kind: LethalKind::Frag, at: Vec3::ZERO, since: 0.0, sent: false };
        let released = |t: f32| {
            matches!(act_look(act, 0, t).throw, Some(ThrowProgress { stage: ThrowStage::Throwing { released: true, .. }, .. }))
        };
        let release = STOW_SECS + RAISE_SECS + THROW_RELEASE_SECS;
        assert!(!released(release - 0.01));
        assert!(released(release + 0.01));
    }
}
