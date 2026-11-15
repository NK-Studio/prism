//! Posing: turning a joint, and two-bone inverse kinematics (upper arm + forearm, thigh + shin).
use super::math::{Q, V3, add, between, cross, dot, len, normalize_or, qconj, qmul, qnorm, scale, sub};
use super::skeleton::{Posed, find, parent_rotation, pose};
use prism_doc::figure::Figure;

/// Turn joint `name` by the world-space rotation `q` (about its own position), so it and every
/// joint below it move.
pub fn rotate_joint(figure: &mut Figure, name: &str, q: Q) -> Result<(), String> {
    let posed = pose(figure);
    let p = parent_rotation(&posed, name).ok_or_else(|| format!("the figure has no joint {name:?}"))?;
    let j = figure.root.find_mut(name).ok_or_else(|| format!("the figure has no joint {name:?}"))?;
    // World W = P·L; turned W' = q·W, so L' = P⁻¹·q·P·L.
    j.rotation = qnorm(qmul(qmul(qmul(qconj(p), q), p), qnorm(j.rotation)));
    Ok(())
}

fn parent_of<'a>(posed: &'a [Posed], j: &Posed) -> Option<&'a Posed> {
    j.parent.and_then(|p| posed.get(p))
}

/// Move the end joint `end` (a hand, a foot) towards the world point `target` by turning its
/// grandparent (upper arm, thigh) and parent (forearm, shin), keeping the side the chain bends
/// to. Out-of-reach targets stretch the chain straight towards them.
pub fn reach(figure: &mut Figure, end: &str, target: V3) -> Result<(), String> {
    if !target.iter().all(|v| v.is_finite()) {
        return Err("invalid reach target".into());
    }
    let posed = pose(figure);
    let c = find(&posed, end).ok_or_else(|| format!("the figure has no joint {end:?}"))?;
    let b = parent_of(&posed, c).ok_or("the joint has no parent to bend")?;
    let a = parent_of(&posed, b).ok_or("the joint's parent has no parent to bend")?;
    let (pa, pb, pc) = (a.pos, b.pos, c.pos);
    let (l1, l2) = (len(sub(pb, pa)), len(sub(pc, pb)));
    if l1 < 1e-9 || l2 < 1e-9 {
        return Err("the limb has no length to bend".into());
    }
    let to = sub(target, pa);
    let dir = normalize_or(to, normalize_or(sub(pc, pa), [0.0, -1.0, 0.0]));
    let d = len(to).clamp((l1 - l2).abs() + 1e-6, l1 + l2 - 1e-6);
    // Bend towards where the middle joint already is.
    let ab = sub(pb, pa);
    let mut perp = sub(ab, scale(dir, dot(ab, dir)));
    if len(perp) < 1e-9 {
        perp = cross(normalize_or(cross(ab, sub(pc, pb)), [0.0, 0.0, 1.0]), dir);
    }
    let perp = normalize_or(perp, [0.0, 0.0, 1.0]);
    let along = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let h = (l1 * l1 - along * along).max(0.0).sqrt();
    let mid = add(pa, add(scale(dir, along), scale(perp, h)));
    let end_at = add(pa, scale(dir, d));
    let (a_name, b_name) = (a.name.clone(), b.name.clone());
    rotate_joint(figure, &a_name, between(sub(pb, pa), sub(mid, pa)))?;
    let posed = pose(figure);
    let (nb, nc) = (find(&posed, &b_name).ok_or("lost the limb")?, find(&posed, end).ok_or("lost the limb")?);
    let q = between(sub(nc.pos, nb.pos), sub(end_at, nb.pos));
    rotate_joint(figure, &b_name, q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figure3d::math::axis_angle;
    use prism_doc::figure::standing_figure;

    fn at(f: &Figure, name: &str) -> V3 {
        find(&pose(f), name).unwrap().pos
    }

    #[test]
    fn turning_a_joint_moves_what_hangs_from_it() {
        let mut f = standing_figure();
        let (elbow, hand) = (at(&f, "leftforearm_bb_"), at(&f, "lefthand_bb_"));
        // Turn the forearm a quarter about the forward axis: the hand swings, the elbow stays.
        rotate_joint(&mut f, "leftforearm_bb_", axis_angle([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2)).unwrap();
        assert!(len(sub(at(&f, "leftforearm_bb_"), elbow)) < 1e-9);
        let moved = at(&f, "lefthand_bb_");
        assert!((len(sub(moved, elbow)) - len(sub(hand, elbow))).abs() < 1e-9, "bone length kept");
        assert!(len(sub(moved, hand)) > 10.0);
        assert!(rotate_joint(&mut f, "tail_bb_", axis_angle([0.0, 1.0, 0.0], 1.0)).is_err());
    }

    #[test]
    fn reaching_moves_the_hand_to_reachable_points_and_stretches_towards_far_ones() {
        let mut f = standing_figure();
        let shoulder = at(&f, "leftarm_bb_");
        let goal = add(shoulder, [-20.0, -15.0, 25.0]);
        reach(&mut f, "lefthand_bb_", goal).unwrap();
        assert!(len(sub(at(&f, "lefthand_bb_"), goal)) < 1e-6, "reached");
        assert!(len(sub(at(&f, "leftarm_bb_"), shoulder)) < 1e-9, "the shoulder stays");
        let far = add(shoulder, [0.0, 200.0, 0.0]);
        reach(&mut f, "lefthand_bb_", far).unwrap();
        let hand = at(&f, "lefthand_bb_");
        let straight = normalize_or(sub(hand, shoulder), [0.0; 3]);
        assert!(dot(straight, [0.0, 1.0, 0.0]) > 0.999, "pointing at the far target");
        assert!(reach(&mut f, "lefthand_bb_", [f64::NAN, 0.0, 0.0]).is_err());
        assert!(reach(&mut f, "root_group_", goal).is_err());
        // Legs too.
        let knee_before = at(&f, "rightleg_bb_");
        let foot_goal = add(at(&f, "rightupleg_bb_"), [0.0, -50.0, 40.0]);
        reach(&mut f, "rightfoot_bb_", foot_goal).unwrap();
        assert!(len(sub(at(&f, "rightfoot_bb_"), foot_goal)) < 1e-6);
        assert!(len(sub(at(&f, "rightleg_bb_"), knee_before)) > 1.0);
    }
}
