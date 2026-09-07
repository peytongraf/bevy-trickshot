//! A minimal TCP listener on the game port.
//!
//! fly.io only forwards UDP to an app that also has a same-port TCP service, and
//! its proxy warns when nothing is actually listening on TCP. This satisfies both
//! and gives a browser-checkable health URL
//! (`http://<app>.fly.dev:<port>/` → `200 ok`). It never touches the game loop.

use std::io::{Read, Write};
use std::net::{Ipv6Addr, TcpListener, TcpStream};

pub fn spawn_tcp_listener(port: u16) {
    std::thread::Builder::new()
        .name("tcp-health".into())
        .spawn(move || {
            // `[::]` is dual-stack on Linux, so this also covers 0.0.0.0 — which
            // is the address fly's proxy check looks for.
            let listener = match TcpListener::bind((Ipv6Addr::UNSPECIFIED, port)) {
                Ok(l) => l,
                Err(e) => {
                    bevy::log::warn!("tcp health listener could not bind :{port}: {e}");
                    return;
                }
            };
            bevy::log::info!("tcp health listener on [::]:{port}");
            for stream in listener.incoming() {
                if let Ok(stream) = stream {
                    let _ = handle(stream);
                }
            }
        })
        .expect("spawn tcp-health thread");
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    // Drain the request (best effort) then reply and close.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
    let mut scratch = [0u8; 1024];
    let _ = stream.read(&mut scratch);
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nbevy-trickshot server ok\n",
    )
}
