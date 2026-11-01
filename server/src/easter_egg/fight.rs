//! The easter egg's boss fight, phase 1 ([`EggStep::BossFight`]): the
//! Hierophant, a heavy melee stalker.
//!
//! * The moment the fight's on (however it got there — he woke, or the
//!   debug panel jumped to it), he's in: standing on the sarcophagus where
//!   the wake-up left him (`shared::easter_egg::hierophant_start`), with
//!   [`shared::easter_egg::hierophant_health`] — a boss's body (its model,
//!   hitbox and smash) on a brain that never hurls blasts and hits harder
//!   (`ai::BotBrain::hierophant`). Who he is goes on the lobby
//!   (`EasterEgg::hierophant`), for the clients' health bar.
//! * He's not one of the round's: no `boss::Boss`, so the round's boss
//!   counting and bringing far-off bosses back in leave him alone
//!   (`zombies::recycle_far_zombies` skips him) — the round's own spawns are
//!   held throughout (`shared::easter_egg::holds_spawns`).
//! * Every so often he stops to taunt ([`shared::HierophantTaunt`], the
//!   clients play the line), standing and glaring for
//!   [`shared::easter_egg::TAUNT_HOLD_SECS`].
//! * Killed, the fight's over ([`EggStep::Ascension`]): the round picks up
//!   where it left off. His corpse lies there like any zombie's.
//! * The egg set anywhere but the fight (a debug jump, a reset, the game
//!   ending) takes him away if he's still up; jumping to the fight again
//!   brings a fresh one in.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use lightyear::prelude::server::Server;
use lightyear::prelude::*;
use shared::bot_players::{bot_peer, BotDifficulty};
use shared::easter_egg::{EggStep, Hierophant as HierophantInfo};
use shared::{GameChannel, Lobby, PlayerId};

use crate::ai::{BotBrain, NextBotId};
use crate::lobby::LobbyPlayer;
use crate::pvp::PlayerCombat;

/// On the Hierophant's zombie entity.
#[derive(Component)]
pub(crate) struct Hierophant;

/// Each lobby's fight: how long (s, not counting pauses) it's been on, and
/// its taunts' timing.
#[derive(Resource, Default)]
pub(super) struct Fights(HashMap<Entity, Fight>);

struct Fight {
    t: f32,
    next_taunt: f32,
    last_taunt: Option<u8>,
    seq: u32,
}

impl Fight {
    fn new() -> Self {
        Self {
            t: 0.0,
            next_taunt: shared::easter_egg::TAUNT_FIRST_SECS,
            last_taunt: None,
            seq: 0,
        }
    }
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

/// Bring him in once the fight's on, taunt now and then, end the fight once
/// he's dead — and take him away if the egg's set anywhere else.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn run_fight(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    endings: Res<crate::killcam::EndingLobbies>,
    mut next_id: ResMut<NextBotId>,
    mut fights: ResMut<Fights>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    mut him: Query<(Entity, &LobbyPlayer, &PlayerId, &PlayerCombat, &mut BotBrain), With<Hierophant>>,
    mut commands: Commands,
) {
    let server = server.into_inner();
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
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
            // goes, and he comes in.
            for (e, lp, ..) in &him {
                if lp.lobby == lobby_e {
                    commands.entity(e).try_despawn();
                }
            }
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
                .hierophant(now);
            let max = shared::easter_egg::hierophant_health(lobby.round, lobby.real_count());
            crate::boss::spawn_boss(&mut commands, lobby_e, &lobby, peer, "The Hierophant", feet, yaw, brain, max, max)
                .insert(Hierophant);
            lobby.easter_egg.hierophant = Some(HierophantInfo { peer, max_health: max });
            fights.0.insert(lobby_e, Fight::new());
            info!("lobby {lobby_e:?}: the Hierophant's in the fight ({max:.0} health)");
            continue;
        };
        let Ok((_, _, _, combat, mut brain)) = him.get_mut(current) else { continue };
        if !combat.alive {
            lobby.easter_egg.step = EggStep::Ascension;
            fights.0.remove(&lobby_e);
            info!("lobby {lobby_e:?}: the Hierophant's dead — the fight's over");
            continue;
        }
        let fight = fights.0.entry(lobby_e).or_insert_with(Fight::new);
        fight.t += dt;
        if fight.t < fight.next_taunt {
            continue;
        }
        if !brain.taunt(now, shared::easter_egg::TAUNT_HOLD_SECS) {
            // (Mid-smash, or stunned: in a moment.)
            fight.next_taunt = fight.t + 0.5;
            continue;
        }
        fight.seq = fight.seq.wrapping_add(1);
        let roll = |k: u32| shared::bots::rand01((fight.seq as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (k as u64) ^ lobby_e.to_bits());
        let count = shared::easter_egg::TAUNTS;
        let mut line = ((roll(1) * count as f32) as u8).min(count - 1);
        if count > 1 && fight.last_taunt == Some(line) {
            line = (line + 1) % count;
        }
        fight.last_taunt = Some(line);
        let (lo, hi) = shared::easter_egg::TAUNT_GAP_SECS;
        fight.next_taunt = fight.t + lo + roll(2) * (hi - lo);
        send_to_lobby(&mut sender, server, &lobby, &shared::HierophantTaunt { line });
    }
}
