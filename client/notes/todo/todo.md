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

# Latest play

- Some operator quotes are overlapping instead of being in a queue and waiting on the first to finish with a timer the keeps the operator from saying an old line serveral seconds later after the event that triggered it has occurred.
- Ensure a priority queue is used for operator quotes with the priority set correctly
- Maybe some more important quotes can interupt and cut off quotes that are already playing
- Kill sound ( seems like power up accept sound is the same ) is too loud. It should be about 80 percent the volume
- Ak74 ammo seems like its reloaded around a half or more second after the reload animation visually completes. If the sound and animation seem like they're finished then you switch weapons thinking its done, when you switch back the reload animation has to play again because it wasn't actually done. Can show a video of this with a screen recording.
- Need to remove whatever was done to "correct" the materials of the walls using the same material. In the future this should instead just be done in blender.
- Need to add a ledge in the lower temple upper level near where the sarcophagus is.
- Currently the perk quote waits for the full jingle to play. If the player runs away and can barely hear it anymore this seems odd. Maybe if the player goes a certain distance away from the perk machine after buying it go ahead and play the perk quote then.
- Find what level of sharpness looks good, then set that as the default.
- Possibly test out different final pass effects / coloring as long as it doesn't decrease fps. Need to check fps before and after making the change.
- Change pap prices to 5k, 15k, 30k.
- When pausing, sometimes fps weapon animations keep playing.
- vial and dagger pickup sounds should play the normal dropped eqiupment pickup sounds
- When astral projecting up to inside the pyramid, player can just get the weapon before the final zombei quote plays. Maybe this quote should play as the player is astral projecting.
- Maybe make the player face the main altar during the ritual where the blood is poured and also face the zombie while the initial quotes are being player and it awakes but still keep the player from being able to do anything until the boss zombie is active. Maybe even add in black bars on the top and bottom of the screen so it is clear it is a cienamatic scene almost like a cut scene.
- Probably would be better to cause some sort of damage with the wonder weapon with the zombies staying on the ground while they are being sucked then once complete, only then have them smoothly animate upwards and souls removed then killed.
- ensure no boss zombies are affected by instakill or nukes
- Create wonder weapon model
- Create animated easter egg boss zombie model and give it a name different from the regular boss
- active zombies who aren't dead when ritual starts should have speed greatly reduced and should stop further away from players instead of getting right up on them.
- Need a scary sound that plays when these zombies are killed due to the ritual. Could also add a smoke effect on the zombies like magic is killing them.
- Diamond above in pyramid altar table should be much lower
- More zombies should be spawned when the boss awakens zombies on phase 2 and 3. Phase 2 and 3 should have the same amount of zombies awakened but phase 3 should of course also add in dogs but now many.
- Ensure that on phases 2 and 3 a small amount of zombies keep spawning after they are killed to keep the distraction from the main boss during the fight. They shouldn't spawn instantly otherwise the player would have to just keep shotting the zombies the entire time.
- Add some sort of extra shader effect when in the astral mode getting the wonder weapon.
- Add some sort of glow / light around the wonder weapon when inside the pyramid
- For the astral effect, add in realistic dmt / astral projection humming / buzzing sounds with a binural beat effect also where the left and right sounds are actually different.

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
