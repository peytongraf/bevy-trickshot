# server

Headless authoritative server for bevy-trickshot. Bevy 0.16 (no renderer /
window / audio) + [lightyear](https://github.com/cBournhonesque/lightyear) 0.23.

## What it does

* Listens on **UDP** (port `$PORT`, default 5000) using lightyear's netcode —
  the wildcard address locally, `fly-global-services` in production (see
  `src/net.rs`'s `bind_addr`).
* Spawns a replicated player entity per connected client
  (`src/net.rs`).
* Each tick (`src/sim.rs`):
  * **`apply_client_pose`** — copies the pose from each client's `PlayerInput`
    onto its replicated `PlayerPose`. Movement is the client's call; the server
    does not correct it.
  * **`resolve_shots`** — for every client that set `fire` this tick, runs
    `shared::ballistics::resolve_shot` against every *other* player's capsule
    hitboxes and broadcasts a `ShotResolved` message on the reliable
    `GameChannel`.

Everything about *what a shot hits* lives in `shared/`, so the client can run the
exact same code to predict.

## Run locally

```sh
cargo run --bin server
# PORT=6000 RUST_LOG=debug cargo run --bin server
```

## Configuration

| Env var                 | Purpose                                    | Default          |
|-------------------------|--------------------------------------------|------------------|
| `PORT`                  | UDP listen port                            | `5000`           |
| `LIGHTYEAR_PRIVATE_KEY` | Netcode private key, 32 comma-separated bytes | dev all-zero key |
| `RUST_LOG`              | Log filter                                 | `info`           |

The client must be built with the **same** `shared::PROTOCOL_ID` and, in
production, the same private key.

## Deploy to fly.io

The Docker build context is the repo root (the server compiles `shared/`); the
image is built from the root `Dockerfile` and `server/fly.toml` is the app config.
**Run `fly` from the repo root** — it resolves `--config` and the Dockerfile
relative to the path argument.

```sh
# one-time
fly apps create bevy-trickshot-server         # or another unique name → update `app` in fly.toml
fly ips allocate-v4                            # dedicated IPv4, ~$2/mo — UDP requires this, see server/fly.toml
fly secrets set LIGHTYEAR_PRIVATE_KEY="$(python3 -c 'import random; print(",".join(str(random.randint(0,255)) for _ in range(32)))')" -a bevy-trickshot-server

# every deploy
fly deploy . --config server/fly.toml -a bevy-trickshot-server
```

After the first deploy, pin it to a single machine (this is one game shard, not a
scale-out service):

```sh
fly scale count 1 -a bevy-trickshot-server --yes
```

### Reachability

UDP on fly.io only ever works over a **dedicated IPv4** — see
<https://fly.io/docs/networking/udp-and-tcp/>: "You need a dedicated IPv4
address. You can't use a shared IPv4 address or an IPv6 address for UDP."
(An earlier version of this setup used a free dedicated IPv6 instead, which
looked fine — `fly ips list` showed it, inbound packets even arrived — but no
client could ever actually connect, since public UDP over IPv6 isn't
supported at all. Dedicated IPv4 is the one that has to exist:
`fly ips allocate-v4 -a bevy-trickshot-server`, ~$2/mo.)

The other requirement is *what* the server binds to. A wildcard bind
(`0.0.0.0` or `[::]`) makes Linux pick the wrong outbound source address for
replies, which fly's NAT then can't rewrite back to the public IP — so
replies silently vanish even though inbound packets get through fine. The fix
is binding to the special `fly-global-services` address, which is what
`bind_addr()` in `src/net.rs` does whenever `FLY_APP_NAME` is set (i.e. only
in production — a local `cargo run --bin server` still binds the `[::]`
wildcard, which is fine off of fly.io).

### Health check

The server also opens plain TCP on the same port (fly's UDP routing wants a
live TCP listener there):

```sh
curl http://bevy-trickshot-server.fly.dev:5000/     # -> "bevy-trickshot server ok"
```

### Sizing

`fly.toml` keeps one machine always running (`auto_stop_machines = "off"`,
`min_machines_running = 1`). 512 MB / shared-cpu-1x is plenty for a handful of
players; bump `[[vm]]` in `fly.toml` if you add bots or more modes.

## Upgrade paths

### Lag compensation (needed for fair 1v1)

`resolve_shots` currently tests shots against **current-tick** hitboxes. For a
head-to-head mode you want to rewind other players to where the shooter saw them
(their interpolation delay ago). lightyear ships this:

* add `lightyear_avian3d` with the `lag_compensation` feature,
* give players an avian `Collider` + `LagCompensationHistory`,
* raycast with `LagCompensationSpatialQuery` instead of the hand-rolled
  `shared::ballistics` path (keep `shared::ballistics` for client prediction).

See the `fps` example in the lightyear repo.

### Map occlusion

`resolve_shot` takes a `|from, to| -> bool` closure that currently always returns
`false`. Implement `shared::map::CollisionWorld` over your map geometry, store it
as a resource, and pass `|a, b| world.segment_blocked(a, b)`.

### Bots

Done — stationary target bots live in `src/bots.rs` (`BotsPlugin`):
`ensure_bots` tops up each lobby's bot count near the map centre, a hit sends a
`BotHit` (resolved in `src/sim.rs`) that tips the bot over, scores the shooter,
and despawns it a couple seconds later. Moving/AI-driven bots (as opposed to
stationary targets) would still need `shared::map::NavProvider::find_path` as
the movement seam.
