//! Forward kinematics: every joint's world position and orientation.
use super::math::{IDENTITY, Q, V3, add, qmul, qnorm, rotate};
use prism_doc::figure::{Figure, Joint, MAX_DEPTH, MAX_JOINTS};

/// One joint in world space.
#[derive(Clone, Debug)]
pub struct Posed {
    pub name: String,
    pub pos: V3,
    pub rot: Q,
    /// Index of the parent in the same list (the root has none).
    pub parent: Option<usize>,
}

/// Every joint of `figure`, parents before children (depth first), bounded by
/// [`MAX_DEPTH`] and [`MAX_JOINTS`].
pub fn pose(figure: &Figure) -> Vec<Posed> {
    let mut out: Vec<Posed> = Vec::new();
    let mut stack: Vec<(&Joint, Option<usize>, usize)> = vec![(&figure.root, None, 0)];
    while let Some((j, parent, depth)) = stack.pop() {
        if out.len() >= MAX_JOINTS {
            break;
        }
        let (ppos, prot) = parent.and_then(|p| out.get(p)).map_or(([0.0; 3], IDENTITY), |p| (p.pos, p.rot));
        let pos = add(ppos, rotate(prot, j.offset));
        let rot = qnorm(qmul(prot, qnorm(j.rotation)));
        let index = out.len();
        out.push(Posed { name: j.name.clone(), pos, rot, parent });
        if depth < MAX_DEPTH {
            stack.extend(j.children.iter().rev().map(|c| (c, Some(index), depth + 1)));
        }
    }
    out
}

/// The posed joint named `name`.
pub fn find<'a>(posed: &'a [Posed], name: &str) -> Option<&'a Posed> {
    posed.iter().find(|p| p.name == name)
}

/// World orientation of the parent of joint `name` (identity for the root).
pub fn parent_rotation(posed: &[Posed], name: &str) -> Option<Q> {
    let j = find(posed, name)?;
    Some(j.parent.and_then(|p| posed.get(p)).map_or(IDENTITY, |p| p.rot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figure3d::math::{axis_angle, len, sub};

    #[test]
    fn a_chain_turns_with_its_parents() {
        let j = |name: &str, offset: V3, rotation: Q, children: Vec<Joint>| Joint { name: name.into(), rotation, offset, children };
        let quarter = axis_angle([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2);
        let f = Figure { root: j("a", [1.0, 0.0, 0.0], quarter, vec![j("b", [2.0, 0.0, 0.0], IDENTITY, vec![j("c", [3.0, 0.0, 0.0], IDENTITY, vec![])])]), ..Default::default() };
        let p = pose(&f);
        assert_eq!(p.len(), 3);
        assert!(len(sub(find(&p, "b").unwrap().pos, [1.0, 2.0, 0.0])) < 1e-9, "b hangs off a, turned a quarter");
        assert!(len(sub(find(&p, "c").unwrap().pos, [1.0, 5.0, 0.0])) < 1e-9);
        assert_eq!(find(&p, "c").unwrap().parent, Some(1));
        let standing = pose(&prism_doc::figure::standing_figure());
        assert_eq!(standing.len(), 67);
        let head = find(&standing, "head_end_bb_").unwrap().pos[1];
        let toe = find(&standing, "lefttoebase_bb_").unwrap().pos[1];
        assert!((150.0..185.0).contains(&(head - toe)), "about 160–170 cm tall: {}", head - toe);
    }
}
