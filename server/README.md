# server

Headless authoritative server for bevy-trickshot. Bevy 0.16 (no renderer /
window / audio) + [lightyear](https://github.com/cBournhonesque/lightyear) 0.23.

## What it does

* Listens on **UDP** (`0.0.0.0:$PORT`, default 5000) using lightyear's netcode.
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

```sh
cd server

# one-time
fly apps create bevy-trickshot-server         # or another unique name → update `app` in fly.toml
fly ips allocate-v4                            # UDP requires a dedicated IPv4
fly secrets set LIGHTYEAR_PRIVATE_KEY="$(python3 -c 'import random; print(",".join(str(random.randint(0,255)) for _ in range(32)))')"

# every deploy (from server/, ".." = repo root as the build context)
fly deploy ..
```

`fly.toml` keeps one machine always running (`auto_stop_machines = "off"`,
`min_machines_running = 1`) so friends can always connect, and pairs the UDP
service with a same-port TCP service because fly.io's UDP routing needs it.

512 MB / shared-cpu-1x is plenty for a handful of players; bump `[[vm]]` in
`fly.toml` if you add bots or more modes.

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

`shared::map::NavProvider::find_path` is the seam. The server already runs a full
Bevy ECS world; spawn bot entities with `PlayerPose` + a `NavProvider`-driven
movement system and they replicate to clients like any other player.
