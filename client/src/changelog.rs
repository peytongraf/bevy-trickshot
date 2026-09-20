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
        "Sniper damage now depends on distance too. Up close every hit kills, even a leg shot; further out a leg shot stops killing (past about 70 m), then a torso shot (past about 175 m), and at extreme range (past about 250 m) even a headshot leaves the target alive.",
        "Sniper shots are no longer always one-shot kills: a hit to the legs or lower body only does 60% damage, so the target survives (bots and players both have health now). When you damage but don't kill someone you get a hit marker and sound. In Free For All, damaged players heal back after 3 seconds.",
        "Fall damage: landing from a big drop now hurts in proportion to how far you fell (nothing under 16 m, dead at 30 m). When you're hurt the screen tints red with a blood splatter and you hear a heartbeat, both stronger the lower your health. Health holds for 3 seconds, then builds back up.",
        "The throwing arms (and the knife in their hand) now sway when you turn and breathe while you stand still, just like the sniper, and the regular knife now breathes too.",
        "Added a sound for switching to the sniper (only you hear it).",
        "Added knife sounds: an equip sound when you switch to the knife (only you hear it), a stab sound when you stab a bot or player (everyone hears it from the victim), and a swing sound when you attack and miss (everyone hears it from you). Volumes are in the debug panel. Throwing knife impact sounds are also quieter now.",
        "A thrown knife now makes an impact sound (one of several, picked at random) from the spot where it hits a wall, the ground or a crate. Everyone in the lobby hears it, and each clip has its own volume slider in the debug panel.",
        "Added throwing knife sounds: a throw sound when you let go (other players hear it from where you are), a whoosh that follows the knife through the air for everyone, and a hit sound from the spot where a thrown knife kills a bot or player. Volumes are in the debug panel.",
        "Shots now leave bullet holes on the surfaces they hit, and everyone in the lobby sees them. They disappear after a minute.",
        "Fixed: the throwing knife crosshair now shows in kill cams (and stays up while the throwing arms are on screen) instead of the plain centre dot.",
        "Fixed: kill cams now show the throwing arms and the knife being thrown (and flying, bouncing and landing the kill), instead of leaving them out.",
        "Shots no longer go through walls, crates or containers: a bullet (and its tracer) stops at the first surface it hits, so you can't hit anything behind cover.",
        "You can now actually throw the throwing knife! Hold V, aim and let go. It flies in an arc, bounces off walls and the ground, losing speed each time, then stops and disappears. Everyone in the lobby sees it. In Freestyle it kills bots (40 points); in Free For All it kills other players.",
        "The throwing arms now hold a throwing knife while you hold V. It disappears the moment the throw animation starts.",
        "Holding the throwing knife key (V) now quickly puts your weapon away, then slides a pair of throwing arms up from below. Let go and they play a throw animation and slide back down, and only then does your weapon come back. Press the swap-weapon key while holding to cancel without throwing. There is no knife to throw yet.",
        "Removed PRACTICE mode. Every game now runs through the server: create a lobby (or join one) and start it to play. Freestyle still gives you bots to shoot at.",
    ],
)];
