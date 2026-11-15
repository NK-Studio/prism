use super::*;
use crate::figure3d::math::{qconj, rotate};
use crate::figure3d::skeleton::pose;
use prism_doc::figure::standing_figure;

fn setup(weights: [f64; 3], closed: Closed) -> HandSetup {
    HandSetup { weights, closed, locked: [false; 5] }
}

fn at(f: &Figure, name: &str) -> V3 {
    pose(f).into_iter().find(|p| p.name == name).unwrap().pos
}

/// A joint's position in its hand's frame.
fn in_hand(f: &Figure, side: Side, name: &str) -> V3 {
    let posed = pose(f);
    let hand = posed.iter().find(|p| p.name == format!("{}hand_bb_", side.prefix())).unwrap();
    let j = posed.iter().find(|p| p.name == name).unwrap();
    rotate(qconj(hand.rot), sub(j.pos, hand.pos))
}

/// How far `digit`'s tip has come towards the palm, and how far it still points along the hand.
fn closing(f: &Figure, side: Side, digit: &str) -> (f64, f64) {
    let p = palm(&f.root, side).unwrap();
    let knuckle = in_hand(f, side, &format!("{}hand{digit}1_bb_", side.prefix()));
    let tip = in_hand(f, side, &format!("{}hand{digit}4_bb_", side.prefix()));
    let along = normalize_or(f.root.find(&format!("{}hand_bb_", side.prefix())).unwrap().children.iter().find(|c| c.name.contains("middle")).unwrap().offset, [0.0; 3]);
    let d = sub(tip, knuckle);
    (dot(d, p.normal), dot(normalize_or(d, [0.0; 3]), along))
}

#[test]
fn the_palm_faces_the_thumb_side() {
    let f = standing_figure();
    for side in [Side::Left, Side::Right] {
        let p = palm(&f.root, side).unwrap();
        let thumb = f.root.find(&format!("{}handthumb1_bb_", side.prefix())).unwrap().offset;
        assert!(dot(thumb, p.normal) > 0.0, "{side:?}");
        assert!((len(p.normal) - 1.0).abs() < 1e-9 && dot(p.normal, p.across).abs() < 1e-9);
    }
}

#[test]
fn a_fist_closes_every_finger_towards_the_palm() {
    let mut f = standing_figure();
    for side in [Side::Left, Side::Right] {
        set_hand(&mut f, side, &setup([0.0, 0.0, 1.0], Closed::Fist)).unwrap();
        for digit in ["index", "middle", "ring", "pinky"] {
            let (towards_palm, along) = closing(&f, side, digit);
            assert!(towards_palm > 1.0, "{side:?} {digit}: the tip comes over the palm ({towards_palm})");
            assert!(along < 0.0, "{side:?} {digit}: the tip curls back past the knuckle ({along})");
        }
    }
    // Flat: straight out along the hand.
    set_hand(&mut f, Side::Left, &setup([1.0, 0.0, 0.0], Closed::Fist)).unwrap();
    for digit in ["index", "middle", "ring", "pinky"] {
        let (towards_palm, along) = closing(&f, Side::Left, digit);
        assert!(along > 0.95 && towards_palm.abs() < 1.0, "{digit}: {along} {towards_palm}");
    }
}

#[test]
fn spreading_fans_the_fingers_out() {
    let fan = |w: [f64; 3]| {
        let mut f = standing_figure();
        set_hand(&mut f, Side::Left, &setup(w, Closed::Fist)).unwrap();
        let tip = |d: &str| in_hand(&f, Side::Left, &format!("lefthand{d}4_bb_"));
        len(sub(tip("index"), tip("pinky")))
    };
    let (flat, spread) = (fan([1.0, 0.0, 0.0]), fan([0.0, 1.0, 0.0]));
    assert!(spread > flat * 1.6, "spread {spread} vs flat {flat}");
    // Halfway lies between.
    let half = fan([0.5, 0.5, 0.0]);
    assert!(flat < half && half < spread);
}

#[test]
fn both_hands_mirror_each_other() {
    let mut f = standing_figure();
    for closed in Closed::ALL {
        for side in [Side::Left, Side::Right] {
            set_hand(&mut f, side, &setup([0.2, 0.3, 0.5], closed)).unwrap();
        }
        for digit in DIGITS {
            for k in 1..=4 {
                let (l, r) = (at(&f, &format!("lefthand{digit}{k}_bb_")), at(&f, &format!("righthand{digit}{k}_bb_")));
                assert!((l[0] + r[0]).abs() < 1e-6 && (l[1] - r[1]).abs() < 1e-6 && (l[2] - r[2]).abs() < 1e-6, "{closed:?} {digit}{k}: {l:?} vs {r:?}");
            }
        }
    }
}

#[test]
fn locked_fingers_keep_their_pose() {
    let mut f = standing_figure();
    set_hand(&mut f, Side::Left, &setup([0.0, 0.0, 1.0], Closed::Tube)).unwrap();
    let before: Vec<Q> = (1..=3).map(|k| f.root.find(&format!("lefthandthumb{k}_bb_")).unwrap().rotation).collect();
    let index_before = f.root.find("lefthandindex2_bb_").unwrap().rotation;
    let mut s = setup([0.0, 1.0, 0.0], Closed::Fist);
    s.locked[0] = true;
    set_hand(&mut f, Side::Left, &s).unwrap();
    let after: Vec<Q> = (1..=3).map(|k| f.root.find(&format!("lefthandthumb{k}_bb_")).unwrap().rotation).collect();
    assert_eq!(before, after, "the thumb is locked");
    assert_ne!(f.root.find("lefthandindex2_bb_").unwrap().rotation, index_before, "the index is not");
    // The other hand is untouched.
    assert_eq!(f.root.find("righthandindex2_bb_").unwrap().rotation, [1.0, 0.0, 0.0, 0.0]);
}

/// A CLIP STUDIO-style left hand (synthetic numbers): fingers along local +X, the knuckle line
/// along Z, the thumb's base on −Y. Its own hand setup poses curl the fingers about local −Z.
#[test]
fn csp_style_fingers_curl_about_negative_z() {
    let j = |name: &str, offset: V3, children: Vec<Joint>| Joint { name: name.into(), rotation: [1.0, 0.0, 0.0, 0.0], offset, children };
    let finger = |d: &str, base: V3| j(&format!("lefthand{d}1_bb_"), base, vec![j(&format!("lefthand{d}2_bb_"), [3.5, 0.0, 0.0], vec![j(&format!("lefthand{d}3_bb_"), [2.2, 0.0, 0.0], vec![j(&format!("lefthand{d}4_bb_"), [2.0, 0.0, 0.0], vec![])])])]);
    let hand = j(
        "lefthand_bb_",
        [0.0, 0.0, 0.0],
        vec![finger("thumb", [1.3, -1.1, 1.9]), finger("index", [6.6, 0.0, 1.5]), finger("middle", [6.6, 0.0, 0.0]), finger("ring", [6.3, 0.0, -1.5]), finger("pinky", [5.8, -0.4, -2.8])],
    );
    let mut f = Figure { root: hand, ..Default::default() };
    set_hand(&mut f, Side::Left, &setup([0.0, 0.0, 1.0], Closed::Fist)).unwrap();
    let q = f.root.find("lefthandmiddle2_bb_").unwrap().rotation;
    let axis = normalize_or([q[1], q[2], q[3]], [0.0; 3]);
    assert!(axis[2] < -0.99, "about −Z: {axis:?}");
    assert!(q[0] < 0.75, "bent well over 80°: {q:?}");
    // The tips come round to the palm side (−Y).
    let tip = at(&f, "lefthandmiddle4_bb_");
    let knuckle = at(&f, "lefthandmiddle1_bb_");
    let d = sub(tip, knuckle);
    assert!(d[1] < -1.0 && d[0] < 0.0, "curled back under the knuckle: {d:?}");
}

#[test]
fn bad_setups_are_errors() {
    let mut f = standing_figure();
    for w in [[0.0, 0.0, 0.0], [-1.0, 1.0, 0.0], [f64::NAN, 1.0, 0.0], [f64::INFINITY, 0.0, 0.0]] {
        assert!(set_hand(&mut f, Side::Left, &setup(w, Closed::Fist)).is_err(), "{w:?}");
    }
    let mut empty = Figure::default();
    assert!(set_hand(&mut empty, Side::Right, &setup([1.0, 0.0, 0.0], Closed::Fist)).is_err());
    assert_eq!(Closed::from_id("CATPAW"), Some(Closed::CatPaw));
    assert_eq!(Closed::from_id("paw"), None);
    assert!(tip_direction(&f, Side::Left, "index").is_some());
}
