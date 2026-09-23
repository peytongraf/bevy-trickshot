Note for Claude: don't send chat messages / commentary about this file or its
contents unless explicitly asked — just do the work.
Also, only do one todo at a time. I will test the changes by running the client and dev server myself before you proceed to the next todo.

- Note - P logs position to client console *

# Done

- Ascension map removed
- Blood splatter / health tint no longer draws over the HUD
- Lights added inside shipment containers

# Added

Everything below is ordered easiest → hardest, within each section.

- Turn down remote player rechamber volume by a lot.
- Increase aim sway
- On windows terminal pops up to play prod client
- Limit throwing knife and sniper ammo on free for all. The player should get 2 throwing knives and the max mags for the sniper should be 6.
- Night shipment is a little too dark in shaded areas and can't see remote player model that well
- Make other players and bots not able to be ran through
- Add current player death sound. Sound should be a body fall sound mixed with a disonant synth sound.
- Add spawn points for all maps
- Clear all client runtime errors so that logging will work for things like position
- Many lines on score aren't correct. For instance a long shot will be awarded when the shot isn't long or a 720 awarded when a 360 is done.
- Ensure state is fully reset when leaving a match / starting a new game, and everything works properly when joining a new match
- When leaving with party the lobby should still be together
- On throwing knife killcam, once the player throws the knife, the camera should follow the throwing knife
- Add effect to scope so it looks like actually being aimed through a scope.
- Add jumpshot points. There can be an icon the player can aim at and when their aim is near it it will highlight. They can then press a keybind to teleport to that point.
- Use Large file storage, r2, etc to store assets so that a map can be used that is over 100mb. Use whatever makes the most sense and is cheap. Only a few friends will be playing occasionally in production.
- Create an egui toggle that turns on the Shroom shader effect. This effect will be used in the future as a cod zombies style perk. The perk should give slight aim assist ( when aim is near a target while ads it should pull the aim to the target slightly ) and also add a glow around enemies that aren't visible meaning the player can see a glow around their body through walls. The main effect though will be adding distortions that look like a realistic mushroom trip. The point is for the player to drink this mushroom perk that aids them and for there to be a cool visual distortion that occurs so that it has a side effect unlike typical cod zombie perks. Also the saturation should be increased. Possibly add some other effects that would be similar to a mushroom trip.

## Bugs

- It doesn't show to update anywhere on the client after it is launched and a new release is out.

## Sound

- Add equip sniper sound ( equip ak74 in old game )
- Add save teleport sound
- Add mantle sound
- Add knife sounds
- Add footstep sounds for other players
- Organize audio directory, stick to naming convention, and possibly convert audio to wav files.

## UI / HUD

- Add ping ui beside fps
- Add throwing knife to ammo hud And update that ui to look like cod
- Add heart beat sound and red around screen when health low
- Add tab to show leaderboard
- Add end game screen with play again button
- Add dot over other players head when playing trickshot mode that goes to the correct side of the screen when looking away from them
- Update main menu / current lobby menu / loadout / in game settings menu ui to look like a call of duty menu instead of a basic indy looking game menu

## Ideas

- See if it is possible for the OS key to not cause keybind behavior like cod does.
- Could add breath hold to reduce aiming idle sway then the breath sound effect with the sudden increase in sway
