use super::*;
use crate::figure3d::ik::rotate_joint;
use crate::figure3d::math::{IDENTITY, axis_angle};
use prism_doc::figure::standing_figure;

fn at(f: &Figure, name: &str) -> V3 {
    pose(f).into_iter().find(|p| p.name == name).unwrap().pos
}

fn dir(f: &Figure, a: &str, b: &str) -> V3 {
    normalize_or(sub(at(f, b), at(f, a)), [0.0; 3])
}

/// A pose: left arm raised, right forearm bent, a knee bent, the trunk leaning.
fn posed() -> Figure {
    let mut f = standing_figure();
    rotate_joint(&mut f, "leftarm_bb_", axis_angle([0.0, 0.0, 1.0], 1.1)).unwrap();
    rotate_joint(&mut f, "rightforearm_bb_", axis_angle([1.0, 0.0, 0.0], -1.2)).unwrap();
    rotate_joint(&mut f, "leftleg_bb_", axis_angle([1.0, 0.0, 0.0], 0.9)).unwrap();
    rotate_joint(&mut f, "spine_bb_", axis_angle([1.0, 0.0, 0.0], 0.25)).unwrap();
    f
}

/// The same skeleton with every bone 30% longer (a different body).
fn longer(mut j: Joint) -> Joint {
    j.offset = scale(j.offset, 1.3);
    j.children = j.children.into_iter().map(longer).collect();
    j
}

const BONES: [(&str, &str); 6] = [
    ("leftarm_bb_", "leftforearm_bb_"),
    ("leftforearm_bb_", "lefthand_bb_"),
    ("rightforearm_bb_", "righthand_bb_"),
    ("leftupleg_bb_", "leftleg_bb_"),
    ("leftleg_bb_", "leftfoot_bb_"),
    ("spine1_bb_", "spine2_bb_"),
];

#[test]
fn a_pose_carries_its_bone_directions_onto_another_body() {
    let source = posed();
    let rest = longer(standing_figure().root);
    let mut target = Figure { root: rest.clone(), ..Default::default() };
    retarget(&source, &mut target, &rest).unwrap();
    for (a, b) in BONES {
        let (s, t) = (dir(&source, a, b), dir(&target, a, b));
        assert!(dot(s, t) > 0.999, "{a}→{b}: {s:?} vs {t:?}");
    }
    // Its own proportions stay.
    let (s, t) = (len(sub(at(&source, "lefthand_bb_"), at(&source, "leftforearm_bb_"))), len(sub(at(&target, "lefthand_bb_"), at(&target, "leftforearm_bb_"))));
    assert!((t / s - 1.3).abs() < 1e-6);
}

#[test]
fn the_character_keeps_its_place_and_facing() {
    let source = posed();
    let rest = standing_figure().root;
    let mut target = Figure { root: rest.clone(), ..Default::default() };
    let turn = axis_angle([0.0, 1.0, 0.0], std::f64::consts::FRAC_PI_2);
    target.root.offset = [100.0, 0.0, 50.0];
    target.root.rotation = turn;
    // The pose's own placement is ignored.
    let mut placed = source.clone();
    placed.root.offset = [-300.0, 0.0, 0.0];
    placed.root.rotation = axis_angle([0.0, 1.0, 0.0], 2.0);
    retarget(&placed, &mut target, &rest).unwrap();
    assert_eq!((target.root.offset, target.root.rotation), ([100.0, 0.0, 50.0], turn));
    for (a, b) in BONES {
        let (s, t) = (rotate(turn, dir(&source, a, b)), dir(&target, a, b));
        assert!(dot(s, t) > 0.999, "{a}→{b} turned with the character: {s:?} vs {t:?}");
    }
}

#[test]
fn hips_and_hands_take_their_twist() {
    // Twist the source's hips about the vertical and its left hand about its bone.
    let mut source = standing_figure();
    rotate_joint(&mut source, "hips_bb_", axis_angle([0.0, 1.0, 0.0], 0.6)).unwrap();
    let hand_axis = dir(&source, "lefthand_bb_", "lefthandmiddle1_bb_");
    rotate_joint(&mut source, "lefthand_bb_", axis_angle(hand_axis, 0.8)).unwrap();
    let rest = standing_figure().root;
    let mut target = Figure { root: rest.clone(), ..Default::default() };
    retarget(&source, &mut target, &rest).unwrap();
    let hips_line = |f: &Figure| normalize_or(sub(at(f, "leftupleg_bb_"), at(f, "rightupleg_bb_")), [0.0; 3]);
    assert!(dot(hips_line(&source), hips_line(&target)) > 0.999, "the hips turn with the pose");
    let knuckles = |f: &Figure| normalize_or(sub(at(f, "lefthandpinky1_bb_"), at(f, "lefthandindex1_bb_")), [0.0; 3]);
    assert!(dot(knuckles(&source), knuckles(&target)) > 0.99, "the hand turns about itself too");
}

#[test]
fn poses_and_characters_without_humanoid_bones_are_refused() {
    let rest = standing_figure().root;
    let mut target = Figure { root: rest.clone(), ..Default::default() };
    let bare = Figure { root: Joint { name: "x".into(), rotation: IDENTITY, offset: [0.0; 3], children: vec![] }, ..Default::default() };
    assert!(retarget(&bare, &mut target, &rest).is_err());
    let mut plain = Figure { root: bare.root.clone(), ..Default::default() };
    assert!(retarget(&standing_figure(), &mut plain, &bare.root).is_err());
}
