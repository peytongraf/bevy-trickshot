//! Preloaded sound handles, per-category volume multipliers, and the
//! looping ambience bed — plus pushing `Settings::master_volume` and the
//! "Sound volumes" panel onto whatever's actually playing.
//!
//! `assets/audio/` layout: one folder per category — `ambient/`, `combat/`,
//! `movement/` (+ `footsteps/`), `music/`, `quotes/<operator>/`, `ui/`,
//! `weapons/<weapon>/`, `zombies/`, and `unused/` for clips not wired up yet
//! (operator quotes move from `unused/quotes/` to `quotes/` as they're used). Files are `snake_case`, named for
//! what they are within their folder (no `-sound` suffix, no repeating the
//! folder's name: `weapons/sniper/shot.wav`), with variations numbered
//! `name_1`, `name_2`, …

use bevy::audio::Volume;
use bevy::prelude::*;

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
    /// `audio/zombies/machines/pap_buzzing.mp3` — the electric hum looped from every
    /// perk machine and the Pack-a-Punch once the power's on
    /// (`power::sync_machine_hums`).
    pub(crate) machine_hum: Handle<AudioSource>,
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
            any: SoundSet::load(asset_server, &format!("{dir}/any"), 1.0),
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
            .map(|(perk, sub)| (perk, SoundSet::load(asset_server, &format!("{dir}/{sub}"), 1.0)))
            .collect(),
        }
    }
}

/// One clip from a [`SoundSet`], with its own volume multiplier.
pub(crate) struct SoundClip {
    /// File name without the extension — the label in the debug panel.
    pub(crate) name: String,
    pub(crate) handle: Handle<AudioSource>,
    /// Multiplier on the clip's built-in level ("Sound volumes" panel).
    pub(crate) volume: f32,
}

/// Every audio file (`.mp3` / `.wav` / `.ogg` / `.flac`) found in one folder
/// under the assets dir when the game started, sorted by name so the order is
/// the same on every client. The server picks which clip plays for an event
/// (`variant % clips.len()`), so the whole lobby hears the same one. Found by
/// listing the folder rather than hard-coding names, so dropping clips in / out
/// of it needs no code change; each clip gets its own slider.
#[derive(Default)]
pub(crate) struct SoundSet {
    pub(crate) clips: Vec<SoundClip>,
}

impl SoundSet {
    /// Load the folder `dir` (relative to the assets dir), every clip starting
    /// at `volume`.
    fn load(asset_server: &AssetServer, dir: &str, volume: f32) -> Self {
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
                    name: file
                        .rsplit_once('.')
                        .map_or(file.clone(), |(stem, _)| stem.to_owned()),
                    handle: asset_server.load(format!("{dir}/{file}")),
                    volume,
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
            impact: SoundSet::load(asset_server, "audio/weapons/throwing_knife/impact", 0.3),
            stab: SoundSet::load(asset_server, "audio/weapons/knife/stab", 1.0),
            swing: SoundSet::load(asset_server, "audio/weapons/knife/swing", 1.0),
        }
    }
}

/// Linear volume shared by both ambience loops (`ambient` and
/// `shipment_ambient`) before their own "Sound volumes" multiplier.
pub(crate) const AMBIENT_VOLUME: f32 = 0.5;

/// Panel-adjustable per-sound volume multipliers ("Sound volumes" panel
/// section). `1.0` leaves a sound at its built-in level; every one-shot is
/// scaled by its entry when it spawns (`apply_sound_volumes`), and whichever
/// ambience loop is currently playing by `ambient` / `shipment_ambient`
/// (folded into `apply_master_volume`). Footsteps have their own controls in
/// the "Footsteps" section and aren't here.
#[derive(Resource)]
pub(crate) struct SoundVolumes {
    pub(crate) shot: f32,
    pub(crate) rechamber: f32,
    pub(crate) reload: f32,
    pub(crate) ak_shot: f32,
    pub(crate) ak_reload: f32,
    pub(crate) ak_reload_fast: f32,
    pub(crate) raygun_shot: f32,
    pub(crate) raygun_reload: f32,
    pub(crate) raygun_equip: f32,
    pub(crate) ambient: f32,
    pub(crate) shipment_ambient: f32,
    pub(crate) aim_in: f32,
    pub(crate) aim_out: f32,
    pub(crate) out_of_ammo: f32,
    pub(crate) slide: f32,
    pub(crate) dive: f32,
    pub(crate) kill_enemy: f32,
    pub(crate) jump_land: f32,
    pub(crate) teleport: f32,
    pub(crate) knife_throw: f32,
    pub(crate) knife_hit: f32,
    pub(crate) knife_in_air: f32,
    pub(crate) knife_equip: f32,
    pub(crate) molotov_light: f32,
    pub(crate) molotov_burst: f32,
    pub(crate) pick_up_equipment: f32,
    pub(crate) sniper_equip: f32,
    /// Loudness of the heartbeat at zero health (it fades toward silence as
    /// health recovers) — see `health::update_heartbeat`.
    pub(crate) heartbeat: f32,
    pub(crate) hit_marker: f32,
    pub(crate) perk_buy: f32,
    /// Every perk machine's jingle (on top of its distance fade).
    pub(crate) perk_jingle: f32,
    /// An operator's line after buying a perk (another player's on top of
    /// its distance fade).
    pub(crate) perk_quote: f32,
    /// The power lever being thrown (on top of its distance fade).
    pub(crate) power_on: f32,
    pub(crate) round_start: f32,
    pub(crate) dog_round_start: f32,
    pub(crate) dog_round_end: f32,
    /// A hellhound's sounds (`dogs`), each on top of its distance fade.
    pub(crate) dog_pre_spawn: f32,
    pub(crate) dog_spawn: f32,
    pub(crate) dog_bark: f32,
    pub(crate) dog_explosion: f32,
    /// A lobby member going down, and a revive finishing (`revive`).
    pub(crate) player_down: f32,
    pub(crate) revived: f32,
    /// A Bomb Shot explosion (on top of its distance fade).
    pub(crate) bomb_shot_explosion: f32,
    /// Zombie voices (`zombie_sounds`), each on top of its distance fade.
    pub(crate) zombie_moan: f32,
    pub(crate) zombie_spawn: f32,
    pub(crate) zombie_death: f32,
    pub(crate) zombie_attack: f32,
    pub(crate) final_zombie: f32,
    /// The `Zombies` ambience loop and game-over music (`zombies_audio`).
    pub(crate) zombies_ambient: f32,
    pub(crate) buy_ammo: f32,
    pub(crate) money_ching: f32,
    pub(crate) power_up_grab: f32,
    /// A dropped power-up's loop (on top of its distance fade).
    pub(crate) power_up_loop: f32,
    /// Every power-up's announcer line.
    pub(crate) power_up_announcer: f32,
    pub(crate) zombies_game_over: f32,
}

impl Default for SoundVolumes {
    fn default() -> Self {
        Self {
            shot: 1.0,
            rechamber: 1.0,
            reload: 1.5,
            ak_shot: 1.0,
            ak_reload: 1.0,
            ak_reload_fast: 1.0,
            raygun_shot: 1.0,
            raygun_reload: 1.0,
            raygun_equip: 1.0,
            ambient: 2.5,
            shipment_ambient: 2.5,
            aim_in: 1.0,
            aim_out: 1.0,
            out_of_ammo: 1.0,
            slide: 1.0,
            dive: 1.0,
            kill_enemy: 3.0,
            jump_land: 0.5,
            teleport: 1.0,
            knife_throw: 1.0,
            knife_hit: 1.5,
            knife_in_air: 1.0,
            knife_equip: 1.0,
            molotov_light: 1.0,
            molotov_burst: 1.5,
            pick_up_equipment: 1.0,
            sniper_equip: 1.0,
            heartbeat: 1.0,
            hit_marker: 1.0,
            perk_buy: 1.0,
            perk_jingle: 1.0,
            perk_quote: 1.0,
            power_on: 1.0,
            round_start: 4.0,
            dog_round_start: 1.0,
            dog_round_end: 1.0,
            dog_pre_spawn: 1.0,
            dog_spawn: 1.0,
            dog_bark: 0.6,
            dog_explosion: 1.0,
            player_down: 1.0,
            revived: 1.0,
            bomb_shot_explosion: 1.0,
            zombie_moan: 1.0,
            zombie_spawn: 1.0,
            zombie_death: 1.0,
            zombie_attack: 1.0,
            final_zombie: 1.0,
            zombies_ambient: 1.0,
            buy_ammo: 1.0,
            money_ching: 1.0,
            power_up_grab: 1.0,
            power_up_loop: 1.0,
            power_up_announcer: 1.0,
            zombies_game_over: 1.0,
        }
    }
}

impl SoundVolumes {
    /// The multiplier for `handle`, or `None` if it isn't a one-shot this
    /// resource covers (footstep clips, the ambient loop).
    pub(crate) fn oneshot_for(
        &self,
        handle: &Handle<AudioSource>,
        sounds: &GameSounds,
    ) -> Option<f32> {
        let id = handle.id();
        [
            (sounds.shot.id(), self.shot),
            (sounds.rechamber.id(), self.rechamber),
            (sounds.reload.id(), self.reload),
            (sounds.ak_shot.id(), self.ak_shot),
            (sounds.ak_reload.id(), self.ak_reload),
            (sounds.ak_reload_fast.id(), self.ak_reload_fast),
            (sounds.raygun_shot.id(), self.raygun_shot),
            (sounds.raygun_reload.id(), self.raygun_reload),
            (sounds.raygun_equip.id(), self.raygun_equip),
            (sounds.aim_in.id(), self.aim_in),
            (sounds.aim_out.id(), self.aim_out),
            (sounds.out_of_ammo.id(), self.out_of_ammo),
            (sounds.slide.id(), self.slide),
            (sounds.phd_slide.id(), self.slide),
            (sounds.dive.id(), self.dive),
            (sounds.kill_enemy.id(), self.kill_enemy),
            (sounds.jump_land.id(), self.jump_land),
            (sounds.teleport.id(), self.teleport),
            (sounds.knife_throw.id(), self.knife_throw),
            (sounds.knife_hit.id(), self.knife_hit),
            (sounds.knife_in_air.id(), self.knife_in_air),
            (sounds.knife_equip.id(), self.knife_equip),
            (sounds.molotov_light.id(), self.molotov_light),
            (sounds.pick_up_equipment.id(), self.pick_up_equipment),
            (sounds.sniper_equip.id(), self.sniper_equip),
            (sounds.hit_marker.id(), self.hit_marker),
            (sounds.perk_buy.id(), self.perk_buy),
            (sounds.buy_ammo.id(), self.buy_ammo),
            (sounds.money_ching.id(), self.money_ching),
            (sounds.power_up_grab.id(), self.power_up_grab),
            (sounds.power_up_max_ammo.id(), self.power_up_announcer),
            (sounds.power_up_insta_kill.id(), self.power_up_announcer),
            (sounds.power_up_double_points.id(), self.power_up_announcer),
            (sounds.power_up_nuke.id(), self.power_up_announcer),
            (sounds.power_up_bonus_points.id(), self.power_up_announcer),
            (sounds.round_start.id(), self.round_start),
            (sounds.dog_round_start.id(), self.dog_round_start),
            (sounds.dog_round_end.id(), self.dog_round_end),
            (sounds.player_down.id(), self.player_down),
            (sounds.revived.id(), self.revived),
        ]
        .into_iter()
        .find_map(|(hid, vol)| (hid == id).then_some(vol))
    }
}

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

/// Tags a sound spawned by `net::receive_remote_sounds` for another player's
/// action (reload, footstep, shot, ...), positioned at their world location.
/// `apply_sound_volumes` skips these — their volume is already fully baked in
/// at spawn time (category volume × distance falloff × the "Remote sounds"
/// panel's master volume × global volume), since re-deriving it post-spawn
/// would need the same distance calculation all over again for no benefit.
#[derive(Component)]
pub(crate) struct RemoteSoundEmitter;

/// Distance falloff for other players' positional sounds ("Remote sounds"
/// debug-panel section). Applied by `net::receive_remote_sounds` on top of
/// this same clip's normal `SoundVolumes` category multiplier.
#[derive(Resource, Clone, Copy)]
pub(crate) struct RemoteSoundSettings {
    /// Overall gain on every remote-player sound, on top of its usual
    /// per-category volume.
    pub(crate) volume: f32,
    /// Distance (m) at which a remote sound has faded to silence.
    pub(crate) max_distance: f32,
    /// Per-sound gain for another player's (or bot's) actions — one per
    /// `killcam::SND_*` bit that's played remotely (not aim in / out), on top of `volume` and the clip's `SoundVolumes`
    /// entry. See [`Self::for_bit`].
    pub(crate) shot: f32,
    pub(crate) reload: f32,
    pub(crate) rechamber: f32,
    pub(crate) ak_shot: f32,
    pub(crate) ak_reload: f32,
    pub(crate) ak_reload_fast: f32,
    pub(crate) raygun_shot: f32,
    pub(crate) raygun_reload: f32,
    pub(crate) slide: f32,
    pub(crate) dive: f32,
    pub(crate) footstep: f32,
    pub(crate) jump_land: f32,
    pub(crate) knife_throw: f32,
}

impl Default for RemoteSoundSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            max_distance: 60.0,
            shot: 1.0,
            reload: 1.0,
            rechamber: 0.05,
            ak_shot: 1.0,
            ak_reload: 1.0,
            ak_reload_fast: 1.0,
            raygun_shot: 1.0,
            raygun_reload: 1.0,
            slide: 1.0,
            dive: 1.0,
            footstep: 1.0,
            jump_land: 1.0,
            knife_throw: 1.0,
        }
    }
}

impl RemoteSoundSettings {
    /// The per-sound gain field for a `killcam::SND_*` bit, with its panel
    /// label — `None` for a bit that isn't played remotely.
    pub(crate) fn field_mut(&mut self, bit: u16) -> Option<(&'static str, &mut f32)> {
        use crate::killcam::*;
        Some(match bit {
            SND_SHOT => ("shot", &mut self.shot),
            SND_RELOAD => ("reload", &mut self.reload),
            SND_RECHAMBER => ("rechamber", &mut self.rechamber),
            SND_AK_SHOT => ("AK-74 shot", &mut self.ak_shot),
            SND_AK_RELOAD => ("AK-74 reload", &mut self.ak_reload),
            SND_AK_RELOAD_FAST => ("AK-74 fast reload", &mut self.ak_reload_fast),
            SND_RAYGUN_SHOT => ("Ray Gun shot", &mut self.raygun_shot),
            SND_RAYGUN_RELOAD => ("Ray Gun reload", &mut self.raygun_reload),
            SND_SLIDE => ("slide", &mut self.slide),
            SND_DIVE => ("dive", &mut self.dive),
            SND_FOOTSTEP => ("footstep", &mut self.footstep),
            SND_JUMP_LAND => ("jump land", &mut self.jump_land),
            SND_THROW => ("knife throw", &mut self.knife_throw),
            _ => return None,
        })
    }

    /// The per-sound gain for a `killcam::SND_*` bit (`1.0` for an unknown one).
    pub(crate) fn for_bit(&self, bit: u16) -> f32 {
        // (PhD Flopper's slide goes by the slide's.)
        let bit = if bit == crate::killcam::SND_PHD_SLIDE {
            crate::killcam::SND_SLIDE
        } else {
            bit
        };
        let mut copy = *self;
        copy.field_mut(bit).map_or(1.0, |(_, v)| *v)
    }
}

/// Playback settings for a sound emitted from a point in the world: spatial
/// (panned by where it is relative to the listener) with rodio's own
/// `1 / distance²` curve shrunk to near-flat, so the loudness-by-distance is
/// whatever `volume` already bakes in — see `net::receive_remote_sounds`, which
/// explains why.
pub(crate) fn positional_playback(volume: Volume) -> PlaybackSettings {
    PlaybackSettings::DESPAWN
        .with_spatial(true)
        .with_spatial_scale(bevy::audio::SpatialScale::new(0.01))
        .with_volume(volume)
}

/// The `[0, 1]` loudness factor for a remote sound `distance` metres from the
/// listener: a squared linear fade to silence at
/// [`RemoteSoundSettings::max_distance`].
pub(crate) fn distance_falloff(distance: f32, remote: &RemoteSoundSettings) -> f32 {
    if distance >= remote.max_distance {
        0.0
    } else {
        (1.0 - distance / remote.max_distance).powi(2)
    }
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
        machine_hum: asset_server.load("audio/zombies/machines/pap_buzzing.mp3"),
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
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    current: Res<CurrentMap>,
    local: Query<&lightyear::prelude::LocalId, With<crate::net::GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    if crate::zombies_hud::my_lobby(&local, &lobbies).is_some_and(|l| l.mode == shared::GameMode::Zombies) {
        return;
    }
    let (clip, volume_mult) = match current.0 {
        shared::MapId::BasicMap | shared::MapId::BreakPoint | shared::MapId::BreakPointNight
        | shared::MapId::AshesOfTheDamned => {
            (sounds.ambient.clone(), vols.ambient)
        }
        shared::MapId::Shipment | shared::MapId::ShipmentDay => {
            (sounds.shipment_ambient.clone(), vols.shipment_ambient)
        }
    };
    commands.spawn((
        AmbientAudio,
        StateScoped(AppState::InGame),
        AudioPlayer::new(clip),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(
            AMBIENT_VOLUME * volume_mult * settings.master_volume,
        )),
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
            // `GlobalVolume` (master volume) is multiplied in at spawn.
            PlaybackSettings::LOOP
                .with_volume(Volume::Linear(MENU_MUSIC_VOLUME * settings.music_volume)),
        ));
    }
}

/// Push `Settings::master_volume` onto Bevy's `GlobalVolume`, which scales
/// every one-shot sound spawned from here on (shots, footsteps, UI, ...) with
/// no per-call-site changes needed. `GlobalVolume` doesn't retroactively touch
/// audio that's already playing, though, so the looping ambience needs its own
/// direct nudge here too — also picking up the "Sound volumes" panel's
/// multiplier for whichever ambience loop `start_ambient` actually started —
/// and the menu music loop likewise (with `Settings::music_volume` on top).
pub(crate) fn apply_master_volume(
    settings: Res<Settings>,
    vols: Res<SoundVolumes>,
    current: Res<CurrentMap>,
    mut global_volume: ResMut<GlobalVolume>,
    mut ambient: Query<&mut AudioSink, With<AmbientAudio>>,
    mut music: Query<&mut AudioSink, (With<MenuMusic>, Without<AmbientAudio>)>,
) {
    if !settings.is_changed() && !vols.is_changed() {
        return;
    }
    global_volume.volume = Volume::Linear(settings.master_volume);
    let ambient_mult = match current.0 {
        shared::MapId::BasicMap | shared::MapId::BreakPoint | shared::MapId::BreakPointNight
        | shared::MapId::AshesOfTheDamned => vols.ambient,
        shared::MapId::Shipment | shared::MapId::ShipmentDay => vols.shipment_ambient,
    };
    for mut sink in &mut ambient {
        sink.set_volume(Volume::Linear(
            AMBIENT_VOLUME * ambient_mult * settings.master_volume,
        ));
    }
    for mut sink in &mut music {
        sink.set_volume(Volume::Linear(
            MENU_MUSIC_VOLUME * settings.music_volume * settings.master_volume,
        ));
    }
}

/// Scale each freshly-started one-shot to its "Sound volumes" multiplier.
/// bevy_audio bakes `PlaybackSettings::volume * GlobalVolume` into the sink when
/// it starts it, so we re-derive the same product with the per-sound factor
/// mixed in. Sounds not covered here (footsteps, the ambient loop) are left be.
pub(crate) fn apply_sound_volumes(
    sounds: Option<Res<GameSounds>>,
    vols: Res<SoundVolumes>,
    global_volume: Res<GlobalVolume>,
    mut fresh: Query<
        (&AudioPlayer, &mut AudioSink),
        (Added<AudioSink>, Without<RemoteSoundEmitter>),
    >,
) {
    let Some(sounds) = sounds else { return };
    for (player, mut sink) in &mut fresh {
        if let Some(mult) = vols.oneshot_for(&player.0, &sounds) {
            sink.set_volume(Volume::Linear(mult.max(0.0)) * global_volume.volume);
        }
    }
}
