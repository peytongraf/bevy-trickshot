//! The flash bang (`Zombies` only) — a *tactical*, not a lethal: it's carried
//! alongside whatever lethal a player has, on its own key, and hurts nobody.
//! Held like a frag but, with no pin out yet, it can be put away again;
//! thrown, it bounces and rolls like one ([`crate::frag::FragBody`]) and goes
//! off a fixed [`FUSE_SECS`] after it leaves the hand.
//!
//! Going off, it stuns every zombie within [`STUN_RADIUS`] for
//! [`STUN_SECS`] — slowed right down, no attacking (the PhD Flopper stun,
//! `server::ai`) — and blinds its own thrower if they're close enough and can
//! see it ([`blind_secs`]). Nobody else is flashed.
//!
//! The server runs it (`server::flash_bangs`); clients draw it, the flash,
//! and the thrower's white-out (`client::flash_bang`).

/// Most flash bangs a player can carry.
pub const MAX_FLASH_BANGS: u32 = 2;
/// Seconds from leaving the hand to going off.
pub const FUSE_SECS: f32 = 1.5;
/// Zombies this close (m) to it when it goes off are stunned...
pub const STUN_RADIUS: f32 = 10.0;
/// ...for this long (s).
pub const STUN_SECS: f32 = 5.0;
/// Its thrower, this close (m) and in sight of it, is blinded...
pub const BLIND_RADIUS: f32 = 10.0;
/// ...for up to this long (s) — right on top of it, looking at it.
pub const BLIND_MAX_SECS: f32 = 4.0;

/// The chance (0–1) a zombie a player kills drops a flash bang.
pub const DROP_CHANCE: f32 = 0.02;

/// How long (s) its thrower is blinded, `distance` m from it, `facing` the
/// dot of their view with the way to it (1 looking straight at it, -1 with
/// their back to it): the full [`BLIND_MAX_SECS`] on top of it, nothing at
/// [`BLIND_RADIUS`], and half as long with their back turned.
pub fn blind_secs(distance: f32, facing: f32) -> f32 {
    let near = (1.0 - distance / BLIND_RADIUS).clamp(0.0, 1.0);
    let look = 0.5 + 0.5 * facing.clamp(0.0, 1.0);
    BLIND_MAX_SECS * near.powf(0.7) * look
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closer_and_looking_at_it_blinds_longer() {
        assert!((blind_secs(0.0, 1.0) - BLIND_MAX_SECS).abs() < 1e-4);
        assert_eq!(blind_secs(BLIND_RADIUS, 1.0), 0.0);
        assert!(blind_secs(3.0, 1.0) > blind_secs(6.0, 1.0));
        assert!(blind_secs(3.0, 1.0) > blind_secs(3.0, -1.0));
        assert!(blind_secs(3.0, -1.0) > 0.0);
    }
}
