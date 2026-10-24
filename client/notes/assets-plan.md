# Assets, R2 and collision / visual map models — plan

Where the game's art lives, how it reaches players, and how maps get built
in Blender. Nothing here is implemented yet except where marked **done**.

## Where things stand

- `client/assets/` is ~1.6 GB, all of it in git (826 files). GitHub refuses
  any single file over 100 MB on push — `der_tempel_map.glb` is 96 MB.
  Every saved version of a big model also stays in git history forever.
- Each release zips the whole folder into one `assets.zip` (1.3 GB at
  v0.4.7) and attaches it to the GitHub Release. **GitHub caps a release
  attachment at 2 GiB**, so releases start failing well before the git
  limit is the real problem.
- The assets hash covers every file, so changing one texture makes every
  player re-download the full 1.3 GB.
- The client's update check now runs inside the game, in a screen that
  blocks the main menu (**done**, see "Updating" below). It still downloads
  the one big zip.

## Storage: Cloudflare R2

Use **Cloudflare R2**:

- 10 GB storage free; past that about $0.015 / GB / month (20 GB ≈ $0.15).
- **Downloads are never charged** (no egress fees) — the part that gets
  expensive anywhere else once players download a lot.
- 10 M reads / 1 M writes a month free — far more than needed.
- S3-compatible: upload with `rclone` or `aws s3` from a script or CI.
- A public bucket gives plain HTTPS URLs (`*.r2.dev`, or a custom domain).
- May ask for a payment method even while you stay at $0.

Alternatives, for the record: Backblaze B2 (10 GB free, downloads free only
up to a cap), itch.io + `butler` (free, does patching, but updates go
through itch rather than our updater). Avoid Git LFS — GitHub's free quota
is ~1 GB storage + bandwidth, and CI checkouts use it up.

### Layout in the bucket

```
objects/<sha256>              every asset file, named by its content hash
manifests/<version>.json      the file list for a release
manifests/latest.json         copy of the newest release's manifest
binaries/<version>/...        (optional — the executables can stay on
                               GitHub Releases, they're ~40 MB)
```

Files are stored by hash, so:

- each version of a file is uploaded once, ever;
- nothing is ever overwritten, so an older client mid-update still finds
  the files its manifest names;
- a release only uploads what actually changed.

### The manifest

```json
{
  "version": "0.4.9",
  "files": [
    { "path": "models/maps/der_tempel_visual.glb", "sha256": "…", "size": 91234567 },
    { "path": "fonts/BebasNeue-Regular.ttf",       "sha256": "…", "size": 61000 }
  ]
}
```

### What the client does with it (replaces `assets.zip`)

1. Fetch the manifest for the version it's about to run.
2. Compare against what's on disk. Cache each local file's hash keyed by
   size + modified time so a launch doesn't re-hash 1.6 GB.
3. Download only missing / changed files into a staging folder, checking
   each hash.
4. Move them into `assets/`, then delete any file not in the manifest
   (renamed / removed assets stop lingering).
5. Interrupted? The next launch just carries on — finished files are
   already in place and verified.

Progress goes to the same update screen (bytes done / total).

### Getting assets out of git

1. Keep **small** files in git (fonts, `.ron`, `mix.ron`, small textures,
   UI icons) — the update screen embeds `fonts/BebasNeue-Regular.ttf` with
   `include_bytes!`, so that one must stay in git.
2. Move the big binaries (map models, skyboxes, weapon / prop `.glb`s,
   long music) to R2. Git tracks a small `assets.lock` — the manifest.
3. Two scripts (an `xtask` or shell):
   - `assets push` — hash every big file, upload the new ones to
     `objects/`, rewrite `assets.lock`. Run it before committing art
     changes.
   - `assets pull` — download whatever `assets.lock` names that's missing
     locally. Run after cloning / pulling.
4. CI's release job publishes `assets.lock` as `manifests/<version>.json`
   and `latest.json` — no zipping, no 2 GiB limit.
5. Optionally rewrite history (`git filter-repo`) to drop the old big
   blobs and shrink the repo. Not required.

### Cheap size wins (do any time)

- Skyboxes: the 8K `.hdr`s are 57–78 MB each. 4K, or a KTX2 cubemap
  (BC6H), is a fraction of that and loads faster.
- `audio/music/main_menu.wav` (53 MB) → `.ogg` (~5 MB).
- 4K weapon / prop textures (`frag_4k.glb` 42 MB, `ak_74.glb` 54 MB) → 2K;
  nearly identical at view-model size.
- Bevy can load KTX2 textures (enable the `ktx2` + `zstd` features);
  `gltf-transform` can convert a `.glb`'s textures. Optional.

## Maps: collision model vs visual model

The standard pipeline: **greybox first** (plain shapes, the layout), then an
**art pass** on top. The greybox becomes the collision model; the art
becomes the visual model. The game already supports this split — Shipment
draws `shipment_visual.glb` over `shipment.glb`'s collision
(`client/src/environment/map.rs`, `map_visual_path`). Der Tempel just needs
the same entry once its visual model exists.

### One .blend, two exports

- `der_tempel.blend` with two collections: **`Collision`** and **`Visual`**.
  Same world space, same origin — they line up by construction.
- An export script writes each collection to its own file:
  - `models/maps/der_tempel_map.glb` — collision (keep the current name so
    the server's `include_bytes!` and `shared::map` stay as they are)
  - `models/maps/der_tempel_visual.glb` — what players see
- Run it headless so it's one step and neither file can go stale:
  `blender -b der_tempel.blend --python tools/export_map.py`
- Never move the origin or apply a transform to one collection and not the
  other. Everything in `shared/levels/der_tempel.ron` (doors, perks,
  windows, spawns) is world-space and stays valid.

### Props (e.g. a downloaded tank)

- Each prop is its own collection holding the detailed mesh **plus**
  simple collision shapes named with a `_col` suffix:
  `tank`, `tank_col_body`, `tank_col_turret`. (Same idea as Unreal's
  `UCX_` prefix / Godot's `-col` suffix.)
- Place props in the level as **collection instances**. The export script
  expands instances, sends every `*_col` object to the collision file and
  everything else to the visual file. Move / rotate the tank and its
  collision goes with it — that's what keeps the two in sync.
- Collision shapes: boxes where possible, otherwise a convex hull of a
  decimated copy (Blender's *Convex Hull*). A handful of shapes per prop.
- Small clutter (cans, signs, cables, papers) gets **no** collision.

### Other standard tricks

- Stairs: a smooth ramp in the collision model — movement and zombie
  pathing don't stutter over each step.
- Invisible collision-only blockers for ledges players shouldn't reach;
  visual-only detail for grass, distant set dressing, ceiling clutter.
- Anything gameplay reads from the map's surfaces (water areas, footstep
  surface types, wallbang thickness) lives on the **collision** model —
  it's the one the server has. Use object / material names.
- Keep the collision model closed and simple; no textures on it.

### What this buys

- The server embeds only the collision `.glb` — untextured, it's tiny, so
  the server build shrinks and the file can stay in git.
- Only the visual model grows, and it goes through R2.
- A debug toggle that draws the collision model as a wireframe over the
  visual makes any misalignment obvious (worth adding with the split).

### When

Now, while several Der Tempel zones are still empty: finish the layout in
greybox, move the greybox into `Collision`, duplicate it into `Visual` and
start the art pass there.

## Updating (done)

- The window opens straight into an update screen (`AppState::Updating`,
  `client/src/update_screen.rs`). The check runs on a background thread
  (`client/src/updater.rs`) with timeouts; the main menu only appears once
  it says the game is up to date.
- An update downloads with a progress bar, then shows **UPDATE REQUIRES
  RESTART** with a RESTART button. A failed check blocks with RETRY / QUIT
  — a shipped build never falls through to an old version.
- The server checks every client's version on connect
  (`shared::ClientHello` / `shared::VersionMismatch`, `server/src/net.rs`)
  and drops a mismatched one, so an old client can never play even if its
  own check was skipped.
- Debug panel → "Update screens" previews every screen without touching
  versions or files.
- **Release order matters now**: tag the release (`git tag vX.Y.Z && git
  push --tags`) and deploy the server (`fly deploy`) together. The server
  only accepts clients on exactly its version, so deploying the server
  ahead of the release locks players out until the release is up.

## Order

1. R2 bucket + `assets push` / `pull` + `assets.lock`; big files out of git.
2. Per-file manifest in the updater (replace `assets.zip`).
3. CI publishes the manifest instead of zipping.
4. Der Tempel `Collision` / `Visual` split + export script + wireframe
   debug toggle.
5. Size wins (skyboxes, music, 4K textures) whenever convenient.
