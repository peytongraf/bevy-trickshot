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
| Shot detection  | **Server**     | `PlayerInput` also carries a *fire request*; the server ray-casts it (`shared::ballistics::resolve_shot`) against everyone else's hitboxes and broadcasts a `ShotResolved` message. |

This keeps movement feeling perfectly responsive while making hits
authoritative — the foundation for a future head-to-head mode. See
`server/README.md` for the lag-compensation upgrade path that mode will want.

## Build & run

```sh
# server (headless, listens on udp://0.0.0.0:5000)
cargo run --bin server

# client
cargo run --bin bevy-trickshot          # or: cd client && cargo run

# tests for the shared simulation math
cargo test -p shared
```

Rust 1.93+ is enough for the workspace as pinned. Deploying the server: see
[`server/README.md`](server/README.md).

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

* **First time:** download `bevy-trickshot-<os>.zip` from the repo's Releases
  page, unzip it anywhere, run it. On first launch it downloads `assets/` next to
  the executable automatically.
* **After that:** just launch the game. If there's a newer release it prints
  `updated — restarting…`, swaps itself and relaunches. A normal code change is a
  ~5 MB download; the 92 MB of art is only re-fetched when a file in it actually
  changed (`manifest.json`'s `assets_hash` is a content hash, not a build hash).

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
* **Client networking** — the client depends on `shared` and `lightyear` but does
  not add `ClientPlugins` yet. Wiring it up is the next client task.

## Version pinning

`lightyear` and Bevy move fast and in lockstep. If you bump one, bump the other
to a compatible pair and re-check `shared/`:

| lightyear | Bevy | min Rust |
|-----------|------|----------|
| 0.23 (current) | 0.16 | 1.88 |
| 0.29 | 0.19 | 1.95 |
