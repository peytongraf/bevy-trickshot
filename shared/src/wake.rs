//! Waking the prod server.
//!
//! The fly.io machine (`server/fly.toml`) is stopped whenever nobody's
//! playing — the server exits on its own once it's been empty a while
//! (`server::net::shut_down_when_idle`) — and started again by fly's proxy
//! the moment a TCP connection arrives on the game port. The game itself is
//! UDP, which that proxy doesn't see, so the client knocks over TCP first
//! ([`wake`], from `client::net`) and then connects as usual.
//!
//! The server answers that knock with [`WAKE_REPLY`]
//! (`server::health::spawn_tcp_listener`), so a reply proves the machine
//! is really up rather than just fly's proxy accepting the connection.
//!
//! `cargo run -p shared --example wake_probe -- <host:port>` runs the same
//! knock from a terminal; `server/test-wake.sh` uses it to check the whole
//! stop → wake round trip against the real server.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// What the server's TCP listener sends back — any reply containing it
/// means the server is up.
pub const WAKE_REPLY: &str = "bevy-trickshot server ok";

/// How long [`wake`] keeps knocking before giving up. A stopped fly
/// machine usually starts in a few seconds; this leaves room for a slow one.
pub const WAKE_TIMEOUT: Duration = Duration::from_secs(45);

/// One knock: connect, say something so the listener stops waiting for a
/// request, and read its reply. While the machine boots, fly's proxy holds
/// the connection open, so this can take several seconds to answer.
pub fn knock(addr: SocketAddr, timeout: Duration) -> std::io::Result<()> {
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(b"GET / HTTP/1.0\r\n\r\n")?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply)?;
    if String::from_utf8_lossy(&reply).contains(WAKE_REPLY) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "no answer from the server (the proxy closed the connection)",
        ))
    }
}

/// Knock until the server answers or `timeout` runs out, a second apart.
/// Returns how long it took to answer, or the last knock's error.
pub fn wake(addr: SocketAddr, timeout: Duration) -> Result<Duration, String> {
    let start = Instant::now();
    loop {
        let left = timeout.saturating_sub(start.elapsed());
        let err = match knock(addr, left.min(Duration::from_secs(15)).max(Duration::from_secs(1))) {
            Ok(()) => return Ok(start.elapsed()),
            Err(e) => e,
        };
        if start.elapsed() + Duration::from_secs(1) >= timeout {
            return Err(err.to_string());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A stand-in for one connection to the game port: `reply` is what it
    /// sends back after reading the request.
    fn serve_once(reply: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            let _ = s.read(&mut buf);
            let _ = s.write_all(reply);
        });
        addr
    }

    #[test]
    fn knock_accepts_the_server_reply() {
        let addr = serve_once(b"HTTP/1.1 200 OK\r\n\r\nbevy-trickshot server ok\n");
        assert!(knock(addr, Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn knock_rejects_a_bare_close() {
        // What fly's proxy does when it can't reach a machine.
        let addr = serve_once(b"");
        assert!(knock(addr, Duration::from_secs(2)).is_err());
    }

    #[test]
    fn wake_gives_up_when_nothing_listens() {
        let addr = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert!(wake(addr, Duration::from_millis(500)).is_err());
    }
}
