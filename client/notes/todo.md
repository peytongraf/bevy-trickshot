Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Sound Terms

Positional vs nonpositional
World vs dry (reverb or no reverb)

# Windows

width: 1.2m
height: 1.4m
1.1m above ground
outside ground height the same as inside

# Doors

width: 2m
height: 3m

# Last play

- Exfil model inside lower tomb in wrong place
- During exfil bosses took forever to get to the exfil zone. All enemies need to respawn when the player goes to it because I didn't even have a chance to kill the bosses.
- Probably twice as many dogs as there should be on exfil.
- Deadshot perk jingle seems too quiet

# Done

# Level editor

- Add lights to zone above lower tomb
- Add fires to all fire bowls like doors that have to be opened on left side

# Current todo

- Add full power power up
- Add fire sale power up
- Can add smoke efffect to boards on windows as the rebuild animation is playing
- Possibly add sound loading to things the loading ui has to wait on to finish. If this and other loading can be done in the background and can just make the game wait ( when it shows waiting on players in the lobby to load ) when starting a game.
- Change settings for when a zombie spawns in the window area and when it climbs out of the ground. On higher rounds more should just spawn out of the ground. Handle this how cold war does it.
- Cutouts in walls for zombies to climb through

# Map

- Add handrails to all stairs or just copy paste stairs
- Add cut outs for zombies to climb through that can be repaired with boards and carpenter
- Add fire bowls to zone above temple

# Map Done

# Added

- Add sun flare when looking near sun on day maps
- Add other zombie walk and attack animations
- Possibly add high and low pass filters to sounds if they are behind walls
- Could use exported eleven labs audio for radio then add distortion and high pass filter to it
- Add a robust testable way for updates, this should go along with using cloud storage for the assets. As soon as game loads it should show the checking for updates up. Need to look at update ui and ensure updating actually works.
- need to also check what determines connecting to prod server and dev server since when developing it probably doesn’t check the remote storage for asset or code updates
- Could add a text near the top compass that shows the name of the curent zone
- Add ability to add more power lights to a map
- On YouTube video shot marks were on 0 level over speed cola even though the ground there is lowered
- Add cynder blocks to the level editor for the mystery box to sit on. These will sit at every mystery box location.
- Mystery box animation is messed up because door slides to position it should be in normally without cinder blocks
- Look into using different footstep sounds based on surface

# Asset todo

- Add new radio quotes
- add red lights in tomb
- Add more fire pits
- Make a better blood overlay
- Could update pap camos
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

- Dial in molotov flame look
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
