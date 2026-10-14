//! The regular knife's stab and swing sounds. The server resolves every knife
//! attack (`server::sim::resolve_shots`) and tells the whole lobby the
//! outcome as a [`shared::KnifeAttackSound`]: a stab that landed on a bot /
//! player plays one of `audio/weapons/knife/stab/` from the victim, anything else
//! plays one of `audio/weapons/knife/swing/` from the attacker — the attacker
//! included, since only the server knows which it was. The server picks the
//! clip, so everyone hears the same one. (The equip sound is local-only and
//! lives in `weapon_system`.)

use bevy::prelude::*;

use crate::audio_engine::prelude::*;
use lightyear::prelude::*;

use crate::killcam::ActiveKillCam;
use crate::{positional_playback, AppState, KnifeSounds};

pub struct KnifeSoundsPlugin;

impl Plugin for KnifeSoundsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            receive_knife_attacks.run_if(in_state(AppState::InGame)),
        );
    }
}

/// Play each announced knife attack's sound from where the server put it
/// (faded by distance by its profile).
fn receive_knife_attacks(
    mut receivers: Query<&mut MessageReceiver<shared::KnifeAttackSound>>,
    killcam: Res<ActiveKillCam>,
    knife_sounds: Res<KnifeSounds>,
    mut commands: Commands,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            if killcam.0.is_some() {
                continue; // a replay is playing; this is the live world's attack
            }
            let set = if msg.stab {
                &knife_sounds.stab
            } else {
                &knife_sounds.swing
            };
            let Some(clip) = set.pick(msg.variant) else {
                continue;
            };
            commands.spawn((
                AudioPlayer::new(clip.handle.clone()),
                Transform::from_translation(Vec3::from_array(msg.point)),
                positional_playback(Volume::Linear(1.0)),
            ));
        }
    }
}
