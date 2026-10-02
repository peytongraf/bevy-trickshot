//! Big Call of Duty style explosion: a white-hot flash + point light, a
//! rolling HDR fireball (additive, so it blooms), sparks that streak along
//! their flight, charred debris, a ground dust ring and a thick smoke column
//! that the fire hands off to — plus camera shake that falls off with
//! distance. All camera-billboarded sprites, same shape as `impacts.rs`.
//!
//! It's the Bomb Shot perk's blast: the server decides when one goes off
//! (a perk owner's 360 no-scope zombie kill, or any kill with the debug
//! `Lobby::bomb_test` on), does the damage, and sends every client a
//! [`shared::BombExplosion`] to show here. The look is tuned in the debug
//! panel's "Zombies perks" → "Bomb Shot" section ([`explosion_section`]).

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::pbr::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::NoFrustumCulling;
use bevy_egui::egui;
use lightyear::prelude::MessageReceiver;

use super::{cone_dir, smoke_billboard_rotation, ROCK_COLS, ROCK_ROWS};
use crate::player::{Player, PlayerHead};
use crate::util::{rand01, rand_roll};
use crate::{AppState, Shake};

/// Hard cap on live explosion sprites (all kinds together).
const EXPLOSION_MAX: usize = 900;

/// Something should blow up with its base (the ground under it) at this
/// world point — from the server ([`receive_bomb_explosions`]) or the debug
/// panel's preview button. Consumed by `spawn_explosions`.
#[derive(Event)]
pub(crate) struct Explosion {
    pub(crate) feet: Vec3,
    /// Which explosion sound (`% clips`) — the server's pick, so everyone
    /// hears the same one.
    pub(crate) variant: u8,
    /// PhD Flopper's: the same blast, burning purple ([`PhdTint`]).
    pub(crate) phd: bool,
}

/// On a PhD Flopper explosion's sprites: its fire, flash and sparks burn
/// purple instead of orange.
#[derive(Component)]
pub(crate) struct PhdTint;

/// A warm explosion colour turned PhD Flopper purple — the red channel kept
/// as the brightness, the blue raised to match it.
fn purple(c: [f32; 3]) -> [f32; 3] {
    [c[0] * 0.62 + c[2] * 0.1, c[1] * 0.22, c[0] * 0.95 + c[2] * 0.3]
}

/// Panel-adjustable explosion look. `scale` multiplies every size/speed so the
/// whole thing can be grown or shrunk in one go.
#[derive(Resource, Clone)]
pub(crate) struct ExplosionSettings {
    /// The debug panel's "preview" button: set one off in front of the
    /// player (visual only), consumed by `fire_preview`.
    pub(crate) preview_requested: bool,
    pub(crate) scale: f32,
    /// Fireball centre height above the feet (m).
    pub(crate) height: f32,

    pub(crate) flash_size: f32,
    pub(crate) flash_time: f32,
    /// HDR multiplier on the flash sprite (bloom kicks in above ~1).
    pub(crate) flash_brightness: f32,
    pub(crate) light_intensity: f32,
    pub(crate) light_range: f32,
    pub(crate) light_time: f32,

    pub(crate) fire_count: u32,
    pub(crate) fire_size: f32,
    pub(crate) fire_speed: f32,
    pub(crate) fire_rise: f32,
    pub(crate) fire_life: f32,
    pub(crate) fire_brightness: f32,

    pub(crate) smoke_count: u32,
    pub(crate) smoke_size: f32,
    pub(crate) smoke_speed: f32,
    pub(crate) smoke_rise: f32,
    pub(crate) smoke_life: f32,
    pub(crate) smoke_opacity: f32,
    /// Smoke's settled colour (it starts lit orange by the fire).
    pub(crate) smoke_color: [f32; 3],

    pub(crate) spark_count: u32,
    pub(crate) spark_speed: f32,
    pub(crate) spark_size: f32,
    pub(crate) spark_life: f32,
    /// How long a spark's streak gets per m/s of speed.
    pub(crate) spark_stretch: f32,
    pub(crate) spark_brightness: f32,

    pub(crate) debris_count: u32,
    pub(crate) debris_speed: f32,
    pub(crate) debris_size: f32,

    pub(crate) dust_count: u32,
    pub(crate) dust_speed: f32,
    pub(crate) dust_size: f32,
    pub(crate) dust_life: f32,
    pub(crate) dust_opacity: f32,

    /// Camera-shake trauma added standing right on top of it (0..1).
    pub(crate) shake: f32,
    /// Distance (m) past which there's no shake.
    pub(crate) shake_radius: f32,
    /// Distance (m) at which the explosion sound has faded to silence —
    /// much further than other players' sounds, it's a bomb.
    pub(crate) sound_max_distance: f32,
}

impl Default for ExplosionSettings {
    fn default() -> Self {
        Self {
            preview_requested: false,
            scale: 1.0,
            height: 0.9,

            flash_size: 7.0,
            flash_time: 0.14,
            flash_brightness: 12.0,
            light_intensity: 4_000_000.0,
            light_range: 30.0,
            light_time: 0.55,

            fire_count: 34,
            fire_size: 3.2,
            fire_speed: 9.0,
            fire_rise: 1.6,
            fire_life: 0.85,
            fire_brightness: 6.0,

            smoke_count: 30,
            smoke_size: 5.0,
            smoke_speed: 5.0,
            smoke_rise: 2.2,
            smoke_life: 4.5,
            smoke_opacity: 0.75,
            smoke_color: [0.10, 0.095, 0.09],

            spark_count: 70,
            spark_speed: 22.0,
            spark_size: 0.07,
            spark_life: 0.9,
            spark_stretch: 0.09,
            spark_brightness: 10.0,

            debris_count: 22,
            debris_speed: 13.0,
            debris_size: 0.22,

            dust_count: 26,
            dust_speed: 12.0,
            dust_size: 3.2,
            dust_life: 1.6,
            dust_opacity: 0.45,

            shake: 0.9,
            shake_radius: 35.0,
            sound_max_distance: 160.0,
        }
    }
}

/// Shared quad + textures. `glow` (soft radial falloff) and `fire` (billowy
/// noise puffs) are generated at startup; smoke and debris reuse the bullet
/// impact textures.
#[derive(Resource)]
pub(crate) struct ExplosionAssets {
    /// (Shared with PhD Flopper's slide trail, `phd_trail`.)
    pub(crate) quad: Handle<Mesh>,
    glow: Handle<Image>,
    pub(crate) fire: [Handle<Image>; 2],
    smoke: Handle<Image>,
    dust: Handle<Image>,
    rocks: Handle<Image>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Flash,
    Fire,
    Smoke,
    Spark,
    Debris,
    Dust,
}

/// One explosion sprite. Invisible until `delay` has passed, then lives
/// `lifetime` seconds.
#[derive(Component)]
pub(crate) struct ExplosionParticle {
    kind: Kind,
    velocity: Vec3,
    /// Downward acceleration (m/s²); negative floats up (fire / smoke).
    gravity: f32,
    /// Per-second velocity damping.
    drag: f32,
    delay: f32,
    age: f32,
    lifetime: f32,
    roll: f32,
    spin: f32,
    scale0: f32,
    scale1: f32,
    /// Peak opacity (blended) or HDR brightness (additive).
    peak: f32,
    /// Per-sprite 0..1 variation (colour temperature etc).
    vary: f32,
}

/// The explosion's point light, fading out over `lifetime`.
#[derive(Component)]
pub(crate) struct ExplosionLight {
    age: f32,
    lifetime: f32,
    peak: f32,
}

pub(crate) struct ExplosionPlugin;

impl Plugin for ExplosionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExplosionSettings>()
            .add_event::<Explosion>()
            .add_systems(Startup, setup_explosion_assets)
            .add_systems(
                Update,
                (
                    (receive_bomb_explosions, fire_preview),
                    spawn_explosions,
                    update_explosion_particles,
                    update_explosion_lights,
                )
                    .chain()
                    .after(crate::player::look_around)
                    .run_if(in_state(AppState::InGame)),
            );
    }
}

fn setup_explosion_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    commands.insert_resource(ExplosionAssets {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        glow: images.add(glow_image(64)),
        fire: [
            images.add(fire_puff_image(128, 0x51f1)),
            images.add(fire_puff_image(128, 0xb00b5)),
        ],
        smoke: asset_server.load("textures/vfx/smoke.png"),
        dust: asset_server.load("textures/vfx/dust.png"),
        rocks: asset_server.load("textures/vfx/rocks.png"),
    });
}

/// White sprite whose alpha falls off smoothly from the centre.
fn glow_image(size: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let d = (u * u + v * v).sqrt().min(1.0);
            let a = (1.0 - d).powf(2.2);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    rgba_image(size, data)
}

/// Smooth value noise in [0, 1].
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let h = |i: i32, j: i32| {
        rand01((i as u32).wrapping_mul(73_856_093) ^ (j as u32).wrapping_mul(19_349_663) ^ seed)
    };
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (s(fx), s(fy));
    let a = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * sx;
    let b = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * sx;
    a + (b - a) * sy
}

fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for o in 0..5 {
        sum += value_noise(x * freq, y * freq, seed.wrapping_add(o * 7919)) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// A white, billowy, roughly round puff with a noisy cauliflower edge and
/// clumpy interior — the building block of the fireball.
fn fire_puff_image(size: u32, seed: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let d = (u * u + v * v).sqrt();
            let n = fbm(u * 3.0 + 10.0, v * 3.0 + 10.0, seed);
            let detail = fbm(u * 7.0 + 3.0, v * 7.0 + 3.0, seed ^ 0xabcd);
            // Push the edge in and out with the noise, then soften it.
            let edge = d + (n - 0.5) * 0.7;
            let mask = 1.0 - smoothstep(0.45, 0.9, edge);
            let a = (mask * (0.45 + 0.75 * detail)).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    rgba_image(size, data)
}

fn rgba_image(size: u32, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Fresh sprite material, invisible until `update_explosion_particles` first
/// reaches it (so it never shows a frame of opaque white).
fn sprite_material(texture: Handle<Image>, additive: bool) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.0),
        base_color_texture: Some(texture),
        unlit: true,
        alpha_mode: if additive {
            AlphaMode::Add
        } else {
            AlphaMode::Blend
        },
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// Server → us: a Bomb Shot went off — show it.
fn receive_bomb_explosions(
    mut receivers: Query<&mut MessageReceiver<shared::BombExplosion>>,
    mut boom: EventWriter<Explosion>,
) {
    for mut rx in &mut receivers {
        for msg in rx.receive() {
            boom.write(Explosion {
                feet: Vec3::from_array(msg.feet),
                variant: msg.variant,
                phd: msg.phd,
            });
        }
    }
}

/// The debug panel's preview button: one 10 m in front of the player, on the
/// ground they're standing on. Visual only — no damage.
fn fire_preview(
    mut settings: ResMut<ExplosionSettings>,
    player: Single<&Transform, With<Player>>,
    mut boom: EventWriter<Explosion>,
    mut count: Local<u8>,
) {
    if !settings.preview_requested {
        return;
    }
    settings.preview_requested = false;
    *count = count.wrapping_add(1);
    let mut fwd = player.rotation * Vec3::NEG_Z;
    fwd.y = 0.0;
    let feet = player.translation - Vec3::Y * crate::player::EYE_HEIGHT
        + fwd.normalize_or(Vec3::NEG_Z) * 10.0;
    boom.write(Explosion {
        feet,
        variant: *count,
        phd: false,
    });
}

fn spawn_explosions(
    mut events: EventReader<Explosion>,
    settings: Res<ExplosionSettings>,
    assets: Res<ExplosionAssets>,
    player: Single<&Transform, With<Player>>,
    existing: Query<(), With<ExplosionParticle>>,
    mut shake: ResMut<Shake>,
    (sounds, volumes): (Option<Res<crate::GameSounds>>, Res<crate::SoundVolumes>),
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let mut budget = EXPLOSION_MAX.saturating_sub(existing.iter().count());
    let s = &*settings;
    let k = s.scale.max(0.05);

    for ev in events.read() {
        *seq = seq.wrapping_add(1);
        let base = seq.wrapping_mul(2_654_435_761);
        let center = ev.feet + Vec3::Y * s.height * k;

        // Camera shake, falling off with distance.
        let dist = player.translation.distance(center);
        let falloff = (1.0 - dist / s.shake_radius.max(0.1)).clamp(0.0, 1.0);
        shake.trauma = (shake.trauma + s.shake * falloff * falloff).min(1.0);

        // The boom, from the blast (positional), fading with distance.
        let clip = sounds
            .as_ref()
            .filter(|snd| !snd.bomb_shot_explosions.is_empty())
            .map(|snd| {
                snd.bomb_shot_explosions[ev.variant as usize % snd.bomb_shot_explosions.len()].clone()
            });
        let fade = (1.0 - dist / s.sound_max_distance.max(1.0)).clamp(0.0, 1.0);
        let loudness = volumes.bomb_shot_explosion * fade * fade;
        if let (Some(clip), true) = (clip, loudness > 0.0) {
            commands.spawn((
                StateScoped(AppState::InGame),
                // Volume is ours (distance fade), not the one-shot pass's.
                crate::RemoteSoundEmitter,
                AudioPlayer::new(clip),
                Transform::from_translation(center),
                crate::positional_playback(bevy::audio::Volume::Linear(loudness)),
            ));
        }

        commands.spawn((
            StateScoped(AppState::InGame),
            ExplosionLight {
                age: 0.0,
                lifetime: s.light_time.max(0.05),
                peak: s.light_intensity,
            },
            PointLight {
                color: if ev.phd {
                    Color::srgb(0.7, 0.25, 1.0)
                } else {
                    Color::srgb(1.0, 0.62, 0.3)
                },
                intensity: s.light_intensity,
                range: s.light_range * k,
                radius: 0.5 * k,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(center + Vec3::Y * 0.5 * k),
        ));

        let phd = ev.phd;
        let mut spawn = |commands: &mut Commands,
                         materials: &mut Assets<StandardMaterial>,
                         p: ExplosionParticle,
                         material: StandardMaterial,
                         at: Vec3| {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let mut e = commands.spawn((
                StateScoped(AppState::InGame),
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(materials.add(material)),
                Transform::from_translation(at).with_scale(Vec3::splat(p.scale0.max(1.0e-4))),
                p,
                NoFrustumCulling,
                NotShadowCaster,
            ));
            if phd {
                e.insert(PhdTint);
            }
        };

        // White-hot flash.
        spawn(
            &mut commands,
            &mut materials,
            ExplosionParticle {
                kind: Kind::Flash,
                velocity: Vec3::ZERO,
                gravity: 0.0,
                drag: 0.0,
                delay: 0.0,
                age: 0.0,
                lifetime: s.flash_time.max(0.02),
                roll: 0.0,
                spin: 0.0,
                scale0: s.flash_size * k * 0.35,
                scale1: s.flash_size * k,
                peak: s.flash_brightness,
                vary: 0.0,
            },
            sprite_material(assets.glow.clone(), true),
            center,
        );

        // Fireball: puffs thrown out of the centre that stall on drag and
        // roll upward, burning white → orange → deep red.
        for i in 0..s.fire_count {
            let r = base
                .wrapping_add(i.wrapping_mul(40_503))
                .wrapping_add(0xf1e);
            let dir = cone_dir(Vec3::Y, 100f32.to_radians(), r);
            let speed = s.fire_speed * k * (0.35 + 0.65 * rand01(r ^ 0x9e37));
            let size = s.fire_size * k * (0.7 + 0.6 * rand01(r ^ 0x2c));
            let tex = assets.fire[(rand01(r ^ 0x61) * 2.0) as usize % 2].clone();
            spawn(
                &mut commands,
                &mut materials,
                ExplosionParticle {
                    kind: Kind::Fire,
                    velocity: dir * speed + Vec3::Y * s.fire_rise * k,
                    gravity: -2.0 * k,
                    drag: 4.0,
                    // Most puffs pop at once; a few trail in as secondary bursts.
                    delay: rand01(r ^ 0x5d).powi(3) * 0.18,
                    age: 0.0,
                    lifetime: s.fire_life.max(0.05) * (0.65 + 0.7 * rand01(r ^ 0x1234)),
                    roll: rand_roll(r ^ 0x77),
                    spin: (rand01(r ^ 0xab) * 2.0 - 1.0) * 1.2,
                    scale0: size * 0.3,
                    scale1: size,
                    peak: s.fire_brightness,
                    vary: rand01(r ^ 0x3131),
                },
                sprite_material(tex, true),
                center + dir * 0.4 * k * rand01(r ^ 0x44),
            );
        }

        // Smoke: takes over from the fire, billows out and up, lingers.
        for i in 0..s.smoke_count {
            let r = base
                .wrapping_add(i.wrapping_mul(83_492_791))
                .wrapping_add(0x5a0e);
            let dir = cone_dir(Vec3::Y, 85f32.to_radians(), r);
            let speed = s.smoke_speed * k * (0.3 + 0.7 * rand01(r ^ 0x9e37));
            let size = s.smoke_size * k * (0.7 + 0.7 * rand01(r ^ 0x2c));
            spawn(
                &mut commands,
                &mut materials,
                ExplosionParticle {
                    kind: Kind::Smoke,
                    velocity: dir * speed,
                    gravity: -s.smoke_rise * k,
                    drag: 1.6,
                    delay: 0.06 + rand01(r ^ 0x5d) * 0.35,
                    age: 0.0,
                    lifetime: s.smoke_life.max(0.1) * (0.6 + 0.8 * rand01(r ^ 0x1234)),
                    roll: rand_roll(r ^ 0x77),
                    spin: (rand01(r ^ 0xab) * 2.0 - 1.0) * 0.35,
                    scale0: size * 0.35,
                    scale1: size,
                    peak: s.smoke_opacity,
                    vary: rand01(r ^ 0x3131),
                },
                sprite_material(assets.smoke.clone(), false),
                center + dir * 0.6 * k * rand01(r ^ 0x44),
            );
        }

        // Sparks: fast, hot streaks that arc down under gravity.
        for i in 0..s.spark_count {
            let r = base
                .wrapping_add(i.wrapping_mul(2_246_822_519))
                .wrapping_add(0x5a4c);
            let dir = cone_dir(Vec3::Y, 95f32.to_radians(), r);
            let speed = s.spark_speed * k.sqrt() * (0.3 + 0.7 * rand01(r ^ 0x9e37));
            spawn(
                &mut commands,
                &mut materials,
                ExplosionParticle {
                    kind: Kind::Spark,
                    velocity: dir * speed,
                    gravity: 9.8,
                    drag: 0.9,
                    delay: rand01(r ^ 0x5d).powi(4) * 0.1,
                    age: 0.0,
                    lifetime: s.spark_life.max(0.05) * (0.4 + 0.8 * rand01(r ^ 0x1234)),
                    roll: 0.0,
                    spin: 0.0,
                    scale0: s.spark_size * k.sqrt() * (0.6 + 0.8 * rand01(r ^ 0x2c)),
                    scale1: 0.0,
                    peak: s.spark_brightness,
                    vary: rand01(r ^ 0x3131),
                },
                sprite_material(assets.glow.clone(), true),
                center,
            );
        }

        // Charred debris chunks.
        for i in 0..s.debris_count {
            let r = base
                .wrapping_add(i.wrapping_mul(3_266_489_917))
                .wrapping_add(0xdeb);
            let dir = cone_dir(Vec3::Y, 70f32.to_radians(), r);
            let speed = s.debris_speed * k.sqrt() * (0.35 + 0.65 * rand01(r ^ 0x9e37));
            let col = (rand01(r ^ 0x3) * ROCK_COLS as f32) as u32 % ROCK_COLS;
            let row = (rand01(r ^ 0x5) * ROCK_ROWS as f32) as u32 % ROCK_ROWS;
            let mut material = sprite_material(assets.rocks.clone(), false);
            material.uv_transform = Affine2::from_scale_angle_translation(
                Vec2::new(1.0 / ROCK_COLS as f32, 1.0 / ROCK_ROWS as f32),
                0.0,
                Vec2::new(col as f32 / ROCK_COLS as f32, row as f32 / ROCK_ROWS as f32),
            );
            let size = s.debris_size * k.sqrt() * (0.5 + 1.0 * rand01(r ^ 0x2c));
            spawn(
                &mut commands,
                &mut materials,
                ExplosionParticle {
                    kind: Kind::Debris,
                    velocity: dir * speed,
                    gravity: 20.0,
                    drag: 0.15,
                    delay: 0.0,
                    age: 0.0,
                    lifetime: 1.3 + 0.9 * rand01(r ^ 0x1234),
                    roll: rand_roll(r ^ 0x77),
                    spin: (rand01(r ^ 0xab) * 2.0 - 1.0) * 14.0,
                    scale0: size,
                    scale1: size,
                    peak: 1.0,
                    vary: rand01(r ^ 0x3131),
                },
                material,
                center,
            );
        }

        // Dust ring racing out along the ground.
        for i in 0..s.dust_count {
            let r = base
                .wrapping_add(i.wrapping_mul(1_597_334_677))
                .wrapping_add(0xd057);
            let a = (i as f32 + rand01(r ^ 0x7) * 0.8) / s.dust_count.max(1) as f32
                * std::f32::consts::TAU;
            let dir = Vec3::new(a.cos(), 0.0, a.sin());
            let speed = s.dust_speed * k * (0.6 + 0.4 * rand01(r ^ 0x9e37));
            let size = s.dust_size * k * (0.7 + 0.6 * rand01(r ^ 0x2c));
            spawn(
                &mut commands,
                &mut materials,
                ExplosionParticle {
                    kind: Kind::Dust,
                    velocity: dir * speed + Vec3::Y * 0.8 * k,
                    gravity: 0.0,
                    drag: 3.2,
                    delay: 0.02,
                    age: 0.0,
                    lifetime: s.dust_life.max(0.1) * (0.7 + 0.6 * rand01(r ^ 0x1234)),
                    roll: rand_roll(r ^ 0x77),
                    spin: 0.0,
                    scale0: size * 0.25,
                    scale1: size,
                    peak: s.dust_opacity,
                    vary: rand01(r ^ 0x3131),
                },
                sprite_material(assets.dust.clone(), false),
                ev.feet + Vec3::Y * 0.3 * k + dir * 0.5 * k,
            );
        }
    }
}

/// Integrate, billboard, grow and colour every live explosion sprite; despawn
/// (freeing its material) at end of life.
fn update_explosion_particles(
    time: Res<Time>,
    settings: Res<ExplosionSettings>,
    player: Single<&Transform, (With<Player>, Without<ExplosionParticle>)>,
    head: Single<&Transform, (With<PlayerHead>, Without<ExplosionParticle>)>,
    mut particles: Query<(
        Entity,
        &mut Transform,
        &mut ExplosionParticle,
        &MeshMaterial3d<StandardMaterial>,
        Has<PhdTint>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let cam_pos = player.translation;
    let cam_rot = player.rotation * head.rotation;
    let cam_up = cam_rot * Vec3::Y;
    let cam_right = cam_rot * Vec3::X;

    for (entity, mut transform, mut p, material, phd) in &mut particles {
        if p.delay > 0.0 {
            p.delay -= dt;
            continue;
        }
        p.age += dt;
        if p.age >= p.lifetime {
            materials.remove(&material.0);
            commands.entity(entity).despawn();
            continue;
        }

        let (gravity, drag) = (p.gravity, p.drag);
        p.velocity.y -= gravity * dt;
        p.velocity *= (1.0 - drag * dt).max(0.0);
        transform.translation += p.velocity * dt;

        let f = (p.age / p.lifetime.max(1.0e-4)).clamp(0.0, 1.0);
        // Fast out, slow settle — explosions punch out then hang.
        let grow = 1.0 - (1.0 - f).powi(3);
        let scale = (p.scale0 + (p.scale1 - p.scale0) * grow).max(1.0e-4);

        let color = match p.kind {
            Kind::Flash => {
                let b = p.peak * (1.0 - f).powi(2);
                transform.scale = Vec3::splat(scale);
                LinearRgba::new(b, b * 0.85, b * 0.6, 1.0)
            }
            Kind::Fire => {
                transform.scale = Vec3::splat(scale);
                // Hotter puffs start whiter; all cool through orange to red.
                let t = (f * (1.1 - 0.3 * p.vary)).clamp(0.0, 1.0);
                let c = if t < 0.3 {
                    lerp3([1.0, 0.85, 0.55], [1.0, 0.45, 0.1], t / 0.3)
                } else {
                    lerp3([1.0, 0.45, 0.1], [0.45, 0.07, 0.015], (t - 0.3) / 0.7)
                };
                let fade_in = (p.age / 0.03).min(1.0);
                let b = p.peak * fade_in * (1.0 - f).powf(1.3);
                LinearRgba::new(c[0] * b, c[1] * b, c[2] * b, 1.0)
            }
            Kind::Smoke => {
                transform.scale = Vec3::splat(scale);
                // Lit orange by the fire at first, then settling to soot.
                let lit = 1.0 - smoothstep(0.0, 0.18, f);
                let c = lerp3(settings.smoke_color, [0.55, 0.22, 0.07], lit * 0.8);
                let a = smoothstep(0.0, 0.1, f) * (1.0 - smoothstep(0.45, 1.0, f));
                LinearRgba::from(Color::srgba(c[0], c[1], c[2], p.peak * a))
            }
            Kind::Spark => {
                let speed = p.velocity.length();
                let len = p.scale0 + speed * settings.spark_stretch;
                transform.scale = Vec3::new(p.scale0.max(1.0e-4), len.max(1.0e-4), 1.0);
                let b = p.peak * (1.0 - f);
                let c = lerp3([1.0, 0.75, 0.35], [1.0, 0.35, 0.08], f.max(p.vary * 0.5));
                LinearRgba::new(c[0] * b, c[1] * b, c[2] * b, 1.0)
            }
            Kind::Debris => {
                transform.scale = Vec3::splat(scale);
                let a = 1.0 - smoothstep(0.7, 1.0, f);
                let g = 0.18 + 0.1 * p.vary;
                LinearRgba::from(Color::srgba(g, g * 0.88, g * 0.8, a))
            }
            Kind::Dust => {
                transform.scale = Vec3::splat(scale);
                let a = smoothstep(0.0, 0.08, f) * (1.0 - f).powf(1.5);
                LinearRgba::from(Color::srgba(0.5, 0.45, 0.38, p.peak * a))
            }
        };

        transform.rotation = if p.kind == Kind::Spark {
            // Stretch the streak along its flight, as seen from the camera.
            let to_cam = (cam_pos - transform.translation).normalize_or(Vec3::Y);
            let along = p.velocity - to_cam * p.velocity.dot(to_cam);
            if along.length_squared() > 1.0e-6 {
                let up = along.normalize();
                let right = up.cross(to_cam);
                Quat::from_mat3(&Mat3::from_cols(right, up, to_cam))
            } else {
                smoke_billboard_rotation(transform.translation, cam_pos, cam_up, cam_right, 0.0)
            }
        } else {
            smoke_billboard_rotation(
                transform.translation,
                cam_pos,
                cam_up,
                cam_right,
                p.roll + p.spin * p.age,
            )
        };

        // PhD Flopper's burns purple (its debris and dust stay as they are).
        let color = if phd && matches!(p.kind, Kind::Flash | Kind::Fire | Kind::Spark | Kind::Smoke) {
            let [r, g, b] = purple([color.red, color.green, color.blue]);
            LinearRgba::new(r, g, b, color.alpha)
        } else {
            color
        };
        if let Some(m) = materials.get_mut(&material.0) {
            m.base_color = Color::LinearRgba(color);
        }
    }
}

fn update_explosion_lights(
    time: Res<Time>,
    mut lights: Query<(Entity, &mut ExplosionLight, &mut PointLight)>,
    mut commands: Commands,
) {
    for (entity, mut l, mut light) in &mut lights {
        l.age += time.delta_secs();
        if l.age >= l.lifetime {
            commands.entity(entity).despawn();
            continue;
        }
        let f = l.age / l.lifetime;
        // A little flicker on the way down.
        let flicker = 0.85 + 0.15 * (l.age * 60.0).sin();
        light.intensity = l.peak * (1.0 - f).powi(2) * flicker;
    }
}

/// The debug panel's "Bomb Shot" explosion controls: a preview button and
/// every slider for the look. (The server-side "every kill explodes" test
/// toggle sits next to it in `debug_ui`, since it needs the lobby.)
pub(crate) fn explosion_section(ui: &mut egui::Ui, s: &mut ExplosionSettings) {
    if ui
        .button("Preview: explode 10 m in front of me (no damage)")
        .clicked()
    {
        s.preview_requested = true;
    }
    ui.separator();
    ui.add(egui::Slider::new(&mut s.scale, 0.2f32..=3.0).text("overall scale"));
    ui.add(egui::Slider::new(&mut s.height, 0.0f32..=3.0).text("height above feet (m)"));
    ui.collapsing("Flash + light", |ui| {
        ui.add(egui::Slider::new(&mut s.flash_size, 0.0f32..=20.0).text("flash size (m)"));
        ui.add(egui::Slider::new(&mut s.flash_time, 0.02f32..=0.6).text("flash time (s)"));
        ui.add(egui::Slider::new(&mut s.flash_brightness, 0.0f32..=40.0).text("flash brightness"));
        ui.add(
            egui::Slider::new(&mut s.light_intensity, 0.0f32..=40_000_000.0)
                .logarithmic(true)
                .text("light intensity (lm)"),
        );
        ui.add(egui::Slider::new(&mut s.light_range, 1.0f32..=100.0).text("light range (m)"));
        ui.add(egui::Slider::new(&mut s.light_time, 0.05f32..=3.0).text("light time (s)"));
    });
    ui.collapsing("Fireball", |ui| {
        ui.add(egui::Slider::new(&mut s.fire_count, 0u32..=120).text("puffs"));
        ui.add(egui::Slider::new(&mut s.fire_size, 0.2f32..=10.0).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.fire_speed, 0.0f32..=30.0).text("burst speed (m/s)"));
        ui.add(egui::Slider::new(&mut s.fire_rise, 0.0f32..=10.0).text("rise (m/s)"));
        ui.add(egui::Slider::new(&mut s.fire_life, 0.1f32..=3.0).text("life (s)"));
        ui.add(egui::Slider::new(&mut s.fire_brightness, 0.0f32..=30.0).text("brightness (HDR)"));
    });
    ui.collapsing("Smoke", |ui| {
        ui.add(egui::Slider::new(&mut s.smoke_count, 0u32..=120).text("puffs"));
        ui.add(egui::Slider::new(&mut s.smoke_size, 0.2f32..=15.0).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.smoke_speed, 0.0f32..=20.0).text("burst speed (m/s)"));
        ui.add(egui::Slider::new(&mut s.smoke_rise, 0.0f32..=10.0).text("buoyancy"));
        ui.add(egui::Slider::new(&mut s.smoke_life, 0.2f32..=15.0).text("life (s)"));
        ui.add(egui::Slider::new(&mut s.smoke_opacity, 0.0f32..=1.0).text("opacity"));
        ui.horizontal(|ui| {
            ui.label("colour");
            ui.color_edit_button_rgb(&mut s.smoke_color);
        });
    });
    ui.collapsing("Sparks", |ui| {
        ui.add(egui::Slider::new(&mut s.spark_count, 0u32..=300).text("count"));
        ui.add(egui::Slider::new(&mut s.spark_speed, 0.0f32..=60.0).text("speed (m/s)"));
        ui.add(egui::Slider::new(&mut s.spark_size, 0.01f32..=0.4).text("width (m)"));
        ui.add(egui::Slider::new(&mut s.spark_stretch, 0.0f32..=0.4).text("streak length"));
        ui.add(egui::Slider::new(&mut s.spark_life, 0.1f32..=3.0).text("life (s)"));
        ui.add(egui::Slider::new(&mut s.spark_brightness, 0.0f32..=40.0).text("brightness (HDR)"));
    });
    ui.collapsing("Debris", |ui| {
        ui.add(egui::Slider::new(&mut s.debris_count, 0u32..=100).text("chunks"));
        ui.add(egui::Slider::new(&mut s.debris_speed, 0.0f32..=40.0).text("speed (m/s)"));
        ui.add(egui::Slider::new(&mut s.debris_size, 0.02f32..=1.0).text("size (m)"));
    });
    ui.collapsing("Ground dust ring", |ui| {
        ui.add(egui::Slider::new(&mut s.dust_count, 0u32..=100).text("puffs"));
        ui.add(egui::Slider::new(&mut s.dust_speed, 0.0f32..=40.0).text("speed (m/s)"));
        ui.add(egui::Slider::new(&mut s.dust_size, 0.2f32..=10.0).text("size (m)"));
        ui.add(egui::Slider::new(&mut s.dust_life, 0.1f32..=5.0).text("life (s)"));
        ui.add(egui::Slider::new(&mut s.dust_opacity, 0.0f32..=1.0).text("opacity"));
    });
    ui.collapsing("Camera shake", |ui| {
        ui.add(egui::Slider::new(&mut s.shake, 0.0f32..=1.0).text("trauma up close"));
        ui.add(egui::Slider::new(&mut s.shake_radius, 1.0f32..=100.0).text("radius (m)"));
    });
    ui.collapsing("Sound", |ui| {
        ui.add(
            egui::Slider::new(&mut s.sound_max_distance, 10.0f32..=400.0)
                .text("heard up to (m)"),
        );
        ui.label("Loudness: Sound volumes → \"zombies: bomb shot explosion\".");
    });
    ui.separator();
    if ui.button("Copy explosion settings to console").clicked() {
        info!(
                    "explosion: scale {:.2}, height {:.2} | flash size {:.2} time {:.2} brightness {:.1} \
                     light {:.0} range {:.1} time {:.2} | fire n {} size {:.2} speed {:.1} rise {:.1} \
                     life {:.2} brightness {:.1} | smoke n {} size {:.2} speed {:.1} rise {:.1} life {:.2} \
                     opacity {:.2} colour {:?} | sparks n {} speed {:.1} size {:.3} stretch {:.3} life {:.2} \
                     brightness {:.1} | debris n {} speed {:.1} size {:.2} | dust n {} speed {:.1} size {:.2} \
                     life {:.2} opacity {:.2} | shake {:.2} radius {:.1}",
                    s.scale, s.height, s.flash_size, s.flash_time, s.flash_brightness,
                    s.light_intensity, s.light_range, s.light_time, s.fire_count, s.fire_size,
                    s.fire_speed, s.fire_rise, s.fire_life, s.fire_brightness, s.smoke_count,
                    s.smoke_size, s.smoke_speed, s.smoke_rise, s.smoke_life, s.smoke_opacity,
                    s.smoke_color, s.spark_count, s.spark_speed, s.spark_size, s.spark_stretch,
                    s.spark_life, s.spark_brightness, s.debris_count, s.debris_speed, s.debris_size,
                    s.dust_count, s.dust_speed, s.dust_size, s.dust_life, s.dust_opacity, s.shake,
                    s.shake_radius,
                );
    }
    if ui.button("Reset look").clicked() {
        *s = default();
    }
}
