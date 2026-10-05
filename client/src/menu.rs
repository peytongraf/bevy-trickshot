//! The pause / settings menu and the first-run username screen, built with
//! `bevy_ui`.
//!
//! * `Esc` opens/closes the settings menu (or the username screen if no name is
//!   set yet). While either is open the game is frozen and the cursor is free.
//! * Categories: **Profile** (username), **Controls** (sensitivity, ADS
//!   sensitivity, FOV, debug toggle), **Keybinds** (rebind any action).
//! * `debug_mode` gates the existing egui tuning panels (`ads_tuning_ui`).
//!
//! The UI is rebuilt from scratch whenever `menu.dirty` is set (tab / screen /
//! rebind changes). Live values (slider fills, the FOV number, the username
//! field) are patched in place by [`refresh_dynamic`] so a slider drag doesn't
//! churn the whole tree.

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::PrimaryWindow;
use lightyear::prelude::*;

use crate::keybinds::{Binding, KeyBindings, SLOTS};
use crate::net::GameClient;
use crate::settings::{
    AutoMantle, CrosshairId, ScopeZoom, Settings, ShadowQuality, ADS_COEFF_MAX, ADS_COEFF_MIN, ADS_SENS_MAX, ADS_SENS_MIN, FOV_MAX,
    FOV_MIN, FRAME_LIMIT_MAX, FRAME_LIMIT_MIN, SENS_MAX, SENS_MIN, VOLUME_MAX, VOLUME_MIN,
};
use crate::ui::{
    divider, field_box, label_body, label_hud, menu_background, page_title, panel_node, section_heading, spawn_button_hud,
    ui_sound, Hoverable, UiSound, ACCENT, ACCENT_DIM, BACKDROP, DEFEAT, EDGE, PANEL, PANEL_SOLID,
    ROW, ROW_HOVER, TEXT, TEXT_DIM, TRACK, VICTORY,
};
use crate::loadout::{weapon_stats, LoadoutCtx};
use crate::{crosshair_asset_path, AppState};
use shared::weapon::{WeaponId, LOADOUT_WEAPONS};

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Screen {
    None,
    Username,
    Settings,
    /// The match just ended — shows VICTORY/DEFEAT and the final scoreboard.
    /// Only dismissed by its own CONTINUE button (see `Btn::ContinueFromResults`),
    /// not `Esc`, so the result can't be skipped past by accident.
    MatchResults,
    /// A lobby game was just started and the party is waiting for every
    /// member's client to finish loading the map. Owned and built by
    /// `game_start`, not this module (it needs live `shared::Lobby` data);
    /// this variant exists purely so `game_active`'s "is a menu up" check
    /// freezes gameplay input the same way it does for every other screen.
    /// Not dismissed by `Esc` — `game_start` clears it once everyone's ready.
    LoadingGame,
    /// The Loadout screen, Call of Duty style: the primary weapons down the
    /// left (click one to equip it), the one picked shown large on the right
    /// with its stats, and its GUNSMITH page ([`Menu::gunsmith`]) for its
    /// options (the sniper's reticle and scope zoom). Reachable from the main
    /// menu and the lobby room (`lobby_ui`'s `MenuBtn::OpenLoadout` /
    /// `OpenGunsmith`) and, in-game, from a button inside `Screen::Settings`.
    /// Whether the pick can change is `loadout::LoadoutCtx`'s call (not in a
    /// `Zombies` game in progress). `Esc` / `Btn::CloseLoadout` return to
    /// [`Menu::loadout_return`] rather than always `Screen::None`, so
    /// backing out from the in-game pause menu lands back on the pause menu
    /// instead of dropping straight into gameplay.
    Loadout,
    /// The `Zombies` Pack-a-Punch menu, opened with the interact key at the
    /// machine. Owned and built by `pap_menu`, not this module (it needs live
    /// lobby data); being a `Screen` is what frees the cursor and freezes
    /// gameplay input while it's up. `Esc` closes it.
    PackAPunch,
    /// The `Zombies` Der Wunderfizz menu (classic perks), opened with the
    /// interact key at the machine. Owned and built by `wunderfizz`, like
    /// [`Screen::PackAPunch`]; unlike it, buying keeps it open — the drink
    /// plays behind it (see [`weapon_active`]). `Esc` closes it.
    Wunderfizz,
    /// The `Zombies` armor station menu, opened with the interact key at the
    /// station. Owned and built by `armor`, like [`Screen::PackAPunch`].
    /// `Esc` closes it.
    Armor,
    /// PAST UPDATES: every older version's patch notes
    /// ([`crate::changelog::history`]) — the main menu's "WHAT'S NEW" panel
    /// only shows the current version's. Opened from that panel
    /// (`lobby_ui`'s `MenuBtn::OpenChangelog`); `Esc` / BACK close it.
    Changelog,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Tab {
    Profile,
    Controls,
    Graphics,
    Audio,
    Keybinds,
    Multiplayer,
}

#[derive(Resource)]
pub struct Menu {
    pub screen: Screen,
    pub tab: Tab,
    /// `Some(i)` while waiting to capture a new binding for `SLOTS[i]`.
    pub rebinding: Option<usize>,
    /// Set true one frame after a rebind starts, so the click that started it
    /// isn't captured as the new binding.
    rebind_armed: bool,
    pub username_draft: String,
    /// Where `Screen::Loadout` returns to on close — whatever `screen` was
    /// right before it opened (`Screen::None` from the main menu,
    /// `Screen::Settings` from the pause menu).
    pub loadout_return: Screen,
    /// The weapon shown large on the Loadout screen — `None` for whatever's
    /// equipped (`Settings::primary`).
    pub(crate) loadout_focus: Option<WeaponId>,
    /// `Some` while the Loadout screen shows that weapon's GUNSMITH page.
    pub(crate) gunsmith: Option<WeaponId>,
    /// Request a full UI rebuild next frame. `pub(crate)` rather than fully
    /// private: modules that build their own content into a `Screen` (e.g.
    /// `lobby_ui` opening `Screen::Loadout`) need to request the rebuild too.
    pub(crate) dirty: bool,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            screen: Screen::None,
            tab: Tab::Profile,
            rebinding: None,
            rebind_armed: false,
            username_draft: String::new(),
            loadout_return: Screen::None,
            loadout_focus: None,
            gunsmith: None,
            dirty: true,
        }
    }
}

impl Menu {
    pub fn is_open(&self) -> bool {
        self.screen != Screen::None
    }

    /// Open the Loadout screen — straight onto `gunsmith`'s GUNSMITH page if
    /// given — returning to whatever's showing now when it closes.
    pub(crate) fn open_loadout(&mut self, gunsmith: Option<WeaponId>) {
        if self.screen != Screen::Loadout {
            self.loadout_return = self.screen;
        }
        self.screen = Screen::Loadout;
        self.loadout_focus = gunsmith;
        self.gunsmith = gunsmith;
        self.dirty = true;
    }

    /// Back out one level: GUNSMITH → the weapon list → wherever the
    /// Loadout screen was opened from.
    fn back_from_loadout(&mut self) {
        if self.gunsmith.take().is_none() {
            self.screen = self.loadout_return;
        }
        self.dirty = true;
    }
}

/// Run condition: gameplay systems only tick while no menu is up, the match
/// hasn't just ended or been paused, and the player isn't dead.
pub fn game_active(
    menu: Res<Menu>,
    freeze: Res<crate::match_end::MatchEndFreeze>,
    death: Res<crate::death_effect::DeathEffect>,
    paused: Res<crate::pause::GamePaused>,
    spectate: Res<crate::revive::Spectate>,
) -> bool {
    // Bled out in `Zombies` and watching a teammate (`revive`): the view's
    // theirs, so no gameplay input either.
    if spectate.active() {
        return false;
    }
    // Also off for a `FreeForAll` match's end-of-match freeze (see
    // `match_end`: nothing counts, so no input does either), and from the
    // moment the local player is killed until their kill cam takes over —
    // dead players can't move, aim, shoot or throw (only the menu still works).
    // Paused by the party leader (`pause`): frozen in place for everyone.
    !menu.is_open() && !freeze.active && !death.is_active() && !paused.0
}

/// Run condition for the weapon: like [`game_active`], but it also keeps
/// running under Der Wunderfizz's menu, so a perk bought there is drunk
/// while it stays open (`weapon_system` itself takes no other input then).
/// Off while we're down in `Zombies`, or reviving someone (`revive`).
pub fn weapon_active(
    menu: Res<Menu>,
    freeze: Res<crate::match_end::MatchEndFreeze>,
    death: Res<crate::death_effect::DeathEffect>,
    paused: Res<crate::pause::GamePaused>,
    revive: Res<crate::revive::LocalRevive>,
) -> bool {
    (!menu.is_open() || menu.screen == Screen::Wunderfizz)
        && !freeze.active
        && !death.is_active()
        && !paused.0
        && !revive.blocks_weapon()
}

/// Run condition for the egui dev panels.
pub fn debug_enabled(settings: Res<Settings>) -> bool {
    settings.debug_mode
}

/// Marker for HUD elements (crosshair / ammo / fps) so they hide behind a menu.
#[derive(Component)]
pub struct HudElement;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Menu>()
            .add_systems(Startup, startup_menu.after(crate::grab_cursor))
            .add_systems(
                Update,
                (
                    (menu_toggle, rebind_capture).chain(),
                    username_input,
                    menu_click,
                    slider_drag,
                    refresh_dynamic,
                    watch_loadout,
                    show_match_results,
                    rebuild_menu,
                    cursor_and_hud,
                )
                    .chain(),
            )
            .add_systems(Update, scroll_hovered);
    }
}

fn startup_menu(mut menu: ResMut<Menu>, settings: Res<Settings>) {
    if !settings.has_username() {
        menu.screen = Screen::Username;
        menu.username_draft.clear();
        menu.dirty = true;
    }
}

/// The match clock ran out: show the results screen. Takes over from
/// whatever was open (there's nothing sensible to resume mid-results).
fn show_match_results(mut ended: EventReader<crate::MatchEndedEvent>, mut menu: ResMut<Menu>) {
    if ended.read().next().is_some() {
        menu.screen = Screen::MatchResults;
        menu.dirty = true;
    }
}

// ---- input ---------------------------------------------------------------

/// Esc opens the menu or backs out of it; the mouse back button backs out
/// the same way (but never opens it, and while a key is being rebound it's
/// left to `rebind_capture` to bind).
pub(crate) fn menu_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut menu: ResMut<Menu>,
    settings: Res<Settings>,
    state: Res<State<AppState>>,
) {
    // (The level editor has its own: `level_editor::input::edit_input`.)
    if *state.get() == AppState::LevelEditor {
        return;
    }
    // (Pack-a-Punch, Der Wunderfizz and the armor station close themselves
    // on it — once it's released, like their EXIT buttons: `pap_menu`,
    // `wunderfizz`, `armor`.)
    if mouse.just_pressed(MouseButton::Back)
        && menu.rebinding.is_none()
        && !matches!(menu.screen, Screen::None | Screen::PackAPunch | Screen::Wunderfizz | Screen::Armor)
    {
        go_back(&mut menu, &settings);
        return;
    }
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if menu.rebinding.is_some() {
        menu.rebinding = None;
        menu.dirty = true;
        return;
    }
    if menu.screen == Screen::None {
        menu.screen = if settings.has_username() {
            Screen::Settings
        } else {
            Screen::Username
        };
        menu.tab = Tab::Profile;
        menu.username_draft = settings.username.clone().unwrap_or_default();
        menu.dirty = true;
        return;
    }
    go_back(&mut menu, &settings);
}

/// Back out of the open menu screen one level — Esc / the mouse back button.
fn go_back(menu: &mut Menu, settings: &Settings) {
    let has_name = settings.has_username();
    match menu.screen {
        Screen::None => {}
        Screen::Settings => {
            menu.screen = if has_name {
                Screen::None
            } else {
                Screen::Username
            };
            menu.dirty = true;
        }
        Screen::Username => {
            if has_name {
                menu.screen = Screen::None;
                menu.dirty = true;
            }
        }
        // Dismissed only by its own CONTINUE button.
        Screen::MatchResults => {}
        // Dismissed only once every party member's client reports ready.
        Screen::LoadingGame => {}
        Screen::Loadout => menu.back_from_loadout(),
        Screen::PackAPunch | Screen::Wunderfizz | Screen::Armor | Screen::Changelog => {
            menu.screen = Screen::None;
            menu.dirty = true;
        }
    }
}

fn rebind_capture(
    mut menu: ResMut<Menu>,
    mut binds: ResMut<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    let Some(slot) = menu.rebinding else {
        return;
    };
    if !menu.rebind_armed {
        menu.rebind_armed = true;
        return;
    }
    if let Some(&code) = keys.get_just_pressed().next() {
        if code != KeyCode::Escape {
            binds.set_slot(slot, Binding::Key(code));
        }
        menu.rebinding = None;
        menu.dirty = true;
        return;
    }
    if let Some(&btn) = mouse.get_just_pressed().next() {
        binds.set_slot(slot, Binding::Mouse(btn));
        menu.rebinding = None;
        menu.dirty = true;
    }
}

fn username_input(
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut events: EventReader<KeyboardInput>,
) {
    let editing = matches!(menu.screen, Screen::Username)
        || (menu.screen == Screen::Settings && menu.tab == Tab::Profile);
    if !editing {
        events.clear();
        return;
    }
    for ev in events.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }
        match &ev.logical_key {
            Key::Character(s) => {
                for ch in s.chars() {
                    if !ch.is_control() && menu.username_draft.chars().count() < 20 {
                        menu.username_draft.push(ch);
                    }
                }
            }
            Key::Space => {
                if menu.username_draft.chars().count() < 20 {
                    menu.username_draft.push(' ');
                }
            }
            Key::Backspace => {
                menu.username_draft.pop();
            }
            Key::Enter => confirm_username(&mut menu, &mut settings),
            _ => {}
        }
    }
}

fn confirm_username(menu: &mut Menu, settings: &mut Settings) {
    let name = menu.username_draft.trim().to_string();
    if name.is_empty() {
        return;
    }
    settings.username = Some(name);
    if menu.screen == Screen::Username {
        menu.screen = Screen::None;
    }
    menu.dirty = true;
}

// ---- button / slider interaction ---------------------------------------

#[derive(Component, Clone)]
enum Btn {
    SelectTab(Tab),
    ToggleDebug,
    ToggleAutoReload,
    ToggleAutoCreate,
    ToggleAutoJoin,
    ToggleVsync,
    SetShadowQuality(ShadowQuality),
    SetCrosshair(CrosshairId),
    SetScopeZoom(ScopeZoom),
    SetAutoMantle(AutoMantle),
    /// Open the Loadout screen from the in-game pause menu — see
    /// `Screen::Loadout`'s doc comment. The main menu's own entry point is
    /// `lobby_ui::MenuBtn::OpenLoadout` instead.
    OpenLoadout,
    CloseLoadout,
    /// Back out of `Screen::Changelog`.
    CloseChangelog,
    /// Loadout screen: show this weapon, and equip it if the pick can change.
    SelectPrimary(WeaponId),
    OpenGunsmith(WeaponId),
    CloseGunsmith,
    Rebind(usize),
    ResetKeybinds,
    Step(SliderField, f32),
    ConfirmUsername,
    /// Leave the current game and return to the main menu: sends
    /// [`shared::LeaveLobby`], which pulls just this player (promoting a new
    /// leader if we were one) and lets the match continue for everyone else.
    LeaveGame,
    /// Party-leader only: end the match for the whole party
    /// ([`shared::EndGame`]) — every player is pulled to the main menu.
    LeaveWithParty,
    /// Dismiss the match-results screen and head back to the lobby room.
    ContinueFromResults,
    /// Party-leader only: pause / resume the game for the whole party
    /// ([`shared::SetPaused`]; see `pause`).
    TogglePause,
}

/// Which leave-game buttons the pause menu should show, derived each rebuild.
struct LeaveCtx {
    /// We're in a game; show a leave control at all.
    in_game: bool,
    /// We're the party leader of that match.
    is_leader: bool,
    /// That match is paused (`pause::GamePaused`).
    paused: bool,
    /// We're the only real player in it (bots don't count) — leaving with
    /// or without the party is the same thing, so there's just LEAVE GAME.
    solo: bool,
}

fn leave_ctx(
    app_state: &State<AppState>,
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&shared::Lobby>,
    paused: &crate::pause::GamePaused,
) -> LeaveCtx {
    let me = local.iter().next().map(|l| l.0);
    let my_lobby = me.and_then(|me| lobbies.iter().find(|l| l.has(me)));
    LeaveCtx {
        in_game: *app_state.get() == AppState::InGame,
        is_leader: matches!((me, my_lobby), (Some(me), Some(l)) if l.leader == me),
        paused: paused.0,
        solo: my_lobby.is_some_and(|l| l.real_count() <= 1),
    }
}

#[derive(Component, Clone, Copy, PartialEq)]
enum SliderField {
    Sensitivity,
    AdsSensitivity,
    AdsCoefficient,
    Fov,
    MasterVolume,
    MusicVolume,
    FrameLimit,
}

#[derive(Component)]
struct SliderTrack(SliderField);
#[derive(Component)]
struct SliderFill(SliderField);

/// Text nodes patched in place by [`refresh_dynamic`].
#[derive(Component)]
enum DynText {
    SliderValue(SliderField),
    Debug,
    Username,
}

#[allow(clippy::too_many_arguments)]
fn menu_click(
    q: Query<(&Interaction, &Btn), Changed<Interaction>>,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut binds: ResMut<KeyBindings>,
    mut next: ResMut<NextState<AppState>>,
    mut leave_lobby: Query<&mut TriggerSender<shared::LeaveLobby>, With<GameClient>>,
    mut end_game: Query<&mut TriggerSender<shared::EndGame>, With<GameClient>>,
    mut set_paused: Query<&mut TriggerSender<shared::SetPaused>, With<GameClient>>,
    paused: Res<crate::pause::GamePaused>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
) {
    for (interaction, btn) in &q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn {
            Btn::SelectTab(tab) => {
                menu.tab = *tab;
                menu.rebinding = None;
                if *tab == Tab::Profile {
                    menu.username_draft = settings.username.clone().unwrap_or_default();
                }
                menu.dirty = true;
            }
            Btn::ToggleDebug => settings.debug_mode = !settings.debug_mode,
            Btn::ToggleAutoReload => {
                settings.auto_reload = !settings.auto_reload;
                menu.dirty = true;
            }
            Btn::SetAutoMantle(mode) => {
                settings.auto_mantle = *mode;
                menu.dirty = true;
            }
            Btn::ToggleAutoCreate => {
                settings.dev_auto_create_lobby = !settings.dev_auto_create_lobby;
                menu.dirty = true;
            }
            Btn::ToggleAutoJoin => {
                settings.dev_auto_join_lobby = !settings.dev_auto_join_lobby;
                menu.dirty = true;
            }
            Btn::SetShadowQuality(q) => {
                settings.shadow_quality = *q;
                menu.dirty = true;
            }
            Btn::ToggleVsync => {
                settings.vsync = !settings.vsync;
                menu.dirty = true;
            }
            Btn::SetCrosshair(id) => {
                settings.crosshair = *id;
                menu.dirty = true;
            }
            Btn::SetScopeZoom(z) => {
                settings.scope_zoom = *z;
                menu.dirty = true;
            }
            Btn::OpenLoadout => menu.open_loadout(None),
            Btn::CloseChangelog => {
                menu.screen = Screen::None;
                menu.dirty = true;
            }
            Btn::CloseLoadout => {
                menu.gunsmith = None;
                menu.back_from_loadout();
            }
            Btn::SelectPrimary(weapon) => {
                menu.loadout_focus = Some(*weapon);
                if !LoadoutCtx::new(&local, &lobbies).locked() {
                    settings.primary = *weapon;
                }
                menu.dirty = true;
            }
            Btn::OpenGunsmith(weapon) => {
                menu.gunsmith = Some(*weapon);
                menu.dirty = true;
            }
            Btn::CloseGunsmith => {
                menu.gunsmith = None;
                menu.dirty = true;
            }
            Btn::Rebind(i) => {
                menu.rebinding = Some(*i);
                menu.rebind_armed = false;
                menu.dirty = true;
            }
            Btn::ResetKeybinds => *binds = KeyBindings::default(),
            Btn::Step(field, delta) => step_field(&mut settings, *field, *delta),
            Btn::ConfirmUsername => confirm_username(&mut menu, &mut settings),
            Btn::LeaveGame => {
                menu.screen = Screen::None;
                menu.dirty = true;
                // Non-leader, or leader "leave without party": ask the server
                // to pull just us (it promotes a new leader if needed). The
                // main-menu jump happens in `drive_ingame_exit` once the
                // server drops us — same flow as the lobby-room LEAVE button,
                // so there's no bounce through the lobby screen.
                if let Ok(mut s) = leave_lobby.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::LeaveLobby);
                }
            }
            Btn::LeaveWithParty => {
                menu.screen = Screen::None;
                menu.dirty = true;
                // End the match for everyone; `drive_ingame_exit` returns each
                // client to the main menu when the lobby disbands.
                if let Ok(mut s) = end_game.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::EndGame);
                }
            }
            Btn::TogglePause => {
                // The button's label flips once the server's answer is
                // replicated back (`pause::sync_paused` rebuilds the menu).
                if let Ok(mut s) = set_paused.single_mut() {
                    s.trigger::<shared::LobbyChannel>(shared::SetPaused { paused: !paused.0 });
                }
            }
            Btn::ContinueFromResults => {
                menu.screen = Screen::None;
                menu.dirty = true;
                // The server already flipped `Lobby.started` false; nothing
                // else drives this transition today (a normally-finished
                // match, unlike leaving, doesn't disband the lobby), so this
                // button is what actually returns everyone to the room.
                next.set(AppState::InLobby);
            }
        }
    }
}

fn step_field(settings: &mut Settings, field: SliderField, delta: f32) {
    match field {
        SliderField::Sensitivity => {
            settings.sensitivity = (settings.sensitivity + delta).clamp(SENS_MIN, SENS_MAX);
        }
        SliderField::AdsSensitivity => {
            settings.ads_sens_multiplier =
                (settings.ads_sens_multiplier + delta).clamp(ADS_SENS_MIN, ADS_SENS_MAX);
        }
        SliderField::AdsCoefficient => {
            settings.ads_sens_coefficient =
                (settings.ads_sens_coefficient + delta).clamp(ADS_COEFF_MIN, ADS_COEFF_MAX);
        }
        SliderField::Fov => {
            settings.fov = (settings.fov + delta).clamp(FOV_MIN, FOV_MAX).round();
        }
        SliderField::MasterVolume => {
            settings.master_volume =
                (settings.master_volume + delta).clamp(VOLUME_MIN, VOLUME_MAX);
        }
        SliderField::MusicVolume => {
            settings.music_volume =
                (settings.music_volume + delta).clamp(VOLUME_MIN, VOLUME_MAX);
        }
        SliderField::FrameLimit => {
            settings.frame_limit =
                (settings.frame_limit + delta).clamp(FRAME_LIMIT_MIN, FRAME_LIMIT_MAX);
        }
    }
}

fn slider_drag(
    tracks: Query<(&Interaction, &SliderTrack, &RelativeCursorPosition)>,
    mut settings: ResMut<Settings>,
) {
    for (interaction, track, rel) in &tracks {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(pos) = rel.normalized else { continue };
        let t = pos.x.clamp(0.0, 1.0);
        match track.0 {
            SliderField::Sensitivity => {
                settings.sensitivity =
                    (SENS_MIN + t * (SENS_MAX - SENS_MIN)).clamp(SENS_MIN, SENS_MAX);
            }
            SliderField::AdsSensitivity => {
                settings.ads_sens_multiplier = (ADS_SENS_MIN + t * (ADS_SENS_MAX - ADS_SENS_MIN))
                    .clamp(ADS_SENS_MIN, ADS_SENS_MAX);
            }
            SliderField::AdsCoefficient => {
                // Snapped to hundredths, so 1.33 can be hit exactly.
                settings.ads_sens_coefficient = ((ADS_COEFF_MIN + t * (ADS_COEFF_MAX - ADS_COEFF_MIN)) * 100.0)
                    .round()
                    .clamp(ADS_COEFF_MIN * 100.0, ADS_COEFF_MAX * 100.0)
                    / 100.0;
            }
            SliderField::Fov => {
                settings.fov = (FOV_MIN + t * (FOV_MAX - FOV_MIN))
                    .round()
                    .clamp(FOV_MIN, FOV_MAX);
            }
            SliderField::MasterVolume => {
                settings.master_volume =
                    (VOLUME_MIN + t * (VOLUME_MAX - VOLUME_MIN)).clamp(VOLUME_MIN, VOLUME_MAX);
            }
            SliderField::MusicVolume => {
                settings.music_volume =
                    (VOLUME_MIN + t * (VOLUME_MAX - VOLUME_MIN)).clamp(VOLUME_MIN, VOLUME_MAX);
            }
            SliderField::FrameLimit => {
                settings.frame_limit = (FRAME_LIMIT_MIN + t * (FRAME_LIMIT_MAX - FRAME_LIMIT_MIN))
                    .round()
                    .clamp(FRAME_LIMIT_MIN, FRAME_LIMIT_MAX);
            }
        }
    }
}

fn field_fraction(settings: &Settings, field: SliderField) -> f32 {
    match field {
        SliderField::Sensitivity => (settings.sensitivity - SENS_MIN) / (SENS_MAX - SENS_MIN),
        SliderField::AdsSensitivity => {
            (settings.ads_sens_multiplier - ADS_SENS_MIN) / (ADS_SENS_MAX - ADS_SENS_MIN)
        }
        SliderField::AdsCoefficient => {
            (settings.ads_sens_coefficient - ADS_COEFF_MIN) / (ADS_COEFF_MAX - ADS_COEFF_MIN)
        }
        SliderField::Fov => (settings.fov - FOV_MIN) / (FOV_MAX - FOV_MIN),
        SliderField::MasterVolume => {
            (settings.master_volume - VOLUME_MIN) / (VOLUME_MAX - VOLUME_MIN)
        }
        SliderField::MusicVolume => {
            (settings.music_volume - VOLUME_MIN) / (VOLUME_MAX - VOLUME_MIN)
        }
        SliderField::FrameLimit => {
            (settings.frame_limit - FRAME_LIMIT_MIN) / (FRAME_LIMIT_MAX - FRAME_LIMIT_MIN)
        }
    }
    .clamp(0.0, 1.0)
}

fn field_value_text(settings: &Settings, field: SliderField) -> String {
    match field {
        SliderField::Sensitivity => format!("{:.2}", settings.sensitivity),
        SliderField::AdsSensitivity => format!("{:.2}", settings.ads_sens_multiplier),
        SliderField::AdsCoefficient => format!("{:.2}", settings.ads_sens_coefficient),
        SliderField::Fov => format!("{:.0}", settings.fov),
        SliderField::MasterVolume => format!("{:.0}%", settings.master_volume * 100.0),
        SliderField::MusicVolume => format!("{:.0}%", settings.music_volume * 100.0),
        SliderField::FrameLimit => format!("{:.0}", settings.frame_limit),
    }
}

fn refresh_dynamic(
    settings: Res<Settings>,
    menu: Res<Menu>,
    mut fills: Query<(&SliderFill, &mut Node)>,
    mut texts: Query<(&DynText, &mut Text)>,
) {
    if !(settings.is_changed() || menu.is_changed()) {
        return;
    }
    for (fill, mut node) in &mut fills {
        node.width = Val::Percent(field_fraction(&settings, fill.0) * 100.0);
    }
    for (dynt, mut text) in &mut texts {
        let s = match dynt {
            DynText::SliderValue(f) => field_value_text(&settings, *f),
            DynText::Debug => if settings.debug_mode { "ON" } else { "OFF" }.to_string(),
            DynText::Username => {
                if menu.username_draft.is_empty() {
                    "_".to_string()
                } else {
                    format!("{}|", menu.username_draft)
                }
            }
        };
        *text = Text::new(s);
    }
}

/// Rebuild the open Loadout screen when the game changes under it — a
/// match starting or ending, or the server handing us a new gun.
fn watch_loadout(
    mut menu: ResMut<Menu>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    mut last: Local<Option<(Option<shared::GameMode>, bool, Option<WeaponId>)>>,
) {
    if menu.screen != Screen::Loadout {
        *last = None;
        return;
    }
    let ctx = LoadoutCtx::new(&local, &lobbies);
    let key = (ctx.mode, ctx.started, ctx.carrying);
    if last.is_some_and(|k| k != key) {
        menu.dirty = true;
    }
    *last = Some(key);
}

/// Scrolls whichever scrollable node the pointer is currently over (e.g. the
/// keybinds list) in response to the mouse wheel.
fn scroll_hovered(
    mut wheel: EventReader<MouseWheel>,
    hover_map: Res<HoverMap>,
    mut scrollable: Query<&mut ScrollPosition>,
) {
    for ev in wheel.read() {
        let dy = match ev.unit {
            MouseScrollUnit::Line => ev.y * 21.0,
            MouseScrollUnit::Pixel => ev.y,
        };
        for pointer_map in hover_map.values() {
            for &entity in pointer_map.keys() {
                if let Ok(mut pos) = scrollable.get_mut(entity) {
                    pos.offset_y -= dy;
                }
            }
        }
    }
}

fn cursor_and_hud(
    menu: Res<Menu>,
    app_state: Res<State<crate::AppState>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !menu.is_changed() {
        return;
    }
    // Only the in-game screen owns the cursor; the main-menu / lobby screens are
    // `bevy_ui` and always want it free, open settings overlay or not. HUD
    // visibility is handled by `crate::hud_visibility` (it also needs the state).
    if *app_state == crate::AppState::InGame {
        let open = menu.is_open();
        for mut window in &mut windows {
            crate::set_cursor_grabbed(&mut window, !open);
        }
    }
}

// ---- rebuild -----------------------------------------------------------

#[derive(Component)]
struct MenuRoot;

#[allow(clippy::too_many_arguments)]
fn rebuild_menu(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    settings: Res<Settings>,
    binds: Res<KeyBindings>,
    app_state: Res<State<AppState>>,
    asset_server: Res<AssetServer>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&shared::Lobby>,
    paused: Res<crate::pause::GamePaused>,
    existing: Query<Entity, With<MenuRoot>>,
) {
    if !menu.dirty {
        return;
    }
    menu.dirty = false;
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    // Out of a game there's no world to see through to — only the main menu.
    let solid = *app_state.get() != AppState::InGame;
    match menu.screen {
        Screen::None => {}
        Screen::Username => build_username(&mut commands, &asset_server),
        Screen::Settings => {
            let leave = leave_ctx(&app_state, &local, &lobbies, &paused);
            build_settings(&mut commands, &asset_server, &menu, &settings, &binds, &leave, solid);
        }
        Screen::MatchResults => build_match_results(&mut commands, &asset_server, &local, &lobbies),
        // Built by `game_start`, not here — see `Screen::LoadingGame`'s doc comment.
        Screen::LoadingGame => {}
        Screen::Loadout => {
            let ctx = LoadoutCtx::new(&local, &lobbies);
            match menu.gunsmith {
                Some(weapon) => build_gunsmith(&mut commands, &settings, &asset_server, weapon, solid),
                None => {
                    let focus = menu.loadout_focus.unwrap_or(settings.primary);
                    build_loadout(&mut commands, &settings, &asset_server, &ctx, focus, solid);
                }
            }
        }
        // Built by `pap_menu`, not here — see `Screen::PackAPunch`'s doc comment.
        Screen::PackAPunch => {}
        // Built by `wunderfizz`, likewise.
        Screen::Wunderfizz => {}
        // Built by `armor`, likewise.
        Screen::Armor => {}
        Screen::Changelog => build_changelog(&mut commands, &asset_server, solid),
    }
}

fn overlay_root(solid: bool) -> impl Bundle {
    (
        MenuRoot,
        GlobalZIndex(50),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(if solid { PANEL_SOLID } else { BACKDROP }),
    )
}

/// A full-screen page (settings / loadout): the standard margins, black —
/// see-through in game, so the world stays visible behind; `solid` (the main
/// menu) so the lobby browser underneath doesn't show through — its builder
/// then lays the menu backdrop art in first (`ui::menu_background`).
fn page_root(solid: bool) -> impl Bundle {
    (
        MenuRoot,
        GlobalZIndex(50),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::axes(Val::Px(56.0), Val::Px(40.0)),
            row_gap: Val::Px(16.0),
            ..default()
        },
        BackgroundColor(if solid { PANEL_SOLID } else { BACKDROP }),
    )
}

/// A line of explanatory copy under a setting.
fn desc(content: &mut ChildSpawnerCommands, asset_server: &AssetServer, text: &str) {
    content.spawn((
        label_body(asset_server, text, 14.0, TEXT_DIM),
        Node {
            max_width: Val::Px(760.0),
            margin: UiRect::bottom(Val::Px(6.0)),
            ..default()
        },
    ));
}

/// One settings line: the setting's name on the left, its controls after it,
/// a hairline underneath.
fn setting_row(
    content: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    name: &str,
    controls: impl FnOnce(&mut ChildSpawnerCommands),
) {
    content
        .spawn((
            Node {
                width: Val::Percent(100.0),
                max_width: Val::Px(1100.0),
                min_height: Val::Px(56.0),
                align_items: AlignItems::Center,
                column_gap: Val::Px(16.0),
                padding: UiRect::vertical(Val::Px(6.0)),
                border: UiRect::bottom(Val::Px(1.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BorderColor(EDGE),
        ))
        .with_children(|row| {
            row.spawn(Node {
                width: Val::Px(280.0),
                flex_shrink: 0.0,
                ..default()
            })
            .with_child(label_hud(asset_server, name, 22.0, TEXT));
            row.spawn(Node {
                flex_grow: 1.0,
                flex_wrap: FlexWrap::Wrap,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(controls);
        });
}

/// A button in the menus' condensed caps. `selected` gives it the gold
/// "current choice" look.
#[allow(clippy::too_many_arguments)]
fn option_button(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    text: &str,
    btn: Btn,
    selected: bool,
    sfx: UiSound,
) {
    spawn_button_hud(
        parent,
        asset_server,
        text,
        18.0,
        btn,
        if selected { ACCENT } else { ROW },
        ROW_HOVER,
        if selected { PANEL_SOLID } else { TEXT },
        sfx,
    );
}

/// A plain (unselected-look) menu button.
fn plain_button(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    text: &str,
    btn: Btn,
    sfx: UiSound,
) {
    spawn_button_hud(parent, asset_server, text, 18.0, btn, ROW, ROW_HOVER, TEXT, sfx);
}

/// An ON / OFF toggle button (its text optionally live-patched by
/// [`refresh_dynamic`]).
fn toggle_button(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    on: bool,
    btn: Btn,
    live: Option<DynText>,
) {
    let (normal, text) = if on { (ACCENT, PANEL_SOLID) } else { (ROW, TEXT) };
    parent
        .spawn((
            Button,
            Interaction::default(),
            btn,
            Hoverable::new(normal, ROW_HOVER, text),
            ui_sound(UiSound::BUTTON),
            Node {
                min_width: Val::Px(96.0),
                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(normal),
        ))
        .with_children(|b| {
            let mut t = b.spawn(label_hud(asset_server, if on { "ON" } else { "OFF" }, 18.0, text));
            if let Some(live) = live {
                t.insert(live);
            }
        });
}

fn build_username(commands: &mut Commands, asset_server: &AssetServer) {
    commands.spawn(overlay_root(true)).with_children(|root| {
        root.spawn(Node {
            width: Val::Px(560.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(18.0),
            ..default()
        })
        .with_children(|card| {
            page_title(card, asset_server, "WELCOME", "CHOOSE A USERNAME");
            card.spawn(divider());
            card.spawn(label_body(
                asset_server,
                "This is how other players will see you. You can change it later in Settings.",
                15.0,
                TEXT_DIM,
            ));
            card.spawn(field_box(560.0)).with_children(|f| {
                f.spawn((label_hud(asset_server, "", 26.0, TEXT), DynText::Username));
            });
            card.spawn(Node::default()).with_children(|row| {
                option_button(
                    row,
                    asset_server,
                    "CONFIRM",
                    Btn::ConfirmUsername,
                    true,
                    UiSound::MENU,
                );
            });
        });
    });
}

/// One member's line on the results screen.
struct ResultRow {
    name: String,
    me: bool,
    /// One per column, the first (the ranking one) shown biggest.
    values: Vec<String>,
}

/// The results screen's look (Call of Duty: Cold War zombies' end screen):
/// over the world, only lightly darkened...
const RESULTS_SHADE: Color = Color::srgba(0.0, 0.0, 0.0, 0.4);
/// ...its headline in warm white, the line under it in blood red...
const RESULTS_TITLE: Color = Color::srgb(0.94, 0.89, 0.81);
const RESULTS_RED: Color = Color::srgb(0.86, 0.09, 0.09);
/// ...the table on a dark glass panel under a red header bar...
const RESULTS_PANEL: Color = Color::srgba(0.07, 0.07, 0.08, 0.86);
const RESULTS_HEADER: Color = Color::srgb(0.6, 0.04, 0.05);
const RESULTS_LINE: Color = Color::srgba(1.0, 1.0, 1.0, 0.1);
/// ...and the numbers in gold.
const RESULTS_GOLD: Color = Color::srgb(0.98, 0.79, 0.24);
/// The table: rank column, value columns (the player column takes the rest).
const RANK_W: f32 = 80.0;
const VALUE_W: f32 = 170.0;

/// "1st", "2nd", "3rd", "4th"...
fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The end-of-game screen: the headline and what it means over the world
/// (still showing, just darkened), then every member ranked on a panel
/// across the middle — a column per stat — and CONTINUE.
fn results_screen(
    commands: &mut Commands,
    asset_server: &AssetServer,
    (title, title_color): (&str, Color),
    subtitle: &str,
    headings: &[&str],
    rows: &[ResultRow],
) {
    let column = |width: f32| Node {
        width: Val::Px(width),
        flex_shrink: 0.0,
        height: Val::Percent(100.0),
        justify_content: JustifyContent::FlexEnd,
        align_items: AlignItems::Center,
        padding: UiRect::right(Val::Px(18.0)),
        ..default()
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(RESULTS_SHADE),
        ))
        .with_children(|root| {
            root.spawn((label_hud(asset_server, title, 112.0, title_color), TextShadow::default()));
            root.spawn((
                Node {
                    width: Val::Px(560.0),
                    height: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.22)),
            ));
            root.spawn((label_hud(asset_server, subtitle, 38.0, RESULTS_RED), TextShadow::default()));
            root.spawn(Node {
                height: Val::Px(18.0),
                ..default()
            });

            // The table.
            root.spawn((
                Node {
                    width: Val::Px(RANK_W + 380.0 + VALUE_W * headings.len() as f32),
                    max_width: Val::Percent(94.0),
                    min_height: Val::Px(360.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                BackgroundColor(RESULTS_PANEL),
            ))
            .with_children(|panel| {
                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(40.0),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        BackgroundColor(RESULTS_HEADER),
                        BorderRadius::top_right(Val::Px(18.0)),
                    ))
                    .with_children(|head| {
                        head.spawn(Node {
                            width: Val::Px(RANK_W),
                            flex_shrink: 0.0,
                            ..default()
                        });
                        head.spawn(Node {
                            flex_grow: 1.0,
                            ..default()
                        });
                        for h in headings {
                            head.spawn(column(VALUE_W)).with_child(label_hud(asset_server, *h, 20.0, TEXT));
                        }
                    });
                for (i, row) in rows.iter().enumerate() {
                    let name_color = if row.me { ACCENT } else { TEXT };
                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(78.0),
                                flex_shrink: 0.0,
                                align_items: AlignItems::Center,
                                border: UiRect::bottom(Val::Px(1.0)),
                                ..default()
                            },
                            BorderColor(RESULTS_LINE),
                        ))
                        .with_children(|line| {
                            line.spawn(Node {
                                width: Val::Px(RANK_W),
                                flex_shrink: 0.0,
                                justify_content: JustifyContent::Center,
                                ..default()
                            })
                            .with_child(label_hud(asset_server, ordinal(i + 1), 30.0, TEXT));
                            // The player's plate (ours edged in gold).
                            line.spawn(Node {
                                flex_grow: 1.0,
                                height: Val::Percent(100.0),
                                padding: UiRect::vertical(Val::Px(8.0)),
                                ..default()
                            })
                            .with_child((
                                Node {
                                    width: Val::Px(340.0),
                                    max_width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    align_items: AlignItems::Center,
                                    padding: UiRect::left(Val::Px(16.0)),
                                    border: UiRect::left(Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.06)),
                                BorderColor(if row.me { ACCENT } else { Color::srgba(1.0, 1.0, 1.0, 0.2) }),
                                children![label_hud(asset_server, row.name.to_uppercase(), 30.0, name_color)],
                            ));
                            for (k, value) in row.values.iter().enumerate() {
                                // (The ranking column biggest.)
                                let size = if k == 0 { 40.0 } else { 32.0 };
                                line.spawn((
                                    Node {
                                        border: UiRect::left(Val::Px(1.0)),
                                        height: Val::Percent(70.0),
                                        ..column(VALUE_W)
                                    },
                                    BorderColor(RESULTS_LINE),
                                ))
                                .with_child(label_hud(asset_server, value.clone(), size, RESULTS_GOLD));
                            }
                        });
                }
            });

            root.spawn(Node {
                margin: UiRect::top(Val::Px(10.0)),
                ..default()
            })
            .with_children(|row| {
                option_button(row, asset_server, "CONTINUE", Btn::ContinueFromResults, true, UiSound::MENU);
            });
        });
}

/// A number with thousands separators ("54,650").
fn grouped(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn build_match_results(
    commands: &mut Commands,
    asset_server: &AssetServer,
    local: &Query<&LocalId, With<GameClient>>,
    lobbies: &Query<&shared::Lobby>,
) {
    let me = local.iter().next().map(|l| l.0);
    let lobby = me.and_then(|me| lobbies.iter().find(|l| l.has(me)));
    let Some(lobby) = lobby else {
        results_screen(commands, asset_server, ("MATCH COMPLETE", RESULTS_TITLE), "", &[], &[]);
        return;
    };
    let mut members: Vec<&shared::LobbyMember> = lobby.members.iter().collect();
    members.sort_by(|a, b| b.score.cmp(&a.score).then(b.kills.cmp(&a.kills)));
    let is_me = |m: &shared::LobbyMember| Some(m.peer) == me;

    if lobby.mode == shared::GameMode::Zombies {
        // Down during round N: N - 1 rounds fully survived.
        let survived = lobby.round.saturating_sub(1);
        let rows: Vec<ResultRow> = members
            .iter()
            .map(|m| ResultRow {
                name: m.name.clone(),
                me: is_me(m),
                values: vec![
                    grouped(m.score),
                    m.kills.to_string(),
                    m.critical_kills.to_string(),
                    m.revives.to_string(),
                    m.downs.to_string(),
                ],
            })
            .collect();
        results_screen(
            commands,
            asset_server,
            ("GAME OVER", RESULTS_TITLE),
            &format!("YOU SURVIVED {survived} ROUND{}", if survived == 1 { "" } else { "S" }),
            &["SCORE", "KILLS", "CRITICAL KILLS", "REVIVES", "DOWNS"],
            &rows,
        );
        return;
    }

    // `FreeForAll` (score = kills) and `Freestyle` (score = style points): a
    // tie for the top still counts as first place.
    let top = members.first().map(|m| (m.name.to_uppercase(), m.score));
    let won = top.as_ref().is_some_and(|(_, best)| members.iter().any(|m| is_me(m) && m.score == *best));
    let unit = if lobby.mode == shared::GameMode::FreeForAll { "KILLS" } else { "POINTS" };
    let subtitle = top.map_or(String::new(), |(name, best)| format!("{name} WINS WITH {} {unit}", grouped(best)));
    let rows: Vec<ResultRow> = members
        .iter()
        .map(|m| ResultRow {
            name: m.name.clone(),
            me: is_me(m),
            values: vec![grouped(m.score)],
        })
        .collect();
    let headline = if won { ("VICTORY", VICTORY) } else { ("DEFEAT", DEFEAT) };
    let heading = if lobby.mode == shared::GameMode::FreeForAll { "KILLS" } else { "SCORE" };
    results_screen(commands, asset_server, headline, &subtitle, &[heading], &rows);
}

/// A tab in the settings screen's top bar: bright with a gold underline when
/// it's the open one.
fn tab_button(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    text: &str,
    btn: Btn,
    selected: bool,
) {
    let text_color = if selected { TEXT } else { TEXT_DIM };
    parent
        .spawn((
            Button,
            Interaction::default(),
            btn,
            Hoverable::new(Color::NONE, ROW_HOVER, text_color),
            ui_sound(UiSound::BUTTON),
            Node {
                padding: UiRect::axes(Val::Px(18.0), Val::Px(8.0)),
                border: UiRect::bottom(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            BorderColor(if selected { ACCENT } else { Color::NONE }),
        ))
        .with_child(label_hud(asset_server, text, 26.0, text_color));
}

fn build_settings(
    commands: &mut Commands,
    asset_server: &AssetServer,
    menu: &Menu,
    settings: &Settings,
    binds: &KeyBindings,
    leave: &LeaveCtx,
    solid: bool,
) {
    commands.spawn(page_root(solid)).with_children(|page| {
        if solid {
            menu_background(page, asset_server);
        }
        page_title(
            page,
            asset_server,
            if leave.in_game { "IN GAME" } else { "MAIN MENU" },
            "SETTINGS",
        );

        // tabs across the top, the Loadout at the far end
        page.spawn(Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::FlexEnd,
            column_gap: Val::Px(4.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|tabs| {
            for (tab, name) in [
                (Tab::Profile, "PROFILE"),
                (Tab::Controls, "CONTROLS"),
                (Tab::Graphics, "GRAPHICS"),
                (Tab::Audio, "AUDIO"),
                (Tab::Keybinds, "KEYBINDS"),
                (Tab::Multiplayer, "MULTIPLAYER"),
            ] {
                tab_button(tabs, asset_server, name, Btn::SelectTab(tab), menu.tab == tab);
            }
            tabs.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            plain_button(tabs, asset_server, "LOADOUT", Btn::OpenLoadout, UiSound::BUTTON);
        });
        page.spawn(divider());

        page.spawn(Node {
            width: Val::Percent(100.0),
            flex_grow: 1.0,
            flex_basis: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::vertical(Val::Px(8.0)),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|content| match menu.tab {
            Tab::Profile => build_profile(content, asset_server),
            Tab::Controls => build_controls(content, asset_server, settings),
            Tab::Graphics => build_graphics(content, asset_server, settings),
            Tab::Audio => build_audio(content, asset_server, settings),
            Tab::Keybinds => build_keybinds(content, asset_server, menu, binds),
            Tab::Multiplayer => build_multiplayer(content, asset_server, settings),
        });

        // footer: key hints left; in game, the pause / leave actions right
        page.spawn(divider());
        page.spawn(Node {
            width: Val::Percent(100.0),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: Val::Px(24.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|f| {
            f.spawn(Node {
                column_gap: Val::Px(28.0),
                ..default()
            })
            .with_children(|hints| {
                hints.spawn(label_hud(
                    asset_server,
                    if leave.in_game { "[ESC]  RESUME" } else { "[ESC]  BACK" },
                    18.0,
                    TEXT_DIM,
                ));
                hints.spawn(label_hud(asset_server, "CHANGES SAVE AUTOMATICALLY", 18.0, TEXT_DIM));
            });

            if leave.in_game {
                f.spawn(Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    ..default()
                })
                .with_children(|actions| {
                    if leave.solo {
                        // Alone: one way out (the lobby goes with us).
                        actions.spawn(label_body(asset_server, "Leaving ends the match.", 13.0, TEXT_DIM));
                        option_button(
                            actions,
                            asset_server,
                            if leave.paused { "RESUME GAME" } else { "PAUSE GAME" },
                            Btn::TogglePause,
                            true,
                            UiSound::BUTTON,
                        );
                        plain_button(actions, asset_server, "LEAVE GAME", Btn::LeaveGame, UiSound::BUTTON_BACK);
                    } else if leave.is_leader {
                        actions.spawn((
                            label_body(
                                asset_server,
                                "Leaving without the party promotes a new leader; the match continues.",
                                13.0,
                                TEXT_DIM,
                            ),
                            Node {
                                max_width: Val::Px(260.0),
                                ..default()
                            },
                        ));
                        option_button(
                            actions,
                            asset_server,
                            if leave.paused { "RESUME GAME" } else { "PAUSE GAME" },
                            Btn::TogglePause,
                            true,
                            UiSound::BUTTON,
                        );
                        plain_button(
                            actions,
                            asset_server,
                            "LEAVE WITH PARTY",
                            Btn::LeaveWithParty,
                            UiSound::BUTTON_BACK,
                        );
                        plain_button(
                            actions,
                            asset_server,
                            "LEAVE WITHOUT PARTY",
                            Btn::LeaveGame,
                            UiSound::BUTTON_BACK,
                        );
                    } else {
                        actions.spawn(label_body(
                            asset_server,
                            "The match continues for the other players.",
                            13.0,
                            TEXT_DIM,
                        ));
                        plain_button(
                            actions,
                            asset_server,
                            "LEAVE GAME",
                            Btn::LeaveGame,
                            UiSound::BUTTON_BACK,
                        );
                    }
                });
            }
        });
    });
}

/// One Loadout tile: gold-edged and marked EQUIPPED when it's the current
/// choice. (Hover is a light lift rather than the white fill, so the reticle
/// images stay readable.)
fn loadout_tile(
    row: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    btn: Btn,
    selected: bool,
    contents: impl FnOnce(&mut ChildSpawnerCommands),
) {
    let normal = if selected { ACCENT_DIM } else { ROW };
    row.spawn((
        Button,
        Interaction::default(),
        btn,
        Hoverable {
            normal,
            hover: Color::srgba(1.0, 1.0, 1.0, 0.18),
            text: None,
        },
        ui_sound(UiSound::BUTTON),
        Node {
            width: Val::Px(200.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::all(Val::Px(14.0)),
            row_gap: Val::Px(10.0),
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(normal),
        BorderColor(if selected { ACCENT } else { EDGE }),
    ))
    .with_children(|tile| {
        contents(tile);
        tile.spawn(label_hud(
            asset_server,
            if selected { "EQUIPPED" } else { " " },
            15.0,
            ACCENT,
        ));
    });
}

/// One primary in the Loadout screen's list: a gold edge and wash when it's
/// the one shown on the right, EQUIPPED when it's our pick.
fn loadout_weapon_row(
    list: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    weapon: WeaponId,
    focused: bool,
    equipped: bool,
) {
    let normal = if focused { ACCENT_DIM } else { ROW };
    list.spawn((
        Button,
        Interaction::default(),
        Btn::SelectPrimary(weapon),
        Hoverable {
            normal,
            hover: Color::srgba(1.0, 1.0, 1.0, 0.18),
            text: None,
        },
        ui_sound(UiSound::BUTTON),
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(96.0),
            align_items: AlignItems::Center,
            column_gap: Val::Px(16.0),
            padding: UiRect::new(Val::Px(12.0), Val::Px(18.0), Val::ZERO, Val::ZERO),
            border: UiRect::left(Val::Px(4.0)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(normal),
        BorderColor(if focused { ACCENT } else { Color::NONE }),
    ))
    .with_children(|row| {
        row.spawn((
            ImageNode::new(asset_server.load(crate::hud::primary_icon_path(weapon))),
            Node {
                width: Val::Px(144.0),
                height: Val::Px(72.0),
                flex_shrink: 0.0,
                ..default()
            },
        ));
        row.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|c| {
            c.spawn(label_hud(asset_server, weapon.label(), 28.0, TEXT));
            c.spawn(label_hud(asset_server, weapon.class_label(), 15.0, TEXT_DIM));
        });
        if equipped {
            row.spawn(label_hud(asset_server, "EQUIPPED", 16.0, ACCENT));
        }
    });
}

/// One of the weapon card's stat bars: its name, then a track filled to
/// `value` (0..1).
fn stat_bar(parent: &mut ChildSpawnerCommands, asset_server: &AssetServer, name: &str, value: f32) {
    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            max_width: Val::Px(620.0),
            align_items: AlignItems::Center,
            column_gap: Val::Px(14.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|r| {
            r.spawn((
                label_hud(asset_server, name, 17.0, TEXT_DIM),
                Node {
                    width: Val::Px(110.0),
                    flex_shrink: 0.0,
                    ..default()
                },
            ));
            r.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(TRACK),
            ))
            .with_child((
                Node {
                    width: Val::Percent(value.clamp(0.0, 1.0) * 100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(TEXT),
            ));
        });
}

/// The Loadout screen, Call of Duty style — see `Screen::Loadout`'s doc
/// comment. `focus` is the weapon shown large on the right.
fn build_loadout(
    commands: &mut Commands,
    settings: &Settings,
    asset_server: &AssetServer,
    ctx: &LoadoutCtx,
    focus: WeaponId,
    solid: bool,
) {
    let breadcrumb = ctx.mode.map_or("MULTIPLAYER".to_string(), |m| m.label().to_uppercase());
    commands.spawn(page_root(solid)).with_children(|page| {
        if solid {
            menu_background(page, asset_server);
        }
        page_title(page, asset_server, &breadcrumb, "LOADOUT");
        page.spawn(divider());

        page.spawn(Node {
            width: Val::Percent(100.0),
            flex_grow: 1.0,
            flex_basis: Val::Px(0.0),
            min_height: Val::Px(0.0),
            column_gap: Val::Px(32.0),
            padding: UiRect::vertical(Val::Px(8.0)),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|body| {
            // The weapons, down the left.
            body.spawn(Node {
                width: Val::Px(480.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|list| {
                section_heading(list, asset_server, "PRIMARY WEAPON");
                for weapon in LOADOUT_WEAPONS {
                    loadout_weapon_row(list, asset_server, weapon, weapon == focus, weapon == settings.primary);
                }
            });

            // The one picked, large, with its stats and actions.
            body.spawn(panel_node(Node {
                flex_grow: 1.0,
                flex_basis: Val::Px(0.0),
                min_width: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(28.0)),
                row_gap: Val::Px(12.0),
                overflow: Overflow::clip(),
                ..default()
            }))
            .with_children(|card| {
                card.spawn(label_hud(asset_server, focus.class_label(), 18.0, TEXT_DIM));
                card.spawn(label_hud(asset_server, focus.label(), 64.0, TEXT));
                card.spawn(Node {
                    width: Val::Percent(100.0),
                    max_width: Val::Px(620.0),
                    justify_content: JustifyContent::Center,
                    padding: UiRect::vertical(Val::Px(6.0)),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_child((
                    ImageNode::new(asset_server.load(crate::hud::primary_icon_path(focus))),
                    Node {
                        width: Val::Px(440.0),
                        height: Val::Px(220.0),
                        ..default()
                    },
                ));
                for (name, value) in weapon_stats(focus) {
                    stat_bar(card, asset_server, name, value);
                }
                card.spawn(Node {
                    flex_grow: 1.0,
                    min_height: Val::Px(12.0),
                    ..default()
                });

                let equipped = focus == settings.primary;
                let (status, color) = if ctx.mode == Some(shared::GameMode::Freestyle) {
                    ("FREESTYLE IS SNIPERS ONLY", TEXT_DIM)
                } else if ctx.locked() && !equipped {
                    ("LOADOUT LOCKED UNTIL THE GAME ENDS", TEXT_DIM)
                } else if equipped && ctx.carrying.is_some_and(|c| c != focus) {
                    ("EQUIPPED  -  ON NEXT SPAWN", ACCENT)
                } else if equipped {
                    ("EQUIPPED", ACCENT)
                } else {
                    ("SELECT TO EQUIP", TEXT_DIM)
                };
                card.spawn(label_hud(asset_server, status, 20.0, color));
                card.spawn(Node {
                    column_gap: Val::Px(12.0),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|actions| {
                    if !equipped && !ctx.locked() {
                        spawn_button_hud(
                            actions,
                            asset_server,
                            "EQUIP",
                            22.0,
                            Btn::SelectPrimary(focus),
                            ACCENT,
                            ROW_HOVER,
                            PANEL_SOLID,
                            UiSound::BUTTON,
                        );
                    }
                    spawn_button_hud(
                        actions,
                        asset_server,
                        "GUNSMITH",
                        22.0,
                        Btn::OpenGunsmith(focus),
                        ROW,
                        ROW_HOVER,
                        TEXT,
                        UiSound::BUTTON,
                    );
                });
            });
        });

        page.spawn(divider());
        page.spawn(Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            column_gap: Val::Px(28.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|f| {
            plain_button(f, asset_server, "BACK", Btn::CloseLoadout, UiSound::BUTTON_BACK);
            f.spawn(label_hud(asset_server, "CHANGES SAVE AUTOMATICALLY", 18.0, TEXT_DIM));
        });
    });
}

/// PAST UPDATES — see `Screen::Changelog`. One panel of every older
/// version's notes, styled like the main menu's "WHAT'S NEW" panel and
/// scrolled with the wheel ([`scroll_hovered`]: the text is
/// `Pickable::IGNORE` so the hover lands on the list itself).
fn build_changelog(commands: &mut Commands, asset_server: &AssetServer, solid: bool) {
    let history = crate::changelog::history();
    commands.spawn(page_root(solid)).with_children(|page| {
        if solid {
            menu_background(page, asset_server);
        }
        page_title(page, asset_server, "WHAT'S NEW", "PAST UPDATES");
        page.spawn(divider());

        page.spawn(Node {
            width: Val::Percent(100.0),
            flex_grow: 1.0,
            flex_basis: Val::Px(0.0),
            min_height: Val::Px(0.0),
            padding: UiRect::vertical(Val::Px(8.0)),
            ..default()
        })
        .with_children(|body| {
            body.spawn(panel_node(Node {
                width: Val::Percent(100.0),
                max_width: Val::Px(900.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(22.0)),
                min_height: Val::Px(0.0),
                ..default()
            }))
            .with_children(|panel| {
                panel
                    .spawn((
                        ScrollPosition::default(),
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            min_height: Val::Px(0.0),
                            row_gap: Val::Px(6.0),
                            padding: UiRect::right(Val::Px(6.0)),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ))
                    .with_children(|list| {
                        if history.is_empty() {
                            list.spawn((
                                label_hud(asset_server, "NO PAST UPDATES YET", 20.0, TEXT_DIM),
                                Pickable::IGNORE,
                            ));
                        }
                        for (i, (version, notes)) in history.iter().enumerate() {
                            list.spawn((
                                label_hud(asset_server, format!("v{version}"), 18.0, ACCENT),
                                Node {
                                    margin: UiRect::top(Val::Px(if i == 0 { 0.0 } else { 10.0 })),
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ));
                            for note in *notes {
                                list.spawn((
                                    label_body(asset_server, format!("\u{2022}  {note}"), 14.0, TEXT),
                                    Pickable::IGNORE,
                                ));
                            }
                        }
                    });
            });
        });

        page.spawn(divider());
        page.spawn(Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|f| {
            plain_button(f, asset_server, "BACK", Btn::CloseChangelog, UiSound::BUTTON_BACK);
        });
    });
}

/// A weapon's GUNSMITH page: the weapon on the left, its options on the
/// right — the sniper's reticle and scope zoom; the AK-74 has none yet.
fn build_gunsmith(commands: &mut Commands, settings: &Settings, asset_server: &AssetServer, weapon: WeaponId, solid: bool) {
    commands.spawn(page_root(solid)).with_children(|page| {
        if solid {
            menu_background(page, asset_server);
        }
        page_title(page, asset_server, &format!("LOADOUT  /  {}", weapon.label()), "GUNSMITH");
        page.spawn(divider());

        page.spawn(Node {
            width: Val::Percent(100.0),
            flex_grow: 1.0,
            flex_basis: Val::Px(0.0),
            min_height: Val::Px(0.0),
            column_gap: Val::Px(32.0),
            padding: UiRect::vertical(Val::Px(8.0)),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|body| {
            body.spawn(panel_node(Node {
                width: Val::Px(520.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(24.0)),
                row_gap: Val::Px(6.0),
                align_self: AlignSelf::FlexStart,
                ..default()
            }))
            .with_children(|card| {
                card.spawn((
                    ImageNode::new(asset_server.load(crate::hud::primary_icon_path(weapon))),
                    Node {
                        width: Val::Px(440.0),
                        height: Val::Px(220.0),
                        ..default()
                    },
                ));
                card.spawn(label_hud(asset_server, weapon.label(), 40.0, TEXT));
                card.spawn(label_hud(asset_server, weapon.class_label(), 16.0, TEXT_DIM));
            });

            body.spawn(Node {
                flex_grow: 1.0,
                flex_basis: Val::Px(0.0),
                min_width: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(16.0),
                ..default()
            })
            .with_children(|content| {
                if weapon != WeaponId::Sniper {
                    return;
                }
                section_heading(content, asset_server, "RETICLE");
                content
                    .spawn(Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(14.0),
                        row_gap: Val::Px(14.0),
                        ..default()
                    })
                    .with_children(|row| {
                        for id in CrosshairId::ALL {
                            let selected = settings.crosshair == id;
                            loadout_tile(row, asset_server, Btn::SetCrosshair(id), selected, |tile| {
                                tile.spawn((
                                    ImageNode::new(asset_server.load(crosshair_asset_path(id))),
                                    Node {
                                        width: Val::Px(150.0),
                                        height: Val::Px(150.0),
                                        ..default()
                                    },
                                ));
                                tile.spawn(label_hud(
                                    asset_server,
                                    id.label().to_uppercase(),
                                    20.0,
                                    if selected { TEXT } else { TEXT_DIM },
                                ));
                            });
                        }
                    });

                content.spawn(Node {
                    height: Val::Px(8.0),
                    ..default()
                });
                section_heading(content, asset_server, "SCOPE ZOOM");
                content
                    .spawn(Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(14.0),
                        row_gap: Val::Px(14.0),
                        ..default()
                    })
                    .with_children(|row| {
                        for zoom in ScopeZoom::ALL {
                            let selected = settings.scope_zoom == zoom;
                            loadout_tile(row, asset_server, Btn::SetScopeZoom(zoom), selected, |tile| {
                                tile.spawn(label_hud(
                                    asset_server,
                                    zoom.label().to_uppercase(),
                                    40.0,
                                    if selected { TEXT } else { TEXT_DIM },
                                ));
                            });
                        }
                    });
            });
        });

        page.spawn(divider());
        page.spawn(Node {
            width: Val::Percent(100.0),
            align_items: AlignItems::Center,
            column_gap: Val::Px(28.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|f| {
            plain_button(f, asset_server, "BACK", Btn::CloseGunsmith, UiSound::BUTTON_BACK);
            f.spawn(label_hud(asset_server, "CHANGES SAVE AUTOMATICALLY", 18.0, TEXT_DIM));
        });
    });
}

fn build_profile(content: &mut ChildSpawnerCommands, asset_server: &AssetServer) {
    setting_row(content, asset_server, "USERNAME", |row| {
        row.spawn(field_box(360.0)).with_children(|f| {
            f.spawn((label_hud(asset_server, "", 24.0, TEXT), DynText::Username));
        });
        option_button(row, asset_server, "APPLY", Btn::ConfirmUsername, true, UiSound::BUTTON);
    });
    desc(content, asset_server, "Start typing to edit. Enter or Apply to save.");
}

fn build_controls(content: &mut ChildSpawnerCommands, asset_server: &AssetServer, settings: &Settings) {
    spawn_slider_row(
        content,
        asset_server,
        "MOUSE SENSITIVITY",
        SliderField::Sensitivity,
        settings,
        0.05,
    );
    spawn_slider_row(
        content,
        asset_server,
        "ADS SENS MULTIPLIER",
        SliderField::AdsSensitivity,
        settings,
        0.05,
    );
    spawn_slider_row(
        content,
        asset_server,
        "MONITOR DISTANCE COEFFICIENT",
        SliderField::AdsCoefficient,
        settings,
        0.01,
    );
    desc(
        content,
        asset_server,
        "Aiming slows your look by however much the weapon zooms, so a high-zoom scope \
         turns slowly and iron sights barely change. The multiplier scales that further \
         (1.00 = matched to the zoom). The coefficient picks which point on screen feels \
         the same at the hip and aimed: 0 the centre, 1.33 the Call of Duty default.",
    );
    spawn_slider_row(content, asset_server, "FIELD OF VIEW", SliderField::Fov, settings, 1.0);

    setting_row(content, asset_server, "AUTO RELOAD", |row| {
        toggle_button(row, asset_server, settings.auto_reload, Btn::ToggleAutoReload, None);
    });
    desc(
        content,
        asset_server,
        "Automatically start reloading after the shot that empties the magazine, \
         instead of waiting for you to press reload.",
    );

    setting_row(content, asset_server, "AUTOMATIC MANTLE", |row| {
        for mode in AutoMantle::ALL {
            option_button(
                row,
                asset_server,
                &mode.label().to_uppercase(),
                Btn::SetAutoMantle(mode),
                settings.auto_mantle == mode,
                UiSound::BUTTON,
            );
        }
    });
    desc(
        content,
        asset_server,
        "Automatically climb a ledge you'd otherwise bonk into and fall from. Off never \
         catches you; Semi-Auto only while jumping toward one; Full-Auto any time you're \
         airborne and moving toward one, jump or not.",
    );

    setting_row(content, asset_server, "DEBUG MODE", |row| {
        // (Its ON / OFF stays live without a rebuild — `DynText::Debug`.)
        toggle_button(
            row,
            asset_server,
            settings.debug_mode,
            Btn::ToggleDebug,
            Some(DynText::Debug),
        );
    });
    desc(
        content,
        asset_server,
        "Debug mode shows the muzzle-flash / smoke / gravity tuning panels (top-right).",
    );
}

fn build_audio(content: &mut ChildSpawnerCommands, asset_server: &AssetServer, settings: &Settings) {
    spawn_slider_row(
        content,
        asset_server,
        "MASTER VOLUME",
        SliderField::MasterVolume,
        settings,
        0.05,
    );
    desc(
        content,
        asset_server,
        "Controls the volume of every game sound — gunshots, footsteps, kills, the \
         ambience, all of it.",
    );
    spawn_slider_row(
        content,
        asset_server,
        "MUSIC VOLUME",
        SliderField::MusicVolume,
        settings,
        0.05,
    );
    desc(content, asset_server, "How loud the main menu music is.");
}

fn build_graphics(content: &mut ChildSpawnerCommands, asset_server: &AssetServer, settings: &Settings) {
    section_heading(content, asset_server, "DISPLAY");
    setting_row(content, asset_server, "VSYNC", |row| {
        toggle_button(row, asset_server, settings.vsync, Btn::ToggleVsync, None);
    });
    desc(
        content,
        asset_server,
        "Off by default: this is a fast-aim shooter, and vsync's queued frames add \
         input-to-screen latency and frame-pacing judder on top of capping the frame rate \
         to your monitor's refresh rate. Turning it on removes screen tearing at the cost \
         of that extra latency.",
    );
    if !settings.vsync {
        spawn_slider_row(
            content,
            asset_server,
            "FRAME RATE LIMIT",
            SliderField::FrameLimit,
            settings,
            10.0,
        );
    }

    setting_row(content, asset_server, "SHADOW MAP", |row| {
        for quality in ShadowQuality::ALL {
            option_button(
                row,
                asset_server,
                &quality.label().to_uppercase(),
                Btn::SetShadowQuality(quality),
                settings.shadow_quality == quality,
                UiSound::BUTTON,
            );
        }
    });
    desc(
        content,
        asset_server,
        "Adjusts the resolution and draw distance of shadows cast by the sun. Higher \
         settings look more accurate at longer range but cost more performance. Disabled \
         removes shadows entirely.",
    );
}

fn build_multiplayer(content: &mut ChildSpawnerCommands, asset_server: &AssetServer, settings: &Settings) {
    section_heading(content, asset_server, "DEV CONVENIENCE");
    desc(
        content,
        asset_server,
        "Skip clicking when launching two clients locally. On reaching the main \
         menu: join an open lobby if one exists, otherwise create one.",
    );
    setting_row(content, asset_server, "AUTO-CREATE LOBBY", |row| {
        toggle_button(
            row,
            asset_server,
            settings.dev_auto_create_lobby,
            Btn::ToggleAutoCreate,
            None,
        );
    });
    setting_row(content, asset_server, "AUTO-JOIN LOBBY", |row| {
        toggle_button(
            row,
            asset_server,
            settings.dev_auto_join_lobby,
            Btn::ToggleAutoJoin,
            None,
        );
    });
}

fn spawn_slider_row(
    content: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    name: &str,
    field: SliderField,
    settings: &Settings,
    step: f32,
) {
    setting_row(content, asset_server, name, |row| {
        plain_button(row, asset_server, "\u{2212}", Btn::Step(field, -step), UiSound::BUTTON);
        // track (a wide, thin bar; the whole node is the drag target)
        row.spawn((
            Button,
            Interaction::default(),
            SliderTrack(field),
            RelativeCursorPosition::default(),
            Node {
                width: Val::Px(380.0),
                height: Val::Px(24.0),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|hit| {
            hit.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(TRACK),
                // Clicks land on the track (the drag target), not this bar.
                bevy::picking::Pickable::IGNORE,
            ))
            .with_children(|track| {
                track.spawn((
                    SliderFill(field),
                    Node {
                        width: Val::Percent(field_fraction(settings, field) * 100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                    bevy::picking::Pickable::IGNORE,
                ));
            });
        });
        plain_button(row, asset_server, "+", Btn::Step(field, step), UiSound::BUTTON);
        row.spawn((
            label_hud(asset_server, field_value_text(settings, field), 24.0, TEXT),
            DynText::SliderValue(field),
            Node {
                width: Val::Px(70.0),
                ..default()
            },
        ));
    });
}

fn build_keybinds(
    content: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    menu: &Menu,
    binds: &KeyBindings,
) {
    desc(
        content,
        asset_server,
        "Click a binding, then press a key or mouse button. Esc cancels.",
    );
    content
        .spawn((
            Node {
                width: Val::Percent(100.0),
                max_width: Val::Px(1100.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                flex_grow: 1.0,
                min_height: Val::Px(0.0),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ScrollPosition::default(),
        ))
        .with_children(|list| {
            for (i, (name, _)) in SLOTS.iter().enumerate() {
                list.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(if i % 2 == 0 { PANEL } else { Color::NONE }),
                ))
                .with_children(|row| {
                    row.spawn(label_hud(asset_server, name.to_uppercase(), 20.0, TEXT));
                    let capturing = menu.rebinding == Some(i);
                    spawn_button_hud(
                        row,
                        asset_server,
                        &if capturing {
                            "PRESS ANY KEY".to_string()
                        } else {
                            binds.slot(i).label().to_uppercase()
                        },
                        18.0,
                        Btn::Rebind(i),
                        if capturing { ACCENT } else { ROW },
                        ROW_HOVER,
                        if capturing { PANEL_SOLID } else { TEXT },
                        UiSound::BUTTON,
                    );
                });
            }
        });
    content.spawn(Node {
        margin: UiRect::top(Val::Px(10.0)),
        ..default()
    })
    .with_children(|row| {
        plain_button(row, asset_server, "RESET TO DEFAULTS", Btn::ResetKeybinds, UiSound::BUTTON);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn active(world: &mut World) -> bool {
        world.run_system_once(game_active).unwrap()
    }

    #[test]
    fn gameplay_is_off_while_dead_frozen_or_in_a_menu_and_only_then() {
        let mut world = World::new();
        world.init_resource::<Menu>();
        world.init_resource::<crate::revive::Spectate>();
        world.init_resource::<crate::match_end::MatchEndFreeze>();
        world.init_resource::<crate::death_effect::DeathEffect>();
        world.init_resource::<crate::pause::GamePaused>();
        assert!(active(&mut world), "alive, no menu");

        world.resource_mut::<crate::pause::GamePaused>().0 = true;
        assert!(!active(&mut world), "paused by the leader");
        world.resource_mut::<crate::pause::GamePaused>().0 = false;
        assert!(active(&mut world));

        world
            .resource_mut::<crate::death_effect::DeathEffect>()
            .set_active_for_test(true);
        assert!(!active(&mut world), "killed, waiting for the kill cam");
        world
            .resource_mut::<crate::death_effect::DeathEffect>()
            .set_active_for_test(false);
        assert!(active(&mut world));

        world.resource_mut::<Menu>().screen = Screen::Settings;
        assert!(!active(&mut world), "a menu is up");
        world.resource_mut::<Menu>().screen = Screen::None;

        world
            .resource_mut::<crate::match_end::MatchEndFreeze>()
            .start(true);
        assert!(!active(&mut world), "match-end freeze");
    }
}

