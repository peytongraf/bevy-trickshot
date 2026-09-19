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
        "Retuned SHIPMENT DAY: a hazier, cooler sky with softer sunlight, and a dark grey sea.",
        "The scope lens now looks like black, slightly frosted glass when you're not aiming, instead of clear blue glass.",
        "The scope's crosshair now moves against the gun's sway, so its centre stays put on the screen while you turn.",
        "Fixed: the scope's crosshair now starts off-centre when you begin aiming and slides to the middle as the scope comes up. That effect had stopped working.",
        "Fixed: the view through the scope now has the same fog as the world around it (on foggy maps like Shipment Night the sky used to look clear inside the lens).",
        "Movement is faster: walking is now 5 m/s (was 3) and sprinting is 9 m/s (was 8).",
        "Added scope zoom levels to the LOADOUT screen: 3x, 8x, and 11x. They're exact multiples of your field of view setting, and the picture inside the scope lens lines up with the world around it at every zoom. The default is 11x, which is close to the old scope.",
        "The knife now sways when you look around, exactly like the sniper does — it lags slightly behind your turn, then catches back up.",
        "Fixed: the knife's stab animation now plays during the kill cam (it used to just sit in its idle pose).",
        "The weapon icon (sniper / knife) in the bottom-right is now twice as big, and the ammo count is hidden while you have the knife out, since it doesn't use ammo.",
        "Added knife attacks: with the knife out, click while close to and roughly aiming at a bot (Freestyle) or another player (Free For All) to stab and instantly kill them. Knife kills on bots are worth 25 points.",
        "Fixed: the kill cam now plays the death animation on you (the player who got shot) at the moment the killer's shot lands, instead of your body just standing there.",
        "Added a new map: SHIPMENT DAY. It's the same Shipment yard, but in bright daytime — clear sky, full sun, barely any fog, and no rain. Pick it from the map buttons in the lobby.",
    ],
)];
