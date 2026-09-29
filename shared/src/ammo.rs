//! The `Zombies` ammo crate: buy a full refill of sniper ammo for points.
//!
//! Where the crate stands is still being tuned on the client (debug panel),
//! so the range check is the client's for now; the server only checks the
//! game, that the buyer is alive, and the points (`server::zombies`).

/// Points a full sniper ammo refill costs.
pub const AMMO_COST: u32 = 500;
