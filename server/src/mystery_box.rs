//! The `Zombies` Mystery Box (`shared::mystery_box`): a member pays
//! ([`SpinMysteryBox`]) and the box rolls its prize for them — never what
//! they already carry — then runs the spin on its clock: spinning, then the
//! prize on offer to them alone ([`TakeBoxPrize`]), then closing (taken, or
//! sinking back in) until the lid's shut and it's free again. Each phase
//! lands in the lobby's replicated [`Lobby::mystery_box`], which every
//! client draws from.
//!
//! A spin can land on the teddy bear instead
//! ([`shared::mystery_box::teddy_chance`], growing with the spins at that
//! spot): it's handed back, and the box leaves, is away a moment, and
//! arrives on another of the map's spots ([`Lobby::box_spot`]). The debug
//! panel's [`SpinTeddy`] does that on demand.
//!
//! The spin and the box's spot are reset with the rest of the lobby's game
//! state whenever a game starts or ends (`lobby`), and the clocks of
//! lobbies with no spin are dropped ([`run_spins`]) — nothing carries over.

use std::collections::HashMap;

use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use shared::mystery_box::{
    BoxPhase, BoxPrize, MysteryBoxSpin, ARRIVE_SECS, AWAY_SECS, CLOSE_SECS, COST, LEAVE_SECS, OFFER_SECS, SPIN_SECS,
};
use shared::weapon::SlotWeapon;
use shared::{BoxPrizeTaken, GameChannel, GameMode, Lobby, PlayerId, PlayerPose, SpinMysteryBox, SpinTeddy, TakeBoxPrize};

use crate::collision::MapColliders;
use crate::killcam::EndingLobbies;
use crate::pvp::PlayerCombat;
use crate::wall_buys::{standing_in_game, swap_into_slot};

pub struct MysteryBoxPlugin;

impl Plugin for MysteryBoxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpinClocks>()
            .add_observer(on_spin)
            .add_observer(on_take_prize)
            .add_observer(on_spin_teddy)
            .add_systems(FixedUpdate, run_spins);
    }
}

/// Each lobby's spin: seconds into its phase, and how many spins it's had
/// (each one's id).
#[derive(Resource, Default)]
struct SpinClocks(HashMap<Entity, (f32, u32)>);

/// A member pays to spin the box: in a running `Zombies` game, up, at the
/// box, with it idle and the points. Anything else is ignored.
#[allow(clippy::too_many_arguments)]
fn on_spin(
    trigger: Trigger<RemoteTrigger<SpinMysteryBox>>,
    time: Res<Time>,
    endings: Res<EndingLobbies>,
    mut clocks: ResMut<SpinClocks>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
) {
    let peer = trigger.from;
    let SpinMysteryBox { knives_full, molotovs_full, monkeys_full, frags_full, flashes_full } = trigger.trigger;
    let Some((lobby_e, feet, _)) = standing_in_game(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) else {
        return;
    };
    if lobby.mystery_box.is_some() || !shared::mystery_box::in_range(&lobby, feet, 0.75) {
        return;
    }
    let Some(member) = lobby.members.iter().find(|m| m.peer == peer) else {
        return;
    };
    if member.score < COST {
        return;
    }
    let carried = member.weapons;
    let excluded = |p: BoxPrize| match p {
        BoxPrize::ThrowingKnife => knives_full,
        BoxPrize::Molotov => molotovs_full,
        BoxPrize::MonkeyBomb => monkeys_full,
        BoxPrize::Frag => frags_full,
        BoxPrize::FlashBang => flashes_full,
        gun => gun.gun().is_some_and(|g| carried.contains(&SlotWeapon::Gun(g))),
    };
    let seed = time.elapsed().as_nanos() as u64 ^ peer.to_bits().wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let Some(prize) = shared::mystery_box::roll(lobby.round, excluded, shared::bots::rand01(seed)) else {
        return;
    };
    let spots = shared::mystery_box::spots(lobby.map).len();
    let chance = shared::mystery_box::teddy_chance(lobby.box_spot.spins, spots);
    let teddy = shared::bots::rand01(seed.rotate_left(17) ^ 0x7ed5_5d1e) < chance;
    if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == peer) {
        m.score -= COST;
    }
    start_spin(&mut clocks, lobby_e, &mut lobby, peer, prize, teddy, COST);
    if teddy {
        info!("{peer:?} spun the Mystery Box on round {}: the teddy bear — it's moving", lobby.round);
    } else {
        info!("{peer:?} spun the Mystery Box on round {}: {}", lobby.round, prize.label());
    }
}

/// Start a spin of `lobby`'s box for `user`, landing on `prize` (or the
/// bear), `paid` points paid for it — one more spin at the box's spot.
#[allow(clippy::too_many_arguments)]
fn start_spin(
    clocks: &mut SpinClocks,
    lobby_e: Entity,
    lobby: &mut Lobby,
    user: PeerId,
    prize: BoxPrize,
    teddy: bool,
    paid: u32,
) {
    let clock = clocks.0.entry(lobby_e).or_insert((0.0, 0));
    clock.0 = 0.0;
    clock.1 = clock.1.wrapping_add(1);
    let id = clock.1;
    lobby.box_spot.spins = lobby.box_spot.spins.saturating_add(1);
    lobby.mystery_box = Some(MysteryBoxSpin {
        id,
        user,
        prize,
        phase: BoxPhase::Spinning,
        taken: false,
        teddy,
        paid,
    });
}

/// Debug: the party leader spins the box for free, from anywhere, and it
/// lands on the teddy bear — in a running, unpaused `Zombies` game on a map
/// with a box, with the box idle.
fn on_spin_teddy(
    trigger: Trigger<RemoteTrigger<SpinTeddy>>,
    endings: Res<EndingLobbies>,
    mut clocks: ResMut<SpinClocks>,
    mut lobbies: Query<(Entity, &mut Lobby)>,
) {
    let peer = trigger.from;
    let Some((lobby_e, mut lobby)) = lobbies
        .iter_mut()
        .find(|(_, l)| l.leader == peer && l.started && l.mode == GameMode::Zombies && !l.paused)
    else {
        return;
    };
    if endings.is_ending(lobby_e) || lobby.mystery_box.is_some() || shared::mystery_box::current(&lobby).is_none() {
        return;
    }
    start_spin(&mut clocks, lobby_e, &mut lobby, peer, BoxPrize::RayGun, true, 0);
    info!("lobby {lobby_e:?}: {peer:?} spun the Mystery Box onto the teddy bear (debug)");
}

/// The member who spun takes the prize on offer, at the box: a gun goes in
/// the slot in their hands (the weapon there's dropped), a lethal or
/// tactical is one more of it (the other kind of its sort they carry is
/// dropped around them).
#[allow(clippy::too_many_arguments)]
fn on_take_prize(
    trigger: Trigger<RemoteTrigger<TakeBoxPrize>>,
    endings: Res<EndingLobbies>,
    colliders: Res<MapColliders>,
    mut clocks: ResMut<SpinClocks>,
    server: Single<&Server>,
    mut sender: ServerMultiMessageSender,
    mut lobbies: Query<(Entity, &mut Lobby)>,
    players: Query<(&PlayerId, &PlayerPose, &PlayerCombat)>,
    mut commands: Commands,
    mut quotes: EventWriter<crate::quotes::SayQuote>,
) {
    let peer = trigger.from;
    let TakeBoxPrize { slot, mag, reserve, dropping } = trigger.trigger;
    let Some((lobby_e, feet, _)) = standing_in_game(peer, &endings, &lobbies, &players) else {
        return;
    };
    let Ok((_, mut lobby)) = lobbies.get_mut(lobby_e) else {
        return;
    };
    let Some(spin) = lobby.mystery_box else { return };
    if spin.user != peer
        || spin.phase != BoxPhase::Offering
        || !shared::mystery_box::in_range(&lobby, feet, 0.75)
    {
        return;
    }
    match spin.prize.gun() {
        Some(gun) => {
            let taken = SlotWeapon::Gun(gun);
            if swap_into_slot(&mut commands, &colliders, lobby_e, &mut lobby, peer, slot, taken, 0, (mag, reserve), feet)
                .is_none()
            {
                return;
            }
        }
        None => {
            if let Some(kind) = spin.prize.lethal() {
                let world = colliders.for_lobby(&lobby);
                crate::lethals::drop_carried(&mut commands, lobby_e, &lobby, peer, feet, dropping, kind, &world);
            }
        }
    }
    lobby.mystery_box = Some(MysteryBoxSpin {
        phase: BoxPhase::Closing,
        taken: true,
        ..spin
    });
    if let Some(clock) = clocks.0.get_mut(&lobby_e) {
        clock.0 = 0.0;
    }
    let msg = BoxPrizeTaken { prize: spin.prize, slot };
    if let Err(e) = sender.send::<_, GameChannel>(&msg, server.into_inner(), &NetworkTarget::Single(peer)) {
        error!("failed to send a Mystery Box prize to {peer:?}: {e:?}");
    }
    // What they took, said every time (nothing for the sniper — no lines).
    let quote = match spin.prize {
        shared::mystery_box::BoxPrize::Ak74 => Some(shared::quotes::Quote::MysteryAk74),
        shared::mystery_box::BoxPrize::RayGun => Some(shared::quotes::Quote::MysteryRayGun),
        shared::mystery_box::BoxPrize::ThrowingKnife => Some(shared::quotes::Quote::MysteryThrowingKnife),
        shared::mystery_box::BoxPrize::Molotov => Some(shared::quotes::Quote::MysteryMolotov),
        shared::mystery_box::BoxPrize::Frag => Some(shared::quotes::Quote::MysteryFrag),
        shared::mystery_box::BoxPrize::FlashBang => Some(shared::quotes::Quote::MysteryFlashBang),
        shared::mystery_box::BoxPrize::MonkeyBomb => Some(shared::quotes::Quote::MysteryMonkeyBomb),
        shared::mystery_box::BoxPrize::M17 => Some(shared::quotes::Quote::MysteryM17),
        shared::mystery_box::BoxPrize::Sniper => None,
    };
    if let Some(quote) = quote {
        quotes.write(crate::quotes::SayQuote::by(lobby_e, peer, quote));
    }
    info!("{peer:?} took the {} from the Mystery Box", spin.prize.label());
}

/// Run each lobby's spin on its clock (held while the game's paused): its
/// prize comes up for offer, then goes back in, and the box is free once
/// the lid's shut — or, landed on the teddy bear, the spin's handed back
/// and the box leaves, is away, and arrives on another spot, free once
/// it's down. A spin outside a
/// running `Zombies` game is dropped.
fn run_spins(time: Res<Time>, mut clocks: ResMut<SpinClocks>, mut lobbies: Query<(Entity, &mut Lobby)>) {
    let dt = time.delta_secs();
    clocks.0.retain(|e, _| lobbies.contains(*e));
    for (lobby_e, mut lobby) in &mut lobbies {
        let Some(spin) = lobby.mystery_box else {
            // (Keep the spin count, so ids don't repeat.)
            if let Some(clock) = clocks.0.get_mut(&lobby_e) {
                clock.0 = 0.0;
            }
            continue;
        };
        if !lobby.started || lobby.mode != GameMode::Zombies {
            lobby.mystery_box = None;
            continue;
        }
        if lobby.paused {
            continue;
        }
        let clock = clocks.0.entry(lobby_e).or_insert((0.0, spin.id));
        clock.0 += dt;
        let next = match spin.phase {
            BoxPhase::Spinning if clock.0 >= SPIN_SECS && spin.teddy => Some(Some(BoxPhase::Leaving)),
            BoxPhase::Spinning if clock.0 >= SPIN_SECS => Some(Some(BoxPhase::Offering)),
            BoxPhase::Offering if clock.0 >= OFFER_SECS => Some(Some(BoxPhase::Closing)),
            BoxPhase::Closing if clock.0 >= CLOSE_SECS => Some(None),
            BoxPhase::Leaving if clock.0 >= LEAVE_SECS => Some(Some(BoxPhase::Away)),
            BoxPhase::Away if clock.0 >= AWAY_SECS => Some(Some(BoxPhase::Arriving)),
            BoxPhase::Arriving if clock.0 >= ARRIVE_SECS => Some(None),
            _ => None,
        };
        let Some(next) = next else { continue };
        clock.0 = 0.0;
        match next {
            // The bear: the spin's handed back.
            Some(BoxPhase::Leaving) => {
                if let Some(m) = lobby.members.iter_mut().find(|m| m.peer == spin.user) {
                    m.score += spin.paid;
                }
            }
            // Down on another spot.
            Some(BoxPhase::Arriving) => {
                let spots = shared::mystery_box::spots(lobby.map).len();
                let seed = ((spin.id as u64) << 32) ^ lobby_e.to_bits().wrapping_mul(0x9e37_79b9_7f4a_7c15);
                let to = shared::mystery_box::next_spot(lobby.box_spot.at, spots, shared::bots::rand01(seed));
                lobby.box_spot.at = to;
                lobby.box_spot.spins = 0;
                info!("lobby {lobby_e:?}: the Mystery Box moved to spot {}", to + 1);
            }
            _ => {}
        }
        lobby.mystery_box = next.map(|phase| MysteryBoxSpin { phase, ..spin });
    }
}
