//! The easter egg gone wrong (`shared::easter_egg::EggStep::is_unleashed`,
//! from the ritual going wrong until the Hierophant's dead): Der Tempel
//! turns on the squad — its fires burn green, its water runs red, the sky
//! darkens, the fog closes in, and every light in the place stutters.
//!
//! [`EggMoodFade`] is how far in it is (0..1), faded in by
//! [`update_egg_mood`] and back out once it's over. Everything that changes
//! reads it, on top of the map's own look (and a dog round's, under it):
//! `apply_scene_tuning` (fog, sun, ambient), `sky::apply_sky_look`,
//! `ground_fog::sync_ground_fog`, `flames` (the fires' colour and light) and
//! `water` (pools and waterfalls).
//!
//! The lights stutter **in unison**: the whole temple flickers as one, like
//! something huge breathing on its power — never the gentle, offset wobble
//! of ordinary torches, which already flicker that way. [`LightFlicker`] is
//! this frame's multiplier (exactly 1 between stutters, so a steady frame
//! costs nothing); [`flicker_lights`] puts it on every light in the world
//! but the players' flashlights, after whatever owns each light has set its
//! brightness, and [`unflicker_lights`] puts each back before they run
//! again — so it works over any light without each one knowing. The fires'
//! flames and the power lights' bulbs dim with it too (`flames`, `power`).
//!
//! **Enraged** (the Hierophant's phase 3): it all turns blood-red — fog,
//! sky, ambient, the low fog and the fires — faded in by [`EggRageFade`]
//! over the rest. What reads the look reads [`EggMoodNow`]: the panel's
//! [`EggMoodLook`] with that blended in (rebuilt only when either moves).
//!
//! Tuned in the debug panel's "Easter egg (Der Tempel)" → "It goes wrong —
//! the map". Out of a game the fades and flicker drop straight back to
//! nothing, so none of it carries into the next one.

use bevy::prelude::*;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::Lobby;

use crate::net::GameClient;
use crate::AppState;

pub(crate) struct EggMoodPlugin;

impl Plugin for EggMoodPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EggMoodLook>()
            .init_resource::<EggMoodNow>()
            .init_resource::<EggMoodFade>()
            .init_resource::<EggRageFade>()
            .init_resource::<LightFlicker>()
            .init_resource::<EggLightCue>()
            .add_systems(First, unflicker_lights)
            .add_systems(Last, flicker_lights);
    }
}

/// Panel-tunable look of it gone wrong.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct EggMoodLook {
    pub(crate) enabled: bool,
    /// Debug: act as if it's gone wrong (untick to fade back out).
    pub(crate) preview: bool,
    /// Seconds to fade in as it goes wrong, and out once he's dead.
    pub(crate) fade_in_secs: f32,
    pub(crate) fade_out_secs: f32,

    // --- the sky and the fog ---
    /// The sky's brightness as a multiple of the map's own...
    pub(crate) sky_mult: f32,
    /// ...tinted (sRGB multiplier)...
    pub(crate) sky_tint: [f32; 3],
    /// ...under a haze in [`Self::fog_color`] (0 = none, 1 = solid).
    pub(crate) sky_haze: f32,
    /// How far (m) you can see through the fog — never clearer than the
    /// map's own.
    pub(crate) fog_visibility_m: f32,
    pub(crate) fog_color: [f32; 3],
    /// The sun's and the ambient light's strength, as a multiple of the
    /// map's own, and the ambient's colour.
    pub(crate) sun_mult: f32,
    pub(crate) ambient_mult: f32,
    pub(crate) ambient_color: [f32; 3],
    /// The low fog over the floor: at least this thick (per m) and opaque,
    /// thinning out over this height (m), in this colour.
    pub(crate) ground_fog_density: f32,
    pub(crate) ground_fog_opacity: f32,
    pub(crate) ground_fog_falloff: f32,
    pub(crate) ground_fog_color: [f32; 3],

    // --- the fires ---
    /// The flames' colours, hottest to their tips.
    pub(crate) fire_core: [f32; 3],
    pub(crate) fire_mid: [f32; 3],
    pub(crate) fire_tip: [f32; 3],
    /// Their light's colour.
    pub(crate) fire_light: [f32; 3],

    // --- the water ---
    /// What a pool tints what's seen through it (sRGB multiplier)...
    pub(crate) water_tint: [f32; 3],
    /// ...the colour it takes on deeper down, and how far (m) light gets
    /// before it has — small, for thick blood.
    pub(crate) water_deep: [f32; 3],
    pub(crate) water_clarity: f32,
    /// A waterfall's colour (sRGB multiplier).
    pub(crate) waterfall_tint: [f32; 3],

    // --- the lights' stutter ---
    pub(crate) flicker: bool,
    /// The gap (s) between one stutter and the next — random, in this range.
    pub(crate) flicker_gap: (f32, f32),
    /// How long (s) one lasts — random, in this range.
    pub(crate) flicker_len: (f32, f32),
    /// How many times a second, during one, the lights can cut in or out.
    pub(crate) flicker_rate_hz: f32,
    /// How likely each of those is to be out (0..1)...
    pub(crate) flicker_off_chance: f32,
    /// ...and how dark (a multiple of their brightness) they go out to.
    pub(crate) flicker_low: f32,
    /// A long stutter right as it goes wrong (s).
    pub(crate) onset_secs: f32,

    // --- enraged (the Hierophant's phase 3), over the rest ---
    /// Debug: act as if he's enraged (with `preview`, or in the fight).
    pub(crate) rage_preview: bool,
    /// Seconds to turn red as he's enraged, and back once he isn't.
    pub(crate) rage_fade_in_secs: f32,
    pub(crate) rage_fade_out_secs: f32,
    pub(crate) rage_sky_mult: f32,
    pub(crate) rage_sky_tint: [f32; 3],
    pub(crate) rage_sky_haze: f32,
    pub(crate) rage_fog_visibility_m: f32,
    pub(crate) rage_fog_color: [f32; 3],
    pub(crate) rage_ambient_mult: f32,
    pub(crate) rage_ambient_color: [f32; 3],
    pub(crate) rage_ground_fog_density: f32,
    pub(crate) rage_ground_fog_opacity: f32,
    pub(crate) rage_ground_fog_color: [f32; 3],
    pub(crate) rage_fire_core: [f32; 3],
    pub(crate) rage_fire_mid: [f32; 3],
    pub(crate) rage_fire_tip: [f32; 3],
    pub(crate) rage_fire_light: [f32; 3],
}

impl Default for EggMoodLook {
    fn default() -> Self {
        Self {
            enabled: true,
            preview: false,
            fade_in_secs: 2.5,
            fade_out_secs: 6.0,
            sky_mult: 0.25,
            sky_tint: [0.6, 1.0, 0.65],
            sky_haze: 0.6,
            fog_visibility_m: 45.0,
            fog_color: [0.02, 0.045, 0.028],
            sun_mult: 0.2,
            ambient_mult: 0.7,
            ambient_color: [0.55, 0.85, 0.6],
            ground_fog_density: 0.14,
            ground_fog_opacity: 0.7,
            ground_fog_falloff: 3.0,
            ground_fog_color: [0.07, 0.14, 0.09],
            fire_core: [0.85, 1.0, 0.7],
            fire_mid: [0.15, 0.95, 0.25],
            fire_tip: [0.0, 0.35, 0.08],
            fire_light: [0.35, 1.0, 0.42],
            water_tint: [0.55, 0.02, 0.02],
            water_deep: [0.35, 0.0, 0.0],
            water_clarity: 0.12,
            waterfall_tint: [0.6, 0.04, 0.04],
            flicker: true,
            flicker_gap: (3.0, 9.0),
            flicker_len: (0.25, 1.3),
            flicker_rate_hz: 14.0,
            flicker_off_chance: 0.55,
            flicker_low: 0.04,
            onset_secs: 2.2,
            rage_preview: false,
            rage_fade_in_secs: 3.0,
            rage_fade_out_secs: 4.0,
            rage_sky_mult: 0.2,
            rage_sky_tint: [1.0, 0.25, 0.2],
            rage_sky_haze: 0.75,
            rage_fog_visibility_m: 32.0,
            rage_fog_color: [0.09, 0.008, 0.006],
            rage_ambient_mult: 0.75,
            rage_ambient_color: [1.0, 0.3, 0.26],
            rage_ground_fog_density: 0.2,
            rage_ground_fog_opacity: 0.8,
            rage_ground_fog_color: [0.2, 0.02, 0.015],
            rage_fire_core: [1.0, 0.8, 0.6],
            rage_fire_mid: [0.95, 0.12, 0.04],
            rage_fire_tip: [0.35, 0.0, 0.0],
            rage_fire_light: [1.0, 0.18, 0.08],
        }
    }
}

impl EggMoodLook {
    /// This look with enraged's blended in by `r` (0..=1).
    fn enraged(&self, r: f32) -> Self {
        if r <= 0.0 {
            return self.clone();
        }
        let f = |own: f32, to: f32| own + (to - own) * r;
        Self {
            sky_mult: f(self.sky_mult, self.rage_sky_mult),
            sky_tint: blend3(self.sky_tint, self.rage_sky_tint, r),
            sky_haze: f(self.sky_haze, self.rage_sky_haze),
            fog_visibility_m: f(self.fog_visibility_m, self.rage_fog_visibility_m),
            fog_color: blend3(self.fog_color, self.rage_fog_color, r),
            ambient_mult: f(self.ambient_mult, self.rage_ambient_mult),
            ambient_color: blend3(self.ambient_color, self.rage_ambient_color, r),
            ground_fog_density: f(self.ground_fog_density, self.rage_ground_fog_density),
            ground_fog_opacity: f(self.ground_fog_opacity, self.rage_ground_fog_opacity),
            ground_fog_color: blend3(self.ground_fog_color, self.rage_ground_fog_color, r),
            fire_core: blend3(self.fire_core, self.rage_fire_core, r),
            fire_mid: blend3(self.fire_mid, self.rage_fire_mid, r),
            fire_tip: blend3(self.fire_tip, self.rage_fire_tip, r),
            fire_light: blend3(self.fire_light, self.rage_fire_light, r),
            ..self.clone()
        }
    }
}

/// The look as it is right now — [`EggMoodLook`] with [`EggRageFade`]
/// blended in — for everything that reads it (as an `EggMoodLook`).
#[derive(Resource, Default)]
pub(crate) struct EggMoodNow(EggMoodLook);

impl std::ops::Deref for EggMoodNow {
    type Target = EggMoodLook;
    fn deref(&self) -> &EggMoodLook {
        &self.0
    }
}

/// How far he's turned it blood-red (his phase 3), 0..=1 (smoothed) — his
/// own aura and eyes follow it too (`easter_egg::fight`).
#[derive(Resource, Default)]
pub(crate) struct EggRageFade(pub(crate) f32);

/// How far it's faded in, 0..=1 (smoothed).
#[derive(Resource, Default)]
pub(crate) struct EggMoodFade(pub(crate) f32);

/// Every light's brightness multiplier this frame (1 = steady) — see the
/// module docs.
#[derive(Resource)]
pub(crate) struct LightFlicker(pub(crate) f32);

impl Default for LightFlicker {
    fn default() -> Self {
        Self(1.0)
    }
}

/// What the egg's moments ask of the lights (`easter_egg::wake`): a long
/// stutter now (`jolt`, taken once seen), or none for now (`steady`).
#[derive(Resource, Default)]
pub(crate) struct EggLightCue {
    pub(crate) jolt: bool,
    pub(crate) steady: bool,
}

/// The stutters' timing — reset on leaving a game.
#[derive(Default)]
pub(crate) struct FlickerClock {
    /// It's been on (s) — for the stutter as it goes wrong.
    on_for: f32,
    /// When (`Time::elapsed_secs`) the current stutter ends, and the next
    /// one starts.
    until: f32,
    next_at: Option<f32>,
    seq: u32,
}

/// Whether our game's easter egg has gone wrong (and he's not dead yet).
fn unleashed(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>) -> bool {
    crate::zombies_hud::zombies_game(local, lobbies)
        .is_some_and(|l| shared::easter_egg::holds_spawns(l))
}

/// Whether he's alive in the fight and enraged (phase 3).
fn enraged(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>) -> bool {
    crate::zombies_hud::zombies_game(local, lobbies).is_some_and(|l| {
        shared::easter_egg::holds_spawns(l)
            && l.easter_egg.step == shared::easter_egg::EggStep::BossFight
            && l.easter_egg.hierophant.is_some_and(|h| h.phase >= 3)
    })
}

/// `progress` (linear) stepped toward `target` over `secs`; eased.
fn fade_toward(progress: &mut f32, target: f32, secs: f32, dt: f32) -> f32 {
    *progress = if secs <= 0.0 {
        target
    } else {
        let step = dt / secs;
        *progress + (target - *progress).clamp(-step, step)
    };
    let t = *progress;
    t * t * (3.0 - 2.0 * t)
}

/// Fade it toward on while it's gone wrong (or the panel's preview) and off
/// otherwise — straight to off out of a game — and stutter the lights while
/// it's in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_egg_mood(
    state: Res<State<AppState>>,
    look: Res<EggMoodLook>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    time: Res<Time>,
    // (Linear; `EggMoodFade` and `EggRageFade` are them eased.)
    (mut progress, mut rage_progress): (Local<f32>, Local<f32>),
    mut clock: Local<FlickerClock>,
    (mut fade, mut rage, mut now): (ResMut<EggMoodFade>, ResMut<EggRageFade>, ResMut<EggMoodNow>),
    mut flicker: ResMut<LightFlicker>,
    mut cue: ResMut<EggLightCue>,
) {
    let in_game = *state.get() == AppState::InGame;
    let on = in_game && look.enabled && (look.preview || unleashed(&local, &lobbies));
    let raging = on && (look.rage_preview || enraged(&local, &lobbies));
    let dt = time.delta_secs();
    if !in_game {
        *progress = 0.0;
        *rage_progress = 0.0;
        *clock = default();
        *cue = default();
    }
    let (target, secs) = if on { (1.0, look.fade_in_secs) } else { (0.0, look.fade_out_secs) };
    let eased = if in_game { fade_toward(&mut progress, target, secs, dt) } else { 0.0 };
    let (target, secs) = if raging { (1.0, look.rage_fade_in_secs) } else { (0.0, look.rage_fade_out_secs) };
    let rage_eased = if in_game { fade_toward(&mut rage_progress, target, secs, dt) } else { 0.0 };
    // (Only when they move — what reads them re-applies on a change.)
    if fade.0 != eased {
        fade.0 = eased;
    }
    if rage.0 != rage_eased {
        rage.0 = rage_eased;
    }
    if look.is_changed() || rage.is_changed() {
        let blended = look.enraged(rage_eased);
        if now.0 != blended {
            now.0 = blended;
        }
    }

    // (A jolt: the long stutter it starts with, over again.)
    if std::mem::take(&mut cue.bypass_change_detection().jolt) {
        clock.on_for = 0.0;
    }
    let raw = if on && look.flicker && cue.steady {
        clock.next_at = None;
        1.0
    } else if on && look.flicker {
        stutter(&look, &mut clock, time.elapsed_secs(), time.delta_secs())
    } else {
        clock.on_for = 0.0;
        clock.next_at = None;
        1.0
    };
    // (Easing in and out with the rest of it.)
    let value = 1.0 - (1.0 - raw) * eased;
    if flicker.0 != value {
        flicker.0 = value;
    }
}

/// The lights' multiplier this moment: 1 between stutters; during one (and
/// for a long one as it first goes wrong) cutting in and out at random,
/// [`EggMoodLook::flicker_rate_hz`] times a second.
fn stutter(look: &EggMoodLook, clock: &mut FlickerClock, now: f32, dt: f32) -> f32 {
    let roll = |seq: u32, k: u32| crate::util::rand01(seq.wrapping_mul(2_654_435_761).wrapping_add(k));
    let range = |(a, b): (f32, f32), r: f32| a.min(b) + (a.max(b) - a.min(b)) * r;
    // (As it first goes wrong: one long stutter, the gaps after it.)
    if clock.on_for < look.onset_secs {
        clock.on_for += dt;
        clock.next_at = None;
    } else {
        let next = *clock.next_at.get_or_insert(now + range(look.flicker_gap, roll(clock.seq, 1)));
        if now >= next {
            clock.seq = clock.seq.wrapping_add(1);
            let len = range(look.flicker_len, roll(clock.seq, 2));
            clock.until = now + len;
            clock.next_at = Some(now + len + range(look.flicker_gap, roll(clock.seq, 3)));
        }
        if now >= clock.until {
            return 1.0;
        }
    }
    // Which slice of the stutter this is, and whether it's out.
    let slice = (now * look.flicker_rate_hz.max(0.5)) as u32;
    let r = crate::util::rand01(slice.wrapping_mul(0x9e37_79b9) ^ 0x5bd1_e995);
    let depth = crate::util::rand01(slice.wrapping_mul(0x85eb_ca6b) ^ 0xc2b2_ae35);
    let low = look.flicker_low.clamp(0.02, 1.0);
    if r < look.flicker_off_chance {
        low + (0.3 - low).max(0.0) * depth * 0.5
    } else {
        0.7 + 0.3 * depth
    }
}

/// A light [`flicker_lights`] dimmed: its brightness before — put back by
/// [`unflicker_lights`].
#[derive(Component)]
pub(crate) struct FlickerBase(f32);

/// The players' flashlights — theirs to see by, never part of the stutter.
type NotFlashlight = (
    Without<crate::player::flashlight::LocalFlashlight>,
    Without<crate::player::flashlight::RemoteFlashlight>,
);

/// Put the [`LightFlicker`] on every light (bar the flashlights), after
/// everything's set this frame's brightness — or, steady, stop.
#[allow(clippy::type_complexity)]
fn flicker_lights(
    flicker: Res<LightFlicker>,
    mut points: Query<(Entity, &mut PointLight, Option<&mut FlickerBase>), (NotFlashlight, Without<SpotLight>)>,
    mut spots: Query<(Entity, &mut SpotLight, Option<&mut FlickerBase>), (NotFlashlight, Without<PointLight>)>,
    dimmed: Query<Entity, With<FlickerBase>>,
    mut commands: Commands,
) {
    let m = flicker.0;
    if m == 1.0 {
        for e in &dimmed {
            commands.entity(e).remove::<FlickerBase>();
        }
        return;
    }
    let mut dim = |e: Entity, intensity: &mut f32, base: Option<Mut<FlickerBase>>| {
        match base {
            Some(mut b) => b.0 = *intensity,
            None => {
                commands.entity(e).insert(FlickerBase(*intensity));
            }
        }
        *intensity *= m;
    };
    for (e, mut light, base) in &mut points {
        dim(e, &mut light.intensity, base);
    }
    for (e, mut light, base) in &mut spots {
        dim(e, &mut light.intensity, base);
    }
}

/// Put every light [`flicker_lights`] dimmed back as it was, before this
/// frame's systems (which may set it themselves) run.
#[allow(clippy::type_complexity)]
fn unflicker_lights(
    mut points: Query<(&FlickerBase, &mut PointLight), Without<SpotLight>>,
    mut spots: Query<(&FlickerBase, &mut SpotLight), Without<PointLight>>,
) {
    for (base, mut light) in &mut points {
        light.intensity = base.0;
    }
    for (base, mut light) in &mut spots {
        light.intensity = base.0;
    }
}

/// `own` blended toward `to` by `t`, channel by channel.
pub(crate) fn blend3(own: [f32; 3], to: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| own[i] + (to[i] - own[i]) * t)
}

/// The debug panel's controls for it.
pub(crate) fn egg_mood_section(ui: &mut egui::Ui, look: &mut EggMoodLook, fade: f32) {
    ui.label(format!("now: {:.0}% in", fade * 100.0));
    ui.checkbox(&mut look.enabled, "enabled");
    ui.checkbox(&mut look.preview, "preview (as if it's gone wrong)");
    ui.add(egui::Slider::new(&mut look.fade_in_secs, 0.0f32..=20.0).text("fade in (s)"));
    ui.add(egui::Slider::new(&mut look.fade_out_secs, 0.0f32..=20.0).text("fade out (s)"));
    let color = |ui: &mut egui::Ui, c: &mut [f32; 3], label: &str| {
        ui.horizontal(|ui| {
            ui.color_edit_button_rgb(c);
            ui.label(label);
        });
    };
    ui.separator();
    ui.label("Sky and fog");
    ui.add(egui::Slider::new(&mut look.sky_mult, 0.0f32..=1.0).text("sky brightness ×"));
    color(ui, &mut look.sky_tint, "sky tint");
    ui.add(egui::Slider::new(&mut look.sky_haze, 0.0f32..=1.0).text("haze on the sky"));
    ui.add(
        egui::Slider::new(&mut look.fog_visibility_m, 5.0f32..=1500.0)
            .logarithmic(true)
            .text("fog visibility (m)"),
    );
    color(ui, &mut look.fog_color, "fog colour");
    ui.add(egui::Slider::new(&mut look.sun_mult, 0.0f32..=1.0).text("sun ×"));
    ui.add(egui::Slider::new(&mut look.ambient_mult, 0.0f32..=2.0).text("ambient ×"));
    color(ui, &mut look.ambient_color, "ambient colour");
    ui.add(egui::Slider::new(&mut look.ground_fog_density, 0.0f32..=1.0).text("ground fog thickness"));
    ui.add(egui::Slider::new(&mut look.ground_fog_opacity, 0.0f32..=1.0).text("ground fog most opacity"));
    ui.add(egui::Slider::new(&mut look.ground_fog_falloff, 0.1f32..=20.0).text("ground fog height (m)"));
    color(ui, &mut look.ground_fog_color, "ground fog colour");
    ui.separator();
    ui.label("Fires");
    color(ui, &mut look.fire_core, "flame core");
    color(ui, &mut look.fire_mid, "flame middle");
    color(ui, &mut look.fire_tip, "flame tips");
    color(ui, &mut look.fire_light, "fire light");
    ui.separator();
    ui.label("Water");
    color(ui, &mut look.water_tint, "pool tint");
    color(ui, &mut look.water_deep, "pool colour deeper down");
    ui.add(egui::Slider::new(&mut look.water_clarity, 0.01f32..=5.0).logarithmic(true).text("pool clarity (m)"));
    color(ui, &mut look.waterfall_tint, "waterfall tint");
    ui.separator();
    ui.label("Lights stuttering (all together)");
    ui.checkbox(&mut look.flicker, "stutter");
    ui.add(egui::Slider::new(&mut look.flicker_gap.0, 0.2f32..=30.0).text("gap from (s)"));
    ui.add(egui::Slider::new(&mut look.flicker_gap.1, 0.2f32..=30.0).text("gap to (s)"));
    ui.add(egui::Slider::new(&mut look.flicker_len.0, 0.05f32..=5.0).text("lasting from (s)"));
    ui.add(egui::Slider::new(&mut look.flicker_len.1, 0.05f32..=5.0).text("lasting to (s)"));
    ui.add(egui::Slider::new(&mut look.flicker_rate_hz, 1.0f32..=40.0).text("cuts per second"));
    ui.add(egui::Slider::new(&mut look.flicker_off_chance, 0.0f32..=1.0).text("chance of being out"));
    ui.add(egui::Slider::new(&mut look.flicker_low, 0.02f32..=1.0).text("out to (× brightness)"));
    ui.add(egui::Slider::new(&mut look.onset_secs, 0.0f32..=10.0).text("stutter as it goes wrong (s)"));
    ui.separator();
    ui.label("Enraged (his phase 3) — blood-red over the rest");
    ui.checkbox(&mut look.rage_preview, "preview enraged (with the preview above, or in the fight)");
    ui.add(egui::Slider::new(&mut look.rage_fade_in_secs, 0.0f32..=20.0).text("turn red over (s)"));
    ui.add(egui::Slider::new(&mut look.rage_fade_out_secs, 0.0f32..=20.0).text("back over (s)"));
    ui.add(egui::Slider::new(&mut look.rage_sky_mult, 0.0f32..=1.0).text("sky brightness ×"));
    color(ui, &mut look.rage_sky_tint, "sky tint");
    ui.add(egui::Slider::new(&mut look.rage_sky_haze, 0.0f32..=1.0).text("haze on the sky"));
    ui.add(
        egui::Slider::new(&mut look.rage_fog_visibility_m, 5.0f32..=1500.0)
            .logarithmic(true)
            .text("fog visibility (m)"),
    );
    color(ui, &mut look.rage_fog_color, "fog colour");
    ui.add(egui::Slider::new(&mut look.rage_ambient_mult, 0.0f32..=2.0).text("ambient ×"));
    color(ui, &mut look.rage_ambient_color, "ambient colour");
    ui.add(egui::Slider::new(&mut look.rage_ground_fog_density, 0.0f32..=1.0).text("ground fog thickness"));
    ui.add(egui::Slider::new(&mut look.rage_ground_fog_opacity, 0.0f32..=1.0).text("ground fog most opacity"));
    color(ui, &mut look.rage_ground_fog_color, "ground fog colour");
    color(ui, &mut look.rage_fire_core, "flame core");
    color(ui, &mut look.rage_fire_mid, "flame middle");
    color(ui, &mut look.rage_fire_tip, "flame tips");
    color(ui, &mut look.rage_fire_light, "fire light");
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            *look = EggMoodLook {
                preview: look.preview,
                rage_preview: look.rage_preview,
                ..default()
            };
        }
        if ui.button("Print settings to console").clicked() {
            info!("{look:#?}");
        }
    });
}
