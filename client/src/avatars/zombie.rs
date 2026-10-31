//! `Zombies` zombie avatars (`models/characters/zombie.glb`): which clip each one plays
//! — from the state the server publishes on its pose
//! ([`shared::PlayerPose::zombie`]) and how fast it's really moving — and
//! how fast, so the feet stay planted whatever its speed. Spawned in place of
//! the soldier by `net::spawn_remote_avatars`; no gun, so no sniper glint,
//! flashlight or bot tint.

use std::time::Duration;

use bevy::animation::prelude::AnimationTransitions;
use bevy::animation::RepeatAnimation;
use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use shared::{PlayerId, PlayerPose, ZombieAnim};

use crate::net::RemoteAvatar;
use crate::player::Player;
use crate::AppState;

pub(crate) const ZOMBIE_MODEL: &str = "models/characters/zombie.glb";

/// Tags a zombie avatar's `SceneRoot` (see [`start_zombie_animation`]).
#[derive(Component)]
pub(crate) struct ZombieVisual;

/// The `AnimationPlayer` inside a zombie avatar's scene, once it's spawned.
#[derive(Component)]
pub(crate) struct ZombieAnimationPlayer(pub(crate) Entity);

/// Which of `models/characters/zombie.glb`'s clips a zombie avatar is playing
/// (`client/notes/zombie.md` — the last three, `Walk` / `Run` / `Walk1`, move
/// the model off its spot, so only the in-place ones are used).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ZombieAnimState {
    /// `Idle`: standing, or climbing out of the ground.
    #[default]
    Idle,
    /// Walking, arms down — in one of its gaits.
    Walk(Gait),
    /// `Walk_InPlace`: arms up (a walker near who it's after).
    WalkArmsUp,
    /// Running — in one of its runs.
    Run(RunGait),
    /// `Attack`: swinging at someone.
    Attack,
    /// `FallingBack` / `FallingForward`: dead, held on the last frame.
    FallBack,
    FallForward,
    /// `Mantle`: climbing in through a window, once, in place (the server
    /// moves it along the climb's path — `shared::windows`). Faster for a
    /// runner.
    Climb { run: bool },
    /// A crawler (`shared::zombies`' `CRAWLER_*`): `CrawlFall`, going down
    /// onto its belly, once...
    CrawlFall,
    /// ...`Crawl`, dragging itself along (as fast as it's really going)...
    Crawl,
    /// ...`CrawlAttack`, swiping up from the ground...
    CrawlAttack,
    /// ...and dead: where it lay, the crawl stopped.
    CrawlDead,
}

/// How a zombie walks (arms down).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Gait {
    /// `Walk1_InPlace`, the model's own.
    Original,
    /// `ShambleWalk`: hunched, limping, one arm dead and one reaching.
    Shamble,
    /// `ShambleWalkHeavy`: further gone — long, slow, lurching strides.
    Heavy,
    /// `StumbleWalk`: four steps a loop, none alike — off balance, swaying,
    /// stumbling.
    Stumble,
}

/// How far the heavy shamble's strides go (× the walk's — the mean of
/// `client/tools/zombie_walk_clips.py`'s `HEAVY` strides): it plays this
/// much slower for the same speed, so its feet keep to the ground.
const HEAVY_STRIDE: f32 = 1.5;

/// The stumble's strides, the same way (`STUMBLE`'s mean `stride`).
const STUMBLE_STRIDE: f32 = 1.35;

/// The scary run's strides (× the run's — `SCARY_RUN`'s mean `stride`).
const SCARY_RUN_STRIDE: f32 = 1.2;

/// The broken run's (`BROKEN_RUN`'s `stride`).
const BROKEN_RUN_STRIDE: f32 = 1.15;

/// How a zombie runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RunGait {
    /// `Run_InPlace`, the model's own.
    Original,
    /// `ScaryRun`: lunging, clawing, twitching — four steps a loop.
    Scary,
    /// `BrokenRun`: neck broken, wrenched round, one leg stiff, stop-start —
    /// four steps a loop.
    Broken,
}

/// Which walks zombies use (the debug panel's "Zombie model & animations"):
/// each its own pick of them all, or all one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WalkStyle {
    Mixed,
    Only(Gait),
}

impl WalkStyle {
    /// The gait for the zombie whose pick is `roll` (`0..1`, its own).
    fn gait(self, roll: f32) -> Gait {
        const ALL: [Gait; 4] = [Gait::Original, Gait::Shamble, Gait::Heavy, Gait::Stumble];
        match self {
            WalkStyle::Mixed => ALL[((roll * ALL.len() as f32) as usize).min(ALL.len() - 1)],
            WalkStyle::Only(gait) => gait,
        }
    }

    pub(crate) const ALL: [(WalkStyle, &'static str); 5] = [
        (WalkStyle::Mixed, "mixed (each picks one)"),
        (WalkStyle::Only(Gait::Shamble), "shamble"),
        (WalkStyle::Only(Gait::Heavy), "heavy shamble"),
        (WalkStyle::Only(Gait::Stumble), "stumble"),
        (WalkStyle::Only(Gait::Original), "the model's own walk"),
    ];
}

/// Which runs zombies use, the same way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RunStyle {
    Mixed,
    Only(RunGait),
}

impl RunStyle {
    /// The run for the zombie whose pick is `roll` (`0..1`, its own).
    fn gait(self, roll: f32) -> RunGait {
        const ALL: [RunGait; 3] = [RunGait::Original, RunGait::Scary, RunGait::Broken];
        match self {
            RunStyle::Mixed => ALL[((roll * ALL.len() as f32) as usize).min(ALL.len() - 1)],
            RunStyle::Only(gait) => gait,
        }
    }

    pub(crate) const ALL: [(RunStyle, &'static str); 4] = [
        (RunStyle::Mixed, "mixed (each picks one)"),
        (RunStyle::Only(RunGait::Scary), "scary run"),
        (RunStyle::Only(RunGait::Broken), "broken run"),
        (RunStyle::Only(RunGait::Original), "the model's own run"),
    ];
}

impl ZombieAnimState {
    fn label(self) -> &'static str {
        match self {
            ZombieAnimState::Idle => "idle",
            ZombieAnimState::Walk(Gait::Original) => "walk (arms down)",
            ZombieAnimState::Walk(Gait::Shamble) => "shamble",
            ZombieAnimState::Walk(Gait::Heavy) => "heavy shamble",
            ZombieAnimState::Walk(Gait::Stumble) => "stumble",
            ZombieAnimState::WalkArmsUp => "walk (arms up)",
            ZombieAnimState::Run(RunGait::Original) => "run",
            ZombieAnimState::Run(RunGait::Scary) => "scary run",
            ZombieAnimState::Run(RunGait::Broken) => "broken run",
            ZombieAnimState::Attack => "attack",
            ZombieAnimState::FallBack => "falling back",
            ZombieAnimState::FallForward => "falling forward",
            ZombieAnimState::Climb { .. } => "climbing in",
            ZombieAnimState::CrawlFall => "crawler going down",
            ZombieAnimState::Crawl => "crawl",
            ZombieAnimState::CrawlAttack => "crawler swipe",
            ZombieAnimState::CrawlDead => "crawler dead",
        }
    }

    /// On the ground for good.
    fn crawler(self) -> bool {
        matches!(
            self,
            ZombieAnimState::CrawlFall | ZombieAnimState::Crawl | ZombieAnimState::CrawlAttack | ZombieAnimState::CrawlDead
        )
    }

    fn moving(self) -> bool {
        matches!(
            self,
            ZombieAnimState::Walk(_) | ZombieAnimState::WalkArmsUp | ZombieAnimState::Run(_)
        )
    }
}

/// Graph + node for each clip used, built once at startup.
#[derive(Resource, Clone)]
pub(crate) struct ZombieAnimations {
    graph: Handle<AnimationGraph>,
    idle: AnimationNodeIndex,
    /// The model's own walk (`Walk1_InPlace`), and the shambles made from it
    /// (`ShambleWalk`, `ShambleWalkHeavy` — `client/tools/zombie_walk_clips.py`).
    walk: AnimationNodeIndex,
    shamble: AnimationNodeIndex,
    heavy_shamble: AnimationNodeIndex,
    /// The four-step stumble (`StumbleWalk`, the same tool's).
    stumble: AnimationNodeIndex,
    /// The four-step scary run (`ScaryRun`, the same tool's, off `run`):
    /// lunging, clawing, twitching.
    scary_run: AnimationNodeIndex,
    /// The four-step broken run (`BrokenRun`, the same tool's, off `run`).
    broken_run: AnimationNodeIndex,
    walk_arms_up: AnimationNodeIndex,
    run: AnimationNodeIndex,
    attack: AnimationNodeIndex,
    fall_back: AnimationNodeIndex,
    fall_forward: AnimationNodeIndex,
    climb: AnimationNodeIndex,
    /// The crawler's (`CrawlFall`, `Crawl`, `CrawlAttack` — the same tool's).
    crawl_fall: AnimationNodeIndex,
    crawl: AnimationNodeIndex,
    crawl_attack: AnimationNodeIndex,
}

impl ZombieAnimations {
    fn node_for(&self, state: ZombieAnimState) -> AnimationNodeIndex {
        match state {
            ZombieAnimState::Idle => self.idle,
            ZombieAnimState::Walk(Gait::Original) => self.walk,
            ZombieAnimState::Walk(Gait::Shamble) => self.shamble,
            ZombieAnimState::Walk(Gait::Heavy) => self.heavy_shamble,
            ZombieAnimState::Walk(Gait::Stumble) => self.stumble,
            ZombieAnimState::WalkArmsUp => self.walk_arms_up,
            ZombieAnimState::Run(RunGait::Original) => self.run,
            ZombieAnimState::Run(RunGait::Scary) => self.scary_run,
            ZombieAnimState::Run(RunGait::Broken) => self.broken_run,
            ZombieAnimState::Attack => self.attack,
            ZombieAnimState::FallBack => self.fall_back,
            ZombieAnimState::FallForward => self.fall_forward,
            ZombieAnimState::Climb { .. } => self.climb,
            ZombieAnimState::CrawlFall => self.crawl_fall,
            ZombieAnimState::Crawl | ZombieAnimState::CrawlDead => self.crawl,
            ZombieAnimState::CrawlAttack => self.crawl_attack,
        }
    }
}

/// Panel-tunable zombie model + animation ("Zombies" debug-panel section).
/// The three moving clips play at `speed (m/s) × *_per_mps`, so a slow
/// zombie's legs slow down with it and the feet stay put on the ground —
/// dial each in until the feet stop sliding.
#[derive(Resource, Clone, Copy)]
pub(crate) struct ZombieAvatarSettings {
    pub(crate) scale: f32,
    /// Extra turn (degrees) so the model faces the way it walks.
    pub(crate) yaw_offset_deg: f32,
    /// `Walk1_InPlace` (arms down) playback speed per m/s of movement.
    pub(crate) walk_per_mps: f32,
    /// `Walk_InPlace` (arms up) playback speed per m/s.
    pub(crate) walk_arms_up_per_mps: f32,
    /// `Run_InPlace` playback speed per m/s.
    pub(crate) run_per_mps: f32,
    /// `Crawl` playback speed per m/s — and the least it plays at, lying
    /// still (still clawing at the ground, slowly).
    pub(crate) crawl_per_mps: f32,
    pub(crate) crawl_min_speed: f32,
    /// Flat playback speeds for the rest.
    pub(crate) idle_speed: f32,
    pub(crate) attack_speed: f32,
    pub(crate) death_speed: f32,
    /// Seconds each clip blends into the next.
    pub(crate) blend_secs: f32,
    /// Below this speed (m/s) a zombie that should be walking stands idle
    /// instead of treading in place (stuck on something, say).
    pub(crate) min_move_speed: f32,
    /// Which walks they use.
    pub(crate) walk_style: WalkStyle,
    /// Whether a walker near who it's after raises its arms
    /// (`Walk_InPlace`) — off, it keeps its own walk all the way in.
    /// (Off for now, to look the shambles over close up.)
    pub(crate) arms_up_walk: bool,
    /// Which runs they use.
    pub(crate) run_style: RunStyle,
}

impl Default for ZombieAvatarSettings {
    fn default() -> Self {
        Self {
            scale: 0.95,
            yaw_offset_deg: 180.0,
            walk_per_mps: 2.5,
            walk_arms_up_per_mps: 2.5,
            run_per_mps: 0.35,
            // (The crawl's hands drag back ~0.41 m/s at 1× —
            // `zombie_walk_clips.py`'s `CRAWL`.)
            crawl_per_mps: 2.46,
            crawl_min_speed: 0.3,
            idle_speed: 1.0,
            // `Attack` is 1 s long, the same as a swing on the server
            // (`shared::zombies::ZOMBIE_ATTACK_SECS`), so at 1× a looping
            // attack lines up with back-to-back swings.
            attack_speed: 1.0,
            death_speed: 1.0,
            blend_secs: 0.25,
            min_move_speed: 0.2,
            walk_style: WalkStyle::Mixed,
            arms_up_walk: false,
            run_style: RunStyle::Mixed,
        }
    }
}

/// The nearest zombie to us, for the debug panel's live readout (to match
/// the feet against).
#[derive(Resource, Default)]
pub(crate) struct ZombieAnimReadout {
    pub(crate) nearest: Option<(f32, f32, &'static str)>,
}

/// A zombie avatar's animation bookkeeping.
#[derive(Component, Default)]
pub(crate) struct ZombieMotion {
    prev_translation: Option<Vec3>,
    /// Horizontal speed (m/s), smoothed — interpolated poses arrive a little
    /// unevenly.
    speed: f32,
    state: ZombieAnimState,
}

impl ZombieMotion {
    /// How fast it's moving right now (m/s, horizontally, smoothed).
    pub(crate) fn speed(&self) -> f32 {
        self.speed
    }

    /// A crawler, dead (its crawl stopped where it lay) — `ragdoll` lets it
    /// go limp.
    pub(crate) fn crawler_dead(&self) -> bool {
        self.state == ZombieAnimState::CrawlDead
    }

    /// Dead and falling (or fallen): `Some(true)` face-first (a crawler
    /// too), `Some(false)` onto its back. `None` while it's still up.
    pub(crate) fn fell_forward(&self) -> Option<bool> {
        match self.state {
            ZombieAnimState::FallForward | ZombieAnimState::CrawlDead => Some(true),
            ZombieAnimState::FallBack => Some(false),
            _ => None,
        }
    }
}

pub(crate) struct ZombieAvatarPlugin;

impl Plugin for ZombieAvatarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZombieAvatarSettings>()
            .init_resource::<ZombieAnimReadout>()
            .add_systems(Startup, setup_zombie_assets)
            .add_systems(
                Update,
                animate_zombie_avatars.run_if(in_state(AppState::InGame)),
            );
    }
}

fn setup_zombie_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // `models/characters/zombie.glb`'s clips: 0 Idle, 1 Walk1_InPlace, 2 Walk_InPlace,
    // 3 Run_InPlace, 4 Attack, 5 FallingBack, 6 FallingForward, (7–9 unused,)
    // 10 Mantle — and 11 ShambleWalk, 12 ShambleWalkHeavy, 13 StumbleWalk,
    // 14 ScaryRun, 15 BrokenRun, 16 CrawlFall, 17 Crawl, 18 CrawlAttack, ours
    // (`client/tools/zombie_walk_clips.py`).
    let clip = |i: usize| -> Handle<AnimationClip> {
        asset_server.load(GltfAssetLabel::Animation(i).from_asset(ZOMBIE_MODEL))
    };
    let (graph, nodes) = AnimationGraph::from_clips((0..7).chain(10..=18).map(clip));
    commands.insert_resource(ZombieAnimations {
        graph: graphs.add(graph),
        idle: nodes[0],
        walk: nodes[1],
        walk_arms_up: nodes[2],
        run: nodes[3],
        attack: nodes[4],
        fall_back: nodes[5],
        fall_forward: nodes[6],
        climb: nodes[7],
        shamble: nodes[8],
        heavy_shamble: nodes[9],
        stumble: nodes[10],
        scary_run: nodes[11],
        broken_run: nodes[12],
        crawl_fall: nodes[13],
        crawl: nodes[14],
        crawl_attack: nodes[15],
    });
}

/// Spawn a zombie avatar following the `PlayerPose` on `src`.
pub(crate) fn spawn_zombie_avatar(
    commands: &mut Commands,
    asset_server: &AssetServer,
    settings: &ZombieAvatarSettings,
    src: Entity,
) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RemoteAvatar { src },
            ZombieMotion::default(),
            ZombieVisual,
            Transform::from_scale(Vec3::splat(settings.scale)),
            Visibility::default(),
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(ZOMBIE_MODEL))),
        ))
        .observe(start_zombie_animation);
}

/// Once a zombie's scene is in: loop `Idle` (it's climbing out of the
/// ground), and remember its `AnimationPlayer`.
pub(crate) fn start_zombie_animation(
    trigger: Trigger<SceneInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    mut players: Query<&mut AnimationPlayer>,
    anims: Res<ZombieAnimations>,
    mut bounds: super::CharacterBounds,
) {
    let root = trigger.target();
    for entity in children.iter_descendants(root) {
        bounds.fit(&mut commands, root, entity);
        if let Ok(mut player) = players.get_mut(entity) {
            let mut transitions = AnimationTransitions::new();
            transitions
                .play(&mut player, anims.idle, Duration::ZERO)
                .set_repeat(RepeatAnimation::Forever);
            commands
                .entity(entity)
                .insert((AnimationGraphHandle(anims.graph.clone()), transitions));
            commands.entity(root).insert(ZombieAnimationPlayer(entity));
        }
    }
}

/// Pick each zombie's clip from what the server says it's doing, blend into
/// it, and keep the moving clips' speed in step with how fast it's really
/// going. Dead: a random fall (the same one on every client), held.
#[allow(clippy::type_complexity)]
fn animate_zombie_avatars(
    time: Res<Time>,
    anims: Option<Res<ZombieAnimations>>,
    settings: Res<ZombieAvatarSettings>,
    poses: Query<(&PlayerPose, Option<&PlayerId>)>,
    me: Query<&Transform, With<Player>>,
    // (Not a ragdoll — its animation's paused for good, `ragdoll`.)
    mut avatars: Query<
        (
            &RemoteAvatar,
            &mut ZombieMotion,
            &ZombieAnimationPlayer,
            Has<crate::melee_predict::PredictedDeath>,
        ),
        Without<crate::ragdoll::Ragdoll>,
    >,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    mut readout: ResMut<ZombieAnimReadout>,
) {
    let Some(anims) = anims else { return };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let blend = Duration::from_secs_f32(settings.blend_secs.max(0.0));
    let me = me.single().ok().map(|t| t.translation);
    let mut nearest: Option<(f32, f32, f32, &'static str)> = None;

    for (avatar, mut motion, anim_player, predicted_death) in &mut avatars {
        let Ok((pose, id)) = poses.get(avatar.src) else {
            continue;
        };

        // Horizontal only — a zombie climbing out of the ground isn't walking.
        let raw = match motion.prev_translation {
            Some(prev) => {
                let d = pose.translation - prev;
                Vec2::new(d.x, d.z).length() / dt
            }
            None => 0.0,
        };
        motion.prev_translation = Some(pose.translation);
        let k = 1.0 - (-dt / 0.12).exp();
        motion.speed += (raw - motion.speed) * k;

        // Its own picks — its fall, its gait — the same on every client: off
        // its (shared) peer id.
        let seed = id.map_or(avatar.src.to_bits(), |id| id.0.to_bits());
        let seed = (seed ^ (seed >> 32)) as u32;

        // (Dead the moment our knife killed it, here — `melee_predict`.)
        let state = if !crate::melee_predict::alive(pose, predicted_death) {
            if matches!(motion.state, ZombieAnimState::FallBack | ZombieAnimState::FallForward) {
                motion.state
            } else if motion.state.crawler() || pose.zombie.is_crawler() {
                // (Already down: it dies where it lies.)
                ZombieAnimState::CrawlDead
            } else {
                if crate::util::rand01(seed) < 0.5 {
                    ZombieAnimState::FallBack
                } else {
                    ZombieAnimState::FallForward
                }
            }
        } else {
            let wanted = match pose.zombie {
                ZombieAnim::Walk => {
                    ZombieAnimState::Walk(settings.walk_style.gait(crate::util::rand01(seed ^ 0x9e37_79b9)))
                }
                ZombieAnim::WalkArmsUp if settings.arms_up_walk => ZombieAnimState::WalkArmsUp,
                // (Arms-up walk off: its own walk all the way in.)
                ZombieAnim::WalkArmsUp => {
                    ZombieAnimState::Walk(settings.walk_style.gait(crate::util::rand01(seed ^ 0x9e37_79b9)))
                }
                ZombieAnim::Run => ZombieAnimState::Run(settings.run_style.gait(crate::util::rand01(seed ^ 0x85eb_ca6b))),
                ZombieAnim::Attack => ZombieAnimState::Attack,
                ZombieAnim::Climb => ZombieAnimState::Climb { run: false },
                ZombieAnim::ClimbRun => ZombieAnimState::Climb { run: true },
                ZombieAnim::CrawlFall => ZombieAnimState::CrawlFall,
                ZombieAnim::Crawl => ZombieAnimState::Crawl,
                ZombieAnim::CrawlAttack => ZombieAnimState::CrawlAttack,
                // (A hellhound gets its own avatar — `dogs`.)
                ZombieAnim::Idle
                | ZombieAnim::Rising
                | ZombieAnim::None
                | ZombieAnim::Dog
                | ZombieAnim::BossIdle
                | ZombieAnim::BossWalk
                | ZombieAnim::BossSmash
                | ZombieAnim::BossBlast => {
                    ZombieAnimState::Idle
                }
            };
            // Only tread along while actually getting somewhere (a little
            // hysteresis so it doesn't flicker at the threshold).
            let threshold = if motion.state.moving() {
                settings.min_move_speed * 0.5
            } else {
                settings.min_move_speed
            };
            if wanted.moving() && motion.speed < threshold {
                ZombieAnimState::Idle
            } else {
                wanted
            }
        };

        let Ok((mut player, mut transitions)) = players.get_mut(anim_player.0) else {
            continue;
        };
        if state != motion.state {
            let was = motion.state;
            motion.state = state;
            let repeat = match state {
                ZombieAnimState::FallBack
                | ZombieAnimState::FallForward
                | ZombieAnimState::Climb { .. }
                | ZombieAnimState::CrawlFall => RepeatAnimation::Never,
                _ => RepeatAnimation::Forever,
            };
            // (A crawler dying mid-crawl just stops where it is.)
            if !(state == ZombieAnimState::CrawlDead && was == ZombieAnimState::Crawl) {
                let active = transitions
                    .play(&mut player, anims.node_for(state), blend)
                    .set_repeat(repeat);
                // (From the top, each window — even one climbed a moment ago.)
                if matches!(state, ZombieAnimState::Climb { .. }) {
                    active.replay();
                }
            }
        }
        // Every frame, so the panel's sliders and speed changes apply live.
        let speed = motion.speed;
        let rate = match motion.state {
            ZombieAnimState::Idle => settings.idle_speed,
            // (The heavy shamble's strides are longer: slower legs, same speed.)
            ZombieAnimState::Walk(Gait::Heavy) => speed * settings.walk_per_mps / HEAVY_STRIDE,
            ZombieAnimState::Walk(Gait::Stumble) => speed * settings.walk_per_mps / STUMBLE_STRIDE,
            ZombieAnimState::Walk(_) => speed * settings.walk_per_mps,
            ZombieAnimState::WalkArmsUp => speed * settings.walk_arms_up_per_mps,
            // (The scary run's strides are longer: slower legs, same speed.)
            ZombieAnimState::Run(RunGait::Scary) => speed * settings.run_per_mps / SCARY_RUN_STRIDE,
            ZombieAnimState::Run(RunGait::Broken) => speed * settings.run_per_mps / BROKEN_RUN_STRIDE,
            ZombieAnimState::Run(RunGait::Original) => speed * settings.run_per_mps,
            ZombieAnimState::Attack => settings.attack_speed,
            ZombieAnimState::FallBack | ZombieAnimState::FallForward => settings.death_speed,
            // (The clip is as long as a walker's climb.)
            ZombieAnimState::Climb { run } => {
                let secs = if run { shared::windows::RUN_CLIMB_SECS } else { shared::windows::CLIMB_SECS };
                shared::windows::CLIMB_SECS / secs
            }
            // (The clip's as long as the server's fall, and its swipe as a
            // swing.)
            ZombieAnimState::CrawlFall | ZombieAnimState::CrawlAttack => settings.attack_speed,
            ZombieAnimState::Crawl => (speed * settings.crawl_per_mps).max(settings.crawl_min_speed),
            ZombieAnimState::CrawlDead => 0.0,
        };
        if let Some(active) = player.animation_mut(anims.node_for(motion.state)) {
            let rate = if motion.state == ZombieAnimState::CrawlDead { 0.0 } else { rate.max(0.01) };
            active.set_speed(rate);
        }

        if let Some(me) = me {
            let d = me.distance(pose.translation);
            if nearest.is_none_or(|(n, ..)| d < n) {
                nearest = Some((d, speed, rate, motion.state.label()));
            }
        }
    }
    readout.nearest = nearest.map(|(_, speed, rate, label)| (speed, rate, label));
}
