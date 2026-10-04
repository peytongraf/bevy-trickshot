Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Done

- Spectate when bled out, back next round
- End game screen over the world
- Revive downed players (zombies)

# Current todo

- Perk quote volume should be higher
- Level out perk jingle sound volumes
- When walking over dropped equipment of the kind the player already has most of the time it doesn't pick it up

# Improve

- Improve molotov fire look
- Dial in phd slider effect look ( trail and explosion )

# New

- Make all maps have a day and night setting with just different fog / sky settings

# Big features

- Add raygun
- Add armor
- Add crafting table
- Add rampage inducer
- Add monkey bomb
- Add semtex
- Add frag
- Add quotes
- Add boss
- Add exfil
- Add killchains
- Add field upgrade like aether shroud

# Bugs

- On zombies if player somehow goes below a certian point or outside of the map bounds they shold be teleported back in. The spawn point shouldn't spawn the player in where they drop down below the map and have to press the teleport button.
- If a zombie is stuck all the way inside of a box it can't escape, add a way for it to be pushed out of it.
- Only one light is coming on when turning on the power

# Today

- Knife won't stab when enemies are totally point blank
- On basic map bots sometimes walk off the edge and fall ( there is no where to land they should die and respawn )
- Add tdm

# Added

Everything below is ordered easiest → hardest, within each section.

- Increase aim sway
- On windows terminal pops up to play prod client
- Night shipment is a little too dark in shaded areas and can't see remote player model that well
- Add spawn points for all maps
- Clear all client runtime errors
- Many lines on score aren't correct. For instance a long shot will be awarded when the shot isn't long or a 720 awarded when a 360 is done.
- Ensure state is fully reset when leaving a match / starting a new game, and everything works properly when joining a new match
- When leaving with party the lobby should still be together
- On throwing knife killcam, once the player throws the knife, the camera should follow the throwing knife
- Add jumpshot points. There can be an icon the player can aim at and when their aim is near it it will highlight. They can then press a keybind to teleport to that point.
- Use Large file storage, r2, etc to store assets so that a map can be used that is over 100mb. Use whatever makes the most sense and is cheap. Only a few friends will be playing occasionally in production.

## Bugs

- It doesn't show to update anywhere on the client after it is launched and a new release is out.

## Sound

- Add current player death sound. Sound should be a body fall sound mixed with a disonant synth sound.
- Add save teleport sound
- Add mantle sound
- Add footstep sounds for other players
- Possibly convert audio to wav files.
- Add pap shot sounds

## UI / HUD

- Add tab to show leaderboard
