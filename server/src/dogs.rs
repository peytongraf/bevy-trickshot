//! Dog rounds' hellhounds (`shared::dogs`), on top of `crate::zombies`'
//! rounds: that queues each dog ([`crate::zombies::PendingDog`]); this tells
//! the lobby to strike lightning where it'll appear, brings it in once the
//! strike's done (with a flash), and the moment one dies — blown up on a
//! player (`ai::drive_bots`) or killed — tells the lobby it exploded and
//! removes it on the spot. Where the last one went is where the dog
//! round's Max Ammo drops (`crate::zombies::run_rounds`).
//!
//! A hellhound is a zombie (`Zombie`) to everything else: scored, counted
//! and culled like one.

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::Server;
use lightyear::prelude::*;

use shared::bot_players::{bot_peer, BotDifficulty};
use shared::bots::rand01;
use shared::{GameChannel, Lobby, PlayerId, PlayerInput, PlayerName, PlayerPose};

use crate::ai::{BotBrain, NextBotId};
use crate::lobby::LobbyPlayer;
use crate::pvp::PlayerCombat;
use crate::sim::EYE_HEIGHT;
use crate::zombies::{Zombie, ZombieRounds};

/// On a hellhound's player entity (along with `Zombie`).
#[derive(Component)]
pub struct Dog;

pub struct DogsPlugin;

impl Plugin for DogsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                bring_in_dogs
                    .after(crate::zombies::run_rounds)
                    .before(crate::ai::drive_bots),
                explode_dead_dogs.after(crate::pvp::apply_player_hits),
            ),
        );
    }
}

fn send_to_lobby<M: lightyear::prelude::Message>(
    sender: &mut ServerMultiMessageSender,
    server: &Server,
    lobby: &Lobby,
    msg: &M,
) {
    if let Err(e) = sender.send::<_, GameChannel>(msg, server, &NetworkTarget::Only(lobby.real_peers())) {
        error!("failed to send a dog message: {e:?}");
    }
}

/// Strike each queued hellhound's lightning, and bring it in once it's due.
fn bring_in_dogs(
    time: Res<Time>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut next_id: ResMut<NextBotId>,
    mut lobbies: Query<(Entity, &Lobby, &mut ZombieRounds)>,
    mut commands: Commands,
) {
    let server = server.into_inner();
    let now = time.elapsed_secs();
    for (lobby_e, lobby, mut rounds) in &mut lobbies {
        if lobby.paused {
            continue;
        }
        // (An exfil's are tougher.)
        let round = rounds.enemy_round();
        let mut i = 0;
        while i < rounds.pending_dogs.len() {
            let dog = &mut rounds.pending_dogs[i];
            if !dog.announced {
                dog.announced = true;
                send_to_lobby(&mut sender, server, lobby, &shared::DogLightning { at: dog.feet.to_array() });
            }
            if now < dog.spawn_at {
                i += 1;
                continue;
            }
            let dog = rounds.pending_dogs.swap_remove(i);

            let peer = bot_peer(next_id.0);
            next_id.0 += 1;
            let seed = (now.to_bits() as u64) << 20 ^ next_id.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ lobby_e.to_bits();
            let speed = shared::dogs::dog_speed(round, rand01(seed ^ 0x2a2a));
            // Quick to pick a player up and quick on its feet.
            let mut skill = crate::zombies::zombie_skill(round);
            skill.reaction_secs = skill.reaction_secs.min(0.3);
            skill.turn_deg_per_sec = skill.turn_deg_per_sec.max(320.0);
            skill.sight_range = skill.sight_range.max(80.0);
            let health = shared::dogs::dog_health(round);
            let real = lobby.real_peers();
            commands.spawn((
                Name::from("Hellhound"),
                Zombie,
                Dog,
                LobbyPlayer { lobby: lobby_e },
                PlayerId(peer),
                PlayerName("Hellhound".into()),
                PlayerPose {
                    translation: dog.feet + Vec3::Y * EYE_HEIGHT,
                    yaw: dog.yaw,
                    zombie: shared::ZombieAnim::Dog,
                    ..default()
                },
                ActionState::<PlayerInput>::default(),
                BotBrain::new(BotDifficulty::Recruit, dog.feet, seed)
                    .facing(dog.yaw)
                    .with_skill(skill)
                    .dog(speed),
                PlayerCombat::zombie(health),
                shared::PlayerHealth(health),
                Replicate::to_clients(NetworkTarget::Only(real.clone())),
                InterpolationTarget::to_clients(NetworkTarget::Only(real)),
            ));
            send_to_lobby(&mut sender, server, lobby, &shared::DogSpawned { at: dog.feet.to_array() });
        }
    }
}

/// A hellhound that's died (blown up on a player, or killed) explodes where
/// it is — every client's told — and is gone at once.
fn explode_dead_dogs(
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    dogs: Query<(Entity, &PlayerId, &PlayerPose, &PlayerCombat, &LobbyPlayer), With<Dog>>,
    lobbies: Query<&Lobby>,
    mut rounds: Query<&mut ZombieRounds>,
    mut commands: Commands,
) {
    let server = server.into_inner();
    for (entity, id, pose, combat, lp) in &dogs {
        if combat.alive {
            continue;
        }
        let at = pose.translation - Vec3::Y * EYE_HEIGHT;
        // (The last one to go is where the round's Max Ammo drops.)
        if let Ok(mut rounds) = rounds.get_mut(lp.lobby) {
            rounds.last_dog_at = Some(at);
        }
        if let Ok(lobby) = lobbies.get(lp.lobby) {
            send_to_lobby(&mut sender, server, lobby, &shared::DogExploded { dog: id.0, at: at.to_array() });
        }
        commands.entity(entity).try_despawn();
    }
}
