Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Done

- Forgiving Ray Gun hits
- No bullet holes on placed objects, smaller holes
- Bullet holes stay off edges
- Smaller, centered "power first" popup
- Dog speed retune

# Current todo

# Added

- When knifing zombies and a power up pops up the player instantly activates it even if they don’t want to. See how cod handles power up spawn position on zombie kill / knife kill
- On YouTube video shot marks were on 0 level over speed cola even though the ground there is lowered
- Add cynder blocks to the level editor for the mystery box to sit on. These will sit at every mystery box location.
- mystery box animation is messed up because door slides to position it should be in normally without cinder blocks
- add ammo crates to each zone
- 2 times going up to juggernog has caused damage with no zombie nearby
- Add debris fall when a door opens
- add penetration to shots so that a round can go through one zombie and still damage another but not as much. At some point it shouldn’t be able to go through any more zombies
- When zombies are far away from all players and no lobby member can see them, they should respawn closer to a player
- look into using different footstep sounds based on surface
- Look into using reverb on sounds based on location like in the echoey tomb

# Asset todo

- Add new radio quotes
- railing in temple at the top of the stairs is too high
- add red lights in tomb
- Add more fire pits
- Add wall to room the exposed part of the pyramid since enemies keep getting caught on it
- Add walls alll the way around so a player can’t fall out
- Make a better blood overlay
- could update pap camos
- Make hd staminup model
- Could use the part above the temple that isn’t used
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
- Only one light is coming on when turning on the power
- It doesn't show to update anywhere on the client after it is launched and a new release is out.

# Improve

- Improve molotov fire look
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
