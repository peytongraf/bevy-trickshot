# Der Tempel — Easter Egg

Draft. Order of events, kept short to skim. Anything here can change.

## The story in one paragraph

A secret society bought the temple and ran rituals there to bring back
their founder, **the Hierophant**, who died in the first rite. That rite
half-worked: it opened the way for the dead, but his spirit never found a
body. It's been waiting in a sealed sarcophagus on the upper altar of the
lower tomb. The society's notes describe a rite they "never finished."
Actual reads it as a **closing rite** to put the dead back down, and the
squad does it to end the outbreak. It's really the **last step of the
summoning**. The society's vaccine trial (Big Steve) was how they bred a
body strong enough to hold him, and the rite needs that blood. Doing the
rite wakes the Hierophant.

## What the players gather (and why it fits)

1. **Barrett's blood:** a sealed vial dropped by the first Big Steve kill.
   It has a society tag on it ("Vessel candidate 07"). This is the blood
   of the strongest of the risen, which is the body the spirit needs.
2. **The Hierophant's dagger:** in a locked society case in the newer
   structure. The case only opens once the power is on. This is the blade
   the first rite was done with.
3. **The players' own blood:** given at the altar. That's why everyone
   has to stand in the zone and accept together: the notes say "blood of
   the living, given freely, all at once."

## Order of events

1. **Round 4 (existing lore radio):** the society ran rituals and somebody
   called the dead. This sets up the motive.
2. **Near the sarcophagus, any round:** faint echoing whispers now and
   then. No prompt and no explanation yet.
3. **First Big Steve killed:** the vial drops where he falls. Anyone can
   pick it up (it shows on the HUD for the whole squad). Radio hint 1
   plays.
4. **Power on:** the society case clicks open. Take the dagger. Radio
   hint 2 plays.
5. **Both items held:** radio hint 3 plays. Actual thinks finishing the
   rite will "close" whatever the society opened.
6. **The altar:** every living player stands in the upper-altar zone and
   accepts. If anyone leaves the zone or declines, it resets. Zombies keep
   coming the whole time, so this is a risk.
7. **The ritual (about 15 s, players locked in place):**
   - The dagger is driven into the altar and the vial is poured over it.
   - Each player's hand is cut and their blood drips into a channel.
   - The channels light up and run to the sarcophagus.
   - The players say a short chant line from the notes, believing it
     seals the place.
8. **It goes wrong (environment change):**
   - The fire bowls turn a sick green, the waterfalls and pools run red,
     the sky darkens, the fog thickens and the lights flicker.
   - All zombies on the map drop dead at once. It goes silent.
   - The Hierophant's echoing voice speaks from inside the sarcophagus
     (container quotes).
   - Actual: "...That wasn't a closing rite. God help me, I read it
     backwards."
9. **He wakes:** the lid blasts off and his spirit pours into the body
   inside. A beat of stillness, then he snaps upright with a scream. This
   is the jump scare: a hard audio hit plus a camera shake.
   - Operators react with surprise (they thought they were ending this).
10. **Boss fight (in the lower tomb and the map around it):**
    - **Phase 1:** a slow, heavy melee stalker who taunts.
    - **Phase 2 (at about 66% health):** he raises a ring of zombies and
      teleports through the green fire bowls.
    - **Phase 3 (at about 33% health):** he's enraged and faster, with
      blood-red fog. Only his voice and glowing eyes show through it.
    - Normal zombies stop spawning for the fight; only the ones he raises
      come.
11. **He dies:** the body collapses, and his spirit flees screaming **up
    into the pyramid**. The environment goes back to normal (fires orange,
    water clear).
12. **Astral projection:** the players' bodies slump at the altar and their
    souls float up through the pyramid's apex. This is the only way inside,
    ever.
    - Inside is a dark chamber with the society's original altar. His
      spirit makes a last scream and fades out for good.
    - **The wonder weapon** sits on the altar, made from his relic (staff,
      dagger or skull — TBD). Each player takes one.
13. **Back to the body:** a white flash and they wake at the altar holding
    it. The round resumes. Radio closer plays. The easter egg can't be
    done twice in a game.

**Why the fight comes before the pyramid (recommended):** the scare lands
best in the map they already know, and the pyramid stays a sacred,
untouchable place that's the reward. It also means no new arena,
navigation or zombie spawns inside the pyramid, just a small room.

## Quotes

All the lines are in `client/notes/quotes/`, with how each is said in
`[ ]` for ElevenLabs:

- **The man on the radio:** `radio-quotes.md`, under "Easter egg" (all
  recorded).
- **The Hierophant:** `hierophant-quotes.md`.
- **The operators:** `operator_quotes/adam/adam-easter-egg-quotes.md` and
  `operator_quotes/von/von-easter-egg-quotes.md`.

## Build notes

- Everything placed goes in `der_tempel.ron` and the level editor (with
  `ObjectId`s): the altar zone, society case, sarcophagus, vial drop spot
  rules and the pyramid chamber spawn points.
- The quote audio starts in `client/assets/audio/unused/quotes/`.
- Reset everything between games: the items held, the step reached, the
  environment, and the boss.
