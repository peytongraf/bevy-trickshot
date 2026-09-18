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
    "0.4.3",
    &[
        "Added a VSYNC toggle and a frame rate limit slider under Settings → Graphics → Display, so you can turn vsync back on (removes screen tearing) or cap the frame rate instead of running fully uncapped.",
        "Turned off vsync by default — mouse look should feel noticeably snappier and less sluggish, at the cost of possible screen tearing (most visible without a G-Sync/FreeSync monitor).",
        "Fixed: switching to the knife no longer blocks you from attacking with it while the grip-adjust animation plays out. That animation is now a random idle fidget instead, and an attack always cuts it short instantly.",
        "Fixed: a reload (or other in-progress sound) no longer keeps playing through a kill cam replay — everything but the ambience bed now cuts off the instant the replay starts.",
        "Fixed: the knife no longer stays visible on top of the sniper during a kill cam replay if you'd switched to it right before you died.",
        "The \"What's New\" notes on the main menu now use a plain, readable font instead of the bold condensed HUD face.",
        "Fixed: you can now slide along a wall you hit at an angle instead of sticking to it until you're moving exactly parallel to it.",
        "Fixed: the water on Shipment no longer renders on top of the barrel smoke.",
        "Fixed: the barrel smoke puff no longer flashes fully opaque and facing the wrong way for one frame right when it spawns. Also tuned the barrel smoke and bullet-tracer trail to be lighter and fade faster.",
        "Fixed: respawning in Free For All now refills your ammo instead of carrying over the mag/reserve count from your last life.",
        "Sprint now stays on through slides and dives (Call of Duty: MW3 style) — it only pauses while you're actually crouched or prone, and resumes on its own the moment you stand back up. No more re-pressing sprint after a slide.",
    ],
)];
