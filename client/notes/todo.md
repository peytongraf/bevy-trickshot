## Fix

- When aiming through the sniper scope on shipment, the sky looks normal in that it is bright and the fog isn't visible.
- For a split second when the barrel smoke sprite spawns it is full opacity and not facing the player regardless of the settings.
- The water is rendering over the smoke on shipment.
- The shipment main lights should be max brightness.

## Sound

- Add knife sounds
- Add footstep sounds for other players
- Add save teleport sound

## Added

-
- Add spawn points for maps
- Add knife and throwing knife models
- Add tab to show leaderboard
- Update readme
- Add ping ui beside fps
- Add bots ( using remote player model ) that can be added to game modes
- Add camera change when killed where it looks at the direction you were shot from like on cod then shows you die in third person then plays kill cam
- Add hitmarkers
- Add heart beat sound and red around screen when health low
- Add dot over other players head when playing trickshot mode that goes to the correct side of the screen when looking away from them
- Add lights inside of far containers in bevy and in blender

---

- Add jumpshot points
- Cursor not showing at end of game screen
- Player who isn't party members screen still shows end game screen with continue button even after a game starts.
- Add different optic options.
- Add end game screen with play again button
- Add ramped slow mo final kill of the game
- Killcam not playing footstep sounds.
- Add effect to scope so it looks like actually being aimed through a scope.
- Add throwing knife model with throwing arms and implement throwing it and hitting enemies.
- Need to make a change so that the glb file is used for collision detection.

# Done

## Security

- Ensure that public github repo can't let random people from using the production server and run up the cost.
- Add security features to production server like rate limiting and max concurrent user count.

## Performance

- Check shader preloading like cod

## Refactor

- main file is thousands of lines so it should be refactored.

## Ideas

- Can add bullet impacts once map is created.
- Add grappling hook or teleportation where the player can aim on a teleport point that will highlight when aimed on and a keybind can teleport the player to it.
- Could add breath hold to reduce aiming idle sway then the breath sound effect with the sudden increase in sway
- See if it is possible for the OS key to not cause keybind behavior like cod does.

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
