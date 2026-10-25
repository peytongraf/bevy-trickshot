//! Knock on a server's game port the way the client does on launch
//! (`shared::wake`) and say how long it took to answer — used by
//! `server/test-wake.sh` to check a stopped fly machine really wakes.
//!
//!     cargo run -p shared --example wake_probe -- bevy-trickshot-server.fly.dev:5000

use std::net::ToSocketAddrs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let target = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("bevy-trickshot-server.fly.dev:{}", shared::DEFAULT_PORT));
    // IPv4 first, like `client::net::server_addr`: fly only gives the
    // dedicated IPv4 the game's UDP, and that's the address the client knocks on.
    let mut addrs: Vec<_> = match target.to_socket_addrs() {
        Ok(a) => a.collect(),
        Err(e) => {
            eprintln!("couldn't resolve {target}: {e}");
            return ExitCode::FAILURE;
        }
    };
    addrs.sort_by_key(|a| a.is_ipv6());
    let Some(addr) = addrs.first().copied() else {
        eprintln!("{target} resolved to no addresses");
        return ExitCode::FAILURE;
    };

    println!("knocking on {addr} (tcp) …");
    match shared::wake::wake(addr, shared::wake::WAKE_TIMEOUT) {
        Ok(took) => {
            println!("server answered after {:.1}s", took.as_secs_f32());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("no answer within {}s: {e}", shared::wake::WAKE_TIMEOUT.as_secs());
            ExitCode::FAILURE
        }
    }
}
