//! `Zombies` operator quotes: what a player's operator says when something
//! happens — killing a boss, buying ammo, the power coming on, a power-up...
//! (Perk quotes are their own thing, `client::zombies_hud`.)
//!
//! The server decides who says what, and when (`server::quotes`): it picks
//! the line, so every client plays the same one ([`crate::QuoteSaid`]), and
//! — knowing how long each runs, from the list of lines here — keeps a
//! speaker from talking over themselves. The lines live under
//! `client/assets/audio/quotes/<operator>/<`[`Quote::dir`]`>/`, listed with
//! their lengths in `lines.txt` there (embedded here at build time).

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::operator::Operator;
use crate::power_ups::PowerUp;

/// Something an operator has lines for.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quote {
    /// They killed a boss.
    BossKill,
    /// The second boss of the game turns up (someone in the lobby says it)...
    BossRepeat,
    /// ...and the very first.
    FirstBoss,
    BuyAk74,
    /// Ammo from the ammo crate.
    BuyAmmo,
    BuyArmor,
    DogKill,
    /// A dog round's begun (someone in the lobby says it).
    DogRound,
    /// Any kill with nothing more particular to say about it.
    EnemyKill,
    /// The exfil's been called (someone in the lobby says it).
    Exfil,
    FragKill,
    /// Down to their last magazine...
    LowAmmo,
    /// ...and out altogether.
    OutOfAmmo,
    /// A knife stab kill.
    MeleeKill,
    MolotovKill,
    /// Their monkey bomb starts singing, and goes off.
    MonkeyBombActivate,
    MonkeyBombExplode,
    /// What they took from the Mystery Box.
    MysteryAk74,
    MysteryFlashBang,
    MysteryFrag,
    MysteryMolotov,
    MysteryMonkeyBomb,
    MysteryRayGun,
    MysteryThrowingKnife,
    /// They Pack-a-Punched a weapon.
    Pap,
    /// A PhD Flopper blast kill.
    PhdKill,
    /// They picked up equipment lying about.
    PickupEquipment,
    /// They turned the power on.
    PowerOn,
    /// A power-up went off (someone in the lobby says it).
    PowerUp(PowerUp),
    /// Their flash bang stunned zombies.
    StunEnemy,
    TakeDamage,
    /// Burnt by their own molotov.
    TakeMolotovDamage,
    ThrowingKnifeKill,
}

/// How often a [`Quote`] is said: the chance each time it could be, and
/// the least time (s) before the same speaker says it again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    pub chance: f32,
    pub cooldown: f32,
}

impl Quote {
    /// Its folder under `audio/quotes/<operator>/`.
    pub fn dir(self) -> &'static str {
        match self {
            Quote::BossKill => "boss_kill",
            Quote::BossRepeat => "boss_repeat",
            Quote::FirstBoss => "first_boss",
            Quote::BuyAk74 => "buy_ak74",
            Quote::BuyAmmo => "buy_ammo",
            Quote::BuyArmor => "buy_armor",
            Quote::DogKill => "dog_kill",
            Quote::DogRound => "dog_round",
            Quote::EnemyKill => "enemy_kill",
            Quote::Exfil => "exfil",
            Quote::FragKill => "frag_kill",
            Quote::LowAmmo => "low_ammo",
            Quote::OutOfAmmo => "out_of_ammo",
            Quote::MeleeKill => "melee_kill",
            Quote::MolotovKill => "molotov_kill",
            Quote::MonkeyBombActivate => "monkey_bomb/activate",
            Quote::MonkeyBombExplode => "monkey_bomb/explode",
            Quote::MysteryAk74 => "mystery_box/ak74",
            Quote::MysteryFlashBang => "mystery_box/flash_bang",
            Quote::MysteryFrag => "mystery_box/frag",
            Quote::MysteryMolotov => "mystery_box/molotov",
            Quote::MysteryMonkeyBomb => "mystery_box/monkey_bomb",
            Quote::MysteryRayGun => "mystery_box/raygun",
            Quote::MysteryThrowingKnife => "mystery_box/throwing_knife",
            Quote::Pap => "pap",
            Quote::PhdKill => "phd_kill",
            Quote::PickupEquipment => "pickup_new_dropped_equipment",
            Quote::PowerOn => "power_on",
            Quote::PowerUp(PowerUp::MaxAmmo) => "power_ups/max_ammo",
            Quote::PowerUp(PowerUp::InstaKill) => "power_ups/insta_kill",
            Quote::PowerUp(PowerUp::DoublePoints) => "power_ups/double_points",
            Quote::PowerUp(PowerUp::Nuke) => "power_ups/nuke",
            Quote::PowerUp(PowerUp::BonusPoints) => "power_ups/bonus_points",
            Quote::StunEnemy => "stun_enemy",
            Quote::TakeDamage => "take_damage",
            Quote::TakeMolotovDamage => "take_damage/take_molotov_damage",
            Quote::ThrowingKnifeKill => "throwing_knife_kill",
        }
    }

    /// How often it's said. The everyday ones (any kill, a knife kill,
    /// taking a hit) only now and then; the rarer moments (a molotov or
    /// monkey bomb kill, a frag) nearly every time, but not again for a
    /// while; the big ones (a boss, the power, a power-up, the box, Pack-a-
    /// Punch) every time.
    pub fn rule(self) -> Rule {
        let (chance, cooldown) = match self {
            Quote::EnemyKill => (0.08, 15.0),
            Quote::MeleeKill => (0.2, 15.0),
            Quote::DogKill => (0.3, 12.0),
            Quote::TakeDamage => (0.3, 12.0),
            Quote::ThrowingKnifeKill => (0.5, 12.0),
            Quote::PickupEquipment => (0.5, 20.0),
            Quote::DogRound => (0.7, 0.0),
            Quote::StunEnemy => (0.75, 12.0),
            Quote::TakeMolotovDamage => (0.8, 10.0),
            Quote::FragKill | Quote::MolotovKill | Quote::PhdKill => (0.9, 15.0),
            Quote::MonkeyBombActivate | Quote::MonkeyBombExplode => (0.9, 15.0),
            // (A low-ammo warning once per reload's worth of fighting.)
            Quote::LowAmmo => (1.0, 20.0),
            _ => (1.0, 0.0),
        };
        Rule { chance, cooldown }
    }
}

/// How long (s) a line someone was about to say while still saying another
/// can wait for them to finish — said then if they finish in time, never
/// otherwise.
pub const GRACE_SECS: f32 = 1.5;
/// Quiet (s) between two of one speaker's lines.
pub const GAP_SECS: f32 = 0.3;
/// How long (s) after a dog round begins, and after the exfil's called,
/// someone says so (the leader can change both — [`crate::SetQuoteDelays`]).
pub const DOG_ROUND_DELAY_SECS: f32 = 4.0;
pub const EXFIL_DELAY_SECS: f32 = 5.0;

/// One line: its file under `audio/quotes/`, and how long it runs (s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub path: &'static str,
    pub secs: f32,
}

const LINES_TXT: &str = include_str!("../../client/assets/audio/quotes/lines.txt");

/// Every line, by operator and folder, in file-name order.
static LINES: LazyLock<HashMap<(Operator, &'static str), Vec<Line>>> = LazyLock::new(|| {
    let mut map: HashMap<(Operator, &'static str), Vec<Line>> = HashMap::new();
    for row in LINES_TXT.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((path, secs)) = row.rsplit_once(' ') else {
            continue;
        };
        let Some((op, rest)) = path.split_once('/') else {
            continue;
        };
        let Some((dir, _file)) = rest.rsplit_once('/') else {
            continue;
        };
        let Some(op) = Operator::ALL.into_iter().find(|o| o.dir() == op) else {
            continue;
        };
        let secs = secs.parse().unwrap_or(4.0);
        map.entry((op, dir)).or_default().push(Line { path, secs });
    }
    for lines in map.values_mut() {
        lines.sort_by(|a, b| a.path.cmp(b.path));
    }
    map
});

/// What `operator` can say for `quote`.
pub fn lines(operator: Operator, quote: Quote) -> &'static [Line] {
    LINES.get(&(operator, quote.dir())).map_or(&[], Vec::as_slice)
}

/// Every [`Quote`] there is (for checking each has lines).
pub fn all() -> Vec<Quote> {
    let mut all = vec![
        Quote::BossKill,
        Quote::BossRepeat,
        Quote::FirstBoss,
        Quote::BuyAk74,
        Quote::BuyAmmo,
        Quote::BuyArmor,
        Quote::DogKill,
        Quote::DogRound,
        Quote::EnemyKill,
        Quote::Exfil,
        Quote::FragKill,
        Quote::LowAmmo,
        Quote::OutOfAmmo,
        Quote::MeleeKill,
        Quote::MolotovKill,
        Quote::MonkeyBombActivate,
        Quote::MonkeyBombExplode,
        Quote::MysteryAk74,
        Quote::MysteryFlashBang,
        Quote::MysteryFrag,
        Quote::MysteryMolotov,
        Quote::MysteryMonkeyBomb,
        Quote::MysteryRayGun,
        Quote::MysteryThrowingKnife,
        Quote::Pap,
        Quote::PhdKill,
        Quote::PickupEquipment,
        Quote::PowerOn,
        Quote::StunEnemy,
        Quote::TakeDamage,
        Quote::TakeMolotovDamage,
        Quote::ThrowingKnifeKill,
    ];
    all.extend(
        [PowerUp::MaxAmmo, PowerUp::InstaKill, PowerUp::DoublePoints, PowerUp::Nuke, PowerUp::BonusPoints]
            .map(Quote::PowerUp),
    );
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_quote_has_lines_for_every_operator_and_every_line_is_there() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/assets/audio/quotes");
        for op in Operator::ALL {
            for q in all() {
                let lines = lines(op, q);
                assert!(!lines.is_empty(), "{op:?} has nothing to say for {q:?}");
                for l in lines {
                    assert!(root.join(l.path).is_file(), "missing {}", l.path);
                    assert!(l.secs > 0.5 && l.secs < 20.0, "{} runs {} s", l.path, l.secs);
                }
            }
        }
        // (Taking damage doesn't pick up the molotov lines in its subfolder.)
        assert!(lines(Operator::Adam, Quote::TakeDamage).iter().all(|l| !l.path.contains("molotov")));
    }

    #[test]
    fn the_rare_ones_come_nearly_every_time_the_common_ones_now_and_then() {
        assert!(Quote::MolotovKill.rule().chance > 0.8 && Quote::MolotovKill.rule().cooldown > 0.0);
        assert!(Quote::MeleeKill.rule().chance < 0.3);
        assert_eq!(Quote::BossKill.rule().chance, 1.0);
        assert_eq!(Quote::OutOfAmmo.rule().chance, 1.0);
    }
}
