Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Done

- Minimap top left
- Operator quotes for game events
- Louder quotes (perk quotes too)
- Rounds end with only bosses left
- Leave with party keeps lobby
- Boss smash knockback
- Stronger hurt effect + damage indicator
- Tab leaderboard
- Realistic perk drink bottles
- Thrown knives only scratch bosses
- Fixed equipment pickups failing
- Exfil on rounds 11, 21, 31...
- No random power-ups on dog rounds
- Smaller perk icons
- Fixed zombie spawn flash at map centre

# Added

- Wunderfizz ui should appear centered in the screen instead of to the left
-

# Current todo

# Performance

- when using phd slider a lot on a higher round with a lot of zombies, fps dropped to around 70-90
- add toggle to show fps, ping, 1 percent low, etc
- if possible could add draw calls or other ways to detect how graphics performance is doing
- Could run btop while game is running
- Could possibly keep a client side graph of entities, effects, fps over time etc

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

# New

- Exfil radio model size should be increased by about 1.3 times
- Some objects shouldn't get bullet holes when shot. For instance they don't look right on some perk machines.

# Big features

- Add full map shown while holding a keybind (uses the minimap's picture)
- Add semtex
- Add rampage inducer
- Add killchains

## Sound

- Level out perk jingle sound volumes
- Add current player death sound. Sound should be a body fall sound mixed with a disonant synth sound.
- Add save teleport sound
- Add mantle sound
- Add footstep sounds for other players
- Possibly convert audio to wav files.
- Add pap shot sounds
- When player is downed a breathing sound should play
