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
    "0.4.6",
    &[
        "Holding the throwing knife key (V) now quickly puts your weapon away, then slides a pair of throwing arms up from below. Let go and they play a throw animation and slide back down, and only then does your weapon come back. Press the swap-weapon key while holding to cancel without throwing. There is no knife to throw yet.",
        "Removed PRACTICE mode. Every game now runs through the server: create a lobby (or join one) and start it to play. Freestyle still gives you bots to shoot at.",
    ],
)];
