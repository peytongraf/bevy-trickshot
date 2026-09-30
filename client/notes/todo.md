Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Done

# New

- Nuke should add 500 bonues points
- Ammo box is very inconsistant. Sometimes ui won't pop up and sometimes even after purchasing ammo isn't actually increased. See if this has anything to do with pap level. Even after max ammo was activated, no ammo was added and the sniper was still empty.
- Check if pausing the game has anything to do with these issues
- When throwing knives or molotov is the active lethal type and running over that dropped equipment, it doesn't automatically pick it up
- Add extra points for critical kill
- Add go prone in front of a perk machine to get 100 free points
- Ammo box isn't working

- On break point night zombies, regular bird ambient sound should be removed and only zombies ambient sound should play
- Use melee animation for remote player when using knife
- Improve molotov fire look
- Add quotes
- Add health bars to zombies
- Separate todo into regular multiplayer and zombies todos
- Add molotov to multiplayer loadout
- Only one light is coming on when turning on the power
- Improve end game screen
- Add music volume setting under audio
- Zombie max speed should be faster and there should be more total zombies on higher rounds
- Zombies base speed should be faster
- Add dogs or dog style rounds with a different enemy
- Add boss
- Add exfil
- Add revive other players
- Add killchains
- Add field upgrade like aether shroud

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

## UI / HUD

- Add tab to show leaderboard
