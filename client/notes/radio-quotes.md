# Zombies radio quotes

The voice on the radio in a `Zombies` game: a handler talking to the whole squad from somewhere
else. These are separate from the operator quotes (`audio/quotes/<operator>/`), which are the
player's own character talking. The radio plays for everyone in the lobby at once, isn't
positional, and goes through a radio filter.

---

## The man on the radio

**Callsign:** WARDEN
**Name (never said in full on the radio):** Sergeant Major Calvin "Cal" Rourke, retired. Brought
back on a contract nobody will show him the full version of.

**Who he is:** Late 50s. Twenty-six years in, mostly Force Recon and then "attached" to units
that don't have names. He has done the job the squad is doing now, crawling into places he
shouldn't, so he talks to them like operators, not recruits. Now he sits in a container
full of monitors at a forward operating base, running a program called **ORCHARD**. He has
satellite feeds, thermal drones, intercepted traffic and a stack of documents with most of the
words blacked out.

**Personality:**
- **Calm under fire.** He never yells on comms; his voice gets lower and slower as things get
  worse. When he does raise it, it means something.
- **Swears like breathing.** It's casual and dry, never screaming: "shit", "fuck", "goddamn",
  "son of a bitch". He swears *at the situation*, not at the squad.
- **Speaks proper military.** Callsigns, SITREPs, "be advised", "how copy", "break", "Oscar
  Mike", "danger close", "Charlie Mike", grid references, "tangos". He keeps radio procedure
  even while cursing.
- **Knows more than he says.** Some of it he's not cleared to share and some of it he honestly
  doesn't know. He tells the squad what ORCHARD has confirmed, flags what he *suspects*, and
  then sends them to find the rest. He hates the people above him for keeping him in the dark,
  and that shows.
- **Dark humour, and a little warmth.** He's seen a lot of squads not come back. He won't say he
  cares, but it slips out when someone's down.
- **Off-mic moments.** Now and then he talks to someone in the room with him ("Get me thermal on
  that. Now."), which makes the other end feel real.

**Voice-casting notes:**
- A low, gravelly baritone, like a lifelong smoker, with a slight Southern or Appalachian edge
  and clipped consonants. Think a tired drill instructor who stopped needing to shout years ago.
- Delivery is measured and dry, with short pauses between clauses like he's reading a feed while
  he talks.
- Pacing: briefings are deliberate, while combat calls (dogs, boss, someone down) are faster and
  tighter but still controlled.
- Processing in game: a band-pass of roughly 300–3400 Hz, light saturation, a squelch click at
  the start and a squelch tail at the end, and a faint carrier hiss under it. Record the lines
  dry; the filter can be done in game or baked in.

**What he calls things:**
- The squad: **"Dagger"**, or **"Dagger team"**. One player by name is never needed.
- The zombies: **"hostiles"**, **"the dead"**, **"Cat-Ones"** (ORCHARD's term).
- The hellhounds: **"Cat-Twos"**, **"the hounds"**.
- The boss: **"Cat-Three"**, **"the big one"**, **"the Brute"**.
- The orange and blue lightning: **"the storm"**. ORCHARD doesn't know what it is.
- The machines (perks, Pack-a-Punch, Mystery Box, Wunderfizz, Rampage Inducer): **"the
  hardware"**. They were already there when ORCHARD found the sites, and nobody knows who built
  them.

---

## The mystery (what's known, what isn't)

So the lines stay consistent with each other:

- **Known:** Every site was tied to an off-books research effort codenamed **VESPER**. Every
  VESPER site went dark the same night. Since then, the dead walk there and the storm hits on a
  schedule.
- **Suspected:** The hardware is *older* than VESPER, and VESPER found it rather than built it.
  The power at each site feeds something besides the lights.
- **Unknown:** What the storm is, where the hounds come from, what the Brutes used to be, and
  who is still sending signals out of the sites.

He feeds these out a bit at a time. He never explains the full picture; he always ends on
"find out" or "dig into it".

---

## Triggers and lines

Format: **ID**: when it plays, then the lines. With more than one line, one is picked at random.
The file name is the line ID, e.g. `game_start/shipment_1.mp3`.

Priority (highest first), for when two would overlap: **game over > someone down / last stand >
game start > dog round / boss > exfil > power > everything else**. A lower-priority line waits
or is dropped; it never cuts off a higher one.

### Game start: opening briefing

Plays a couple of seconds after round 1 begins, once per game. There's one per place, and a
generic one for any map without its own. It tells the squad what to do: survive, get the power
on, find out what happened. This is the longest line, at roughly 20–30 s.

**game_start/shipment_1** (Shipment, night or day)
> "Dagger, this is Warden, how copy. ...Good. Listen up, because I'm only saying this once. You
> are standing on a cargo ship that left a VESPER site seventy-two hours ago and never cleared
> customs. Crew's not answering because the crew is what's walking toward you right now. Hold
> that deck. Somewhere on that boat is a breaker that'll get the power back up. Find it, turn it
> on. And Dagger: those containers came off the manifest blank. I want to know what the fuck is
> inside them. Warden out."

**game_start/break_point_1** (Break Point, day or night)
> "Dagger, Warden. Be advised: Break Point went dark eleven days ago. Last transmission out of
> that compound was forty seconds of somebody screaming a grid reference, and that grid
> reference is where you're standing. ORCHARD wants the site held and the power restored. I
> want to know why a research outpost needed walls that thick. Weapons free. Stay off the
> walls, stay together, and don't let them box you in. Warden out."

**game_start/ashes_of_the_damned_1** (Ashes of the Damned)
> "Dagger, Warden, radio check. ...Alright. Whatever VESPER was digging for, they dug deep
> enough to split the goddamn ground open. Those chasms weren't there on last year's imagery. I've
> got heat signatures climbing up out of them that should not be warm. Hold the high ground,
> find the power, and do NOT go looking over the edge. Not yet. Warden out."

**game_start/generic_1** (any other map)
> "Dagger, this is Warden, how copy. You're on the ground at a VESPER site. Everybody who
> worked here is dead, and they did not stay that way. Priorities: stay alive, get the power back
> on, and find me something, anything, that tells me what they were doing here. I'll feed you
> what I've got as I get it. Weapons free. Warden out."

**game_start/generic_2**
> "Warden to Dagger. Here's the SITREP, short version: site's overrun, power's out, and
> command is giving me the runaround. What we know: hostiles don't stop, don't feel pain, and
> they're attracted to noise. What we don't know is fucking everything else. Hold your ground.
> Find the power. Ask questions later. Warden out."

### Round milestones

**round_start/round_2**: start of round 2 (once)
> "Warden. Good work on the first wave, Dagger. Don't get comfortable. Drone's counting more
> signatures moving your way, and they're quicker than the last bunch."

**round_start/round_3_power_off**: start of round 3, but only if the power's still off
> "Dagger, Warden. Power's still out at your location. You want to keep fighting in the dark,
> that's your business. But whatever's in that hardware runs off that grid. Get it on."

**round_start/round_10**: start of round 10
> "Warden to Dagger. Ten waves. Most teams I've sent in didn't see five. ...Be advised, it's
> getting worse, not better. Every round they come back faster, meaner, more of them. Somebody up
> the chain knew this would happen. I intend to find out who."

**round_start/round_15**: start of round 15
> "Dagger, Warden. I've got a document in front of me. Sixty percent black ink. The part I can
> read says VESPER's people were 'feeding the site.' Feeding it what? ...I'll keep digging. You
> keep breathing."

**round_start/round_20**: start of round 20
> "Warden. Twenty rounds. Command just asked me why you're still alive. Not how. *Why.* Like it's
> a problem. ...Watch your six, Dagger. And I mean all of it."

**round_start/round_25** (and every 5 after, as a fallback)
> "Dagger, Warden. You are well past anything I've got a playbook for. Keep doing whatever the
> hell it is you're doing."

### Dogs

**dog_round/first_1**: round 5, the first dog round (once per game)
> "Dagger, Warden, break, break. The storm's spinning up over your position. Same pattern as the
> night the sites went dark. We lost three recon teams to what came down with it last time.
> Fast, four-legged, and they burn. ORCHARD calls them Cat-Twos. I call them a pain in my ass.
> Backs together, watch your flanks. Here they come."

**dog_round/repeat_1**: any later dog round
> "Storm's back, Dagger. Hounds inbound. You know the drill."

**dog_round/repeat_2**
> "Warden. Lightning on your grid. Cat-Twos, multiple. Tighten up."

**dog_round/repeat_3**
> "Here comes the goddamn storm again. Every fifth wave, like clockwork. Somebody set that clock,
> Dagger."

**dog_round/cleared_first**: end of the first dog round (once)
> "Warden. Hounds are down. ...Dagger, I pulled the thermal from that strike. The hounds didn't
> come *down* with the lightning. They came *through* it. Through from where, I don't know. Yet."

### Bosses

**boss/first_1**: the first boss of the game shows up (round 7)
> "Dagger, Warden. Big thermal signature, one hundred meters and closing. That's a Cat-Three.
> We've only ever seen one on a drone feed and the drone didn't last long. It throws energy, it
> hits like a truck, and small arms barely scratch it. Keep moving. Don't let it get close.
> Kill that son of a bitch."

**boss/repeat_1**: any later boss
> "Warden. Cat-Three on your grid. Spread out, keep it at range."

**boss/repeat_2**
> "Another Brute, Dagger. Same rules: keep moving and empty your mags into it."

**boss/killed_first**: the first boss goes down (once)
> "...Confirm, Cat-Three is down. Hell of a job, Dagger. Be advised, get eyes on what's left of
> it. ORCHARD swears the Brutes were VESPER personnel. I want to know which ones."

**boss/killed_1**: any later boss kill
> "Big one's down. Good shooting."

### Power

**power/on_1**: someone turns the power on
> "Warden. I'm reading power at your location. ...Holy shit, the whole grid just lit up. More
> than lights, Dagger. That hardware's waking up. Whatever VESPER plugged into that site, it's
> drawing current now. Go see what it does."

**power/on_2**
> "Power's up, Dagger. Good. Now I've got eyes on more of the site. ...And I'm seeing machines
> on my feed that aren't on any blueprint I was given. Check them out."

### The hardware

**pack_a_punch/first_1**: first time anyone Pack-a-Punches (once)
> "Dagger, Warden. What in God's name did that machine just do to your weapon? My readings went
> off the scale. ...Don't tell command. Just use it."

**mystery_box/first_1**: first time anyone uses the Mystery Box (once)
> "Warden. That box, Dagger. VESPER logged it as 'non-local.' I don't know what that means
> either. Take what it gives you and don't stand there staring at it."

**wunderfizz/awake_1**: round 17, when Der Wunderfizz wakes up
> "Dagger, Warden. Something just powered on at your site that was cold for sixteen rounds.
> Nobody touched it, and it turned itself on. ...Go see what it wants."

**rampage/on_1**: someone turns the Rampage Inducer on
> "Dagger, did you just flip something? Every hostile on my thermal just picked up speed.
> ...I sure as shit hope you know what you're doing."

**rampage/off_1**: someone turns it back off
> "Warden. Hostiles slowing down. Whatever you shut off, keep it that way unless you mean it."

**aether_shroud/first_1**: first Aether Shroud use in a game (once)
> "Dagger, I just lost you on thermal. Completely. ...You're still there, right? Christ, VESPER
> had that too."

### Exfil

**exfil/available_first_1**: round 11, the first time the exfil can be called
> "Dagger, Warden. Got you a ride. There's a radio on your site that's still tied into the
> relay. Get on it and I'll call an exfil. Fair warning: the second that bird spins up, every
> dead thing on that site is going to hear it. You'll have to clear the LZ. Your call."

**exfil/available_1**: every later exfil round (21, 31, ...)
> "Warden. Exfil window's open again, Dagger. Radio's live. Say the word."

**exfil/called_1**: the exfil gets called
> "Copy, Dagger, exfil is inbound. Ninety seconds. Get inside the LZ and kill everything that
> moves. Kills outside the LZ don't count for shit. Go!"

**exfil/called_2**
> "Bird's in the air. Ninety seconds, Dagger. Clear that LZ. Danger close, all directions."

**exfil/thirty_seconds_1**: 30 s left on the exfil
> "Thirty seconds! LZ's still hot, Dagger, finish it!"

**exfil/success_1**: the exfil succeeds
> "LZ is clear. Get on the bird, Dagger. ...Good work. Hell of a job. Debrief's at oh-six-hundred.
> Bring everything you saw."

**exfil/failed_1**: the exfil fails (time ran out)
> "Too hot. Bird's waving off! Pilot won't set down in that. ...Goddamn it. Dig in, Dagger.
> I'll get you another window."

### Down and out

**down/first_1**: a player goes down
> "Man down, man down! Somebody get to 'em!"

**down/1**
> "Dagger, you've got a man on the ground. Revive, revive!"

**down/2**
> "I've got a downed signal. Move, they're bleeding out!"

**revived_1**: a downed player gets picked back up (sometimes, not every time)
> "They're up. Good. Nobody gets left on that site."

**bled_out_1**: a player bleeds out
> "...I lost their signal. Goddamn it. Keep fighting, Dagger. They'll be back on the next wave,
> or I don't know a damn thing about this place."

**last_stand_1**: only one player is still up
> "You're the last one standing, Dagger. Everybody else is on you. Do not go down."

**last_stand_2**
> "It's all you now. Breathe. Pick your shots. Get them up."

### Game over

**game_over/1**: everyone's down
> "Dagger, Warden, come in. ...Dagger, how copy? ...Dagger. ...Mark the time. Site is lost.
> ...Get me the next team."

**game_over/2**
> "All signals dark. ...Son of a bitch. ...Log it, ORCHARD. Same as the others."

**game_over/high_round_1**: game over after round 20 or later
> "Dagger, come in. ...Nothing. ...Twenty-plus rounds. Longer than anybody. Whatever's down
> there, they pushed it further than we ever have. ...Pull everything off their helmet cams.
> Every frame."

---

## Recording checklist

- One file per line ID as `<event>/<id>.mp3` (snake_case, no hyphens, no `sound` suffix).
- They start in `client/assets/audio/unused/radio/` and move to `client/assets/audio/radio/` as
  they get wired up, the same way the operator quotes do.
- Record dry, with a second of silence trimmed off each end.
- Record a couple of short generic squelch/static clicks (`radio/squelch_in`,
  `radio/squelch_out`) as well, if you're not baking them into each line.
