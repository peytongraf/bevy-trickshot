//! `Zombies` kill chains, the modern Call of Duty zombies way (Cold War's
//! medals, topped off with Black Ops 6's last two): kill fast enough —
//! each kill within [`KillChainSettings::window_secs`] of the one before —
//! and every [`TIERS`] count the chain passes earns its medal, up to
//! Lifedrinker at 30.
//!
//! The chain counts our `LobbyMember::kills` as the server replicates it,
//! so every kill counts whatever made it (a gun, the knife, a monkey bomb,
//! a molotov…). A burst that passes several counts at once (a monkey bomb
//! taking out a crowd) earns every medal it passed, not just the best:
//! they queue and slam in one after another.
//!
//! The look is Cold War's: top centre, under the compass, the medal slams in
//! big and settles with its name under it; when the next one lands, the last
//! shrinks off to its right for a moment, then fades. One sound per medal
//! (`GameSounds::kill_chain_medal`, flat and dry).
//!
//! Everything's reset on entering and leaving a game, and the medals are
//! `StateScoped(InGame)`.

use std::collections::VecDeque;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::Lobby;

use crate::audio_engine::prelude::*;
use crate::net::GameClient;
use crate::zombies_hud::zombies_game;
use crate::{menu, AppState, GameSounds, HUD_FONT};

/// Each medal: its name and the chain length that earns it, in order.
pub(crate) const TIERS: [(&str, u32); 6] = [
    ("SLAUGHTER", 5),
    ("CARNAGE", 10),
    ("MASSACRE", 15),
    ("BLOODBATH", 20),
    ("EXTERMINATION", 25),
    ("LIFEDRINKER", 30),
];

/// Each medal's icon (`textures/hud/kill_chains/`), in [`TIERS`]' order.
const ICONS: [&str; 6] = [
    "textures/hud/kill_chains/slaughter.png",
    "textures/hud/kill_chains/carnage.png",
    "textures/hud/kill_chains/massacre.png",
    "textures/hud/kill_chains/bloodbath.png",
    "textures/hud/kill_chains/extermination.png",
    "textures/hud/kill_chains/lifedrinker.png",
];

/// Each medal's box (logical px wide) — room for the longest name.
const MEDAL_WIDTH: f32 = 320.0;

pub(crate) struct KillChainsPlugin;

impl Plugin for KillChainsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KillChainSettings>()
            .init_resource::<KillChain>()
            .add_systems(OnEnter(AppState::InGame), (reset_chain, spawn_root))
            .add_systems(OnExit(AppState::InGame), reset_chain)
            .add_systems(
                Update,
                (track_chain, show_next_medal, animate_medals, place_root)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// Panel-adjustable kill chain timing and look ("Kill chains (Zombies)"
/// debug-panel section).
#[derive(Resource, Clone, Debug)]
pub(crate) struct KillChainSettings {
    /// Longest gap (s) between two kills that still keeps the chain going.
    pub(crate) window_secs: f32,
    /// Gap (s) between medals earned at once, each slamming in in turn.
    pub(crate) stagger_secs: f32,
    /// How long (s) a medal takes to slam in...
    pub(crate) slam_secs: f32,
    /// ...from this many times its size.
    pub(crate) slam_scale: f32,
    /// How long (s) the newest medal stays up before fading.
    pub(crate) hold_secs: f32,
    /// How long (s) a medal takes to fade out.
    pub(crate) fade_secs: f32,
    /// When the next one lands: how long (s) the last takes to move aside...
    pub(crate) push_secs: f32,
    /// ...how far right (px) it goes...
    pub(crate) push_x: f32,
    /// ...what size it shrinks to...
    pub(crate) push_scale: f32,
    /// ...and how long (s) it stays there before fading.
    pub(crate) pushed_hold_secs: f32,
    /// The icon's height (px) at rest.
    pub(crate) icon_size: f32,
    /// The name's size (px) at rest.
    pub(crate) name_size: f32,
    /// Gap (px) from the compass down to the medals.
    pub(crate) top_offset: f32,
}

impl Default for KillChainSettings {
    fn default() -> Self {
        Self {
            window_secs: 1.5,
            stagger_secs: 0.45,
            slam_secs: 0.2,
            slam_scale: 2.4,
            hold_secs: 2.5,
            fade_secs: 0.35,
            push_secs: 0.18,
            push_x: 150.0,
            push_scale: 0.55,
            pushed_hold_secs: 1.0,
            icon_size: 165.0,
            name_size: 26.0,
            top_offset: 10.0,
        }
    }
}

/// The chain so far, and the medals earned but not shown yet.
#[derive(Resource, Default)]
pub(crate) struct KillChain {
    /// Our kills the last we looked (`None` outside a `Zombies` game).
    last_kills: Option<u32>,
    /// Kills in the current chain.
    count: u32,
    /// Seconds since the chain's last kill.
    since_kill: f32,
    /// Medals ([`TIERS`] indices) waiting to slam in, oldest first...
    queue: VecDeque<usize>,
    /// ...and how long (s) until the next may.
    next_in: f32,
    /// Kills the debug panel's test buttons add to the chain.
    test_kills: u32,
}

fn reset_chain(mut chain: ResMut<KillChain>) {
    *chain = KillChain::default();
}

/// The strip under the compass the medals sit in.
#[derive(Component)]
struct KillChainRoot;

/// One medal on screen.
#[derive(Component)]
struct Medal {
    icon: Entity,
    name: Entity,
    /// Seconds since it started slamming in.
    age: f32,
    /// Its age when the next medal pushed it aside...
    pushed_at: Option<f32>,
    /// ...and when the one after that sent it off.
    gone_at: Option<f32>,
}

fn spawn_root(mut commands: Commands) {
    commands.spawn((
        StateScoped(AppState::InGame),
        KillChainRoot,
        menu::HudElement,
        GlobalZIndex(6),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(crate::hud::COMPASS_HEIGHT),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

/// Count the chain off our kills, and queue every medal it passes.
fn track_chain(
    time: Res<Time>,
    settings: Res<KillChainSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut chain: ResMut<KillChain>,
) {
    let me = local.iter().next().map(|l| l.0);
    let kills = zombies_game(&local, &lobbies)
        .and_then(|lobby| lobby.members.iter().find(|m| Some(m.peer) == me))
        .map(|m| m.kills);
    let mut gained = std::mem::take(&mut chain.test_kills);
    match (kills, chain.last_kills) {
        // (A count that went down is a new game's: just the new baseline.)
        (Some(now), Some(last)) if now > last => gained += now - last,
        // The game's over: nothing carries into the next.
        (None, Some(_)) => {
            chain.count = 0;
            chain.queue.clear();
        }
        _ => {}
    }
    chain.last_kills = kills;
    chain.since_kill += time.delta_secs();
    if gained == 0 {
        return;
    }
    if chain.since_kill > settings.window_secs {
        chain.count = 0;
    }
    chain.since_kill = 0.0;
    let before = chain.count;
    chain.count += gained;
    let count = chain.count;
    for (tier, (_, needed)) in TIERS.iter().enumerate() {
        if before < *needed && count >= *needed {
            chain.queue.push_back(tier);
        }
    }
}

/// Slam the next queued medal in (one every `stagger_secs`), moving the
/// ones already up out of its way.
fn show_next_medal(
    time: Res<Time>,
    settings: Res<KillChainSettings>,
    mut chain: ResMut<KillChain>,
    root: Option<Single<Entity, With<KillChainRoot>>>,
    mut medals: Query<&mut Medal>,
    sounds: Res<GameSounds>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    chain.next_in = (chain.next_in - time.delta_secs()).max(0.0);
    if chain.next_in > 0.0 {
        return;
    }
    let Some(root) = root else {
        return;
    };
    let Some(tier) = chain.queue.pop_front() else {
        return;
    };
    chain.next_in = settings.stagger_secs;

    for mut medal in &mut medals {
        let age = medal.age;
        if medal.pushed_at.is_none() {
            medal.pushed_at = Some(age);
        } else if medal.gone_at.is_none() {
            medal.gone_at = Some(age);
        }
    }

    commands.spawn((AudioPlayer::new(sounds.kill_chain_medal.clone()), PlaybackSettings::DESPAWN));

    let icon = commands
        .spawn((
            ImageNode {
                image: asset_server.load(ICONS[tier]),
                color: Color::NONE,
                ..default()
            },
            Node {
                height: Val::Px(settings.icon_size),
                ..default()
            },
        ))
        .id();
    let name = commands
        .spawn((
            Text::new(TIERS[tier].0),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: settings.name_size,
                ..default()
            },
            TextColor(Color::NONE),
            TextShadow {
                offset: Vec2::splat(2.0),
                color: Color::NONE,
            },
        ))
        .id();
    commands
        .spawn((
            Medal {
                icon,
                name,
                age: 0.0,
                pushed_at: None,
                gone_at: None,
            },
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Percent(50.0),
                width: Val::Px(MEDAL_WIDTH),
                margin: UiRect::left(Val::Px(-MEDAL_WIDTH / 2.0)),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Transform::from_scale(Vec3::splat(settings.slam_scale)),
            Pickable::IGNORE,
            ChildOf(*root),
        ))
        .add_children(&[icon, name]);
}

/// Each medal's slam in, its move aside, and its fade — then it goes.
fn animate_medals(
    time: Res<Time>,
    settings: Res<KillChainSettings>,
    mut medals: Query<(Entity, &mut Medal, &mut Node, &mut Transform)>,
    mut icons: Query<&mut ImageNode>,
    mut names: Query<(&mut TextColor, &mut TextShadow)>,
    mut commands: Commands,
) {
    let s = &*settings;
    for (entity, mut medal, mut node, mut transform) in &mut medals {
        medal.age += time.delta_secs();
        let age = medal.age;

        // Slam in: from big and clear down to its size, fast, slowing into it.
        let slam = 1.0 - (1.0 - (age / s.slam_secs.max(0.01)).clamp(0.0, 1.0)).powi(3);
        let mut scale = s.slam_scale.lerp(1.0, slam);
        let appear = (age / (s.slam_secs * 0.6).max(0.01)).clamp(0.0, 1.0);

        // Aside: off to the right, smaller.
        let push = medal
            .pushed_at
            .map_or(0.0, |at| crate::util::ease((age - at) / s.push_secs.max(0.01)));
        scale *= 1.0f32.lerp(s.push_scale, push);
        node.margin.left = Val::Px(-MEDAL_WIDTH / 2.0 + s.push_x * push);

        // Fade: once it's been up long enough, aside long enough, or sent off.
        let mut fade_from = medal.pushed_at.map_or(s.hold_secs, |at| at + s.pushed_hold_secs);
        if let Some(at) = medal.gone_at {
            fade_from = fade_from.min(at);
        }
        let fade = 1.0 - ((age - fade_from) / s.fade_secs.max(0.01)).clamp(0.0, 1.0);
        if age > fade_from && fade <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let alpha = appear * fade;

        transform.scale = Vec3::splat(scale);
        if let Ok(mut icon) = icons.get_mut(medal.icon) {
            icon.color = Color::srgba(1.0, 1.0, 1.0, alpha);
        }
        if let Ok((mut color, mut shadow)) = names.get_mut(medal.name) {
            color.0 = Color::srgba(0.93, 0.92, 0.88, alpha);
            shadow.color = Color::srgba(0.0, 0.0, 0.0, 0.8 * alpha);
        }
    }
}

/// Keep the medals under the compass — and under the exfil banner while
/// that's up, rather than over it.
fn place_root(
    settings: Res<KillChainSettings>,
    banner: Query<(&ComputedNode, &InheritedVisibility), With<crate::exfil::ExfilBanner>>,
    mut root: Query<&mut Node, With<KillChainRoot>>,
) {
    let below_banner = banner
        .iter()
        .find(|(_, vis)| vis.get())
        .map_or(0.0, |(node, _)| node.size().y * node.inverse_scale_factor() + 8.0);
    let top = Val::Px(crate::hud::COMPASS_HEIGHT + settings.top_offset + below_banner);
    for mut node in &mut root {
        if node.top != top {
            node.top = top;
        }
    }
}

/// The debug panel's "Kill chains (Zombies)" section.
#[derive(SystemParam)]
pub(crate) struct KillChainDebug<'w> {
    settings: ResMut<'w, KillChainSettings>,
    chain: ResMut<'w, KillChain>,
}

impl KillChainDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label(format!(
            "Chain: {} kills ({:.1}s since the last)",
            self.chain.count, self.chain.since_kill
        ));
        ui.horizontal(|ui| {
            if ui.button("Test: +1 kill").clicked() {
                self.chain.test_kills += 1;
            }
            if ui.button("Test: +5 kills").clicked() {
                self.chain.test_kills += 5;
            }
            if ui.button("Test: +30 at once").clicked() {
                self.chain.test_kills += 30;
            }
        });
        let s = &mut *self.settings;
        ui.add(egui::Slider::new(&mut s.window_secs, 0.2f32..=5.0).text("max gap between kills (s)"));
        ui.add(egui::Slider::new(&mut s.stagger_secs, 0.05f32..=1.5).text("gap between medals earned at once (s)"));
        ui.separator();
        ui.add(egui::Slider::new(&mut s.slam_secs, 0.02f32..=1.0).text("slam in (s)"));
        ui.add(egui::Slider::new(&mut s.slam_scale, 1.0f32..=5.0).text("slam in from (× size)"));
        ui.add(egui::Slider::new(&mut s.hold_secs, 0.2f32..=6.0).text("newest stays up (s)"));
        ui.add(egui::Slider::new(&mut s.fade_secs, 0.02f32..=2.0).text("fade out (s)"));
        ui.add(egui::Slider::new(&mut s.push_secs, 0.02f32..=1.0).text("move aside (s)"));
        ui.add(egui::Slider::new(&mut s.push_x, 0.0f32..=400.0).text("aside, to the right (px)"));
        ui.add(egui::Slider::new(&mut s.push_scale, 0.1f32..=1.0).text("aside, size (×)"));
        ui.add(egui::Slider::new(&mut s.pushed_hold_secs, 0.0f32..=4.0).text("stays aside (s)"));
        ui.separator();
        ui.add(egui::Slider::new(&mut s.icon_size, 20.0f32..=300.0).text("icon height (px, new medals)"));
        ui.add(egui::Slider::new(&mut s.name_size, 8.0f32..=72.0).text("name size (px, new medals)"));
        ui.add(egui::Slider::new(&mut s.top_offset, -40.0f32..=300.0).text("below the compass (px)"));
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                *s = KillChainSettings::default();
            }
            if ui.button("Print settings to console").clicked() {
                info!("{s:#?}");
            }
        });
    }
}
