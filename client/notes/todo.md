# Done

- Update screen gates the menu; server version check
- Zombies pacing: CoD zombie counts, boss from round 9
- Dog rounds only drop last dog's max ammo
- Smoke on rebuilt window boards

# Current todo

- SETUP DIFFERENT MYSTERY BOX LOCATIONS THEN ADD MOVEMENT OF IT, BYE BYE, ETC
- Dial in zombie and other enemy / remote player footsteps and ensure they also make the water noise when in water
- Add full power power up
- Add fire sale power up
- Change settings for when a zombie spawns in the window area and when it climbs out of the ground. On higher rounds more should just spawn out of the ground. Handle this how cold war does it.
- Cutouts in walls for zombies to climb through

# Latest play

- Radio quote volume should be about 1.25 times as loud
- Blood overlay is too strong. 1/3 damage looks like near death.

# First person animations

# Remote player animations

- Raise right arm some on drink perk remote player animation so that cap is at mouth level
- Sniper that is used should still have the lenses. If the sniper model has these removed then instead use the model that doesn't have them removed and have bevy remove them just when in fps mode.
- Sniper and maybe other guns should not have the pap camo on some objects.
- Could have reload animation when moving because currently it is only with the player not moving so their feet slide if they move when reloading.
- Possibly see how hard it is to change the remote player model but keep bones / animations

# Added

- Could add a text near the top compass that shows the name of the curent zone
- Add ability to add more power lights to a map
- Player who is downed should see a progress bar as they are being revived by a bot for their revive percentage like the reviver sees.

# Mystery box

- Add cynder blocks to the level editor for the mystery box to sit on. These will sit at every mystery box location.
- Mystery box animation is messed up because door slides to position it should be in normally without cinder blocks

# Nice to have

- Add sun flare when looking near sun on day maps
- Possibly add high and low pass filters to sounds if they are behind walls

# Level editor

- Add lights to zone above lower tomb
- Add fires to all fire bowls like doors that have to be opened on left side

# Map

- Add cut outs for zombies to climb through that can be repaired with boards and carpenter
- Add fire bowls to zone above temple

# Map Done

# Animations

- Add other zombie walk and attack animations

# Asset todo

- Add more fire pits
- Make a better blood overlay
- Could update pap camos
- Finish other side of map but could keep it dark to keep fog look or at least allow the player to go out in the fog for something like a trap

# Performance

Benchmark (compare with the debug-mode PERFORMANCE report on the results screen; reports in `~/.config/bevy-trickshot/perf/`): Der Tempel night, Zombies, FULL POWER on, start round 21 → call the exfil at once, run to it, kill only with the Ray Gun (no nuke), win the exfil. Baseline 2026-10-22 (v0.4.8, after zombie frustum culling; 3840x2160, shadows EXTRA, 240 cap): avg 146 FPS, 1% low 101, 0.1% low 83; 30+ enemies 1% low 98; ~6,700 entities idle → 14,700 at ~52 enemies (~150 per enemy); draw calls ~950 → ~2,600 (~33 per enemy). Shadow Map setting then only changed the moon: NORMAL vs EXTRA was avg 166 vs 155, 1% low 113 vs 101 (after the zombie mesh merge + footstep cap). With only the nearest power lights casting shadows (EXTRA): avg 162, 1% low 111, 0.1% low 89; 40+ enemies 152 FPS vs 161 with under 5. It doesn't cover the "worse after many rounds" problem — that needs a long run from round 1 (watch ENTITIES AT ROUND START).

## Next changes (most likely to help first)

1. Throttle animation for off-screen / far zombies — 56 bones each, all animated every frame even when culled; the biggest per-zombie cost left (watch for frozen shadows of off-screen ones)
2. Trim the remaining shadow cost if the GPU is the limit — the moon's EXTRA cascades (4096, 4 cascades, 160 m) and the power lights' shadow ranges
3. Don't spawn name tags for zombies — never shown, but each gets a hidden tag that's checked every frame
4. Only touch the PaP camo materials when their scroll actually changes — rewritten every frame now
5. Compress the models' textures — memory sits at ~10.8 GB in game (memory / load time more than FPS)

## Tools and tests

- Add CPU vs GPU frame time to the perf report, to know which kind of fix will actually move the numbers
- Long run from round 1 with debug mode on, to check whether anything piles up over rounds (the by-round chart and ENTITIES AT ROUND START)
- When using phd slider a lot on a higher round with a lot of zombies, fps dropped to around 70-90. This seems to just be the case after going through this many rounds instead of starting on a high round from the start.
- Could run btop while game is running

# Flashlight

- Can add a flash light on off sound
- Can turn on flashlight if an area is too dark instead of having it only on when the power is off

# Bugs

- On zombies if player somehow goes below a certian point or outside of the map bounds they shold be teleported back in. The spawn point shouldn't spawn the player in where they drop down below the map and have to press the teleport button.
- If a zombie is stuck all the way inside of a box it can't escape, add a way for it to be pushed out of it.

# Improve / Adjust

- Knife arms too high and far out
- Deadshot perk jingle seems too quiet
- Dial in molotov flame look
- Dial in phd slider effect look ( trail and explosion )
- Night shipment is a little too dark in shaded areas and can't see remote player model that well
- Increase aim sway

# Big features

- Add semtex
- Add killchains

## Sound

- Add wind sound to der tempel that plays only when outside
- Create new rampage inducer music
- Level out perk jingle sound volumes
- Add current player death sound. Sound should be a body fall sound mixed with a disonant synth sound.
- Add save teleport sound
- Add mantle sound
- Add footstep sounds for other players
- Add pap shot sounds
- When player is downed a breathing sound should play
- Look into using different footstep sounds based on surface
