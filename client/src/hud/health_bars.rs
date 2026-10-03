//! Call of Duty style enemy health bars (`Zombies`): a small red bar over a
//! zombie's head, on a slightly see-through black back, showing how much of
//! its health is left. Only *our* hits bring it up — the server names the
//! victim on the [`shared::HitMarker`] it sends us alone ([`DamagedByMe`]),
//! so another player's hits never show us a bar. It stays up for
//! [`HealthBarSettings::show_secs`] after our last hit, then fades out over
//! [`HealthBarSettings::fade_secs`]; it's gone the moment the zombie dies,
//! and hidden while a wall's in the way or the HUD's down.
//!
//! Like the name tags, drawn as HUD nodes placed at the head's projected
//! screen position every frame, so it's the same size on screen at any
//! distance.
//!
//! The zombie's full health is the most it's been seen with: it's
//! replicated ([`shared::PlayerHealth`]) from the moment it rises, at full.
//!
//! Reset between games: every bar is `StateScoped(InGame)` and goes with its
//! zombie's avatar, and the last-hit times live on the bars.

use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use bevy::window::PrimaryWindow;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::{Interpolated, PeerId};
use shared::{PlayerHealth, PlayerId, PlayerPose};

use crate::net::RemoteAvatar;
use crate::{killcam, menu, AppState, WorldModelCamera, ZombieVisual};

/// We hurt the zombie (or player) `0` — from our own hit markers.
#[derive(Event)]
pub(crate) struct DamagedByMe(pub(crate) PeerId);

/// Panel-tunable bar look ("Zombie health bars" debug-panel section).
#[derive(Resource, Clone)]
pub(crate) struct HealthBarSettings {
    /// How far above the zombie's eye the bar's bottom sits (m).
    pub(crate) height: f32,
    /// On-screen size (px), the same at any distance.
    pub(crate) width: f32,
    pub(crate) thickness: f32,
    /// Seconds it stays fully up after our last hit...
    pub(crate) show_secs: f32,
    /// ...then fades out over.
    pub(crate) fade_secs: f32,
    /// The back's opacity (black).
    pub(crate) back_alpha: f32,
}

impl Default for HealthBarSettings {
    fn default() -> Self {
        Self {
            height: 0.45,
            width: 64.0,
            thickness: 6.0,
            show_secs: 2.5,
            fade_secs: 0.6,
            back_alpha: 0.55,
        }
    }
}

const FILL_RED: Color = Color::srgb(0.86, 0.1, 0.1);

/// A bar's root (the black back), for the zombie avatar `avatar`.
#[derive(Component)]
struct HealthBar {
    avatar: Entity,
    /// The most health it's been seen with — its full health.
    max: f32,
    /// When (`Time::elapsed_secs`) we last hurt it.
    last_hit: Option<f32>,
}

/// How much lower (m) a hellhound's bar sits than a zombie's.
const DOG_BAR_DROP: f32 = 1.0;

/// A bar's red fill.
#[derive(Component)]
struct HealthBarFill;

pub(crate) struct HealthBarsPlugin;

impl Plugin for HealthBarsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HealthBarSettings>()
            .add_event::<DamagedByMe>()
            .add_systems(
                Update,
                (spawn_health_bars, note_hits)
                    .chain()
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                PostUpdate,
                update_health_bars
                    .before(bevy::ui::UiSystem::Layout)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

/// One (hidden) bar per zombie avatar.
fn spawn_health_bars(
    avatars: Query<
        Entity,
        (
            With<RemoteAvatar>,
            Or<(With<ZombieVisual>, With<crate::dogs::DogVisual>)>,
            Without<killcam::KillCamPlayerGhost>,
        ),
    >,
    bars: Query<&HealthBar>,
    settings: Res<HealthBarSettings>,
    mut commands: Commands,
) {
    let have: std::collections::HashSet<Entity> = bars.iter().map(|b| b.avatar).collect();
    for avatar in &avatars {
        if have.contains(&avatar) {
            continue;
        }
        commands
            .spawn((
                StateScoped(AppState::InGame),
                HealthBar {
                    avatar,
                    max: 0.0,
                    last_hit: None,
                },
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(settings.width),
                    height: Val::Px(settings.thickness),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, settings.back_alpha)),
                Visibility::Hidden,
                // Under the rest of the HUD, like the name tags.
                GlobalZIndex(-1),
            ))
            .with_children(|bar| {
                bar.spawn((
                    HealthBarFill,
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(FILL_RED),
                ));
            });
    }
}

/// The peer behind an avatar's (interpolated) source entity — off it, or
/// off its confirmed entity.
fn peer_of(src: Entity, ids: &Query<&PlayerId>, interp: &Query<&Interpolated>) -> Option<PeerId> {
    ids.get(src)
        .ok()
        .or_else(|| interp.get(src).ok().and_then(|i| ids.get(i.confirmed_entity).ok()))
        .map(|id| id.0)
}

/// Start (or restart) the bar of each zombie we just hurt.
fn note_hits(
    time: Res<Time>,
    mut hits: EventReader<DamagedByMe>,
    avatars: Query<&RemoteAvatar>,
    ids: Query<&PlayerId>,
    interp: Query<&Interpolated>,
    mut bars: Query<&mut HealthBar>,
) {
    for hit in hits.read() {
        for mut bar in &mut bars {
            let hurt = avatars
                .get(bar.avatar)
                .ok()
                .and_then(|a| peer_of(a.src, &ids, &interp))
                .is_some_and(|peer| peer == hit.0);
            if hurt {
                bar.last_hit = Some(time.elapsed_secs());
            }
        }
    }
}

/// Track each zombie's full health, then place, fill, fade and show / hide
/// every bar. Runs in `PostUpdate` before UI layout, with the camera's
/// transform computed fresh, so the bars don't trail a frame behind.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_health_bars(
    (menu, active_killcam): (Res<menu::Menu>, Res<killcam::ActiveKillCam>),
    time: Res<Time>,
    settings: Res<HealthBarSettings>,
    avatars: Query<(&RemoteAvatar, Has<crate::dogs::DogVisual>)>,
    poses: Query<&PlayerPose>,
    (ids, interp, healths): (Query<&PlayerId>, Query<&Interpolated>, Query<(&PlayerHealth, Option<&PlayerId>)>),
    camera: Single<(Entity, &Camera), With<WorldModelCamera>>,
    transforms: TransformHelper,
    window: Single<&Window, With<PrimaryWindow>>,
    rapier: ReadRapierContext,
    mut bars: Query<
        (Entity, &mut HealthBar, &mut Node, &mut BackgroundColor, &mut Visibility, &Children),
        Without<HealthBarFill>,
    >,
    mut fills: Query<(&mut Node, &mut BackgroundColor), (With<HealthBarFill>, Without<HealthBar>)>,
    mut commands: Commands,
) {
    let (cam_entity, cam) = camera.into_inner();
    let cam_gt = transforms.compute_global_transform(cam_entity).ok();
    let rapier = rapier.single().ok();
    let hud_up = !menu.is_open() && active_killcam.0.is_none();
    let now = time.elapsed_secs();

    for (entity, mut bar, mut node, mut back, mut vis, children) in &mut bars {
        let Ok((avatar, dog)) = avatars.get(bar.avatar) else {
            commands.entity(entity).try_despawn();
            continue;
        };
        let Ok(pose) = poses.get(avatar.src) else {
            continue;
        };
        // Health lives on the confirmed entity; the pose on the interpolated
        // one (the avatar's source).
        let health = healths
            .get(avatar.src)
            .ok()
            .or_else(|| interp.get(avatar.src).ok().and_then(|i| healths.get(i.confirmed_entity).ok()))
            .map(|(h, _)| h.0)
            .or_else(|| {
                // Last resort: whichever entity carries its peer's health.
                let peer = peer_of(avatar.src, &ids, &interp)?;
                healths.iter().find(|(_, id)| id.is_some_and(|id| id.0 == peer)).map(|(h, _)| h.0)
            });
        if let Some(h) = health {
            bar.max = bar.max.max(h);
        }

        let shown = (|| {
            if !hud_up || !pose.alive {
                return None;
            }
            let health = health?;
            let since = now - bar.last_hit?;
            let fade = 1.0 - ((since - settings.show_secs) / settings.fade_secs.max(0.01)).clamp(0.0, 1.0);
            if fade <= 0.0 {
                return None;
            }
            let cam_gt = cam_gt?;
            let cam_pos = cam_gt.translation();
            // (A hellhound's back is well under a zombie's head.)
            let dog_drop = if dog { DOG_BAR_DROP } else { 0.0 };
            let anchor = pose.translation + Vec3::Y * (settings.height - dog_drop);
            // `world_to_viewport` fails for points behind the camera.
            let screen = cam.world_to_viewport(&cam_gt, anchor).ok()?;
            let size = window.size();
            if screen.x < 0.0 || screen.y < 0.0 || screen.x > size.x || screen.y > size.y {
                return None;
            }
            // In view if the head or chest is: nothing but the map has
            // colliders, so any hit short of the zombie is a wall.
            let rapier = rapier.as_ref()?;
            let clear = |target: Vec3| {
                let to = target - cam_pos;
                let dist = to.length();
                dist < 1e-3
                    || rapier
                        .cast_ray(cam_pos, to / dist, dist - 0.05, true, QueryFilter::default())
                        .is_none()
            };
            if !clear(pose.translation + Vec3::Y * 0.1) && !clear(pose.translation - Vec3::Y * 0.45) {
                return None;
            }
            let frac = if bar.max > 0.0 { (health / bar.max).clamp(0.0, 1.0) } else { 1.0 };
            Some((screen, size, frac, fade))
        })();

        let Some((screen, size, frac, fade)) = shown else {
            if *vis != Visibility::Hidden {
                *vis = Visibility::Hidden;
            }
            continue;
        };

        // Bottom-centre of the bar on the anchor.
        node.width = Val::Px(settings.width);
        node.height = Val::Px(settings.thickness);
        node.left = Val::Px(screen.x - settings.width * 0.5);
        node.bottom = Val::Px(size.y - screen.y);
        back.0 = Color::srgba(0.0, 0.0, 0.0, settings.back_alpha * fade);
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        for child in children.iter() {
            if let Ok((mut fill_node, mut fill_color)) = fills.get_mut(child) {
                fill_node.width = Val::Percent(frac * 100.0);
                fill_color.0 = FILL_RED.with_alpha(fade);
            }
        }
    }
}
