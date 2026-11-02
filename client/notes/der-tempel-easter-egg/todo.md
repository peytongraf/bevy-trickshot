# Der Tempel Easter Egg — todo

The story and order of events are in `story.md`. Work top to bottom, one
item at a time. Each code step uses placeholder models and sounds, and has
a **Test** line so you can check it before moving on. **[you]** marks the
asset work that's yours; until it's done, that step runs on placeholders.

## Next session: start here

Where things stand (Nov 1): steps through **his death** (step 10) are
built. "It goes wrong", "He wakes", phases 1–3 and the music are tested.
The wake-up's body standing up as the lid flew off is fixed (a seek to
the death clip's very end wrapped round to its first frame). His death
needs testing: in the debug panel, "The fight" → "Kill him" (or jump to
"His death"). Next up is **the astral projection** (step 12 below; step
11 is yours).

- **His death as built** (`client/src/easter_egg/death.rs`, built Nov 1,
  needs testing):
  - Killed, the step goes to the new `EggStep::HisDeath` (between
    BossFight and Ascension), with where he fell on
    `Hierophant::fell_at`. `holds_spawns` counts it, so the round stays
    held and the map stays gone wrong. His health bar, eyes, the fight
    music and phase 3's red all go as he falls.
  - His body falls with the boss's own death (clip, aura fading).
    `death.mp3` plays from where he falls (`DeathCry`).
  - At 1.3 s (`DEATH_SPIRIT_AT`) his spirit tears out of the body: a
    green glow with a tail of wisps and its own light, the lights
    stuttering. It rises and hangs over him, writhing, and his death
    cry goes with it.
  - At 2.9 s it flies in an arc for 2.3 s, up into the pyramid at its
    altar (`pyramid_altar`, + 1.5 m), with `spirit_flee.mp3`. That's a
    placeholder copied from `awakening/spirit.mp3`, so record over it.
    It goes in with a flash and another stutter.
  - At 6.8 s (`DEATH_SECS`, after his scream) the server moves on to
    Ascension. The map fades back to normal, the ambience comes back
    and the round carries on. Step 12 will take over from Ascension.
  - Tune it in the panel under "His death". When his real model comes,
    he needs his own death clip.

- **Phase 3 as built** (built Nov 1, needs testing):
  - At 1/3 health (`PHASE_3_AT`, `shared::easter_egg::phase_at`) he
    stops and roars for 3 s with `phase_3.mp3`. Skipping straight from
    phase 1 (a debug set) also raises phase 2's ring.
  - Enraged: he walks at `HIEROPHANT_ENRAGED_SPEED` (2.6 vs 1.95,
    `BotBrain::enrage`) and teleports every 8–12 s
    (`TELEPORT_GAP_ENRAGED_SECS`). Rings carry on as in phase 2.
  - The map goes blood-red on top of "it goes wrong": fog, sky, ambient,
    the ground fog and the fires (`EggRageFade`; the `rage_*` fields of
    `EggMoodLook`, panel → "It goes wrong — the map" → "Enraged", with
    its own preview). Everything that read `EggMoodLook` now reads
    `EggMoodNow`, the look with the red blended in.
  - He fades from green to red with it: his aura, body glow, light and
    teleport / raise bursts (`rage_aura_color`).
  - His eyes now glow the whole fight (`HisEye` in
    `client/src/easter_egg/fight.rs`, placed off his head bone), green,
    then red and brighter enraged. Their spot is a guess for `boss.glb`:
    tune "eyes forward / up / apart" under "The fight" (and again once
    his real model's in).
  - Each new phase (2 and 3) sets off the power-on earthquake
    (`earthquake::Quake::rumble`), silent by default because its sound
    has the power-on music in it. There's a checkbox to turn the sound on.

- **His body in the sarcophagus** (built Nov 1, needs a look): the
  box-figure placeholder is gone. The boss model lies on its back in the
  sarcophagus, posed on the last frame of its `death` clip. It rises by
  playing that clip backwards, then stands in `idle` (`wake.rs`,
  `pose_body`). The sarcophagus placeholder grew to 1.6 × 1 × 3.2 m and is
  hollow (`SARCOPHAGUS_HALF_EXTENTS`), so the body fits. When his real
  model comes, it needs a lying-down pose or clip; also re-check
  `lying_at` ("He wakes" in the panel). The new sarcophagus model should
  keep roughly that inside size.
- **His voice effect** (built Nov 1, needs a listen): his spoken lines
  are recorded clean into `client/voice_src/hierophant/`.
  `client/tools/voice_layers.sh` bakes in ±3 semitone layers and writes
  them to `assets/audio/zombies/easter_egg/hierophant/voice/`. That folder
  is marked `voice_fx` in mix.ron, so the engine sends it through its own
  echo + reverb (`client/src/audio_engine/voice_fx.rs`, debug panel →
  Audio → Voice effect). The whispers and sound effects stay in
  `hierophant/` without it. Nothing muffles the "Hierophant voice"
  profile through walls or the floor (its `wall_hz` 20 kHz, `wall_db` 0
  in mix.ron), so he's clear while sunk mid-teleport or dying.

- **Phase 2 as built** (the same two `fight.rs` files):
  - At 2/3 health (`PHASE_2_AT`) he stops for 3 s and plays
    `phase_2.mp3`. 0.8 s in, a ring of the dead rises around him:
    6 zombies, plus 2 per extra player, up to 12 (`ring_size`), 3.5–6.5 m
    out. They're tagged `Raised`.
  - Another ring comes whenever 2 or fewer are left, at most every 25 s.
  - Raised zombies give points but drop nothing:
    `BotBrain::without_drops` → `ZombieKilled::drops`, which power-ups
    and equipment both check. The Hierophant drops nothing either.
  - Raised zombies are never recycled into the round. When he dies (or
    the egg is set elsewhere), they all drop dead too, with no points.
  - Teleport: every 12–18 s, or once nobody has been within 10 m of him
    for 4 s. He sinks into the floor in green smoke (1.2 s), stays under
    for 0.8 s, then rises out of the ground 4–7 m from his target, on
    their floor (1.4 s). Nothing can hurt him for that whole time
    (`BotBrain::untouchable`, checked in `pvp::apply_player_hits`).
  - The client sinks his avatar (`Sinking`), plays the green bursts
    (`Burst`), and plays `teleport_out` / `teleport_in` / `raise_dead`.
  - Voice lines (`HierophantLine` in `shared::easter_egg`):
    - taunts, as before;
    - `player_down_*` when his hit downs someone (at most every 6 s);
    - `hurt_*` when he loses 5% of his health within 0.75 s (at most
      every 9 s);
    - `phase_2`, `phase_3`, and `death` when he dies.
    He never talks over himself; the phase and death lines cut in. His
    avatar has no boss growls or death roar (`boss::BossVoiced`).
- **Recordings are all in** (Nov 1): every Hierophant line plus the three
  phase-2 sounds. `WAKE_SCREAM_SECS` is now 8.28 to match the new
  `wake.mp3`. Operator easter-egg quotes are still to come (the user
  will add them). `boss_raises_dead` should play at the ring.
- **Known failing (not from the egg work):** 4 `server` nav tests, e.g.
  "DerTempel door 11: the ground either side of it is one zone", from
  the map's door and spawn layout.

- **The music** (`client/src/easter_egg/music.rs`):
  `audio/music/disastrous.mp3` (the user's track, 106 s, looped).
  - At 9.5 s into the ritual the Zombies ambience starts fading out, and
    it's gone by 12 s. At 12.2 s the music starts fading in, over 3 s.
    His "every drop given freely" line comes at 13 s.
  - It plays for as long as `is_unleashed()`. After that (he's dead, the
    egg is reset, or the game ends) it fades out over 4 s, then the
    ambience fades back in over 4 s.
  - The ambience's level is `zombies_audio::ZombiesAmbientLevel`. Tune
    the timings in the debug panel under "Easter egg (Der Tempel)" →
    "Music".
  - Each phase has its own track (`TRACKS` in `music.rs`, built Nov 1,
    needs a listen): `disastrous.mp3` from the ritual through phase 1,
    then `hierophant_phase_2.mp3` and `hierophant_phase_3.mp3` (60 s
    each, looped). As his phase changes, what's playing fades out over
    2 s, then the next fades in over 1 s ("Music" in the panel).
- **Phase 1 as built** (`server/src/easter_egg/fight.rs`,
  `client/src/easter_egg/fight.rs`):
  - **He's the normal boss's model (`boss.glb`) for now.** The user is
    making his real model (occult look, his own animations) and will swap
    it in later. Until then the only visible difference is his green aura
    (`boss::BossTint`) and the name on his health bar. When the model
    lands, he needs his own avatar and clips instead of
    `boss::spawn_boss_avatar`, and probably his own `ZombieAnim` states
    (taunt, teleport, summon, etc.). Right now his pose uses the
    `Boss*` states, which also give him the boss hitbox.
  - The moment the step is BossFight, the server spawns him on top of
    the sarcophagus (`shared::easter_egg::hierophant_start`, the same
    `HIEROPHANT_FEET` the wake-up's placeholder body stands on). He
    steps down off it. The placeholder body disappears once
    `EasterEgg::hierophant` is set.
  - His brain is `BotBrain::hierophant`. It's a boss that only smashes,
    never blasts, walks at `HIEROPHANT_SPEED` (1.95 vs a boss's 1.7) and
    hits for `HIEROPHANT_MELEE_DAMAGE` (85 vs 70). His health is
    `hierophant_health`: 3× that round's boss health. All of these are
    in `shared::easter_egg`.
  - He has no `boss::Boss` component, so recycling far-off bosses and
    the round's boss counting skip him. Nukes don't touch him, the
    "zombies drop dead" step skips him, and Maddox's first-boss-kill
    line doesn't fire for him. Insta-Kill doesn't work on him, as with
    any boss.
  - Taunts: the first comes 6 s into the fight, then one every 16–26 s.
    He stops and glares for 2.5 s while each one plays
    (`TAUNT_*`, `shared::HierophantSays`). The whole `hierophant/`
    folder is on "Hierophant voice" in `mix.ron`, except the whispers,
    which stay on "Hierophant".
  - His health bar sits top centre under the compass. Tune it in the
    debug panel under "Easter egg (Der Tempel)" → "The fight".
  - Killed: the step moves to Ascension (which does nothing yet), so
    the round resumes and the map's mood fades back. Step 10 replaces
    that with his death sequence. `EasterEgg::hierophant` keeps who he
    was until the egg is set back before the fight.
  - Ideas for when the real model and animations exist (discussed with
    the user): an overhead slam with a ground shockwave, a dagger lunge,
    and a cursed orb thrown at anyone keeping their distance (phase 1).
    Floor sigils that erupt under players (phase 2). A life-drain grab
    (phase 3).

- **"He wakes" as built** (`client/src/easter_egg/wake.rs`):
  - The server moves from Unleashed to Awakening once the radio line is
    done plus 2.5 s (`shared::easter_egg::unleashed_secs`), then from
    Awakening to BossFight after `AWAKENING_SECS`. That's the
    `run_unleashed` system in `server/src/easter_egg.rs`.
  - The timeline is the `WAKE_*` consts in `shared::easter_egg`:
    - 0 s: the lid blasts off (it's two `PartRole::Lid` boxes on the
      sarcophagus placeholder), with a flash, a camera jolt and the lights
      stuttering.
    - 1.2–4.2 s: the wisps spiral down into the body.
    - 4.2–6.2 s: stillness. The glow and the spirit sound cut dead, and
      the lights hold steady (`EggLightCue::steady`).
    - 6.2 s: he snaps upright with glowing eyes and the scream, plus a
      non-positional hit, a hard camera shake (`EggCameraShake`, layered
      in `weapons::camera_shake`) and another stutter.
  - The placeholder body stays standing on the sarcophagus through
    BossFight. **Step 7 has to replace it with the real boss** (despawn
    `WakeFx` once the boss entity exists, spawning him where it stands:
    `FEET` on the sarcophagus).
  - Placeholder sounds, copied from existing ones (record over them):
    `audio/zombies/easter_egg/awakening/{lid_blast,spirit,wake_hit}.mp3`
    and `hierophant/wake.mp3`. If the real scream isn't 5.86 s long,
    change `WAKE_SCREAM_SECS`.
  - BossFight currently does nothing, and the round stays held (no
    spawns) until the boss is built.
- **"It goes wrong" as built:**
  - `during_ritual.mp3` plays from the sarcophagus at 13 s into the
    ritual. Because it runs 5.6 s, it carries on past the change at 15 s.
    `ritual_gone_wrong.mp3` follows 0.6 s after it ends, then the radio
    line 0.5 s after that. The timings are in `shared::easter_egg`
    (`RITUAL_VOICE_*`, `UNLEASHED_*`). A re-recording of a different
    length means changing `RITUAL_VOICE_SECS` / `UNLEASHED_VOICE_LEN_SECS`.
  - Both lines use their own mix profile, "Hierophant voice" (heard out
    to 60 m), set per clip in `mix.ron`. The whispers keep "Hierophant"
    (16 m). The fight lines will probably want "Hierophant voice" too.
  - Every zombie drops dead 0.2 s after the change, silently: no points,
    no drops, no death cries. The round's own spawns, its clocks, dogs
    and Big Steves are all held while `EggStep::is_unleashed()`
    (Unleashed, Awakening, BossFight), via
    `shared::easter_egg::holds_spawns` in `server/src/zombies.rs`
    `run_rounds`. The round picks up where it left off after he dies.
    The boss's own zombie ring (step 8) needs its own spawning and
    mustn't go through the held round.
  - The map's change is `client/src/environment/egg_mood.rs`. It fades
    in over 2.5 s, layered over the map's own look (and a dog round's),
    and lasts as long as `is_unleashed()`. It covers fog, ambient, sky,
    ground fog, green fires and fire light (`flames.rs`), red pools and
    waterfalls (`water.rs`, and `waterfall.wgsl` now tints by the
    material's colour), and every light in the world stuttering
    **in unison** (`LightFlicker`, `flicker_lights` / `unflicker_lights`
    in `First`/`Last`, flashlights excluded; the power bulbs dim in
    `power.rs`). Tune it in the debug panel under "Easter egg (Der
    Tempel)" → "It goes wrong — the map" (there's a preview checkbox).
- **The pyramid altar and spots are placed** in `shared/levels/der_tempel.ron`.
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
  visuals and captions, whispers, his two lines (`hierophant_speaks`),
  and the debug panel section ("Easter egg (Der Tempel)").
- `client/src/environment/egg_mood.rs`: the map gone wrong and the light
  stutter.
- `client/src/easter_egg/wake.rs`: the wake-up.
- `server/src/easter_egg.rs` `run_unleashed`: zombies dropping, the
  radio line.
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

### The boss

5. **[you]** Model, rig and animate the Hierophant: lying in the
   sarcophagus, waking, walking, attacking, teleporting, raising zombies
   and dying. Model the sarcophagus and its lid.
6. **[you]** Record the boss voice lines (waking, taunts, phase 2, phase
   3, downing a player, hurt, death, the pyramid scream:
   `client/notes/quotes/hierophant-quotes.md`; `wake.mp3` is in with a
   placeholder), the wake-up's sounds (`audio/zombies/easter_egg/awakening/`:
   `lid_blast.mp3`, the spirit pouring in `spirit.mp3` (≤ 3 s, it's cut
   at the stillness) and the hard hit as he snaps up `wake_hit.mp3`, all
   placeholders now), the operators' reactions
   (`operator_quotes/*/…-easter-egg-quotes.md`), and the boss fight
   music.
7. **Boss fight phase 1 (built, needs testing):** a heavy melee stalker
   with taunts, his own health bar, and normal zombie spawns stopped
   during the fight.
   **Test:** use the debug panel to spawn him (jump to "Boss fight"); he
   chases, hits and taunts, and can be killed.
8. **Phase 2 (66%) (built, needs testing):** he raises a ring of zombies
    and teleports, sinking into the ground and rising out of it near a
    player (changed from the fire bowls).
    **Test:** use a debug button to set his health; check the zombie ring
    and the teleports.
9. **Phase 3 (33%) (built, needs testing):** he's enraged and faster,
    in blood-red fog with glowing eyes, turning red himself; each new
    phase shakes the ground.
    **Test:** use a debug button to set his health; check that he's faster
    and the fog comes in.
10. **His death (built, needs testing):** the body collapses, his spirit
    flies screaming into the pyramid, and the environment goes back to
    normal.
    **Test:** kill him; everything resets to how it looked before the
    ritual.

### The pyramid

11. **[you]** Model the pyramid's inner chamber and its altar, and record
    the sound for floating up and the chamber's ambience.
12. **Astral projection:** the players' bodies slump at the altar, their
    souls float up through the pyramid's apex into the chamber, and the
    zombies pause.
    **Test:** use the debug button to jump to after the boss's death; float
    up, look around, and check that nothing can hurt the bodies left below.
13. **[you]** Model and animate the wonder weapon and record its sounds:
    firing, reloading and pickup. Decide what it does, plus its magazine
    sizes for each Pack-a-Punch level.
14. **The wonder weapon:** one for each player, taken from the chamber
    altar, then a white flash and they wake at the altar holding it.
    **Test:** take it; you're back below with the gun, and the round
    carries on.
15. **The wonder weapon's behaviour + Pack-a-Punch upgrades.**
    **Test:** shoot zombies at each Pack-a-Punch level.
16. **Finish:** the final radio line (`RadioLine::EggAfterPyramid`, recorded) plays, the egg can't be done twice in
    one game, and everything resets.
    **Test:** a full run start to finish, solo and with 2 players, then a
    new game to check that nothing carries over.
17. **Changelog line** (keep it vague so it doesn't spoil the easter egg).

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

- His voice never muffled by walls
- His death (spirit flees into the pyramid)
- Wake-up body no longer stands early
- Phase 2 and 3 music
- Phase 3 (enraged, red fog, red eyes, phase quakes)
- He wakes (lid blast, spirit, stillness, snap + scream, shake)
- It goes wrong (map change, light stutter, zombies drop, lines + radio)
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
