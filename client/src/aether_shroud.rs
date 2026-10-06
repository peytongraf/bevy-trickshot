//! The Aether Shroud, client side — a `Zombies` field upgrade (the server
//! owns the charges and the timer, `server::field_upgrades`; the rules are
//! `shared::field_upgrade`):
//!
//! * **Using it** — the field upgrade key (X by default) asks the server
//!   ([`shared::UseFieldUpgrade`]) once a charge is stored.
//! * **While it's up** (read off our `LobbyMember::field_upgrade`): the
//!   activate sound and the active loop play as it goes up, the loop is cut
//!   and the deactivate sound plays as it wears off; our guns are reloaded
//!   on the spot as it goes up; we move [`SPEED_MULT`]× as fast
//!   (`player::move_player`); and the screen goes purple
//!   (`vfx::aether_shroud`). The zombies ignoring us and the no-damage are
//!   the server's.
//! * **Other players** with theirs up are drawn a see-through purple
//!   ([`shroud_remote_avatars`]).
//! * **HUD** — its icon at the end of the bottom-right ammo row
//!   (`hud::ammo_text` spawns it, [`spawn_shroud_hud`]): the charges stored
//!   beside it, a yellow ring filling clockwise from the top as the next one
//!   builds (a purple one running down while it's up), dimmed with none
//!   stored, the key under it. Only shown in a `Zombies` game.
//!
//! Everything resets with the game: out of a running `Zombies` game our
//! state goes back to nothing (silently), the loop is `StateScoped(InGame)`
//! and despawned with it, and a remote avatar's purple look goes with the
//! avatar.

use std::collections::HashMap;

use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use bevy_egui::{egui, EguiContexts};
use lightyear::prelude::*;
use shared::field_upgrade::{FieldUpgrade, CHARGE_SECS, DURATION_SECS, SPEED_MULT};
use shared::{Lobby, PlayerId};

use crate::keybinds::KeyBindings;
use crate::net::{GameClient, RemoteAvatar};
use crate::zombies_hud::zombies_game;
use crate::{menu, AetherScreen, AetherScreenSettings, AppState, GameSounds, Weapon};

const ICON_PATH: &str = "textures/icons/field_upgrades/aether_shroud.png";
const ICON_SHADER: &str = "shaders/aether_shroud_icon.wgsl";
/// On-screen size of the icon (at the ammo HUD's reference height).
const ICON_SIZE: f32 = 52.0;
/// Opacity of the HUD group with no charge stored.
const EMPTY_ALPHA: f32 = 0.45;

pub(crate) struct AetherShroudPlugin;

impl Plugin for AetherShroudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalShroud>()
            .init_resource::<ShroudLookSettings>()
            .add_plugins(UiMaterialPlugin::<ShroudIconMaterial>::default())
            .add_systems(
                Update,
                (
                    use_aether_shroud.run_if(
                        in_state(AppState::InGame)
                            .and(menu::game_active)
                            .and(crate::killcam::no_killcam)
                            .and(crate::revive::up),
                    ),
                    sync_aether_shroud,
                    (update_shroud_hud, shroud_remote_avatars).run_if(in_state(AppState::InGame)),
                )
                    .chain(),
            )
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                aether_debug_ui.run_if(menu::debug_enabled.and(in_state(AppState::InGame))),
            );
    }
}

/// Our Aether Shroud, as of our `LobbyMember::field_upgrade` (default out of
/// a running `Zombies` game).
#[derive(Resource, Default)]
pub(crate) struct LocalShroud {
    /// What the server last said.
    state: FieldUpgrade,
    /// Seconds since `state.charge_secs` last changed — the ring runs on
    /// smoothly between the server's once-a-second updates.
    since_charge: f32,
    /// Seconds left of the one up, run down locally.
    active_left: f32,
}

impl LocalShroud {
    pub(crate) fn active(&self) -> bool {
        self.state.active()
    }

    /// Movement speed multiplier (`player::move_player`).
    pub(crate) fn movement(&self) -> f32 {
        if self.active() {
            SPEED_MULT
        } else {
            1.0
        }
    }

    /// How far the next charge has built (0..=1), smoothed.
    fn progress(&self) -> f32 {
        if self.state.full() {
            1.0
        } else {
            ((self.state.charge_secs as f32 + self.since_charge.min(1.0)) / CHARGE_SECS).clamp(0.0, 1.0)
        }
    }
}

/// The looping "it's up" sound — cut as it wears off.
#[derive(Component)]
struct ShroudLoop;

/// The field upgrade key: use a stored charge.
fn use_aether_shroud(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<KeyBindings>,
    shroud: Res<LocalShroud>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut sender: Query<&mut TriggerSender<shared::UseFieldUpgrade>, With<GameClient>>,
) {
    if !binds.field_upgrade.just_pressed(&keys, &mouse)
        || zombies_game(&local, &lobbies).is_none()
        || shroud.state.charges == 0
        || shroud.active()
    {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::UseFieldUpgrade);
        info!("asked to use the Aether Shroud");
    }
}

/// Follow our `LobbyMember::field_upgrade`: as it goes up, the sounds, the
/// reload and the screen effect; as it wears off, the loop cut and the
/// deactivate sound. Out of a running `Zombies` game, back to nothing
/// without a sound.
#[allow(clippy::too_many_arguments)]
fn sync_aether_shroud(
    time: Res<Time>,
    state: Res<State<AppState>>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    sounds: Option<Res<GameSounds>>,
    mut shroud: ResMut<LocalShroud>,
    mut screen: ResMut<AetherScreen>,
    mut weapon: ResMut<Weapon>,
    loops: Query<Entity, With<ShroudLoop>>,
    mut commands: Commands,
) {
    let me = local.iter().next().map(|l| l.0);
    let in_game = *state.get() == AppState::InGame;
    let now = in_game
        .then(|| zombies_game(&local, &lobbies))
        .flatten()
        .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
        .map(|m| m.field_upgrade);
    let dt = time.delta_secs();

    let Some(now) = now else {
        // Out of the game: nothing up, nothing stored.
        if shroud.state != FieldUpgrade::default() || shroud.active_left > 0.0 {
            *shroud = LocalShroud::default();
        }
        for e in &loops {
            commands.entity(e).despawn();
        }
        screen.set_if_neq(AetherScreen(false));
        return;
    };

    let was = shroud.state;
    if now.charge_secs != was.charge_secs || now.charges != was.charges {
        shroud.since_charge = 0.0;
    } else {
        shroud.since_charge += dt;
    }
    shroud.active_left = (shroud.active_left - dt).max(0.0);

    if now.active() && !was.active() {
        shroud.active_left = DURATION_SECS;
        weapon.reload_instantly();
        if let Some(sounds) = &sounds {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.aether_activate.clone()),
                PlaybackSettings::DESPAWN,
            ));
            commands.spawn((
                StateScoped(AppState::InGame),
                ShroudLoop,
                AudioPlayer::new(sounds.aether_active.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
        info!("Aether Shroud up");
    } else if !now.active() && was.active() {
        shroud.active_left = 0.0;
        for e in &loops {
            commands.entity(e).despawn();
        }
        if let Some(sounds) = &sounds {
            commands.spawn((
                StateScoped(AppState::InGame),
                AudioPlayer::new(sounds.aether_deactivate.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
        info!("Aether Shroud wore off");
    }
    shroud.state = now;
    screen.set_if_neq(AetherScreen(now.active()));
}

// --- HUD -----------------------------------------------------------------

/// The HUD icon: the icon in a ring showing the next charge building.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub(crate) struct ShroudIconMaterial {
    /// x: build progress (0..=1), y: opacity, z: 1 while it's up, w: what's
    /// left of it (0..=1). (A `Vec4` for WebGL2's 16-byte uniform alignment.)
    #[uniform(0)]
    params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    icon: Handle<Image>,
}

impl UiMaterial for ShroudIconMaterial {
    fn fragment_shader() -> ShaderRef {
        ICON_SHADER.into()
    }
}

/// The whole field upgrade group (with its divider) — only shown in a
/// `Zombies` game.
#[derive(Component)]
pub(crate) struct ShroudHud;

#[derive(Component)]
pub(crate) struct ShroudIcon;

/// Charges stored, beside the icon.
#[derive(Component)]
pub(crate) struct ShroudCountText;

/// The white key cap under the icon, and its label.
#[derive(Component)]
pub(crate) struct ShroudKeyCap;

#[derive(Component)]
pub(crate) struct ShroudKeyText;

/// The field upgrade's group at the end of the ammo HUD's row (inside it, so
/// `hud::scale_ammo_hud` scales it with the rest): a divider, then the
/// charges stored beside the icon, the key cap under it.
pub(crate) fn spawn_shroud_hud(
    row: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    materials: &mut Assets<ShroudIconMaterial>,
    font: Handle<Font>,
    binds: &KeyBindings,
    shadow: (Color, f32),
) {
    let (shadow_color, shadow_offset) = shadow;
    let box_shadow = || BoxShadow::new(shadow_color, Val::Px(shadow_offset), Val::Px(shadow_offset), Val::ZERO, Val::Px(1.0));
    row.spawn((
        ShroudHud,
        Node {
            display: Display::None,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(16.0),
            ..default()
        },
    ))
    .with_children(|group| {
        // Divider.
        group.spawn((
            Node {
                width: Val::Px(2.0),
                height: Val::Px(44.0),
                ..default()
            },
            BackgroundColor(Color::WHITE),
            box_shadow(),
        ));
        group
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|fu| {
                fu.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(5.0), Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                ))
                .with_child((
                    ShroudCountText,
                    Text::new("0"),
                    TextFont {
                        font: font.clone(),
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ));
                fu.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((
                        ShroudIcon,
                        Node {
                            width: Val::Px(ICON_SIZE),
                            height: Val::Px(ICON_SIZE),
                            ..default()
                        },
                        MaterialNode(materials.add(ShroudIconMaterial {
                            params: Vec4::new(0.0, EMPTY_ALPHA, 0.0, 0.0),
                            icon: asset_server.load(ICON_PATH),
                        })),
                    ));
                    col.spawn((
                        ShroudKeyCap,
                        Node {
                            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                            min_width: Val::Px(24.0),
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(Color::WHITE),
                        BorderRadius::all(Val::Px(3.0)),
                        box_shadow(),
                    ))
                    .with_child((
                        ShroudKeyText,
                        Text::new(binds.field_upgrade.label().to_uppercase()),
                        TextFont {
                            font,
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::BLACK),
                    ));
                });
            });
    });
}

/// Show the group only in a `Zombies` game, and keep the count, the ring,
/// the dimming and the key cap up to date.
#[allow(clippy::type_complexity)]
fn update_shroud_hud(
    shroud: Res<LocalShroud>,
    binds: Res<KeyBindings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut materials: ResMut<Assets<ShroudIconMaterial>>,
    mut group: Query<&mut Node, With<ShroudHud>>,
    icons: Query<&MaterialNode<ShroudIconMaterial>, With<ShroudIcon>>,
    mut texts: ParamSet<(
        Query<(&mut Text, &mut TextColor), With<ShroudCountText>>,
        Query<&mut Text, With<ShroudKeyText>>,
    )>,
    mut caps: Query<&mut BackgroundColor, With<ShroudKeyCap>>,
) {
    let shown = zombies_game(&local, &lobbies).is_some();
    let display = if shown { Display::Flex } else { Display::None };
    for mut node in &mut group {
        if node.display != display {
            node.display = display;
        }
    }
    if !shown {
        return;
    }
    let s = &shroud.state;
    let alpha = if s.charges == 0 && !s.active() { EMPTY_ALPHA } else { 1.0 };
    let params = Vec4::new(
        shroud.progress(),
        alpha,
        if s.active() { 1.0 } else { 0.0 },
        (shroud.active_left / DURATION_SECS).clamp(0.0, 1.0),
    );
    for handle in &icons {
        if materials.get(&handle.0).is_some_and(|m| m.params != params) {
            if let Some(m) = materials.get_mut(&handle.0) {
                m.params = params;
            }
        }
    }
    let count = s.charges.to_string();
    for (mut text, mut color) in &mut texts.p0() {
        if text.0 != count {
            text.0 = count.clone();
        }
        if color.0.alpha() != alpha {
            color.0.set_alpha(alpha);
        }
    }
    if binds.is_changed() {
        let label = binds.field_upgrade.label().to_uppercase();
        for mut text in &mut texts.p1() {
            text.0 = label.clone();
        }
    }
    for mut bg in &mut caps {
        if bg.0.alpha() != alpha {
            bg.0.set_alpha(alpha);
        }
    }
}

// --- other players ---------------------------------------------------------

/// How another player looks with their Aether Shroud up (the debug window).
#[derive(Resource, Clone, PartialEq)]
pub(crate) struct ShroudLookSettings {
    /// sRGB tint over their textures...
    pub(crate) color: [f32; 3],
    /// ...how see-through (0 = invisible, 1 = solid)...
    pub(crate) alpha: f32,
    /// ...and their purple glow.
    pub(crate) glow: f32,
}

impl Default for ShroudLookSettings {
    fn default() -> Self {
        Self {
            color: [0.7, 0.45, 1.0],
            alpha: 0.45,
            glow: 1.5,
        }
    }
}

impl ShroudLookSettings {
    /// These settings as Rust, to paste over [`Default`].
    fn to_rust(&self) -> String {
        format!(
            "ShroudLookSettings {{\n    color: {:?},\n    alpha: {:.3},\n    glow: {:.3},\n}}",
            crate::round3(self.color),
            self.alpha,
            self.glow
        )
    }

    /// `original` with the purple, see-through look.
    fn apply(&self, original: &StandardMaterial) -> StandardMaterial {
        let [r, g, b] = self.color;
        let mut m = original.clone();
        m.base_color = Color::srgba(r, g, b, self.alpha);
        m.emissive = LinearRgba::from(Color::srgb(r, g, b)) * self.glow;
        m.alpha_mode = AlphaMode::Blend;
        m
    }
}

/// A mesh of a shrouded player's avatar: the material it had.
#[derive(Component)]
struct ShroudOriginal(Handle<StandardMaterial>);

/// Swap every mesh of a soldier avatar whose player has the Aether Shroud up
/// to a purple, see-through copy of its material (one copy per original,
/// shared), and back once it wears off.
#[allow(clippy::type_complexity)]
fn shroud_remote_avatars(
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    look: Res<ShroudLookSettings>,
    avatars: Query<(Entity, &RemoteAvatar), With<crate::SoldierVisual>>,
    ids: Query<&PlayerId>,
    children: Query<&Children>,
    mut meshes: Query<(&mut MeshMaterial3d<StandardMaterial>, Option<&ShroudOriginal>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut copies: Local<HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>>,
    mut commands: Commands,
) {
    // A new look: re-make every copy.
    if look.is_changed() {
        for (original, copy) in copies.iter() {
            if let Some(m) = materials.get(*original).map(|o| look.apply(o)) {
                if let Some(c) = materials.get_mut(copy) {
                    *c = m;
                }
            }
        }
    }
    let lobby = zombies_game(&local, &lobbies);
    for (root, avatar) in &avatars {
        let shrouded = lobby.is_some_and(|l| {
            ids.get(avatar.src)
                .is_ok_and(|id| l.members.iter().any(|m| m.peer == id.0 && m.field_upgrade.active()))
        });
        for e in children.iter_descendants(root) {
            let Ok((mut mat, original)) = meshes.get_mut(e) else {
                continue;
            };
            match (shrouded, original) {
                (true, None) => {
                    let id = mat.0.id();
                    let copy = match copies.get(&id) {
                        Some(c) => c.clone(),
                        None => {
                            let Some(m) = materials.get(id).map(|o| look.apply(o)) else {
                                continue;
                            };
                            let c = materials.add(m);
                            copies.insert(id, c.clone());
                            c
                        }
                    };
                    commands.entity(e).insert(ShroudOriginal(mat.0.clone()));
                    mat.0 = copy;
                }
                (false, Some(original)) => {
                    mat.0 = original.0.clone();
                    commands.entity(e).remove::<ShroudOriginal>();
                }
                _ => {}
            }
        }
    }
}

// --- debug -----------------------------------------------------------------

/// The "Aether Shroud" debug window: fill the charges, preview the screen
/// effect and tune it, and tune other players' look.
fn aether_debug_ui(
    mut contexts: EguiContexts,
    mut screen: ResMut<AetherScreenSettings>,
    mut look: ResMut<ShroudLookSettings>,
    shroud: Res<LocalShroud>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    mut fill: Query<&mut TriggerSender<shared::FillFieldUpgrade>, With<GameClient>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let in_zombies = zombies_game(&local, &lobbies).is_some();
    egui::Window::new("Aether Shroud")
        .default_open(false)
        .default_pos([20.0, 460.0])
        .show(ctx, |ui| {
            let s = &shroud.state;
            ui.label(format!(
                "charges {} · next {:.0}% · {}",
                s.charges,
                shroud.progress() * 100.0,
                if s.active() { format!("UP ({:.1}s left)", shroud.active_left) } else { "not up".into() }
            ));
            if ui
                .add_enabled(in_zombies, egui::Button::new("Fill my charges (leader, Zombies)"))
                .clicked()
            {
                if let Ok(mut f) = fill.single_mut() {
                    f.trigger::<shared::LobbyChannel>(shared::FillFieldUpgrade);
                }
            }
            ui.separator();
            let d = &mut *screen;
            ui.checkbox(&mut d.preview, "Preview screen effect");
            ui.add(egui::Slider::new(&mut d.fade_in_secs, 0.0f32..=3.0).text("fade in (s)"));
            ui.add(egui::Slider::new(&mut d.fade_out_secs, 0.0f32..=3.0).text("fade out (s)"));
            ui.label("Colour grade (sRGB: darks / mids / highlights)");
            for (name, c) in [("darks", &mut d.dark), ("mids", &mut d.mid), ("highlights", &mut d.light)] {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.color_edit_button_rgb(c);
                });
            }
            ui.add(egui::Slider::new(&mut d.tint, 0.0f32..=1.0).text("grade amount"));
            ui.add(egui::Slider::new(&mut d.exposure, 0.2f32..=3.0).text("exposure"));
            ui.add(egui::Slider::new(&mut d.gamma, 0.2f32..=2.0).text("gamma"));
            ui.label("Distortion");
            ui.add(egui::Slider::new(&mut d.warp, -0.3f32..=0.5).text("warp toward centre"));
            ui.add(egui::Slider::new(&mut d.chromatic, 0.0f32..=0.05).text("colour fringing"));
            ui.add(egui::Slider::new(&mut d.edge_glow, 0.0f32..=1.0).text("edge haze"));
            ui.add(egui::Slider::new(&mut d.shimmer, 0.0f32..=0.02).text("edge shimmer"));
            ui.add(egui::Slider::new(&mut d.shimmer_speed, 0.0f32..=8.0).text("shimmer speed"));
            ui.label("Kick (as it goes up)");
            ui.add(egui::Slider::new(&mut d.kick_warp, 0.0f32..=1.0).text("extra warp"));
            ui.add(egui::Slider::new(&mut d.kick_flash, 0.0f32..=4.0).text("flash"));
            ui.add(egui::Slider::new(&mut d.kick_secs, 0.0f32..=3.0).text("eases off over (s)"));
            ui.label("Lightning from the edges");
            ui.add(egui::Slider::new(&mut d.bolt_brightness, 0.0f32..=5.0).text("brightness (0 = none)"));
            ui.add(egui::Slider::new(&mut d.bolt_gap.0, 0.02f32..=3.0).text("gap between, min (s)"));
            ui.add(egui::Slider::new(&mut d.bolt_gap.1, 0.02f32..=3.0).text("gap between, max (s)"));
            ui.add(egui::Slider::new(&mut d.bolt_burst, 0..=4).text("burst as it goes up"));
            ui.add(egui::Slider::new(&mut d.bolt_life, 0.02f32..=1.0).text("each lasts (s)"));
            ui.add(egui::Slider::new(&mut d.bolt_length.0, 0.02f32..=1.2).text("length, min"));
            ui.add(egui::Slider::new(&mut d.bolt_length.1, 0.02f32..=1.2).text("length, max"));
            ui.add(egui::Slider::new(&mut d.bolt_jag, 0.0f32..=0.5).text("jaggedness"));
            ui.add(egui::Slider::new(&mut d.bolt_width, 0.0002f32..=0.01).text("core thickness"));
            ui.add(egui::Slider::new(&mut d.bolt_glow, 0.0005f32..=0.05).text("glow size"));
            ui.horizontal(|ui| {
                ui.label("glow colour");
                ui.color_edit_button_rgb(&mut d.bolt_color);
            });
            if ui.button("Reset screen effect").clicked() {
                *d = AetherScreenSettings {
                    preview: d.preview,
                    ..default()
                };
            }
            ui.separator();
            ui.label("Other players with it up");
            let mut l = look.clone();
            ui.horizontal(|ui| {
                ui.label("tint");
                ui.color_edit_button_rgb(&mut l.color);
            });
            ui.add(egui::Slider::new(&mut l.alpha, 0.0f32..=1.0).text("opacity"));
            ui.add(egui::Slider::new(&mut l.glow, 0.0f32..=6.0).text("glow"));
            if ui.button("Reset look").clicked() {
                l = ShroudLookSettings::default();
            }
            if l != *look {
                *look = l;
            }
            ui.separator();
            if ui.button("Print settings to console").clicked() {
                info!(
                    "Aether Shroud settings:\n{}\n{}",
                    screen.to_rust(),
                    look.to_rust()
                );
            }
        });
    Ok(())
}
