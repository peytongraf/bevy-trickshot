Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Pre practice removal cloc . in client src/

❯❯ /home/peyton/Dev/bevy-trickshot/client/src : cloc .
54 text files.
54 unique files.  
0 files ignored.

github.com/AlDanial/cloc v 2.10 T=0.02 s (2634.6 files/s, 864651.0 lines/s)
-------------------------------------------------------------------------------

Language files blank comment code
-------------------------------------------------------------------------------

Rust 54 1167 3374 13181
-------------------------------------------------------------------------------

SUM: 54 1167 3374 13181
-------------------------------------------------------------------------------

# Post practice removal

❯❯ /home/peyton/Dev/bevy-trickshot/client/src : cloc .
53 text files.
53 unique files.  
0 files ignored.

github.com/AlDanial/cloc v 2.10 T=0.02 s (2730.8 files/s, 879019.5 lines/s)
-------------------------------------------------------------------------------

Language files blank comment code
-------------------------------------------------------------------------------

Rust 53 1131 3299 12630
-------------------------------------------------------------------------------

SUM: 53 1131 3299 12630
-------------------------------------------------------------------------------

# Added

- Increase aim sway
- On windows terminal pops up to play prod client
- Add current player death sound. Sound should be a body fall sound mixed with a disonant synth sound.
- Update to bevy 0.19 from 0.16
- Many lines on score aren't correct. For instance a long shot will be awarded when the shot isn't long or a 720 awarded when a 360 is done.

## Dev Tools

- `P` now logs the player's world-space position (`transform.translation` on `Player`) to the client console — `log_player_position` in `client/src/player/movement.rs`. Deliberately a raw key check, not a `KeyBindings` entry, so it's not rebindable/shown in the Keybinds settings tab. Only fires while the cursor is grabbed (actually playing), matching how other gameplay input is gated.

## Do Later

- Can remove some things from the top right controls ui (skipped for now — asked which sections to trim, told to come back to it later)
- Add tabs to top right controls ui to group other tabs into
- Replay in kill cam isn't smooth. Movement of sniper is jittery like it is snapping from one position to the next very quickly
- Night shipment is a little too dark in shaded areas and can't see remote player model that well
- Remote player model transitions to new position when they dead and respawning instead of having their body stay there then disappear and a new model appear

## Bugs

Ordered easiest → hardest to fix.

- Player who isn't party members screen still shows end game screen with continue button even after a game starts.
- It doesn't show to update anywhere on the client after it is launched and a new release is out.
- Z says capslock won't work to set as crouch / slide keybind
- When backing out of free for all then going to the basic map free style, players don't see each others remote model moving
- Bug: the remote player's death animation didn't play in the kill cam — the replay's frozen `models/soldier.glb` ghosts (`KillCamPlayer`) carried no "this one was shot" flag, so the victim's ghost just idled where it stood while the bot ghosts (which have `KillCamBot::killed`) did topple. Added `KillCamPlayer::killed` (`shared/src/protocol.rs`), set server-side from the `FreeForAll` kill's victim (`server/src/killcam.rs`), and `drive_killcam` now plays the soldier `death` clip (once, at `death_speed`, held on its last frame) on that ghost when the playhead reaches `kill_time` (`client/src/killcam.rs`'s `KillCamPlayerGhost`).
- Bug: the knife's stab/slice animation didn't play in the kill cam — the recorder only captured the sniper's animation playhead (`anim_time`), so a replayed knife just sat in its idle pose. Added `knife_anim_time` to `PlayerInput` / `KillCamSample` (`shared/src/protocol.rs`), recorded from the knife's own `AnimationPlayer` on the client (`net::write_input`, Practice's `record_local_replay`) and server (`server/src/killcam.rs`), and `drive_killcam` now seeks + pauses the knife clip to it every frame the same way it does the sniper's; the replay's teardown (`drive_killcam` / `stop_killcam`) parks the knife at rest and resets `KnifeAnimState` so a slice that was mid-play at death can't leave the live knife stuck busy.
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
- Ammo HUD: the weapon icon is now 128×64 (twice its old size), and the `mag / reserve` text is hidden (`Display::None`, so the panel shrinks to just the icon) while the knife is the equipped slot (`update_ammo_ui`, `client/src/hud/ammo_text.rs`).
- Added a weapon icon (sniper/knife) next to the bottom-right ammo readout, swapped live by `update_weapon_icon` (`client/src/hud/ammo_text.rs`) whenever `Weapon::slot` changes. Icon box is a fixed size regardless of which texture is showing, so the swap never shifts the ammo readout beside it — `sniper_icon.png` and `knife_icon.png` are both drawn to the same 1774×887 canvas for exactly this reason. Moved both into a new `client/assets/textures/icons/` directory to keep `textures/` organized.
- Add ping ui beside fps
- Add tab to show leaderboard
- Add heart beat sound and red around screen when health low
- Add end game screen with play again button
- Add dot over other players head when playing trickshot mode that goes to the correct side of the screen when looking away from them

## Gameplay Features

Ordered easiest → hardest to fix.

- Added a "waiting for party" screen: starting a lobby game now shows the map/mode plus each member's loading/ready status (centre screen, reusing `menu::Screen::LoadingGame` to freeze gameplay input/HUD the same way the pause menu does) until every member's client reports its assets loaded, then hides and gameplay starts normally. New client module `client/src/game_start.rs`. Simplified from the original ask: no forced minimum display time (`if both load fast still show for ~3s`) and no separate bottom-left indicator — the per-member ready list already shows whether it's on you, at the cost of the screen sometimes only flashing briefly. "Assets" currently only tracks the map scene (`environment::map::MapLoadState` — `SceneInstanceReady` on the collision blockout, plus the nicer visual overlay if the map has one), not every other asset (bot/soldier models, textures, etc.) — revisit if those turn out to matter in practice. New wire types: `shared::AssetsReady` (client → server trigger) and `LobbyMember::loaded` (reset on `StartGame`, set server-side on receipt, replicated back out with the rest of `Lobby`).

- Added death by falling: an absolute void-height floor (`client/src/fall_death.rs`'s `VOID_DEATH_Y`) catches maps with no ground under a long drop (like the current basic map), and a CoD-style net-fall-distance check (`LETHAL_FALL_DISTANCE`) catches a lethal landing on maps that do have ground down there — same mechanic, two ways to trigger it. Works in both game modes: client-authoritative (matches how all local movement already works), the client decides it happened and tells the server (new `shared::FellToDeath` trigger); server-side, a `FreeForAll` player gets marked dead like a PvP kill, while a `Freestyle` player (no health concept, always "alive") just gets sent a fresh spot — either way it answers with the same `PlayerRespawn` a PvP kill gets, but queues no kill cam (no killer). The camera holds its exact position/orientation from the moment of death, weapon instantly hidden and the blood/red-tint overlay up exactly like a kill death (`death_effect::show_overlay_and_hide_weapon`, now shared between the two), while a one-off `models/soldier.glb` body — spawned fresh just for this, since the local player normally has no third-person model at all — keeps falling (carrying over the player's actual fall speed and horizontal drift) and slowly tumbling around a random axis, tracked by a continuous look-at; ends on `net::LocalPlayerRespawned`, same as `client/src/death_effect.rs`'s kill-death pan, which this was modeled on.

- Added a daytime Shipment map (`MapId::ShipmentDay`, "SHIPMENT DAY" in the lobby map picker): same `shipment.glb` / `shipment_visual.glb` models, collision, spawns and bounds as night Shipment (`MapId::is_shipment()` covers both), but with no rain (`update_rain` stays `Shipment`-only), very little fog, strong sun + bright sky ambient (`ShipmentDaySceneTuning` in `client/src/environment/atmosphere.rs`, live-tunable under the debug panel's "Fog & Sky (Shipment Day)"), `basic_map`'s clear-sky HDR instead of the overcast one, a brighter ocean tint (`WaterSettings::day_tint`), and the night-only floodlights / container fixture lights hidden. Reuses Shipment's ambience loop — swap in a daytime one if that reads wrong.
- Add jumpshot points
- Add spawn points for maps
- Add hitmarkers ( where when shooting a bot or another player lower and from a distance it doesn't kill them and you get the hitmarker sound )
- Add different optic options.
- Add effect to scope so it looks like actually being aimed through a scope.
- Add ramped slow mo final kill of the game
- Add camera change when killed where it looks at the direction you were shot from like on cod then shows you die in third person then plays kill cam
- Add bots ( using remote player model ) that can be added to game modes

- Bug: the scope lens showed the world without fog (e.g. a clear sky on foggy Shipment Night). `apply_scene_tuning` only writes each map's `DistanceFog` onto `WorldModelCamera`; the `ScopeCamera` had none. It now spawns with one, and `weapons::scope::sync_scope_fog` copies the world camera's fog onto it whenever that changes. Bloom is still only on the world camera.

## Movement

- Added ledge mantling (Call of Duty style). New `client/src/player/mantle.rs`: `try_mantle` runs a three-probe ledge check (forward wall probe at chest height → downward probe past the wall face for the ledge top → upward probe at the landing spot for headroom) whenever the player is airborne, in `Stance::Standing`, and moving toward something; `drive_mantle` then eases them up and onto it (rise first, then move forward, so the camera clears the ledge face instead of cutting through it) over `MantleSettings::duration`, pausing every other movement/gravity/firing system for the climb (`not_mantling` run condition, mirroring how `killcam::no_killcam` pauses them for a replay). Player-facing setting: Settings → Controls → "Automatic Mantle" (Off / Semi-Auto / Full-Auto — `Settings::auto_mantle`, same three options and semantics as real CoD: Semi-Auto only triggers while actively jumping, Full-Auto any time you're airborne toward a ledge). Dev tuning: debug panel's "Mantle" section (`MantleSettings` — probe heights/distances, min/max ledge height, climb duration). No climb animation yet — the view model/arms/knife just ride along however they normally would; noted as a follow-up once the user handles animations.

## Loadout

- Added a LOADOUT screen for picking the scope crosshair — reachable from the main menu (`lobby_ui::MenuBtn::OpenLoadout`) and, in a game, from a button in the pause menu (`menu::Btn::OpenLoadout`). Both open the exact same `menu::Screen::Loadout`, built purely from `Settings` with no state/lobby involvement, so it's guaranteed to look identical either way rather than being two screens kept in sync by hand. Shows all three reticle textures as bordered image tiles, highlighted border on the current pick; selection persists via `Settings::crosshair` (`client/src/settings.rs`), applied live to the scope's reticle material by `weapons::scope::apply_crosshair_texture`. Renamed the three texture assets to describe what they actually look like: `crosshair.png` → `hash_reticle.png` (plain bullet-drop-compensator hash marks), `crosshair_2.png` → `duplex_reticle.png` (circular scope vignette + simple duplex cross), `red_dot_crosshair.png` → `hash_reticle_red_dot.png` (the hash reticle with a glowing red centre dot). Weapon loadout options are still open.
- Added scope zoom levels (3x / 8x / 11x) to the Loadout screen — `Settings::scope_zoom` (`ScopeZoom`, default 11x, which is closest to the old ~12x scope). Model unchanged; only the zoom differs. Zoom is exact and relative to the player's hip FOV setting: `weapons::ads::full_ads_fov_rad` uses `tan(ads/2) = tan(hip/2) / zoom`. The scope camera's FOV is the world's ADS FOV scaled by a constant `AdsTuning::lens_fit` (in tangent space), calibrated from the old hand-tuned 9.5° world / 6.5° scope pair. The view-model camera has a fixed FOV, so the lens covers a fixed screen fraction, and one ratio keeps the lens picture lined up with the world at every zoom and hip FOV. The debug panel's FOV section now shows the computed FOVs and has one `lens fit` slider (replacing the two ADS FOV sliders). The kill cam replays the shooter's zoom: `scope_zoom` was added to `shared::PlayerInput` / `KillCamSample` (a wire change: client and server must be deployed together), and `killcam::KillCamRun::optic` overrides the live optic in `apply_ads` / `update_scope`. ADS sensitivity is _not_ scaled per zoom yet.

## Weapons & Models

Ordered easiest → hardest to fix. Note: a knife model was added recently (see git log) — check whether "Add knife and throwing knife models" is now partially done.

- Added weapon sway to the knife view model: `knife_weapon_sway` (`client/src/weapons/sway.rs`) applies the exact same turn-lag transform as the sniper's — `weapon_sway` and it now share one `sway_pose` helper and the same `WeaponSwayState` offset / `WeaponSwaySettings`, so the "Weapon sway" debug section tunes both and they can't drift apart. Turn-lag only (no idle breathing or recoil shudder on the knife). Scheduled after `apply_knife_transform` (which rewrites the knife's base pose every frame) and chained right after `weapon_sway`. The kill cam replays the recorded sway rotation onto the knife too (`drive_killcam`; rotation only, same as it already does for the sniper), which needed the knife's `Transform` added to its knife query (plus `Without<KnifeViewModel>` on `RigFilter`) and `.after(apply_knife_transform)` on the replay chain.
- Added knife stabs (Call-of-Duty style): with the knife drawn, pressing fire starts the slice animation and files a stab request from the camera's eye along its forward direction (`PendingMelee` / `LocalMelee`, `client/src/weapons/weapon.rs`; `PlayerInput::melee` rides the existing `fire_origin`/`fire_dir` fields). The server resolves it with `shared::melee::resolve_melee` — nearest body capsule within ~2.2 m and within ~50° of the aim, with body-radius + 0.4 m aim slack, always a kill, no headshot/pierce/falloff (`server/src/sim.rs`). `Freestyle`: kills a bot for `KNIFE_KILL_POINTS` (25, flat — no trick multipliers) via the normal `BotHit`, so the score popup, bot topple and kill cam all follow. `FreeForAll`: lethal `PlayerHit` (`KNIFE_DAMAGE`) so death/respawn/kill credit/victim kill cam are the existing path. Solo Practice mirrors it offline (`practice::resolve_local_melee`). Instant on press (no wind-up delay). Known gaps: no stab/hit sound (see "Add knife sounds"), and the kill cam replays the killer's knife as static (only the sniper's animation playhead is recorded, not the knife's). Bumped `PROTOCOL_ID` (also covers `KillCamPlayer::killed`).
- Fixed: switching to the knife no longer blocks attacking while its grip-adjust animation plays. `Adjust Grip` is now a random idle fidget (every 3-10s while idle, `KnifeAnimState::next_adjust_in`) instead of auto-playing right after the draw, and a `KnifeBusy::interruptible` flag lets a fire press cut it short instantly and swing instead of waiting it out (`client/src/weapons/weapon.rs`).
- Add knife and throwing knife models
- Add throwing knife model with throwing arms and implement throwing it and hitting enemies.
- Need to make a change so that the glb file is used for collision detection.
- Added the throwing arms view model (`models/arms_throwing.glb`, `client/src/weapons/throw_arms.rs`) for the throwing-knife key (still no knife to actually throw): holding it plays the equipped weapon's own Hide (sniper *or* knife — previously only the sniper, and it snapped away) at `weapon_hide_speed`× (default 6), then the arms slide up parked on the clip's first frame; releasing (once they're fully in) plays the `throw` clip once, the arms slide back down, and only once they're fully out of view is the previous weapon drawn again with its normal Show (`ThrowPhase` in `weapon.rs`). Pressing swap-weapon while held cancels instead (no clip, arms hide, same weapon back). The clip has no show/hide, so `slide_throw_arms` supplies one: the arms slide up from below while `ThrowingKnife::active` and back down after (snapping away instantly during a kill cam / death effect). Pose is tunable from the egui "Throwing arms" section, and weapon hide speed / arms show-hide speed / hidden drop from "Throwing knife" (`ThrowArmsSettings`; defaults dialled in by the user). Known gaps: the throw isn't recorded into the kill cam (arms just hide for the replay), and nothing is sent to the server or other players yet.

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
- Removed offline Practice mode entirely: deleted `client/src/practice.rs` (local bots, local scoring/score text, local knife resolution) and the `PRACTICE` main-menu button (`lobby_ui::MenuBtn::Practice`), plus everything that existed only for it — the kill cam's local ring-buffer recorder and pending-cam path (`killcam::record_local_replay` / `start_local_killcam` / `LocalReplay` / `PendingLocalCam`), the `LocalMelee` event, `lobby_ui::GameSession` (every `InGame` is now a lobby match, so `drive_ingame_exit` is unconditional), and the solo branch of the pause menu's leave button (`menu::LeaveCtx::online`). The shooter's instant tracer + ground dust survived as a slim `weapon::resolve_local_shot` (was Practice's `resolve_local_shot` minus the offline hit resolution). Doc comments and READMEs updated to match.

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
