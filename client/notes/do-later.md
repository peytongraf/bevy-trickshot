## Do Later

- Update to bevy 0.19 from 0.16
- Download high quality arms model along with a sniper and knife. Create own custom animations with it.
- Can remove some things from the top right controls ui (skipped for now — asked which sections to trim, told to come back to it later)
- Add tabs to top right controls ui to group other tabs into
- Replay in kill cam isn't smooth. Movement of sniper is jittery like it is snapping from one position to the next very quickly
- Night shipment is a little too dark in shaded areas and can't see remote player model that well
- Remote player model transitions to new position when they dead and respawning instead of having their body stay there then disappear and a new model appear
- On shipment some bullet impacts don't show. This is probably because the asset like the container is slightly larger than it should be so the impact is shown behind the surface.
- Add effect to scope so it looks like actually being aimed through a scope.
- On windows terminal pops up to play prod client
- Use Large file storage, r2, etc to store assets so that a map can be used that is over 100mb. Use whatever makes the most sense and is cheap. Only a few friends will be playing occasionally in production.
- Add tdm
- Look into "2026-10-04T22:28:47.509421Z WARN bevy_gltf::loader: Unknown vertex attribute TEXCOORD_3" client log on startup
- Clear all client runtime errors
