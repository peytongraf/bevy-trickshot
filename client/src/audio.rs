//! Preloaded sound handles and the looping ambience bed — plus pushing
//! `Settings::master_volume` into the audio engine. Each sound's
//! own level is the audio mixer's business (`audio_engine::mix`).
//!
//! `assets/audio/` layout: one folder per category — `ambient/`, `combat/`,
//! `movement/` (+ `footsteps/`), `music/`, `quotes/<operator>/`, `ui/`,
//! `weapons/<weapon>/`, `zombies/`, and `unused/` for clips not wired up yet
//! (operator quotes move from `unused/quotes/` to `quotes/` as they're used). Files are `snake_case`, named for
//! what they are within their folder (no `-sound` suffix, no repeating the
//! folder's name: `weapons/sniper/shot.wav`), with variations numbered
//! `name_1`, `name_2`, …

use bevy::prelude::*;

use crate::audio_engine::prelude::*;

use crate::environment::CurrentMap;
use crate::player::FOOTSTEP_CLIPS;
use crate::settings::Settings;
use crate::AppState;

/// Preloaded sounds. Loaded once at startup so playback has no first-use hitch.
#[derive(Resource)]
pub(crate) struct GameSounds {
    pub(crate) shot: Handle<AudioSource>,
    pub(crate) rechamber: Handle<AudioSource>,
    pub(crate) reload: Handle<AudioSource>,
    /// `audio/weapons/ak_74/` — the AK-74's shot, its full (empty-mag,
    /// chambering) reload and its fast (mag-swap only) reload.
    pub(crate) ak_shot: Handle<AudioSource>,
    pub(crate) ak_reload: Handle<AudioSource>,
    pub(crate) ak_reload_fast: Handle<AudioSource>,
    /// `audio/weapons/raygun/` — the Ray Gun's shot, reload, and the sound
    /// it makes when it first comes into our hands.
    pub(crate) raygun_shot: Handle<AudioSource>,
    pub(crate) raygun_reload: Handle<AudioSource>,
    pub(crate) raygun_equip: Handle<AudioSource>,
    ambient: Handle<AudioSource>,
    /// `Shipment`'s own ambience bed — a cargo ship out on open water, so
    /// nothing like `ambient`'s outdoor-nature loop. `start_ambient` picks
    /// between the two by [`CurrentMap`]; `basic_map.glb` keeps `ambient`.
    shipment_ambient: Handle<AudioSource>,
    pub(crate) aim_in: Handle<AudioSource>,
    pub(crate) aim_out: Handle<AudioSource>,
    pub(crate) out_of_ammo: Handle<AudioSource>,
    pub(crate) slide: Handle<AudioSource>,
    /// `audio/movement/phd_slider_slide.mp3` — a slide with PhD Flopper, in
    /// place of `slide`.
    pub(crate) phd_slide: Handle<AudioSource>,
    pub(crate) dive: Handle<AudioSource>,
    pub(crate) kill_enemy: Handle<AudioSource>,
    pub(crate) jump_land: Handle<AudioSource>,
    pub(crate) teleport: Handle<AudioSource>,
    /// `audio/weapons/throwing_knife/throw.mp3` — the knife leaving the hand.
    pub(crate) knife_throw: Handle<AudioSource>,
    /// `audio/weapons/throwing_knife/hit_enemy.mp3` — a thrown knife killing a bot / player.
    pub(crate) knife_hit: Handle<AudioSource>,
    /// `audio/weapons/throwing_knife/in_air.mp3` — the whoosh that follows a thrown knife.
    pub(crate) knife_in_air: Handle<AudioSource>,
    /// `audio/weapons/knife/equip.mp3` — switching to the regular knife.
    pub(crate) knife_equip: Handle<AudioSource>,
    /// `audio/weapons/molotov/molotov_light.mp3` — the rag catching and
    /// burning while the molotov's held (only the holder hears it, looped).
    pub(crate) molotov_light: Handle<AudioSource>,
    /// `audio/weapons/molotov/molotov_burst.mp3` — a thrown molotov breaking,
    /// from where it broke, for the whole lobby.
    pub(crate) molotov_burst: Handle<AudioSource>,
    /// `audio/weapons/monkey_bomb/` — a monkey bomb primed (the thrower's
    /// alone), thrown, landing, its song while it claps, and its "bye bye
    /// zombies" before it blows up (the rest from the monkey, for the whole
    /// lobby — `monkey_bomb`).
    pub(crate) monkey_bomb_prime: Handle<AudioSource>,
    pub(crate) monkey_bomb_throw: Handle<AudioSource>,
    pub(crate) monkey_bomb_land: Handle<AudioSource>,
    pub(crate) monkey_bomb_song: Handle<AudioSource>,
    pub(crate) monkey_bomb_explode_vox: Handle<AudioSource>,
    /// `audio/weapons/frag/` — a frag's pin coming out (the thrower's alone,
    /// not positional) and its explosion (from where it went off, for the
    /// whole lobby — `vfx::explosion`).
    pub(crate) frag_pin_pull: Handle<AudioSource>,
    pub(crate) frag_explosion: Handle<AudioSource>,
    /// `audio/weapons/flash_bang/flash_bang_detonate.mp3` — a flash bang
    /// going off, from there, for the whole lobby (`flash_bang`).
    pub(crate) flash_bang_detonate: Handle<AudioSource>,
    /// `audio/weapons/pick_up_equipment.mp3` — picking up a thrown knife
    /// (only the picker hears it).
    pub(crate) pick_up_equipment: Handle<AudioSource>,
    /// `audio/weapons/sniper/equip.mp3` — switching to the sniper.
    pub(crate) sniper_equip: Handle<AudioSource>,
    /// `audio/combat/hit_marker.mp3` — one of your shots damaged
    /// (but didn't kill) a bot / player (`hit_marker`).
    pub(crate) hit_marker: Handle<AudioSource>,
    /// `audio/zombies/purchases/buy_perk.mp3` — we just bought a perk in
    /// `Zombies` (`zombies_hud::sync_owned_perks`).
    pub(crate) perk_buy: Handle<AudioSource>,
    /// `audio/zombies/purchases/buy_pap.mp3` — we just Pack-a-Punched a weapon
    /// (`pap_menu`).
    pub(crate) pap_buy: Handle<AudioSource>,
    /// `audio/zombies/perks/jingles/{custom,classic}/<perk>` — a perk machine's
    /// jingle, played from the machine for the whole lobby when anyone buys
    /// that perk (`zombies_hud::play_perk_jingles`). Pick one with
    /// [`Self::jingle`].
    jingles: std::collections::HashMap<shared::perks::Perk, Handle<AudioSource>>,
    /// `audio/quotes/<operator>/perk_quotes/` — each operator's lines after
    /// buying a perk (`zombies_hud::play_perk_quotes`). Pick one with
    /// [`Self::perk_quote`].
    perk_quotes: std::collections::HashMap<shared::operator::Operator, PerkQuotes>,
    /// `audio/zombies/machines/power_on.mp3` — someone threw the `Zombies` power
    /// lever: from the lever, for the whole lobby (`power::sync_power_lever`).
    pub(crate) power_on: Handle<AudioSource>,
    /// `audio/zombies/doors/open.mp3` — a door sliding open once it's bought:
    /// from the door, for the whole lobby (`doors::slide_doors`).
    pub(crate) door_open: Handle<AudioSource>,
    /// `audio/zombies/doors/main_door_ambience.mp3` — a scary swell as a
    /// map's main door (`shared::doors::Door::main`) opens: from it, for the
    /// whole lobby, heard well beyond it (`doors::slide_doors`).
    pub(crate) main_door_ambience: Handle<AudioSource>,
    /// `audio/zombies/machines/der_tempel_power_on.wav` — the rumble and
    /// music of Der Tempel's power-on earthquake: for the whole lobby, not
    /// positional, faded with the quake (`earthquake.rs`).
    pub(crate) der_tempel_power_on: Handle<AudioSource>,
    /// `audio/zombies/mystery_box/` — the Mystery Box's lid opening and
    /// shutting, and the music while its prizes cycle: from the box, for the
    /// whole lobby (`mystery_box::play_box_sounds`).
    pub(crate) mystery_box_open: Handle<AudioSource>,
    pub(crate) mystery_box_close: Handle<AudioSource>,
    pub(crate) mystery_box_spin: Handle<AudioSource>,
    /// `audio/zombies/armor/destroy.mp3` — the last of our armor broke
    /// (`armor::play_armor_broken`).
    pub(crate) armor_destroy: Handle<AudioSource>,
    /// `audio/zombies/aether_shroud/` — our Aether Shroud going up, its
    /// loop while it's up, and it wearing off (`aether_shroud`).
    pub(crate) aether_activate: Handle<AudioSource>,
    pub(crate) aether_active: Handle<AudioSource>,
    pub(crate) aether_deactivate: Handle<AudioSource>,
    /// `audio/zombies/boss/` — a boss's lightning strike in, its growls,
    /// its smash, its death, a blast's flight (looped from the ball) and its
    /// explosion: all from where they happen, for the whole lobby (`boss`).
    pub(crate) boss_spawn: Handle<AudioSource>,
    pub(crate) boss_growl: Handle<AudioSource>,
    pub(crate) boss_attack: Handle<AudioSource>,
    pub(crate) boss_death: Handle<AudioSource>,
    pub(crate) boss_blast: Handle<AudioSource>,
    pub(crate) boss_blast_hit: Handle<AudioSource>,
    /// `audio/zombies/machines/pap_buzzing.mp3` — the electric hum looped from every
    /// perk machine and the Pack-a-Punch once the power's on
    /// (`power::sync_machine_hums`).
    pub(crate) machine_hum: Handle<AudioSource>,
    /// `audio/ambient/fire_crackle.mp3` — looped from every fire the map
    /// keeps burning (`flames`).
    pub(crate) fire_crackle: Handle<AudioSource>,
    /// `audio/zombies/rounds/round_start.wav` — a `Zombies` round starting, for
    /// everyone (`round_counter`).
    pub(crate) round_start: Handle<AudioSource>,
    /// `audio/zombies/rounds/dog_round_{start,end}.mp3` — a dog round
    /// starting (in place of `round_start`), and ending (`round_counter`).
    pub(crate) dog_round_start: Handle<AudioSource>,
    pub(crate) dog_round_end: Handle<AudioSource>,
    /// `audio/zombies/dogs/*.mp3` — a hellhound's: the lightning before it
    /// appears, its appearing, its bark (looped while it's alive) and its
    /// explosion, each from where it happens (`dogs`).
    pub(crate) dog_pre_spawn: Handle<AudioSource>,
    pub(crate) dog_spawn: Handle<AudioSource>,
    pub(crate) dog_bark: Handle<AudioSource>,
    pub(crate) dog_explosion: Handle<AudioSource>,
    /// `audio/zombies/revive/{player_down,revived}.mp3` — a lobby member
    /// going down (everyone hears it), and a revive finishing (the revived
    /// and their reviver hear it) — neither positional (`revive`).
    pub(crate) player_down: Handle<AudioSource>,
    pub(crate) revived: Handle<AudioSource>,
    /// `audio/zombies/perks/bomb_shot_explosions/bomb_shot_explosion_1..N.wav` — a
    /// Bomb Shot blast, from where it went off (`vfx::spawn_explosions`);
    /// the server picks which one so the whole lobby hears the same clip.
    pub(crate) bomb_shot_explosions: Vec<Handle<AudioSource>>,
    /// `audio/combat/heartbeat.mp3` — looped while the player is
    /// hurt (`health::update_heartbeat`).
    pub(crate) heartbeat: Handle<AudioSource>,
    /// `audio/movement/footsteps/footstep_1..N.wav` — `footsteps` picks one at random
    /// per step.
    pub(crate) footsteps: Vec<Handle<AudioSource>>,
    /// `audio/music/main_menu.wav` — looped on the main menu and in lobbies
    /// (`sync_menu_music`).
    menu_music: Handle<AudioSource>,
    /// `audio/ambient/zombies.mp3` — the eerie bed looped under a running
    /// `Zombies` game (`zombies_audio`).
    pub(crate) zombies_ambient: Handle<AudioSource>,
    /// `audio/zombies/purchases/buy_ammo.mp3` — buying ammo at the ammo crate (only
    /// the buyer hears it).
    pub(crate) buy_ammo: Handle<AudioSource>,
    /// `audio/zombies/purchases/money_ching.mp3` — the prone-at-a-perk-machine bonus
    /// paying out (only the one who got it hears it).
    pub(crate) money_ching: Handle<AudioSource>,
    /// `audio/zombies/power_ups/grab.mp3` — walking into a power-up (only
    /// the one who grabbed it hears it).
    pub(crate) power_up_grab: Handle<AudioSource>,
    /// `audio/zombies/power_ups/drop_loop.mp3` — looped from every power-up
    /// lying on the ground (positional).
    pub(crate) power_up_loop: Handle<AudioSource>,
    /// `audio/zombies/power_ups/<name>.mp3` — the announcer for each
    /// power-up, for everyone when it's grabbed (`power_ups::announcer`).
    pub(crate) power_up_max_ammo: Handle<AudioSource>,
    pub(crate) power_up_insta_kill: Handle<AudioSource>,
    pub(crate) power_up_double_points: Handle<AudioSource>,
    pub(crate) power_up_nuke: Handle<AudioSource>,
    pub(crate) power_up_bonus_points: Handle<AudioSource>,
    /// `audio/music/zombies_game_over.mp3` — played once when a `Zombies`
    /// game ends (`zombies_audio`).
    pub(crate) zombies_game_over: Handle<AudioSource>,
    /// `audio/zombies/windows/` — a window's boards (`windows`): a zombie
    /// snapping one off (one of `board_snaps` at random), one floating back
    /// up into place and slamming home (`board_float`, then one of
    /// `board_slams`), and the hammering while we rebuild.
    pub(crate) board_snaps: Vec<Handle<AudioSource>>,
    pub(crate) board_slams: Vec<Handle<AudioSource>>,
    pub(crate) board_float: Handle<AudioSource>,
    pub(crate) hammering: Handle<AudioSource>,
}

impl GameSounds {
    /// `perk`'s machine jingle — `None` for a perk that doesn't have one yet.
    pub(crate) fn jingle(&self, perk: shared::perks::Perk) -> Option<Handle<AudioSource>> {
        self.jingles.get(&perk).cloned()
    }

    /// What `operator` says after buying `perk`: for a classic perk, one of
    /// its own lines or one of the operator's `any/` lines; for a custom
    /// perk (no lines of its own), one of `any/`. `seed` picks which, so
    /// every client can pick the same one.
    pub(crate) fn perk_quote(
        &self,
        operator: shared::operator::Operator,
        perk: shared::perks::Perk,
        seed: u64,
    ) -> Option<&SoundClip> {
        let quotes = self.perk_quotes.get(&operator)?;
        let own = quotes.by_perk.get(&perk).map_or(&[][..], |set| &set.clips[..]);
        let pool: Vec<&SoundClip> = own.iter().chain(&quotes.any.clips).collect();
        (!pool.is_empty()).then(|| pool[(seed % pool.len() as u64) as usize])
    }
}

/// One operator's perk quotes (`audio/quotes/<operator>/perk_quotes/`):
/// `any/`, and a folder per classic perk.
pub(crate) struct PerkQuotes {
    any: SoundSet,
    by_perk: std::collections::HashMap<shared::perks::Perk, SoundSet>,
}

impl PerkQuotes {
    fn load(asset_server: &AssetServer, operator: shared::operator::Operator) -> Self {
        use shared::perks::Perk;
        let dir = format!("audio/quotes/{}/perk_quotes", operator.dir());
        Self {
            any: SoundSet::load(asset_server, &format!("{dir}/any")),
            by_perk: [
                (Perk::Juggernog, "jug"),
                (Perk::QuickRevive, "quick_revive"),
                (Perk::SpeedCola, "speed_cola"),
                (Perk::StaminUp, "stamin_up"),
                (Perk::DoubleTap, "double_tap"),
                (Perk::DeadshotDaiquiri, "deadshot"),
                (Perk::PhdFlopper, "phd"),
                (Perk::DeathPerception, "death_perception"),
            ]
            .into_iter()
            .map(|(perk, sub)| (perk, SoundSet::load(asset_server, &format!("{dir}/{sub}"))))
            .collect(),
        }
    }
}

/// One clip from a [`SoundSet`].
pub(crate) struct SoundClip {
    pub(crate) handle: Handle<AudioSource>,
}

/// Every audio file (`.mp3` / `.wav` / `.ogg` / `.flac`) found in one folder
/// under the assets dir when the game started, sorted by name so the order is
/// the same on every client. The server picks which clip plays for an event
/// (`variant % clips.len()`), so the whole lobby hears the same one. Found by
/// listing the folder rather than hard-coding names, so dropping clips in / out
/// of it needs no code change.
#[derive(Default)]
pub(crate) struct SoundSet {
    pub(crate) clips: Vec<SoundClip>,
}

impl SoundSet {
    /// Load the folder `dir` (relative to the assets dir).
    fn load(asset_server: &AssetServer, dir: &str) -> Self {
        let path = bevy::asset::io::file::FileAssetReader::get_base_path()
            .join("assets")
            .join(dir);
        let mut names: Vec<String> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let ext = p.extension()?.to_str()?.to_ascii_lowercase();
                matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "flac")
                    .then(|| p.file_name()?.to_str().map(str::to_owned))
                    .flatten()
            })
            .collect();
        names.sort();
        if names.is_empty() {
            warn!("no sounds found in {}", path.display());
        }
        Self {
            clips: names
                .into_iter()
                .map(|file| SoundClip {
                    handle: asset_server.load(format!("{dir}/{file}")),
                })
                .collect(),
        }
    }

    /// The clip the server's random `variant` picks, if the set isn't empty.
    pub(crate) fn pick(&self, variant: u8) -> Option<&SoundClip> {
        (!self.clips.is_empty()).then(|| &self.clips[variant as usize % self.clips.len()])
    }
}

/// The knife sounds that live in folders: the throwing knife's surface
/// impacts, and the regular knife's stabs and swings.
#[derive(Resource)]
pub(crate) struct KnifeSounds {
    /// `audio/weapons/throwing_knife/impact/` — a thrown knife striking a surface.
    pub(crate) impact: SoundSet,
    /// `audio/weapons/knife/stab/` — a stab landing on a bot / player.
    pub(crate) stab: SoundSet,
    /// `audio/weapons/knife/swing/` — a stab that hits nothing.
    pub(crate) swing: SoundSet,
}

impl KnifeSounds {
    fn load(asset_server: &AssetServer) -> Self {
        Self {
            impact: SoundSet::load(asset_server, "audio/weapons/throwing_knife/impact"),
            stab: SoundSet::load(asset_server, "audio/weapons/knife/stab"),
            swing: SoundSet::load(asset_server, "audio/weapons/knife/swing"),
        }
    }
}

/// Linear volume shared by both ambience loops (`ambient` and
/// `shipment_ambient`) before their level in the mix.
pub(crate) const AMBIENT_VOLUME: f32 = 0.5;

/// The looping ambient-nature bed. `StateScoped(InGame)`, so it starts when the
/// player enters the world and stops on the way out.
#[derive(Component)]
pub(crate) struct AmbientAudio;

/// Linear volume of the main-menu / lobby music loop (before master volume).
pub(crate) const MENU_MUSIC_VOLUME: f32 = 0.5;

/// The looping main-menu music. Plays whenever we're not `InGame` (main menu
/// and lobby rooms) and is despawned as soon as a match starts — see
/// [`sync_menu_music`].
#[derive(Component)]
pub(crate) struct MenuMusic;

/// Playback settings for a sound emitted from a point in the world: panned
/// by where it is relative to the listener, and faded with distance by its
/// positional profile in the mix (`audio_engine`) — `volume` is its own
/// level on top, not a distance fade.
pub(crate) fn positional_playback(volume: Volume) -> PlaybackSettings {
    PlaybackSettings::DESPAWN.with_spatial(true).with_volume(volume)
}

pub(crate) fn setup_audio(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(KnifeSounds::load(&asset_server));
    commands.insert_resource(GameSounds {
        shot: asset_server.load("audio/weapons/sniper/shot.wav"),
        rechamber: asset_server.load("audio/weapons/sniper/rechamber.wav"),
        reload: asset_server.load("audio/weapons/sniper/reload.wav"),
        ak_shot: asset_server.load("audio/weapons/ak_74/shot.mp3"),
        ak_reload: asset_server.load("audio/weapons/ak_74/reload.mp3"),
        ak_reload_fast: asset_server.load("audio/weapons/ak_74/reload_fast.mp3"),
        raygun_shot: asset_server.load("audio/weapons/raygun/shot.mp3"),
        raygun_reload: asset_server.load("audio/weapons/raygun/reload.mp3"),
        raygun_equip: asset_server.load("audio/weapons/raygun/equip.mp3"),
        ambient: asset_server.load("audio/ambient/nature.ogg"),
        shipment_ambient: asset_server.load("audio/ambient/shipment.ogg"),
        aim_in: asset_server.load("audio/weapons/sniper/aim_in.mp3"),
        aim_out: asset_server.load("audio/weapons/sniper/aim_out.mp3"),
        out_of_ammo: asset_server.load("audio/weapons/sniper/out_of_ammo.mp3"),
        slide: asset_server.load("audio/movement/slide.mp3"),
        phd_slide: asset_server.load("audio/movement/phd_slider_slide.mp3"),
        dive: asset_server.load("audio/movement/dive.mp3"),
        kill_enemy: asset_server.load("audio/combat/kill_enemy.mp3"),
        jump_land: asset_server.load("audio/movement/jump_land.mp3"),
        teleport: asset_server.load("audio/movement/teleport.wav"),
        knife_throw: asset_server.load("audio/weapons/throwing_knife/throw.mp3"),
        knife_hit: asset_server.load("audio/weapons/throwing_knife/hit_enemy.mp3"),
        knife_in_air: asset_server.load("audio/weapons/throwing_knife/in_air.mp3"),
        knife_equip: asset_server.load("audio/weapons/knife/equip.mp3"),
        molotov_light: asset_server.load("audio/weapons/molotov/molotov_light.mp3"),
        molotov_burst: asset_server.load("audio/weapons/molotov/molotov_burst.mp3"),
        monkey_bomb_prime: asset_server.load("audio/weapons/monkey_bomb/monkey_bomb_prime.mp3"),
        monkey_bomb_throw: asset_server.load("audio/weapons/monkey_bomb/monkey_bomb_throw.mp3"),
        monkey_bomb_land: asset_server.load("audio/weapons/monkey_bomb/monkey_bomb_land.mp3"),
        monkey_bomb_song: asset_server.load("audio/weapons/monkey_bomb/monkey_bomb_song.mp3"),
        monkey_bomb_explode_vox: asset_server.load("audio/weapons/monkey_bomb/monkey_bomb_explode_vox.mp3"),
        frag_pin_pull: asset_server.load("audio/weapons/frag/frag_pin_pull.mp3"),
        frag_explosion: asset_server.load("audio/weapons/frag/frag_explosion.mp3"),
        flash_bang_detonate: asset_server.load("audio/weapons/flash_bang/flash_bang_detonate.mp3"),
        pick_up_equipment: asset_server.load("audio/weapons/pick_up_equipment.mp3"),
        sniper_equip: asset_server.load("audio/weapons/sniper/equip.mp3"),
        heartbeat: asset_server.load("audio/combat/heartbeat.mp3"),
        hit_marker: asset_server.load("audio/combat/hit_marker.mp3"),
        perk_buy: asset_server.load("audio/zombies/purchases/buy_perk.mp3"),
        pap_buy: asset_server.load("audio/zombies/purchases/buy_pap.mp3"),
        jingles: {
            use shared::perks::Perk;
            [
                (Perk::ShroomTea, "custom/shroom_tea.wav"),
                (Perk::NitroBrew, "custom/nitro_brew.wav"),
                (Perk::LiquidCourage, "custom/liquid_courage.wav"),
                (Perk::BombShot, "custom/bomb_shot.wav"),
                (Perk::Kangabrew, "custom/kangabrew.wav"),
                (Perk::Juggernog, "classic/jug.mp3"),
                (Perk::QuickRevive, "classic/quick_revive.mp3"),
                (Perk::SpeedCola, "classic/speed_cola.mp3"),
                (Perk::StaminUp, "classic/stamin_up.mp3"),
                (Perk::DoubleTap, "classic/double_tap.mp3"),
                (Perk::DeadshotDaiquiri, "classic/dead_shot.mp3"),
                (Perk::PhdFlopper, "classic/phd_slider.mp3"),
                (Perk::DeathPerception, "classic/death_perception.mp3"),
            ]
            .into_iter()
            .map(|(perk, file)| (perk, asset_server.load(format!("audio/zombies/perks/jingles/{file}"))))
            .collect()
        },
        perk_quotes: shared::operator::Operator::ALL
            .into_iter()
            .map(|op| (op, PerkQuotes::load(&asset_server, op)))
            .collect(),
        power_on: asset_server.load("audio/zombies/machines/power_on.mp3"),
        door_open: asset_server.load("audio/zombies/doors/open.mp3"),
        main_door_ambience: asset_server.load("audio/zombies/doors/main_door_ambience.mp3"),
        der_tempel_power_on: asset_server.load("audio/zombies/machines/der_tempel_power_on.wav"),
        mystery_box_open: asset_server.load("audio/zombies/mystery_box/open.mp3"),
        mystery_box_close: asset_server.load("audio/zombies/mystery_box/close.mp3"),
        mystery_box_spin: asset_server.load("audio/zombies/mystery_box/spin.mp3"),
        armor_destroy: asset_server.load("audio/zombies/armor/destroy.mp3"),
        aether_activate: asset_server.load("audio/zombies/aether_shroud/activate.mp3"),
        aether_active: asset_server.load("audio/zombies/aether_shroud/active.mp3"),
        aether_deactivate: asset_server.load("audio/zombies/aether_shroud/deactivate.mp3"),
        boss_spawn: asset_server.load("audio/zombies/boss/spawn.mp3"),
        boss_growl: asset_server.load("audio/zombies/boss/growl.mp3"),
        boss_attack: asset_server.load("audio/zombies/boss/normal_attack.mp3"),
        boss_death: asset_server.load("audio/zombies/boss/death.mp3"),
        boss_blast: asset_server.load("audio/zombies/boss/blast.mp3"),
        boss_blast_hit: asset_server.load("audio/zombies/boss/blast_hit.mp3"),
        machine_hum: asset_server.load("audio/zombies/machines/pap_buzzing.mp3"),
        fire_crackle: asset_server.load("audio/ambient/fire_crackle.mp3"),
        round_start: asset_server.load("audio/zombies/rounds/round_start.wav"),
        dog_round_start: asset_server.load("audio/zombies/rounds/dog_round_start.mp3"),
        dog_round_end: asset_server.load("audio/zombies/rounds/dog_round_end.mp3"),
        dog_pre_spawn: asset_server.load("audio/zombies/dogs/pre_spawn.mp3"),
        dog_spawn: asset_server.load("audio/zombies/dogs/spawn.mp3"),
        dog_bark: asset_server.load("audio/zombies/dogs/bark.mp3"),
        dog_explosion: asset_server.load("audio/zombies/dogs/explosion.mp3"),
        player_down: asset_server.load("audio/zombies/revive/player_down.mp3"),
        revived: asset_server.load("audio/zombies/revive/revived.mp3"),
        bomb_shot_explosions: (1..=3)
            .map(|i| asset_server.load(format!("audio/zombies/perks/bomb_shot_explosions/bomb_shot_explosion_{i}.wav")))
            .collect(),
        footsteps: (1..=FOOTSTEP_CLIPS)
            .map(|i| asset_server.load(format!("audio/movement/footsteps/footstep_{i}.wav")))
            .collect(),
        menu_music: asset_server.load("audio/music/main_menu.wav"),
        zombies_ambient: asset_server.load("audio/ambient/zombies.mp3"),
        buy_ammo: asset_server.load("audio/zombies/purchases/buy_ammo.mp3"),
        money_ching: asset_server.load("audio/zombies/purchases/money_ching.mp3"),
        power_up_grab: asset_server.load("audio/zombies/power_ups/grab.mp3"),
        power_up_loop: asset_server.load("audio/zombies/power_ups/drop_loop.mp3"),
        power_up_max_ammo: asset_server.load("audio/zombies/power_ups/max_ammo.mp3"),
        power_up_insta_kill: asset_server.load("audio/zombies/power_ups/insta_kill.mp3"),
        power_up_double_points: asset_server.load("audio/zombies/power_ups/double_points.mp3"),
        power_up_nuke: asset_server.load("audio/zombies/power_ups/nuke.mp3"),
        power_up_bonus_points: asset_server.load("audio/zombies/power_ups/bonus_points.mp3"),
        zombies_game_over: asset_server.load("audio/music/zombies_game_over.mp3"),
        board_snaps: (1..=5)
            .map(|i| asset_server.load(format!("audio/zombies/windows/board_snap_{i}.mp3")))
            .collect(),
        board_slams: (1..=6)
            .map(|i| asset_server.load(format!("audio/zombies/windows/board_slam_{i}.mp3")))
            .collect(),
        board_float: asset_server.load("audio/zombies/windows/board_float.mp3"),
        hammering: asset_server.load("audio/zombies/windows/hammering.mp3"),
    });
}

/// Start the map's looping ambience bed when the player enters the world —
/// `basic_map.glb`'s outdoor-nature loop, or `shipment.glb`'s own cargo-ship
/// one, by [`CurrentMap`]. Ordered `.after(lobby_ui::sync_current_map)`
/// (both run on `OnEnter(AppState::InGame)`) so this always sees the map
/// just selected, not whatever `CurrentMap` was left at after the previous
/// match. Not in `Zombies`, on any map: its own ambience
/// (`zombies_audio`) is the only one there.
pub(crate) fn start_ambient(
    mut commands: Commands,
    sounds: Res<GameSounds>,
    current: Res<CurrentMap>,
    local: Query<&lightyear::prelude::LocalId, With<crate::net::GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    if crate::zombies_hud::my_lobby(&local, &lobbies).is_some_and(|l| l.mode == shared::GameMode::Zombies) {
        return;
    }
    let clip = match current.0 {
        map if map.is_shipment() => sounds.shipment_ambient.clone(),
        _ => sounds.ambient.clone(),
    };
    commands.spawn((
        AmbientAudio,
        StateScoped(AppState::InGame),
        AudioPlayer::new(clip),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(AMBIENT_VOLUME)),
    ));
}

/// Keep the menu music loop playing on the main menu and in lobbies, and
/// silent in a match: spawned the first time we're outside `InGame` with none
/// playing (game launch, or leaving a match / lobby back to the menu), and
/// despawned on entering `InGame`. Moving between `MainMenu` and `InLobby`
/// leaves the one loop running rather than restarting it.
pub(crate) fn sync_menu_music(
    mut commands: Commands,
    state: Res<State<AppState>>,
    sounds: Res<GameSounds>,
    settings: Res<Settings>,
    music: Query<Entity, With<MenuMusic>>,
) {
    if matches!(state.get(), AppState::InGame | AppState::LevelEditor) {
        for e in &music {
            commands.entity(e).despawn();
        }
    } else if music.is_empty() {
        commands.spawn((
            MenuMusic,
            AudioPlayer::new(sounds.menu_music.clone()),
            PlaybackSettings::LOOP
                .with_volume(Volume::Linear(MENU_MUSIC_VOLUME * settings.music_volume)),
        ));
    }
}

/// Push `Settings::master_volume` onto `GlobalVolume` — applied live to
/// every sound, playing or not — and `Settings::music_volume` onto the menu
/// music loop.
pub(crate) fn apply_master_volume(
    settings: Res<Settings>,
    mut global_volume: ResMut<GlobalVolume>,
    mut music: Query<&mut AudioSink, With<MenuMusic>>,
) {
    if !settings.is_changed() {
        return;
    }
    global_volume.volume = Volume::Linear(settings.master_volume);
    for mut sink in &mut music {
        sink.set_volume(Volume::Linear(MENU_MUSIC_VOLUME * settings.music_volume));
    }
}
