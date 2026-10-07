//! The lethals a player can carry — one kind at a time. Picking up (or
//! taking from the Mystery Box) another kind swaps to it: the client says
//! what it's carrying of the old kind ([`Carried`]) and the server drops
//! that many around them.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LethalKind {
    ThrowingKnife,
    Molotov,
    MonkeyBomb,
    Frag,
}

impl LethalKind {
    /// The most of it a player can carry.
    pub const fn max_carried(self) -> u32 {
        match self {
            LethalKind::ThrowingKnife => crate::throwing_knife::MAX_CARRIED,
            LethalKind::Molotov => crate::molotov::MAX_MOLOTOVS,
            LethalKind::MonkeyBomb => crate::monkey_bomb::MAX_MONKEYS,
            LethalKind::Frag => crate::frag::MAX_FRAGS,
        }
    }
}

/// How many of a kind a player's carrying — sent along with a pickup of
/// another kind, so the server can drop them.
pub type Carried = Option<(LethalKind, u32)>;
