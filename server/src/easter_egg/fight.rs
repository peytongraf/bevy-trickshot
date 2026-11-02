//! The easter egg's boss fight ([`EggStep::BossFight`]): the Hierophant.
//!
//! * The moment the fight's on (however it got there — he woke, or the
//!   debug panel jumped to it), he's in: standing on the sarcophagus where
//!   the wake-up left him (`shared::easter_egg::hierophant_start`), with
//!   [`shared::easter_egg::hierophant_health`] — a boss's body (its model,
//!   hitbox and smash) on a brain that never hurls blasts and hits harder
//!   (`ai::BotBrain::hierophant`). Who he is, and his phase, go on the lobby
//!   (`EasterEgg::hierophant`), for the clients.
//! * He's not one of the round's: no `boss::Boss`, so the round's boss
//!   counting and bringing far-off bosses back in leave him alone
//!   (`zombies::recycle_far_zombies` skips him), and nothing drops from him
//!   — the round's own spawns are held throughout
//!   (`shared::easter_egg::holds_spawns`).
//! * **Phase 1** — a heavy melee stalker. Every so often he stops to taunt,
//!   standing and glaring for [`shared::easter_egg::TAUNT_HOLD_SECS`].
//! * **Phase 2** (from [`shared::easter_egg::PHASE_2_AT`] of his health) —
//!   he stops and roars ("rise, my children"), raising a ring of the dead
//!   around him ([`Raised`]: worth points, dropping nothing), and another
//!   whenever the last is nearly gone (at most every
//!   [`shared::easter_egg::RING_EVERY_SECS`]). Every so often — or as soon
//!   as he's been left alone a while — he sinks into the ground and rises
//!   out of it near a player ([`shared::HierophantTeleport`]), nothing
//!   hurting him while he's under.
//! * **Phase 3** (from [`shared::easter_egg::PHASE_3_AT`]) — he roars again
//!   ("enough"), and comes on enraged: faster
//!   ([`shared::easter_egg::HIEROPHANT_ENRAGED_SPEED`]) and teleporting
//!   more often, his dead still rising. (The clients turn the map, and
//!   him, blood-red.)
//! * **His voice** ([`shared::HierophantSays`]): taunts, a gloat when he
//!   downs someone, a roar of pain at a big hit, his phase lines and his
//!   death — never one over another (bar the phase and death lines, which
//!   cut in).
//! * Killed, the fight's over ([`EggStep::HisDeath`], where he fell kept on
//!   `EasterEgg::hierophant` for his spirit): the dead he raised drop with
//!   him (no points). His corpse lies there like any zombie's; once his
//!   spirit's in the pyramid the round picks up where it left off
//!   (`run_unleashed`).
//! * The egg set anywhere but the fight (a debug jump, a reset, the game
//!   ending) takes him and his dead away; jumping to the fight again
//!   brings a fresh one in.
//! * The leader's debug panel can set his health (his phase following it),
//!   raise a ring or teleport him ([`shared::HierophantDebug`]).

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use lightyear::prelude::server::Server;
use lightyear::prelude::*;
use shared::bot_players::{bot_peer, is_bot_peer, BotDifficulty};
use shared::easter_egg::{
    EggStep, Hierophant as HierophantInfo, HierophantDebugAction, HierophantLine, TELEPORT_SINK_SECS, TELEPORT_UNDER_SECS,
};
use shared::{GameChannel, HierophantDebug, Lobby, PlayerId, PlayerPose};

use crate::ai::{BotBrain, NextBotId};
use crate::lobby::LobbyPlayer;
use crate::nav::NavGraphs;
use crate::pvp::{PlayerCombat, PlayerHit};
use crate::sim::EYE_HEIGHT;
use crate::zombies::Zombie;

/// On the Hierophant's zombie entity.
#[derive(Component)]
pub(crate) struct Hierophant;

/// On a zombie the Hierophant raised (phase 2).
#[derive(Component)]
pub(crate) struct Raised;

/// Each lobby's fight.
#[derive(Resource, Default)]
pub(super) struct Fights(HashMap<Entity, Fight>);

/// The debug panel's pokes, waiting for the next tick: (lobby, what).
#[derive(Resource, Default)]
pub(super) struct DebugPokes(Vec<(Entity, HierophantDebugAction)>);

/// His teleport under way: when it started (fight time), where he comes up
/// and facing which way, and whether he's been moved there yet.
struct Teleport {
    start: f32,
    to: Vec3,
    yaw: f32,
    moved: bool,
}

/// One lobby's fight — its times are how long (s, not counting pauses)
/// it's been on.
struct Fight {
    t: f32,
    seq: u32,
    /// Nothing of his starts (bar a phase or death line) until then.
    speaking_until: f32,
    next_taunt: f32,
    last_taunt: Option<u8>,
    /// His health over the last moment, for a big hit's roar of pain.
    health_log: Vec<(f32, f32)>,
    hurt_ready_at: f32,
    down_ready_at: f32,
    last_hurt: Option<u8>,
    last_down: Option<u8>,
    /// His phase-2 roar: when it began, and whether its ring's up yet.
    roar: Option<(f32, bool)>,
    last_ring_at: f32,
    next_teleport: f32,
    alone_for: f32,
    teleport: Option<Teleport>,
}

impl Fight {
    fn new() -> Self {
        Self {
            t: 0.0,
            seq: 0,
            speaking_until: 0.0,
            next_taunt: shared::easter_egg::TAUNT_FIRST_SECS,
            last_taunt: None,
            health_log: Vec::new(),
            hurt_ready_at: 0.0,
            down_ready_at: 0.0,
            last_hurt: None,
            last_down: None,
            roar: None,
            last_ring_at: f32::NEG_INFINITY,
            next_teleport: 0.0,
            alone_for: 0.0,
            teleport: None,
        }
    }

    /// A seeded roll in `0..1`, fresh each call.
    fn roll(&mut self, salt: u64) -> f32 {
        self.seq = self.seq.wrapping_add(1);
        shared::bots::rand01((self.seq as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ salt)
    }

    /// One of `count` lines, not `last` again.
    fn pick(&mut self, count: u8, last: Option<u8>, salt: u64) -> u8 {
        let mut i = ((self.roll(salt) * count as f32) as u8).min(count.saturating_sub(1));
        if count > 1 && last == Some(i) {
            i = (i + 1) % count;
        }
        i
    }

    /// A random gap (s) in `range`.
    fn gap(&mut self, range: (f32, f32), salt: u64) -> f32 {
        range.0 + self.roll(salt) * (range.1 - range.0)
    }
}

/// The gap (s) between his teleports in `phase` — shorter once he's
/// enraged.
fn teleport_gap(phase: u8) -> (f32, f32) {
    if phase >= 3 {
        shared::easter_egg::TELEPORT_GAP_ENRAGED_SECS
    } else {
        shared::easter_egg::TELEPORT_GAP_SECS
    }
}

type RaisedQuery<'w, 's> =
    Query<'w, 's, (&'static LobbyPlayer, &'static mut PlayerCombat), (With<Raised>, With<Zombie>, Without<Hierophant>)>;

/// His dead in `lobby_e` drop where they stand (no points, no drops).
fn drop_raised(lobby_e: Entity, raised: &mut RaisedQuery) {
    for (lp, mut combat) in raised.iter_mut() {
        if lp.lobby == lobby_e && combat.alive {
            combat.alive = false;
            combat.health = 0.0;
            combat.respawn_at = f32::INFINITY;
        }
    }
}

/// He says `line` — to every client, unless he's still speaking (a phase or
/// death line cuts in). Whether he did.
fn say(fight: &mut Fight, line: HierophantLine, sender: &mut ServerMultiMessageSender, server: &Server, lobby: &Lobby) -> bool {
    if !line.interrupts() && fight.t < fight.speaking_until {
        return false;
    }
    fight.speaking_until = fight.t + line.secs();
    send_to_lobby(sender, server, lobby, &shared::HierophantSays { line });
    true
}

fn send_to_lobby<M: lightyear::prelude::Message>(
    sender: &mut ServerMultiMessageSender,
    server: &Server,
    lobby: &Lobby,
    msg: &M,
) {
    if let Err(e) = sender.send::<_, GameChannel>(msg, server, &NetworkTarget::Only(lobby.real_peers())) {
        error!("failed to send a Hierophant message: {e:?}");
    }
}

/// Debug: the leader pokes the Hierophant — only in a fight that's on.
pub(super) fn on_hierophant_debug(
    trigger: Trigger<RemoteTrigger<HierophantDebug>>,
    lobbies: Query<(Entity, &Lobby)>,
    mut pokes: ResMut<DebugPokes>,
) {
    let peer = trigger.from;
    let Some((lobby_e, _)) = lobbies.iter().find(|(_, l)| {
        l.leader == peer && shared::easter_egg::available(l) && l.easter_egg.step == EggStep::BossFight
    }) else {
        return;
    };
    pokes.0.push((lobby_e, trigger.trigger.action));
}

/// Bring him in once the fight's on; run his phases, his dead, his
/// teleports and his voice; end the fight once he's dead — and take him
/// (and his dead) away if the egg's set anywhere else.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn run_fight(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    endings: Res<crate::killcam::EndingLobbies>,
    navs: Res<NavGraphs>,
    (mut next_id, mut fights, mut pokes): (ResMut<NextBotId>, ResMut<Fights>, ResMut<DebugPokes>),
    mut hits: EventReader<PlayerHit>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut him: Query<
        (Entity, &LobbyPlayer, &PlayerId, &mut PlayerCombat, &mut BotBrain),
        (With<Hierophant>, With<Zombie>, Without<Raised>),
    >,
    mut raised: RaisedQuery,
    players: Query<(&PlayerId, &PlayerPose, &LobbyPlayer, &PlayerCombat), Without<Zombie>>,
    mut commands: Commands,
) {
    let server = server.into_inner();
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let hits: Vec<(PeerId, PeerId)> = hits.read().map(|h| (h.killer, h.victim)).collect();
    let pokes: Vec<(Entity, HierophantDebugAction)> = std::mem::take(&mut pokes.0);
    fights.0.retain(|e, _| lobbies.contains(*e));

    for (lobby_e, mut lobby) in &mut lobbies {
        let fighting = shared::easter_egg::holds_spawns(&lobby) && lobby.easter_egg.step == EggStep::BossFight;
        if !fighting {
            fights.0.remove(&lobby_e);
            // (Still up — the egg's been set elsewhere: gone. A corpse lies
            // on for its death.)
            for (e, lp, _, combat, _) in &him {
                if lp.lobby == lobby_e && combat.alive {
                    commands.entity(e).try_despawn();
                }
            }
            drop_raised(lobby_e, &mut raised);
            // Set back before the fight: he's no one now.
            if lobby.easter_egg.step < EggStep::BossFight && lobby.easter_egg.hierophant.is_some() {
                lobby.easter_egg.hierophant = None;
            }
            continue;
        }
        if lobby.paused || endings.is_ending(lobby_e) {
            continue;
        }
        let current = lobby
            .easter_egg
            .hierophant
            .and_then(|h| him.iter().find(|(_, lp, id, ..)| lp.lobby == lobby_e && id.0 == h.peer).map(|(e, ..)| e));
        let Some(current) = current else {
            // Not in yet (or a fresh jump to the fight): any other of his
            // (and his dead) go, and he comes in.
            for (e, lp, ..) in &him {
                if lp.lobby == lobby_e {
                    commands.entity(e).try_despawn();
                }
            }
            drop_raised(lobby_e, &mut raised);
            let Some((feet, yaw)) = shared::easter_egg::hierophant_start(lobby.map) else {
                continue;
            };
            let peer = bot_peer(next_id.0);
            next_id.0 += 1;
            let seed = (now.to_bits() as u64) << 20 ^ next_id.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ lobby_e.to_bits() ^ 0x41e7;
            let mut skill = crate::zombies::zombie_skill(lobby.round.max(1));
            // He always knows where you are, and turns on you fast.
            skill.reaction_secs = skill.reaction_secs.min(0.3);
            skill.turn_deg_per_sec = skill.turn_deg_per_sec.max(170.0);
            skill.sight_range = skill.sight_range.max(80.0);
            let brain = BotBrain::new(BotDifficulty::Recruit, feet, seed)
                .facing(yaw)
                .with_skill(skill)
                .hierophant(now)
                .without_drops();
            let max = shared::easter_egg::hierophant_health(lobby.round, lobby.real_count());
            crate::boss::spawn_boss(&mut commands, lobby_e, &lobby, peer, "The Hierophant", feet, yaw, brain, max, max)
                .insert(Hierophant);
            lobby.easter_egg.hierophant = Some(HierophantInfo { peer, max_health: max, phase: 1, fell_at: None });
            fights.0.insert(lobby_e, Fight::new());
            info!("lobby {lobby_e:?}: the Hierophant's in the fight ({max:.0} health)");
            continue;
        };
        let Some(info) = lobby.easter_egg.hierophant else { continue };
        let Ok((_, _, _, mut combat, mut brain)) = him.get_mut(current) else { continue };
        let fight = fights.0.entry(lobby_e).or_insert_with(Fight::new);
        let salt = lobby_e.to_bits();

        if !combat.alive {
            say(fight, HierophantLine::Death, &mut sender, server, &lobby);
            fights.0.remove(&lobby_e);
            drop_raised(lobby_e, &mut raised);
            let fell_at = brain.feet();
            if let Some(h) = lobby.easter_egg.hierophant.as_mut() {
                h.fell_at = Some(fell_at);
            }
            lobby.easter_egg.step = EggStep::HisDeath;
            info!("lobby {lobby_e:?}: the Hierophant's dead — the fight's over");
            continue;
        }
        fight.t += dt;
        let t = fight.t;
        let max = info.max_health.max(1.0);

        // The debug panel's pokes.
        let mut force_ring = false;
        let mut force_teleport = false;
        for &(_, action) in pokes.iter().filter(|(e, _)| *e == lobby_e) {
            match action {
                HierophantDebugAction::SetHealth(share) => {
                    combat.health = (max * share.clamp(0.01, 1.0)).max(1.0);
                    fight.health_log.clear();
                    info!("lobby {lobby_e:?}: the Hierophant's health set to {:.0} (debug)", combat.health);
                }
                HierophantDebugAction::RaiseRing => force_ring = true,
                // (He falls next tick, as if shot.)
                HierophantDebugAction::Kill => {
                    combat.health = 0.0;
                    combat.alive = false;
                    combat.respawn_at = f32::INFINITY;
                    brain.untouchable_until(f32::NEG_INFINITY);
                }
                HierophantDebugAction::Teleport => force_teleport = true,
            }
        }
        if !combat.alive {
            continue;
        }

        // His phase follows his health (back up too — a debug set).
        let share = combat.health / max;
        let phase = shared::easter_egg::phase_at(share);
        if phase != info.phase {
            if let Some(h) = lobby.easter_egg.hierophant.as_mut() {
                h.phase = phase;
            }
            fight.teleport = None;
            brain.untouchable_until(f32::NEG_INFINITY);
            brain.enrage(phase >= 3);
            if phase > info.phase {
                // Phase 2 or 3: he stops and roars — the dead rising at
                // phase 2's (or phase 3's, if it skipped straight past 2).
                fight.roar = Some((t, info.phase >= 2));
                let gap = teleport_gap(phase);
                fight.next_teleport = t + shared::easter_egg::PHASE_ROAR_SECS + fight.gap(gap, salt ^ 1);
                fight.alone_for = 0.0;
                brain.hold_for(now, shared::easter_egg::PHASE_ROAR_SECS);
                let line = if phase >= 3 { HierophantLine::Phase3 } else { HierophantLine::Phase2 };
                say(fight, line, &mut sender, server, &lobby);
                info!("lobby {lobby_e:?}: the Hierophant's in phase {phase}");
            } else {
                // (Healed back by the debug panel.)
                fight.roar = None;
            }
        }

        // Living players in the game, and where their feet are.
        let living: Vec<(PeerId, Vec3)> = players
            .iter()
            .filter(|(id, _, lp, c)| lp.lobby == lobby_e && !is_bot_peer(id.0) && c.alive)
            .map(|(id, pose, ..)| (id.0, pose.translation - Vec3::Y * EYE_HEIGHT))
            .collect();
        let feet = brain.feet();
        let nav = navs.graph(lobby.map);

        // A ring of the dead around him: at his roar, whenever the last's
        // nearly gone — or the debug panel's.
        let mut raise = false;
        if let Some((start, risen)) = fight.roar.as_mut() {
            if !*risen && t - *start >= shared::easter_egg::RING_RISE_AT_SECS {
                *risen = true;
                raise = true;
            }
            if t - *start >= shared::easter_egg::PHASE_ROAR_SECS {
                fight.roar = None;
            }
        } else if phase >= 2 && fight.teleport.is_none() && t - fight.last_ring_at >= shared::easter_egg::RING_EVERY_SECS {
            let left = raised.iter().filter(|(lp, c)| lp.lobby == lobby_e && c.alive).count();
            if left <= shared::easter_egg::RING_REFILL_LEFT && !brain.attacking() {
                raise = true;
                brain.hold_for(now, 1.5);
            }
        }
        if raise || force_ring {
            fight.last_ring_at = t;
            let n = shared::easter_egg::ring_size(lobby.real_count());
            let round = lobby.round.max(1);
            let turn = fight.roll(salt ^ 2) * std::f32::consts::TAU;
            let mut up = 0;
            for i in 0..n {
                let a = turn + i as f32 / n as f32 * std::f32::consts::TAU;
                let (lo, hi) = shared::easter_egg::RING_RADIUS;
                let r = lo + fight.roll(salt ^ 3) * (hi - lo);
                let dir = Vec3::new(a.cos(), 0.0, a.sin());
                let want = feet + dir * r;
                let Some(spot) = nav
                    .nearest_spot(want)
                    .filter(|s| (*s - want).with_y(0.0).length() < 2.5 && (s.y - feet.y).abs() < 1.5)
                else {
                    continue;
                };
                let seed = (fight.roll(salt ^ 4) * u32::MAX as f32) as u64 ^ (i as u64) << 40 ^ salt;
                let e = crate::zombies::spawn_zombie(
                    &mut commands,
                    &mut next_id,
                    lobby_e,
                    &lobby,
                    (round, round),
                    spot,
                    spot + dir * 10.0,
                    None,
                    seed,
                    now,
                );
                commands
                    .entity(e)
                    .insert(Raised)
                    .entry::<BotBrain>()
                    .and_modify(|mut b| b.set_drops(false));
                up += 1;
            }
            send_to_lobby(&mut sender, server, &lobby, &shared::HierophantRaise { at: feet.to_array() });
            info!("lobby {lobby_e:?}: the Hierophant raised {up} of the dead");
        }

        // His teleport: sink, gone, rise near a player.
        let nearest = living.iter().map(|(_, p)| p.distance(feet)).fold(f32::INFINITY, f32::min);
        if nearest > shared::easter_egg::TELEPORT_ALONE_DIST {
            fight.alone_for += dt;
        } else {
            fight.alone_for = 0.0;
        }
        let total = shared::easter_egg::teleport_secs();
        if let Some(tp) = fight.teleport.as_mut() {
            let into = t - tp.start;
            // (Kept on each tick, so a pause can't let either run out early.)
            brain.hold_for(now, (total - into).max(0.0));
            brain.untouchable_until(now + (total - into).max(0.0));
            if !tp.moved && into >= TELEPORT_SINK_SECS + TELEPORT_UNDER_SECS * 0.5 {
                tp.moved = true;
                brain.move_to(tp.to, tp.yaw);
            }
            if into >= total {
                fight.teleport = None;
                brain.untouchable_until(f32::NEG_INFINITY);
                fight.alone_for = 0.0;
                fight.next_teleport = t + fight.gap(teleport_gap(phase), salt ^ 5);
            }
        } else {
            let due = phase >= 2
                && fight.roar.is_none()
                && (t >= fight.next_teleport || fight.alone_for >= shared::easter_egg::TELEPORT_ALONE_SECS);
            if (due || force_teleport) && !brain.attacking() && !living.is_empty() {
                // Near whoever he's after (or anyone), on their floor.
                let pick = brain
                    .target()
                    .and_then(|p| living.iter().find(|(id, _)| *id == p))
                    .copied()
                    .unwrap_or_else(|| {
                        let i = ((fight.roll(salt ^ 6) * living.len() as f32) as usize).min(living.len() - 1);
                        living[i]
                    });
                let at = pick.1;
                let seed = (fight.roll(salt ^ 7) * u32::MAX as f32) as u64 ^ salt;
                let (lo, hi) = shared::easter_egg::TELEPORT_NEAR;
                let spot = nav.random_spot_near_where(at, lo, hi, seed, |n| (nav.node(n).y - at.y).abs() < 1.5);
                match spot {
                    Some(to) => {
                        let face = at - to;
                        let yaw = f32::atan2(-face.x, -face.z);
                        fight.teleport = Some(Teleport { start: t, to, yaw, moved: false });
                        brain.hold_for(now, total);
                        brain.untouchable_until(now + total);
                        send_to_lobby(
                            &mut sender,
                            server,
                            &lobby,
                            &shared::HierophantTeleport { from: feet.to_array(), to: to.to_array(), yaw },
                        );
                    }
                    // (Nowhere this time: in a moment.)
                    None => fight.next_teleport = t + 1.0,
                }
            }
        }
        let busy = fight.roar.is_some() || fight.teleport.is_some();

        // He's downed someone: a gloat.
        let downed = hits.iter().any(|(killer, victim)| {
            *killer == info.peer
                && players
                    .iter()
                    .any(|(id, _, lp, c)| id.0 == *victim && lp.lobby == lobby_e && c.down.is_some())
        });
        if downed && t >= fight.down_ready_at {
            let i = fight.pick(shared::easter_egg::PLAYER_DOWN_LINES, fight.last_down, salt ^ 8);
            if say(fight, HierophantLine::PlayerDown(i), &mut sender, server, &lobby) {
                fight.last_down = Some(i);
                fight.down_ready_at = t + shared::easter_egg::PLAYER_DOWN_COOLDOWN_SECS;
            }
        }

        // A big hit: a roar of pain.
        fight.health_log.push((t, combat.health));
        fight.health_log.retain(|(at, _)| t - at <= shared::easter_egg::HURT_WINDOW_SECS);
        let lost = fight.health_log.first().map_or(0.0, |(_, h)| h - combat.health);
        if lost >= max * shared::easter_egg::HURT_SHARE && t >= fight.hurt_ready_at {
            let i = fight.pick(shared::easter_egg::HURT_LINES, fight.last_hurt, salt ^ 9);
            if say(fight, HierophantLine::Hurt(i), &mut sender, server, &lobby) {
                fight.last_hurt = Some(i);
                fight.hurt_ready_at = t + shared::easter_egg::HURT_COOLDOWN_SECS;
                fight.health_log.clear();
            }
        }

        // Now and then, a taunt — standing still to say it.
        if t >= fight.next_taunt && !busy {
            if t < fight.speaking_until || !brain.taunt(now, shared::easter_egg::TAUNT_HOLD_SECS) {
                // (Mid-line, mid-smash or stunned: in a moment.)
                fight.next_taunt = t + 0.5;
            } else {
                let i = fight.pick(shared::easter_egg::TAUNTS, fight.last_taunt, salt ^ 10);
                say(fight, HierophantLine::Taunt(i), &mut sender, server, &lobby);
                fight.last_taunt = Some(i);
                fight.next_taunt = t + fight.gap(shared::easter_egg::TAUNT_GAP_SECS, salt ^ 11);
            }
        }
    }
}
