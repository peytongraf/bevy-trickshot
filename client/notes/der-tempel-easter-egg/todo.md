# Der Tempel Easter Egg — todo

The story and order of events are in `story.md`. Work top to bottom, one
item at a time. Each code step uses placeholder models and sounds, and has
a **Test** line so you can check it before moving on. **[you]** marks the
asset work that's yours; until it's done, that step runs on placeholders.

## Next session: start here

Where things stand (Oct 31): steps through the ritual are built and
tested. Nothing is committed to git yet. Next up is the code for **"It goes
wrong"** (step 5 below). Do this first:

- **Wire in the two Hierophant lines that are already in the game:**
  - `audio/zombies/easter_egg/hierophant/during_ritual.mp3` plays during
    the ritual, around its last phase (`RITUAL_PHASES` in
    `client/src/easter_egg.rs`, at 13 s; it's 5.6 s long), coming from
    the sarcophagus.
  - `ritual_gone_wrong.mp3` (8.4 s) plays in step 5, from the
    sarcophagus, along with the radio line `RadioLine::EggRitualGoesWrong`
    (recorded). Have the radio wait until the Hierophant is done.
  - Peyton may re-record both more aggressive (`hierophant-quotes.md`);
    record over the same file names.
- **The "Hierophant" mix profile** (`client/assets/audio/mix.ron`, folder
  `zombies/easter_egg/hierophant`) fades out by 16 m, which is right for
  the whispers. His fight lines will need longer reach: give them their
  own subfolder or profile.
- **The pyramid altar isn't placed** in `shared/levels/der_tempel.ron`
  yet (`pyramid_altar: None`). It's needed by step 16. The pyramid spots
  are placed.
- **Quote scripts** (ElevenLabs `[ ]` direction) are in
  `client/notes/quotes/`: `hierophant-quotes.md`, `radio-quotes.md`
  ("Easter egg", all recorded), and
  `operator_quotes/{adam,von}/{adam,von}-easter-egg-quotes.md` (with the
  folder each recording goes in). None of the operator lines are wired up
  yet.
- **No changelog line yet** on purpose (it would spoil the easter egg).
  Add one at the end (the last step).

Where the code is:

- `shared/src/easter_egg.rs`: the steps (`EggStep`), progress
  (`EasterEgg`, on `Lobby::easter_egg`), reach checks, `RITUAL_SECS`,
  and the pieces' sizes.
- `server/src/easter_egg.rs`: the vial drop, interact handling, the altar,
  the ritual clock and the debug `SetEasterEgg`.
- `client/src/easter_egg.rs`: placeholders, vial, prompts, HUD, ritual
  visuals and captions, whispers, and the debug panel section ("Easter
  egg (Der Tempel)").
- Layout fields `sarcophagus`, `society_case`, `ritual_area`,
  `pyramid_altar` and `pyramid_spots` are in `shared/src/level.rs`. The
  editor objects are `ObjectId::{Sarcophagus, SocietyCase, RitualArea,
  PyramidAltar, PyramidSpot}` in `client/src/level_editor/`.
- The ritual locks input through `EggRitual` in `menu::game_active` /
  `weapon_active`, and zombies hold fire through `Lobby::bots_hold_fire`.

## To do

### Gathering

1. **[you]** Model the vial and record its pickup sound.
2. **[you]** Model the society case (shut and open) and the dagger, and
   record the case-unlock sound.

### The ritual

3. **[you]** Animate the ritual in first person (dagger into the altar,
   pouring the vial, cutting the hand). Record the chant line and the
   ritual music or sound bed. The ritual already runs on placeholders,
   with a caption for each phase. Its timing is `RITUAL_PHASES` in
   `client/src/easter_egg.rs` (15 s total, `shared::easter_egg::RITUAL_SECS`),
   so time the animations to it or tell me the new timings.
4. **[you]** (Optional) Re-record the two sarcophagus lines more aggressive
   (`client/notes/quotes/hierophant-quotes.md`). The current recordings are
   in the game and will be used either way. (The "read it backwards"
   radio line is recorded too: `RadioLine::EggRitualGoesWrong`.)
5. **It goes wrong:** the fires turn green, the water turns red, the sky
   darkens and the fog thickens, the lights flicker, every zombie drops
   dead, the sarcophagus speaks, and the radio line plays.
   **Test:** use the debug button to jump to this step; everything changes
   together, and it all goes back to normal on reset or in a new game.

### The boss

6. **[you]** Model, rig and animate the Hierophant: lying in the
   sarcophagus, waking, walking, attacking, teleporting, raising zombies
   and dying. Model the sarcophagus and its lid.
7. **He wakes:** the lid blasts off, his spirit pours into the body, a
   beat of stillness, then he snaps up with a scream, a hard audio hit and
   a camera shake.
   **Test:** use the debug button to jump to the wake-up and check the
   timing; it should make you jump.
8. **[you]** Record the boss voice lines (waking, taunts, phase 2, phase
   3, downing a player, hurt, death, the pyramid scream:
   `client/notes/quotes/hierophant-quotes.md`), the operators' reactions
   (`operator_quotes/*/…-easter-egg-quotes.md`), and the boss fight
   music.
9. **Boss fight phase 1:** a heavy melee stalker with taunts, his own
   health bar, and normal zombie spawns stopped during the fight.
   **Test:** use the debug panel to spawn him; he chases, hits and taunts,
   and can be killed.
10. **Phase 2 (66%):** he raises a ring of zombies and teleports between
    the fire bowls.
    **Test:** use a debug button to set his health; check the zombie ring
    and the teleports.
11. **Phase 3 (33%):** he's enraged and faster, in blood-red fog with
    glowing eyes.
    **Test:** use a debug button to set his health; check that he's faster
    and the fog comes in.
12. **His death:** the body collapses, his spirit flies screaming into the
    pyramid, and the environment goes back to normal.
    **Test:** kill him; everything resets to how it looked before the
    ritual.

### The pyramid

13. **[you]** Model the pyramid's inner chamber and its altar, and record
    the sound for floating up and the chamber's ambience.
14. **Astral projection:** the players' bodies slump at the altar, their
    souls float up through the pyramid's apex into the chamber, and the
    zombies pause.
    **Test:** use the debug button to jump to after the boss's death; float
    up, look around, and check that nothing can hurt the bodies left below.
15. **[you]** Model and animate the wonder weapon and record its sounds:
    firing, reloading and pickup. Decide what it does, plus its magazine
    sizes for each Pack-a-Punch level.
16. **The wonder weapon:** one for each player, taken from the chamber
    altar, then a white flash and they wake at the altar holding it.
    **Test:** take it; you're back below with the gun, and the round
    carries on.
17. **The wonder weapon's behaviour + Pack-a-Punch upgrades.**
    **Test:** shoot zombies at each Pack-a-Punch level.
18. **Finish:** the final radio line (`RadioLine::EggAfterPyramid`, recorded) plays, the egg can't be done twice in
    one game, and everything resets.
    **Test:** a full run start to finish, solo and with 2 players, then a
    new game to check that nothing carries over.
19. **Changelog line** (keep it vague so it doesn't spoil the easter egg).

## Models needed

Each one is a placeholder of plain boxes for now, at the size below. When
you make the real one: put its origin on the ground under its middle, face
its front along +Z, and use metres, so it drops in where the placeholder
is. If it comes out a different size, the level editor's Scale field
fixes that.

- **Sarcophagus** (2.2 m long, 0.9 m wide, 1 m tall, long side along Z):
  old carved stone, ancient Egyptian / occult. Gold trim, worn
  hieroglyphs, and the society's sigil scratched in more recently. Make
  the **lid a separate object** so it can blast off. Hollow inside, with
  the body lying in it.
- **Society case** (0.8 m wide, 1.3 m tall, 0.5 m deep): a modern,
  expensive display case on a dark wood stand, with a glass top and a
  brass keypad or lock. It looks out of place in the ruins. Make the
  **glass lid / door a separate object** so it can open. Leave the dagger
  out; it's its own model.
- **Hierophant's dagger** (about 35 cm long): ceremonial, with a curved
  black blade, a bone or gold handle, and the society's sigil on the
  pommel. Needs a first-person version for the ritual animation.
- **Barrett's vial** (about 12 cm): a sealed lab vial of glowing dark-red
  blood, with a metal cap and a printed society tag reading "Vessel
  candidate 07". Clinical and modern, unlike the dagger.
- **Pyramid altar** (1.8 m wide, 1 m tall, 1 m deep): older and rougher
  than the tomb's, black stone, blood-stained, with candles and a slot or
  stand on top for the wonder weapon.
- **Pyramid inner chamber**: a dark room inside the pyramid's top, with
  sloped walls that have light coming through the apex. Big enough for
  the whole squad to stand around the altar. It can be part of the map
  `.glb` (hide its walls in the level editor with V to place things
  inside).
- **The Hierophant (boss)**: a robed, mummified society founder, gaunt,
  with long fingers, ritual scars and the sigil branded on his chest.
  Glowing eyes once he's awake. Needs to be rigged. The animations are
  listed in his step below.
- **Wonder weapon**: made from his relic (staff, dagger or skull — still
  to be decided).

## Done

- Whispers twice as loud
- Hierophant + operator easter egg quote scripts
- Ritual sequence (placeholder visuals + captions)
- Whispers recorded, gap halved
- Sarcophagus whispers (placeholder audio)
- Gathering radio lines wired up
- Vial drop from Big Steve + pickup
- Case opens with power, take the dagger
- Gathered items on the HUD
- Altar zone + accept prompt
- Easter egg progress + debug panel section
- Editor objects + placeholder models
