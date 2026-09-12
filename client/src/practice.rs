//! Solo Practice: offline bots + style-point scoring, mirroring the server so
//! practising feels identical to a real Freestyle match. The bot systems sit
//! idle whenever the local player is in a networked lobby (the server owns the
//! bots there); `resolve_local_shot` always runs so a missed shot still kicks
//! up ground dust.

use bevy::prelude::*;
use lightyear::prelude::*;

use shared::ballistics::{ground_impact, resolve_shot_pierce, Target};
use shared::bots::{
    respawn_pose, BOTS_ALIVE, BOT_DEAD_SECS, BOT_HEAD_RADIUS, BOT_HEIGHT, BOT_RADIUS,
};
use shared::hitbox::Capsule;
use shared::weapon::WeaponId;

use crate::{play_bot_death, BotAnimationPlayer, BotAnimations, BotVisual};

use crate::killcam::{PendingLocal, PendingLocalCam};
use crate::net::GameClient;
use crate::{
    Ads, AppState, GroundImpact, Player, PlayerPhysics, TrickScoredEvent, TrickState,
    NOSCOPE_ADS_MAX,
};

/// The local player fired: the ray to resolve against the world this frame.
#[derive(Event)]
pub(crate) struct LocalShot {
    pub(crate) origin: Vec3,
    pub(crate) dir: Vec3,
}

/// Running score in solo Practice (a lobby game keeps score on the server).
#[derive(Resource, Default)]
struct PracticeScore(u32);

/// One offline target bot.
#[derive(Component)]
struct PracticeBot {
    pos: Vec3,
    yaw: f32,
    /// Set once `tick_practice_bots` has kicked off the death animation, so it
    /// isn't restarted every frame the bot lingers as a corpse.
    die_played: bool,
    /// Seconds-since-startup the bot was shot; `None` while alive.
    dead_at: Option<f32>,
}

#[derive(Component)]
struct PracticeScoreText;

pub struct PracticePlugin;

impl Plugin for PracticePlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<LocalShot>()
            .init_resource::<PracticeScore>()
            .add_systems(
                OnEnter(AppState::InGame),
                (reset_practice, spawn_score_text),
            )
            .add_systems(
                Update,
                (
                    resolve_local_shot,
                    (spawn_practice_bots, place_practice_bots, tick_practice_bots)
                        .run_if(is_practice.and(crate::killcam::no_killcam)),
                    update_score_text,
                )
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// True while the local player is *not* in a networked lobby.
pub(crate) fn is_practice(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) -> bool {
    let me = local.iter().next().map(|l| l.0);
    !me.map(|me| lobbies.iter().any(|l| l.has(me)))
        .unwrap_or(false)
}

fn reset_practice(
    mut score: ResMut<PracticeScore>,
    bots: Query<Entity, With<PracticeBot>>,
    mut commands: Commands,
) {
    score.0 = 0;
    for e in &bots {
        commands.entity(e).try_despawn();
    }
}

/// Keep the arena topped up to `BOTS_ALIVE` live bots, placed the same way the
/// server places them (`shared::bots::respawn_pose`).
fn spawn_practice_bots(
    time: Res<Time>,
    asset_server: Res<AssetServer>,
    bots: Query<&PracticeBot>,
    mut seq: Local<u64>,
    mut commands: Commands,
) {
    let alive = bots.iter().filter(|b| b.dead_at.is_none()).count();
    for _ in alive..BOTS_ALIVE {
        *seq = seq.wrapping_add(1);
        let seed = time.elapsed().as_nanos() as u64 ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        // Solo Practice always plays on the basic map — no lobby to pick another.
        let (pos, yaw) = respawn_pose(seed, shared::MapId::BasicMap);
        commands
            .spawn((
                PracticeBot {
                    pos,
                    yaw,
                    die_played: false,
                    dead_at: None,
                },
                BotVisual,
                crate::TargetBotVisual,
                StateScoped(AppState::InGame),
                Transform::from_translation(pos).with_scale(Vec3::splat(crate::BOT_MODEL_SCALE)),
                Visibility::default(),
                SceneRoot(
                    asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/bot.glb")),
                ),
            ))
            .observe(crate::start_bot_animation);
    }
}

/// Push each bot's yaw onto its transform. The death animation itself now
/// conveys the fall, so this no longer layers a topple rotation on top.
fn place_practice_bots(mut bots: Query<(&PracticeBot, &mut Transform)>) {
    for (bot, mut tf) in &mut bots {
        tf.translation = bot.pos;
        tf.rotation = Quat::from_rotation_y(bot.yaw);
    }
}

/// Kick off the death animation the first tick a bot is seen dead, then
/// despawn it once it's lingered long enough (`spawn_practice_bots` refills).
fn tick_practice_bots(
    time: Res<Time>,
    mut bots: Query<(Entity, &mut PracticeBot)>,
    roots: Query<&BotAnimationPlayer>,
    mut players: Query<&mut AnimationPlayer>,
    anims: Res<BotAnimations>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    for (entity, mut bot) in &mut bots {
        let Some(dead_at) = bot.dead_at else {
            continue;
        };
        if !bot.die_played {
            bot.die_played = true;
            play_bot_death(entity, &roots, &mut players, &anims);
        }
        if now - dead_at >= BOT_DEAD_SECS {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Every `LocalShot` kicks up ground dust where it lands; in Practice it also
/// resolves against the offline bots and scores exactly as the server would.
#[allow(clippy::too_many_arguments)]
fn resolve_local_shot(
    time: Res<Time>,
    ads: Res<Ads>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    physics: Query<&PlayerPhysics, With<Player>>,
    mut shots: EventReader<LocalShot>,
    mut bots: Query<(Entity, &mut PracticeBot)>,
    mut trick: ResMut<TrickState>,
    mut score: ResMut<PracticeScore>,
    mut pending_cam: ResMut<PendingLocalCam>,
    mut ground_hit: ResMut<crate::killcam::ReplayGroundImpact>,
    mut tracer_rec: ResMut<crate::killcam::ReplayTracer>,
    mut impacts: EventWriter<GroundImpact>,
    mut blood: EventWriter<crate::BloodImpact>,
    mut scored: EventWriter<TrickScoredEvent>,
    mut tracers: EventWriter<crate::FireTracer>,
) {
    let practice = {
        let me = local.iter().next().map(|l| l.0);
        !me.map(|me| lobbies.iter().any(|l| l.has(me)))
            .unwrap_or(false)
    };

    for shot in shots.read() {
        let mut hit_bot = false;
        // The shooter's own tracer, spawned instantly (both modes) rather than
        // waiting on the server: the exact impact point if the shot connected
        // (Practice only — online has no local target data), else the ground
        // point below, else a max-range whiff. Other players' tracers come off
        // the server's authoritative `ShotResolved` (`net::receive_shots`).
        let mut tracer_end: Option<Vec3> = None;

        if practice {
            let targets: Vec<Target> = bots
                .iter()
                .filter(|(_, b)| b.dead_at.is_none())
                .map(|(e, b)| Target {
                    id: e.to_bits(),
                    body: Capsule::standing(b.pos, BOT_HEIGHT, BOT_RADIUS),
                    head: Capsule::head(b.pos, BOT_HEIGHT, BOT_HEAD_RADIUS),
                })
                .collect();

            // Collateral: pierce through as many bots as the shot can reach
            // (Call-of-Duty style) instead of stopping at the first one hit.
            let hits =
                resolve_shot_pierce(WeaponId::Sniper, shot.origin, shot.dir, &targets, |_, _| false);

            if let Some(last) = hits.last() {
                tracer_end = Some(last.point);

                // Freeze every live bot for the kill cam before any of them
                // die, marking every one this shot reached as killed — a
                // collateral topples them all in the same replay.
                let snap: Vec<(Vec3, f32, bool)> = bots
                    .iter()
                    .filter(|(_, b)| b.dead_at.is_none())
                    .map(|(e, b)| (b.pos, b.yaw, hits.iter().any(|h| h.target == e.to_bits())))
                    .collect();

                let now = time.elapsed_secs();
                let mut bots_killed = 0u32;
                for hit in &hits {
                    let Some((bot_e, _)) = bots.iter().find(|(e, _)| e.to_bits() == hit.target)
                    else {
                        continue;
                    };
                    if let Ok((_, mut bot)) = bots.get_mut(bot_e) {
                        if bot.dead_at.is_none() {
                            bot.dead_at = Some(now);
                            hit_bot = true;
                            bots_killed += 1;
                            // Squirt blood out along the shot from this hit's point.
                            blood.write(crate::BloodImpact {
                                point: hit.point,
                                dir: shot.dir.normalize_or_zero(),
                            });
                        }
                    }
                }

                if bots_killed > 0 {
                    // Kick off this player's own kill cam once enough
                    // follow-through is buffered.
                    if pending_cam.0.is_none() {
                        pending_cam.0 = Some(PendingLocal {
                            kill_at: now,
                            fire_at: now + crate::killcam::POST_SECS,
                            bots: snap,
                        });
                    }

                    // The shot's own trick score (kill + spin + no-scope) is
                    // worked out once for the whole pull of the trigger, then
                    // scaled by distance and multiplied by how many bots it
                    // hit — see `shared::scoring::score_multi_kill`. Distance
                    // goes off the nearest bot hit (every target here is a
                    // bot, so that's just the shot's overall nearest hit).
                    let grounded = physics.single().map(|p| p.grounded).unwrap_or(true);
                    let distance = hits.first().map(|h| h.distance).unwrap_or(0.0);
                    let (total, lines) = shared::scoring::score_multi_kill(
                        trick.total_deg(),
                        trick.airborne || !grounded,
                        ads.t <= NOSCOPE_ADS_MAX,
                        bots_killed,
                        distance,
                        WeaponId::Sniper.spec().max_range,
                    );
                    score.0 += total;
                    scored.write(TrickScoredEvent {
                        total,
                        lines: lines.into_iter().map(|l| (l.label, l.points)).collect(),
                    });
                    trick.reset();
                }
            }
        }

        let ground_pt = if hit_bot { None } else { ground_impact(shot.origin, shot.dir) };
        if let Some(p) = ground_pt {
            impacts.write(GroundImpact(p));
            // Stamp it onto this tick's `PlayerInput` too, so a networked
            // kill cam can replay the burst for the other players watching.
            ground_hit.0 = Some(p);
        }

        let end = tracer_end.or(ground_pt).unwrap_or_else(|| {
            shot.origin + shot.dir.normalize_or_zero() * WeaponId::Sniper.spec().max_range
        });
        tracers.write(crate::FireTracer { start: shot.origin, end });
        // Stamp it onto this tick's `PlayerInput` too, so a networked kill cam
        // re-draws the tracer along its true path instead of leaving the live
        // one hanging in the world. (Practice records off the event above.)
        tracer_rec.0 = Some((shot.origin, end));
    }
}

fn spawn_score_text(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((
        PracticeScoreText,
        StateScoped(AppState::InGame),
        GlobalZIndex(5),
        Text::new(""),
        TextFont {
            font: asset_server.load(crate::HUD_FONT),
            font_size: 26.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(33.0),
            left: Val::Px(16.0),
            ..default()
        },
    ));
}

fn update_score_text(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    score: Res<PracticeScore>,
    mut text: Query<&mut Text, With<PracticeScoreText>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let me = local.iter().next().map(|l| l.0);
    let practice = !me
        .map(|me| lobbies.iter().any(|l| l.has(me)))
        .unwrap_or(false);
    let wanted = if practice {
        format!("SCORE  {}", score.0)
    } else {
        String::new()
    };
    if text.0 != wanted {
        text.0 = wanted;
    }
}
