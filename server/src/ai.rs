//! `FreeForAll` bot players: very basic AI.
//!
//! A bot is a lobby member with a fake peer id (`shared::bot_players`) and a
//! real player entity — `PlayerId`, `PlayerPose`, `PlayerCombat`,
//! `ActionState<PlayerInput>` — exactly what a connected client gets. The only
//! difference is who fills in the `ActionState`: [`drive_bots`] does it here,
//! every tick, ahead of the systems that read it. Everything downstream is
//! therefore unchanged: [`crate::sim::apply_client_pose`] publishes the bot's
//! pose (so real clients animate it as a remote player), [`crate::sim`]
//! resolves its shots against everyone else, [`crate::pvp`] takes health off
//! whoever it hits and credits the kill, and [`crate::killcam`] records its
//! "view" so a real player it kills gets a kill cam.
//!
//! Routes come from `nav`: a walkable graph computed from the map's collision
//! mesh (nothing per-map to author), searched with A*. A bot re-plans about
//! once a second, or sooner if its target moves a lot or it gets stuck, and
//! falls back to walking straight at the target when there's no route.
//!
//! The behaviour, per bot per tick: pick the nearest living enemy (real
//! players *and* other bots — it's a free-for-all); if it can see it and it's
//! been in view for the difficulty's reaction time, stop, swing the aim onto it
//! and fire on the difficulty's interval with the difficulty's aim error;
//! otherwise follow its route toward it (walking, or sprinting on higher
//! difficulties), sliding along walls and picking a random detour when stuck.
//! No cover, strafing or dodging yet.

use bevy::prelude::*;

use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::*;

use shared::bot_players::{BotDifficulty, BotSkill};
use shared::bots::rand01;
use shared::map::CollisionWorld;
use shared::weapon::sniper_feel;
use shared::{Lobby, PlayerId, PlayerInput, PlayerPose};

use crate::collision::MapColliders;
use crate::lobby::LobbyPlayer;
use crate::nav::{NavGraph, NavGraphs};
use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;

/// The client's one-shot sound bits (`client::killcam::SND_*`) that a bot
/// triggers, so real players hear it: the sniper's shot and its footsteps.
const SND_SHOT: u16 = 1 << 0;
const SND_RECHAMBER: u16 = 1 << 2;
const SND_AIM_IN: u16 = 1 << 5;
const SND_AIM_OUT: u16 = 1 << 6;
const SND_FOOTSTEP: u16 = 1 << 7;

/// Walking / sprinting speeds (m/s) — the client's `WALK_SPEED` / `SPRINT_SPEED`
/// defaults, so a bot's remote model plays the right animation (the client
/// picks walk vs. sprint from how fast the avatar actually moves).
pub(crate) const WALK_SPEED: f32 = 5.0;
const SPRINT_SPEED: f32 = 9.0;
/// A bot sprints toward a target farther than this (if its difficulty sprints).
const SPRINT_MIN_DISTANCE: f32 = 30.0;

// A bot's body — a sphere swept at chest height, and the tallest step it climbs
// — is defined in `nav`, shared with the pathfinding graph so the two agree
// on what can be walked.
use crate::nav::{BODY_RADIUS, CHEST_HEIGHT, STEP_HEIGHT};
/// Downward acceleration while airborne (m/s²).
const GRAVITY: f32 = 20.0;
/// A bot that ends up this far below the map is put back at a spawn point.
const VOID_Y: f32 = -40.0;

/// Aiming counts as "on target" within this many degrees.
const AIM_TOLERANCE_DEG: f32 = 2.0;
/// How often (s) a bot reconsiders who to go after.
const RETARGET_SECS: f32 = 1.0;
/// Blocked this long (s) while trying to move → pick a detour.
const STUCK_SECS: f32 = 0.6;
/// How long (s) a detour lasts.
const DETOUR_SECS: f32 = 1.2;
/// A route is re-planned about this often (s), plus a little jitter so 20 bots
/// don't all search on the same tick...
const REPATH_SECS: f32 = 1.2;
/// ...or as soon as the target has moved this far (m) from where the route was
/// planned to.
const REPATH_TARGET_MOVED: f32 = 4.0;

/// Counter for handing out fake peer ids.
#[derive(Resource, Default)]
pub struct NextBotId(pub u64);

/// Rising up out of the ground (a `Zombies` spawn): the feet climb from
/// [`RISE_DEPTH`] under `ground` up onto it over `secs`, the bot doing nothing
/// else meanwhile.
#[derive(Clone, Copy)]
struct Rise {
    start: f32,
    secs: f32,
    ground: Vec3,
}

/// How far under the ground (m) a rising bot starts — a whole body.
const RISE_DEPTH: f32 = 1.9;

/// A bot player's mind and body state.
#[derive(Component)]
pub struct BotBrain {
    difficulty: BotDifficulty,
    /// Replaces `difficulty.skill()` when set (`Zombies` scales it by round).
    skill_override: Option<BotSkill>,
    rise: Option<Rise>,
    /// Feet position — the bot's authoritative place in the world (the pose
    /// published to clients is this plus the eye height).
    feet: Vec3,
    vertical_velocity: f32,
    yaw: f32,
    pitch: f32,
    target: Option<PeerId>,
    retarget_at: f32,
    /// Seconds the current target has been continuously in view.
    seen_for: f32,
    next_fire_at: f32,
    /// Was alive last tick — a `false` → `true` flip is a respawn.
    was_alive: bool,
    blocked_for: f32,
    detour_until: f32,
    detour_dir: Vec3,
    /// The current route (feet waypoints), how far along it the bot is, where
    /// the target was when it was planned, and when to plan again.
    path: Vec<Vec3>,
    path_index: usize,
    path_goal: Vec3,
    repath_at: f32,
    stride: f32,
    /// Aim-down-sight amount (0 hip … 1 scoped), eased like a real player's so
    /// the kill cam's scope-in isn't a snap.
    ads: f32,
    /// When the bot last fired (`Time::elapsed_secs`) — drives the recorded
    /// fire animation and whether the rechamber sound has played yet.
    last_shot_at: Option<f32>,
    rechambered: bool,
    /// Camera-shake state, run through the client's formulas (see
    /// `shared::weapon::sniper_feel`) so a kill cam through a bot's eyes shakes
    /// and kicks like a human's.
    shake_trauma: f32,
    shake_phase: f32,
    shake_recoil: f32,
    /// Running counter feeding [`rand01`], so every roll differs.
    rng: u64,
}

impl BotBrain {
    pub fn new(difficulty: BotDifficulty, feet: Vec3, seed: u64) -> Self {
        Self {
            difficulty,
            skill_override: None,
            rise: None,
            feet,
            vertical_velocity: 0.0,
            yaw: rand01(seed) * core::f32::consts::TAU,
            pitch: 0.0,
            target: None,
            retarget_at: 0.0,
            seen_for: 0.0,
            // Don't fire in the first moment of a match.
            next_fire_at: 1.0,
            was_alive: true,
            blocked_for: 0.0,
            detour_until: 0.0,
            detour_dir: Vec3::ZERO,
            path: Vec::new(),
            path_index: 0,
            path_goal: Vec3::ZERO,
            repath_at: 0.0,
            stride: 0.0,
            ads: 0.0,
            last_shot_at: None,
            rechambered: true,
            shake_trauma: 0.0,
            shake_phase: 0.0,
            shake_recoil: 0.0,
            rng: seed,
        }
    }

    /// Start facing `yaw` (radians) instead of a random way.
    pub fn facing(mut self, yaw: f32) -> Self {
        self.yaw = yaw;
        self
    }

    /// Play with `skill` instead of the difficulty's own.
    pub fn with_skill(mut self, skill: BotSkill) -> Self {
        self.skill_override = Some(skill);
        self
    }

    /// Start under the ground at the feet position and rise up onto it over
    /// `secs` from `now`.
    pub fn rising(mut self, now: f32, secs: f32) -> Self {
        self.rise = Some(Rise {
            start: now,
            secs,
            ground: self.feet,
        });
        self.feet -= Vec3::Y * RISE_DEPTH;
        // No shooting the moment it's up, either.
        self.next_fire_at = now + secs + 1.0;
        self
    }

    fn skill(&self) -> BotSkill {
        self.skill_override.unwrap_or_else(|| self.difficulty.skill())
    }

    fn roll(&mut self) -> f32 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        rand01(self.rng)
    }
}

pub struct BotAiPlugin;

impl Plugin for BotAiPlugin {
    fn build(&self, app: &mut App) {
        // Every map's walkable graph, computed from its collision mesh now (a
        // fraction of a second) rather than authored by hand.
        let started = std::time::Instant::now();
        let navs = NavGraphs::build(&MapColliders::load());
        info!(
            "built the bot navigation graphs in {:?} ({} / {} / {} usable nodes: basic / shipment / break point)",
            started.elapsed(),
            navs.graph(shared::MapId::BasicMap).usable_count(),
            navs.graph(shared::MapId::Shipment).usable_count(),
            navs.graph(shared::MapId::BreakPoint).usable_count(),
        );

        app.insert_resource(navs)
            .init_resource::<NextBotId>()
            .add_systems(
                FixedUpdate,
                drive_bots.before(crate::sim::apply_client_pose),
            );
    }
}

/// Where the bot's feet end up after trying to move `wish` (a horizontal unit
/// direction, or zero) at `speed` for `dt`: swept as a small sphere at chest
/// height (sliding along what it hits), then snapped to the ground below —
/// falling if there is none, refusing a step up taller than [`STEP_HEIGHT`].
pub(crate) struct Moved {
    /// Ran into something on the way.
    pub(crate) blocked: bool,
    pub(crate) grounded: bool,
}

pub(crate) fn move_bot(
    world: &dyn CollisionWorld,
    feet: &mut Vec3,
    vertical_velocity: &mut f32,
    wish: Vec3,
    speed: f32,
    dt: f32,
) -> Moved {
    let before = *feet;
    let mut blocked = false;

    let delta = wish * speed * dt;
    if delta.length_squared() > 1e-10 {
        let center = *feet + Vec3::Y * CHEST_HEIGHT;
        match world.sweep_sphere(center, center + delta, BODY_RADIUS) {
            None => *feet += delta,
            Some(hit) => {
                blocked = true;
                let travelled = delta * hit.fraction;
                let rest = delta - travelled;
                let slide = rest - hit.normal * rest.dot(hit.normal);
                let c2 = center + travelled;
                let moved = if slide.length_squared() > 1e-10
                    && world
                        .sweep_sphere(c2, c2 + slide, BODY_RADIUS)
                        .is_none()
                {
                    travelled + slide
                } else {
                    travelled
                };
                *feet += moved;
            }
        }
    }

    // Ground: look for a surface from a metre above the feet down to a step
    // below them.
    let origin = *feet + Vec3::Y;
    let grounded = match world.raycast(origin, Vec3::NEG_Y, 1.0 + STEP_HEIGHT) {
        Some(hit) => {
            let surface = origin.y - hit.distance;
            if surface - before.y > STEP_HEIGHT {
                // A wall, not a step: stay where we were.
                feet.x = before.x;
                feet.z = before.z;
                blocked = true;
            } else {
                feet.y = surface;
            }
            *vertical_velocity = 0.0;
            true
        }
        None => {
            *vertical_velocity -= GRAVITY * dt;
            feet.y += *vertical_velocity * dt;
            false
        }
    };
    Moved { blocked, grounded }
}

/// Wrap `to - from` into `(-π, π]`.
pub(crate) fn shortest_angle(from: f32, to: f32) -> f32 {
    let tau = core::f32::consts::TAU;
    let d = (to - from).rem_euclid(tau);
    if d > core::f32::consts::PI {
        d - tau
    } else {
        d
    }
}

/// Yaw / pitch (radians) that look along `dir` — the client's convention:
/// yaw 0 faces -Z, positive pitch looks up.
pub(crate) fn look_angles(dir: Vec3) -> (f32, f32) {
    (f32::atan2(-dir.x, -dir.z), dir.y.clamp(-1.0, 1.0).asin())
}

fn forward(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos())
}

/// One bot's whole turn: decide, move, aim, maybe fire, and write the result
/// into its `ActionState` for the rest of the tick to consume.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_bots(
    time: Res<Time>,
    colliders: Res<MapColliders>,
    navs: Res<NavGraphs>,
    lobbies: Query<&Lobby>,
    endings: Res<crate::killcam::EndingLobbies>,
    others: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &PlayerCombat)>,
    mut bots: Query<(
        &PlayerId,
        &LobbyPlayer,
        &PlayerCombat,
        &mut BotBrain,
        &mut ActionState<PlayerInput>,
    )>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    if dt <= 0.0 {
        return;
    }

    for (id, lp, combat, mut brain_ref, mut action) in &mut bots {
        // Plain `&mut` so its fields can be borrowed separately below.
        let brain = &mut *brain_ref;
        let Some(lobby) = lobbies.get(lp.lobby).ok().filter(|l| l.started) else {
            continue;
        };
        let world = colliders.world(lobby.map);
        let nav = navs.graph(lobby.map);
        let skill: BotSkill = brain.skill();
        // Who it may go after: anyone else in a free-for-all; only the real
        // players in `Zombies` (the zombies are all on one side).
        let zombies = lobby.mode == shared::GameMode::Zombies;
        let enemy = |peer: PeerId| peer != id.0 && !(zombies && shared::bot_players::is_bot_peer(peer));

        // The match is over (`EndingLobbies`): stand still, fire nothing.
        if endings.is_frozen(lp.lobby) {
            action.0 = PlayerInput {
                translation: (brain.feet + Vec3::Y * EYE_HEIGHT).to_array(),
                yaw: brain.yaw,
                pitch: brain.pitch,
                ..default()
            };
            continue;
        }

        // Dead bots stand still and shoot no one; a flip back to alive is a
        // respawn at a fresh spot.
        if !combat.alive {
            brain.was_alive = false;
            brain.target = None;
            brain.seen_for = 0.0;
            action.0 = PlayerInput {
                translation: (brain.feet + Vec3::Y * EYE_HEIGHT).to_array(),
                yaw: brain.yaw,
                pitch: brain.pitch,
                ..default()
            };
            continue;
        }
        // Still climbing out of the ground: just rise, nothing else.
        if let Some(rise) = brain.rise {
            let t = ((now - rise.start) / rise.secs.max(1e-3)).clamp(0.0, 1.0);
            brain.feet = rise.ground - Vec3::Y * RISE_DEPTH * (1.0 - t);
            if t >= 1.0 {
                brain.rise = None;
            }
            action.0 = PlayerInput {
                translation: (brain.feet + Vec3::Y * EYE_HEIGHT).to_array(),
                yaw: brain.yaw,
                pitch: 0.0,
                ..default()
            };
            continue;
        }
        if !brain.was_alive {
            let enemies: Vec<Vec3> = others
                .iter()
                .filter(|(pid, _, olp, oc)| pid.0 != id.0 && olp.lobby == lp.lobby && oc.alive)
                .map(|(_, pose, ..)| pose.translation)
                .collect();
            let seed = (now.to_bits() as u64) ^ id.0.to_bits();
            let (pos, yaw) = shared::spawns::spawn_point(seed, &enemies, lobby.map);
            brain.feet = pos;
            brain.yaw = yaw;
            brain.pitch = 0.0;
            brain.vertical_velocity = 0.0;
            brain.was_alive = true;
            brain.next_fire_at = now + 1.0;
            brain.seen_for = 0.0;
        }

        let eye = brain.feet + Vec3::Y * EYE_HEIGHT;

        // Who to go after: the nearest living enemy in the lobby, re-picked
        // periodically or when the last one dies.
        let target_alive = brain.target.is_some_and(|t| {
            others
                .iter()
                .any(|(pid, _, olp, oc)| pid.0 == t && olp.lobby == lp.lobby && oc.alive && enemy(t))
        });
        if !target_alive || now >= brain.retarget_at {
            brain.retarget_at = now + RETARGET_SECS;
            let previous = brain.target;
            brain.target = others
                .iter()
                .filter(|(pid, _, olp, oc)| enemy(pid.0) && olp.lobby == lp.lobby && oc.alive)
                .min_by(|a, b| {
                    a.1.translation
                        .distance_squared(eye)
                        .total_cmp(&b.1.translation.distance_squared(eye))
                })
                .map(|(pid, ..)| pid.0);
            // Only a *different* target starts the reaction clock over — and
            // needs a new route.
            if brain.target != previous {
                brain.seen_for = 0.0;
                brain.path.clear();
                brain.repath_at = now;
            }
        }
        let target_pose = brain.target.and_then(|t| {
            others
                .iter()
                .find(|(pid, ..)| pid.0 == t)
                .map(|(_, pose, ..)| pose.translation)
        });

        // Can it see them? (Chest of the target, through the map's geometry.)
        let mut visible = false;
        let mut to_target = Vec3::ZERO;
        let mut distance = f32::MAX;
        if let Some(t_eye) = target_pose {
            let chest = t_eye - Vec3::Y * 0.45;
            to_target = chest - eye;
            distance = to_target.length();
            visible = distance <= skill.sight_range && !world.segment_blocked(eye, chest);
        }
        brain.seen_for = if visible { brain.seen_for + dt } else { 0.0 };
        let engaged = visible && brain.seen_for >= skill.reaction_secs;

        // Where to walk if not engaged: along the planned route to the target,
        // re-planned now and then; `None` (no route / no target) falls back to
        // heading straight at it below.
        let flat_to_target = Vec3::new(to_target.x, 0.0, to_target.z).normalize_or_zero();
        let mut path_wish: Option<Vec3> = None;
        if !engaged {
            if let Some(t_eye) = target_pose {
                let goal = t_eye - Vec3::Y * EYE_HEIGHT;
                if now >= brain.repath_at || brain.path_goal.distance(goal) > REPATH_TARGET_MOVED {
                    let jitter = brain.roll() * 0.6;
                    brain.repath_at = now + REPATH_SECS + jitter;
                    brain.path_goal = goal;
                    brain.path = nav.find_path(world, brain.feet, goal).unwrap_or_default();
                    brain.path_index = 0;
                }
                let feet = brain.feet;
                let mut index = brain.path_index;
                if let Some(wp) = NavGraph::next_waypoint(world, &brain.path, &mut index, feet) {
                    path_wish = Some(Vec3::new(wp.x - feet.x, 0.0, wp.z - feet.z).normalize_or_zero())
                        .filter(|d| *d != Vec3::ZERO);
                }
                brain.path_index = index;
            }
        }

        // Swing the aim toward the target (or the way it's walking).
        let (mut want_yaw, want_pitch) = if visible && distance > 0.1 {
            look_angles(to_target / distance)
        } else if let Some(d) = path_wish {
            (look_angles(d).0, 0.0)
        } else if flat_to_target != Vec3::ZERO {
            (look_angles(flat_to_target).0, 0.0)
        } else {
            (brain.yaw, 0.0)
        };
        if brain.detour_until > now {
            want_yaw = look_angles(brain.detour_dir).0;
        }
        let max_turn = skill.turn_deg_per_sec.to_radians() * dt;
        brain.yaw += shortest_angle(brain.yaw, want_yaw).clamp(-max_turn, max_turn);
        // Keep it in (-π, π] so angle comparisons stay simple.
        brain.yaw = shortest_angle(0.0, brain.yaw);
        brain.pitch += (want_pitch - brain.pitch).clamp(-max_turn, max_turn);

        // Movement: stand and aim while engaged; otherwise close in.
        let mut wish = Vec3::ZERO;
        let mut speed = WALK_SPEED;
        if !engaged {
            if brain.detour_until > now {
                wish = brain.detour_dir;
            } else if let Some(d) = path_wish {
                wish = d;
            } else if flat_to_target != Vec3::ZERO {
                wish = flat_to_target;
            } else {
                // Nobody to go after: wander the way it's facing.
                wish = Vec3::new(-brain.yaw.sin(), 0.0, -brain.yaw.cos());
            }
            if skill.sprints && distance > SPRINT_MIN_DISTANCE && !visible {
                speed = SPRINT_SPEED;
            }
        }
        let moved = move_bot(
            world,
            &mut brain.feet,
            &mut brain.vertical_velocity,
            wish,
            speed,
            dt,
        );

        // Stuck against something while trying to move → a random detour.
        if wish != Vec3::ZERO && moved.blocked {
            brain.blocked_for += dt;
        } else {
            brain.blocked_for = 0.0;
        }
        if brain.blocked_for > STUCK_SECS {
            brain.blocked_for = 0.0;
            let turn = (brain.roll() - 0.5) * core::f32::consts::PI * 1.5;
            let base = if wish != Vec3::ZERO { wish } else { Vec3::NEG_Z };
            brain.detour_dir = Quat::from_rotation_y(turn) * base;
            brain.detour_until = now + DETOUR_SECS;
            // Whatever it was following didn't work — plan again afterwards.
            brain.repath_at = now + DETOUR_SECS;
        }
        if brain.feet.y < VOID_Y {
            // Off the map somehow — back to a spawn point.
            brain.feet = shared::spawns::spawn_point(now.to_bits() as u64, &[], lobby.map).0;
            brain.vertical_velocity = 0.0;
        }

        // Fire: aimed, engaged, and the bolt is cycled.
        let mut fire = false;
        let mut fire_dir = forward(brain.yaw, brain.pitch);
        let aim_error = shortest_angle(brain.yaw, want_yaw)
            .abs()
            .max((brain.pitch - want_pitch).abs());
        if engaged && now >= brain.next_fire_at && aim_error <= AIM_TOLERANCE_DEG.to_radians() {
            let err = skill.aim_error_deg.to_radians();
            let yaw = brain.yaw + (brain.roll() * 2.0 - 1.0) * err;
            let pitch = brain.pitch + (brain.roll() * 2.0 - 1.0) * err;
            fire_dir = forward(yaw, pitch);
            fire = true;
            brain.next_fire_at = now + skill.fire_interval_secs * (0.85 + brain.roll() * 0.4);
        }

        // Footsteps while it's actually walking.
        let mut sound_bits = if fire { SND_SHOT } else { 0 };
        if moved.grounded && wish != Vec3::ZERO {
            brain.stride += speed * dt;
            let stride_len = if speed > WALK_SPEED { 2.7 } else { 2.2 };
            if brain.stride >= stride_len {
                brain.stride -= stride_len;
                sound_bits |= SND_FOOTSTEP;
            }
        }

        // Scope in / out over the same 0.4 s a real player takes, with the same
        // aim sounds on each change of direction.
        let ads_goal = if engaged { 1.0 } else { 0.0 };
        if ads_goal != brain.ads {
            let step = dt / sniper_feel::ADS_SECS;
            let started = if ads_goal > brain.ads {
                brain.ads == 0.0
            } else {
                brain.ads == 1.0
            };
            if started {
                sound_bits |= if engaged { SND_AIM_IN } else { SND_AIM_OUT };
            }
            brain.ads = if ads_goal > brain.ads {
                (brain.ads + step).min(1.0)
            } else {
                (brain.ads - step).max(0.0)
            };
        }

        // The shot's animation, recoil kick and shake, then their decay.
        if fire {
            brain.last_shot_at = Some(now);
            brain.rechambered = false;
            brain.shake_trauma = (brain.shake_trauma + sniper_feel::SHAKE_ADD).min(1.0);
            brain.shake_recoil = sniper_feel::RECOIL_KICK;
        } else {
            brain.shake_recoil *= (-sniper_feel::RECOIL_RETURN * dt).exp();
            if brain.shake_recoil < 1.0e-5 {
                brain.shake_recoil = 0.0;
            }
            brain.shake_trauma = (brain.shake_trauma - sniper_feel::SHAKE_DECAY * dt).max(0.0);
            if brain.shake_trauma > 0.0 {
                brain.shake_phase += dt * sniper_feel::SHAKE_FREQ;
            } else {
                brain.shake_phase = 0.0;
            }
        }
        let since_shot = brain.last_shot_at.map_or(f32::INFINITY, |t| now - t);
        // The bolt-cycle click, once the Shoot segment gives way to Rechamber.
        if !brain.rechambered && since_shot >= sniper_feel::SHOOT_END_SECS {
            brain.rechambered = true;
            sound_bits |= SND_RECHAMBER;
        }

        let eye = brain.feet + Vec3::Y * EYE_HEIGHT;
        action.0 = PlayerInput {
            translation: eye.to_array(),
            yaw: brain.yaw,
            pitch: brain.pitch,
            fire,
            fire_origin: eye.to_array(),
            fire_dir: fire_dir.to_array(),
            sound_bits,
            // The remote model's aim pose and the kill cam's scope.
            ads_t: brain.ads,
            anim_time: sniper_feel::anim_time(since_shot),
            shake_trauma: brain.shake_trauma,
            shake_phase: brain.shake_phase,
            shake_recoil: brain.shake_recoil,
            jumping: !moved.grounded,
            ..default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::MapColliders;
    use shared::MapId;

    #[test]
    fn look_angles_face_the_direction_they_are_given() {
        let (yaw, pitch) = look_angles(Vec3::NEG_Z);
        assert!(yaw.abs() < 1e-5 && pitch.abs() < 1e-5);
        let (yaw, _) = look_angles(Vec3::NEG_X);
        // yaw +90° turns -Z toward -X.
        assert!((yaw - core::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let f = forward(yaw, 0.0);
        assert!((f - Vec3::NEG_X).length() < 1e-5);
        let (_, pitch) = look_angles(Vec3::Y);
        assert!((pitch - core::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn shortest_angle_takes_the_short_way_round() {
        let d = shortest_angle(3.0, -3.0);
        assert!(d > 0.0 && d < 0.5, "wrapped the wrong way: {d}");
    }

    #[test]
    fn a_bot_walks_across_open_ground_and_stays_on_it() {
        let c = MapColliders::load();
        let world = c.world(MapId::BreakPoint);
        // A flat, open strip along the south of Break Point.
        let mut feet = Vec3::new(-30.0, 0.0, -40.0);
        let mut vy = 0.0;
        let start = feet;
        for _ in 0..64 {
            let m = move_bot(world, &mut feet, &mut vy, Vec3::X, WALK_SPEED, 1.0 / 64.0);
            assert!(m.grounded && !m.blocked);
        }
        assert!((feet.x - start.x - WALK_SPEED).abs() < 0.2, "moved {}", feet.x - start.x);
        assert!(feet.y.abs() < 0.05, "left the ground: y = {}", feet.y);
    }

    #[test]
    fn a_bot_is_stopped_by_a_wall_and_never_walks_through_it() {
        let c = MapColliders::load();
        let world = c.world(MapId::BreakPoint);
        // Just west of Break Point's tall wall at x ≈ 11.9 (running z 24..60),
        // walking straight at it.
        let mut feet = Vec3::new(6.0, 0.0, 40.0);
        let mut vy = 0.0;
        let mut blocked_once = false;
        for _ in 0..(4 * 64) {
            let m = move_bot(world, &mut feet, &mut vy, Vec3::X, WALK_SPEED, 1.0 / 64.0);
            blocked_once |= m.blocked;
            assert!(feet.x < 11.9, "walked through the wall to x = {}", feet.x);
        }
        assert!(blocked_once, "never noticed the wall");
    }

    #[test]
    fn a_bot_off_the_edge_of_the_ground_falls() {
        let c = MapColliders::load();
        let world = c.world(MapId::BreakPoint);
        let mut feet = Vec3::new(-20.0, 30.0, -40.0);
        let mut vy = 0.0;
        for _ in 0..8 {
            let m = move_bot(world, &mut feet, &mut vy, Vec3::ZERO, 0.0, 1.0 / 64.0);
            assert!(!m.grounded);
        }
        assert!(feet.y < 30.0 && vy < 0.0);
    }

    // --- the whole `drive_bots` loop, on the real map -----------------------

    use shared::bot_players::bot_peer;
    use shared::GameMode;

    fn lobby(started: bool) -> Lobby {
        Lobby {
            name: "test".into(),
            leader: bot_peer(0),
            mode: GameMode::FreeForAll,
            map: MapId::BreakPoint,
            started,
            time_limit_secs: 300,
            time_left_secs: 300,
            kill_limit: 30,
            end_cam: shared::EndCam::default(),
            round: 0,
            enemies_left: 0,
            members: Vec::new(),
        }
    }

    /// A one-lobby world: a bot of `difficulty` at `bot_feet`, and a stationary
    /// living target (another player) with its eye at `target_feet + eye height`.
    fn world(difficulty: BotDifficulty, bot_feet: Vec3, target_feet: Vec3) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            core::time::Duration::from_secs_f64(1.0 / 64.0),
        ));
        app.insert_resource(MapColliders::load());
        app.insert_resource(NavGraphs::build(&MapColliders::load()));
        app.init_resource::<crate::killcam::EndingLobbies>();
        app.add_systems(Update, drive_bots);
        let lobby = app.world_mut().spawn(lobby(true)).id();
        let bot = app
            .world_mut()
            .spawn((
                PlayerId(bot_peer(0)),
                LobbyPlayer { lobby },
                PlayerCombat::default(),
                BotBrain::new(difficulty, bot_feet, 7),
                ActionState::<PlayerInput>::default(),
            ))
            .id();
        app.world_mut().spawn((
            PlayerId(bot_peer(1)),
            LobbyPlayer { lobby },
            PlayerCombat::default(),
            PlayerPose {
                translation: target_feet + Vec3::Y * EYE_HEIGHT,
                ..default()
            },
        ));
        (app, bot)
    }

    fn input(app: &App, bot: Entity) -> PlayerInput {
        app.world()
            .get::<ActionState<PlayerInput>>(bot)
            .unwrap()
            .0
            .clone()
    }

    #[test]
    fn a_veteran_that_can_see_its_target_turns_to_it_and_shoots() {
        // Open yard, target 30 m due east (+X), nothing in between.
        let (mut app, bot) = world(
            BotDifficulty::Veteran,
            Vec3::new(-30.0, 0.0, -40.0),
            Vec3::new(0.0, 0.0, -40.0),
        );
        let mut fired = 0;
        app.update(); // (the first update has no time delta)
        let mut last = input(&app, bot);
        for _ in 0..(6 * 64) {
            app.update();
            last = input(&app, bot);
            if last.fire {
                fired += 1;
                // A shot leaves from the eye, roughly toward +X.
                assert!(last.fire_dir[0] > 0.95, "shot went {:?}", last.fire_dir);
                assert_eq!(last.sound_bits & SND_SHOT, SND_SHOT);
            }
        }
        assert!(fired >= 1, "the veteran never fired in 6 s");
        // Facing east: yaw -90°.
        assert!(
            (last.yaw + core::f32::consts::FRAC_PI_2).abs() < 0.1,
            "yaw {}",
            last.yaw
        );
        // Fully scoped by now (it's been engaged for seconds).
        assert_eq!(last.ads_t, 1.0);
    }

    #[test]
    fn a_bots_recorded_first_person_feel_matches_a_real_players() {
        let (mut app, bot) = world(
            BotDifficulty::Veteran,
            Vec3::new(-30.0, 0.0, -40.0),
            Vec3::new(0.0, 0.0, -40.0),
        );
        app.update(); // (the first update has no time delta)
        let max_step = 1.0 / 64.0 / sniper_feel::ADS_SECS + 1e-4;
        let (mut prev_ads, mut sum_bits) = (0.0_f32, 0u16);
        let (mut saw_partial, mut saw_anim, mut saw_shake) = (false, false, false);
        for _ in 0..(8 * 64) {
            app.update();
            let i = input(&app, bot);
            // Never a snap: the scope moves at most one tick's worth per tick.
            assert!(
                (i.ads_t - prev_ads).abs() <= max_step,
                "ads jumped {prev_ads} -> {}",
                i.ads_t
            );
            prev_ads = i.ads_t;
            saw_partial |= i.ads_t > 0.05 && i.ads_t < 0.95;
            saw_anim |= i.anim_time > 0.0;
            saw_shake |= i.shake_trauma > 0.0 && i.shake_recoil > 0.0;
            sum_bits |= i.sound_bits;
        }
        assert!(saw_partial, "never saw a partly scoped frame");
        assert!(saw_anim, "no fire animation recorded");
        assert!(saw_shake, "no recoil / shake recorded");
        assert_eq!(sum_bits & SND_AIM_IN, SND_AIM_IN);
        assert_eq!(sum_bits & SND_RECHAMBER, SND_RECHAMBER);
    }

    #[test]
    fn a_recruit_is_slower_to_react_and_shoot_than_a_veteran() {
        let count = |d: BotDifficulty| {
            let (mut app, bot) = world(d, Vec3::new(-30.0, 0.0, -40.0), Vec3::new(0.0, 0.0, -40.0));
            let mut fired = 0;
            for _ in 0..(10 * 64) {
                app.update();
                fired += input(&app, bot).fire as u32;
            }
            fired
        };
        assert!(count(BotDifficulty::Veteran) > count(BotDifficulty::Recruit));
    }

    #[test]
    fn a_bot_with_a_wall_in_the_way_walks_toward_it_but_never_shoots_through_it() {
        // Bot just west of Break Point's tall wall at x ≈ 11.9 (running
        // z 24..60); the target stands right behind it at ground level.
        let (mut app, bot) = world(
            BotDifficulty::Veteran,
            Vec3::new(6.0, 0.0, 40.0),
            Vec3::new(17.0, 0.0, 40.0),
        );
        app.update(); // (the first update has no time delta)
        for _ in 0..(8 * 64) {
            app.update();
            let i = input(&app, bot);
            // (Once it has rounded the wall's south end it may see — and shoot
            // — the target; along the wall it never can.)
            assert!(!i.fire || i.translation[2] < 24.0, "shot through a wall from {:?}", i.translation);
            // (The wall ends at z = 24 — rounding that end, or past it, is fine.)
            assert!(
                i.translation[0] < 11.9 || i.translation[2] < 24.0,
                "walked through the wall to {:?}",
                i.translation
            );
        }
        // It did try to get there: it isn't still standing at its spawn.
        let t = input(&app, bot).translation;
        let moved = Vec3::new(t[0] - 6.0, 0.0, t[2] - 40.0).length();
        assert!(moved > 1.0, "never moved toward the target (at {t:?})");
    }
    #[test]
    fn a_bot_in_a_lobby_whose_match_is_ending_stands_still_and_fires_nothing() {
        let (mut app, bot) = world(
            BotDifficulty::Veteran,
            Vec3::new(-30.0, 0.0, -40.0),
            Vec3::new(0.0, 0.0, -40.0),
        );
        let lobby = app.world().get::<LobbyPlayer>(bot).unwrap().lobby;
        app.update(); // (the first update has no time delta)
        app.update();
        let start = input(&app, bot).translation;
        app.world_mut()
            .resource_mut::<crate::killcam::EndingLobbies>()
            .begin(lobby, 0, true);
        for _ in 0..(6 * 64) {
            app.update();
            let i = input(&app, bot);
            assert!(!i.fire, "fired during the end-of-match freeze");
        }
        let end = input(&app, bot).translation;
        assert!(
            Vec3::from_array(end).distance(Vec3::from_array(start)) < 0.5,
            "moved from {start:?} to {end:?}"
        );
    }

    #[test]
    fn a_dead_bot_stands_still_and_never_fires() {
        let (mut app, bot) = world(
            BotDifficulty::Veteran,
            Vec3::new(-30.0, 0.0, -40.0),
            Vec3::new(0.0, 0.0, -40.0),
        );
        app.world_mut().get_mut::<PlayerCombat>(bot).unwrap().alive = false;
        let start = input(&app, bot).translation;
        for _ in 0..(3 * 64) {
            app.update();
            let i = input(&app, bot);
            assert!(!i.fire);
            let _ = start;
        }
        let i = input(&app, bot);
        assert!((i.translation[0] - (-30.0)).abs() < 0.01, "a dead bot moved: {:?}", i.translation);
    }

    #[test]
    fn a_bot_in_a_lobby_that_has_not_started_does_nothing() {
        let (mut app, bot) = world(
            BotDifficulty::Veteran,
            Vec3::new(-30.0, 0.0, -40.0),
            Vec3::new(0.0, 0.0, -40.0),
        );
        let lobby = app.world().get::<LobbyPlayer>(bot).unwrap().lobby;
        app.world_mut().get_mut::<Lobby>(lobby).unwrap().started = false;
        for _ in 0..64 {
            app.update();
        }
        assert!(!input(&app, bot).fire);
        assert_eq!(input(&app, bot).translation, PlayerInput::default().translation);
    }
}
