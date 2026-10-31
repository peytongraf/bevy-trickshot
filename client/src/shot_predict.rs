//! Our own shots, Ray Gun bolts and thrown knives, shown landing the moment
//! they do here — hit marker, damage number, blood, and for a kill the
//! enemy's fall, the kill sound and its points — rather than a round trip
//! later, as Call of Duty does. The knife's stab does the same
//! (`melee_predict`, which shows it through [`Feedback::show`] too).
//!
//! A shot's foreseen exactly as the server will resolve it: the hits it
//! claims (`hit_detection`, tested on the models as drawn here) go through
//! `shared::ballistics::resolve_claimed_hits` — the server's own piercing,
//! range and damage — then each one's damage (Pack-a-Punch, penetration,
//! Insta-Kill, a weapon's PvP softening) against the health its target has
//! by the server's last word (`PlayerHealth`, a `Freestyle` target's
//! `Bot::health`). A kill's points are the same sums the server pays
//! (`shared::scoring`).
//!
//! The server stays the judge: its own hit markers, numbers, blood and
//! points for what we foresaw are passed over (`melee_predict::Predicted`),
//! anything it says we didn't foresee still shows, and a body we saw fall
//! that the server says is still standing gets back up
//! ([`crate::melee_predict::CONFIRM_SECS`] on).
//!
//! What's foreseen lives in `melee_predict::Predicted` (cleared leaving a
//! game); a body's fall in a `PredictedDeath` on its avatar.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::input::native::{ActionState, InputMarker};
use lightyear::prelude::{LocalId, PeerId};
use shared::ballistics::{resolve_claimed_hits, Claim, Target};
use shared::hitbox::Capsule;
use shared::weapon::{SlotWeapon, WeaponId};
use shared::{GameMode, Lobby, PlayerInput, PlayerPose, Projectile};

use crate::audio_engine::prelude::*;
use crate::hit_detection::{Kind, ModelHit, ShotModels};
use crate::melee_predict::{Healths, PredictedDeath};
use crate::net::{BotPose, GameClient};
use crate::AppState;

pub(crate) struct ShotPredictPlugin;

impl Plugin for ShotPredictPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<ShotFired>()
            .add_event::<ProjectileStruck>()
            .add_systems(Update, show_shots.run_if(in_state(AppState::InGame)));
    }
}

/// A shot that just went up (`net::write_input`): what it sent, and the
/// models it struck on our screen.
#[derive(Event)]
pub(crate) struct ShotFired {
    origin: Vec3,
    dir: Vec3,
    weapon: WeaponId,
    spin_deg: f32,
    airborne: bool,
    noscope: bool,
    hits: Vec<ModelHit>,
}

/// One of our Ray Gun bolts or thrown knives struck an enemy
/// (`hit_detection::track_own_projectiles`, as it claims it).
#[derive(Event)]
pub(crate) struct ProjectileStruck {
    pub(crate) projectile: Projectile,
    pub(crate) origin: Vec3,
    pub(crate) hit: ModelHit,
}

/// Right after `net::write_input`: the shot it just sent, if any, with the
/// models `hit_detection::claim_shot_hits` found it struck.
pub(crate) fn note_shot(
    input: Query<&ActionState<PlayerInput>, With<InputMarker<PlayerInput>>>,
    mut models: ResMut<ShotModels>,
    mut fired: EventWriter<ShotFired>,
) {
    let hits = std::mem::take(&mut models.0);
    let Ok(input) = input.single() else { return };
    if !input.fire || input.fire_hits.is_none() || hits.is_empty() {
        return;
    }
    let Some(weapon) = WeaponId::from_u8(input.weapon) else { return };
    info!("[predict] shot from {:.2} ({} claims)", Vec3::from_array(input.fire_origin), hits.len());
    fired.write(ShotFired {
        origin: Vec3::from_array(input.fire_origin),
        dir: Vec3::from_array(input.fire_dir),
        weapon,
        spin_deg: input.spin_deg,
        airborne: input.airborne,
        noscope: input.noscope,
        hits,
    });
}

/// One enemy a shot / stab / throw landed on, as we foresee it.
pub(crate) struct Landed {
    pub(crate) victim: Option<PeerId>,
    pub(crate) avatar: Entity,
    /// Its avatar's source (a `PlayerPose` entity, or a `Freestyle`
    /// target's `BotPose` stand-in).
    pub(crate) src: Entity,
    pub(crate) point: Vec3,
    /// The way it went in.
    pub(crate) dir: Vec3,
    pub(crate) kind: Kind,
    pub(crate) dog: bool,
    pub(crate) boss: bool,
    pub(crate) kill: bool,
    /// A zombie's damage number, and whether it's a critical (yellow) one.
    pub(crate) number: Option<(u32, bool)>,
    /// A zombie's kill: whether it's a critical one (a headshot, a knife).
    pub(crate) critical: bool,
}

/// Points we foresee a shot scoring all at once (a `Freestyle` kill or
/// collateral) — a zombie's are per kill, worked out in [`Feedback::show`].
pub(crate) type Score = (u32, Vec<shared::ScoreLine>);

/// Everything a foreseen hit shows through.
#[derive(SystemParam)]
pub(crate) struct Feedback<'w, 's> {
    time: Res<'w, Time>,
    predicted: ResMut<'w, crate::melee_predict::Predicted>,
    blood_settings: Res<'w, crate::vfx::EnemyBloodSettings>,
    sounds: Res<'w, crate::GameSounds>,
    local: Query<'w, 's, &'static LocalId, With<GameClient>>,
    lobbies: Query<'w, 's, &'static Lobby>,
    poses: Query<'w, 's, &'static PlayerPose>,
    stand_ins: Query<'w, 's, &'static mut BotPose>,
    blood_rec: ResMut<'w, crate::killcam::ReplayBloodImpact>,
    blood: EventWriter<'w, crate::vfx::BloodImpact>,
    markers: EventWriter<'w, crate::hit_marker::LocalHitMarker>,
    numbers: EventWriter<'w, crate::hud::ShowDamageNumber>,
    scored: EventWriter<'w, crate::TrickScoredEvent>,
    commands: Commands<'w, 's>,
}

impl Feedback<'_, '_> {
    /// Show what landed: each one's hit marker, blood and damage number,
    /// and each kill's fall and points (a zombie's its own; a `Freestyle`
    /// kill's `score`, once for them all) — with the kill sound.
    pub(crate) fn show(&mut self, landed: &[Landed], score: Option<Score>) {
        let now = self.time.elapsed_secs();
        let zombies = crate::zombies_hud::zombies_game(&self.local, &self.lobbies).is_some();
        let double = crate::zombies_hud::my_lobby(&self.local, &self.lobbies)
            .is_some_and(|l| l.power_up_active(shared::power_ups::PowerUp::DoublePoints));
        // (A hit that didn't kill matches a server marker first: a kill one
        // only ever matches a kill.)
        let mut order: Vec<&Landed> = landed.iter().collect();
        order.sort_by_key(|l| l.kill);
        for l in order {
            let zombie = l.kind == Kind::Zombie;
            if zombie {
                let enemy = if l.dog {
                    &self.blood_settings.dog
                } else if l.boss {
                    &self.blood_settings.boss
                } else {
                    &self.blood_settings.zombie
                };
                self.blood.write(crate::vfx::BloodImpact {
                    point: l.point,
                    dir: l.dir,
                    style: Some(enemy.burst(l.kill)),
                });
            }
            self.markers.write(crate::hit_marker::LocalHitMarker {
                kill: l.kill,
                victim: l.victim,
            });
            if let Some((damage, critical)) = l.number {
                self.numbers.write(crate::hud::ShowDamageNumber {
                    point: l.point,
                    damage,
                    critical,
                });
            }

            let mut points = None;
            let mut ffa_kill = false;
            if l.kill {
                match l.kind {
                    // A zombie's (or hellhound's) kill pays its own points —
                    // its body falls (a hellhound's blows up when the server
                    // says so; a boss is left to the server altogether).
                    Kind::Zombie if !l.boss => {
                        let mult = if double { 2 } else { 1 };
                        let mut lines = vec![("KILL".to_string(), shared::ZOMBIE_KILL_POINTS * mult)];
                        if l.critical {
                            lines.push(("CRITICAL".to_string(), shared::ZOMBIE_CRITICAL_POINTS * mult));
                        }
                        let total = lines.iter().map(|(_, p)| p).sum();
                        points = Some(total);
                        self.scored.write(crate::TrickScoredEvent {
                            total,
                            color: crate::hud::score_color(true, &lines),
                            lines,
                            sound: None,
                        });
                        if !l.dog {
                            self.commands.entity(l.avatar).try_insert(PredictedDeath::new(now));
                        }
                    }
                    Kind::Zombie => {}
                    // A target topples, with the burst out of its chest the
                    // server's own death would have set off.
                    Kind::Bot => {
                        self.commands.entity(l.avatar).try_insert(PredictedDeath::new(now));
                        if let (Ok(mut stand_in), Ok(pose)) = (self.stand_ins.get_mut(l.src), self.poses.get(l.src)) {
                            if !stand_in.died {
                                stand_in.died = true;
                                let point = pose.translation - Vec3::Y * crate::EYE_HEIGHT
                                    + Vec3::Y * (shared::bots::BOT_HEIGHT * 0.55);
                                self.blood.write(crate::vfx::BloodImpact { point, dir: Vec3::Y, style: None });
                                self.blood_rec.0 = Some(point);
                            }
                        }
                    }
                    // Another player: down they go, and the kill sound (the
                    // server's comes as our score goes up — `lobby_ui`).
                    Kind::Player => {
                        self.commands.entity(l.avatar).try_insert(PredictedDeath::new(now));
                        self.commands.spawn((
                            AudioPlayer::new(self.sounds.kill_enemy.clone()),
                            PlaybackSettings::DESPAWN,
                        ));
                        ffa_kill = true;
                    }
                }
            }
            self.predicted.foresee(crate::melee_predict::Foreseen::new(
                now,
                l.victim,
                l.point,
                l.kill,
                zombie,
                l.number.is_some(),
                points,
                ffa_kill,
            ));
        }
        if let Some((total, lines)) = score {
            let lines: Vec<(String, u32)> = lines.into_iter().map(|l| (l.label, l.points)).collect();
            self.scored.write(crate::TrickScoredEvent {
                total,
                color: crate::hud::score_color(zombies, &lines),
                lines,
                sound: None,
            });
            self.predicted.foresee(crate::melee_predict::Foreseen::score_only(now, total));
        }
    }
}

/// What we need to work out a hit's damage and whether it kills.
#[derive(SystemParam)]
pub(crate) struct Judge<'w, 's> {
    local: Query<'w, 's, &'static LocalId, With<GameClient>>,
    lobbies: Query<'w, 's, &'static Lobby>,
    healths: Healths<'w, 's>,
}

impl Judge<'_, '_> {
    pub(crate) fn me(&self) -> Option<PeerId> {
        self.local.iter().next().map(|l| l.0)
    }

    pub(crate) fn lobby(&self) -> Option<&Lobby> {
        let me = self.me()?;
        self.lobbies.iter().find(|l| l.has(me))
    }

    /// Our Pack-a-Punch damage multiplier for `weapon`.
    fn pap(&self, weapon: shared::pap::PapWeapon) -> f32 {
        let me = self.me();
        self.lobby()
            .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
            .map_or(1.0, |m| shared::pap::damage_mult(m.pap.get(weapon)))
    }

    fn insta(&self) -> bool {
        self.lobby()
            .is_some_and(|l| l.power_up_active(shared::power_ups::PowerUp::InstaKill))
    }

    /// A `Freestyle` target's health left.
    fn bot_health(&self, hit: &ModelHit) -> f32 {
        hit.bot_health.unwrap_or(shared::bots::BOT_HEALTH)
    }

    /// `damage` landing on a zombie (or hellhound, or boss): whether it
    /// kills, and the number it shows — as `server::pvp::apply_player_hits`
    /// takes it (an Insta-Kill's every bit of it, bar a boss's).
    fn on_zombie(&self, hit: &ModelHit, damage: f32) -> (bool, Option<u32>) {
        let health = self.healths.of(hit.src, hit.victim).unwrap_or(0.0);
        let insta = self.insta() && !hit.boss;
        let taken = if insta { health } else { damage.min(health) };
        // (Diagnostics: kills we foresee that the server doesn't agree with.)
        info!(
            "[predict] {:?}: {damage:.0} of {health:.0} health{} at {:?} -> {}",
            hit.victim,
            if insta { " (insta)" } else { "" },
            hit.claim.point,
            if health > 0.0 && (insta || damage >= health) { "KILL" } else { "hit" },
        );
        (
            health > 0.0 && (insta || damage >= health),
            (health > 0.0).then(|| taken.round().max(1.0) as u32),
        )
    }

    /// `damage` landing on another player: whether it kills — never a
    /// teammate (`SearchAndDestroy`), who it doesn't hurt.
    fn on_player(&self, hit: &ModelHit, damage: f32) -> Option<bool> {
        let victim = hit.victim?;
        let lobby = self.lobby()?;
        let me = self.me()?;
        if lobby.teammates(me, victim) {
            return None;
        }
        let perks = lobby.members.iter().find(|m| m.peer == victim).map_or(&[][..], |m| m.perks.as_slice());
        let health = self.healths.of(hit.src, hit.victim).unwrap_or(0.0);
        Some(health > 0.0 && shared::perks::damage_taken(perks, damage) >= health)
    }
}

fn landed(hit: &ModelHit, from: Vec3, kill: bool, number: Option<(u32, bool)>, critical: bool) -> Landed {
    let point = Vec3::from_array(hit.claim.point);
    Landed {
        victim: hit.victim,
        avatar: hit.avatar,
        src: hit.src,
        point,
        dir: (point - from).normalize_or(Vec3::NEG_Z),
        kind: hit.kind,
        dog: hit.dog,
        boss: hit.boss,
        kill,
        number,
        critical,
    }
}

/// Show each shot and projectile that just landed (see the module docs).
fn show_shots(
    mut shots: EventReader<ShotFired>,
    mut struck: EventReader<ProjectileStruck>,
    judge: Judge,
    rapier: ReadRapierContext,
    mut feedback: Feedback,
) {
    for shot in shots.read() {
        let (landed, score) = foresee_shot(shot, &judge, &rapier);
        if !landed.is_empty() {
            feedback.show(&landed, score);
        }
    }
    for p in struck.read() {
        let (landed, score) = foresee_projectile(p, &judge);
        if let Some(landed) = landed {
            feedback.show(&[landed], score);
        }
    }
}

/// What a bullet does, by `server::sim::resolve_shots`' rules.
fn foresee_shot(shot: &ShotFired, judge: &Judge, rapier: &ReadRapierContext) -> (Vec<Landed>, Option<Score>) {
    let none = (Vec::new(), None);
    let (Some(lobby), Some(me)) = (judge.lobby(), judge.me()) else { return none };
    let mode = lobby.mode;
    // (Anything but the sniper only from a member carrying it, in a mode
    // with a loadout — otherwise the server fires the sniper.)
    let mut weapon = shot.weapon;
    if weapon != WeaponId::Sniper {
        let carried = lobby
            .members
            .iter()
            .any(|m| m.peer == me && m.weapons.contains(&SlotWeapon::Gun(weapon)));
        if !mode.has_loadout() || !carried {
            weapon = WeaponId::Sniper;
        }
    }
    if weapon == WeaponId::RayGun {
        return none;
    }
    let aim = shot.dir.normalize_or_zero();
    let max_range = weapon.spec().max_range;
    let wall = rapier
        .single()
        .ok()
        .and_then(|r| r.cast_ray(shot.origin, aim, max_range, true, QueryFilter::only_fixed()))
        .map(|(_, d)| d);

    // Who the server has as a target: `Zombies` players and zombies only
    // each other; every other player otherwise (a `Freestyle` player's
    // struck for nothing, but still eats into the bullet's reach); and a
    // `Freestyle` game's targets.
    let target = |h: &ModelHit| match (mode, h.kind) {
        (GameMode::Zombies, kind) => kind == Kind::Zombie,
        (GameMode::Freestyle, _) => true,
        (_, kind) => kind == Kind::Player,
    };
    let candidates: Vec<&ModelHit> = shot.hits.iter().filter(|h| target(h)).collect();
    // Each one a target of its own, right where it was struck — so only the
    // server's checks on the path itself (reach, walls, piercing) apply.
    let targets: Vec<Target> = candidates
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let at = Vec3::from_array(h.claim.point);
            let capsule = Capsule { a: at, b: at, radius: 0.0 };
            Target { id: i as u64, body: capsule, head: capsule }
        })
        .collect();
    let claims: Vec<Claim> = candidates
        .iter()
        .enumerate()
        .map(|(i, h)| Claim {
            target: i as u64,
            zone: h.claim.zone,
            point: Vec3::from_array(h.claim.point),
        })
        .collect();
    let hits = resolve_claimed_hits(weapon, shot.origin, aim, &claims, &targets, wall);

    let mut out = Vec::new();
    match mode {
        GameMode::Freestyle => {
            // Every target it pierced takes its hit; the shot's points go
            // once for all it killed, off the nearest one's distance.
            let mut killed: Vec<f32> = Vec::new();
            for hit in &hits {
                let model = candidates[hit.target as usize];
                if model.kind != Kind::Bot {
                    continue;
                }
                let kill = judge.bot_health(model) - hit.damage <= 0.0;
                if kill {
                    killed.push(hit.distance);
                }
                out.push(landed(model, shot.origin, kill, None, false));
            }
            let score = killed.first().map(|&distance| {
                shared::scoring::score_multi_kill(
                    shot.spin_deg,
                    shot.airborne,
                    shot.noscope,
                    killed.len() as u32,
                    distance,
                    max_range,
                )
            });
            (out, score)
        }
        GameMode::Zombies => {
            // The nearest zombie takes it all; the ones behind, less for
            // each it's gone through, until the round gives out.
            let pap = judge.pap(shared::pap::PapWeapon::gun(weapon));
            for (pierced, hit) in hits.iter().enumerate() {
                let keep = if pierced == 0 {
                    1.0
                } else {
                    match weapon.penetration_damage_mult(pierced) {
                        Some(keep) => keep,
                        None => break,
                    }
                };
                let model = candidates[hit.target as usize];
                let (kill, number) = judge.on_zombie(model, hit.damage * pap * keep);
                out.push(landed(model, shot.origin, kill, number.map(|n| (n, hit.headshot)), hit.headshot));
            }
            (out, None)
        }
        // A player's only the nearest one hit (a teammate stops it dead,
        // hurting no one).
        _ => {
            if let Some(hit) = hits.first() {
                let model = candidates[hit.target as usize];
                if let Some(kill) = judge.on_player(model, hit.damage * weapon.pvp_damage_mult()) {
                    out.push(landed(model, shot.origin, kill, None, false));
                }
            }
            (out, None)
        }
    }
}

/// What a Ray Gun bolt's direct hit or a thrown knife does, by
/// `server::raygun` / `server::knives`' rules. (A bolt's burst around it
/// is left to the server.)
fn foresee_projectile(p: &ProjectileStruck, judge: &Judge) -> (Option<Landed>, Option<Score>) {
    let Some(lobby) = judge.lobby() else { return (None, None) };
    let hit = &p.hit;
    let headshot = hit.claim.zone == shared::ballistics::HitZone::Head;
    match (p.projectile, hit.kind) {
        (Projectile::RayGun, Kind::Zombie) => {
            let damage = WeaponId::RayGun.spec().base_damage * judge.pap(shared::pap::PapWeapon::RayGun);
            let (kill, number) = judge.on_zombie(hit, damage);
            (Some(landed(hit, p.origin, kill, number.map(|n| (n, headshot)), headshot)), None)
        }
        (Projectile::Knife, Kind::Zombie) => {
            let damage = if hit.boss {
                shared::boss::throwing_knife_damage(lobby.round)
            } else {
                shared::zombies::ZOMBIE_THROWING_KNIFE_DAMAGE
            };
            let (kill, number) = judge.on_zombie(hit, damage);
            (Some(landed(hit, p.origin, kill, number.map(|n| (n, false)), false)), None)
        }
        (Projectile::Knife, Kind::Bot) => (
            Some(landed(hit, p.origin, true, None, false)),
            Some(shared::scoring::score_throwing_knife_kill()),
        ),
        (Projectile::Knife, Kind::Player) => match judge.on_player(hit, shared::melee::KNIFE_DAMAGE) {
            Some(kill) => (Some(landed(hit, p.origin, kill, None, false)), None),
            None => (None, None),
        },
        _ => (None, None),
    }
}

/// A knife stab that struck (`melee_predict`): what it does, by
/// `server::sim::resolve_shots`' melee rules — a zombie takes the knife's
/// damage (with our Pack-a-Punch), a target always dies, a player takes a
/// knife's lethal damage.
pub(crate) fn foresee_stab(hit: &ModelHit, eye: Vec3, judge: &Judge) -> (Option<Landed>, Option<Score>) {
    match hit.kind {
        Kind::Zombie => {
            let damage = shared::zombies::ZOMBIE_KNIFE_DAMAGE * judge.pap(shared::pap::PapWeapon::Knife);
            let (kill, number) = judge.on_zombie(hit, damage);
            (Some(landed(hit, eye, kill, number.map(|n| (n, true)), true)), None)
        }
        Kind::Bot => (Some(landed(hit, eye, true, None, false)), Some(shared::scoring::score_knife_kill())),
        Kind::Player => match judge.on_player(hit, shared::melee::KNIFE_DAMAGE) {
            Some(kill) => (Some(landed(hit, eye, kill, None, false)), None),
            None => (None, None),
        },
    }
}
