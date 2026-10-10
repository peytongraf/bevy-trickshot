//! The dog-round mood: while a `Zombies` dog round is on
//! (`shared::dogs::is_dog_round`, with dogs still left), the sky darkens and
//! a thick, blood-dark fog rolls in — fading in as the round starts and back
//! out once its last dog is dead.
//!
//! [`DogRoundFade`] is how far in it is (0..1); `apply_scene_tuning` blends
//! the map's own fog / sun / ambient toward [`DogRoundLook`]'s by it, and
//! [`apply_dog_round_sky`] tints the sky sphere. The thicker fog would hide
//! the sky, so while it's on the sky's taken out of the fog and a haze shell
//! ([`SkyHazeVeil`]) stands in for it: matching the map's usual fog on the
//! sky as it starts, thinning to [`DogRoundLook::sky_fog`] of that. Tuned in the debug panel's
//! "Dogs (Zombies)" → "Dog round sky + fog". Out of a game the fade is
//! dropped straight back to 0, so nothing carries into the next one.

use bevy::color::Mix;
use bevy::pbr::FogFalloff;
use bevy::prelude::*;
use bevy_egui::egui;
use lightyear::prelude::LocalId;
use shared::Lobby;

use crate::net::GameClient;
use crate::util::color_from_parts;
use crate::AppState;

use super::atmosphere::MapLooks;
use super::map::CurrentMap;
use super::sky::{SkyHazeVeil, SkyMaterial, SKY_HAZE_RADIUS};

/// Panel-tunable dog-round look.
#[derive(Resource, Clone)]
pub(crate) struct DogRoundLook {
    pub(crate) enabled: bool,
    /// Debug: act as if a dog round's on (untick to fade back out).
    pub(crate) preview: bool,
    /// Seconds to fade in as the round starts, and out after its last dog.
    pub(crate) fade_in_secs: f32,
    pub(crate) fade_out_secs: f32,
    /// The sky's brightness at full darkness (1 = untouched)...
    pub(crate) sky_brightness: f32,
    /// ...and its tint (sRGB multiplier).
    pub(crate) sky_tint: [f32; 3],
    /// How much of the map's usual haze stays over the sky at full darkness
    /// (0 = a clear sky, 1 = as hazy as ever) — the thicker fog never covers it.
    pub(crate) sky_fog: f32,
    /// How far (m) you can see through the fog at full darkness — never
    /// clearer than the map's own.
    pub(crate) fog_visibility_m: f32,
    /// The fog's colour at full darkness.
    pub(crate) fog_color: [f32; 3],
    /// The sun's (and its glow in the fog's) strength, and the ambient
    /// light's, as a multiple of the map's own at full darkness.
    pub(crate) sun_mult: f32,
    pub(crate) ambient_mult: f32,
    /// The ambient light's colour at full darkness.
    pub(crate) ambient_color: [f32; 3],
}

impl Default for DogRoundLook {
    fn default() -> Self {
        Self {
            enabled: true,
            preview: false,
            fade_in_secs: 4.0,
            fade_out_secs: 6.0,
            sky_brightness: 1.0,
            sky_tint: [0.325, 0.032, 0.0],
            sky_fog: 0.73,
            fog_visibility_m: 40.0,
            fog_color: [0.172, 0.051, 0.051],
            sun_mult: 0.01,
            ambient_mult: 0.26,
            ambient_color: [1.0, 0.321, 0.282],
        }
    }
}

/// How far the dog-round mood is faded in, 0..=1 (smoothed).
#[derive(Resource, Default)]
pub(crate) struct DogRoundFade(pub(crate) f32);

/// Whether our `Zombies` game is in a dog round that still has dogs left.
fn dog_round_on(local: &Query<&LocalId, With<GameClient>>, lobbies: &Query<&Lobby>) -> bool {
    crate::zombies_hud::zombies_game(local, lobbies)
        .is_some_and(|l| shared::dogs::is_dog_round(l.round) && l.enemies_left > 0)
}

/// Fade the mood toward on during a dog round (or the panel's preview) and
/// off otherwise — straight to off out of a game.
pub(crate) fn update_dog_round_fade(
    state: Res<State<AppState>>,
    look: Res<DogRoundLook>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    time: Res<Time>,
    // (Linear; `DogRoundFade` is it eased.)
    mut progress: Local<f32>,
    mut fade: ResMut<DogRoundFade>,
) {
    if *state.get() != AppState::InGame {
        *progress = 0.0;
    } else {
        let on = look.enabled && (look.preview || dog_round_on(&local, &lobbies));
        let (target, secs) = if on { (1.0, look.fade_in_secs) } else { (0.0, look.fade_out_secs) };
        *progress = if secs <= 0.0 {
            target
        } else {
            let step = time.delta_secs() / secs;
            *progress + (target - *progress).clamp(-step, step)
        };
    }
    let t = *progress;
    let eased = t * t * (3.0 - 2.0 * t);
    // (Only when it moves — `apply_scene_tuning` re-applies on a change.)
    if fade.0 != eased {
        fade.0 = eased;
    }
}

/// Tint the sky sphere by [`DogRoundFade`], and while it's on, swap the
/// fog on the sky for the haze shell.
pub(crate) fn apply_dog_round_sky(
    fade: Res<DogRoundFade>,
    look: Res<DogRoundLook>,
    current: Res<CurrentMap>,
    looks: MapLooks,
    sky: Query<&SkyMaterial>,
    mut veil: Query<(&SkyHazeVeil, &mut Visibility)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !fade.is_changed() && !look.is_changed() && !current.is_changed() && !looks.is_changed() {
        return;
    }
    let t = fade.0;
    let on = t > 0.0;
    if let Some(material) = sky.single().ok().and_then(|s| materials.get_mut(&s.0)) {
        let dark = LinearRgba::from(color_from_parts(look.sky_tint)) * look.sky_brightness;
        material.base_color = Color::LinearRgba(LinearRgba::WHITE * (1.0 - t) + dark * t);
        // (Only flipped when it changes — it re-specialises the material.)
        if material.fog_enabled == on {
            material.fog_enabled = !on;
        }
    }
    let Ok((veil, mut vis)) = veil.single_mut() else { return };
    vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    let Some(material) = materials.get_mut(&veil.0) else { return };
    // How much the map's own fog covers the sky (as Bevy's exponential fog
    // would at the shell's distance), thinned as the darkness comes in, in
    // the fog's colour of the moment (`apply_scene_tuning`'s blend).
    let base = looks.get(current.0);
    let FogFalloff::Exponential { density } = FogFalloff::from_visibility(base.fog_visibility_m) else {
        return;
    };
    let haze = 1.0 - (-density * SKY_HAZE_RADIUS).exp();
    let alpha = haze * (1.0 + (look.sky_fog - 1.0) * t);
    let color = color_from_parts(base.fog_color).mix(&color_from_parts(look.fog_color), t);
    material.base_color = color.with_alpha(alpha);
}

/// The debug panel's controls for it.
pub(crate) fn dog_round_look_section(ui: &mut egui::Ui, look: &mut DogRoundLook, fade: f32) {
    ui.label(format!("now: {:.0}% in", fade * 100.0));
    ui.checkbox(&mut look.enabled, "enabled");
    ui.checkbox(&mut look.preview, "preview (as if a dog round's on)");
    ui.add(egui::Slider::new(&mut look.fade_in_secs, 0.0f32..=20.0).text("fade in (s)"));
    ui.add(egui::Slider::new(&mut look.fade_out_secs, 0.0f32..=20.0).text("fade out (s)"));
    ui.separator();
    ui.add(egui::Slider::new(&mut look.sky_brightness, 0.0f32..=1.0).text("sky brightness"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut look.sky_tint);
        ui.label("sky tint");
    });
    ui.add(egui::Slider::new(&mut look.sky_fog, 0.0f32..=1.0).text("haze left on the sky"));
    ui.add(
        egui::Slider::new(&mut look.fog_visibility_m, 5.0f32..=1500.0)
            .logarithmic(true)
            .text("fog visibility (m)"),
    );
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut look.fog_color);
        ui.label("fog colour");
    });
    ui.add(egui::Slider::new(&mut look.sun_mult, 0.0f32..=1.0).text("sun ×"));
    ui.add(egui::Slider::new(&mut look.ambient_mult, 0.0f32..=1.0).text("ambient ×"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut look.ambient_color);
        ui.label("ambient colour");
    });
    if ui.button("Copy to console").clicked() {
        info!(
            "dog round look: fade_in_secs: {:.1}, fade_out_secs: {:.1}, sky_brightness: {:.2}, sky_tint: {:?}, sky_fog: {:.2}, \
             fog_visibility_m: {:.0}, fog_color: {:?}, sun_mult: {:.2}, ambient_mult: {:.2}, ambient_color: {:?}",
            look.fade_in_secs,
            look.fade_out_secs,
            look.sky_brightness,
            look.sky_tint,
            look.sky_fog,
            look.fog_visibility_m,
            look.fog_color,
            look.sun_mult,
            look.ambient_mult,
            look.ambient_color,
        );
    }
    if ui.button("Reset").clicked() {
        *look = DogRoundLook {
            preview: look.preview,
            ..default()
        };
    }
}
