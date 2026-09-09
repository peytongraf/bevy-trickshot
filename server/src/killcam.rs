//! Kill-cam recording. Every player carries a per-tick ring buffer of
//! [`shared::KillCamSample`]s (eye pose + one-shot sound bits). On a bot kill we
//! note the killer + tick, wait for the follow-through to be buffered, then
//! ship the `[kill − 3 s, kill + 1.5 s]` window to the lobby.

use std::collections::VecDeque;

use bevy::prelude::*;
use lightyear::prelude::input::native::ActionState;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use shared::{Bot, GameChannel, KillCam, KillCamBot, KillCamSample, Lobby, PlayerId, PlayerInput};

use crate::bots::{BotHit, LobbyBot};

/// Samples before / after the kill (at `TICK_HZ`).
const PRE: u64 = (shared::TICK_HZ as u64) * 3;
const POST: u64 = (shared::TICK_HZ * 1.5) as u64;
/// Ring-buffer capacity (a touch over `PRE + POST`).
const CAP: usize = (PRE + POST + 32) as usize;

/// One player's rolling recording.
#[derive(Component, Default)]
pub struct ReplayBuffer {
    frames: VecDeque<(u64, KillCamSample)>,
}

/// Monotonic tick counter for buffer bookkeeping (avoids `Tick` wrap math).
#[derive(Resource, Default)]
struct ReplayClock(u64);

struct PendingCam {
    killer: PeerId,
    killer_name: String,
    kill_seq: u64,
    bots: Vec<KillCamBot>,
}

#[derive(Resource, Default)]
struct PendingCams(Vec<PendingCam>);

pub struct KillCamPlugin;

impl Plugin for KillCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ReplayClock>()
            .init_resource::<PendingCams>()
            .add_systems(
                FixedUpdate,
                (
                    advance_clock,
                    attach_buffers,
                    record_frames,
                    queue_killcams,
                    flush_killcams,
                )
                    .chain(),
            );
    }
}

fn advance_clock(mut clock: ResMut<ReplayClock>) {
    clock.0 = clock.0.wrapping_add(1);
}

fn attach_buffers(
    missing: Query<Entity, (With<PlayerId>, Without<ReplayBuffer>)>,
    mut commands: Commands,
) {
    for e in &missing {
        commands.entity(e).insert(ReplayBuffer::default());
    }
}

fn record_frames(
    clock: Res<ReplayClock>,
    mut players: Query<(&ActionState<PlayerInput>, &mut ReplayBuffer)>,
) {
    for (action, mut buf) in &mut players {
        let i = &action.0;
        buf.frames.push_back((
            clock.0,
            KillCamSample {
                translation: i.translation,
                yaw: i.yaw,
                pitch: i.pitch,
                shake_trauma: i.shake_trauma,
                shake_phase: i.shake_phase,
                shake_recoil: i.shake_recoil,
                sway_offset: i.sway_offset,
                fov_deg: i.fov_deg,
                sound_bits: i.sound_bits,
                anim_time: i.anim_time,
                ads_t: i.ads_t,
                ground_pt: i.ground_pt,
            },
        ));
        while buf.frames.len() > CAP {
            buf.frames.pop_front();
        }
    }
}

fn queue_killcams(
    clock: Res<ReplayClock>,
    mut hits: EventReader<BotHit>,
    lobbies: Query<(Entity, &Lobby)>,
    bots: Query<(Entity, &Bot, &LobbyBot)>,
    mut pending: ResMut<PendingCams>,
) {
    for ev in hits.read() {
        let Some((lobby_e, lobby)) = lobbies.iter().find(|(_, l)| l.has(ev.by)) else {
            continue;
        };
        let name = lobby
            .members
            .iter()
            .find(|m| m.peer == ev.by)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Someone".to_string());
        // Freeze every bot in the killer's game as it stands now.
        let snap: Vec<KillCamBot> = bots
            .iter()
            .filter(|(_, _, lb)| lb.lobby == lobby_e)
            .map(|(e, b, _)| KillCamBot {
                pos: b.pos.to_array(),
                yaw: b.yaw,
                killed: e == ev.bot,
            })
            .collect();
        pending.0.push(PendingCam {
            killer: ev.by,
            killer_name: name,
            kill_seq: clock.0,
            bots: snap,
        });
    }
}

fn flush_killcams(
    clock: Res<ReplayClock>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut pending: ResMut<PendingCams>,
    buffers: Query<(&PlayerId, &ReplayBuffer)>,
    lobbies: Query<&Lobby>,
) {
    let server = server.into_inner();
    let now = clock.0;
    pending.0.retain(|cam| {
        if now.wrapping_sub(cam.kill_seq) < POST {
            return true; // not enough follow-through buffered yet
        }

        let Some((_, buf)) = buffers.iter().find(|(pid, _)| pid.0 == cam.killer) else {
            return false; // killer gone
        };
        let lo = cam.kill_seq.wrapping_sub(PRE);
        let hi = cam.kill_seq.wrapping_add(POST);
        let window: Vec<KillCamSample> = buf
            .frames
            .iter()
            .filter(|(seq, _)| in_range(*seq, lo, hi))
            .map(|(_, s)| *s)
            .collect();
        let kill_index = buf
            .frames
            .iter()
            .filter(|(seq, _)| in_range(*seq, lo, hi) && in_range(*seq, lo, cam.kill_seq))
            .count()
            .saturating_sub(1) as u32;

        if window.len() < 8 {
            return false; // nothing worth showing
        }

        let targets: Vec<PeerId> = lobbies
            .iter()
            .find(|l| l.has(cam.killer))
            .map(|l| l.members.iter().map(|m| m.peer).collect())
            .unwrap_or_default();

        let msg = KillCam {
            killer_name: cam.killer_name.clone(),
            samples: window,
            kill_index,
            bots: cam.bots.clone(),
        };
        if let Err(e) = sender.send::<_, GameChannel>(&msg, server, &NetworkTarget::Only(targets)) {
            error!("failed to send kill cam: {e:?}");
        }
        false
    });
}

/// `seq` in `[lo, hi]`, tolerant of the `u64` counter wrapping.
fn in_range(seq: u64, lo: u64, hi: u64) -> bool {
    seq.wrapping_sub(lo) <= hi.wrapping_sub(lo)
}
