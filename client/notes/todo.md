# Current todo

- Add full power power up
- Add fire sale power up
- Can add smoke efffect to boards on windows as the rebuild animation is playing
- Possibly add sound loading to things the loading ui has to wait on to finish. If this and other loading can be done in the background and can just make the game wait ( when it shows waiting on players in the lobby to load ) when starting a game.
- Change settings for when a zombie spawns in the window area and when it climbs out of the ground. On higher rounds more should just spawn out of the ground. Handle this how cold war does it.
- Cutouts in walls for zombies to climb through

# Latest play

- Weapon / arms fps model looks very dark in low light when it seemingly should be lit up more
- Radio quote volume should be about 1.25 times as loud
- First dog round the 3rd to last dog dropped a nuke, 2nd to last dropped a max ammo, then last dropped a max ammo. Only the last dog should drop a power up which should always be the max ammo and should always drop.
- Got frags from the mystery box after having throwing knives and frag ammo was already at 4 instead of 1.
- Alter in lower temple that pap is on should be about 1/4 as tall.
- Maybe boss should spawn later because couldn't get enough perks or even pap weapon before he showed up.
- Blood overlay is too strong. 1/3 damage looks like near death.
- Operator quotes repeat too much. The same quote shouldn't be able to play again for a certain amount of time like 3-5 minutes.
- On round 15 dogs all spawned quick but the last 3-4 took a while each to spawn after the last died so I just sat there waiting for the next one multiple times.
- Exfil line should wait a few seconds since its after a dog round and the max ammo line is usually being said. Maybe make the exfil line wait if the max ammo or another power up line is being said?
- The fps is consistently a lot lower when there are more enemies at one time.
- Exfil is too hard. Enemies should only have their speed increased barely and the speed shouldn't go past the max speed. Maybe spawn in slightly less enemies also. Still have about half as many dogs. The dogs shouldn't all spawn right at the start all together but should be spread out because they make it very hard to survive the exfil.

# FPS lowered to around 50 when this was constantly logged

2026-10-22T03:15:30.876091Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.876094Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.892613Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.892619Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.920066Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.920077Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.926934Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.926941Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.943401Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.943408Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.943447Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.943449Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.943474Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.943476Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.959542Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=bext, len=602.
2026-10-22T03:15:30.959549Z INFO symphonia_format_riff::common: ignoring unknown chunk: tag=junk, len=28.
2026-10-22T03:15:30.975862Z INFO symphonia_bundle_mp3::demuxer: estimating duration from bitrate, may be inaccurate for vbr files

# Done

- Search and Destroy mode
- Wired up new radio quotes

# First person animations

# Remote player animations

- Raise right arm some on drink perk remote player animation so that cap is at mouth level
- Sniper that is used should still have the lenses. If the sniper model has these removed then instead use the model that doesn't have them removed and have bevy remove them just when in fps mode.
- Sniper and maybe other guns should not have the pap camo on some objects.
- Could have reload animation when moving because currently it is only with the player not moving so their feet slide if they move when reloading.
- Possibly see how hard it is to change the remote player model but keep bones / animations

# Added

- Add a robust testable way for updates, this should go along with using cloud storage for the assets. As soon as game loads it should show the checking for updates up. Need to look at update ui and ensure updating actually works.
- Need to also check what determines connecting to prod server and dev server since when developing it probably doesn’t check the remote storage for asset or code updates
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

- When using phd slider a lot on a higher round with a lot of zombies, fps dropped to around 70-90. This seems to just be the case after going through this many rounds instead of starting on a high round from the start.
- Could run btop while game is running
- Could possibly keep a client side graph of entities, effects, fps over time etc

# Flashlight

- Can add a flash light on off sound
- Can turn on flashlight if an area is too dark instead of having it only on when the power is off

# Bugs

- On zombies if player somehow goes below a certian point or outside of the map bounds they shold be teleported back in. The spawn point shouldn't spawn the player in where they drop down below the map and have to press the teleport button.
- If a zombie is stuck all the way inside of a box it can't escape, add a way for it to be pushed out of it.
- It doesn't show to update anywhere on the client after it is launched and a new release is out.

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
- Possibly convert audio to wav files.
- Add pap shot sounds
- When player is downed a breathing sound should play
- Look into using different footstep sounds based on surface
