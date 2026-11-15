//! Put a CLIP STUDIO pose on a character's own skeleton (VRM / glTF humanoids, whose bones carry
//! Prism's joint names but lie in their own rest frames and lengths).
//!
//! Like CLIP STUDIO applying a pose to a placed figure, the character keeps its root (where it
//! stands, which way it faces) and the pose's root placement is ignored. Then, parents first,
//! every bone both skeletons share is turned so it points where the pose's bone points (the
//! character's proportions stay its own), and a few bones whose twist shows are also turned about
//! themselves to match a second direction: the hips (the line between the thighs), the chest (the
//! line between the shoulders), the head (its facing), and the hands (the line across the
//! knuckles). Bones the pose doesn't have keep their rest turn relative to their parent.
use super::ik::rotate_joint;
use super::math::{V3, axis_angle, between, cross, dot, len, normalize_or, qconj, qmul, rotate, scale, sub};
use super::skeleton::{Posed, pose};
use prism_doc::figure::{Figure, Joint};
use std::collections::HashMap;

/// The joint each bone aims at, first match present in both skeletons (Prism / CLIP STUDIO names).
fn aim_targets(name: &str) -> Vec<String> {
    let side = if name.starts_with("left") { "left" } else if name.starts_with("right") { "right" } else { "" };
    let rest = name.get(side.len()..).unwrap_or("");
    let s = |t: &str| format!("{side}{t}");
    match (side, rest) {
        ("", "hips_bb_") => vec!["spine2_bb_".into(), "spine1_bb_".into(), "neck_bb_".into(), "spine_bb_".into()],
        ("", "spine_bb_") => vec!["spine1_bb_".into(), "spine2_bb_".into(), "neck_bb_".into()],
        ("", "spine1_bb_") => vec!["spine2_bb_".into(), "neck_bb_".into()],
        ("", "spine2_bb_") => vec!["neck_bb_".into()],
        ("", "neck_bb_") => vec!["head_bb_".into()],
        (_, "shoulder_bb_") => vec![s("arm_bb_")],
        (_, "arm_bb_") => vec![s("forearm_bb_")],
        (_, "forearm_bb_") => vec![s("hand_bb_")],
        (_, "hand_bb_") => vec![s("handmiddle1_bb_"), s("handindex1_bb_"), s("handring1_bb_")],
        (_, "upleg_bb_") => vec![s("leg_bb_")],
        (_, "leg_bb_") => vec![s("foot_bb_")],
        (_, "foot_bb_") => vec![s("toebase_bb_")],
        _ => {
            // Fingers: segment k aims at k + 1.
            for d in ["thumb", "index", "middle", "ring", "pinky"] {
                if let Some(k) = rest.strip_prefix(&format!("hand{d}")).and_then(|r| r.strip_suffix("_bb_")).and_then(|k| k.parse::<u32>().ok()) {
                    return vec![s(&format!("hand{d}{}_bb_", k + 1))];
                }
            }
            Vec::new()
        }
    }
}

/// Joints only the pose has, for its bone directions: the head's end, finger tips, toe ends.
fn source_extra(name: &str) -> Vec<String> {
    if name == "head_bb_" {
        return vec!["head_end_bb_".into()];
    }
    if let Some(stem) = name.strip_suffix("toebase_bb_") {
        return vec![format!("{stem}toebase_end_bb_")];
    }
    Vec::new()
}

/// Out of the face of a CLIP STUDIO pose's head, in the head's local frame: local +X runs up the
/// head and −Z out of the face (measured on the local poses: facing·(−Z) 0.92–1.00; see
/// docs/sut-analysis.md).
const CSP_HEAD_FORWARD: V3 = [0.0, 0.0, -1.0];

fn map(list: &[Posed]) -> HashMap<&str, &Posed> {
    list.iter().map(|p| (p.name.as_str(), p)).collect()
}

/// The twist reference of joint `name`: a direction to bring onto the pose's, about the bone.
fn second(name: &str, at: &HashMap<&str, &Posed>, head_forward: impl Fn(&Posed) -> Option<V3>) -> Option<V3> {
    let pos = |n: &str| at.get(n).map(|p| p.pos);
    match name {
        "hips_bb_" => Some(sub(pos("leftupleg_bb_")?, pos("rightupleg_bb_")?)),
        "spine2_bb_" | "spine1_bb_" => Some(sub(pos("leftarm_bb_").or_else(|| pos("leftshoulder_bb_"))?, pos("rightarm_bb_").or_else(|| pos("rightshoulder_bb_"))?)),
        "head_bb_" => head_forward(at.get("head_bb_")?),
        "lefthand_bb_" => Some(sub(pos("lefthandpinky1_bb_").or_else(|| pos("lefthandring1_bb_"))?, pos("lefthandindex1_bb_")?)),
        "righthand_bb_" => Some(sub(pos("righthandpinky1_bb_").or_else(|| pos("righthandring1_bb_"))?, pos("righthandindex1_bb_")?)),
        _ => None,
    }
}

/// `v` without its part along unit `axis`, normalised (`None` when nothing is left).
fn across(v: V3, axis: V3) -> Option<V3> {
    let w = sub(v, scale(axis, dot(v, axis)));
    (len(w) > 1e-6).then(|| normalize_or(w, [0.0; 3]))
}

/// Signed angle (radians) from `a` to `b` about unit `axis` (both square to it).
fn angle_about(a: V3, b: V3, axis: V3) -> f64 {
    dot(cross(a, b), axis).atan2(dot(a, b))
}

/// Pose `target` (a character on `rest`, its own rest skeleton) like `source` (a CLIP STUDIO
/// pose). The target's root keeps its place and turn.
pub fn retarget(source: &Figure, target: &mut Figure, rest: &Joint) -> Result<(), String> {
    // The pose as it stands on its own root, unplaced.
    let mut src = source.clone();
    src.root.offset = [0.0; 3];
    src.root.rotation = [1.0, 0.0, 0.0, 0.0];
    let src_posed = pose(&src);
    let src_at = map(&src_posed);
    if !src_at.contains_key("hips_bb_") {
        return Err("the pose has no hips to put on the character".into());
    }
    // Start from the character's rest pose under its own root.
    let (offset, rotation) = (target.root.offset, target.root.rotation);
    let mut fresh = rest.clone();
    fresh.offset = offset;
    fresh.rotation = rotation;
    target.root = fresh;
    if target.root.find("hips_bb_").is_none() {
        return Err("the character has no humanoid bones to pose".into());
    }
    let root_turn = prism_doc::figure::normalize_quat(rotation);
    // Directions of the pose, carried into the character's root frame.
    let src_dir = |a: &str, b: &str| -> Option<V3> {
        let (pa, pb) = (src_at.get(a)?.pos, src_at.get(b)?.pos);
        let d = sub(pb, pa);
        (len(d) > 1e-6).then(|| rotate(root_turn, normalize_or(d, [0.0; 3])))
    };
    let head_forward = |p: &Posed| Some(rotate(p.rot, CSP_HEAD_FORWARD));
    let rest_posed = pose(&Figure { root: rest.clone(), ..Default::default() });
    let rest_at = map(&rest_posed);
    // The character's head faces +Z at rest: carry that through its current turn.
    let rest_head = rest_at.get("head_bb_").map(|p| p.rot);
    let tgt_head_forward = |p: &Posed| rest_head.map(|r| rotate(p.rot, rotate(qconj(r), [0.0, 0.0, 1.0])));
    // Parents first (the tree's own order).
    let order: Vec<String> = pose(target).into_iter().map(|p| p.name).collect();
    for name in &order {
        let tgt_posed = pose(target);
        let tgt_at = map(&tgt_posed);
        let (Some(j), Some(rj)) = (tgt_at.get(name.as_str()), rest_at.get(name.as_str())) else { continue };
        if !src_at.contains_key(name.as_str()) {
            continue;
        }
        // Where the pose's bone points: towards its next joint (the pose has tips the character
        // may lack: the head's end, finger tips, toe ends).
        let mut src_aims = aim_targets(name);
        src_aims.extend(source_extra(name));
        let Some(want) = src_aims.iter().find_map(|c| src_dir(name, c)) else { continue };
        // Where the character's bone points now: at the same next joint, or (a tip it lacks) its
        // rest direction carried through its turn since rest.
        let shared = aim_targets(name).into_iter().find(|c| src_at.contains_key(c.as_str()) && tgt_at.contains_key(c.as_str()));
        let have = match shared.as_ref().and_then(|c| tgt_at.get(c.as_str())) {
            Some(k) => sub(k.pos, j.pos),
            None => {
                let rest_dir = if name == "head_bb_" {
                    [0.0, 1.0, 0.0]
                } else {
                    let parent = rj.parent.and_then(|i| rest_posed.get(i));
                    match parent {
                        Some(pp) => sub(rj.pos, pp.pos),
                        None => continue,
                    }
                };
                rotate(qmul(j.rot, qconj(rj.rot)), rest_dir)
            }
        };
        if len(have) < 1e-6 {
            continue;
        }
        rotate_joint(target, name, between(have, want))?;
        let axis = want;
        // Twist about the bone to match a second direction.
        let tgt_posed = pose(target);
        let tgt_at = map(&tgt_posed);
        let want2 = second(name, &src_at, head_forward).map(|v| rotate(root_turn, v));
        let have2 = second(name, &tgt_at, tgt_head_forward);
        if let (Some(w), Some(h)) = (want2.and_then(|v| across(v, axis)), have2.and_then(|v| across(v, axis))) {
            let a = angle_about(h, w, axis);
            if a.is_finite() && a.abs() > 1e-6 {
                rotate_joint(target, name, axis_angle(axis, a))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "retarget_tests.rs"]
mod tests;
