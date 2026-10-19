//! Our own knife stabs, shown landing the moment the thrust gets there —
//! as Call of Duty does — rather than a round trip later.
//!
//! When the thrust reaches out (`shared::melee::KNIFE_HIT_SECS` into the
//! stab, [`time_stab`]), the knife's tip is run out ahead of us against
//! every enemy's model as drawn on this screen (`hit_detection::Models` —
//! the way our bullets are), and what it strikes goes up with the stab as
//! a claim ([`shared::PlayerInput::melee_hit`]) for the server to check
//! (`shared::melee::check_melee_claim`; failing that it resolves the stab
//! itself, as it always did). Then, at once, here ([`show_stab`]): the stab
//! or swing sound, the enemy's blood, the hit marker and damage number,
//! and — when the stab's sure to kill a zombie (its health against the
//! knife's damage, with our Pack-a-Punch and an Insta-Kill counted) — its
//! death fall and cry ([`PredictedDeath`]), and the kill's points and sound.
//!
//! The server's own word on each of those, arriving a moment later, is
//! then passed over ([`Predicted`]) so nothing shows twice; anything it
//! says that wasn't foreseen (a kill where only a hit was) still shows. If
//! it doesn't agree a zombie died ([`CONFIRM_SECS`] on), that zombie gets
//! back up. (The server doesn't send us our own stab's sound at all.)
//!
//! [`Stab`] and [`Predicted`] are cleared leaving a game; a
//! [`PredictedDeath`] lives on its zombie's avatar and goes with it.

use bevy::prelude::*;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use lightyear::prelude::{LocalId, PeerId};
use shared::melee::{KNIFE_HIT_SECS, KNIFE_REACH_M, KNIFE_TIP_DROP_M};
use shared::{GameMode, Lobby, PlayerHealth, PlayerPose, Projectile, ShotClaim};

use crate::audio_engine::prelude::*;
use crate::hit_detection::{may_hit, Kind, ModelHit, Models};
use crate::net::{GameClient, RemoteAvatar, STAB_ANIM_SECS};
use crate::{AppState, KnifeSounds, PendingMelee, PendingShot, WorldModelCamera};

/// How long (s) a foreseen hit waits for the server's word on it — and a
/// zombie we saw die, for the server to agree — before it's given up on.
pub(crate) const CONFIRM_SECS: f32 = 1.2;

/// How close (m) the knife's line can pass by a model and still strike
/// it — the blade's width, and a forearm's.
const STAB_SLACK_M: f32 = 0.12;

/// How close (m) the server's blood / damage number has to land to the
/// one we foresaw to be it.
const SAME_SPOT_M: f32 = 0.6;

pub(crate) struct MeleePredictPlugin;

impl Plugin for MeleePredictPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stab>()
            .init_resource::<Predicted>()
            .add_event::<StabLanded>()
            .add_systems(OnExit(AppState::InGame), forget)
            .add_systems(Update, (show_stab, expire).chain().run_if(in_state(AppState::InGame)));
    }
}

/// Our knife stab, from the moment it starts to the moment it goes up.
#[derive(Resource, Default)]
pub(crate) struct Stab {
    /// When (`Time` in the fixed step) it started, and whether it's gone
    /// to the server yet.
    started: Option<(f32, bool)>,
    /// This tick: the stab going up — the eye, the aim, and what it struck
    /// on our screen.
    pub(crate) send: Option<(Vec3, Vec3, Option<ShotClaim>)>,
    /// Whether the thrust is still playing (`PlayerInput::stabbing`).
    pub(crate) stabbing: bool,
}

/// The thrust reached out (from the eye at `eye`): what it struck, if
/// anything, for [`show_stab`] to show.
#[derive(Event)]
pub(crate) struct StabLanded {
    eye: Vec3,
    struck: Option<Struck>,
}

struct Struck {
    /// Who (a player entity's peer id — `None` for a `Freestyle` target).
    victim: Option<PeerId>,
    avatar: Entity,
    point: Vec3,
    /// The way the knife went in.
    dir: Vec3,
    kind: Kind,
    dog: bool,
    boss: bool,
    /// Whether it's sure to kill.
    kill: bool,
    /// A zombie's: the damage it'll show.
    damage: Option<u32>,
}

/// What we've already shown of our stabs, for the server's word on them to
/// be passed over.
#[derive(Resource, Default)]
pub(crate) struct Predicted(Vec<Foreseen>);

struct Foreseen {
    /// When (`Time`) it was shown.
    at: f32,
    victim: Option<PeerId>,
    point: Vec3,
    kill: bool,
    /// Still to be matched against the server's word.
    marker: bool,
    blood: bool,
    number: bool,
    /// The points shown for it.
    score: Option<u32>,
}

impl Predicted {
    /// Whether the server's hit marker for `victim` was foreseen (and so
    /// already flashed) — a kill one only if we saw the kill too.
    pub(crate) fn marker(&mut self, victim: Option<PeerId>, kill: bool) -> bool {
        self.0
            .iter_mut()
            .find(|f| f.marker && f.victim == victim && (f.kill || !kill))
            .map(|f| f.marker = false)
            .is_some()
    }

    /// Whether the server's blood at `point` was foreseen.
    pub(crate) fn blood(&mut self, point: Vec3) -> bool {
        self.0
            .iter_mut()
            .find(|f| f.blood && f.point.distance(point) < SAME_SPOT_M)
            .map(|f| f.blood = false)
            .is_some()
    }

    /// Whether the server's damage number at `point` was foreseen.
    pub(crate) fn number(&mut self, point: Vec3) -> bool {
        self.0
            .iter_mut()
            .find(|f| f.number && f.point.distance(point) < SAME_SPOT_M)
            .map(|f| f.number = false)
            .is_some()
    }

    /// Whether the server's `total` points were foreseen.
    pub(crate) fn score(&mut self, total: u32) -> bool {
        self.0
            .iter_mut()
            .find(|f| f.score == Some(total))
            .map(|f| f.score = None)
            .is_some()
    }
}

/// On a zombie's avatar we saw our knife kill, from when (`Time`): it falls
/// and lies there at once — held where it fell, whatever the server says
/// for its body — unless the server hasn't agreed by [`CONFIRM_SECS`].
#[derive(Component)]
pub(crate) struct PredictedDeath {
    since: f32,
}

/// Whether an avatar's alive: by the server, unless we saw it die.
pub(crate) fn alive(pose: &PlayerPose, predicted_death: bool) -> bool {
    pose.alive && !predicted_death
}

fn forget(mut stab: ResMut<Stab>, mut predicted: ResMut<Predicted>) {
    *stab = Stab::default();
    predicted.0.clear();
}

/// Time our stab: started when `weapon_system` files it, and when the
/// thrust gets there, tested against the enemies' models and sent
/// (`net::write_input`, which runs right after) — what it struck shown
/// at once ([`StabLanded`]). A stab never goes in a tick with a shot (they
/// share the aim fields); one begun long ago (a match left mid-stab) never
/// goes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn time_stab(
    time: Res<Time>,
    mut pending_melee: ResMut<PendingMelee>,
    pending_shot: Res<PendingShot>,
    cam: Query<&GlobalTransform, With<WorldModelCamera>>,
    rapier: ReadRapierContext,
    models: Models,
    (local, lobbies, healths): (Query<&LocalId, With<GameClient>>, Query<&Lobby>, Healths),
    mut stab: ResMut<Stab>,
    mut landed: EventWriter<StabLanded>,
) {
    stab.send = None;
    let now = time.elapsed_secs();
    if pending_melee.0.take().is_some() {
        stab.started = Some((now, false));
    }
    let mut send = None;
    if let Some((at, sent)) = stab.started.as_mut() {
        let since = now - *at;
        if !*sent && since >= KNIFE_HIT_SECS && pending_shot.0.is_none() {
            *sent = true;
            if let (true, Ok(cam)) = (since < KNIFE_HIT_SECS + 0.25, cam.single()) {
                let eye = cam.translation();
                let aim = cam.forward().as_vec3();
                let me = local.iter().next().map(|l| l.0);
                let lobby = lobbies.iter().find(|l| me.is_some_and(|me| l.has(me)));
                let hit = strike(eye, aim, &models, &rapier, lobby.map(|l| l.mode));
                send = Some((eye, aim, hit.as_ref().map(|h| h.claim)));
                let struck = hit.map(|h| foresee(h, eye, me, lobby, &healths));
                landed.write(StabLanded { eye, struck });
            }
        }
    }
    stab.send = send;
    stab.stabbing = stab.started.is_some_and(|(at, _)| now - at < STAB_ANIM_SECS);
}

/// What our knife's tip strikes running out ahead of the eye at `eye`
/// (facing `aim`): straight out at chest height (`shared::melee`'s line),
/// or failing that, angled down — a hellhound, or a zombie still climbing
/// out of the ground. Never anyone the mode doesn't let us hurt, nor
/// through a wall.
fn strike(eye: Vec3, aim: Vec3, models: &Models, rapier: &ReadRapierContext, mode: Option<GameMode>) -> Option<ModelHit> {
    let flat = Vec3::new(aim.x, 0.0, aim.z).try_normalize()?;
    let chest = eye - Vec3::Y * KNIFE_TIP_DROP_M;
    let lines = [
        [chest, chest + flat * KNIFE_REACH_M],
        [chest, chest + flat * (KNIFE_REACH_M * 0.85) - Vec3::Y * 1.0],
    ];
    let rapier = rapier.single().ok();
    lines.iter().find_map(|line| {
        let hit = models
            .hits_along_near(line, STAB_SLACK_M)
            .into_iter()
            .find(|h| may_hit(Projectile::Knife, mode, h.kind))?;
        let point = Vec3::from_array(hit.claim.point);
        let to = point - eye;
        let walled = rapier.as_ref().is_some_and(|r| {
            r.cast_ray(eye, to.normalize_or(Vec3::NEG_Z), to.length() - 0.05, true, QueryFilter::only_fixed())
                .is_some()
        });
        (!walled).then_some(hit)
    })
}

/// Every player entity's health (a zombie's included) — on its confirmed
/// entity, not the interpolated one its avatar follows.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Healths<'w, 's> {
    healths: Query<'w, 's, (&'static PlayerHealth, Option<&'static shared::PlayerId>)>,
    interp: Query<'w, 's, &'static lightyear::prelude::Interpolated>,
}

impl Healths<'_, '_> {
    /// `src`'s health (an avatar's source), or failing that `peer`'s.
    fn of(&self, src: Entity, peer: Option<PeerId>) -> Option<f32> {
        self.healths
            .get(src)
            .ok()
            .or_else(|| self.interp.get(src).ok().and_then(|i| self.healths.get(i.confirmed_entity).ok()))
            .or_else(|| {
                let peer = peer?;
                self.healths.iter().find(|(_, id)| id.is_some_and(|id| id.0 == peer))
            })
            .map(|(h, _)| h.0)
    }
}

/// What a stab that struck will do, as far as we can tell: a zombie dies
/// if the knife's damage (with our Pack-a-Punch on it) is at least what
/// health it has left, or an Insta-Kill's on; a `Freestyle` target always
/// does. (A player might have armour — that's left to the server.)
fn foresee(hit: ModelHit, eye: Vec3, me: Option<PeerId>, lobby: Option<&Lobby>, healths: &Healths) -> Struck {
    let point = Vec3::from_array(hit.claim.point);
    let victim = hit.victim;
    let (kill, damage) = match hit.kind {
        Kind::Zombie => {
            let health = healths.of(hit.src, hit.victim).unwrap_or(0.0);
            let pap = lobby
                .and_then(|l| l.members.iter().find(|m| Some(m.peer) == me))
                .map_or(1.0, |m| shared::pap::damage_mult(m.pap.get(shared::pap::PapWeapon::Knife)));
            let insta = lobby.is_some_and(|l| l.power_up_active(shared::power_ups::PowerUp::InstaKill));
            let knife = shared::zombies::ZOMBIE_KNIFE_DAMAGE * pap;
            let taken = if insta { health } else { knife.min(health) };
            (health > 0.0 && (insta || knife >= health), (health > 0.0).then(|| taken.round().max(1.0) as u32))
        }
        Kind::Bot => (true, None),
        Kind::Player => (false, None),
    };
    Struck {
        victim,
        avatar: hit.avatar,
        point,
        dir: (point - eye).normalize_or(Vec3::NEG_Z),
        kind: hit.kind,
        dog: hit.dog,
        boss: hit.boss,
        kill,
        damage,
    }
}

/// Show our stab landing: its sound, and on whoever it struck the blood,
/// hit marker, damage number — and if it kills a zombie, its fall and the
/// kill's points.
#[allow(clippy::too_many_arguments)]
fn show_stab(
    time: Res<Time>,
    mut landed: EventReader<StabLanded>,
    knife_sounds: Res<KnifeSounds>,
    blood_settings: Res<crate::vfx::EnemyBloodSettings>,
    (local, lobbies): (Query<&LocalId, With<GameClient>>, Query<&Lobby>),
    mut predicted: ResMut<Predicted>,
    mut out: (
        EventWriter<crate::vfx::BloodImpact>,
        EventWriter<crate::hit_marker::LocalHitMarker>,
        EventWriter<crate::hud::ShowDamageNumber>,
        EventWriter<crate::TrickScoredEvent>,
    ),
    mut commands: Commands,
    mut seq: Local<u32>,
) {
    let now = time.elapsed_secs();
    for stab in landed.read() {
        *seq = seq.wrapping_add(1);
        let variant = (seq.wrapping_mul(2_654_435_761) >> 24) as u8;
        // Its sound: a stab from where it went in, or a swing from us.
        let (set, at) = match &stab.struck {
            Some(s) => (&knife_sounds.stab, s.point),
            None => (&knife_sounds.swing, stab.eye),
        };
        if let Some(clip) = set.pick(variant) {
            commands.spawn((
                AudioPlayer::new(clip.handle.clone()),
                Transform::from_translation(at),
                crate::positional_playback(Volume::Linear(1.0)),
            ));
        }
        let Some(s) = &stab.struck else { continue };
        let zombie = s.kind == Kind::Zombie;

        if zombie {
            let enemy = if s.dog {
                &blood_settings.dog
            } else if s.boss {
                &blood_settings.boss
            } else {
                &blood_settings.zombie
            };
            out.0.write(crate::vfx::BloodImpact {
                point: s.point,
                dir: s.dir,
                style: Some(enemy.burst(s.kill)),
            });
        }
        out.1.write(crate::hit_marker::LocalHitMarker {
            kill: s.kill,
            victim: s.victim,
        });
        if let Some(damage) = s.damage {
            out.2.write(crate::hud::ShowDamageNumber {
                point: s.point,
                damage,
                critical: true,
            });
        }

        // A zombie's kill: its points (and the kill sound with them), the
        // same the server pays — a knife kill's a critical one.
        let mut score = None;
        if zombie && s.kill && !s.dog && !s.boss {
            let lobby = crate::zombies_hud::my_lobby(&local, &lobbies);
            let mult = if lobby.is_some_and(|l| l.power_up_active(shared::power_ups::PowerUp::DoublePoints)) {
                2
            } else {
                1
            };
            let lines = vec![
                ("KILL".to_string(), shared::ZOMBIE_KILL_POINTS * mult),
                ("CRITICAL".to_string(), shared::ZOMBIE_CRITICAL_POINTS * mult),
            ];
            let total = lines.iter().map(|(_, p)| p).sum();
            score = Some(total);
            out.3.write(crate::TrickScoredEvent {
                total,
                color: crate::hud::score_color(true, &lines),
                lines,
                sound: None,
            });
            commands.entity(s.avatar).try_insert(PredictedDeath { since: now });
        }

        predicted.0.push(Foreseen {
            at: now,
            victim: s.victim,
            point: s.point,
            kill: s.kill,
            marker: true,
            blood: zombie,
            number: s.damage.is_some(),
            score,
        });
    }
}

/// Give up on what the server never confirmed: forget foreseen hits after
/// [`CONFIRM_SECS`], and stand a zombie back up we saw die if it's still
/// alive by the server's word by then.
fn expire(
    time: Res<Time>,
    mut predicted: ResMut<Predicted>,
    deaths: Query<(Entity, &RemoteAvatar, &PredictedDeath)>,
    poses: Query<&PlayerPose>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    predicted.0.retain(|f| now - f.at < CONFIRM_SECS);
    for (entity, avatar, death) in &deaths {
        if now - death.since >= CONFIRM_SECS && poses.get(avatar.src).is_ok_and(|p| p.alive) {
            commands.entity(entity).remove::<PredictedDeath>();
        }
    }
}
