Run the client against the prod server: `TRICKSHOT_PROD=1 cargo run`

# Done

# New latest play ( Der tempel, night, no starting money or full power, played from round 1)

- Top priority and very easy to change - make pap 1 5k, pap2 15k, pap3 30k. Do this first before anything else
- Quotes said after a certain time still play even when they haven't started yet and the game is paused. For instance the we do not know when we will have power on... adam quote played after the game was paused.
- The quick revive quote seems like it plays a few seconds after the perk jingle ends.
- On round 9 the big steve radio quote played normally but it seems like he spawned far away because he wasn't even seen until nearly all of the zombies for that round were killed. This isn't the first time this has happened. He should spawn relatively close to the players without beign right beside them. Close enough to be in the same room.
- Last zombie does the last zombie scream. During the ritual it wouldn't shut up and was extremely annoying. Zombies should stop making moaning noises during the ritual ( their death sounds should still play though when they die). The last zombie sound should instead play once when its the last zombie and after that it should play whatever moan / scream sound it normally plays.
- The sarcophagus should face the altar so that when the boss stands up he's facing the altar.
- A line of blood extends from the altar towards the sarcophagus but doesn't trace to objects at the ground level to get to it. Should it not move out at all on the ritual altar or trace all the way to it going over objects at ground level so that it can be seen? Before when it traced to the sarcophagus it went under the lifted portion where the altar sits on top of so it didn't look correct.
- Kill enemy sound effect is about 1.5 times too loud.
- When the exhumer shot is made and several enemies are killed at once, like 15-20 or so, the kill enemy sound plays so much and stacks on it self and seems extremely loud. How would call of duty handle this?
- The lid of the sarcophagus flies off and looks good but it ends up slightly clipping through the rasied surface that the ritual altar is on
- When pausing the game first person animations still play
- When sliding the fps running animation plays. Instead it shouldn't play because the player isn't taking steps here.
- For the exhumer, it would look more realistic if instead of the beam always going out perfectly straight, as the player moves their aim it behaved like a rope does as you move it around so that the end of it slightly lags behind the front. This effect can be subtle but it would help make it not look so robotic.

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
- ensure no boss zombies are affected by instakill or nukes

# Level editor

- Add lights to zone above lower tomb
- Add fires to all fire bowls like doors that have to be opened on left side

# Current todo

- 4 `server` nav tests were failing (Nov 1), e.g. "DerTempel door 11: the ground either side of it is one zone", from the map's door and spawn layout
- Windows in spawn are too high up. Could just shift down the vertices a certain amount.
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
