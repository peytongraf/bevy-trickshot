//! Headless authoritative server for bevy-trickshot.
//!
//! * **Movement** is client-authoritative: we copy each client's reported pose
//!   onto its replicated [`shared::PlayerPose`] without correcting it.
//! * **Shots** are server-authoritative: every fire request in a client's input
//!   is ray-cast here (see [`sim`]) and the result is broadcast to everyone.
//! * **Thrown throwing knives** are simulated entirely here (see [`knives`]),
//!   bouncing off the map's collision mesh ([`collision`]) and replicated to
//!   the lobby.
//!
//! Configuration (all environment variables, all optional):
//!
//! | var                     | meaning                                | default              |
//! |-------------------------|----------------------------------------|----------------------|
//! | `PORT`                  | UDP listen port                        | `5000`               |
//! | `LIGHTYEAR_PRIVATE_KEY` | 32 comma-separated bytes, netcode key   | dev all-zero key     |
//! | `RUST_LOG`              | log filter                             | `info`               |

mod ai;
mod armor;
mod boss;
mod bots;
mod collision;
mod dogs;
mod exfil;
mod field_upgrades;
mod health;
mod killcam;
mod knives;
mod lethals;
mod lobby;
mod molotovs;
mod monkey_bombs;
mod mystery_box;
mod nav;
mod net;
mod power_ups;
mod pvp;
mod raygun;
mod revive;
mod sim;
mod wall_buys;
mod zombies;

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
        .add_plugins(pvp::PvpPlugin)
        .add_plugins(revive::RevivePlugin)
        .add_plugins(killcam::KillCamPlugin)
        .add_plugins(knives::KnivesPlugin)
        .add_plugins(molotovs::MolotovsPlugin)
        .add_plugins(ai::BotAiPlugin)
        .add_plugins(zombies::ZombiesPlugin)
        .add_plugins(dogs::DogsPlugin)
        .add_plugins(boss::BossPlugin)
        .add_plugins(exfil::ExfilPlugin)
        .add_plugins(power_ups::PowerUpsPlugin)
        .add_plugins(wall_buys::WallBuysPlugin)
        .add_plugins(mystery_box::MysteryBoxPlugin)
        .add_plugins(armor::ArmorPlugin)
        .add_plugins(field_upgrades::FieldUpgradesPlugin)
        .add_plugins(monkey_bombs::MonkeyBombsPlugin)
        .run();
}
