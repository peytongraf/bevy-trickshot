//! `Zombies` operator quotes, as heard: the server picks who says what
//! ([`shared::QuoteSaid`], `server::quotes`), and every client plays that
//! very line — from where its speaker is, following them about and fading
//! with distance ("Remote sounds" range), or, our own, in our head.
//!
//! Two things only this client knows of it asks the server to say for us
//! ([`shared::RequestQuote`]): being down to our last magazine, and out of
//! ammo altogether ([`ammo_quotes`]).
//!
//! The lines are `audio/quotes/<operator>/…`, every one listed (with how
//! long it runs) in `lines.txt` there — [`shared::quotes::lines`] — all
//! loaded at startup. How loud they are (perk quotes too), and how long after
//! a dog round begins and the exfil's called someone speaks up, is the
//! debug panel's "Quotes" window ([`quotes_debug_ui`]).
//!
//! The lines playing are `StateScoped(InGame)`; nothing else carries between
//! games.

use std::collections::HashMap;

use bevy::audio::Volume;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use lightyear::prelude::{LocalId, MessageReceiver, PeerId, TriggerSender};
use shared::quotes::{lines, Quote, DOG_ROUND_DELAY_SECS, EXFIL_DELAY_SECS};
use shared::weapon::SlotWeapon;
use shared::{Lobby, PlayerId, PlayerPose};

use crate::net::GameClient;
use crate::{menu, AppState};

pub(crate) struct QuotesPlugin;

impl Plugin for QuotesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QuoteSettings>()
            .add_systems(Startup, load_lines)
            .add_systems(
                Update,
                (play_quotes, follow_quotes, ammo_quotes, send_quote_delays).run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                quotes_debug_ui.run_if(menu::debug_enabled.and(in_state(AppState::InGame))),
            );
    }
}

/// How the quotes sound — the "Quotes" debug window.
#[derive(Resource, Clone, PartialEq)]
pub(crate) struct QuoteSettings {
    /// Every operator quote's volume (perk quotes' too, on top of their own
    /// "perk quote" volume).
    pub(crate) volume: f32,
    /// How long (s) after a dog round begins, and after the exfil's called,
    /// someone says so (the leader's count — the server's told).
    pub(crate) dog_round_delay: f32,
    pub(crate) exfil_delay: f32,
}

impl Default for QuoteSettings {
    fn default() -> Self {
        Self {
            volume: 2.0,
            dog_round_delay: DOG_ROUND_DELAY_SECS,
            exfil_delay: EXFIL_DELAY_SECS,
        }
    }
}

/// Every line, loaded, by its path under `audio/quotes/`.
#[derive(Resource, Default)]
struct QuoteLines(HashMap<&'static str, Handle<AudioSource>>);

fn load_lines(asset_server: Res<AssetServer>, mut commands: Commands) {
    let mut loaded = HashMap::new();
    for op in shared::operator::Operator::ALL {
        for quote in shared::quotes::all() {
            for line in lines(op, quote) {
                loaded
                    .entry(line.path)
                    .or_insert_with(|| asset_server.load(format!("audio/quotes/{}", line.path)));
            }
        }
    }
    commands.insert_resource(QuoteLines(loaded));
}

/// Someone else's line, playing from them (`speaker`).
#[derive(Component)]
struct SpokenQuote {
    speaker: PeerId,
}

/// The server says someone's operator said something: play that line —
/// ours in our head, anyone else's from them.
#[allow(clippy::too_many_arguments)]
fn play_quotes(
    mut receivers: Query<&mut MessageReceiver<shared::QuoteSaid>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    loaded: Option<Res<QuoteLines>>,
    settings: Res<QuoteSettings>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    for mut rx in &mut receivers {
        for said in rx.receive() {
            let Some(lobby) = me.and_then(|me| lobbies.iter().find(|l| l.has(me))) else {
                continue;
            };
            let Some(member) = lobby.members.iter().find(|m| m.peer == said.speaker) else {
                continue;
            };
            let Some(line) = lines(member.operator, said.quote).get(said.line as usize) else {
                continue;
            };
            let Some(clip) = loaded.as_ref().and_then(|l| l.0.get(line.path)) else {
                continue;
            };
            if Some(said.speaker) == me {
                commands.spawn((
                    Name::new("Quote"),
                    StateScoped(AppState::InGame),
                    AudioPlayer::new(clip.clone()),
                    // (`GlobalVolume` is multiplied in at spawn.)
                    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(settings.volume.max(0.0))),
                ));
            } else {
                let at = poses
                    .iter()
                    .find(|(id, _)| id.0 == said.speaker)
                    .map_or(Vec3::ZERO, |(_, pose)| pose.translation);
                commands.spawn((
                    Name::new("Quote"),
                    StateScoped(AppState::InGame),
                    SpokenQuote { speaker: said.speaker },
                    // (Its loudness is ours to set — `follow_quotes`.)
                    crate::RemoteSoundEmitter,
                    AudioPlayer::new(clip.clone()),
                    Transform::from_translation(at),
                    crate::positional_playback(Volume::Linear(0.0)),
                ));
            }
        }
    }
}

/// Keep each line someone else is saying with them, as loud as how far off
/// they are.
fn follow_quotes(
    listener: Query<&GlobalTransform, With<crate::WorldModelCamera>>,
    poses: Query<(&PlayerId, &PlayerPose)>,
    settings: Res<QuoteSettings>,
    remote: Res<crate::RemoteSoundSettings>,
    global: Res<GlobalVolume>,
    mut quotes: Query<(&SpokenQuote, &mut Transform, Option<&mut SpatialAudioSink>)>,
) {
    let Ok(ear) = listener.single() else {
        return;
    };
    for (quote, mut tf, sink) in &mut quotes {
        if let Some((_, pose)) = poses.iter().find(|(id, _)| id.0 == quote.speaker) {
            tf.translation = pose.translation;
        }
        if let Some(mut sink) = sink {
            let loudness = settings.volume * crate::distance_falloff(ear.translation().distance(tf.translation), &remote);
            sink.set_volume(Volume::Linear(loudness.max(0.0)) * global.volume);
        }
    }
}

/// How the gun in hand's ammo stands, for [`ammo_quotes`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum AmmoState {
    Plenty,
    /// Its last magazine (nothing left in reserve).
    LastMag,
    Out,
}

fn ammo_state(mag: u32, reserve: u32) -> AmmoState {
    match (mag, reserve) {
        (0, 0) => AmmoState::Out,
        (_, 0) => AmmoState::LastMag,
        _ => AmmoState::Plenty,
    }
}

/// The gun in our hands just ran down to its last magazine, or out: ask the
/// server for our operator to say so. (Only as it gets worse — not on
/// swapping to a gun that already was.)
fn ammo_quotes(
    weapon: Res<crate::Weapon>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut sender: Query<&mut TriggerSender<shared::RequestQuote>, With<GameClient>>,
    mut last: Local<Option<(SlotWeapon, AmmoState)>>,
) {
    if crate::zombies_hud::zombies_game(&local, &lobbies).is_none() {
        *last = None;
        return;
    }
    let held = weapon.held_weapon();
    let state = match held {
        SlotWeapon::Gun(_) => {
            let (mag, reserve) = weapon.held_ammo();
            ammo_state(mag, reserve)
        }
        SlotWeapon::Knife => AmmoState::Plenty,
    };
    let before = last.replace((held, state));
    let worse = before.is_some_and(|(w, s)| w == held && state > s);
    if !worse {
        return;
    }
    let quote = match state {
        AmmoState::LastMag => Quote::LowAmmo,
        AmmoState::Out => Quote::OutOfAmmo,
        AmmoState::Plenty => return,
    };
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::RequestQuote { quote });
    }
}

/// The leader's dog-round / exfil quote delays changed (or they've just
/// joined a game): tell the server.
fn send_quote_delays(
    settings: Res<QuoteSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut sender: Query<&mut TriggerSender<shared::SetQuoteDelays>, With<GameClient>>,
    mut sent: Local<Option<(f32, f32)>>,
) {
    let me = local.iter().next().map(|l| l.0);
    let leading = crate::zombies_hud::zombies_game(&local, &lobbies).is_some_and(|l| Some(l.leader) == me);
    let want = (settings.dog_round_delay, settings.exfil_delay);
    if !leading {
        *sent = None;
        return;
    }
    // (Only if they're not the defaults the server starts with — or were.)
    let default = (DOG_ROUND_DELAY_SECS, EXFIL_DELAY_SECS);
    if *sent == Some(want) || (sent.is_none() && want == default) {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::SetQuoteDelays {
            dog_round: want.0,
            exfil: want.1,
        });
        *sent = Some(want);
    }
}

/// The "Quotes" debug window.
fn quotes_debug_ui(mut contexts: EguiContexts, mut settings: ResMut<QuoteSettings>) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("Quotes")
        .default_open(false)
        .default_pos([20.0, 640.0])
        .show(ctx, |ui| {
            let s = &mut *settings;
            ui.add(egui::Slider::new(&mut s.volume, 0.0f32..=5.0).text("volume (every quote, perk quotes too)"));
            ui.label("Party leader: how long after it starts someone says so");
            ui.add(egui::Slider::new(&mut s.dog_round_delay, 0.0f32..=20.0).text("dog round (s)"));
            ui.add(egui::Slider::new(&mut s.exfil_delay, 0.0f32..=20.0).text("exfil (s)"));
            if ui.button("Reset").clicked() {
                *s = QuoteSettings::default();
            }
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ammo_runs_down_to_the_last_mag_then_out() {
        assert_eq!(ammo_state(30, 90), AmmoState::Plenty);
        assert_eq!(ammo_state(30, 0), AmmoState::LastMag);
        assert_eq!(ammo_state(0, 0), AmmoState::Out);
        // (An empty mag with reserve left is a reload, not out.)
        assert_eq!(ammo_state(0, 30), AmmoState::Plenty);
        assert!(AmmoState::Out > AmmoState::LastMag && AmmoState::LastMag > AmmoState::Plenty);
    }
}
