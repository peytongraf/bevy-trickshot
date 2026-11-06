//! Shared rules for the map lights that take turns casting shadows
//! (`power::limit_power_shadows`, `flames::limit_flame_shadows`). Each one
//! costs six shadow views a frame, so only a few can cast at once.
//!
//! A light without its shadows shines straight through every wall in its
//! reach, so one we can't see that loses its turn fades out rather than
//! carrying on unshadowed: it keeps its shadows until it's dark, and gets them
//! back before it brightens. It can't light anything we're looking at except
//! through a wall, so nothing's missed. One in view stays on without shadows
//! (going dark would be plain to see), but turns go to those first
//! ([`score`]), so that's only when more are in view than there are turns.
//!
//! On a shadow quality with no turns to give (`0`), there's nothing to fade
//! between: the lights just stay on without shadows.
//!
//! Everything here is held on the lights themselves or in their systems'
//! `Local`s, and cleared with them when the game's left.

use bevy::prelude::*;
use bevy_rapier3d::prelude::{QueryFilter, ReadRapierContext};
use std::collections::HashMap;
use std::hash::Hash;

/// How many metres nearer than it is a light we can see counts as — enough
/// to outrank the lights just the other side of a wall.
const IN_VIEW_BONUS: f32 = 25.0;
/// How long (s) a light still counts as in view after we last saw it, so
/// passing a pillar doesn't cost it its turn.
const IN_VIEW_HOLD_SECS: f32 = 2.0;
/// How long (s) a light takes to fade out losing its turn, or in getting it.
const FADE_SECS: f32 = 0.4;
/// How close (m) to the light a ray can stop and still count as reaching it
/// — one tucked into a fixture or a floor.
const RAY_SLACK: f32 = 0.5;

/// Whether a ray from `eye` reaches `light` past the map (and its doors).
fn in_view(rapier: &ReadRapierContext, eye: Vec3, light: Vec3) -> bool {
    let to = light - eye;
    let distance = to.length();
    if distance <= RAY_SLACK {
        return true;
    }
    let Ok(context) = rapier.single() else {
        return false;
    };
    context
        .cast_ray(eye, to / distance, distance - RAY_SLACK, true, QueryFilter::only_fixed())
        .is_none()
}

/// A light's claim to a turn, lower first: `distance` (m, however its caller
/// measures it) less the bonuses for being in view and for already having a
/// turn (`swap_margin` — so two about level don't keep swapping). Notes in
/// `last_seen` if it's in view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn score<K: Eq + Hash>(
    rapier: &ReadRapierContext,
    last_seen: &mut HashMap<K, f32>,
    key: K,
    eye: Vec3,
    light: Vec3,
    now: f32,
    distance: f32,
    held: bool,
    swap_margin: f32,
) -> f32 {
    if in_view(rapier, eye, light) {
        last_seen.insert(key, now);
        return distance - IN_VIEW_BONUS - if held { swap_margin } else { 0.0 };
    }
    let seen = last_seen.get(&key).is_some_and(|at| seen_at(*at, now));
    distance - if seen { IN_VIEW_BONUS } else { 0.0 } - if held { swap_margin } else { 0.0 }
}

fn seen_at(at: f32, now: f32) -> bool {
    now - at <= IN_VIEW_HOLD_SECS
}

/// Whether `key`'s light counts as in view, going by [`score`]'s notes.
pub(crate) fn seen<K: Eq + Hash>(last_seen: &HashMap<K, f32>, key: &K, now: f32) -> bool {
    last_seen.get(key).is_some_and(|at| seen_at(*at, now))
}

/// Move a light's `fade` (0..=1) toward where it should be, returning whether
/// it casts shadows this frame: with a turn, or (out of view) until it's dark.
pub(crate) fn step_fade(fade: &mut f32, has_turn: bool, in_view: bool, turns: usize, dt: f32) -> bool {
    if turns == 0 {
        *fade = 1.0;
        return false;
    }
    let target = if has_turn || in_view { 1.0 } else { 0.0 };
    let step = dt / FADE_SECS;
    *fade += (target - *fade).clamp(-step, step);
    has_turn || (!in_view && *fade > 0.0)
}
