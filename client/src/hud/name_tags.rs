//! Call of Duty style name tags: a small diamond over each lobby member's
//! remote avatar, their name above it.
//!
//! Drawn as HUD nodes placed at the head's projected screen position every
//! frame (not 3D quads), so they're the same size on screen at any distance.
//!
//! * `Freestyle` — only other *real* players are tagged (the target bots have
//!   no name), in blue, and through walls: they're not your enemies.
//! * `FreeForAll` — every other member (players and bots), in red, but only
//!   while they're actually in view: the map's colliders mustn't be in the way.
//!
//! * Pings (`Freestyle`) — aim straight at a target bot and press the ping
//!   key ([`ping_bots`]): a nameless diamond in the enemy colour sits over it
//!   for `shared::bots::PING_SECS`, through walls, CoD style — for *every*
//!   lobby member: the server times it and replicates it as `Bot::pinged`.
//!
//! Tags are `StateScoped(InGame)`, and each despawns with its avatar. A ping
//! lives on the bot's own entity, so it goes when the bot does.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::transform::helper::TransformHelper;
use bevy::window::PrimaryWindow;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::{Interpolated, LocalId, TriggerSender};
use shared::bots::{BOT_HEAD_RADIUS, BOT_HEIGHT, BOT_RADIUS, PING_SECS};
use shared::hitbox::{ray_capsule, Capsule};
use shared::{Bot, GameMode, Lobby, PlayerId, PlayerPose};

use crate::keybinds::KeyBindings;
use crate::net::{BotPose, GameClient, RemoteAvatar};
use crate::{killcam, menu, WorldModelCamera, HUD_FONT};

/// Width of the tag's layout box (px, at scale 1) — wide enough for any name;
/// the name and diamond are centred in it.
const TAG_WIDTH: f32 = 240.0;
const NAME_SIZE: f32 = 17.0;
const DIAMOND_PX: f32 = 14.0;
const ROW_GAP: f32 = 3.0;

/// Panel-adjustable tag placement / size ("Name tags" debug-panel section).
#[derive(Resource)]
pub(crate) struct NameTagSettings {
    /// How far above the replicated eye position the diamond's bottom tip
    /// sits (m).
    pub(crate) height: f32,
    /// On-screen size multiplier for the name, the diamond and the gap
    /// between them (still constant at any distance).
    pub(crate) scale: f32,
    /// sRGB tag colour for enemies (`FreeForAll`).
    pub(crate) enemy_color: [f32; 3],
    /// sRGB tag colour for fellow lobby members who aren't enemies
    /// (`Freestyle`).
    pub(crate) friendly_color: [f32; 3],
}

impl Default for NameTagSettings {
    fn default() -> Self {
        Self {
            height: 0.28,
            scale: 1.35,
            enemy_color: [213.0 / 255.0, 32.0 / 255.0, 50.0 / 255.0],
            friendly_color: [0.30, 0.62, 1.0],
        }
    }
}

fn srgb([r, g, b]: [f32; 3]) -> Color {
    Color::srgb(r, g, b)
}

/// On an interpolated `Bot` entity this client has just pinged, until
/// `Time::elapsed_secs` reaches `until` — shows the pinger their own diamond
/// straight away, rather than a round trip later when the server's
/// replicated `Bot::pinged` arrives (which is what everyone else sees).
#[derive(Component)]
pub(crate) struct Pinged {
    until: f32,
}

/// `Freestyle`: on the ping key, ping the living bot under the crosshair —
/// the camera's forward ray must hit its hitbox (where this client *sees* it)
/// before any wall — by sending the server a [`shared::PingBot`]. Pinging an
/// already-pinged bot restarts its timer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ping_bots(
    binds: Res<KeyBindings>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    camera: Single<&GlobalTransform, With<WorldModelCamera>>,
    bots: Query<(Entity, &Bot, &Interpolated)>,
    rapier: ReadRapierContext,
    mut sender: Query<&mut TriggerSender<shared::PingBot>, With<GameClient>>,
    mut commands: Commands,
) {
    if !binds.ping.just_pressed(&keys, &mouse) {
        return;
    }
    let me = local.iter().next().map(|l| l.0);
    let in_freestyle = me
        .and_then(|me| lobbies.iter().find(|l| l.has(me)))
        .is_some_and(|l| l.mode == GameMode::Freestyle);
    if !in_freestyle {
        return;
    }
    let origin = camera.translation();
    let dir = camera.forward().as_vec3();
    let Some((bot, dist, confirmed)) = bots
        .iter()
        .filter(|(_, b, _)| b.alive)
        .filter_map(|(e, b, interp)| {
            let body = Capsule::standing(b.pos, BOT_HEIGHT, BOT_RADIUS);
            let head = Capsule::head(b.pos, BOT_HEIGHT, BOT_HEAD_RADIUS);
            let t = match (ray_capsule(origin, dir, &body), ray_capsule(origin, dir, &head)) {
                (Some(a), Some(b)) => a.min(b),
                (a, b) => a.or(b)?,
            };
            Some((e, t, interp.confirmed_entity))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return;
    };
    // Nothing but the map has colliders: a hit short of the bot is a wall.
    let blocked = rapier.single().ok().is_some_and(|r| {
        r.cast_ray(origin, dir, dist, true, QueryFilter::default()).is_some()
    });
    if blocked {
        return;
    }
    if let Ok(mut s) = sender.single_mut() {
        s.trigger::<shared::LobbyChannel>(shared::PingBot { bot: confirmed });
    }
    commands.entity(bot).try_insert(Pinged {
        until: time.elapsed_secs() + PING_SECS,
    });
}

/// A tag's root node, for the (live, non-kill-cam) [`RemoteAvatar`] `avatar`.
#[derive(Component)]
pub(crate) struct NameTag {
    avatar: Entity,
}

#[derive(Component)]
pub(crate) struct NameTagText;

#[derive(Component)]
pub(crate) struct NameTagDiamond;

/// A small anti-aliased white diamond, tinted per mode by `ImageNode::color`.
fn diamond_image() -> Image {
    const N: u32 = 32;
    let half = N as f32 / 2.0;
    let mut data = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            // |dx| + |dy| ≤ half is the diamond; a 1 px soft edge.
            let d = (x as f32 + 0.5 - half).abs() + (y as f32 + 0.5 - half).abs();
            let a = (half - d).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// One tag per live remote avatar (kill-cam stand-ins excluded); starts
/// hidden until [`update_name_tags`] places it.
pub(crate) fn spawn_name_tags(
    avatars: Query<Entity, (With<RemoteAvatar>, Without<killcam::KillCamPlayerGhost>)>,
    tags: Query<&NameTag>,
    settings: Res<NameTagSettings>,
    mut diamond: Local<Option<Handle<Image>>>,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let have: std::collections::HashSet<Entity> = tags.iter().map(|t| t.avatar).collect();
    for avatar in &avatars {
        if have.contains(&avatar) {
            continue;
        }
        let diamond = diamond.get_or_insert_with(|| images.add(diamond_image())).clone();
        commands
            .spawn((
                StateScoped(crate::AppState::InGame),
                NameTag { avatar },
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(TAG_WIDTH * settings.scale),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(ROW_GAP * settings.scale),
                    ..default()
                },
                Visibility::Hidden,
                // Under the rest of the HUD (crosshair, hit markers, ...).
                GlobalZIndex(-1),
            ))
            .with_children(|tag| {
                tag.spawn((
                    NameTagText,
                    Text::new(""),
                    TextFont {
                        font: asset_server.load(HUD_FONT),
                        font_size: NAME_SIZE * settings.scale,
                        ..default()
                    },
                    TextColor(srgb(settings.enemy_color)),
                    TextShadow {
                        offset: Vec2::splat(1.0),
                        color: Color::srgba(0.0, 0.0, 0.0, 0.6),
                    },
                ));
                tag.spawn((
                    NameTagDiamond,
                    ImageNode::new(diamond).with_color(srgb(settings.enemy_color)),
                    Node {
                        width: Val::Px(DIAMOND_PX * settings.scale),
                        height: Val::Px(DIAMOND_PX * settings.scale),
                        ..default()
                    },
                ));
            });
    }
}

/// Re-size every existing tag when the "Name tags" scale slider moves (new
/// tags are spawned at the current scale).
pub(crate) fn apply_name_tag_scale(
    settings: Res<NameTagSettings>,
    mut roots: Query<&mut Node, (With<NameTag>, Without<NameTagDiamond>)>,
    mut diamonds: Query<&mut Node, (With<NameTagDiamond>, Without<NameTag>)>,
    mut fonts: Query<&mut TextFont, With<NameTagText>>,
) {
    if !settings.is_changed() {
        return;
    }
    let s = settings.scale;
    for mut node in &mut roots {
        node.width = Val::Px(TAG_WIDTH * s);
        node.row_gap = Val::Px(ROW_GAP * s);
    }
    for mut node in &mut diamonds {
        node.width = Val::Px(DIAMOND_PX * s);
        node.height = Val::Px(DIAMOND_PX * s);
    }
    for mut font in &mut fonts {
        font.font_size = NAME_SIZE * s;
    }
}

/// Place, fill in and show / hide every tag. Runs in `PostUpdate` before UI
/// layout, with the camera's transform computed fresh (`TransformHelper`) so
/// the tags don't trail a frame behind when turning.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn update_name_tags(
    // Bundled — a system function tops out at 16 top-level params.
    (menu, active_killcam): (Res<menu::Menu>, Res<killcam::ActiveKillCam>),
    time: Res<Time>,
    settings: Res<NameTagSettings>,
    local: Query<&LocalId, With<GameClient>>,
    lobbies: Query<&Lobby>,
    avatars: Query<&RemoteAvatar>,
    poses: Query<(&PlayerPose, Option<&PlayerId>, Option<&BotPose>)>,
    pinged: Query<(&Bot, Option<&Pinged>)>,
    camera: Single<(Entity, &Camera), With<WorldModelCamera>>,
    transforms: TransformHelper,
    window: Single<&Window, With<PrimaryWindow>>,
    rapier: ReadRapierContext,
    mut tags: Query<(Entity, &NameTag, &mut Node, &mut Visibility, &Children)>,
    mut texts: Query<(&mut Text, &mut TextColor), With<NameTagText>>,
    mut diamonds: Query<&mut ImageNode, With<NameTagDiamond>>,
    mut commands: Commands,
) {
    let (cam_entity, cam) = camera.into_inner();
    let cam_gt = transforms.compute_global_transform(cam_entity).ok();
    let cam_pos = cam_gt.map(|gt| gt.translation());
    let me = local.iter().next().map(|l| l.0);
    let lobby = me.and_then(|me| lobbies.iter().find(|l| l.has(me)));
    let rapier = rapier.single().ok();
    let hud_up = !menu.is_open() && active_killcam.0.is_none();

    for (entity, tag, mut node, mut vis, children) in &mut tags {
        let Ok(avatar) = avatars.get(tag.avatar) else {
            commands.entity(entity).try_despawn();
            continue;
        };

        // Who it is, and whether (and how) this mode tags them. A `BotPose`
        // is a `Freestyle` target bot's stand-in — tagged (no name) only while
        // pinged.
        let now = time.elapsed_secs();
        let shown = (|| {
            if !hud_up {
                return None;
            }
            let (pose, id, bot) = poses.get(avatar.src).ok()?;
            if !pose.alive {
                return None;
            }
            let (name, color, through_walls) = if let Some(bot) = bot {
                let live_ping = pinged.get(bot.bot).is_ok_and(|(b, local)| {
                    b.pinged || local.is_some_and(|p| p.until > now)
                });
                if !live_ping || lobby?.mode != GameMode::Freestyle {
                    return None;
                }
                ("", srgb(settings.enemy_color), true)
            } else {
                let member = lobby?.members.iter().find(|m| Some(m.peer) == id.map(|i| i.0))?;
                match lobby?.mode {
                    GameMode::Freestyle if member.bot.is_none() => {
                        (member.name.as_str(), srgb(settings.friendly_color), true)
                    }
                    GameMode::Freestyle => return None,
                    GameMode::FreeForAll => (member.name.as_str(), srgb(settings.enemy_color), false),
                }
            };
            let (cam_gt, cam_pos) = (cam_gt?, cam_pos?);
            let anchor = pose.translation + Vec3::Y * settings.height;
            // `world_to_viewport` fails for points behind the camera.
            let screen = cam.world_to_viewport(&cam_gt, anchor).ok()?;
            let size = window.size();
            if screen.x < 0.0 || screen.y < 0.0 || screen.x > size.x || screen.y > size.y {
                return None;
            }
            if !through_walls {
                // In view if the head or chest is: nothing but the map has
                // colliders, so any hit short of the target is a wall.
                let rapier = rapier.as_ref()?;
                let clear = |target: Vec3| {
                    let to = target - cam_pos;
                    let dist = to.length();
                    dist < 1e-3
                        || rapier
                            .cast_ray(cam_pos, to / dist, dist - 0.05, true, QueryFilter::default())
                            .is_none()
                };
                let head = pose.translation + Vec3::Y * 0.1;
                let chest = pose.translation - Vec3::Y * 0.45;
                if !clear(head) && !clear(chest) {
                    return None;
                }
            }
            Some((name, color, screen, size))
        })();

        let Some((name, color, screen, size)) = shown else {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        };

        // Bottom-centre of the box (the diamond's lower tip) on the anchor.
        node.left = Val::Px(screen.x - TAG_WIDTH * settings.scale * 0.5);
        node.bottom = Val::Px(size.y - screen.y);
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        for child in children.iter() {
            if let Ok((mut text, mut text_color)) = texts.get_mut(child) {
                if text.0 != name {
                    text.0 = name.to_string();
                }
                if text_color.0 != color {
                    text_color.0 = color;
                }
            } else if let Ok(mut image) = diamonds.get_mut(child) {
                if image.color != color {
                    image.color = color;
                }
            }
        }
    }
}
