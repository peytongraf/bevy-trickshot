- Killcam not playing footstep sounds.
- See if it is possible for the OS key to not cause keybind behavior like cod does.

- Add effect to scope so it looks like actually being aimed through a scope.
- Add throwing knife model with throwing arms and implement throwing it and hitting enemies.
- Need to make a change so that the glb file is used for collision detection.

# Done

- Teleport point should save direction the player is facing / looking.
- Enable auto reload, check if rechamber animation should happen after last round is fired of a mag and if it should happen after a new mag is reloaded.
- Add idle sway of sniper
- Add aiming idle sway
- Since the players capsule is visible in the shadow, the capsule should shrink when crouching and lay down sideways when prone.
- Add max shot distance for sniper.
- Add colat plus colat detection. Bullet should slow when hitting a target.

## Security

- Ensure that public github repo can't let random people from using the production server and run up the cost.
- Add security features to production server like rate limiting and max concurrent user count.

## Performance

- Check shader preloading like cod

## Sound

- Add ui button sounds
- Add footstep sounds for other players
- Add teleport sound and save teleport sound

## Refactor

- main file is thousands of lines so it should be refactored.

## Fix

- Fix smoke sprite rotation issue where it doesn't always face player as they look left or right.

## Ideas

- Can add bullet impacts once map is created.
- Add grappling hook or teleportation where the player can aim on a teleport point that will highlight when aimed on and a keybind can teleport the player to it.
- Could add breath hold to reduce aiming idle sway then the breath sound effect with the sudden increase in sway

## Scoring Points

- Could add sounds for the highest score line per shotso headshot could have a sound, no scope could have a sound, etc. It would play the sound that has the highest score since there can be multiple at a time. Sounds can be things like a mario type level up or coin sound, a chaching sound, a taco bell bell toll sound, etc.

+100 180, 360, etc ( right or left )
+100 no scope
+100 collat ( multiply by number of extra bots hit )
+100 silent shot ( need to add throwing knife )
+100 midair weapon swap ( need to add knife )
+100 throwing knife ( could be a multiplier for the entire set of points where sniper shot is double or some other amount)
+100 headshot
+100 wallbang
