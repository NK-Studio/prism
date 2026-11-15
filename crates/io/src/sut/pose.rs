//! CLIP STUDIO pose materials (`data/material_0.pose` of a `pose` material): a C2F container
//! holding a SQLite database whose `File` row carries an XML `.pep`:
//! `<celsysdocument><pose version="…"><node name="root_group_" rotquat="w x y z" trans="x y z">`
//! with the figure's joints nested inside. `trans` is the joint's position in its parent's frame
//! (centimetres, Y up) and `rotquat` its local rotation, so the file holds both the pose and the
//! bone lengths. The material name is `Material.MaterialName`. Joints of the right arm and leg
//! below the shoulder and thigh store their offsets with `y` and `z` mirrored (CLIP STUDIO's own
//! symmetric default pose is only symmetric read that way), so they are flipped back here and
//! Prism's skeleton stays plain forward kinematics. Evidence: docs/sut-analysis.md › Pose materials.
use super::sqlite::{Database, Value};
use prism_doc::figure::{Figure, Joint, MAX_DEPTH, MAX_JOINTS, MAX_NAME, normalize_quat};

type Result<T> = std::result::Result<T, String>;

/// Largest `.pep` read (a 67-joint pose is about 17 KB).
const PEP_LIMIT: usize = 1 << 20;

/// A pose file's content: a whole figure, or one hand's fingers.
#[derive(Clone, Debug, PartialEq)]
pub enum Pose {
    Body(Figure),
    Hand(HandPose),
}

/// A hand pose (CLIP STUDIO's hand pose materials and its own `handsetup_*.pep`): the finger
/// chains of one hand, nested under that hand's joint (`lefthand_bb_` / `righthand_bb_`, made up
/// when the file starts at the fingers). It poses the fingers of a figure's hand; its bone
/// lengths only tell which way the fingers lie.
#[derive(Clone, Debug, PartialEq)]
pub struct HandPose {
    pub name: String,
    /// The hand it was made for.
    pub left: bool,
    pub hand: Joint,
}

/// Read a pose material that holds a whole figure ([`read_any`] reads hand poses too).
pub fn read(bytes: &[u8], fallback_name: &str, ctl: &prism_raster::Interrupt) -> Result<Figure> {
    match read_any(bytes, fallback_name, ctl)? {
        Pose::Body(f) => Ok(f),
        Pose::Hand(_) => Err("this is a hand pose: apply it to a 3D figure's hands (layer.figure.applyHandPose)".into()),
    }
}

/// Read a pose material (`.pose`, plain SQLite or in its C2F envelope) or a bare `.pep` XML file:
/// a whole figure or a hand. `fallback_name` names it when the file carries no name.
pub fn read_any(bytes: &[u8], fallback_name: &str, ctl: &prism_raster::Interrupt) -> Result<Pose> {
    ctl.check().map_err(|e| e.to_string())?;
    if bytes.len() > 64 << 20 {
        return Err("CLIP STUDIO pose exceeds 64 MiB".into());
    }
    let decoded;
    let plain = if bytes.starts_with(crate::csp_container::MAGIC) {
        decoded = crate::csp_container::decode(bytes, ctl)?;
        &decoded[..]
    } else {
        bytes
    };
    let (xml, name) = if plain.starts_with(b"SQLite format 3\0") {
        let db = Database::new(plain)?;
        let name = db.table("Material").ok().and_then(|rows| rows.first().and_then(|r| r.get("MaterialName").and_then(Value::text).map(str::to_string)));
        let files = db.table("File").map_err(|e| format!("not a CLIP STUDIO pose ({e})"))?;
        let data = files.iter().find_map(|r| r.get("FileData").and_then(Value::blob)).ok_or("the CLIP STUDIO pose holds no pose data")?;
        (data.to_vec(), name)
    } else {
        (plain.to_vec(), None)
    };
    let mut pose = read_pep_any(&xml)?;
    let name = clean(name.as_deref().unwrap_or(""), fallback_name);
    match &mut pose {
        Pose::Body(f) => f.name = name,
        Pose::Hand(h) => h.name = name,
    }
    ctl.check().map_err(|e| e.to_string())?;
    Ok(pose)
}

fn clean(s: &str, fallback: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let s = s.trim();
    if s.is_empty() { fallback.chars().take(MAX_NAME).collect() } else { s.to_string() }
}

fn numbers<const N: usize>(node: roxmltree::Node, attr: &str) -> Result<[f64; N]> {
    let text = node.attribute(attr).ok_or_else(|| format!("pose joint without `{attr}`"))?;
    let mut out = [0.0; N];
    let mut parts = text.split_ascii_whitespace();
    for v in &mut out {
        *v = parts.next().and_then(|p| p.parse::<f64>().ok()).filter(|v| v.is_finite() && v.abs() <= 1e6).ok_or_else(|| format!("pose joint `{attr}` is not {N} numbers"))?;
    }
    if parts.next().is_some() {
        return Err(format!("pose joint `{attr}` is not {N} numbers"));
    }
    Ok(out)
}

/// A `.pep` document holding a whole figure: its `<pose>` element's single top `<node>` tree.
pub fn read_pep(bytes: &[u8]) -> Result<Figure> {
    match read_pep_any(bytes)? {
        Pose::Body(f) => Ok(f),
        Pose::Hand(_) => Err("this is a hand pose, not a whole figure".into()),
    }
}

/// `Some(left)` when `name` is a joint of one hand: the hand itself or one of its fingers.
fn hand_joint(name: &str) -> Option<bool> {
    let n = name.to_ascii_lowercase();
    let (left, rest) = if let Some(r) = n.strip_prefix("lefthand") { (true, r.to_string()) } else { (false, n.strip_prefix("righthand")?.to_string()) };
    (rest == "_bb_" || ["thumb", "index", "middle", "ring", "pinky"].iter().any(|d| rest.starts_with(d))).then_some(left)
}

/// A `.pep` document: one top `<node>` tree (a figure), or the top nodes of one hand (a hand pose:
/// its finger chains, or the hand joint itself).
pub fn read_pep_any(bytes: &[u8]) -> Result<Pose> {
    if bytes.len() > PEP_LIMIT {
        return Err("CLIP STUDIO pose data exceeds 1 MiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "CLIP STUDIO pose data is not UTF-8 text")?;
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("CLIP STUDIO pose data is not readable XML ({e})"))?;
    let pose = doc.descendants().find(|n| n.has_tag_name("pose")).ok_or("CLIP STUDIO pose data has no <pose>")?;
    let tops: Vec<roxmltree::Node> = pose.children().filter(|n| n.has_tag_name("node")).collect();
    if tops.is_empty() {
        return Err("CLIP STUDIO pose has no joints".into());
    }
    let mut count = 0usize;
    let sides: Vec<Option<bool>> = tops.iter().map(|n| hand_joint(n.attribute("name").unwrap_or(""))).collect();
    let hand = match sides.first() {
        Some(Some(left)) if sides.iter().all(|s| *s == Some(*left)) => Some(*left),
        _ => None,
    };
    let Some(left) = hand else {
        let [top] = tops.as_slice() else { return Err("CLIP STUDIO pose has more than one root joint".into()) };
        let root = joint(*top, 0, &mut count)?;
        return Ok(Pose::Body(Figure { name: String::new(), source: None, root }));
    };
    let name = format!("{}hand_bb_", if left { "left" } else { "right" });
    let mut hand = Joint { name: name.clone(), rotation: [1.0, 0.0, 0.0, 0.0], offset: [0.0; 3], children: Vec::new() };
    for top in tops {
        let j = joint(top, 1, &mut count)?;
        if j.name.eq_ignore_ascii_case(&name) {
            // The hand joint itself: its fingers join the others (its own turn is the wrist's).
            hand.children.extend(j.children);
        } else {
            hand.children.push(j);
        }
    }
    if hand.children.is_empty() {
        return Err("CLIP STUDIO hand pose has no fingers".into());
    }
    Ok(Pose::Hand(HandPose { name: String::new(), left, hand }))
}

/// A right-side joint hanging off another right-side joint (everything of the right arm and leg
/// but the shoulder and thigh, which hang off the torso): its stored offset has `y` and `z`
/// mirrored.
fn mirrored_side(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("right") && n != "rightshoulder_bb_" && n != "rightupleg_bb_"
}

fn joint(node: roxmltree::Node, depth: usize, count: &mut usize) -> Result<Joint> {
    *count += 1;
    if *count > MAX_JOINTS {
        return Err(format!("CLIP STUDIO pose has more than {MAX_JOINTS} joints"));
    }
    if depth > MAX_DEPTH {
        return Err(format!("CLIP STUDIO pose joints nest deeper than {MAX_DEPTH}"));
    }
    let name: String = node.attribute("name").unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let rotation = normalize_quat(numbers::<4>(node, "rotquat")?);
    let mut offset = numbers::<3>(node, "trans")?;
    if mirrored_side(&name) {
        offset = [offset[0], -offset[1], -offset[2]];
    }
    let mut children = Vec::new();
    for c in node.children().filter(|n| n.has_tag_name("node")) {
        children.push(joint(c, depth + 1, count)?);
    }
    Ok(Joint { name, rotation, offset, children })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const PEP: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<celsysdocument version="1.0.0">
  <info author="" comment="" tool="test"/>
  <pose alias_name="" bone_mapping_version="0" search_tags="" version="1.0.14">
    <node id="-1" name="root_group_" rotquat="1 0 0 0" trans="0 0 0">
      <node id="-1" name="hips_bb_" rotquat="0.70710678 0 0 0.70710678" trans="0 96 0">
        <node id="-1" name="spine_bb_" rotquat="1 0 0 0" trans="10 0 0"/>
      </node>
    </node>
  </pose>
</celsysdocument>"#;

    #[test]
    fn a_pep_becomes_a_joint_tree() {
        let f = read_pep(PEP.as_bytes()).unwrap();
        assert_eq!(f.root.name, "root_group_");
        let hips = f.root.find("hips_bb_").unwrap();
        assert_eq!(hips.offset, [0.0, 96.0, 0.0]);
        assert!((hips.rotation[0] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-6, "normalised");
        assert_eq!(f.root.find("spine_bb_").unwrap().offset, [10.0, 0.0, 0.0]);
        let named = read(PEP.as_bytes(), "Pose 1", &prism_raster::Interrupt::NONE).unwrap();
        assert_eq!(named.name, "Pose 1", "a bare .pep takes the fallback name");
    }

    /// CLIP STUDIO's layout, synthetic numbers: the right shoulder's frame is the left one turned
    /// half a turn about the body's side axis, the right side's local rotations equal the left
    /// side's, and its stored offsets mirror only `x`. Read back, the figure is symmetric.
    #[test]
    fn the_right_side_reads_as_the_mirror_of_the_left() {
        let side = |s: &str, x: f64, q: &str| {
            format!(
                r#"<node name="{s}shoulder_bb_" rotquat="{q}" trans="{} 10 0">
                     <node name="{s}hand_bb_" rotquat="0.9 0.1 -0.3 0.2" trans="{} 0 0">
                       <node name="{s}handthumb1_bb_" rotquat="0.8 0.4 -0.2 0.3" trans="{} -1 2">
                         <node name="{s}handthumb2_bb_" rotquat="0.95 0 0 -0.3" trans="{} 0 0"/>
                       </node>
                     </node>
                   </node>"#,
                2.0 * x,
                20.0 * x,
                1.5 * x,
                3.0 * x
            )
        };
        let xml = format!(r#"<pose><node name="hips_bb_" rotquat="1 0 0 0" trans="0 90 0">{}{}</node></pose>"#, side("left", 1.0, "1 0 0 0"), side("right", -1.0, "0 1 0 0"));
        let f = read_pep(xml.as_bytes()).unwrap();
        assert_eq!(f.root.find("rightshoulder_bb_").unwrap().offset, [-2.0, 10.0, 0.0], "the shoulder hangs off the torso: as stored");
        assert_eq!(f.root.find("righthandthumb1_bb_").unwrap().offset, [-1.5, 1.0, -2.0]);
        let posed = prism_algo::figure3d::skeleton::pose(&f);
        let at = |n: &str| prism_algo::figure3d::skeleton::find(&posed, n).unwrap().pos;
        for j in ["shoulder_bb_", "hand_bb_", "handthumb1_bb_", "handthumb2_bb_"] {
            let (l, r) = (at(&format!("left{j}")), at(&format!("right{j}")));
            assert!((l[0] + r[0]).abs() < 1e-9 && (l[1] - r[1]).abs() < 1e-9 && (l[2] - r[2]).abs() < 1e-9, "{j}: {l:?} vs {r:?}");
        }
    }

    /// The layout of CLIP STUDIO's hand poses (its `handsetup_*.pep`; synthetic numbers): the
    /// fingers' chains are the top nodes.
    #[test]
    fn hand_poses_read_as_one_hand() {
        let xml = r#"<celsysdocument><pose alias_name="Grip" search_tags="hand_pose" version="1.0.0">
            <node id="-1" name="righthandmiddle1_bb_" rotquat="0.7 0 0 -0.7" trans="-6.6 0 0"><node id="-1" name="righthandmiddle2_bb_" rotquat="0.6 0 0 -0.8" trans="-3.8 0 0"/></node>
            <node id="-1" name="righthandthumb1_bb_" rotquat="1 0 0 0" trans="-1.3 -1.1 1.9"/>
        </pose></celsysdocument>"#;
        let Pose::Hand(h) = read_any(xml.as_bytes(), "Grip pose", &prism_raster::Interrupt::NONE).unwrap() else { panic!("not a hand pose") };
        assert!(!h.left);
        assert_eq!(h.name, "Grip pose");
        assert_eq!(h.hand.name, "righthand_bb_");
        assert_eq!(h.hand.children.len(), 2);
        // Right-side offsets come back unmirrored, as in whole figures.
        assert_eq!(h.hand.find("righthandthumb1_bb_").unwrap().offset, [-1.3, 1.1, -1.9]);
        assert!(read(xml.as_bytes(), "x", &prism_raster::Interrupt::NONE).is_err(), "not a whole figure");
        assert!(read_pep(xml.as_bytes()).is_err());
        // The hand joint itself as the top node.
        let with_hand = r#"<pose><node name="lefthand_bb_" rotquat="0.9 0.1 0 0" trans="20 0 0"><node name="lefthandindex1_bb_" rotquat="1 0 0 0" trans="6 0 1"/></node></pose>"#;
        let Pose::Hand(h) = read_pep_any(with_hand.as_bytes()).unwrap() else { panic!("not a hand pose") };
        assert!(h.left && h.hand.rotation == [1.0, 0.0, 0.0, 0.0] && h.hand.children.len() == 1);
        assert!(matches!(read_pep_any(PEP.as_bytes()).unwrap(), Pose::Body(_)));
    }

    #[test]
    fn malformed_poses_are_errors_never_panics() {
        let bad = [
            "",
            "<pose/>",
            "not xml",
            r#"<pose><node name="a" rotquat="1 0 0" trans="0 0 0"/></pose>"#,
            r#"<pose><node name="a" rotquat="1 0 0 0" trans="0 0 x"/></pose>"#,
            r#"<pose><node name="a" rotquat="1 0 0 0" trans="0 0 1e400"/></pose>"#,
            r#"<pose><node name="a" rotquat="1 0 0 0 5" trans="0 0 0"/></pose>"#,
            r#"<pose><node name="a" rotquat="1 0 0 0" trans="0 0 0"/><node name="b" rotquat="1 0 0 0" trans="0 0 0"/></pose>"#,
            // Two hands at once, a hand and something else, or a hand without fingers.
            r#"<pose><node name="lefthandindex1_bb_" rotquat="1 0 0 0" trans="0 0 0"/><node name="righthandindex1_bb_" rotquat="1 0 0 0" trans="0 0 0"/></pose>"#,
            r#"<pose><node name="lefthandindex1_bb_" rotquat="1 0 0 0" trans="0 0 0"/><node name="head_bb_" rotquat="1 0 0 0" trans="0 0 0"/></pose>"#,
            r#"<pose><node name="lefthand_bb_" rotquat="1 0 0 0" trans="0 0 0"/></pose>"#,
        ];
        for b in bad {
            assert!(read_pep(b.as_bytes()).is_err(), "{b}");
        }
        // Too deep, and too many joints.
        let deep = format!("<pose>{}{}</pose>", r#"<node name="j" rotquat="1 0 0 0" trans="0 1 0">"#.repeat(MAX_DEPTH + 3), "</node>".repeat(MAX_DEPTH + 3));
        assert!(read_pep(deep.as_bytes()).is_err());
        let wide = format!(r#"<pose><node name="r" rotquat="1 0 0 0" trans="0 0 0">{}</node></pose>"#, r#"<node name="c" rotquat="1 0 0 0" trans="0 1 0"/>"#.repeat(MAX_JOINTS + 1));
        assert!(read_pep(wide.as_bytes()).is_err());
        // A zero quaternion becomes the identity rather than an error.
        let zero = r#"<pose><node name="a" rotquat="0 0 0 0" trans="0 0 0"/></pose>"#;
        assert_eq!(read_pep(zero.as_bytes()).unwrap().root.rotation, [1.0, 0.0, 0.0, 0.0]);
        for b in [&b"SQLite format 3\0junk"[..], b"\x89C2F\r\n\x1a\nbroken"] {
            assert!(read(b, "x", &prism_raster::Interrupt::NONE).is_err());
        }
    }
}
