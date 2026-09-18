# Added

Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

Ordered easiest → hardest to fix / add.

- Use sniper icon at bottom right when sniper is loaded out for ammo ui. Create knife icon to use when it is loaded out.
- Add scope selections in loadout ( just different zoom levels, model won't change ). Will need to adjust ui control settings for different zoom levels to ensure what the scope renders lines up with the view outside the scope

# New added

- Add weapon sway to knife model which should be exactly the same as with the sniper sway settings
- Many lines on score aren't correct. For instance a long shot will be awarded when the shot isn't long or a 720 awarded when a 360 is done.

# Done

## Do Later

- Can remove some things from the top right controls ui (skipped for now — asked which sections to trim, told to come back to it later)
- Add tabs to top right controls ui to group other tabs into
- Replay in kill cam isn't smooth. Movement of sniper is jittery like it is snapping from one position to the next very quickly

## Performance / Feel

- Mouse look felt less smooth/snappy than Call of Duty — the window was left on Bevy's default `PresentMode::Fifo` (vsync on), which queues up to ~3 frames and caps the render rate to the monitor's refresh rate. Switched to `PresentMode::AutoNoVsync` (`client/src/main.rs`) for uncapped rendering and lower input-to-screen latency. Trade-off: possible screen tearing without a variable-refresh-rate display.
- Added a VSYNC toggle (off by default) and a FRAME RATE LIMIT slider (60-360, shown only while vsync is off) under Settings → Graphics → Display, so a player who prefers vsync's no-tearing trade-off, or wants to cap GPU/fan noise instead of running fully uncapped, can do either. `Settings::vsync` / `Settings::frame_limit`, applied live by `apply_vsync` / `limit_frame_rate` in `client/src/settings.rs`; the frame limiter paces by sleeping out the remainder of each frame's budget, only while vsync is off.

## Bugs

Ordered easiest → hardest to fix.

- Player who isn't party members screen still shows end game screen with continue button even after a game starts.
- It doesn't show to update anywhere on the client after it is launched and a new release is out.
- Z says capslock won't work to set as crouch / slide keybind
- When aiming through the sniper scope on shipment, the sky looks normal in that it is bright and the fog isn't visible.
- When backing out of free for all then going to the basic map free style, players don't see each others remote model moving
- Bug: dev-log spam on launch from a stale `.wav` asset path (`ambient_nature.wav`) — the file is `.ogg` now; fixed the load path in `audio.rs`.
- Bug: knife view model stayed visible on top of the replayed sniper during a kill cam if you'd switched to the knife right before dying — `weapon_system` (the only thing that toggled `KnifeViewModel`'s visibility) is disabled for the whole replay, so `drive_killcam` now drives it too, mirrored off the recorded sniper visibility per frame (`client/src/killcam.rs`).
- Bug: a reload (or any other in-progress one-shot sound) kept playing through a kill cam replay instead of cutting off — `start_killcam` now despawns every playing `AudioSink` entity except `AmbientAudio` right when the replay takes over; the replay re-fires its own recorded sounds on top as it plays (`client/src/killcam.rs`).
- Bug: crouch / slide / prone didn't replay in kill cam — the killer's camera height (`Slide::drop`, written onto the head's Y translation by `crouch_slide`, which is disabled for the whole replay like the rest of live movement) was never recorded, so the head stayed pinned at standing height for the entire replay regardless of what the killer's stance actually was. Added `crouch_drop` to `PlayerInput` / `KillCamSample` (`shared/src/protocol.rs`), recorded it on both the client (`net::write_input`) and server (`server/src/killcam.rs`'s `record_frames`) plus Practice's local ring buffer (`client/src/killcam.rs`'s `record_local_replay`), and `drive_killcam` now interpolates it onto the head's translation each frame instead of leaving it at zero.
- Bug: the basic map showed a doubled-up ground plane — `basic_map.glb` now has its own `Plane` ground node (confirmed by inspecting the file), which already gets real collision for free from the whole-scene `AsyncSceneCollider`, but the generic procedural ground bevy always spawns underneath every map was still shown and collidable too, at the same height. Same double-collider bug class already fixed for Shipment (grounded-edge flicker / spurious landing sounds from two coincident colliders), so applied the same fix: `setup_world` now spawns the procedural ground already `Visibility::Hidden` + `ColliderDisabled` (`client/src/main.rs`), and `sync_shipment_only_visibility` no longer has a per-map ground toggle at all — every current map ships its own now. Also set the basic map's default scale to `0.65` (`MapSettings::default`, `client/src/environment/map.rs`) to match the updated model.

## Sound

Ordered easiest → hardest to fix.

- Add save teleport sound
- Add knife sounds
- Add footstep sounds for other players

## UI / HUD

Ordered easiest → hardest to fix.

- Made the main-menu "What's New" notes use a readable body font (Inter, `assets/fonts/Inter-Variable.ttf`) instead of the condensed all-caps HUD font — see `crate::ui::label_body` / `BODY_FONT` in `main.rs`. Only that panel's notes changed; the HUD/menu titles still use `HUD_FONT`.
- Add ping ui beside fps
- Add tab to show leaderboard
- Add heart beat sound and red around screen when health low
- Add end game screen with play again button
- Add dot over other players head when playing trickshot mode that goes to the correct side of the screen when looking away from them

## Gameplay Features

Ordered easiest → hardest to fix.

- Added a "waiting for party" screen: starting a lobby game now shows the map/mode plus each member's loading/ready status (centre screen, reusing `menu::Screen::LoadingGame` to freeze gameplay input/HUD the same way the pause menu does) until every member's client reports its assets loaded, then hides and gameplay starts normally. New client module `client/src/game_start.rs`. Simplified from the original ask: no forced minimum display time (`if both load fast still show for ~3s`) and no separate bottom-left indicator — the per-member ready list already shows whether it's on you, at the cost of the screen sometimes only flashing briefly. "Assets" currently only tracks the map scene (`environment::map::MapLoadState` — `SceneInstanceReady` on the collision blockout, plus the nicer visual overlay if the map has one), not every other asset (bot/soldier models, textures, etc.) — revisit if those turn out to matter in practice. New wire types: `shared::AssetsReady` (client → server trigger) and `LobbyMember::loaded` (reset on `StartGame`, set server-side on receipt, replicated back out with the rest of `Lobby`).

- Add jumpshot points
- Add spawn points for maps
- Add hitmarkers ( where when shooting a bot or another player lower and from a distance it doesn't kill them and you get the hitmarker sound )
- Add different optic options.
- Add effect to scope so it looks like actually being aimed through a scope.
- Add ramped slow mo final kill of the game
- Add camera change when killed where it looks at the direction you were shot from like on cod then shows you die in third person then plays kill cam
- Add bots ( using remote player model ) that can be added to game modes

## Movement

- Added ledge mantling (Call of Duty style). New `client/src/player/mantle.rs`: `try_mantle` runs a three-probe ledge check (forward wall probe at chest height → downward probe past the wall face for the ledge top → upward probe at the landing spot for headroom) whenever the player is airborne, in `Stance::Standing`, and moving toward something; `drive_mantle` then eases them up and onto it (rise first, then move forward, so the camera clears the ledge face instead of cutting through it) over `MantleSettings::duration`, pausing every other movement/gravity/firing system for the climb (`not_mantling` run condition, mirroring how `killcam::no_killcam` pauses them for a replay). Player-facing setting: Settings → Controls → "Automatic Mantle" (Off / Semi-Auto / Full-Auto — `Settings::auto_mantle`, same three options and semantics as real CoD: Semi-Auto only triggers while actively jumping, Full-Auto any time you're airborne toward a ledge). Dev tuning: debug panel's "Mantle" section (`MantleSettings` — probe heights/distances, min/max ledge height, climb duration). No climb animation yet — the view model/arms/knife just ride along however they normally would; noted as a follow-up once the user handles animations.

## Loadout

- Added a LOADOUT screen for picking the scope crosshair — reachable from the main menu (`lobby_ui::MenuBtn::OpenLoadout`) and, in a game, from a button in the pause menu (`menu::Btn::OpenLoadout`). Both open the exact same `menu::Screen::Loadout`, built purely from `Settings` with no state/lobby involvement, so it's guaranteed to look identical either way rather than being two screens kept in sync by hand. Shows all three reticle textures as bordered image tiles, highlighted border on the current pick; selection persists via `Settings::crosshair` (`client/src/settings.rs`), applied live to the scope's reticle material by `weapons::scope::apply_crosshair_texture`. Renamed the three texture assets to describe what they actually look like: `crosshair.png` → `hash_reticle.png` (plain bullet-drop-compensator hash marks), `crosshair_2.png` → `duplex_reticle.png` (circular scope vignette + simple duplex cross), `red_dot_crosshair.png` → `hash_reticle_red_dot.png` (the hash reticle with a glowing red centre dot). Only the crosshair is selectable for now — weapon and scope-zoom loadout options are still open (see `# Added`).

## Weapons & Models

Ordered easiest → hardest to fix. Note: a knife model was added recently (see git log) — check whether "Add knife and throwing knife models" is now partially done.

- Fixed: switching to the knife no longer blocks attacking while its grip-adjust animation plays. `Adjust Grip` is now a random idle fidget (every 3-10s while idle, `KnifeAnimState::next_adjust_in`) instead of auto-playing right after the draw, and a `KnifeBusy::interruptible` flag lets a fire press cut it short instantly and swing instead of waiting it out (`client/src/weapons/weapon.rs`).
- Add knife and throwing knife models
- Add throwing knife model with throwing arms and implement throwing it and hitting enemies.
- Need to make a change so that the glb file is used for collision detection.

## Docs / Housekeeping

- Reworded todo.md: moved the "don't message unless asked" note to the top of `# Added`, and ordered the `# Added` list easiest → hardest to match the `# Done` convention.
- Updated `client/README.md`'s stale prototype-era technical section (single-weapon, `L`-key animation dev tooling) and `server/README.md`'s "Bots" upgrade-path note (stationary practice bots are already implemented in `server/src/bots.rs`); kept the still-accurate player-facing install/play instructions.
- Expanded `CLAUDE.md` with a "What this game is" summary and a short "Working in this repo" checklist (todo.md workflow, changelog rule, build/test commands, the `Startup`-schedule gotcha).

---

## Security

- Ensure that public github repo can't let random people from using the production server and run up the cost.
- Add security features to production server like rate limiting and max concurrent user count.

## Performance

- Check shader preloading like cod

## Refactor

- main file is thousands of lines so it should be refactored.

## Ideas

- Can add bullet impacts once map is created.
- Add grappling hook or teleportation where the player can aim on a teleport point that will highlight when aimed on and a keybind can teleport the player to it.
- Could add breath hold to reduce aiming idle sway then the breath sound effect with the sudden increase in sway
- See if it is possible for the OS key to not cause keybind behavior like cod does.
- Add lights inside of far containers in bevy and in blender (likely done — see recent "Add lights to shipment" commit)

## Scoring Points

- Could add sounds for the highest score line per shotso headshot could have a sound, no scope could have a sound, etc. It would play the sound that has the highest score since there can be multiple at a time. Sounds can be things like a mario type level up or coin sound, a chaching sound, a taco bell bell toll sound, etc.

+100 180, 360, etc ( right or left )
+100 no scope
+100 collat ( multiply by number of extra bots hit )
+100 silent shot ( need to add throwing knife )
+100 midair weapon swap ( need to add knife )
+100 throwing knife ( could be a multiplier for the entire set of points where sniper shot is double or some other amount)
+100 headshot
+100 wallbang
