//! `Zombies` last stand: downed players bleeding out, teammates reviving
//! them, bled-out players coming back next round, and the game ending once
//! everybody's down — Call of Duty style (the rules are `shared::revive`'s).
//!
//! Going down itself happens where health runs out (`crate::pvp`:
//! `PlayerCombat::go_down`, or `bleed_out` for a fall out of the world);
//! from there everything's here. The state lives on [`PlayerCombat`] and is
//! mirrored onto the replicated [`Downed`] whenever it changes in a way the
//! clients can't run on by themselves (going down, a revive starting or
//! stopping, bleeding out) — the player entities go with the game, so
//! there's nothing to reset between games.

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::bot_players::is_bot_peer;
use shared::perks::Perk;
use shared::revive::{
    Downed, PlayerRevived, PlayerWentDown, BLEED_OUT_SECS, REVIVE_RANGE, SOLO_SELF_REVIVE_SECS,
};
use shared::{GameChannel, GameMode, Lobby, PlayerId, PlayerInput, PlayerPose, PlayerRespawn};

use crate::lobby::LobbyPlayer;
use crate::nav::NavGraphs;
use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;

/// A downed player's last stand (`PlayerCombat::down`).
#[derive(Clone, Debug)]
pub(crate) struct LastStand {
    /// What they held going down, purchase order — lost last-first as the
    /// clock runs ([`shared::revive::perks_kept`]).
    perks: Vec<Perk>,
    /// Bleed-out seconds left.
    bleed_left: f32,
    /// Who's reviving them (themselves, for solo Quick Revive).
    reviver: Option<PeerId>,
    /// How long the current revive takes, and how far into it they are.
    revive_secs: f32,
    revive_done: f32,
    /// Playing solo with Quick Revive: they get themselves back up.
    self_revive: bool,
}

impl LastStand {
    pub(crate) fn new(perks: Vec<Perk>, solo: bool) -> Self {
        Self {
            self_revive: solo && perks.contains(&Perk::QuickRevive),
            perks,
            bleed_left: BLEED_OUT_SECS,
            reviver: None,
            revive_secs: 0.0,
            revive_done: 0.0,
        }
    }
}

pub struct RevivePlugin;

impl Plugin for RevivePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (run_last_stands, sync_downed)
                .chain()
                .after(crate::pvp::apply_player_hits),
        );
    }
}

/// One player in a running `Zombies` game, as [`run_last_stands`] sees them.
struct Member {
    peer: PeerId,
    feet: Vec3,
    holding_revive: bool,
}

/// Run every downed player's last stand on by a tick: revives (a living
/// teammate holding the interact key in reach, or solo Quick Revive) and
/// the bleed-out clock (stopped while someone's reviving), with perks lost
/// along it; bring the bled-out back once a new round starts; and end the
/// game once nobody's left standing (a solo Quick Revive still counts).
#[allow(clippy::too_many_arguments)]
fn run_last_stands(
    time: Res<Time>,
    navs: Res<NavGraphs>,
    clock: Res<crate::killcam::ReplayClock>,
    mut endings: ResMut<crate::killcam::EndingLobbies>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut players: Query<(
        &PlayerId,
        &PlayerPose,
        &LobbyPlayer,
        &mut PlayerCombat,
        Option<&ActionState<PlayerInput>>,
    )>,
) {
    let server = server.into_inner();
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    for (lobby_e, mut lobby) in &mut lobbies {
        if !lobby.started
            || lobby.mode != GameMode::Zombies
            || lobby.paused
            || endings.is_ending(lobby_e)
        {
            continue;
        }
        let members: Vec<Member> = players
            .iter()
            .filter(|(id, _, lp, ..)| lp.lobby == lobby_e && !is_bot_peer(id.0))
            .map(|(id, pose, _, combat, input)| Member {
                peer: id.0,
                feet: pose.translation - Vec3::Y * EYE_HEIGHT,
                holding_revive: combat.alive && input.is_some_and(|i| i.0.revive),
            })
            .collect();

        // Who's reviving whom: each teammate holding the key takes the
        // nearest downed player in reach — keeping whoever they were already
        // on, and nobody being revived by two at once.
        let downed: Vec<(PeerId, Vec3, Option<PeerId>)> = players
            .iter()
            .filter(|(id, _, lp, c, _)| lp.lobby == lobby_e && !is_bot_peer(id.0) && c.down.is_some())
            .filter(|(.., c, _)| c.down.as_ref().is_some_and(|d| !d.self_revive))
            .map(|(id, pose, _, c, _)| {
                (id.0, pose.translation - Vec3::Y * EYE_HEIGHT, c.down.as_ref().and_then(|d| d.reviver))
            })
            .collect();
        let mut revivers: Vec<(PeerId, PeerId)> = Vec::new(); // (downed, reviver)
        let in_reach = |reviver: Vec3, downed: Vec3| {
            Vec2::new(reviver.x - downed.x, reviver.z - downed.z).length() <= REVIVE_RANGE
                && (reviver.y - downed.y).abs() <= 1.5
        };
        // Revives already under way carry on while the key's still held.
        for &(peer, at, reviver) in &downed {
            if let Some(r) = reviver.filter(|r| {
                members
                    .iter()
                    .any(|m| m.peer == *r && m.holding_revive && in_reach(m.feet, at))
            }) {
                revivers.push((peer, r));
            }
        }
        for m in members.iter().filter(|m| m.holding_revive) {
            if revivers.iter().any(|(_, r)| *r == m.peer) {
                continue;
            }
            let nearest = downed
                .iter()
                .filter(|(peer, at, _)| {
                    *peer != m.peer && in_reach(m.feet, *at) && !revivers.iter().any(|(d, _)| d == peer)
                })
                .min_by(|a, b| a.1.distance_squared(m.feet).total_cmp(&b.1.distance_squared(m.feet)));
            if let Some((peer, ..)) = nearest {
                revivers.push((*peer, m.peer));
            }
        }

        let mut respawns: Vec<PeerId> = Vec::new();
        for (id, _, lp, mut combat, _) in &mut players {
            if lp.lobby != lobby_e || is_bot_peer(id.0) {
                continue;
            }
            // Out since an earlier round: back now.
            if combat.bled_out.is_some_and(|r| lobby.round > r) {
                respawns.push(id.0);
                continue;
            }
            let Some(down) = combat.down.as_mut() else {
                continue;
            };
            let reviver = if down.self_revive {
                Some(id.0)
            } else {
                revivers.iter().find(|(d, _)| *d == id.0).map(|(_, r)| *r)
            };
            if reviver != down.reviver {
                down.reviver = reviver;
                down.revive_done = 0.0;
                down.revive_secs = match reviver {
                    Some(r) if r == id.0 => SOLO_SELF_REVIVE_SECS,
                    Some(r) => shared::revive::revive_secs(
                        lobby
                            .members
                            .iter()
                            .find(|m| m.peer == r)
                            .map_or(&[], |m| m.perks.as_slice()),
                    ),
                    None => 0.0,
                };
            }
            let member = lobby.members.iter_mut().find(|m| m.peer == id.0);
            if down.reviver.is_some() {
                down.revive_done += dt;
                if down.revive_done >= down.revive_secs {
                    // Solo Quick Revive is spent getting back up.
                    if down.self_revive {
                        if let Some(m) = member {
                            m.perks.retain(|p| *p != Perk::QuickRevive);
                        }
                    }
                    info!("{:?} was revived by {:?}", id.0, down.reviver);
                    let reviver = down.reviver.unwrap_or(id.0);
                    if reviver != id.0 {
                        if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == reviver) {
                            m.revives += 1;
                        }
                    }
                    let msg = PlayerRevived { peer: id.0, reviver };
                    let mut to = vec![id.0];
                    if reviver != id.0 {
                        to.push(reviver);
                    }
                    if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(to)) {
                        error!("failed to send a revive to {:?}: {e:?}", id.0);
                    }
                    combat.revive();
                }
                continue;
            }
            down.bleed_left -= dt;
            // Perks go as the clock passes their marks.
            let kept = shared::revive::perks_kept(down.perks.len(), down.bleed_left);
            if let Some(m) = member {
                let keep = &down.perks[..kept];
                if m.perks.iter().any(|p| !keep.contains(p)) {
                    m.perks.retain(|p| keep.contains(p));
                }
                if down.bleed_left <= 0.0 {
                    m.perks.clear();
                    m.armor = Default::default();
                }
            }
            if down.bleed_left <= 0.0 {
                info!("{:?} bled out", id.0);
                let round = lobby.round;
                combat.bleed_out(round);
            }
        }

        // The bled-out come back beside someone still standing.
        for peer in respawns {
            let living: Vec<Vec3> = players
                .iter()
                .filter(|(id, _, lp, c, _)| lp.lobby == lobby_e && !is_bot_peer(id.0) && c.alive && id.0 != peer)
                .map(|(_, pose, ..)| pose.translation - Vec3::Y * EYE_HEIGHT)
                .collect();
            let seed = (now.to_bits() as u64) ^ peer.to_bits();
            let (pos, yaw) = match living.first() {
                Some(&near) => {
                    let pos = navs
                        .graph(lobby.map)
                        .random_spot_near(near, 1.5, 5.0, seed)
                        .unwrap_or(near);
                    let to = near - pos;
                    (pos, f32::atan2(-to.x, -to.z))
                }
                None => shared::spawns::zombies_start(lobby.map, seed, &[])
                    .unwrap_or_else(|| shared::spawns::spawn_point(seed, &[], lobby.map)),
            };
            if let Some((.., mut combat, _)) = players.iter_mut().find(|(id, ..)| id.0 == peer) {
                combat.respawn(now);
            }
            let msg = PlayerRespawn {
                pos: pos.to_array(),
                yaw,
                immediate: true,
            };
            if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Single(peer)) {
                error!("failed to send respawn to {peer:?}: {e:?}");
            }
            info!("{peer:?} is back for round {}", lobby.round);
        }

        // Nobody left standing: game over. (Solo Quick Revive is still
        // getting up.)
        let mut any = false;
        let all_down = players
            .iter()
            .filter(|(id, _, lp, ..)| lp.lobby == lobby_e && !is_bot_peer(id.0))
            .inspect(|_| any = true)
            .all(|(.., c, _)| {
                c.bled_out.is_some() || c.down.as_ref().is_some_and(|d| !d.self_revive)
            });
        if any && all_down {
            info!("lobby {lobby_e:?}: everyone's down — zombies game over");
            endings.begin(lobby_e, clock.0, false);
        }
    }
}

/// What a player's replicated [`Downed`] should say, from their combat
/// state — `None` while they're up. The clocks are as of now; clients run
/// them on from there.
fn downed_state(combat: &PlayerCombat) -> Option<Downed> {
    if combat.bled_out.is_some() {
        return Some(Downed {
            perks: Vec::new(),
            bleed_left: 0.0,
            reviver: None,
            revive_secs: 0.0,
            bled_out: true,
        });
    }
    let down = combat.down.as_ref()?;
    Some(Downed {
        perks: down.perks.clone(),
        bleed_left: down.bleed_left,
        reviver: down.reviver,
        revive_secs: down.revive_secs,
        bled_out: false,
    })
}

/// Mirror each player's last stand onto their replicated [`Downed`] — only
/// when it changes in a way the clients can't follow by themselves (so not
/// for the clocks just running down) — and tell the lobby whoever's just
/// gone down ([`PlayerWentDown`]).
fn sync_downed(
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    lobbies: Query<&Lobby>,
    players: Query<(Entity, &PlayerId, &LobbyPlayer, &PlayerCombat, Option<&Downed>)>,
    mut commands: Commands,
) {
    let server = server.into_inner();
    for (entity, id, lp, combat, current) in &players {
        let want = downed_state(combat);
        match (want, current) {
            (None, None) => {}
            (None, Some(_)) => {
                commands.entity(entity).remove::<Downed>();
            }
            (Some(want), Some(have))
                if want.bled_out == have.bled_out
                    && want.reviver == have.reviver
                    && want.perks == have.perks => {}
            (Some(want), current) => {
                // Just down (not straight out — a fall out of the world).
                if current.is_none() && !want.bled_out {
                    if let Ok(lobby) = lobbies.get(lp.lobby) {
                        let msg = PlayerWentDown { peer: id.0 };
                        let to = NetworkTarget::Only(lobby.real_peers());
                        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &to) {
                            error!("failed to send {:?} going down: {e:?}", id.0);
                        }
                    }
                }
                commands.entity(entity).insert(want);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_quick_revive_gets_you_up_but_only_alone() {
        assert!(LastStand::new(vec![Perk::QuickRevive], true).self_revive);
        assert!(!LastStand::new(vec![Perk::QuickRevive], false).self_revive);
        assert!(!LastStand::new(vec![Perk::Juggernog], true).self_revive);
    }

    #[test]
    fn going_down_then_being_revived_brings_you_back_whole() {
        let mut c = PlayerCombat::spawned(0.0);
        c.go_down(vec![Perk::Juggernog], false);
        assert!(!c.alive && c.down.is_some());
        let d = downed_state(&c).unwrap();
        assert!(!d.bled_out);
        assert_eq!(d.bleed_left, BLEED_OUT_SECS);
        c.revive();
        assert!(c.alive && c.down.is_none() && c.health > 0.0);
        assert!(downed_state(&c).is_none());
        c.bleed_out(4);
        assert!(downed_state(&c).unwrap().bled_out);
    }
}
