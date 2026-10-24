//! The update screen: an opaque cover over everything, the very first thing
//! the window shows. The game starts in [`AppState::Updating`] behind it
//! while `updater` checks the latest release on a background thread —
//! "CHECKING FOR UPDATES" — and only moves on to the main menu once that
//! says the game is up to date. If it isn't, the update downloads with a
//! progress bar, then **UPDATE REQUIRES RESTART** with a RESTART button
//! (Call of Duty's). A check that fails blocks too, with RETRY / QUIT: a
//! shipped build never falls through to an old version.
//!
//! It comes back later if the server turns us away for being on another
//! version (`shared::VersionMismatch`): **UPDATE REQUIRED** with RESTART
//! (which checks again on the way up) if the server's newer, or **SERVER
//! UPDATING** with RETRY if it's behind.
//!
//! Its text is set in a font built into the executable, so it shows even on
//! a fresh install with no `assets/` yet. Under `cargo run` there's no check
//! (`updater::enabled`) and it never shows — the debug panel's "Update
//! screens" section previews every screen instead ([`UpdateScreenDebug`]),
//! with a CLOSE PREVIEW button (Esc / the mouse back button too). Nothing
//! here carries between games: it's only ever up before the menu, or
//! instead of it.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use lightyear::prelude::*;

use crate::net::{GameClient, ServerLink};
use crate::ui::{Hoverable, ACCENT, ROW, ROW_HOVER, TEXT, TEXT_DIM};
use crate::updater::{self, Progress, SharedProgress};
use crate::AppState;

/// Above every other UI layer, the launch loading screen included (which
/// waits on fonts a fresh install doesn't have yet).
const Z: i32 = i32::MAX;
const COVER: Color = Color::srgb(0.02, 0.02, 0.025);
/// The update screen's own copy of `HUD_FONT`, built in.
const FONT: &[u8] = include_bytes!("../assets/fonts/BebasNeue-Regular.ttf");
/// Width (px) of the column the screen's laid out in.
const WIDTH: f32 = 640.0;
/// How big the preview pretends a download is (MB), and how long it takes.
const PREVIEW_MB: f32 = 1240.0;
const PREVIEW_DOWNLOAD_SECS: f32 = 8.0;

pub(crate) struct UpdateScreenPlugin;

impl Plugin for UpdateScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, setup).add_systems(
            Update,
            (
                receive_mismatch,
                poll_check,
                leave_updating.run_if(in_state(AppState::Updating)),
                update_back.before(crate::menu::menu_toggle),
                sync_screen,
                update_screen_text,
                screen_clicks,
            )
                .chain(),
        );
    }
}

/// What the screen's showing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum View {
    Checking,
    /// Downloading `what` for version `to`: `done` of `total` bytes.
    Downloading {
        what: &'static str,
        to: String,
        done: u64,
        total: u64,
    },
    /// Version `version`'s in place — restart into it.
    RestartRequired { version: String },
    /// The check couldn't finish, and why.
    Failed(String),
    /// The server's on `server`, newer than us: restart to update.
    ClientOutdated { server: String },
    /// The server's on `server`, behind us: it's being updated.
    ServerOutdated { server: String },
}

impl View {
    fn title(&self) -> &'static str {
        match self {
            View::Checking => "CHECKING FOR UPDATES",
            View::Downloading { .. } => "DOWNLOADING UPDATE",
            View::RestartRequired { .. } => "UPDATE REQUIRES RESTART",
            View::Failed(_) => "UPDATE CHECK FAILED",
            View::ClientOutdated { .. } => "UPDATE REQUIRED",
            View::ServerOutdated { .. } => "SERVER UPDATING",
        }
    }

    fn message(&self) -> String {
        let me = updater::current_version();
        match self {
            View::Checking => "Making sure you have the latest version".into(),
            View::Downloading { to, .. } => format!("Version {me} → {to}"),
            View::RestartRequired { version } => format!("Version {version} is ready. Restart the game to play."),
            View::Failed(why) => format!("{why} The game needs to check for updates before you can play."),
            View::ClientOutdated { server } => {
                format!("A new version ({server}) is out. Restart the game to update.")
            }
            View::ServerOutdated { server } => {
                format!("The server is still on version {server}. Try again in a few minutes.")
            }
        }
    }

    fn buttons(&self) -> &'static [Btn] {
        match self {
            View::Checking | View::Downloading { .. } => &[],
            View::RestartRequired { .. } => &[Btn::Restart { check_again: false }],
            View::Failed(_) | View::ServerOutdated { .. } => &[Btn::Retry, Btn::Quit],
            View::ClientOutdated { .. } => &[Btn::Restart { check_again: true }, Btn::Quit],
        }
    }

    /// Same screen, whatever its numbers — whether it needs building afresh.
    fn same_kind(&self, other: &View) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum Btn {
    /// Start a fresh copy and quit this one (`check_again`: the fresh one
    /// checks for updates on launch).
    Restart { check_again: bool },
    Retry,
    Quit,
    /// Close the debug panel's preview.
    ClosePreview,
}

impl Btn {
    fn label(self) -> &'static str {
        match self {
            Btn::Restart { .. } => "RESTART",
            Btn::Retry => "RETRY",
            Btn::Quit => "QUIT",
            Btn::ClosePreview => "CLOSE PREVIEW",
        }
    }
}

#[derive(Resource)]
pub(crate) struct UpdateScreen {
    font: Handle<Font>,
    /// The launch check, while there is one (`None` once it's said we're up
    /// to date, or under `cargo run`).
    check: Option<SharedProgress>,
    /// What the check last said.
    progress: Progress,
    /// The server's version, once it's turned us away for not matching.
    mismatch: Option<String>,
    /// The debug panel's preview, and when it started (for its fake
    /// download).
    preview: Option<(View, f32)>,
}

impl UpdateScreen {
    /// What to show right now, if anything.
    fn view(&self, now: f32) -> Option<View> {
        if let Some((view, since)) = &self.preview {
            return Some(match view {
                View::Downloading { what, to, .. } => {
                    let total = (PREVIEW_MB * 1024.0 * 1024.0) as u64;
                    let k = ((now - since) / PREVIEW_DOWNLOAD_SECS).fract();
                    View::Downloading {
                        what,
                        to: to.clone(),
                        done: (total as f32 * k) as u64,
                        total,
                    }
                }
                v => v.clone(),
            });
        }
        if self.check.is_some() {
            return match &self.progress {
                Progress::Checking => Some(View::Checking),
                Progress::Downloading { what, to, done, total } => Some(View::Downloading {
                    what,
                    to: to.clone(),
                    done: *done,
                    total: *total,
                }),
                Progress::Ready { version } => Some(View::RestartRequired { version: version.clone() }),
                Progress::Failed(why) => Some(View::Failed(why.clone())),
                Progress::UpToDate => None,
            };
        }
        self.mismatch.as_ref().map(|server| {
            if updater::newer_than(server, updater::current_version()) {
                View::ClientOutdated { server: server.clone() }
            } else {
                View::ServerOutdated { server: server.clone() }
            }
        })
    }

    fn previewing(&self) -> bool {
        self.preview.is_some()
    }
}

/// Build the font and start the check (if this build checks at all).
fn setup(mut fonts: ResMut<Assets<Font>>, mut commands: Commands) {
    let font = fonts.add(Font::try_from_bytes(FONT.to_vec()).expect("the built-in update screen font"));
    let check = updater::enabled().then(updater::start);
    commands.insert_resource(UpdateScreen {
        font,
        check,
        progress: Progress::Checking,
        mismatch: None,
        preview: None,
    });
}

/// Pick up what the check's got to; once it says we're up to date it's done.
fn poll_check(mut screen: ResMut<UpdateScreen>) {
    let Some(latest) = screen.check.as_ref().and_then(|c| c.lock().ok().map(|p| p.clone())) else {
        return;
    };
    if latest == Progress::UpToDate {
        screen.check = None;
    }
    if screen.progress != latest {
        screen.progress = latest;
    }
}

/// Up to date (or not checking): on to the main menu.
fn leave_updating(screen: Res<UpdateScreen>, mut next: ResMut<NextState<AppState>>) {
    if screen.check.is_none() {
        next.set(AppState::MainMenu);
    }
}

/// The server turned us away for being on another version: stop trying to
/// connect and say why.
fn receive_mismatch(
    mut receivers: Query<(Entity, &mut MessageReceiver<shared::VersionMismatch>), With<GameClient>>,
    mut screen: ResMut<UpdateScreen>,
    mut link: ResMut<ServerLink>,
    mut commands: Commands,
) {
    for (client, mut rx) in &mut receivers {
        for msg in rx.receive() {
            warn!(
                "server is on v{}, we're on v{} — it won't let us play",
                msg.server_version,
                updater::current_version()
            );
            screen.mismatch = Some(msg.server_version);
            link.hold = true;
            commands.trigger_targets(Disconnect, client);
        }
    }
}

/// Esc / the mouse back button: closes the preview — the same as its CLOSE
/// PREVIEW button. On a real update screen there's no going back, so they're
/// just kept from opening the settings menu behind it.
fn update_back(
    time: Res<Time>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    menu: Res<crate::menu::Menu>,
    mut screen: ResMut<UpdateScreen>,
) {
    if menu.rebinding.is_some() || screen.view(time.elapsed_secs()).is_none() {
        return;
    }
    if mouse.just_pressed(MouseButton::Back) || keys.just_pressed(KeyCode::Escape) {
        mouse.clear_just_pressed(MouseButton::Back);
        keys.clear_just_pressed(KeyCode::Escape);
        screen.preview = None;
    }
}

#[derive(Component)]
struct UpdateCover;

#[derive(Component)]
struct UpdateTitle;

#[derive(Component)]
struct UpdateMessage;

#[derive(Component)]
struct UpdateDetail;

#[derive(Component)]
struct UpdateBar;

#[derive(Component)]
struct UpdateBarFill;

/// Keep the cover up while there's something to show, rebuilt whenever the
/// screen changes to another kind.
fn sync_screen(
    time: Res<Time>,
    screen: Res<UpdateScreen>,
    covers: Query<Entity, With<UpdateCover>>,
    mut shown: Local<Option<(View, bool)>>,
    mut commands: Commands,
) {
    let view = screen.view(time.elapsed_secs());
    let previewing = screen.previewing();
    let same = match (&*shown, &view) {
        (Some((a, p)), Some(b)) => a.same_kind(b) && *p == previewing && !covers.is_empty(),
        (None, None) => true,
        _ => false,
    };
    if same {
        return;
    }
    for e in &covers {
        commands.entity(e).despawn();
    }
    *shown = view.clone().map(|v| (v, previewing));
    if let Some(view) = view {
        spawn_cover(&mut commands, &screen.font, &view, previewing);
    }
}

fn spawn_cover(commands: &mut Commands, font: &Handle<Font>, view: &View, previewing: bool) {
    let text = |s: String, size: f32, color: Color| {
        (
            Text::new(s),
            TextFont {
                font: font.clone(),
                font_size: size,
                ..default()
            },
            TextColor(color),
            TextLayout::new_with_justify(JustifyText::Center),
        )
    };
    commands
        .spawn((
            UpdateCover,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(COVER),
            GlobalZIndex(Z),
            // Swallows clicks meant for whatever's underneath.
            Interaction::default(),
            bevy::ui::FocusPolicy::Block,
        ))
        .with_children(|c| {
            c.spawn(Node {
                width: Val::Px(WIDTH),
                max_width: Val::Percent(90.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(14.0),
                ..default()
            })
            .with_children(|col| {
                col.spawn((UpdateTitle, text(view.title().into(), 52.0, TEXT)));
                col.spawn((UpdateMessage, text(view.message(), 22.0, TEXT_DIM)));
                if matches!(view, View::Downloading { .. }) {
                    col.spawn((
                        UpdateBar,
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(10.0),
                            margin: UiRect::top(Val::Px(12.0)),
                            ..default()
                        },
                        BackgroundColor(ROW),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            UpdateBarFill,
                            Node {
                                width: Val::Percent(0.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(ACCENT),
                        ));
                    });
                }
                col.spawn((UpdateDetail, text(String::new(), 18.0, TEXT_DIM)));
                let mut buttons: Vec<Btn> = view.buttons().to_vec();
                if previewing {
                    buttons.push(Btn::ClosePreview);
                }
                if !buttons.is_empty() {
                    col.spawn(Node {
                        column_gap: Val::Px(12.0),
                        margin: UiRect::top(Val::Px(18.0)),
                        ..default()
                    })
                    .with_children(|row| {
                        for btn in buttons {
                            row.spawn((
                                Button,
                                Interaction::default(),
                                btn,
                                Hoverable::new(ROW, ROW_HOVER, TEXT),
                                Node {
                                    min_width: Val::Px(150.0),
                                    padding: UiRect::axes(Val::Px(22.0), Val::Px(10.0)),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                BackgroundColor(ROW),
                            ))
                            .with_children(|b| {
                                b.spawn(text(btn.label().into(), 26.0, TEXT));
                            });
                        }
                    });
                }
            });
            // The version this is, bottom centre.
            c.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(24.0),
                    ..default()
                },
                text(format!("v{}", updater::current_version()), 16.0, TEXT_DIM),
            ));
        });
}

/// The bits that change while a screen's up: the download's progress, the
/// title's pulse and dots while checking.
#[allow(clippy::type_complexity)]
fn update_screen_text(
    time: Res<Time>,
    screen: Res<UpdateScreen>,
    mut title: Query<(&mut Text, &mut TextColor), With<UpdateTitle>>,
    mut message: Query<&mut Text, (With<UpdateMessage>, Without<UpdateTitle>)>,
    mut detail: Query<&mut Text, (With<UpdateDetail>, Without<UpdateTitle>, Without<UpdateMessage>)>,
    mut fill: Query<&mut Node, With<UpdateBarFill>>,
) {
    let now = time.elapsed_secs();
    let Some(view) = screen.view(now) else { return };
    let set = |t: &mut Text, s: String| {
        if t.0 != s {
            t.0 = s;
        }
    };
    let waiting = matches!(view, View::Checking | View::Downloading { .. });
    for (mut t, mut c) in &mut title {
        let dots = if waiting { ".".repeat((now * 2.0) as usize % 4) } else { String::new() };
        set(&mut t, format!("{}{dots}", view.title()));
        // A slow pulse while it works, like the connecting screen's.
        c.0 = if waiting { TEXT.with_alpha(0.65 + 0.3 * (now * 3.0).sin()) } else { TEXT };
    }
    for mut t in &mut message {
        set(&mut t, view.message());
    }
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    let (line, frac) = match &view {
        View::Downloading { what, done, total, .. } if *total > 0 => (
            format!("{what}  ·  {:.1} / {:.1} MB  ·  {:.0}%", mb(*done), mb(*total), *done as f64 / *total as f64 * 100.0),
            *done as f32 / *total as f32,
        ),
        View::Downloading { what, done, .. } => (format!("{what}  ·  {:.1} MB", mb(*done)), 0.0),
        _ => (String::new(), 0.0),
    };
    for mut t in &mut detail {
        set(&mut t, line.clone());
    }
    for mut n in &mut fill {
        let w = Val::Percent((frac * 100.0).clamp(0.0, 100.0));
        if n.width != w {
            n.width = w;
        }
    }
}

fn screen_clicks(
    q: Query<(&Interaction, &Btn), Changed<Interaction>>,
    mut screen: ResMut<UpdateScreen>,
    mut link: ResMut<ServerLink>,
    mut exit: EventWriter<AppExit>,
) {
    for (interaction, &btn) in &q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // In the preview nothing's for real: every button just closes it.
        if screen.previewing() {
            info!("update screen preview: {} pressed", btn.label());
            screen.preview = None;
            continue;
        }
        match btn {
            Btn::Restart { check_again } => match updater::restart(check_again) {
                Ok(()) => {
                    exit.write(AppExit::Success);
                }
                Err(e) => {
                    screen.check = Some(std::sync::Arc::new(std::sync::Mutex::new(Progress::Failed(format!(
                        "Couldn't restart the game ({e}). Close it and open it again."
                    )))));
                }
            },
            Btn::Retry => {
                if screen.mismatch.take().is_some() {
                    // Dial the server again; it'll tell us if it's still
                    // on another version.
                    link.hold = false;
                    link.retry_in = Some(0.0);
                } else {
                    screen.progress = Progress::Checking;
                    screen.check = Some(updater::start());
                }
            }
            Btn::Quit => {
                exit.write(AppExit::Success);
            }
            Btn::ClosePreview => screen.preview = None,
        }
    }
}

/// The debug panel's "Update screens" section: shows each screen as it
/// would look, without touching versions or files.
#[derive(SystemParam)]
pub(crate) struct UpdateScreenDebug<'w> {
    screen: ResMut<'w, UpdateScreen>,
    time: Res<'w, Time>,
}

impl UpdateScreenDebug<'_> {
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Preview the launch update screens. Nothing is downloaded and no button does anything but close it (so do Esc / mouse back).");
        let me = updater::current_version();
        let next = semver::Version::parse(me)
            .map(|v| semver::Version::new(v.major, v.minor, v.patch + 1).to_string())
            .unwrap_or_else(|_| "9.9.9".into());
        let older = semver::Version::parse(me)
            .map(|v| semver::Version::new(v.major, v.minor, v.patch.saturating_sub(1)).to_string())
            .unwrap_or_else(|_| "0.0.1".into());
        let views = [
            ("Checking for updates", View::Checking),
            (
                "Downloading update",
                View::Downloading {
                    what: "GAME FILES",
                    to: next.clone(),
                    done: 0,
                    total: 0,
                },
            ),
            ("Update requires restart", View::RestartRequired { version: next.clone() }),
            (
                "Update check failed",
                View::Failed("Couldn't reach the update server. Check your internet connection.".into()),
            ),
            ("Server newer (update required)", View::ClientOutdated { server: next }),
            ("Server older (server updating)", View::ServerOutdated { server: older }),
        ];
        for (label, view) in views {
            if ui.button(label).clicked() {
                self.screen.preview = Some((view, self.time.elapsed_secs()));
            }
        }
        if self.screen.previewing() && ui.button("Close preview").clicked() {
            self.screen.preview = None;
        }
    }
}
