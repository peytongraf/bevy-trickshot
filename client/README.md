# bevy-trickshot (client)

This is the game client, one crate in the `bevy-trickshot/` workspace (see the
repo-root `README.md` for the server and the shared protocol/ballistics crate).
Run it with `cargo run --bin bevy-trickshot` from the repo root, or `cargo run`
from this directory.

It self-updates from GitHub Releases on launch (`src/updater.rs`); the release
flow and what a friend does to install/update are in the repo-root `README.md`
under "Distributing the client". `cargo run` never triggers the updater.

A tiny first-person prototype built with [Bevy](https://bevyengine.org/) 0.16.

It loads `sniper.glb` (an animated sniper view model with arms) and parents it to
the player camera. The glTF ships one baked clip, `allanims`, that packs every
action back to back; `SEGMENTS` in `src/main.rs` slices it into the individual
animations by frame range. The scene is a lit ground plane with a gizmo grid
under a `citrus_orchard_puresky_8k.hdr` sky.

The sky is the equirectangular HDR mapped onto the inside of a big UV sphere
(`SkySphere`, radius `SKY_RADIUS`) that `sky_follow_camera` recentres on the
camera each frame — not a cubemap `Skybox` (which would need offline conversion),
and it does not light the scene. Swap in a `.ktx2` cubemap + `EnvironmentMapLight`
if you want image-based lighting later. If the sky looks mirrored, set the
`SkySphere` transform's `scale.x` to `-1.0`; if it's upside down, add a
`uv_transform` Y-flip to its material. The 8K source is ~0.5 GB of VRAM — a 2K/4K
version is plenty.

## Run

```sh
cargo run
```

Run it with `cargo run` (not the raw binary) so Bevy resolves the `assets/`
folder from the project root. `assets/sniper.glb` is the model in use;
`assets/sniper-old.glb` symlinks to the original Sketchfab export.

A white centre dot marks the screen middle for lining up the scope, and four
1.8 m reference capsules stand straight ahead at 10 / 25 / 50 / 100 m so the ADS
zoom is easy to read.

## Controls

| Input               | Action                                  |
|---------------------|-----------------------------------------|
| `W` `A` `S` `D`     | Move                                    |
| Mouse               | Look around                             |
| right mouse (hold)  | Aim down sight (zooms the whole view)   |
| `L`                 | Play the next animation segment once    |
| `Esc`               | Release / recapture the mouse cursor    |

### Aim down sight

Holding the right mouse button ramps `Ads::t` 0 → 1 over `ADS_DURATION`
(~130 ms). That single value simultaneously:

* blends the view model from the hip pose to the ADS pose,
* drops the world-camera FOV from `HIP_FOV_DEG` to `AdsTuning::fov_deg` (zooms
  the whole screen, in the scope or not),
* switches the **scope camera** on and fades its render target in on the rear
  lens (see below),
* scales mouse sensitivity down.

Nothing snaps; the cameras are never swapped.

**Scope (render to texture).** A second camera (`ScopeCamera`, child of the head,
FOV `AdsTuning::scope_fov_deg`) renders the world plus a reticle quad
(`assets/crosshair.png`, on render layer `SCOPE_OVERLAY_LAYER` so only this camera
sees it) into a `SCOPE_RT_SIZE`² image. That image is shown unlit on the
`lens_lens_0` mesh, so the player looks *through* a magnified, reticled view
instead of down the tube — and the crosshair, being part of that image, fades in
with the rest of it and never appears outside the scope. The scope camera has
`is_active = false` until `ads.t > SCOPE_SHOW_AT`, so it costs nothing at the hip.
It's parented to the head (not the animated scope) so the sight picture doesn't
wobble with the idle animation. `update_scope` also rescales the reticle quad so
it keeps exactly filling the scope view as `scope_fov_deg` changes.

The **ADS tuning** egui panel (top-left; press `Esc` to free the cursor to use
it) has sliders for the ADS pose x/y/z/yaw/pitch/scale, the main ADS FOV and the
scope FOV, a **Force full ADS** toggle for tuning without holding the button, and
a **Copy pose to console** button that prints the pose as a Rust literal for
`ViewModelPoses::default()`.

### Dialing in the animation segments

`L` plays `SEGMENTS[next]` from its start frame to its end frame, then parks the
animation there. Each press advances to the following segment (wrapping around),
so you can step through the whole clip and check each cut. On startup the console
prints the clip's real duration and the fps a 156-frame timeline would imply, plus
the second range each segment currently resolves to. Edit the frame numbers in
`SEGMENTS` and rebuild.

### Lining up the view model

`ViewModelPoses` holds the `hip` and `ads` poses. The tweak keys edit the ADS
pose while the right mouse button is held, the hip pose otherwise; `apply_ads`
blends between them each frame. Bake good values into `ViewModelPoses::default()`.

| Input                 | Action                             |
|-----------------------|------------------------------------|
| arrow keys            | Move gun forward / back / left / right |
| `PageUp` / `PageDown` | Move gun up / down                  |
| `[` / `]`             | Scale gun down / up                 |
| `,` / `.`             | Yaw gun   (hold `Shift` to reverse)|
| `;` / `'`             | Pitch gun (hold `Shift` to reverse)|
| `P`                   | Print both poses to stdout          |

## How it fits together

* A `Player` entity holds position + yaw. Its child `PlayerHead` holds pitch, so
  walking stays level regardless of where you look.
* Three cameras hang off the head: the world-model camera (layer 0, FOV driven
  by ADS), the view-model camera (layer 1, fixed 70° FOV, drawn on top), and the
  scope camera (layer 0, renders to the scope texture, `order: -1`, active only
  during ADS). The sniper scene and every entity it spawns are moved onto layer 1
  so the weapon never clips into walls.
* When the glTF scene finishes spawning (`SceneInstanceReady`), an observer finds
  the `AnimationPlayer` Bevy added for the skinned mesh, arms it on a one-node
  `AnimationGraph`, and parks it paused on frame 0 until `L` is pressed.
* That observer also tags every view-model entity `NoFrustumCulling`. A skinned
  mesh is culled against its *rest-pose* bounds, and this model's re-exported
  arms rest behind the near plane — without this they vanish from the camera
  while still showing up in the shadow pass.
* The observer finds the scope's rear lens by name (`lens_lens_0`) and swaps in an
  unlit blend-mode material sampling the scope render target; `update_scope` fades
  it in with ADS and toggles the scope camera / its magnification.
* The ADS tuning panel uses `bevy_egui`. It only needs the cursor when you want
  to drag a slider, so free it with `Esc` first.

## Note on the model

`assets/sniper.glb` was round-tripped through Blender. Blender's glTF exporter
drops vertex **tangents** unless *Geometry → Tangents* is ticked; the `sniper` and
`arms` materials are normal-mapped, so re-export with that option enabled to keep
the surface detail matching `sniper-old.glb`.
