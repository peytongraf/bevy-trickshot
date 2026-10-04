//! Player health, death and respawn — the server owns all of it. Shots
//! (`FreeForAll` PvP) and falls (both modes) take health off
//! [`PlayerCombat::health`]; it's replicated to the clients as
//! [`shared::PlayerHealth`] for the damage overlay and heartbeat, holds for a
//! few seconds after damage and then regenerates. This module also owns the
//! `FreeForAll` kill-limit win condition. Bots (`crate::bots`) and style-point
//! scoring (`shared::scoring`) are `Freestyle`-only and untouched by any of
//! this.

use bevy::prelude::*;

use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{
    FallDeath, FallLanded, FellToDeath, GameChannel, GameMode, HitMarker, Lobby, PlayerHealth,
    PlayerId, PlayerKilledBy, PlayerPose, PlayerRespawn, RespawnReady, ScoreLine, TrickScore,
    ZOMBIE_CRITICAL_POINTS, ZOMBIE_KILL_POINTS, ZombieDamaged,
};

use lightyear::prelude::input::native::ActionState;
use shared::bot_players::is_bot_peer;
use shared::weapon::LOADOUT_SWAP_GRACE_SECS;
use shared::PlayerInput;
use shared::health::{
    fall_damage, FULL_HEALTH,
};

/// Seconds a dead player stays untargetable / unable to fire before they can
/// be hit again — generous enough to outlast the kill-cam the victim's own
/// client plays (buffered ~1.5 s post-kill, then up to ~4.5 s of replay; see
/// `server::killcam`).
const RESPAWN_DELAY_SECS: f32 = 4.5;

/// Seconds a player who died *falling* (no kill cam) stays dead — just past
/// the client's own wait before it respawns them
/// (`client::net::RESPAWN_KILLCAM_TIMEOUT_SECS`, 3 s), so they're never
/// respawned-but-still-dead for long.
const FALL_RESPAWN_DELAY_SECS: f32 = 3.2;

/// Per-player combat state, on every player in a started game (see
/// `lobby::on_start`) in either mode.
#[derive(Component)]
pub struct PlayerCombat {
    pub health: f32,
    /// What `health` starts at: a player's [`FULL_HEALTH`], or a `Zombies`
    /// zombie's for its round.
    max_health: f32,
    /// Whether `health` climbs back after a hit — players' does, a zombie's
    /// doesn't (every shot counts toward bringing it down).
    regenerates: bool,
    /// `Time::elapsed_secs()` of the last damage taken — health holds for
    /// [`REGEN_DELAY_SECS`] after it, then recovers (see [`tick_respawns`]).
    last_damage: f32,
    pub alive: bool,
    /// `Time::elapsed_secs()` this player becomes targetable / can fire again.
    /// Meaningless while `alive`.
    pub(crate) respawn_at: f32,
    /// `Time::elapsed_secs()` this life began (held at "now" while the
    /// match is still loading, pushed back by a pause), and whether they've
    /// fired since — together, whether a `FreeForAll` loadout change still
    /// swaps their gun on the spot ([`Self::can_swap_loadout`]).
    life_started: f32,
    fired: bool,
    /// `Time::elapsed_secs()` this player's next PhD Flopper explosion may
    /// go off ([`Self::try_phd_blast`]).
    phd_ready_at: f32,
    /// `Zombies`: down and bleeding out (`crate::revive`) — `alive` is
    /// `false` meanwhile, so nothing targets them and they can't attack.
    pub(crate) down: Option<crate::revive::LastStand>,
    /// `Zombies`: bled out (or fell out of the world) in this round — dead
    /// until the next one starts (`crate::revive`).
    pub(crate) bled_out: Option<u32>,
}

impl PlayerCombat {
    /// A fresh life starting at `now`.
    pub fn spawned(now: f32) -> Self {
        Self {
            life_started: now,
            ..default()
        }
    }

    /// Back to life: full health, targetable, no damage history — everything a
    /// respawn resets. Shared by the respawn timer and the client's
    /// `RespawnReady`.
    pub(crate) fn respawn(&mut self, now: f32) {
        *self = Self::spawned(now);
    }

    /// Whether a PhD Flopper explosion may go off now (alive, and past its
    /// [`shared::perks::PHD_COOLDOWN_SECS`]) — starting the cooldown if so.
    fn try_phd_blast(&mut self, now: f32) -> bool {
        if !self.alive || now < self.phd_ready_at {
            return false;
        }
        self.phd_ready_at = now + shared::perks::PHD_COOLDOWN_SECS;
        true
    }

    /// Call of Duty's class-swap grace: alive, spawned no more than
    /// [`LOADOUT_SWAP_GRACE_SECS`] ago, and not fired yet.
    pub fn can_swap_loadout(&self, now: f32) -> bool {
        self.alive && !self.fired && now - self.life_started <= LOADOUT_SWAP_GRACE_SECS
    }

    /// `Zombies`: health's run out — go down instead of dying, holding
    /// `perks` (purchase order). Playing `solo` with Quick Revive they'll
    /// get themselves back up (`crate::revive`).
    pub(crate) fn go_down(&mut self, perks: Vec<shared::perks::Perk>, solo: bool) {
        self.alive = false;
        self.health = 0.0;
        self.respawn_at = f32::INFINITY;
        self.down = Some(crate::revive::LastStand::new(perks, solo));
    }

    /// `Zombies`: back up from a last stand — full health, targetable.
    pub(crate) fn revive(&mut self) {
        self.alive = true;
        self.health = self.max_health;
        self.last_damage = f32::NEG_INFINITY;
        self.respawn_at = 0.0;
        self.down = None;
    }

    /// `Zombies`: dead for the rest of `round` (bled out, or fell out of the
    /// world) — back when the next one starts (`crate::revive`).
    pub(crate) fn bleed_out(&mut self, round: u32) {
        self.alive = false;
        self.health = 0.0;
        self.respawn_at = f32::INFINITY;
        self.down = None;
        self.bled_out = Some(round);
    }

    /// A `Zombies` zombie with `health` (for its round), which never
    /// regenerates.
    pub fn zombie(health: f32) -> Self {
        Self {
            health,
            max_health: health,
            regenerates: false,
            ..default()
        }
    }
}

impl Default for PlayerCombat {
    fn default() -> Self {
        Self {
            health: FULL_HEALTH,
            max_health: FULL_HEALTH,
            regenerates: true,
            last_damage: f32::NEG_INFINITY,
            alive: true,
            respawn_at: 0.0,
            life_started: 0.0,
            fired: false,
            phd_ready_at: 0.0,
            down: None,
            bled_out: None,
        }
    }
}

/// A `FreeForAll` player took damage — written by
/// [`crate::sim::resolve_shots`]. A fatal hit additionally fires
/// [`PlayerKilled`] for [`crate::killcam::queue_killcams`] to pick up.
#[derive(Event)]
pub struct PlayerHit {
    pub victim: PeerId,
    pub killer: PeerId,
    pub damage: f32,
    /// A Bomb Shot trickshot ([`shared::perks::is_trickshot`] by a perk
    /// owner): if it kills a zombie, the zombie explodes ([`BombBlast`]).
    pub bomb_shot: bool,
    /// Damage from a [`BombBlast`] itself — never sets off another one, even
    /// with the lobby's debug `bomb_test` on.
    pub blast: bool,
    /// A headshot or a knife stab: a `Zombies` zombie killed by one scores
    /// its killer [`ZOMBIE_CRITICAL_POINTS`] on top of the kill.
    pub critical: bool,
    /// Where the hit landed, when it landed somewhere in particular (a shot,
    /// a stab, a thrown knife) — where a `Zombies` damage number floats up
    /// from ([`shared::ZombieDamaged`]). `None`: the middle of the body.
    pub point: Option<Vec3>,
}

/// A Bomb Shot went off in `lobby` with its base at `feet`, set off by `by`
/// — written by [`apply_player_hits`] when a zombie dies to one, consumed by
/// [`apply_bomb_blasts`]. `phd` marks PhD Flopper's instead (a slide into an
/// enemy, [`on_phd_slam`], or a big drop, [`on_fall_landed`]) — the same
/// damage, drawn purple.
#[derive(Event)]
pub struct BombBlast {
    pub lobby: Entity,
    pub feet: Vec3,
    pub by: PeerId,
    pub phd: bool,
}

/// A player killed a `Zombies` zombie standing at `feet` in `lobby` —
/// written by [`apply_player_hits`], consumed by `crate::power_ups` to roll
/// for a drop. (A Nuke's kills don't go through here, so they never drop.)
#[derive(Event)]
pub struct ZombieKilled {
    pub lobby: Entity,
    pub feet: Vec3,
}

/// A `FreeForAll` kill — the PvP counterpart of [`crate::bots::BotHit`].
/// Written by [`apply_player_hits`], consumed by
/// [`crate::killcam::queue_killcams`] to queue the victim-only replay.
#[derive(Event)]
pub struct PlayerKilled {
    pub victim: PeerId,
    pub killer: PeerId,
}

pub struct PvpPlugin;

impl Plugin for PvpPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<PlayerHit>()
            .add_event::<BombBlast>()
            .add_event::<PlayerKilled>()
            .add_event::<ZombieKilled>()
            .add_observer(on_fell_to_death)
            .add_observer(on_fall_landed)
            .add_observer(on_respawn_ready)
            .add_observer(on_phd_slam)
            .add_systems(
                FixedUpdate,
                (
                    apply_player_hits,
                    apply_bomb_blasts,
                    track_lives,
                    tick_respawns,
                    sync_health,
                    check_kill_limit,
                )
                    .chain(),
            );
    }
}

/// How high (m) above a zombie's feet a hit with no particular spot (fire,
/// a blast) shows its damage number — about mid-chest.
const ZOMBIE_DAMAGE_CENTER_Y: f32 = 1.1;

/// Apply queued damage; a fatal hit marks the victim dead, starts their
/// respawn timer, credits the killer's kill count, and tells the victim
/// where they'll reappear.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_player_hits(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut hits: EventReader<PlayerHit>,
    mut killed: EventWriter<PlayerKilled>,
    mut zombie_kills: EventWriter<ZombieKilled>,
    mut blasts: EventWriter<BombBlast>,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    endings: Res<crate::killcam::EndingLobbies>,
) {
    let server = server.into_inner();
    for ev in hits.read() {
        // (A `Zombies` zombie isn't a lobby member — its lobby is its victim's
        // or its killer's.)
        let in_lobby = |l: &Lobby| l.has(ev.victim) || l.has(ev.killer);
        // The match is over: nothing counts (see `EndingLobbies`).
        if lobbies
            .iter()
            .any(|(e, l)| in_lobby(l) && (endings.is_frozen(e) || endings.is_ending(e) && l.mode == GameMode::Zombies))
        {
            continue;
        }
        let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == ev.victim) else {
            continue;
        };
        if !combat.alive {
            continue;
        }
        // Liquid Courage (a `Zombies` perk) softens every hit.
        let perks: &[shared::perks::Perk] = lobbies
            .iter()
            .find_map(|(_, l)| l.members.iter().find(|m| m.peer == ev.victim))
            .map_or(&[], |m| m.perks.as_slice());
        // Insta-Kill (a `Zombies` power-up): a player's hit on a zombie
        // always kills.
        let insta = is_bot_peer(ev.victim)
            && !is_bot_peer(ev.killer)
            && lobbies.iter().any(|(_, l)| {
                in_lobby(l)
                    && l.mode == GameMode::Zombies
                    && l.power_up_active(shared::power_ups::PowerUp::InstaKill)
            });
        let health_before = combat.health.max(0.0);
        combat.health -= if insta {
            health_before + 1.0
        } else {
            shared::perks::damage_taken(perks, ev.damage)
        };
        combat.last_damage = time.elapsed_secs();
        // `Zombies`: a player's hit on a zombie floats its damage number up
        // on their screen (an Insta-Kill's being whatever health it took).
        let zombie_hit = is_bot_peer(ev.victim)
            && !is_bot_peer(ev.killer)
            && lobbies.iter().any(|(_, l)| in_lobby(l) && l.mode == GameMode::Zombies);
        if zombie_hit {
            let point = ev.point.or_else(|| {
                poses.iter().find(|(id, _)| id.0 == ev.victim).map(|(_, pose)| {
                    pose.translation - Vec3::Y * (crate::sim::EYE_HEIGHT - ZOMBIE_DAMAGE_CENTER_Y)
                })
            });
            let damage = if insta { health_before } else { ev.damage };
            if let Some(point) = point {
                if let Err(e) = sender.send::<_, GameChannel>(
                    &ZombieDamaged {
                        point: point.to_array(),
                        damage: damage.round().max(1.0) as u32,
                        critical: ev.critical,
                    },
                    server,
                    &NetworkTarget::Single(ev.killer),
                ) {
                    error!("failed to send zombie damage to {:?}: {e:?}", ev.killer);
                }
            }
        }
        if combat.health > 0.0 {
            // Hurt but alive: the shooter gets a hit marker (a bot shooter has
            // no client to show it to, and a player burnt by their own
            // molotov gets none).
            if !is_bot_peer(ev.killer) && ev.killer != ev.victim {
                if let Err(e) = sender.send::<_, GameChannel>(
                    &HitMarker {
                        kill: false,
                        victim: Some(ev.victim),
                    },
                    server,
                    &NetworkTarget::Single(ev.killer),
                ) {
                    error!("failed to send hit marker to {:?}: {e:?}", ev.killer);
                }
            }
            continue;
        }
        combat.alive = false;
        combat.respawn_at = time.elapsed_secs() + RESPAWN_DELAY_SECS;
        // The killer's red kill marker (a bot killer has no client).
        if !is_bot_peer(ev.killer) && ev.killer != ev.victim {
            if let Err(e) = sender.send::<_, GameChannel>(
                &HitMarker {
                    kill: true,
                    victim: Some(ev.victim),
                },
                server,
                &NetworkTarget::Single(ev.killer),
            ) {
                error!("failed to send kill marker to {:?}: {e:?}", ev.killer);
            }
        }

        let Some((lobby_e, mut lobby)) = lobbies.iter_mut().find(|(_, l)| in_lobby(l)) else {
            continue;
        };

        // `Zombies`: nobody respawns. A dead zombie scores its killer and is
        // cleared away by `crate::zombies`; a *player* goes down instead, for
        // a teammate to revive (`crate::revive`, which also ends the game
        // once everyone's down). No kill cams either way.
        if lobby.mode == GameMode::Zombies {
            combat.respawn_at = f32::INFINITY;
            // Bomb Shot (or the debug `bomb_test`, for any player's kill):
            // the zombie explodes. A blast's own kills never chain.
            if is_bot_peer(ev.victim)
                && !is_bot_peer(ev.killer)
                && !ev.blast
                && (ev.bomb_shot || lobby.bomb_test)
            {
                if let Some((_, pose)) = poses.iter().find(|(id, _)| id.0 == ev.victim) {
                    blasts.write(BombBlast {
                        lobby: lobby_e,
                        feet: pose.translation - Vec3::Y * crate::sim::EYE_HEIGHT,
                        by: ev.killer,
                        phd: false,
                    });
                }
            }
            if is_bot_peer(ev.victim) {
                // Double Points (a `Zombies` power-up) doubles the kill —
                // and the critical bonus (a headshot or knife kill).
                let mult = if lobby.power_up_active(shared::power_ups::PowerUp::DoublePoints) {
                    2
                } else {
                    1
                };
                let mut lines = vec![ScoreLine {
                    label: "KILL".into(),
                    points: ZOMBIE_KILL_POINTS * mult,
                }];
                if ev.critical {
                    lines.push(ScoreLine {
                        label: "CRITICAL".into(),
                        points: ZOMBIE_CRITICAL_POINTS * mult,
                    });
                }
                let points: u32 = lines.iter().map(|l| l.points).sum();
                if !is_bot_peer(ev.killer) {
                    if let Some((_, pose)) = poses.iter().find(|(id, _)| id.0 == ev.victim) {
                        zombie_kills.write(ZombieKilled {
                            lobby: lobby_e,
                            feet: pose.translation - Vec3::Y * crate::sim::EYE_HEIGHT,
                        });
                    }
                }
                if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == ev.killer) {
                    m.score += points;
                    m.kills += 1;
                    if ev.critical {
                        m.critical_kills += 1;
                    }
                    let trick = TrickScore {
                        shooter: ev.killer,
                        total: points,
                        lines,
                    };
                    if let Err(e) = sender.send::<_, GameChannel>(
                        &trick,
                        server,
                        &NetworkTarget::Single(ev.killer),
                    ) {
                        error!("failed to send zombie kill score: {e:?}");
                    }
                }
            } else {
                let perks = lobby
                    .members
                    .iter()
                    .find(|m| m.peer == ev.victim)
                    .map(|m| m.perks.clone())
                    .unwrap_or_default();
                combat.go_down(perks, lobby.real_count() == 1);
                if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == ev.victim) {
                    m.downs += 1;
                }
                info!("{:?} is down", ev.victim);
            }
            continue;
        }
        if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == ev.killer) {
            m.score += 1;
        }

        let others: Vec<Vec3> = poses
            .iter()
            .filter(|(id, _)| id.0 != ev.victim && lobby.has(id.0))
            .map(|(_, pose)| pose.translation)
            .collect();
        let seed = time.elapsed().as_nanos() as u64 ^ ev.victim.to_bits();
        let (pos, yaw) = shared::spawns::spawn_point(seed, &others, lobby.map);

        // A bot victim has no client: nothing to tell, and `ai::drive_bots`
        // respawns it itself when its timer is up.
        let victim_is_bot = is_bot_peer(ev.victim);

        // Sent immediately, well ahead of the kill cam (buffered ~1.5s server-side —
        // see `PlayerKilledBy`'s doc comment) so the victim's own death effect can
        // snap their view toward the killer right away.
        if let Some((_, killer_pose)) = poses
            .iter()
            .find(|(id, _)| id.0 == ev.killer && !victim_is_bot)
        {
            let killed_by = PlayerKilledBy {
                killer_pos: killer_pose.translation.to_array(),
            };
            if let Err(e) =
                sender.send::<_, GameChannel>(&killed_by, server, &NetworkTarget::Single(ev.victim))
            {
                error!("failed to send killed-by to {:?}: {e:?}", ev.victim);
            }
        }

        let msg = PlayerRespawn {
            pos: pos.to_array(),
            yaw,
            immediate: false,
        };
        if !victim_is_bot {
            if let Err(e) =
                sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(ev.victim))
            {
                error!("failed to send respawn to {:?}: {e:?}", ev.victim);
            }
        }

        info!("{:?} killed {:?}", ev.killer, ev.victim);
        killed.write(PlayerKilled {
            victim: ev.victim,
            killer: ev.killer,
        });
    }
}

/// Set off each [`BombBlast`]: show every real player in the lobby the
/// explosion and have them hear it (all members, not just the shooter), then hurt every living zombie in range by how close it is
/// ([`shared::perks::bomb_shot_damage`]) — through [`PlayerHit`], so kills
/// score, count and give hit / kill markers like any other (next tick).
fn apply_bomb_blasts(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut blasts: EventReader<BombBlast>,
    mut hits: EventWriter<PlayerHit>,
    lobbies: Query<&Lobby>,
    mut zombies: Query<(
        &PlayerId,
        &PlayerPose,
        &PlayerCombat,
        &crate::lobby::LobbyPlayer,
        Option<&mut crate::ai::BotBrain>,
    )>,
) {
    let server = server.into_inner();
    let now = time.elapsed_secs();
    for blast in blasts.read() {
        let Ok(lobby) = lobbies.get(blast.lobby) else {
            continue;
        };
        let variant = ((time.elapsed().as_nanos() as u64) ^ blast.by.to_bits())
            .wrapping_mul(0x2545_F491_4F6C_DD1D)
            >> 56;
        let msg = shared::BombExplosion {
            feet: blast.feet.to_array(),
            variant: variant as u8,
            phd: blast.phd,
        };
        if let Err(e) =
            sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(lobby.real_peers()))
        {
            error!("failed to send bomb explosion: {e:?}");
        }
        let mut n = 0;
        for (id, pose, combat, lp, brain) in &mut zombies {
            if !is_bot_peer(id.0) || !combat.alive || lp.lobby != blast.lobby {
                continue;
            }
            let feet = pose.translation - Vec3::Y * crate::sim::EYE_HEIGHT;
            let damage = shared::perks::bomb_shot_damage(feet.distance(blast.feet));
            if damage <= 0.0 {
                continue;
            }
            // PhD Flopper's blast stuns whatever it doesn't kill (a kill
            // makes the stun moot).
            if blast.phd {
                if let Some(mut brain) = brain {
                    brain.stun(now + shared::perks::PHD_STUN_SECS);
                }
            }
            n += 1;
            hits.write(PlayerHit {
                victim: id.0,
                killer: blast.by,
                damage,
                bomb_shot: false,
                blast: true,
                critical: false,
                point: None,
            });
        }
        let what = if blast.phd { "PhD Flopper blast" } else { "bomb shot" };
        info!("{:?}'s {what} went off, catching {n} zombies", blast.by);
    }
}

/// The local player's client reported falling out of the world (below the
/// void floor — see `client::fall_death`; movement is client-authoritative,
/// same trust model as `PlayerInput`, see `sim::apply_client_pose`). There's no
/// height to grade, so it's a straight kill — see [`fall_kill`].
fn on_fell_to_death(
    trigger: Trigger<RemoteTrigger<FellToDeath>>,
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
) {
    fall_kill(
        trigger.from,
        None,
        &time,
        server.into_inner(),
        &mut sender,
        &mut combats,
        &poses,
        &mut lobbies,
    );
    info!("{:?} fell out of the world", trigger.from);
}

/// The local player's client reported landing after a fall of some height
/// ([`shared::FallLanded`]). The *server* turns the distance into damage
/// ([`fall_damage`]): under the minimum nothing happens; over it, health drops
/// (and is replicated back for the damage overlay + heartbeat); if that kills,
/// [`fall_kill`] runs and the victim is told to play the fall-death effect.
/// Works in both game modes — every player has a [`PlayerCombat`].
#[allow(clippy::too_many_arguments)]
fn on_fall_landed(
    trigger: Trigger<RemoteTrigger<FallLanded>>,
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    (endings, mut blasts): (Res<crate::killcam::EndingLobbies>, EventWriter<BombBlast>),
) {
    let peer = trigger.from;
    let landed = trigger.trigger;
    // Liquid Courage and Kangabrew (`Zombies` perks) soften falls; PhD
    // Flopper takes none at all, and a big enough drop explodes.
    let (lobby_e, perks): (Option<Entity>, &[shared::perks::Perk]) = lobbies
        .iter()
        .find_map(|(e, l)| {
            l.members
                .iter()
                .find(|m| m.peer == peer)
                .map(|m| ((l.started && l.mode == GameMode::Zombies && !l.paused).then_some(e), m.perks.as_slice()))
        })
        .unwrap_or((None, &[]));
    if let Some(lobby_e) = lobby_e.filter(|e| !endings.is_ending(*e)) {
        if perks.contains(&shared::perks::Perk::PhdFlopper)
            && landed.distance >= shared::perks::PHD_DROP_MIN_DISTANCE
        {
            let feet = poses
                .iter()
                .find(|(id, _)| id.0 == peer)
                .map(|(_, pose)| pose.translation - Vec3::Y * crate::sim::EYE_HEIGHT);
            let ready = combats
                .iter_mut()
                .find(|(id, _)| id.0 == peer)
                .is_some_and(|(_, mut c)| c.try_phd_blast(time.elapsed_secs()));
            if let (Some(feet), true) = (feet, ready) {
                blasts.write(BombBlast {
                    lobby: lobby_e,
                    feet,
                    by: peer,
                    phd: true,
                });
            }
        }
    }
    let damage = shared::perks::fall_damage_taken(perks, fall_damage(landed.distance));
    if damage <= 0.0 {
        return;
    }
    let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == peer) else {
        return;
    };
    if !combat.alive {
        return;
    }
    combat.health = (combat.health - damage).max(0.0);
    combat.last_damage = time.elapsed_secs();
    let dead = combat.health <= 0.0;
    drop(combat);
    if dead {
        fall_kill(
            peer,
            Some(landed.speed),
            &time,
            server.into_inner(),
            &mut sender,
            &mut combats,
            &poses,
            &mut lobbies,
        );
        info!("{peer:?} died from a {:.1} m fall", landed.distance);
    } else {
        info!("{peer:?} took {damage:.0} fall damage");
    }
}

/// A PhD Flopper owner slid into an enemy ([`shared::PhdSlam`]): if they
/// really have the perk in a running, unpaused `Zombies` game, its cooldown's
/// up, and a live zombie is close to where the server has them, it explodes
/// at their feet.
fn on_phd_slam(
    trigger: Trigger<RemoteTrigger<shared::PhdSlam>>,
    time: Res<Time>,
    endings: Res<crate::killcam::EndingLobbies>,
    lobbies: Query<(Entity, &Lobby)>,
    mut combats: Query<(&PlayerId, &mut PlayerCombat)>,
    poses: Query<(&PlayerId, &PlayerPose, &crate::lobby::LobbyPlayer)>,
    mut blasts: EventWriter<BombBlast>,
) {
    let peer = trigger.from;
    let Some((lobby_e, _)) = lobbies.iter().find(|(e, l)| {
        l.started
            && l.mode == GameMode::Zombies
            && !l.paused
            && !endings.is_ending(*e)
            && l.members
                .iter()
                .any(|m| m.peer == peer && m.perks.contains(&shared::perks::Perk::PhdFlopper))
    }) else {
        return;
    };
    let Some(feet) = poses
        .iter()
        .find(|(id, ..)| id.0 == peer)
        .map(|(_, pose, _)| pose.translation - Vec3::Y * crate::sim::EYE_HEIGHT)
    else {
        return;
    };
    let alive = |p: PeerId| combats.iter().any(|(id, c)| id.0 == p && c.alive);
    let enemy_close = poses.iter().any(|(id, pose, lp)| {
        lp.lobby == lobby_e
            && is_bot_peer(id.0)
            && alive(id.0)
            && (pose.translation - Vec3::Y * crate::sim::EYE_HEIGHT).distance(feet)
                <= shared::perks::PHD_SLAM_SERVER_RADIUS
    });
    if !enemy_close {
        return;
    }
    let now = time.elapsed_secs();
    let ready = combats
        .iter_mut()
        .find(|(id, _)| id.0 == peer)
        .is_some_and(|(_, mut c)| c.try_phd_blast(now));
    if ready {
        blasts.write(BombBlast {
            lobby: lobby_e,
            feet,
            by: peer,
            phd: true,
        });
    }
}

/// Kill `peer` from a fall (no killer, so no kill cam is ever queued): mark
/// them dead — respecting a death already in progress, e.g. shot moments
/// before this arrives — start the respawn timer, and send where they'll
/// reappear. If the fall was a graded landing (`speed` is `Some`) the victim
/// is also told to play the fall-death effect ([`shared::FallDeath`]); a void
/// fall already started it client-side. (`Zombies` is different: see below.)
#[allow(clippy::too_many_arguments)]
fn fall_kill(
    peer: PeerId,
    speed: Option<f32>,
    time: &Time,
    server: &Server,
    sender: &mut ServerMultiMessageSender,
    combats: &mut Query<(&PlayerId, &mut PlayerCombat)>,
    poses: &Query<(&PlayerId, &PlayerPose)>,
    lobbies: &mut Query<(Entity, &mut Lobby)>,
) {
    let Some((_, mut lobby)) = lobbies.iter_mut().find(|(_, l)| l.has(peer)) else {
        return;
    };
    // `Zombies`: a hard landing puts them down where they land, for a
    // teammate to revive; falling out of the world leaves nobody to reach,
    // so they're out until the next round (`crate::revive` — which also
    // ends the game once everyone's down or out). No respawn here.
    if lobby.mode == GameMode::Zombies {
        if let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == peer) {
            if !combat.alive {
                return;
            }
            if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
                m.downs += 1;
            }
            if speed.is_some() {
                let perks = lobby
                    .members
                    .iter()
                    .find(|m| m.peer == peer)
                    .map(|m| m.perks.clone())
                    .unwrap_or_default();
                combat.go_down(perks, lobby.real_count() == 1);
                info!("{peer:?} is down from a fall");
            } else {
                combat.bleed_out(lobby.round);
                // (Out loses every perk, as a bleed-out does.)
                if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
                    m.perks.clear();
                }
                info!("{peer:?} fell out of the world — out until the next round");
            }
        }
        return;
    }
    if let Some((_, mut combat)) = combats.iter_mut().find(|(id, _)| id.0 == peer) {
        // Only a player who was already dead is skipped: a landing that just
        // took health to zero arrives here with `alive` still true.
        if !combat.alive {
            return;
        }
        combat.alive = false;
        combat.health = 0.0;
        combat.respawn_at = time.elapsed_secs() + FALL_RESPAWN_DELAY_SECS;
    }
    let others: Vec<Vec3> = poses
        .iter()
        .filter(|(id, _)| id.0 != peer && lobby.has(id.0))
        .map(|(_, pose)| pose.translation)
        .collect();
    let seed = time.elapsed().as_nanos() as u64 ^ peer.to_bits();
    let (pos, yaw) = shared::spawns::spawn_point(seed, &others, lobby.map);

    if let Some(speed) = speed {
        if let Err(e) =
            sender.send::<_, GameChannel>(&FallDeath { speed }, server, &NetworkTarget::Single(peer))
        {
            error!("failed to send fall death to {peer:?}: {e:?}");
        }
    }
    let msg = PlayerRespawn {
        pos: pos.to_array(),
        yaw,
        immediate: false,
    };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(peer)) {
        error!("failed to send respawn to {peer:?}: {e:?}");
    }
}

/// Copy each player's health onto their replicated [`PlayerHealth`] — only when
/// it changed, so an idle player costs no replication.
fn sync_health(mut players: Query<(&PlayerCombat, &mut PlayerHealth)>) {
    for (combat, mut health) in &mut players {
        let value = combat.health.max(0.0);
        if health.0 != value {
            health.0 = value;
        }
    }
}

/// A new life: `peer` now carries whatever they last picked in the loadout
/// (a `FreeForAll` change made too late to swap mid-life lands here).
fn arm_from_loadout(lobby: &mut Mut<Lobby>, peer: PeerId) {
    // (`Zombies` keeps what was bought or picked up through a bleed-out,
    // like its Pack-a-Punch levels — and its loadout can't change mid-game.)
    if !lobby.mode.has_loadout()
        || lobby.mode == GameMode::Zombies
        || !lobby.members.iter().any(|m| m.peer == peer && m.primary != m.loadout)
    {
        return;
    }
    if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
        m.primary = m.loadout;
        m.weapons = shared::weapon::SlotWeapon::starting(m.loadout);
    }
}

/// The player's client has respawned (see [`RespawnReady`]): they're alive now,
/// whatever the timer says. Either way their new life — and its loadout-swap
/// grace — starts now, actually back in the world (the server timer can
/// beat a long kill cam), with their picked loadout.
fn on_respawn_ready(
    trigger: Trigger<RemoteTrigger<RespawnReady>>,
    time: Res<Time>,
    mut combats: Query<(&PlayerId, &mut PlayerCombat, Option<&crate::lobby::LobbyPlayer>)>,
    mut lobbies: Query<&mut Lobby>,
) {
    let peer = trigger.from;
    let now = time.elapsed_secs();
    if let Some((_, mut combat, lp)) = combats.iter_mut().find(|(id, ..)| id.0 == peer) {
        if !combat.alive {
            info!("{peer:?} respawned");
        }
        combat.respawn(now);
        if let Some(mut lobby) = lp.and_then(|lp| lobbies.get_mut(lp.lobby).ok()) {
            arm_from_loadout(&mut lobby, peer);
        }
    }
}

/// Keep each player's [`PlayerCombat::life_started`] / `fired` current for
/// the loadout-swap grace: the clock doesn't start until everyone's loaded
/// in, and any trigger pull while alive uses it up.
fn track_lives(
    time: Res<Time>,
    lobbies: Query<&Lobby>,
    mut players: Query<(&crate::lobby::LobbyPlayer, &mut PlayerCombat, Option<&ActionState<PlayerInput>>)>,
) {
    let now = time.elapsed_secs();
    for (lp, mut combat, input) in &mut players {
        let Ok(lobby) = lobbies.get(lp.lobby) else {
            continue;
        };
        if !lobby.started || lobby.paused {
            continue;
        }
        if !lobby.members.iter().all(|m| m.loaded) {
            combat.life_started = now;
        } else if combat.alive && !combat.fired && input.is_some_and(|i| i.0.fire) {
            combat.fired = true;
        }
    }
}

/// Once a dead player's respawn timer is up, make them targetable / able to
/// fire again. The actual reposition is client-driven (see
/// `client::net::flush_pending_respawn`) — this only ungates hit detection.
fn tick_respawns(
    time: Res<Time>,
    mut lobbies: Query<&mut Lobby>,
    mut combats: Query<(&PlayerId, &mut PlayerCombat, Option<&crate::lobby::LobbyPlayer>)>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    for (id, mut combat, lp) in &mut combats {
        // Paused: push the respawn, the regen hold and the loadout-swap
        // grace back by the pause, so no timer runs down while the game's
        // frozen.
        if lp.is_some_and(|lp| lobbies.get(lp.lobby).is_ok_and(|l| l.paused)) {
            combat.respawn_at += dt;
            combat.last_damage += dt;
            combat.life_started += dt;
            continue;
        }
        // (A player's — not a zombie's — health follows their perks:
        // Juggernog's bigger bar, Quick Revive's quicker regeneration.)
        let perks: Vec<shared::perks::Perk> = if combat.regenerates {
            lp.and_then(|lp| lobbies.get(lp.lobby).ok())
                .and_then(|l| l.members.iter().find(|m| m.peer == id.0))
                .map(|m| m.perks.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if combat.regenerates {
            let max = shared::perks::max_health(&perks);
            if combat.max_health != max {
                combat.max_health = max;
                combat.health = combat.health.min(max);
            }
        }
        if !combat.alive && now >= combat.respawn_at {
            combat.respawn(now);
            if let Some(mut lobby) = lp.and_then(|lp| lobbies.get_mut(lp.lobby).ok()) {
                arm_from_loadout(&mut lobby, id.0);
            }
        } else if combat.alive
            && combat.regenerates
            && combat.health < combat.max_health
            && now - combat.last_damage >= shared::perks::regen_delay(&perks)
        {
            // Hurt but not dead: hold, then climb back linearly.
            combat.health = (combat.health + shared::perks::regen_rate(&perks) * dt).min(combat.max_health);
        }
    }
}

/// End a `FreeForAll` match the instant someone reaches the lobby's kill
/// limit, rather than waiting for the clock (see `lobby::end_match`).
fn check_kill_limit(
    clock: Res<crate::killcam::ReplayClock>,
    lobbies: Query<(Entity, &Lobby)>,
    mut endings: ResMut<crate::killcam::EndingLobbies>,
) {
    for (lobby_e, lobby) in &lobbies {
        if !lobby.started || lobby.mode != GameMode::FreeForAll {
            continue;
        }
        if lobby.members.iter().any(|m| m.score >= lobby.kill_limit) {
            endings.begin(lobby_e, clock.0, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respawning_restores_everything_a_death_or_damage_changed() {
        let mut c = PlayerCombat {
            health: -12.0,
            max_health: FULL_HEALTH,
            regenerates: true,
            last_damage: 40.0,
            alive: false,
            respawn_at: 99.0,
            life_started: 3.0,
            fired: true,
            phd_ready_at: 999.0,
            down: None,
            bled_out: Some(3),
        };
        c.respawn(120.0);
        assert!(c.alive);
        assert_eq!(c.health, FULL_HEALTH);
        // No leftover damage timer holding off (or restarting) regeneration.
        assert_eq!(c.last_damage, f32::NEG_INFINITY);
        assert_eq!(c.respawn_at, 0.0);
        // A fresh loadout-swap grace.
        assert!(c.can_swap_loadout(120.0));
        // PhD Flopper's cooldown doesn't carry over a death.
        assert!(c.try_phd_blast(120.0));
        assert!(!c.try_phd_blast(120.5));
        assert!(c.bled_out.is_none());
    }

    #[test]
    fn a_loadout_swaps_at_once_only_just_after_spawning_and_before_firing() {
        let c = PlayerCombat::spawned(100.0);
        assert!(c.can_swap_loadout(100.0 + LOADOUT_SWAP_GRACE_SECS));
        assert!(!c.can_swap_loadout(100.1 + LOADOUT_SWAP_GRACE_SECS));
        let fired = PlayerCombat { fired: true, ..PlayerCombat::spawned(100.0) };
        assert!(!fired.can_swap_loadout(100.5));
        let dead = PlayerCombat { alive: false, ..PlayerCombat::spawned(100.0) };
        assert!(!dead.can_swap_loadout(100.5));
    }
}

