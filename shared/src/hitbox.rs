//! Capsule hitboxes and the ray test the shot resolver uses against them.

use bevy::math::Vec3;
use serde::{Deserialize, Serialize};

use crate::ballistics::HitZone;

/// A capsule: the segment `a`..`b` swept by a sphere of `radius`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Capsule {
    pub a: Vec3,
    pub b: Vec3,
    pub radius: f32,
}

impl Capsule {
    /// Upright body capsule for a player whose feet are at `feet`.
    pub fn standing(feet: Vec3, height: f32, radius: f32) -> Self {
        let r = radius.min(height * 0.5);
        Self {
            a: feet + Vec3::Y * r,
            b: feet + Vec3::Y * (height - r),
            radius: r,
        }
    }

    /// Head sphere sitting on top of the [`Capsule::standing`] body.
    pub fn head(feet: Vec3, height: f32, radius: f32) -> Self {
        let c = feet + Vec3::Y * (height - radius);
        Self { a: c, b: c, radius }
    }
}

/// Which part of a body a skeleton bone named `name` is: [`HitZone::Head`]
/// for the head (jaw, eyes, a boss's ponytail), [`HitZone::Legs`] for the
/// hips down (and a hellhound's legs and tail), [`HitZone::Torso`] for the
/// rest — spine, neck, shoulders, arms, hands and fingers. Works on every
/// character's rig (`soldier`, `zombie`, `dog` and `boss.glb`), whose bones
/// are named differently but all say what they are.
pub fn zone_of_bone(name: &str) -> HitZone {
    let n = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    // (Before the head: the hellhound's neck is "head neck".)
    if has(&["neck"]) {
        return HitZone::Torso;
    }
    if has(&["head", "jaw", "eye", "ponytail"]) {
        return HitZone::Head;
    }
    if has(&["arm", "hand", "finger", "palm", "thumb", "clavicle", "collarbone", "shoulder"]) {
        return HitZone::Torso;
    }
    if has(&["thigh", "calf", "leg", "foot", "toe", "hip", "pelvis", "tail", "digit"]) {
        return HitZone::Legs;
    }
    HitZone::Torso
}

/// Closest positive distance `t` along the ray `origin + t * dir` at which it
/// enters `cap`, or `None` on a miss. `dir` must be unit length. An `origin`
/// already inside the capsule returns `Some(0.0)`.
pub fn ray_capsule(origin: Vec3, dir: Vec3, cap: &Capsule) -> Option<f32> {
    let mut best: Option<f32> = None;
    let mut consider = |t: f32| {
        if t >= 0.0 && best.map_or(true, |b| t < b) {
            best = Some(t);
        }
    };

    // Spheres at each end.
    if let Some(t) = ray_sphere(origin, dir, cap.a, cap.radius) {
        consider(t);
    }
    if let Some(t) = ray_sphere(origin, dir, cap.b, cap.radius) {
        consider(t);
    }

    // Cylindrical middle: solve |m + t·dir - (·)·d̂ d̂|² = r² and keep hits whose
    // projection lands within the segment.
    let ab = cap.b - cap.a;
    let ab_len2 = ab.length_squared();
    if ab_len2 > 1e-12 {
        let inv_len = ab_len2.sqrt().recip();
        let d = ab * inv_len;
        let m = origin - cap.a;
        let md = m.dot(d);
        let nd = dir.dot(d);
        let a_coef = 1.0 - nd * nd;
        let b_coef = 2.0 * (m.dot(dir) - md * nd);
        let c_coef = m.length_squared() - md * md - cap.radius * cap.radius;
        if a_coef.abs() > 1e-12 {
            let disc = b_coef * b_coef - 4.0 * a_coef * c_coef;
            if disc >= 0.0 {
                let sq = disc.sqrt();
                let seg_len = ab_len2.sqrt();
                for t in [
                    (-b_coef - sq) / (2.0 * a_coef),
                    (-b_coef + sq) / (2.0 * a_coef),
                ] {
                    if t >= 0.0 {
                        let along = md + t * nd;
                        if (0.0..=seg_len).contains(&along) {
                            consider(t);
                        }
                    }
                }
            }
        }
    }

    if best.is_none() && point_in_capsule(origin, cap) {
        return Some(0.0);
    }
    best
}

/// Ray vs sphere, `dir` unit length. Returns the nearer non-negative root.
fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let c = oc.length_squared() - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t0 = -b - sq;
    if t0 >= 0.0 {
        Some(t0)
    } else {
        let t1 = -b + sq;
        (t1 >= 0.0).then_some(t1)
    }
}

fn point_in_capsule(p: Vec3, cap: &Capsule) -> bool {
    let ab = cap.b - cap.a;
    let ab_len2 = ab.length_squared();
    let t = if ab_len2 > 1e-12 {
        ((p - cap.a).dot(ab) / ab_len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest = cap.a + ab * t;
    p.distance_squared(closest) <= cap.radius * cap.radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rigs_bones_land_in_the_right_zone() {
        use HitZone::*;
        for (bone, zone) in [
            // soldier.glb
            ("Bind_Head_014_015", Head),
            ("Bind_Neck_013_014", Torso),
            ("Bind_Spine2_012_013", Torso),
            ("Bind_LeftShoulder_015_016", Torso),
            ("Bind_RightForeArm_036_037", Torso),
            ("Bind_LeftHandPinky3_033_034", Torso),
            ("Bind_Hips_03_00", Legs),
            ("Bind_RightUpLeg_01_07", Legs),
            ("Bind_LeftToeBase_06_06", Legs),
            // zombie.glb
            ("Base HumanHead_09", Head),
            ("Base HumanHeadJaw_010", Head),
            ("Base HumanLEye_014", Head),
            ("Base HumanRibcage_07", Torso),
            ("Base HumanRArmDigit23_023", Torso),
            ("Base HumanLArmCollarbone_033", Torso),
            ("Base HumanRDigit11_05", Legs),
            ("Base HumanLLegCalf_052", Legs),
            ("Base HumanPelvis_01", Legs),
            // dog.glb
            ("head_23", Head),
            ("head jaw_22", Head),
            ("head neck.001_24", Torso),
            ("spine up 2 _25", Torso),
            ("leg front 2_L_28", Legs),
            ("Leg back 3_R_18", Legs),
            ("tail4_7", Legs),
            // boss.glb
            ("Bip01_Head_014", Head),
            ("Bip01_Ponytail1_015", Head),
            ("Bip01_L_Finger12_024", Torso),
            ("Bip01_R_UpperArm_037", Torso),
            ("Bip01_Spine1_011", Torso),
            ("Bip01_L_Calf_06", Legs),
            ("Bip01_Pelvis_02", Legs),
        ] {
            assert_eq!(zone_of_bone(bone), zone, "{bone}");
        }
    }

    #[test]
    fn straight_hit_down_neg_z() {
        let cap = Capsule::standing(Vec3::new(0.0, 0.0, -10.0), 1.8, 0.35);
        let t = ray_capsule(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, &cap);
        assert!(t.is_some());
        assert!((t.unwrap() - 9.65).abs() < 0.1);
    }

    #[test]
    fn clean_miss_to_the_side() {
        let cap = Capsule::standing(Vec3::new(0.0, 0.0, -10.0), 1.8, 0.35);
        assert!(ray_capsule(Vec3::new(5.0, 1.0, 0.0), Vec3::NEG_Z, &cap).is_none());
    }

    #[test]
    fn headshot_ray_clears_body_ray() {
        let feet = Vec3::new(0.0, 0.0, -20.0);
        let head = Capsule::head(feet, 1.8, 0.12);
        let t = ray_capsule(Vec3::new(0.0, 1.68, 0.0), Vec3::NEG_Z, &head);
        assert!(t.is_some());
    }
}
