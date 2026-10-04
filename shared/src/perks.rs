//! `Zombies` perks: machines on the map a player walks up to and buys a perk
//! from with their points (Call of Duty zombies' perk-a-colas). The server
//! owns purchases (`server::zombies::on_buy_perk`); each member's owned perks
//! are replicated in `LobbyMember::perks`.
//!
//! There are two sets, and a lobby plays one ([`PerkSet`],
//! `Lobby::perk_set`): the game's own [`Perk::CUSTOM`] perks, or the
//! [`Perk::CLASSIC`] Call of Duty ones, which behave as they do in Black
//! Ops Cold War.

use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::MapId;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Perk {
    /// Shroom Tea — the shroom screen effect, x-ray and a little aim assist.
    ShroomTea,
    /// Nitro Brew — faster movement, ADS, reload, rechamber and weapon swap
    /// (the multipliers are client-side: `zombies_hud::NitroBrew`).
    NitroBrew,
    /// Liquid Courage — takes less damage from everything
    /// ([`LIQUID_COURAGE_DAMAGE_MULT`]), with a drunk screen effect.
    LiquidCourage,
    /// Bomb Shot — a trickshot kill (a 360 no-scope, [`is_trickshot`])
    /// explodes, killing zombies close by and hurting ones a little further
    /// out ([`bomb_shot_damage`]).
    BombShot,
    /// Kangabrew — jump much higher, and jump off walls mid-air (Black Ops 7
    /// style). Movement is client-authoritative, so it's all client-side:
    /// `client::zombies_hud::Kangabrew`.
    Kangabrew,

    // --- classic (Call of Duty, as in Cold War) ---
    /// Juggernog — more maximum health ([`JUGGERNOG_MAX_HEALTH`]).
    Juggernog,
    /// Quick Revive — health starts coming back sooner and faster
    /// ([`regen_delay`], [`regen_rate`]).
    QuickRevive,
    /// Speed Cola — faster reloads (client-side: `zombies_hud::NitroBrew`'s
    /// classic multipliers).
    SpeedCola,
    /// Stamin-Up — faster movement (client-side, likewise).
    StaminUp,
    /// Double Tap — a faster rate of fire (client-side, likewise).
    DoubleTap,
    /// Deadshot Daiquiri — aiming down sights pulls onto enemies (the
    /// client's aim assist, `player::shroom_aim_assist`).
    DeadshotDaiquiri,
    /// PhD Flopper (Der Wunderfizz only) — no fall damage, longer slides, and an explosion when
    /// sliding into an enemy ([`crate::PhdSlam`]) or landing a big fall
    /// ([`PHD_DROP_MIN_DISTANCE`]).
    PhdFlopper,
    /// Death Perception (Der Wunderfizz only) — enemies behind walls show as an outline
    /// (client-side: `vfx::shroom_xray`).
    DeathPerception,
}

/// Which perks a lobby's machines sell — set by the party leader before a
/// `Zombies` game ([`crate::SetPerkSet`]).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum PerkSet {
    /// Call of Duty's ([`Perk::CLASSIC`]), as in Cold War — the default.
    #[default]
    Classic,
    /// The game's own perks ([`Perk::CUSTOM`]).
    Custom,
}

impl PerkSet {
    /// In the order the lobby offers them, the default first.
    pub const ALL: [PerkSet; 2] = [PerkSet::Classic, PerkSet::Custom];

    /// Its perks, in the order their machines' spots are numbered.
    pub fn perks(self) -> &'static [Perk] {
        match self {
            PerkSet::Custom => &Perk::CUSTOM,
            PerkSet::Classic => &Perk::CLASSIC,
        }
    }

    /// The perks with a machine of their own on the map
    /// ([`Perk::has_machine`]).
    pub fn machine_perks(self) -> impl Iterator<Item = Perk> {
        self.perks().iter().copied().filter(|p| p.has_machine())
    }

    pub fn label(self) -> &'static str {
        match self {
            PerkSet::Custom => "CUSTOM",
            PerkSet::Classic => "CLASSIC",
        }
    }
}

/// Juggernog's maximum health (everyone else's is
/// [`crate::health::FULL_HEALTH`]).
pub const JUGGERNOG_MAX_HEALTH: f32 = 150.0;

/// Quick Revive: health starts coming back after this fraction of the usual
/// [`crate::health::REGEN_DELAY_SECS`]...
pub const QUICK_REVIVE_REGEN_DELAY_MULT: f32 = 0.5;
/// ...and comes back this many times as fast.
pub const QUICK_REVIVE_REGEN_RATE_MULT: f32 = 1.5;

/// PhD Flopper: a landing from at least this far up (m, apex → landing)
/// explodes.
pub const PHD_DROP_MIN_DISTANCE: f32 = 5.0;
/// PhD Flopper: a slide sets off its explosion when an enemy is within this
/// far (m) of the slider's feet — the server allows a little more for the
/// pose being a moment old ([`PHD_SLAM_SERVER_RADIUS`]).
pub const PHD_SLAM_RADIUS: f32 = 1.3;
pub const PHD_SLAM_SERVER_RADIUS: f32 = 3.0;
/// PhD Flopper: seconds before one player's next explosion can go off.
pub const PHD_COOLDOWN_SECS: f32 = 1.5;
/// PhD Flopper: a zombie its blast hurts but doesn't kill is stunned this
/// many seconds — it can't attack, and it moves at only this fraction of its
/// speed.
pub const PHD_STUN_SECS: f32 = 3.0;
pub const PHD_STUN_SPEED_MULT: f32 = 0.3;

/// `perks`' maximum health.
pub fn max_health(perks: &[Perk]) -> f32 {
    if perks.contains(&Perk::Juggernog) {
        JUGGERNOG_MAX_HEALTH
    } else {
        crate::health::FULL_HEALTH
    }
}

/// Seconds after a hit before `perks`' health starts coming back.
pub fn regen_delay(perks: &[Perk]) -> f32 {
    if perks.contains(&Perk::QuickRevive) {
        crate::health::REGEN_DELAY_SECS * QUICK_REVIVE_REGEN_DELAY_MULT
    } else {
        crate::health::REGEN_DELAY_SECS
    }
}

/// Health a second `perks` get back once it's coming back.
pub fn regen_rate(perks: &[Perk]) -> f32 {
    if perks.contains(&Perk::QuickRevive) {
        crate::health::REGEN_PER_SEC * QUICK_REVIVE_REGEN_RATE_MULT
    } else {
        crate::health::REGEN_PER_SEC
    }
}

/// Damage a Liquid Courage owner takes, as a fraction of the normal amount
/// (0.6 ≈ two thirds more health).
pub const LIQUID_COURAGE_DAMAGE_MULT: f32 = 0.6;

/// Bomb Shot's blast: everything within this many metres of the dead
/// zombie's feet dies — about the size of the explosion's fireball and dust
/// ring (the client's `ExplosionSettings` defaults)...
pub const BOMB_SHOT_KILL_RADIUS: f32 = 4.0;
/// ...and zombies out to this far still get hurt, less the further out.
pub const BOMB_SHOT_DAMAGE_RADIUS: f32 = 8.0;
/// Damage just outside the kill radius, fading linearly to...
pub const BOMB_SHOT_EDGE_DAMAGE_MAX: f32 = 65.0;
/// ...this at the damage radius (nothing beyond it).
pub const BOMB_SHOT_EDGE_DAMAGE_MIN: f32 = 15.0;
/// Enough to kill anything inside the kill radius outright.
pub const BOMB_SHOT_LETHAL_DAMAGE: f32 = 10_000.0;

/// Whether a shot fired with this trick metadata sets off Bomb Shot: a
/// 360 no-scope (a full turn, any amount airborne or not).
pub fn is_trickshot(spin_deg: f32, noscope: bool) -> bool {
    noscope && spin_deg >= 360.0
}

/// Bomb Shot damage to a zombie standing `distance` metres from the blast
/// (feet to feet): lethal inside [`BOMB_SHOT_KILL_RADIUS`], then fading from
/// [`BOMB_SHOT_EDGE_DAMAGE_MAX`] to [`BOMB_SHOT_EDGE_DAMAGE_MIN`] out to
/// [`BOMB_SHOT_DAMAGE_RADIUS`], and nothing past that.
pub fn bomb_shot_damage(distance: f32) -> f32 {
    if distance <= BOMB_SHOT_KILL_RADIUS {
        BOMB_SHOT_LETHAL_DAMAGE
    } else if distance <= BOMB_SHOT_DAMAGE_RADIUS {
        let t = (distance - BOMB_SHOT_KILL_RADIUS) / (BOMB_SHOT_DAMAGE_RADIUS - BOMB_SHOT_KILL_RADIUS);
        BOMB_SHOT_EDGE_DAMAGE_MAX + (BOMB_SHOT_EDGE_DAMAGE_MIN - BOMB_SHOT_EDGE_DAMAGE_MAX) * t
    } else {
        0.0
    }
}

/// Kangabrew halves fall damage (on top of any other protection).
pub const KANGABREW_FALL_DAMAGE_MULT: f32 = 0.5;

/// Fall `damage` scaled by what `perks` protect against — everything
/// [`damage_taken`] covers, plus Kangabrew's softer landings; none at all
/// with PhD Flopper.
pub fn fall_damage_taken(perks: &[Perk], damage: f32) -> f32 {
    let damage = damage_taken(perks, damage);
    if perks.contains(&Perk::PhdFlopper) {
        0.0
    } else if perks.contains(&Perk::Kangabrew) {
        damage * KANGABREW_FALL_DAMAGE_MULT
    } else {
        damage
    }
}

/// `damage` scaled by what `perks` protect against.
pub fn damage_taken(perks: &[Perk], damage: f32) -> f32 {
    if perks.contains(&Perk::LiquidCourage) {
        damage * LIQUID_COURAGE_DAMAGE_MULT
    } else {
        damage
    }
}

impl Perk {
    /// The game's own perks, in the order their machines' spots are
    /// numbered ([`Perk::slot`]).
    pub const CUSTOM: [Perk; 5] = [
        Perk::ShroomTea,
        Perk::NitroBrew,
        Perk::LiquidCourage,
        Perk::BombShot,
        Perk::Kangabrew,
    ];

    /// Call of Duty's, likewise. The first five stand where the custom
    /// set's do; the rest have spots of their own.
    pub const CLASSIC: [Perk; 8] = [
        Perk::Juggernog,
        Perk::QuickRevive,
        Perk::SpeedCola,
        Perk::StaminUp,
        Perk::DoubleTap,
        Perk::DeadshotDaiquiri,
        Perk::PhdFlopper,
        Perk::DeathPerception,
    ];

    /// Both sets.
    pub const ALL: [Perk; 13] = [
        Perk::ShroomTea,
        Perk::NitroBrew,
        Perk::LiquidCourage,
        Perk::BombShot,
        Perk::Kangabrew,
        Perk::Juggernog,
        Perk::QuickRevive,
        Perk::SpeedCola,
        Perk::StaminUp,
        Perk::DoubleTap,
        Perk::DeadshotDaiquiri,
        Perk::PhdFlopper,
        Perk::DeathPerception,
    ];

    /// The most perks one set has — what one player can own at once.
    pub const MAX_PER_SET: usize = 8;

    /// Whether it has a machine of its own — Death Perception and PhD
    /// Flopper are only sold by Der Wunderfizz ([`crate::wunderfizz`]).
    pub fn has_machine(self) -> bool {
        !matches!(self, Perk::DeathPerception | Perk::PhdFlopper)
    }

    /// Which set it's from.
    pub fn set(self) -> PerkSet {
        if Perk::CUSTOM.contains(&self) {
            PerkSet::Custom
        } else {
            PerkSet::Classic
        }
    }

    /// Its machine's spot number: its place in its set.
    pub fn slot(self) -> usize {
        self.set().perks().iter().position(|&p| p == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Perk::ShroomTea => "Shroom Tea",
            Perk::NitroBrew => "Nitro Brew",
            Perk::LiquidCourage => "Liquid Courage",
            Perk::BombShot => "Bomb Shot",
            Perk::Kangabrew => "Kangabrew",
            Perk::Juggernog => "Juggernog",
            Perk::QuickRevive => "Quick Revive",
            Perk::SpeedCola => "Speed Cola",
            Perk::StaminUp => "Stamin-Up",
            Perk::DoubleTap => "Double Tap",
            Perk::DeadshotDaiquiri => "Deadshot Daiquiri",
            Perk::PhdFlopper => "PhD Flopper",
            Perk::DeathPerception => "Death Perception",
        }
    }

    /// One-line blurb for the machine's card (CoD style).
    pub fn description(self) -> &'static str {
        match self {
            Perk::ShroomTea => "See enemies through walls and gain aim assist.",
            Perk::NitroBrew => "Move, aim, reload, rechamber and swap weapons faster.",
            Perk::LiquidCourage => "Take less damage from everything.",
            Perk::BombShot => "360 no-scope kills explode, blowing up nearby zombies.",
            Perk::Kangabrew => "Jump three times as high, jump again off walls, and take half fall damage.",
            Perk::Juggernog => "Increases maximum health.",
            Perk::QuickRevive => "Revive teammates faster, or yourself when playing solo. Health regenerates sooner and faster.",
            Perk::SpeedCola => "Reload faster.",
            Perk::StaminUp => "Move faster.",
            Perk::DoubleTap => "Increases rate of fire.",
            Perk::DeadshotDaiquiri => "Aiming down sights snaps to enemies.",
            Perk::PhdFlopper => {
                "Immune to fall damage. Slide further, and sliding into enemies or landing from a height causes an explosion."
            }
            Perk::DeathPerception => "See enemies through walls.",
        }
    }

    /// What's in it, for the machine's card (`client/notes/perk-ingredients.md`)
    /// — the custom perks' own lore; the classic ones have none.
    pub fn ingredients(self) -> &'static [&'static str] {
        match self {
            Perk::ShroomTea => &[
                "Heroic dose of psilocybin mushrooms",
                "Fresh-squeezed lemon juice",
                "Bioluminescent foxfire fungus",
                "Lion's mane extract",
                "Cordyceps, harvested off a zombie",
                "Carrot concentrate and bilberry",
                "Owl eyeball garnish",
                "Ground-up x-ray film",
                "Third eye drops",
            ],
            Perk::NitroBrew => &[
                "Caffeine anhydrous",
                "Distilled rocket fuel",
                "Methamphetamine",
                "Dash of preworkout",
                "Crushed adderall",
                "Splash of nitroglycerin",
                "Twelve energy drinks, boiled down",
                "Freeze-dried hummingbird heartbeats",
                "Squirt of WD-40",
                "Cheetah sweat",
            ],
            Perk::LiquidCourage => &[
                "A whole handle of cheap vodka",
                "Smelling salts, to stay upright",
                "Crushed ibuprofen",
                "Bull's blood, freshly squeezed",
                "Rhino hide shavings",
                "Grandpa's war stories",
                "A splash of cough syrup",
                "Maraschino cherry juice",
                "Pickle brine for the morning after",
            ],
            Perk::BombShot => &[
                "A fistful of black powder",
                "Orange soda, violently shaken",
                "Ghost pepper extract",
                "Shaved fireworks, the illegal kind",
                "Leftover nitroglycerin from Nitro Brew",
                "A lit fuse, still burning",
                "Dizziness, bottled mid-spin",
                "Tears of a camping sniper",
            ],
            Perk::Kangabrew => &[
                "Kangaroo tail soup",
                "Pogo stick springs, finely ground",
                "Moon gravity, bottled on the dark side",
                "Eucalyptus leaves",
                "Trampoline sweat",
                "Frog legs, still twitching",
                "A pinch of helium",
                "Crushed parkour YouTube thumbnails",
            ],
            _ => &[],
        }
    }

    /// Points it costs. A zombie kill is worth 100 and round `n` sends
    /// `4 + 2(n-1)` (solo), so one player has about 400 points after round
    /// 1, 2,800 after round 4, 4,000 after round 5 and 10,800 after round 9:
    /// the cheaper perks come around rounds 3–4, Liquid Courage (the
    /// strongest, like Juggernog) around round 5, and the whole set plus
    /// the power by about round 9. The classic perks cost what they do in
    /// Cold War.
    pub fn cost(self) -> u32 {
        match self {
            // X-ray + a little aim assist: handy, not life-saving.
            Perk::ShroomTea => 1500,
            // Mobility — escape routes and high ground.
            Perk::Kangabrew => 1500,
            // Everything faster: movement, ADS, reload, rechamber, swap.
            Perk::NitroBrew => 2000,
            // Crowd clearing, but only off a 360 no-scope.
            Perk::BombShot => 2000,
            // Survive more hits — the one everyone wants first.
            Perk::LiquidCourage => 2500,
            Perk::Juggernog => 2500,
            Perk::QuickRevive => 500,
            Perk::SpeedCola => 3000,
            Perk::StaminUp => 2000,
            Perk::DoubleTap => 2000,
            Perk::DeadshotDaiquiri => 1500,
            Perk::PhdFlopper => 2500,
            Perk::DeathPerception => 3000,
        }
    }

    /// Where the perk's machine stands on `map` — the centre of the ground
    /// under it (feet level, like `Bot::pos`). The buy range, the machine's
    /// collision box, its model, light and jingle all go from here. Each set
    /// numbers its machines' spots the same way ([`Perk::slot`]), so the
    /// classic set's first five stand where the custom set's do. Maps
    /// without spots yet use somewhere near the origin.
    pub fn machine_pos(self, map: MapId) -> Vec3 {
        slot_pos(self.slot(), map)
    }

    /// Which way the perk's machine faces on `map`: its turn around the
    /// vertical axis (degrees).
    pub fn machine_yaw_deg(self, map: MapId) -> f32 {
        slot_yaw_deg(self.slot(), map)
    }

    /// The machine's solid box on `map` (only there in `Zombies`):
    /// `(centre, rotation, half extents)`.
    pub fn machine_box(self, map: MapId) -> (Vec3, Quat, Vec3) {
        (
            self.machine_pos(map) + Vec3::Y * MACHINE_HALF_EXTENTS.y,
            Quat::from_rotation_y(self.machine_yaw_deg(map).to_radians()),
            MACHINE_HALF_EXTENTS,
        )
    }
}

/// Machine spots past the custom set's five stand this far (m) to the side
/// of one of those, along the wall it's backed against.
const EXTRA_SLOT_SPACING: f32 = 3.2;

/// Where machine spot `slot` is on `map` (see [`Perk::machine_pos`]).
fn slot_pos(slot: usize, map: MapId) -> Vec3 {
    match map {
        // Ashes of the Damned: placeholders in a row across the ground-level
        // platform until they're placed properly.
        MapId::AshesOfTheDamned => {
            const X: [f32; 8] = [-12.0, -6.0, 0.0, 6.0, 12.0, 18.0, -18.0, 24.0];
            Vec3::new(X[slot % X.len()], 0.0, -22.0)
        }
        MapId::BreakPoint | MapId::BreakPointNight => {
            // Tuned in the client's debug panel ("Machine placement").
            const SPOTS: [Vec3; 5] = [
                Vec3::new(-2.76, 4.8, -57.43),
                Vec3::new(-35.62, 6.0, 13.54),
                Vec3::new(28.92, 6.0, 59.62),
                // Ground floor, under the upper walkway.
                Vec3::new(-30.7, 0.0, -2.03),
                Vec3::new(-35.6, 15.6, -20.0),
            ];
            if slot < SPOTS.len() {
                SPOTS[slot]
            } else {
                // Beside one of the five, the same way round.
                let beside = (slot - SPOTS.len()) % SPOTS.len();
                let side = Quat::from_rotation_y(slot_yaw_deg(beside, map).to_radians()) * Vec3::X;
                SPOTS[beside] + side * EXTRA_SLOT_SPACING
            }
        }
        _ => {
            const SPOTS: [Vec3; 8] = [
                Vec3::ZERO,
                Vec3::new(6.0, 0.0, 0.0),
                Vec3::new(-6.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 6.0),
                Vec3::new(0.0, 0.0, -6.0),
                Vec3::new(6.0, 0.0, 6.0),
                Vec3::new(-6.0, 0.0, 6.0),
                Vec3::new(6.0, 0.0, -6.0),
            ];
            SPOTS[slot % SPOTS.len()]
        }
    }
}

/// Which way machine spot `slot` faces on `map` (degrees).
fn slot_yaw_deg(slot: usize, map: MapId) -> f32 {
    match map {
        MapId::BreakPoint | MapId::BreakPointNight => {
            const YAW: [f32; 5] = [-90.0, 90.0, 180.0, 0.0, 90.0];
            YAW[slot % YAW.len()]
        }
        _ => 0.0,
    }
}

/// Every perk machine is the same box (m): half its width, height and depth
/// before it's turned. The models (`client/assets/models/props/perk_machines/custom/*_perk_machine.glb`)
/// are this box at the client's machine scale.
pub const MACHINE_HALF_EXTENTS: Vec3 = Vec3::new(0.75, 1.2, 0.36);

/// Points a `Zombies` player gets for going prone at a perk machine — once
/// per machine per game, to whoever does it first (Call of Duty's hidden
/// freebie).
pub const PRONE_BONUS_POINTS: u32 = 100;

/// How close (m, horizontally) a player's feet must be to a machine to buy
/// from it...
pub const PERK_USE_RADIUS: f32 = 2.0;
/// ...and within this much height of its base (not a floor above / below).
pub const PERK_USE_HEIGHT: f32 = 1.5;

/// Whether feet position `feet` is close enough to `perk`'s machine on `map`
/// to buy it. `slack` widens the radius (the server's check allows for the
/// client's pose being a moment old).
pub fn in_range(perk: Perk, map: MapId, feet: Vec3, slack: f32) -> bool {
    in_range_of(perk.machine_pos(map), feet, slack)
}

/// [`in_range`] for a machine standing at `machine` (the client's debug
/// panel can move one away from its [`Perk::machine_pos`]).
pub fn in_range_of(machine: Vec3, feet: Vec3, slack: f32) -> bool {
    Vec3::new(feet.x - machine.x, 0.0, feet.z - machine.z).length() <= PERK_USE_RADIUS + slack
        && (feet.y - machine.y).abs() <= PERK_USE_HEIGHT + slack
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_at_the_logged_spot_is_in_range_and_a_few_metres_away_is_not() {
        let m = Perk::ShroomTea.machine_pos(MapId::BreakPoint);
        assert!(in_range(Perk::ShroomTea, MapId::BreakPoint, m, 0.0));
        assert!(in_range(Perk::ShroomTea, MapId::BreakPoint, m + Vec3::X * 1.5, 0.0));
        assert!(!in_range(Perk::ShroomTea, MapId::BreakPoint, m + Vec3::X * 3.0, 0.0));
        // A floor below doesn't count.
        assert!(!in_range(Perk::ShroomTea, MapId::BreakPoint, m - Vec3::Y * 4.0, 0.0));
    }

    #[test]
    fn bomb_shot_kills_close_hurts_further_out_and_spares_the_rest() {
        assert!(bomb_shot_damage(0.0) >= crate::health::FULL_HEALTH);
        assert!(bomb_shot_damage(BOMB_SHOT_KILL_RADIUS) >= crate::health::FULL_HEALTH);
        let just_out = bomb_shot_damage(BOMB_SHOT_KILL_RADIUS + 0.1);
        let further = bomb_shot_damage(BOMB_SHOT_DAMAGE_RADIUS - 0.1);
        assert!(just_out < crate::health::FULL_HEALTH && just_out > further && further > 0.0);
        assert_eq!(bomb_shot_damage(BOMB_SHOT_DAMAGE_RADIUS + 0.1), 0.0);
    }

    #[test]
    fn only_a_360_no_scope_is_a_trickshot() {
        assert!(is_trickshot(360.0, true));
        assert!(is_trickshot(720.0, true));
        assert!(!is_trickshot(359.0, true));
        assert!(!is_trickshot(720.0, false));
    }

    #[test]
    fn only_liquid_courage_cuts_damage() {
        assert_eq!(damage_taken(&[], 50.0), 50.0);
        assert_eq!(damage_taken(&[Perk::ShroomTea, Perk::NitroBrew], 50.0), 50.0);
        assert_eq!(damage_taken(&[Perk::LiquidCourage], 50.0), 50.0 * LIQUID_COURAGE_DAMAGE_MULT);
    }

    #[test]
    fn no_two_machines_can_be_bought_from_the_same_spot() {
        for map in [
            MapId::BasicMap,
            MapId::Shipment,
            MapId::ShipmentDay,
            MapId::BreakPoint,
            MapId::BreakPointNight,
            MapId::AshesOfTheDamned,
        ] {
            for set in PerkSet::ALL {
                for a in set.machine_perks() {
                    for b in set.machine_perks() {
                        if a != b {
                            assert!(!in_range(b, map, a.machine_pos(map), 0.75), "{a:?}/{b:?} on {map:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn both_sets_share_the_first_five_spots() {
        for (c, k) in Perk::CUSTOM.iter().zip(Perk::CLASSIC.iter()) {
            assert_eq!(c.machine_pos(MapId::BreakPoint), k.machine_pos(MapId::BreakPoint));
        }
        assert_eq!(Perk::DeathPerception.slot(), 7);
        assert_eq!(Perk::DeathPerception.set(), PerkSet::Classic);
    }

    #[test]
    fn classic_health_perks() {
        assert_eq!(max_health(&[]), crate::health::FULL_HEALTH);
        assert_eq!(max_health(&[Perk::Juggernog]), JUGGERNOG_MAX_HEALTH);
        assert!(regen_delay(&[Perk::QuickRevive]) < regen_delay(&[]));
        assert!(regen_rate(&[Perk::QuickRevive]) > regen_rate(&[]));
        assert_eq!(fall_damage_taken(&[Perk::PhdFlopper], 80.0), 0.0);
    }
}
