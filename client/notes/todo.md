Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Done

# Today

- Knife won't stab when enemies are totally point blank
- Could make shroom tea brighten map slightly like a sort of night vision
- Add lights to break point and add a way to turn on the power.
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

## UI / HUD

- Add heart beat sound and red around screen when health low
- Add tab to show leaderboard
- Add end game screen with play again button
- Add dot over other players head when playing trickshot mode that goes to the correct side of the screen when looking away from them

## Ideas

- See if it is possible for the OS key to not cause keybind behavior like cod does.
- Could add breath hold to reduce aiming idle sway then the breath sound effect with the sudden increase in sway
