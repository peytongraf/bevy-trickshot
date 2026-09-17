//! Practice-mode and networked target-bot animation: a looped idle clip that
//! switches to a one-shot death clip.

use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::render::view::NoFrustumCulling;
use bevy::scene::SceneInstanceReady;

/// On every target-bot visual (practice-local *and* networked avatars), so the
/// kill cam can hide the live bots and show its own frozen snapshot instead.
#[derive(Component)]
pub(crate) struct TargetBotVisual;

/// `models/bot.glb` is imported noticeably larger than [`shared::bots::BOT_HEIGHT`]
/// (the invisible hitbox capsule) — scaled down so what's on screen lines up
/// with where shots actually register.
pub(crate) const BOT_MODEL_SCALE: f32 = 2.0 / 3.0;

/// Playback speed for the death clip (`Armature|MTF_Die`) — plays out twice
/// as fast as authored.
pub(crate) const BOT_DIE_SPEED: f32 = 2.0;

/// Graph + node indices for `models/bot.glb`'s two animations, built once at
/// startup: `MTF_IdleAction1` (looped while alive) and `Armature|MTF_Die`
/// (played once on death).
#[derive(Resource, Clone)]
pub(crate) struct BotAnimations {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    pub(crate) die: AnimationNodeIndex,
}

/// Tags a bot's `SceneRoot` entity (practice bot, networked avatar, or kill-cam
/// ghost) so `start_bot_animation` knows to wire it up once the scene finishes
/// spawning — and so the observer can tell a bot's scene apart from any other
/// (the sniper, the map, ...).
#[derive(Component)]
pub(crate) struct BotVisual;

/// Set by `start_bot_animation` once it finds the `AnimationPlayer` inside a
/// bot's spawned scene, so later systems (death handling) can reach it
/// directly instead of re-walking the hierarchy every frame.
#[derive(Component)]
pub(crate) struct BotAnimationPlayer(pub(crate) Entity);

/// Build the bot animation graph. Added to the same `Startup` tuple as
/// `setup_player` / `setup_crosshair` — adding a *separate*
/// `add_systems(Startup, ...)` call (e.g. from a plugin) perturbs that tuple's
/// execution order enough to black out the in-game 3D view, so this must stay
/// part of the one tuple.
pub(crate) fn setup_bot_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let idle_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(2).from_asset("models/bot.glb"));
    let die_clip: Handle<AnimationClip> =
        asset_server.load(GltfAssetLabel::Animation(3).from_asset("models/bot.glb"));
    let (graph, indices) = AnimationGraph::from_clips([idle_clip, die_clip]);
    let graph = graphs.add(graph);
    commands.insert_resource(BotAnimations {
        graph,
        idle: indices[0],
        die: indices[1],
    });
}

/// Fires once a bot's `SceneRoot` (tagged [`BotVisual`]) finishes spawning:
/// starts the idle animation looping and remembers which descendant holds the
/// `AnimationPlayer`, via [`BotAnimationPlayer`], for `play_bot_death` to use
/// later.
pub(crate) fn start_bot_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    bots: Query<(), With<BotVisual>>,
    mut players: Query<&mut AnimationPlayer>,
    anims: Res<BotAnimations>,
) {
    let root = trigger.target();
    if !bots.contains(root) {
        return;
    }
    for entity in children.iter_descendants(root) {
        // Same fix as the sniper view model: a skinned mesh is frustum-culled
        // against its *rest-pose* AABB, not the animated one, so the idle
        // sway can walk the head/body right out of a rest-pose-computed
        // frustum — most visible zoomed in (scope), where the FOV is narrow.
        commands.entity(entity).insert(NoFrustumCulling);

        if let Ok(mut player) = players.get_mut(entity) {
            let active = player.play(anims.idle);
            active.set_repeat(RepeatAnimation::Forever);
            commands
                .entity(entity)
                .insert(AnimationGraphHandle(anims.graph.clone()));
            commands.entity(root).insert(BotAnimationPlayer(entity));
        }
    }
}

/// Switch a bot to its death animation, played once. Callers should only
/// invoke this on the frame death is first observed (e.g. guarded by their
/// own "already dying" flag) — a no-op if the scene hasn't finished spawning
/// yet, in which case the bot just never animates a death, same as a shot
/// landing before `start_bot_animation` has run.
pub(crate) fn play_bot_death(
    root: Entity,
    roots: &Query<&BotAnimationPlayer>,
    players: &mut Query<&mut AnimationPlayer>,
    anims: &BotAnimations,
) {
    let Ok(target) = roots.get(root) else { return };
    let Ok(mut player) = players.get_mut(target.0) else {
        return;
    };
    let active = player.play(anims.die);
    active.set_repeat(RepeatAnimation::Never);
    active.set_speed(BOT_DIE_SPEED);
    active.replay();
}
