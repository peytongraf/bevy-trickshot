//! The equipment a player can carry: one lethal kind and one tactical kind
//! at a time ([`LethalKind::is_tactical`] — the monkey bomb and the flash
//! bang, as in Cold War). Picking up (or taking from the Mystery Box, or
//! crafting) another kind of the same sort swaps to it: the client says
//! what it's carrying of the old kind ([`Carried`]) and the server drops
//! that many around them.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LethalKind {
    ThrowingKnife,
    Molotov,
    MonkeyBomb,
    Frag,
    FlashBang,
}

impl LethalKind {
    /// The most of it a player can carry.
    pub const fn max_carried(self) -> u32 {
        match self {
            LethalKind::ThrowingKnife => crate::throwing_knife::MAX_CARRIED,
            LethalKind::Molotov => crate::molotov::MAX_MOLOTOVS,
            LethalKind::MonkeyBomb => crate::monkey_bomb::MAX_MONKEYS,
            LethalKind::Frag => crate::frag::MAX_FRAGS,
            LethalKind::FlashBang => crate::flash_bang::MAX_FLASH_BANGS,
        }
    }

    /// A tactical (carried beside the lethal, on its own key) rather than a
    /// lethal.
    pub const fn is_tactical(self) -> bool {
        matches!(self, LethalKind::MonkeyBomb | LethalKind::FlashBang)
    }
}

/// How many of a kind a player's carrying — sent along with a pickup of
/// another kind, so the server can drop them.
pub type Carried = Option<(LethalKind, u32)>;

/// The chance (0–1) a zombie a player kills drops a piece of equipment at
/// all — one roll shared by every kind ([`zombie_drop`]), so a frag drop
/// and a flash bang drop come out of the same chance rather than each
/// adding its own. About one kill in fifty, as in Cold War / Black Ops 6.
pub const ZOMBIE_DROP_CHANCE: f32 = 0.02;

/// How [`ZOMBIE_DROP_CHANCE`] splits between the kinds (shares of it,
/// adding up to 1) — the monkey bomb, the strongest, rarest.
const ZOMBIE_DROP_SHARES: [(LethalKind, f32); 4] = [
    (LethalKind::Molotov, 0.3),
    (LethalKind::Frag, 0.3),
    (LethalKind::FlashBang, 0.25),
    (LethalKind::MonkeyBomb, 0.15),
];

/// What a zombie killed drops for a `roll` (0–1, one per kill): one kind,
/// or (mostly) nothing.
pub fn zombie_drop(roll: f32) -> Option<LethalKind> {
    let mut below = 0.0;
    for (kind, share) in ZOMBIE_DROP_SHARES {
        below += share * ZOMBIE_DROP_CHANCE;
        if roll < below {
            return Some(kind);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_roll_drops_at_most_one_kind_at_the_overall_chance() {
        let total: f32 = ZOMBIE_DROP_SHARES.iter().map(|(_, s)| s).sum();
        assert!((total - 1.0).abs() < 1e-5);
        let n = 100_000;
        let drops = (0..n).filter(|i| zombie_drop(*i as f32 / n as f32).is_some()).count();
        assert!((drops as f32 / n as f32 - ZOMBIE_DROP_CHANCE).abs() < 1e-3);
        assert_eq!(zombie_drop(0.0), Some(LethalKind::Molotov));
        assert_eq!(zombie_drop(ZOMBIE_DROP_CHANCE * 0.95), Some(LethalKind::MonkeyBomb));
        assert_eq!(zombie_drop(ZOMBIE_DROP_CHANCE * 1.001), None);
        assert!(ZOMBIE_DROP_SHARES.iter().all(|(k, _)| *k != LethalKind::ThrowingKnife));
    }
}
