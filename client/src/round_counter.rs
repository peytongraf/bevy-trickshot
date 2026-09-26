//! `Zombies`' round number (top right, Call of Duty red) and what happens when
//! a round starts: the round-start sound plays for everyone (not
//! positional), and the number slides to the middle of the screen, turns
//! into the new round, swells up, then eases back down as it slides home.
//! Round 1 skips the slide — there's no old number — and just swells up in
//! the middle before settling into the corner.
//!
//! Everything goes off the replicated `Lobby::round`. The counter is
//! `StateScoped(InGame)` and keeps its own state, so each game starts fresh.

use bevy::prelude::*;
use lightyear::prelude::LocalId;
use shared::Lobby;

use crate::net::GameClient;
use crate::zombies_hud::zombies_game;
use crate::{AppState, GameSounds, HUD_FONT};

pub(crate) struct RoundCounterPlugin;

impl Plugin for RoundCounterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RoundAnimSettings>()
            .add_systems(OnEnter(AppState::InGame), spawn_round_counter)
            .add_systems(Update, update_round_counter.run_if(in_state(AppState::InGame)));
    }
}

/// Panel-tunable round animation ("Zombies round counter").
#[derive(Resource, Clone)]
pub(crate) struct RoundAnimSettings {
    /// Resting font size (px).
    pub(crate) size: f32,
    /// Seconds to slide to the middle...
    pub(crate) slide_in_secs: f32,
    /// ...to swell up to `peak_scale` (as the new number)...
    pub(crate) grow_secs: f32,
    /// ...to hold there...
    pub(crate) hold_secs: f32,
    /// ...and to ease back down while sliding home.
    pub(crate) return_secs: f32,
    /// Size at the top of the swell, as a multiple of `size`.
    pub(crate) peak_scale: f32,
    /// Debug: play it again (with the sound) next frame.
    pub(crate) replay: bool,
}

impl Default for RoundAnimSettings {
    fn default() -> Self {
        Self {
            size: 84.0,
            slide_in_secs: 0.7,
            grow_secs: 0.6,
            hold_secs: 1.2,
            return_secs: 1.0,
            peak_scale: 2.6,
            replay: false,
        }
    }
}

/// Call of Duty zombies' round red.
const ROUND_RED: Color = Color::srgb(0.72, 0.04, 0.04);
/// The corner it rests in: px from the screen's right / top edges.
const HOME_RIGHT: f32 = 28.0;
const HOME_TOP: f32 = 10.0;

/// The full-width strip the number moves along.
#[derive(Component)]
struct RoundStrip;

/// The number, and where its animation is up to.
#[derive(Component, Default)]
struct RoundCounter {
    /// The round it's settled on (0 = none yet).
    round: u32,
    /// The round it shows until the swell starts, while an animation runs.
    old: u32,
    /// Seconds into the running animation, if one is.
    anim: Option<f32>,
}

fn spawn_round_counter(mut commands: Commands, asset_server: Res<AssetServer>, settings: Res<RoundAnimSettings>) {
    commands
        .spawn((
            StateScoped(AppState::InGame),
            RoundStrip,
            GlobalZIndex(6),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                ..default()
            },
        ))
        .with_child((
            RoundCounter::default(),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(HOME_TOP),
                ..default()
            },
            Text::new(""),
            TextFont {
                font: asset_server.load(HUD_FONT),
                font_size: settings.size,
                ..default()
            },
            TextColor(ROUND_RED),
            TextShadow {
                offset: Vec2::splat(2.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.75),
            },
            Visibility::Hidden,
        ));
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_round_counter(
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<GameSounds>>,
    mut settings: ResMut<RoundAnimSettings>,
    strip: Single<&ComputedNode, With<RoundStrip>>,
    counter: Single<
        (&mut RoundCounter, &mut Node, &mut Text, &mut TextFont, &mut Visibility, &ComputedNode),
        Without<RoundStrip>,
    >,
    mut commands: Commands,
) {
    let (mut c, mut node, mut text, mut font, mut vis, computed) = counter.into_inner();
    let round = zombies_game(&local, &lobbies).map_or(0, |l| l.round);

    // A new round (or the panel's replay): sound, and start the animation.
    let replay = settings.replay && c.round > 0;
    if settings.replay {
        settings.replay = false;
    }
    if (round > 0 && round != c.round) || replay {
        c.old = if replay { c.round.saturating_sub(1) } else { c.round };
        c.round = round.max(c.round);
        // No old number to slide over: start in the middle, swelling.
        c.anim = Some(if c.old == 0 { settings.slide_in_secs } else { 0.0 });
        if let Some(sounds) = &sounds {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.round_start.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
    if c.round == 0 {
        vis.set_if_neq(Visibility::Hidden);
        return;
    }
    vis.set_if_neq(Visibility::Inherited);

    let s = &*settings;
    // How far toward the middle (0 = home) and how big (1 = resting).
    let (to_middle, scale, shown) = match c.anim {
        None => (0.0, 1.0, c.round),
        Some(t) => {
            let grow_at = s.slide_in_secs;
            let hold_at = grow_at + s.grow_secs;
            let return_at = hold_at + s.hold_secs;
            let peak = s.peak_scale;
            if t < grow_at {
                (ease(t / s.slide_in_secs.max(1e-3)), 1.0, c.old)
            } else if t < hold_at {
                let k = ease((t - grow_at) / s.grow_secs.max(1e-3));
                (1.0, 1.0 + (peak - 1.0) * k, c.round)
            } else if t < return_at {
                (1.0, peak, c.round)
            } else {
                let k = ease((t - return_at) / s.return_secs.max(1e-3));
                (1.0 - k, peak + (1.0 - peak) * k, c.round)
            }
        }
    };
    if let Some(t) = &mut c.anim {
        *t += time.delta_secs();
        if *t >= s.slide_in_secs + s.grow_secs + s.hold_secs + s.return_secs {
            c.anim = None;
        }
    }

    let wanted = if shown == 0 { String::new() } else { shown.to_string() };
    if text.0 != wanted {
        text.0 = wanted;
    }
    let size = s.size * scale;
    if font.font_size != size {
        font.font_size = size;
    }
    // Layout sizes are physical pixels; `Node` offsets are logical.
    let screen_w = strip.size().x * strip.inverse_scale_factor();
    let own = computed.size() * computed.inverse_scale_factor();
    let middle_left = (screen_w - own.x) * 0.5;
    let home_left = screen_w - own.x - HOME_RIGHT;
    let left = home_left + (middle_left - home_left) * to_middle;
    // Swell about its middle rather than growing down from its top edge.
    let resting_h = own.y / scale.max(1e-3);
    let top = HOME_TOP - (own.y - resting_h) * 0.5;
    if node.left != Val::Px(left) {
        node.left = Val::Px(left);
    }
    if node.top != Val::Px(top.max(0.0)) {
        node.top = Val::Px(top.max(0.0));
    }
}
