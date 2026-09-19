//! Patch notes shown under "WHAT'S NEW" on the main-menu screen
//! ([`crate::lobby_ui::build_browser`]) — the only place players see what
//! changed between releases, since the client updates itself silently
//! (`updater.rs`) with no other notification.
//!
//! AI agents and contributors: whenever you finish a feature or bug fix that
//! ships to players, add one line to `ENTRIES` under the current in-progress
//! version (newest version first, newest line within a version first). This
//! doesn't happen automatically — if you don't add the line here, players
//! never see it.

/// `(version, notes)`, newest version first. `version` is the tag this will
/// ship under — keep it in sync with the workspace `Cargo.toml` `version`
/// (bump both together when starting the next round of changes).
pub const ENTRIES: &[(&str, &[&str])] = &[(
    "0.4.5",
    &[
        "Added knife attacks: with the knife out, click while close to and roughly aiming at a bot (Freestyle) or another player (Free For All) to stab and instantly kill them. Knife kills on bots are worth 25 points.",
        "Fixed: the kill cam now plays the death animation on you (the player who got shot) at the moment the killer's shot lands, instead of your body just standing there.",
        "Added a new map: SHIPMENT DAY. It's the same Shipment yard, but in bright daytime — clear sky, full sun, barely any fog, and no rain. Pick it from the map buttons in the lobby.",
    ],
)];
