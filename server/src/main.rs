//! Headless authoritative server for bevy-trickshot.
//!
//! * **Movement** is client-authoritative: we copy each client's reported pose
//!   onto its replicated [`shared::PlayerPose`] without correcting it.
//! * **Shots** are server-authoritative: every fire request in a client's input
//!   is ray-cast here (see [`sim`]) and the result is broadcast to everyone.
//!
//! Configuration (all environment variables, all optional):
//!
//! | var                     | meaning                                | default              |
//! |-------------------------|----------------------------------------|----------------------|
//! | `PORT`                  | UDP listen port                        | `5000`               |
//! | `LIGHTYEAR_PRIVATE_KEY` | 32 comma-separated bytes, netcode key   | dev all-zero key     |
//! | `RUST_LOG`              | log filter                             | `info`               |

mod bots;
mod health;
mod lobby;
mod net;
mod sim;

use bevy::app::ScheduleRunnerPlugin;
use bevy::diagnostic::DiagnosticsPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use core::time::Duration;
use lightyear::prelude::server::ServerPlugins;

fn main() {
    let tick = Duration::from_secs_f64(1.0 / shared::TICK_HZ);

    // fly.io routes UDP only alongside a same-port TCP service; this answers it
    // (and doubles as a plain-HTTP health check). Harmless anywhere else.
    health::spawn_tcp_listener(net::listen_port());

    App::new()
        // Headless: no window, no renderer. Throttle the outer loop to the tick
        // rate so an idle server isn't pinning a core.
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(tick)),
            LogPlugin::default(),
            StatesPlugin,
            DiagnosticsPlugin,
        ))
        .add_plugins(ServerPlugins { tick_duration: tick })
        .add_plugins(shared::SharedPlugin)
        .add_plugins(net::ServerNetPlugin)
        .add_plugins(lobby::LobbyPlugin)
        .add_plugins(sim::SimPlugin)
        .add_plugins(bots::BotsPlugin)
        .run();
}
