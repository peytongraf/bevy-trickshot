# Der Tempel Easter Egg — todo

What's left to do or fix. The story and order of events are in
`story.md`. How each part works is in the module docs at the top of its
code (`shared/src/easter_egg.rs`, `client/src/easter_egg.rs` and
`client/src/easter_egg/*.rs`, `server/src/easter_egg.rs` and
`server/src/easter_egg/*.rs`, `*/src/exhumer.rs`,
`client/src/environment/egg_mood.rs`). The whole egg plays start to finish
on placeholders. **[you]** marks asset work; each asset drops in where its
placeholder is, and some need a small hook-up from me afterwards (noted
on each).

# Bugs / fix

- When first firing the beam, it shouldn't just appear at full length but instead move outward from the gun over time to its full length.
- Blood on the ground should look real
- Add hand cutting animation for current and remote players with blood effect
- Should text on screen have a quote that is read aloud?
- Soul of boss zombie should look more realistic (the Exhumer's souls are smoke now — maybe match them)
- particle animation that is shown after top is blown of sarcophagus should look more realistic
- vial and dagger pickup sounds should play the normal dropped eqiupment pickup sounds
- Diamond above in pyramid altar table should be much lower
- active zombies who aren't dead when ritual starts should have speed greatly reduced and should stop further away from players instead of getting right up on them.
- Need a scary sound that plays when these zombies are killed due to the ritual. Could also add a smoke effect on the zombies like magic is killing them.
- Maybe make the player face the main altar during the ritual where the blood is poured and also face the zombie while the initial quotes are being player and it awakes but still keep the player from being able to do anything until the boss zombie is active. Maybe even add in black bars on the top and bottom of the screen so it is clear it is a cienamatic scene almost like a cut scene.
- When astral projecting up to inside the pyramid, player can just get the weapon before the final zombei quote plays. Maybe this quote should play as the player is astral projecting.
- Add some sort of extra shader effect when in the astral mode getting the wonder weapon.
- For the astral effect, add in realistic dmt / astral projection humming / buzzing sounds with a binural beat effect also where the left and right sounds are actually different.
- Add some sort of glow / light around the wonder weapon when inside the pyramid
- Probably would be better to cause some sort of damage with the wonder weapon with the zombies staying on the ground while they are being sucked then once complete, only then have them smoothly animate upwards and souls removed then killed. (Right now they float up steadily the whole time they're drained — decide which you want.)
- Other players' souls float up to the pyramid as their normal soldier
  models. Give them (and players in the chamber) a ghostly look.
- Sarcophagus needs a look: its solid box is still 1.6 × 1 × 3.2 m, so
  the lid (to 1.1 m) pokes 0.1 m above it until it's blown off.

# Test

- A full run start to finish with 2 players, then a new game to check
  that nothing carries over.

# To do

## Gathering

1. **[you]** Model the vial and record its pickup sound.
2. **[you]** Model the society case (shut and open) and record the
   case-unlock sound. **Me after:** swing its lid open with the power
   (right now the glass just disappears) and match the dagger's height
   to the velvet bed (now 0.812 m).

## The ritual

3. **[you]** Animate the ritual in first person (dagger into the altar,
   pouring the vial, cutting the hand). Record the chant line and the
   ritual music or sound bed. The ritual runs on placeholders with a
   caption for each phase. Its timing is `RITUAL_PHASES` in
   `client/src/easter_egg.rs` (15 s total, `shared::easter_egg::RITUAL_SECS`),
   so time the animations to it or tell me the new timings. A close-up
   (the hand cut) can use the 4k dagger in
   `client/model_src/easter_egg/hierophant_dagger_4k.glb`.
4. **[you]** (Optional) Re-record the two sarcophagus lines more aggressive
   (`client/notes/quotes/hierophant-quotes.md`). The current ones are used
   either way. A new length means changing `RITUAL_VOICE_SECS` /
   `UNLEASHED_VOICE_LEN_SECS` in `shared::easter_egg`.

## The boss

5. **[you]** Model, rig and animate the Hierophant: lying in the
   sarcophagus, waking, walking, attacking, teleporting, raising zombies
   and dying. Give him his own name, different from the regular boss.
   He's `boss.glb` with a green aura for now.
   **Me after:** his own avatar and clips instead of
   `boss::spawn_boss_avatar` (and probably his own `ZombieAnim` states:
   taunt, teleport, summon), his own death clip, a lying-down pose
   (re-check `lying_at`, panel → "He wakes"), and his eyes re-tuned
   (panel → "The fight").
   Attack ideas once he has animations: an overhead slam with a ground
   shockwave, a dagger lunge, a cursed orb at anyone keeping their
   distance (phase 1); floor sigils that erupt under players (phase 2); a
   life-drain grab (phase 3).
6. **[you]** Record over the placeholder sounds:
   `audio/zombies/easter_egg/awakening/` `lid_blast.mp3`, `spirit.mp3`
   (≤ 3 s, it's cut at the stillness) and `wake_hit.mp3`, and
   `hierophant/spirit_flee.mp3` (his spirit flying into the pyramid).
7. **[you]** Record the operators' easter egg quotes
   (`client/notes/quotes/operator_quotes/{adam,von}/…-easter-egg-quotes.md`).
   **Me after:** wire them up, timed so they never talk over his lines or
   the radio.

## The pyramid

8. **[you]** Model the pyramid's inner chamber, and record over
   `audio/zombies/easter_egg/pyramid/float_up.mp3` and
   `chamber_ambience.mp3` (placeholders).
9. **[you]** Model and animate the Exhumer (it uses the Ray Gun until
   then) and record over its placeholder sounds in
   `audio/zombies/exhumer/` (`beam.mp3`, `lift.mp3`, `soul_tear.mp3`,
   `release.mp3`; letting souls out also plays the Ray Gun's shot for
   now), plus a pickup sound. Make its HUD icon.

## Models needed

When you make one: put its origin on the ground under its middle, face
its front along +Z (Blender's −Y, the side Front view shows), use metres,
and apply scale (Ctrl+A → Scale) before UVs. If it comes out a different
size, the level editor's Scale field fixes that.

- **Society case** (0.8 m wide, 1.3 m tall, 0.5 m deep at most): a
  modern, expensive museum case on a dark wood stand, with a glass lid
  hinged at the back and a brass keypad / magnetic lock with a red → green
  light. It looks out of place in the ruins. Separate objects: body,
  **lid** (origin on its hinge line), lock, and the light (its own
  material). Leave the dagger out; it's its own model.
- **Barrett's vial** (about 12 cm): a sealed lab vial of glowing dark-red
  blood, with a metal cap and a printed society tag reading "Vessel
  candidate 07". Clinical and modern, unlike the dagger.
- **Pyramid inner chamber**: a dark room inside the pyramid's top, with
  sloped walls that have light coming through the apex. Big enough for
  the whole squad to stand around the altar. It can be part of the map
  `.glb` (hide its walls in the level editor with V to place things
  inside).
- **The Hierophant (boss)**: a robed, mummified society founder, gaunt,
  with long fingers, ritual scars and the sigil branded on his chest.
  Glowing eyes once he's awake. Needs to be rigged.
- **The Exhumer (wonder weapon)**: the relic the first rite was done
  with, an ancient soul-pulling instrument later fitted with the
  society's hardware (see `story.md`). Built like a view model gun, the
  Ray Gun's size (about 45 cm). Its camo parts wear
  `textures/camos/exhumer.webp`.

## Re-exporting the pyramid altar

Its UVs were fixed inside the `.glb`, not in Blender. Re-exporting from
Blender brings the stretched marble back unless you fix them there too:
Ctrl+A → Scale on each piece, then in Edit Mode select all → U → Cube
Projection, Cube Size 1.0.
