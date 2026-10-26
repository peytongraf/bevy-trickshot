# Server performance: CPU and memory

Ideas for making the game server (`server/`) cheaper to run and quicker to
start. Ordered easiest → hardest within each section, like `todo.md`.

## Where things stand (Oct 25, 2026)

- **Machine:** fly.io `performance-1x` (1 dedicated core, 2 GB), stopped
  while idle (`server/fly.toml`, `IDLE_SHUTDOWN_MINS`). Before this it was
  `shared-cpu-1x`, which only guarantees 1/16 of a core. On round 13 of Der
  Tempel the tick rate fell from 64 Hz to 25–48 Hz and every zombie and
  remote player froze on the clients.
- **Memory:** about 112 MB in use on average (fly metrics). That's well
  within 2 GB, so memory work is about startup time and image size, not
  about running out.
- **Startup:** about 6 s from boot to the game socket listening, every time
  the server wakes (fly logs, Oct 25):
  - ~4.0 s building the bot navigation graphs (`server::ai`'s "built the bot
    navigation graphs in 4.02s" line)
  - ~2 s before that, mostly parsing the map `.glb`s, **twice**
- **The map files are almost all texture:**

  | File | Size | Textures | Triangles (placed) |
  |---|---|---|---|
  | `der_tempel_map.glb` | 99.8 MB | 46 images, **99.2 MB** | 44,766 |
  | `break_point_map.glb` | 47.4 MB | 6 images, **47.3 MB** | 934 |
  | `shipment.glb` | 50 KB | none | 506 |
  | `basic_map.glb` | 29 KB | none | 244 |

  The server only reads vertex positions and triangle indices out of these
  (`server/src/collision.rs`, `collect`). The geometry it actually uses is
  well under 1 MB. The other ~146 MB of textures is:
  - **embedded in the server binary** (`include_bytes!`), so it's in the
    Docker image and gets read off disk at every boot
  - **copied onto the heap at startup**: `gltf::Gltf::from_slice` copies the
    whole binary chunk, textures included, into a `Vec`. That happens once
    from `KnivesPlugin` and once more from `BotAiPlugin` (both call
    `MapColliders::load()`), so startup briefly holds an extra ~150 MB
    twice over.

## Measure first

1. **Log tick time and memory every 30 s.** Average and worst tick in ms,
   how many ticks behind real time, living zombies per lobby, and RSS (from
   `/proc/self/statm`). Also warn whenever a tick overruns its 15.6 ms
   budget. Then `fly logs` shows straight away whether a change helped, and
   a slow server shows up as a warning instead of having to be worked out
   from frozen zombies.
2. **Profile a heavy round locally.** Run a release server limited to a
   fraction of a core so problems show up on a fast desktop, e.g.
   `systemd-run --user --scope -p CPUQuota=50% cargo run --release -p server`.
   Start Der Tempel on round 20+, then record it with `perf record -g -p
   <pid>` or `cargo flamegraph`. The flame graph shows which systems
   actually cost the most, before guessing at any of the CPU items below.

## Memory and startup

1. **Load the map colliders once.** `KnivesPlugin` and `BotAiPlugin` each
   call `MapColliders::load()`, so every map gets parsed twice and two
   identical sets of triangle meshes get built. Build it once (e.g. in
   `main.rs`, before the plugins) and share it. This saves about half of the
   pre-nav startup time and one full set of collision meshes.
2. **Strip textures at build time.** The plan is below. It removes ~146 MB
   from the binary and Docker image, and the large heap copies at startup.
   It needs no change to how maps are made in Blender.
3. **Drop the runtime `gltf` dependency** once (2) is in. Only the build
   script would still parse `.glb`s.
4. **Build nav graphs lazily or in the background.** Right now all four
   maps' graphs are built before the server starts listening, even though
   a game only ever needs one or two. Options:
   - Build each map's graph the first time a lobby starts on that map, while
     the clients are on their loading screen anyway.
   - Build them on a background thread after the socket is up, and have
     lobby start wait for that map's graph.

   Running them in parallel won't help, because `performance-1x` has one
   core.
5. **Answer the wake knock only once the server's ready.** The TCP health
   listener starts before anything else (`server/src/main.rs`), so the
   client's knock gets an answer about 1 s after boot. The client then
   spends the remaining ~5 s on "CONNECTING" with failed attempts instead of
   "WAKING SERVER". Starting the listener (or replying to the knock) only
   after the UDP socket is bound would keep "WAKING SERVER" up for the whole
   startup. (4) and (5) together would make waking almost instant.
6. **Check what the nav graphs cost in memory.** Der Tempel has tens of
   thousands of nodes, each with edges, regions and door data. Check against
   the RSS log in "Measure first" before changing anything. A tighter node
   layout is only worth it if this turns out to be a large share of the
   112 MB.

## CPU during a game

At round 13+ with 2 players, up to 30 zombies are alive at once
(`zombies::max_alive`). `ai::drive_bots` runs every tick (64 Hz) for every
one of them. Each zombie, each tick, does a line-of-sight check
(`segment_blocked`), a collision move (`move_bot`: overlap push, one or two
sphere sweeps, a ground ray), and an A* path plus smoothing every 1.2 s
(`REPATH_SECS`). Smoothing does several more sweeps per path. All of these
run against Der Tempel's 45k-triangle mesh.

1. **Look up targets with a map, not a scan.** `drive_bots` finds each
   zombie's target with `others.iter().find(...)`, a linear search per
   zombie per tick. A `HashMap<PeerId, …>` built once per tick is a small,
   easy win.
2. **Think less often than moving.** Movement needs every tick so motion
   stays smooth. Picking a target and the line-of-sight check don't:
   10–16 Hz is plenty for a zombie to notice you. Stagger it by entity so
   each tick only does a quarter of the zombies, which cuts the per-tick
   line-of-sight rays by about 4×.
3. **Repath only when it matters.** Skip a repath when the target is still
   at the same nav node as last time, not just "within 4 m"
   (`REPATH_TARGET_MOVED`). Zombies chasing a player who's standing still
   then stop recomputing the same path every 1.2 s.
4. **Simplify the collision mesh in Blender.** Instructions are below. Every
   ray, sweep and overlap above costs roughly with how many triangles are
   nearby, and the nav graph build casts thousands of rays at startup.
   Fewer triangles makes all of that cheaper, and removing decorative detail
   also stops zombies snagging on it.
5. **Use one flow field per player instead of an A* per zombie.** Every
   ~0.5 s, run one Dijkstra outward from each living player over the nav
   graph (respecting shut doors). Each zombie then just steps to its
   neighbouring node with the lowest cost. This replaces up to 30 A* searches
   and smoothing passes with 2–4 graph fills, and it doesn't get more
   expensive with more zombies. This is the biggest change, and it touches
   the trickiest code (`ai.rs`, `nav.rs`: mantles, windows, monkey bombs,
   doors), so do it last and only if the profile says pathing is the cost.
6. **Replication.** Lightyear sends updates at 20 Hz
   (`REPLICATION_INTERVAL_MS = 50`) and only sends what changed since the
   last ack. Check in the profile how much time serialising ~30 zombie
   poses takes. If it's significant, lower-precision poses or a slower rate
   for far zombies are the levers.

## Plan: strip textures from the server's map models (no Blender change)

You keep exporting one `.glb` per map with textures. The client uses it as
it does today, and the server build throws away everything except the
triangles.

1. **`server/build.rs`**, a build script that runs when the server compiles:
   - For each map `.glb` (the same four paths `collision.rs` embeds now),
     parse it with `gltf` (moved to `[build-dependencies]`).
   - Walk the default scene exactly as `collision.rs::collect` does now:
     triangle primitives only, each node's full transform applied. Do
     **not** apply the map's placement (`shared::map::placement`); that
     stays at runtime so it remains in one place for client and server.
   - Write a tiny flat file per map to `$OUT_DIR/<map>.col`: a magic/version
     header, vertex count, `[f32; 3]` positions, triangle count, `[u32; 3]`
     indices, all little-endian. Der Tempel comes out at roughly 0.5–1 MB.
   - `cargo:rerun-if-changed=` on each `.glb` (and on `build.rs`), so it only
     re-runs when a map changes.
2. **`server/src/collision.rs`:**
   - `include_bytes!(concat!(env!("OUT_DIR"), "/der_tempel.col"))` instead
     of the `.glb`.
   - Replace `MapMesh::from_glb` with a `from_flat(bytes, placement)` that
     reads the header and arrays (with `from_le_bytes`, so alignment doesn't
     matter), applies the placement, and builds the same
     `TriMesh::with_flags(…, MERGE_DUPLICATE_VERTICES)`.
   - The collision result is identical: same triangles, same transforms,
     same flags.
3. **Keep the docs honest:** update the module comments in `collision.rs`
   and `Cargo.toml` that describe the server parsing `.glb`s, and the
   `.dockerignore` comment ("a few tens of KB each" was never true for the
   art maps). The Docker build context still needs the `.glb`s, because the
   build script reads them.
4. **Verify:**
   - `cargo test -p server`: the existing collision and nav tests (floors,
     wall bounces, knives resting in the yard, every door splitting its map)
     all run through `MapColliders::load()`, so they cover the new path.
   - Add one test that each map's flat file has the expected triangle
     count (44,766 for Der Tempel right now).
   - Expected result: `target/release/server` shrinks by ~146 MB, startup
     loses most of its ~2 s pre-nav time, and the startup heap spike is gone.

This also protects you later: whatever you export, textures never reach the
server.

## Optional: a separate collision export from Blender

You only need this for the CPU side (CPU item 4): fewer, simpler triangles
for every collision query and a quicker nav build. The texture strip above
already handles memory. The client already supports it: Shipment ships
`shipment.glb` (collision blockout) plus `shipment_visual.glb` (art).

### In Blender

1. **Make a collision collection.** In the Outliner, right-click → New
   Collection and name it `Collision`. Select the map objects, duplicate them
   (Shift+D, then right-click to cancel the move so they stay put), and move
   the duplicates into `Collision` (M → Collision). Never move, rotate or
   scale the collection itself: both exports must share the same origin and
   scale, or collision won't line up with what players see.
2. **Delete what nothing can touch.** This includes props out of reach,
   ceiling and roof detail, foliage, small clutter (cups, bricks, trim),
   faces hidden inside walls, and scenery outside the playable area (the
   foggy far side).
3. **Swap detailed props for simple shapes.** A crate, statue or machine
   becomes a cube (Add → Mesh → Cube, set its size in the N panel's
   Dimensions to cover the prop), then delete the original from
   `Collision`. Pillars become 8–12-sided cylinders. Rubble piles become a
   few boxes or a wedge.
4. **Reduce organic shapes.** On terrain and rocks, add a **Decimate**
   modifier. Use *Planar* mode (~5°) to merge flat areas, or *Collapse* with
   a lower ratio. Then apply it.
5. **Clean up.** Select all in Edit Mode, then Mesh → Clean Up → **Merge by
   Distance**, **Delete Loose**, and **Limited Dissolve**.
6. **Keep anything gameplay depends on exactly as it was:**
   - doorways (the nav door zones must still split the map; the
     `every_door_splits_its_map` test checks this)
   - window frames and sills (zombies raycast the sill to climb in)
   - stairs and ledges players mantle
   - walls players wallbang through, and cover they hide behind (shots and
     line of sight use this mesh)
   - floors under water

   Stairs can become a ramp, but test how that feels to walk on first,
   since the client walks on this mesh too.
7. **Export the collision file.** With the `Collision` collection active,
   go to File → Export → glTF 2.0 (.glb):
   - **Include:** Limit to → *Active Collection*. Untick cameras and
     punctual lights.
   - **Data → Mesh:** *Apply Modifiers* on. **UVs off, Normals off, Vertex
     Colors off.**
   - **Data → Material:** Materials → *No export*. Images → *None*.
   - **Animation, Shape Keys, Skinning:** off.
   - Leave **Transform → +Y Up** on, as for the visual export.
   - Save these settings as an operator preset (the **+** next to
     "Operator Presets" at the top of the export dialog), named "Collision".
   - Export as e.g. `der_tempel_collision.glb`. It should come out at a few
     hundred KB at most.
8. **Export the visual file as before,** with `Collision` excluded. Untick
   its checkbox in the Outliner, or use *Limit to → Visible Objects*. Export
   as e.g. `der_tempel_visual.glb`.

### In code, once a map has both files

- `shared::map::collision_model_path`: point the map at
  `der_tempel_collision.glb`.
- `client/src/environment/map.rs::map_visual_path`: add
  `DerTempel | DerTempelDay => "models/maps/der_tempel_visual.glb"`. The
  client then shows the visual model and hides the collision one once the
  visual loads, the same as Shipment.
- `server/src/collision.rs` and `.dockerignore`: embed and allow the
  collision file instead of the art file.
- **Minimap:** `minimap::bake_current_map` draws from the `MapModel`
  (collision) meshes, so it would show the simplified shapes. Either accept
  that (it's a top-down silhouette), or have it bake from `MapVisualModel`
  when there is one.
- Re-check the zombies layout (`shared/levels/der_tempel.ron`): windows,
  doors, barrier and spawn spots still need to land on the new floors.
  `cargo test -p server` catches doors that no longer split the map and
  spots that fall through.
