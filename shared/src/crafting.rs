//! The `Zombies` crafting table, Cold War's: placed in the level editor
//! ([`crate::level::ZombiesLayout::crafting_table`]), it sells equipment one
//! at a time for points — the lethals (frag, molotov) and the tacticals
//! (flash bang, monkey bomb) — at Cold War's crafting prices ([`cost`]).
//! Crafting a kind other than the one carried of its sort swaps to it, the
//! old ones dropped (`crate::lethal`).
//!
//! The server takes the points (`server::crafting`); the client keeps the
//! count, as it does for every lethal (`client::crafting`).

use bevy::math::{Quat, Vec3};

use crate::lethal::LethalKind;
use crate::level::Placement;
use crate::perks::PERK_USE_HEIGHT;
use crate::MapId;

/// The table's solid box (half extents, m) at scale 1 — `client::crafting`
/// fits the model to it.
pub const HALF_EXTENTS: Vec3 = Vec3::new(1.25, 1.03, 0.35);

/// How far (m, across the ground) from the table's middle a player can use
/// it.
pub const USE_RADIUS: f32 = 2.2;

/// The menu's two tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Tactical,
    Lethal,
}

/// What the table sells, tab by tab, in the order its tiles show.
pub const TACTICALS: [LethalKind; 2] = [LethalKind::FlashBang, LethalKind::MonkeyBomb];
pub const LETHALS: [LethalKind; 2] = [LethalKind::Frag, LethalKind::Molotov];

/// What `tab` sells.
pub fn items(tab: Tab) -> &'static [LethalKind] {
    match tab {
        Tab::Tactical => &TACTICALS,
        Tab::Lethal => &LETHALS,
    }
}

/// What one `kind` costs to craft (Cold War's salvage prices, in points) —
/// `None` for what the table doesn't sell.
pub fn cost(kind: LethalKind) -> Option<u32> {
    match kind {
        LethalKind::Frag | LethalKind::Molotov => Some(250),
        // (Cold War's stun grenade — its flash.)
        LethalKind::FlashBang => Some(250),
        LethalKind::MonkeyBomb => Some(1000),
        LethalKind::ThrowingKnife => None,
    }
}

/// Where `map`'s table stands, if it has one.
pub fn placement(map: MapId) -> Option<Placement> {
    crate::level::layout(map).crafting_table
}

/// Whether feet at `feet` can use a table standing at `at` (`slack` widens
/// it — the server allows for the pose being a moment old).
pub fn in_range_of(at: Placement, feet: Vec3, slack: f32) -> bool {
    let d = feet - at.pos;
    // (A bigger table reaches further out.)
    let reach = USE_RADIUS + (at.scale - 1.0).max(0.0) * HALF_EXTENTS.x;
    Vec3::new(d.x, 0.0, d.z).length() <= reach + slack && d.y.abs() <= PERK_USE_HEIGHT + slack
}

/// Whether feet at `feet` can use `map`'s table.
pub fn in_range(map: MapId, feet: Vec3, slack: f32) -> bool {
    placement(map).is_some_and(|at| in_range_of(at, feet, slack))
}

/// The table's solid box on `map`, if it has one: `(centre, rotation, half
/// extents)`.
pub fn solid_box(map: MapId) -> Option<(Vec3, Quat, Vec3)> {
    placement(map).map(|at| {
        let half = HALF_EXTENTS * at.scale;
        (at.pos + Vec3::Y * half.y, at.rotation(), half)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_sells_the_tacticals_and_lethals_at_cold_wars_prices() {
        for kind in TACTICALS {
            assert!(kind.is_tactical());
        }
        for kind in LETHALS {
            assert!(!kind.is_tactical());
        }
        assert_eq!(cost(LethalKind::Frag), Some(250));
        assert_eq!(cost(LethalKind::MonkeyBomb), Some(1000));
        assert_eq!(cost(LethalKind::ThrowingKnife), None);
        for kind in TACTICALS.into_iter().chain(LETHALS) {
            assert!(cost(kind).is_some());
        }
    }
}
