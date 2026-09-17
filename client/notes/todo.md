# Added

- Switching to knife shouldn't play adjust animation. That animation should only play randomly when the player isn't trying to attack. If the player does attack it should smoothly transition to that animation.
- Crouch / slide doesn't work in kill cam. If I crouch and shoot a bot, the kill cam shows me stand the entire time and shoot over the bots head.
- Replay in kill cam isn't smooth. Movement of sniper is jittery like it is snapping from one position to the next very quickly
- Look into how make looking around with mouse movement slightly smoother. There is a sublte suddeness / almost jitteryness to the movement like it needs a very subltle smoothing to it or at least something visually should change very slightly.
- Make home screen updates text font easier to read. Something similar to typical mac font.
- Add a loadout section where the user can change their scope and crosshairs. This won't change the model. It will change the texture used for the crosshair and the zoom level of the scope. This will need settings to fine tune it to ensure that the scope perfectly lines up with what is seen outside of the scope.
- Ensure todo.md is worded well and at the top make sure it says for Claude not to text anything unless explicitly asked to
- Main.rs should be refactored parts at a time
- Can remove some things from the top right controls ui
- Add tabs to top right controls ui to group other tabs into
- Can possibly delete the client read me and server read me or at least update them
- Claude md can be updated with all the the things it should normally do and tell it what the point of the game is
- Wait for all clients to be done loading assets before starting game. Show picture of map while waiting. If both assets load fast still show the picture for something like 3 seconds, or not so dev can be quicker. At the bottom left show loading assets so user knows if its on them or not

# Done

## Bugs

Ordered easiest → hardest to fix.

- Player who isn't party members screen still shows end game screen with continue button even after a game starts.
- It doesn't show to update anywhere on the client after it is launched and a new release is out.
- Z says capslock won't work to set as crouch / slide keybind
- When aiming through the sniper scope on shipment, the sky looks normal in that it is bright and the fog isn't visible.
- When backing out of free for all then going to the basic map free style, players don't see each others remote model moving

## Sound

Ordered easiest → hardest to fix.

- Add save teleport sound
- Add knife sounds
- Add footstep sounds for other players

## UI / HUD

Ordered easiest → hardest to fix.

- Add ping ui beside fps
- Add tab to show leaderboard
- Add heart beat sound and red around screen when health low
- Add end game screen with play again button
- Add dot over other players head when playing trickshot mode that goes to the correct side of the screen when looking away from them

## Gameplay Features

Ordered easiest → hardest to fix.

- Add jumpshot points
- Add spawn points for maps
- Add hitmarkers ( where when shooting a bot or another player lower and from a distance it doesn't kill them and you get the hitmarker sound )
- Add different optic options.
- Add effect to scope so it looks like actually being aimed through a scope.
- Add ramped slow mo final kill of the game
- Add camera change when killed where it looks at the direction you were shot from like on cod then shows you die in third person then plays kill cam
- Add bots ( using remote player model ) that can be added to game modes

## Weapons & Models

Ordered easiest → hardest to fix. Note: a knife model was added recently (see git log) — check whether "Add knife and throwing knife models" is now partially done.

- Add knife and throwing knife models
- Add throwing knife model with throwing arms and implement throwing it and hitting enemies.
- Need to make a change so that the glb file is used for collision detection.

---

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
- Add lights inside of far containers in bevy and in blender (likely done — see recent "Add lights to shipment" commit)

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
