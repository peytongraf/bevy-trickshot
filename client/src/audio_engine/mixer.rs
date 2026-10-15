//! The audio mixer: a debug-mode window (F9, or the debug panel's
//! "Audio" section) for dialling in the [`Mix`] while the game plays —
//! bus faders with mute / solo, and every clip under `assets/audio/` in its
//! folder tree with its level (dB), bus and reverb, each auditionable.
//! Changes are heard at once; Save writes them to `assets/audio/mix.ron`.
//! Esc, the mouse back button or F9 close it.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use super::mix::{Bus, BusMonitor, Mix, MixEntry, Profile, AIR_OFF_HZ, DEFAULT_PROFILE};
use super::{AudioPlayer, PlaybackSettings, SpatialListener};
use crate::AppState;

pub(super) struct MixerPlugin;

impl Plugin for MixerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioMixer>()
            .insert_resource(AudioCatalog::scan())
            .add_systems(Update, (toggle_mixer, stop_auditions_when_closed).chain())
            .add_systems(OnEnter(AppState::InGame), close_mixer)
            .add_systems(OnExit(AppState::InGame), close_mixer)
            .add_systems(EguiPrimaryContextPass, mixer_ui.run_if(crate::menu::debug_enabled));
    }
}

/// Whether the mixer's open, and its own UI state. While it's open in a
/// match the cursor's free (`menu::cursor_and_hud`).
#[derive(Resource, Default)]
pub(crate) struct AudioMixer {
    pub(crate) open: bool,
    /// Only clips whose path contains this (case-insensitive), as a flat
    /// list.
    filter: String,
    /// The mix as last loaded / saved — what Revert goes back to, and what
    /// "unsaved changes" compares against.
    saved: Option<Mix>,
    status: Option<String>,
    /// How far in front of us ▶ plays a clip (m) — through its positional
    /// profile; `0` plays it in our head.
    audition_distance: f32,
    /// The name typed for a new profile.
    new_profile: String,
}

/// Every clip under `assets/audio/` (bar `unused/`), as a folder tree —
/// listed once at startup.
#[derive(Resource)]
struct AudioCatalog {
    root: Folder,
}

struct Folder {
    name: String,
    /// Its key in the mix (`weapons/sniper`); empty for the root.
    path: String,
    folders: Vec<Folder>,
    /// Clip keys (`weapons/sniper/shot.wav`).
    clips: Vec<String>,
}

impl AudioCatalog {
    fn scan() -> Self {
        let base = bevy::asset::io::file::FileAssetReader::get_base_path().join("assets/audio");
        Self {
            root: scan_folder(&base, String::new(), String::new()),
        }
    }

    fn all_clips(&self) -> Vec<&str> {
        fn walk<'a>(f: &'a Folder, out: &mut Vec<&'a str>) {
            out.extend(f.clips.iter().map(String::as_str));
            for sub in &f.folders {
                walk(sub, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }
}

fn scan_folder(dir: &std::path::Path, name: String, path: String) -> Folder {
    let mut folders = Vec::new();
    let mut clips = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let key = if path.is_empty() { file_name.clone() } else { format!("{path}/{file_name}") };
        let p = entry.path();
        if p.is_dir() {
            if key != "unused" {
                folders.push(scan_folder(&p, file_name, key));
            }
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "mp3" | "wav" | "ogg" | "flac"))
        {
            clips.push(key);
        }
    }
    folders.retain(|f| !f.folders.is_empty() || !f.clips.is_empty());
    Folder {
        name,
        path,
        folders,
        clips,
    }
}

/// A clip playing from the mixer's ▶ — one at a time.
#[derive(Component)]
struct MixerAudition;

fn toggle_mixer(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<crate::settings::Settings>,
    state: Res<State<AppState>>,
    mut mixer: ResMut<AudioMixer>,
) {
    let allowed = settings.debug_mode && *state.get() != AppState::LevelEditor;
    if !allowed {
        if mixer.open {
            mixer.open = false;
        }
        return;
    }
    if keys.just_pressed(crate::menu::AUDIO_EDITOR_KEY) {
        mixer.open = !mixer.open;
    }
}

fn close_mixer(mut mixer: ResMut<AudioMixer>) {
    mixer.open = false;
}

fn stop_auditions_when_closed(
    mixer: Res<AudioMixer>,
    auditions: Query<Entity, With<MixerAudition>>,
    mut commands: Commands,
) {
    if mixer.is_changed() && !mixer.open {
        for e in &auditions {
            commands.entity(e).despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mixer_ui(
    mut contexts: EguiContexts,
    mut mixer: ResMut<AudioMixer>,
    mut mix: ResMut<Mix>,
    mut monitor: ResMut<BusMonitor>,
    catalog: Res<AudioCatalog>,
    asset_server: Res<AssetServer>,
    auditions: Query<Entity, With<MixerAudition>>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    mut commands: Commands,
) -> Result {
    if !mixer.open {
        return Ok(());
    }
    // UI-only state, edited without flagging the mixer changed (only
    // opening / closing it should — `menu::cursor_and_hud` follows that).
    let state = mixer.bypass_change_detection();
    if state.saved.is_none() {
        state.saved = Some(mix.clone());
    }
    let ctx = contexts.ctx_mut()?;
    // Edited through this (not `ResMut`'s `DerefMut`), so the mix only
    // counts as changed — and gets re-applied to what's playing — when
    // something actually is.
    let m = mix.bypass_change_detection();
    let mut changed = false;
    let mut open = true;
    let mut audition: Option<String> = None;
    let mut stop = false;
    egui::Window::new("Audio mixer")
        .open(&mut open)
        .default_size([620.0, 680.0])
        .show(ctx, |ui| {
            // --- save / revert -----------------------------------------
            ui.horizontal(|ui| {
                let dirty = state.saved.as_ref().is_some_and(|s| s.pruned() != m.pruned());
                if ui.button("💾 Save").clicked() {
                    state.status = Some(match m.save() {
                        Ok(path) => {
                            state.saved = Some(m.clone());
                            format!("Saved to {}", path.display())
                        }
                        Err(e) => format!("Couldn't save: {e}"),
                    });
                }
                if ui.add_enabled(dirty, egui::Button::new("↺ Revert")).clicked() {
                    if let Some(saved) = state.saved.clone() {
                        *m = saved;
                        changed = true;
                    }
                    state.status = None;
                }
                if dirty {
                    ui.colored_label(egui::Color32::YELLOW, "unsaved changes");
                } else if let Some(status) = &state.status {
                    ui.weak(status);
                }
            });
            ui.weak(
                "Levels are dB, added up: bus + each folder + the clip (+ each \"pos\" offset \
                 when it's played positionally). Bus, reverb and profile come from the clip, \
                 else its nearest folder that sets one. \"world\" sounds feed the room's \
                 reverb; \"auto\" = positional sounds do, others don't. Reverb changes apply \
                 to sounds started afterwards; everything else is live.",
            );
            ui.separator();

            // --- buses ---------------------------------------------------
            ui.strong("Buses");
            egui::Grid::new("mixer_buses").num_columns(4).striped(true).show(ui, |ui| {
                for bus in Bus::ALL {
                    ui.label(bus.label());
                    let mut db = m.bus_db(bus);
                    if db_slider(ui, &mut db) {
                        m.buses.insert(bus, db);
                        changed = true;
                    }
                    let mut muted = monitor.muted.contains(&bus);
                    if ui.toggle_value(&mut muted, "M").on_hover_text("mute").changed() {
                        toggle(&mut monitor.muted, bus, muted);
                    }
                    let mut soloed = monitor.soloed.contains(&bus);
                    if ui.toggle_value(&mut soloed, "S").on_hover_text("solo").changed() {
                        toggle(&mut monitor.soloed, bus, soloed);
                    }
                    ui.end_row();
                }
            });
            ui.separator();

            // --- positional profiles --------------------------------------
            ui.collapsing("Positional profiles", |ui| {
                ui.weak(
                    "How positional sounds carry. Every sound using a profile follows it, live; \
                     pick a sound's profile in its row below.",
                );
                if profiles_ui(ui, m, &mut state.new_profile) {
                    changed = true;
                }
            });
            ui.separator();

            // --- sounds --------------------------------------------------
            ui.horizontal(|ui| {
                ui.strong("Sounds");
                ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text("filter…").desired_width(160.0));
                if ui.add_enabled(!auditions.is_empty(), egui::Button::new("■ Stop")).clicked() {
                    stop = true;
                }
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = 150.0;
                ui.add(egui::Slider::new(&mut state.audition_distance, 0.0f32..=150.0).suffix(" m"))
                    .on_hover_text("0 = in your head; further plays it that far ahead, through its profile");
                ui.weak(if state.audition_distance > 0.0 { "▶ plays this far ahead" } else { "▶ plays in your head" });
            });
            let names = profile_names(m);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let filter = state.filter.trim().to_lowercase();
                if filter.is_empty() {
                    for folder in &catalog.root.folders {
                        folder_ui(ui, folder, m, &names, &mut changed, &mut audition);
                    }
                    for clip in &catalog.root.clips {
                        clip_row(ui, clip, m, &names, &mut changed, &mut audition);
                    }
                } else {
                    for clip in catalog.all_clips().into_iter().filter(|c| c.to_lowercase().contains(&filter)) {
                        clip_row(ui, clip, m, &names, &mut changed, &mut audition);
                    }
                }
            });
        });
    if changed {
        mix.set_changed();
    }
    if !open {
        mixer.open = false;
    }
    if stop || audition.is_some() {
        for e in &auditions {
            commands.entity(e).despawn();
        }
    }
    if let Some(clip) = audition {
        let player = (
            Name::new("Mixer audition"),
            MixerAudition,
            AudioPlayer::new(asset_server.load(format!("audio/{clip}"))),
        );
        let distance = mixer.bypass_change_detection().audition_distance;
        match listener.single() {
            Ok(ears) if distance > 0.0 => {
                let at = ears.translation() + ears.forward() * distance;
                commands.spawn((player, Transform::from_translation(at), crate::positional_playback(super::Volume::Linear(1.0))));
            }
            _ => {
                commands.spawn((player, PlaybackSettings::DESPAWN));
            }
        }
    }
    Ok(())
}

fn toggle(set: &mut std::collections::HashSet<Bus>, bus: Bus, on: bool) {
    if on {
        set.insert(bus);
    } else {
        set.remove(&bus);
    }
}

fn db_slider(ui: &mut egui::Ui, db: &mut f32) -> bool {
    ui.spacing_mut().slider_width = 150.0;
    ui.add(
        egui::Slider::new(db, -40.0f32..=24.0)
            .suffix(" dB")
            .fixed_decimals(1)
            .clamping(egui::SliderClamping::Never),
    )
    .changed()
}

fn folder_ui(
    ui: &mut egui::Ui,
    folder: &Folder,
    mix: &mut Mix,
    names: &[String],
    changed: &mut bool,
    audition: &mut Option<String>,
) {
    let id = ui.make_persistent_id(("mixer_folder", &folder.path));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            ui.strong(format!("{}/", folder.name));
            let mut entry = mix.folders.get(&folder.path).cloned().unwrap_or_default();
            if entry_controls(ui, &folder.path, names, &mut entry) {
                set_entry(&mut mix.folders, &folder.path, entry);
                *changed = true;
            }
        })
        .body(|ui| {
            for sub in &folder.folders {
                folder_ui(ui, sub, mix, names, changed, audition);
            }
            for clip in &folder.clips {
                clip_row(ui, clip, mix, names, changed, audition);
            }
        });
}

fn clip_row(
    ui: &mut egui::Ui,
    clip: &str,
    mix: &mut Mix,
    names: &[String],
    changed: &mut bool,
    audition: &mut Option<String>,
) {
    ui.horizontal(|ui| {
        if ui.small_button("▶").on_hover_text("audition").clicked() {
            *audition = Some(clip.to_owned());
        }
        let name = clip.rsplit_once('/').map_or(clip, |(_, n)| n);
        ui.label(name).on_hover_text(clip);
        let mut entry = mix.clips.get(clip).cloned().unwrap_or_default();
        if entry_controls(ui, clip, names, &mut entry) {
            set_entry(&mut mix.clips, clip, entry);
            *changed = true;
        }
        let r = mix.resolve(clip);
        let reverb = match r.reverb {
            Some(true) => "world",
            Some(false) => "dry",
            None => "auto",
        };
        let pos = if r.positional_db != 0.0 { format!(" ({:+.1} pos)", r.positional_db) } else { String::new() };
        ui.weak(format!("= {:+.1} dB{pos} · {} · {reverb} · {}", r.volume_db, r.bus.label(), r.profile));
    });
}

/// Level, bus, reverb, profile and positional offset for one folder /
/// clip; `true` if changed.
fn entry_controls(ui: &mut egui::Ui, key: &str, names: &[String], entry: &mut MixEntry) -> bool {
    let before = entry.clone();
    db_slider(ui, &mut entry.volume_db);
    egui::ComboBox::from_id_salt(("mixer_bus", key))
        .width(110.0)
        .selected_text(entry.bus.map_or("bus: inherit", Bus::label))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut entry.bus, None, "inherit");
            for bus in Bus::ALL {
                ui.selectable_value(&mut entry.bus, Some(bus), bus.label());
            }
        });
    egui::ComboBox::from_id_salt(("mixer_reverb", key))
        .width(100.0)
        .selected_text(match entry.reverb {
            None => "reverb: inherit",
            Some(true) => "world",
            Some(false) => "dry",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut entry.reverb, None, "inherit");
            ui.selectable_value(&mut entry.reverb, Some(true), "world (reverb)");
            ui.selectable_value(&mut entry.reverb, Some(false), "dry");
        });
    egui::ComboBox::from_id_salt(("mixer_profile", key))
        .width(110.0)
        .selected_text(entry.profile.as_deref().unwrap_or("profile: inherit"))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut entry.profile, None, "inherit");
            for name in names {
                ui.selectable_value(&mut entry.profile, Some(name.clone()), name);
            }
        });
    ui.add(
        egui::DragValue::new(&mut entry.positional_db)
            .speed(0.1)
            .range(-60.0..=24.0)
            .fixed_decimals(1)
            .prefix("pos ")
            .suffix(" dB"),
    )
    .on_hover_text("extra level when it's played positionally (e.g. another player's)");
    if !entry.is_default() && ui.small_button("↺").on_hover_text("reset").clicked() {
        *entry = MixEntry::default();
    }
    *entry != before
}

fn set_entry(map: &mut std::collections::BTreeMap<String, MixEntry>, key: &str, entry: MixEntry) {
    if entry.is_default() {
        map.remove(key);
    } else {
        map.insert(key.to_owned(), entry);
    }
}

/// Every profile's name, the default first.
fn profile_names(mix: &Mix) -> Vec<String> {
    let mut names = vec![DEFAULT_PROFILE.to_owned()];
    names.extend(mix.profiles.keys().filter(|n| n.as_str() != DEFAULT_PROFILE).cloned());
    names
}

/// Each profile's settings, with its fade drawn; adding / removing them.
/// `true` if anything changed.
fn profiles_ui(ui: &mut egui::Ui, mix: &mut Mix, new_name: &mut String) -> bool {
    let mut changed = false;
    let mut remove = None;
    for name in profile_names(mix) {
        let users = mix
            .folders
            .values()
            .chain(mix.clips.values())
            .filter(|e| e.profile.as_deref() == Some(name.as_str()))
            .count();
        let mut p = mix.profile(&name);
        let before = p.clone();
        ui.push_id(("mixer_profile_edit", &name), |ui| {
            ui.collapsing(format!("{name}  ({users} folders / clips)"), |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().slider_width = 160.0;
                        ui.add(egui::Slider::new(&mut p.volume_db, -40.0f32..=24.0).suffix(" dB").text("level"));
                        ui.add(egui::Slider::new(&mut p.near, 0.0f32..=100.0).suffix(" m").text("full volume within"));
                        ui.add(egui::Slider::new(&mut p.far, 1.0f32..=400.0).suffix(" m").text("silent beyond"));
                        ui.add(egui::Slider::new(&mut p.curve, 0.25f32..=6.0).text("curve (1 linear, higher drops sooner)"));
                        ui.add(egui::Slider::new(&mut p.width, 0.0f32..=1.0).text("stereo width"));
                        ui.add(
                            egui::Slider::new(&mut p.air_hz, 500.0f32..=AIR_OFF_HZ)
                                .logarithmic(true)
                                .suffix(" Hz")
                                .text("air: highs left at the far end"),
                        );
                        if p.air_hz >= AIR_OFF_HZ {
                            ui.weak("(air absorption off)");
                        }
                    });
                    falloff_plot(ui, &p);
                });
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        p = Profile::default();
                    }
                    if name != DEFAULT_PROFILE && ui.button("Delete").clicked() {
                        remove = Some(name.clone());
                    }
                });
            });
        });
        p.near = p.near.min(p.far - 0.1).max(0.0);
        if p != before {
            mix.profiles.insert(name.clone(), p);
            changed = true;
        }
    }
    if let Some(name) = remove {
        mix.profiles.remove(&name);
        // Whatever used it falls back to its folder's (or the default).
        for entry in mix.folders.values_mut().chain(mix.clips.values_mut()) {
            if entry.profile.as_deref() == Some(name.as_str()) {
                entry.profile = None;
            }
        }
        mix.folders.retain(|_, e| !e.is_default());
        mix.clips.retain(|_, e| !e.is_default());
        changed = true;
    }
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(new_name).hint_text("new profile name").desired_width(160.0));
        let name = new_name.trim().to_owned();
        let free = !name.is_empty() && !profile_names(mix).contains(&name);
        if ui.add_enabled(free, egui::Button::new("Add")).clicked() {
            mix.profiles.insert(name, mix.profile(DEFAULT_PROFILE));
            new_name.clear();
            changed = true;
        }
    });
    changed
}

/// A profile's fade (gain against distance, its level left out), with its
/// near / far marked.
fn falloff_plot(ui: &mut egui::Ui, p: &Profile) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 90.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let stroke = ui.visuals().widgets.noninteractive.fg_stroke;
    painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
    let span = (p.far * 1.15).max(1.0);
    let level = super::mix::db_to_linear(p.volume_db).max(1e-6);
    let point = |d: f32| {
        let g = p.gain(d) / level;
        egui::pos2(rect.left() + rect.width() * d / span, rect.bottom() - rect.height() * g)
    };
    let points: Vec<_> = (0..=60).map(|i| point(span * i as f32 / 60.0)).collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill)));
    for (d, label) in [(p.near, "near"), (p.far, "far")] {
        let x = rect.left() + rect.width() * d / span;
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], stroke);
        painter.text(egui::pos2(x + 2.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(10.0), stroke.color);
    }
    painter.text(
        rect.right_bottom() + egui::vec2(-2.0, -2.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{:.0} m", span),
        egui::FontId::proportional(10.0),
        stroke.color,
    );
}
