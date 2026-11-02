Run the client against the prod server: `TRICKSHOT_PROD=1 cargo run`

# Done

- Level editor map hiding / height cut
- Zombie walk / run variety
- Scope blur, vignette and fringe
- Weapon movement inertia
- Weapon motion blur setting
- Bot kill cam sniper kick
- Motion blur setting
- Sniper shot kick-back removed
- MW3 smoothness: zoom curve, sniper kick, smoother sway
- Instant kill / hit feedback
- Retuned blood overlay
- Zombie explosion ragdolls
- Longer zombie corpses
- Muffled sounds through walls
- Kill chain medals
- Mantle breath + step sounds
- Downed breathing loop

# Level editor

- Add lights to zone above lower tomb
- Add fires to all fire bowls like doors that have to be opened on left side

# Current todo

- Windows in spawn are too high up. Could just shift down the vertices a certain amount.
- Fix fire light getting inside temple
- Re-enable zombie arms-up walk (`ZombieAvatarSettings::arms_up_walk`, off while testing the shambles)
- Default frame cap to the display's refresh rate (judder when uncapped)
- Make teleport keybind only work when in debug mode so that it won't be accidentally pressed when playing normally
- Test and improve server performance for cpu and memory
- Could download bottle model and replace the plastic model in the drinking arms with it and redo the animation, then just swap out the material that is used if they would all look correct.
- Dial in zombie and other enemy / remote player footsteps and ensure they also make the water noise when in water
- Add full power power up
- Add fire sale power up
- Change settings for when a zombie spawns in the window area and when it climbs out of the ground. On higher rounds more should just spawn out of the ground. Handle this how cold war does it.
- Cutouts in walls for zombies to climb through

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

# Map

- Add cut outs for zombies to climb through that can be repaired with boards and carpenter
- Add fire bowls to zone above temple

# Animations

- Add other zombie walk and attack animations

# Asset todo

- Add more fire pits
- Could update pap camos
- Finish other side of map but could keep it dark to keep fog look or at least allow the player to go out in the fog for something like a trap

## Next changes (most likely to help first)

1. Throttle animation for off-screen / far zombies — 56 bones each, all animated every frame even when culled; the biggest per-zombie cost left (watch for frozen shadows of off-screen ones)
2. Trim the remaining shadow cost if the GPU is the limit — the moon's EXTRA cascades (4096, 4 cascades, 160 m) and the power lights' shadow ranges
3. Don't spawn name tags for zombies — never shown, but each gets a hidden tag that's checked every frame
4. Only touch the PaP camo materials when their scroll actually changes — rewritten every frame now
5. Compress the models' textures — memory sits at ~10.8 GB in game (memory / load time more than FPS)

# Bugs

- On zombies if player somehow goes below a certian point or outside of the map bounds they shold be teleported back in. The spawn point shouldn't spawn the player in where they drop down below the map and have to press the teleport button.
- If a zombie is stuck all the way inside of a box it can't escape, add a way for it to be pushed out of it.

# Improve / Adjust

- Dial in muffle ( low pass ) of sounds behind walls. When going behind something like a pillar in an open space, the sound is muffled.
- Knife arms too high and far out
- Dial in molotov flame look
- Dial in phd slider effect look ( trail and explosion )
- Night shipment is a little too dark in shaded areas and can't see remote player model that well
- Increase aim sway

# Big features

- Add semtex

## Sound

- Can add a flash light on off sound
- Mystery box bye bye and laugh need to be louder along with other move sounds
- Add wind sound to der tempel that plays only when outside
- Create new rampage inducer music
- Level out perk jingle sound volumes ( Deadshot perk jingle seems too quiet )
- Add current player death sound. Sound should be a body fall sound mixed with a disonant synth sound.
- Add save teleport sound
- Add footstep sounds for other players
- Add pap shot sounds
- Look into using different footstep sounds based on surface

# Nice to have

- Add sun flare when looking near sun on day maps
