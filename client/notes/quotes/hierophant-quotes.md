# The Hierophant (Der Tempel easter egg boss)

Voice for every line: an extremely deep, distorted, demonic voice that
screams more than it speaks. Ancient and furious, with a guttural growl
under every word. He's been trapped for a thousand years and he hates
everyone alive.

Each recording goes in `client/assets/audio/zombies/easter_egg/hierophant/`
under the file name in its heading. Record over a file that's already
there to replace it.

---

# Recorded

## Whispers before the ritual (`whisper_1.mp3` – `whisper_3.mp3`)

Recorded and playing. Leave these as they are.

## During the ritual (`during_ritual.mp3`)

Recorded, but re-record it more aggressive. The file is in the game but
nothing plays it yet; it gets wired into the ritual next session.

[ extremely deep, distorted, demonic, rising from a growl into a scream ] Yes... [ louder ] YES! [ roaring scream ] EVERY DROP! [ snarling, guttural ] Given... [ screaming, distorted ] FREELY!

## The ritual goes wrong (`ritual_gone_wrong.mp3`)

Recorded, but re-record it more aggressive. It plays in the "it goes
wrong" step, which isn't built yet.

[ deep, distorted, slow, mocking guttural laugh ] You thought you were closing a door... [ pause ] [ screaming, monstrous, distorted ] YOU HELD IT OPEN FOR ME!

---

# Also recorded (Nov 1)

All of the lines below are recorded. Where each one plays:

- `wake.mp3`: when he wakes.
- Taunts, `phase_2`, `player_down_*`, `hurt_*` and `death`: in the
  fight (`server/src/easter_egg/fight.rs` decides when).
- `phase_3` and `pyramid_fade`: not wired up yet (phase 3 and the
  pyramid are later steps).

Phase 2 also uses three sound effects from this folder, not voice lines:
`teleport_out.mp3`, `teleport_in.mp3` and `raise_dead.mp3`.

If you re-record a fight line at a different length, change its length
in `HierophantLine::secs` in `shared/src/easter_egg.rs`. The server uses
those lengths so he never talks over himself.

## He wakes in his body (`wake.mp3`)

[ huge rasping inhale ] [ roaring, distorted, earth-shaking scream ] A THOUSAND NIGHTS IN THE DARK! [ snarling, guttural ] AND NOW... [ screaming ] THIS BODY IS MINE!

## Taunts during the fight (`taunt_1.mp3` – `taunt_6.mp3`)

A random one every so often.

- 1 [ guttural, distorted, screaming ] YOUR BLOOD IS IN MY VEINS! [ snarling ] I CAN HEAR YOUR HEARTS!
- 2 [ sadistic, deep growl ] Yes.. Run! [ evil insane laugh ] [ screaming, growing louder ] I LOVE IT WHEN THEY RUN!!!
- 3 [ roaring, distorted ] MY CHILDREN DIED FOR ME! [ snarling, screaming ] SO WILL YOU!
- 4 [ deep, distorted, hissing ] You cannot kill what you brought back... [ screaming, demonic ] YOU MADE ME!
- 5 [ guttural laugh ] [ screaming, distorted, commanding ] KNEEL! [ roaring ] KNEEL BEFORE YOUR MASTER!
- 6 [ deep, distorted, mocking scream ] YOUR BULLETS ARE PRAYERS! [ snarling, voice raising high, screaming ] AND NO ONE IS LISTENING!

## Phase 2: raising the dead (`phase_2.mp3`)

[ commanding, distorted, earth-shaking roar ] RISE! [ pause ] [ screaming ] RISE, MY CHILDREN! [ roaring ] YOUR MASTER HAS RETURNED!

## Phase 3: enraged (`phase_3.mp3`)

[ enraged, distorted, shrieking scream ] ENOUGH!!! [ guttural, snarling, screaming ] I WILL WEAR YOUR SOULS LIKE A CROWN!

## He downs a player (`player_down_1.mp3` – `player_down_3.mp3`)

- 1 [ distorted, triumphant scream ] ANOTHER OFFERING!
- 2 [ deep, mocking, distorted ] Bleed for me... [ screaming ] BLEED!
- 3 [ guttural laugh, distorted, screaming ] ONE LESS HEART BEATING!

## Hurt (`hurt_1.mp3` – `hurt_3.mp3`)

Short pain roars when he takes a big hit (no words).

- 1 [ distorted, pained, furious roar ] AGH!
- 2 [ short, distorted, snarling shriek ] UGH!
- 2 [ deep, distorted, agonized growl turning into a scream ] AGGHHHH!!!!!

## His death (`death.mp3`)

[ agonized, distorted, screaming ] NO! [ louder ] NO!!! [ screaming, desperate ] THIS TEMPLE... [ shrieking, scream fading upward, echoing ] IS MIIIIINE!!!

## His last scream in the pyramid (`pyramid_fade.mp3`)

[ distant, distorted, agonized wail, fading into echoes ] You... will... NEVER... leave... [ dying shriek, fading out ]
