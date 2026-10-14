Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Sound Terms

Positional vs nonpositional
World vs dry (reverb or no reverb)

# Done

- Kira audio engine
- Automatic room reverb

# Level editor

- Fix der tempel crafting table position
- Add fires to 4 bowls around pyramid and remove 4 lights
- Add lights to zone above lower tomb

# Current todo

- Cutouts in walls for zombies to climb through

# Map

- Add cut outs for zombies to climb through that can be repaired with boards and carpenter
- Add fire bowls to wooden building
- Add fire bowls to zone above temple

# Map Done

- Add ledge to side area that is lowered with the 2 large stair cases
- Add large fire bowls at the four corners of the pyramid and just remove the lights

# Added

- Make perk machines already on before power is cut on, maybe make this an option
- For the client rendering of molotovs, instead of several small flames, see if it is possible to make this shape look like one large realistic flame
- Make boss zombie glow somehow so they stand out from regular zombies when looking at a crowd of them together
- Add short cut scene of a map screen shot that plays before zombies, just use one of der tempel for all for now. Don't play when debug mode is on unless this would affect time for match to start. Could just see it if any players don't have debug mode turned on. Show a few different screen shots with an intro music.
- Could add a text near the top compass that shows the name of the curent zone
- Add ability to add more power lights to a map

- On YouTube video shot marks were on 0 level over speed cola even though the ground there is lowered
- Add cynder blocks to the level editor for the mystery box to sit on. These will sit at every mystery box location.
- mystery box animation is messed up because door slides to position it should be in normally without cinder blocks
- look into using different footstep sounds based on surface

# Asset todo

- Add new radio quotes
- add red lights in tomb
- Add more fire pits
- Make a better blood overlay
- could update pap camos
- Add underground connection where speed color is to other parts of the map
- Finish other side of map but could keep it dark to keep fog look or at least allow the player to go out in the fog for something like a trap

# Performance

- When using phd slider a lot on a higher round with a lot of zombies, fps dropped to around 70-90. This seems to just be the case after going through this many rounds instead of starting on a high round from the start.
- Could run btop while game is running
- Could possibly keep a client side graph of entities, effects, fps over time etc

# Flashlight

- Can add a flash light on off sound
- Can turn on flashlight if an area is too dark instead of having it only on when the power is off

# Bugs

- Check: Knife won't stab when enemies are totally point blank
- On zombies if player somehow goes below a certian point or outside of the map bounds they shold be teleported back in. The spawn point shouldn't spawn the player in where they drop down below the map and have to press the teleport button.
- If a zombie is stuck all the way inside of a box it can't escape, add a way for it to be pushed out of it.
- It doesn't show to update anywhere on the client after it is launched and a new release is out.

# Improve

- Dial in phd slider effect look ( trail and explosion )
- Night shipment is a little too dark in shaded areas and can't see remote player model that well

# Adjust

- Increase aim sway

# Big features

- Add semtex
- Add killchains

## Sound

- Create new rampage inducer music
- Level out perk jingle sound volumes
- Add current player death sound. Sound should be a body fall sound mixed with a disonant synth sound.
- Add save teleport sound
- Add mantle sound
- Add footstep sounds for other players
- Possibly convert audio to wav files.
- Add pap shot sounds
- When player is downed a breathing sound should play
