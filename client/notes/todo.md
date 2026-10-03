Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Done

- Launch loading screen

# Current todo

- Mouse back button should work to go back on any menu like pressing the ui back button ( add this to cluade.md for future features that may require a ui back button )

# New

- Make normal zombie points added text white unless it is a critical in which case it should be yellow
- Add point text in white that randomly floats away from the damaged enemy on zombies based on how much damage it took on that hit.
- Improve molotov fire look
- Dial in phd slider effect look ( trail and explosion )
- Make all maps have a day and night setting with just different fog / sky settings
- Ensure that walk and sprint bob for sniper, knife, and throwing knife are the same as the ak74. ALl should be the same as it. They should all have the same general settings instead of individual so that setting should be removed for ak74 and made general.
- On break point night zombies, regular bird ambient sound should be removed and only zombies ambient sound should play
- Improve end game screen

# Big features

- Add quotes
- Add dogs or dog style rounds with a different enemy
- Add boss
- Add exfil
- Add revive other players
- Add killchains
- Add field upgrade like aether shroud

# Bugs

- On zombies if player somehow goes below a certian point or outside of the map bounds they shold be teleported back in. The spawn point shouldn't spawn the player in where they drop down below the map and have to press the teleport button.
- If a zombie is stuck all the way inside of a box it can't escape, add a way for it to be pushed out of it.
- Only one light is coming on when turning on the power

# Today

- Knife won't stab when enemies are totally point blank
- On zombies could add a boss that can only take damage from trick shots
- On basic map bots sometimes walk off the edge and fall ( there is no where to land they should die and respawn )
- Add tdm

# Added

Everything below is ordered easiest → hardest, within each section.

- Increase aim sway
- On windows terminal pops up to play prod client
- Night shipment is a little too dark in shaded areas and can't see remote player model that well
- Add spawn points for all maps
- Clear all client runtime errors so that logging will work for things like position
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
