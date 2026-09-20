# bevy-trickshot

A multiplayer 3D trickshot game: Call-of-Duty-style trick shots across several
game modes. Rust end to end.

```
bevy-trickshot/
├── client/     the game — Bevy 0.16 (was the old repo root, unchanged in behaviour)
├── server/     headless authoritative server — Bevy 0.16 + lightyear 0.23, deploys to fly.io
├── shared/     protocol + shot-resolution math compiled into BOTH client and server
└── Cargo.toml  workspace
```

## Download the latest release (just want to play)

No Rust toolchain needed for this — it's a prebuilt executable.

1. Go to the [Releases page](https://github.com/peytongraf/bevy-trickshot/releases/latest)
   and download the zip for your OS: `bevy-trickshot-linux.zip` or
   `bevy-trickshot-windows.zip`.
2. Unzip it anywhere.
3. Run it:
   * **Linux:** `chmod +x bevy-trickshot && ./bevy-trickshot` (or double-click
     it in a file manager that allows running executables).
   * **Windows:** double-click `bevy-trickshot.exe`. A console window stays
     open alongside the game — that's normal, it shows update progress.
4. On first launch it downloads `client/assets/` (~92 MB of art) next to the
   executable automatically, then connects to the production server
   (`bevy-trickshot-server.fly.dev`) and drops you at the main menu.

After that, just relaunch the same executable to play — it checks for a newer
release on startup and self-updates if one exists (a code update is a small
download; the ~92 MB art pack is only re-fetched when it actually changed).

## Why this shape

The server runs the **same Bevy version as the client** so that `shared/` can
hold the single, authoritative definition of

* the network protocol (replicated components, the per-tick input packet,
  gameplay messages), and
* "did this shot hit" — the ballistics and hitbox math.

There is no second implementation to keep in sync.

## Authority model

| System        | Authority        | How                                                              |
|---------------|------------------|-----------------------------------------------------------------|
| Player movement | **Client**     | The client fills its pose into `PlayerInput` each tick; the server copies it onto the replicated `PlayerPose` untouched. |
| Thrown knives   | **Server**     | A client's `ThrowKnife` request starts the flight; the server steps it (`shared::throwing_knife`) against the map's collision mesh — the same `.glb` the client's colliders come from, embedded in the server build (`server/src/collision.rs`) — and replicates it as a `ThrownKnife` the clients just draw. |
| Shot detection  | **Server**     | `PlayerInput` also carries a *fire request*; the server ray-casts it (`shared::ballistics::resolve_shot`) against everyone else's hitboxes and broadcasts a `ShotResolved` message. |

This keeps movement feeling perfectly responsive while making hits
authoritative — the foundation for a future head-to-head mode. See
`server/README.md` for the lag-compensation upgrade path that mode will want.

## Running locally

Rust 1.93+ is enough for the workspace as pinned. On Linux the client also
needs `libasound2-dev` and `libudev-dev` (Bevy's audio/input backends):

```sh
sudo apt-get install libasound2-dev libudev-dev   # Debian/Ubuntu; skip on Windows/macOS
```

### 1. Run a dev server

```sh
cargo run --bin server
```

Headless, no window. Listens on `udp://[::]:5000` (dual-stack, so both
`127.0.0.1:5000` and `[::1]:5000` reach it). Uses the all-zero dev netcode key
automatically — see `server/README.md` if you want to set a real
`LIGHTYEAR_PRIVATE_KEY` locally too.

### 2. Run the client

```sh
cargo run --bin bevy-trickshot          # or: cd client && cargo run
```

`cargo run` always points at your local dev server (`127.0.0.1:5000`) and
never runs the self-updater — both of those only kick in for a shipped
binary. To try two players on one machine, just run the command twice in
separate terminals; each instance picks a distinct client ID automatically.

To instead point a local client at the **production** server on occasion
(e.g. to check prod is reachable, or test something live without cutting a
release), set `TRICKSHOT_PROD`:

```sh
TRICKSHOT_PROD=1 cargo run
```

For anything else — a friend's self-hosted server, a non-default port,
`localhost` explicitly — set `TRICKSHOT_SERVER` to the host:port instead
(only the exact string `bevy-trickshot-server.fly.dev:5000` gets the prod
key; anything else falls back to the dev key):

```sh
TRICKSHOT_SERVER=bevy-trickshot-server.fly.dev:5000 cargo run
```

`TRICKSHOT_SERVER` wins if both are set. Neither variable does anything in a
shipped build — that always defaults to production. See
`client::net::server_addr`.

### 3. Run the tests

```sh
cargo test -p shared   # shared simulation/ballistics math
```

Deploying your own server to fly.io: see [`server/README.md`](server/README.md).

## Distributing the client to friends (Linux + Windows)

The client self-updates from GitHub Releases. You cut a release by pushing a tag;
a friend runs the game and it pulls the update on launch.

### One-time setup

1. Push this repo to GitHub as a **public** repo.
2. Check `DEFAULT_REPO` in [`client/src/updater.rs`](client/src/updater.rs) is
   your `owner/repo` (a friend can also override it with `TRICKSHOT_REPO=...`).
3. Nothing else — `.github/workflows/release.yml` runs on tag push.

### Ship an update

```sh
git tag v0.4.1
git push --tags
```

GitHub Actions builds the client for Linux and Windows and publishes a Release
`v0.4.1` with:

| asset                         | contents                                  |
|-------------------------------|-------------------------------------------|
| `bevy-trickshot-linux.zip`    | the Linux executable                      |
| `bevy-trickshot-windows.zip`  | the Windows executable                    |
| `assets.zip`                  | the whole `client/assets/` folder (~92 MB) |
| `manifest.json`               | `{ version, assets_hash }` the updater reads |

You can also run the workflow by hand from the Actions tab (give it a version).

### What a friend does

See [Download the latest release](#download-the-latest-release-just-want-to-play)
above for first-time setup. After that, just relaunch the game — if there's a
newer release it prints `updated — restarting…`, swaps itself and relaunches.
A normal code change is a ~5 MB download; the 92 MB of art is only re-fetched
when a file in it actually changed (`manifest.json`'s `assets_hash` is a
content hash, not a build hash).

Update progress prints to the console. On Windows the client keeps a console
window for this; swap in a proper updater window later if you want (the seam is
`updater.rs`). To run the local build without the update check:
`TRICKSHOT_SKIP_UPDATE=1`. `cargo run` never checks.

> The very first release must exist before the updater has anything to read —
> until then it just logs "skipped" and starts the current build.

## Roadmap hooks already in place

* **Maps / occlusion** — `shared::map::CollisionWorld`. The shot resolver takes a
  `|from, to| -> bool` "is this blocked" closure; it's currently `|_, _| false`.
  Implement `CollisionWorld` over your map geometry and pass it through.
* **AI bots** — `shared::map::NavProvider::find_path`. Back it with a navmesh
  (e.g. `oxidized_navigation`) once the map exists; the server already has the
  ECS world to drive bot entities.

## Version pinning

`lightyear` and Bevy move fast and in lockstep. If you bump one, bump the other
to a compatible pair and re-check `shared/`:

| lightyear | Bevy | min Rust |
|-----------|------|----------|
| 0.23 (current) | 0.16 | 1.88 |
| 0.29 | 0.19 | 1.95 |
