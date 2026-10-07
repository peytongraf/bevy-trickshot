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
