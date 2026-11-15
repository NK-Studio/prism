//! Humanoid bones of a character model, named the way Prism's figures (and CLIP STUDIO's pose
//! materials) name them, so every pose tool works on characters too. Sources: VRM 0.x
//! (`extensions.VRM.humanoid.humanBones`, a list), VRM 1.0 (`extensions.VRMC_vrm.humanoid.
//! humanBones`, a map), and Mixamo's `mixamorig:` node names for plain glTF.
use serde_json::Value;

/// Which kind of model the file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// VRM 0.x: faces −Z.
    Vrm0,
    /// VRM 1.0: faces +Z.
    Vrm1,
    /// Plain glTF (faces +Z by the glTF convention).
    Gltf,
}

/// The Prism joint name of a VRM humanoid bone (0.x and 1.0 names).
fn vrm_name(bone: &str, kind: Kind) -> Option<String> {
    let fixed = match bone {
        "hips" => "hips_bb_",
        "spine" => "spine_bb_",
        "chest" => "spine1_bb_",
        "upperChest" => "spine2_bb_",
        "neck" => "neck_bb_",
        "head" => "head_bb_",
        _ => "",
    };
    if !fixed.is_empty() {
        return Some(fixed.into());
    }
    let (side, rest) = if let Some(r) = bone.strip_prefix("left") {
        ("left", r)
    } else {
        ("right", bone.strip_prefix("right")?)
    };
    let limb = match rest {
        "Shoulder" => Some("shoulder"),
        "UpperArm" => Some("arm"),
        "LowerArm" => Some("forearm"),
        "Hand" => Some("hand"),
        "UpperLeg" => Some("upleg"),
        "LowerLeg" => Some("leg"),
        "Foot" => Some("foot"),
        "Toes" => Some("toebase"),
        _ => None,
    };
    if let Some(l) = limb {
        return Some(format!("{side}{l}_bb_"));
    }
    // Fingers: VRM 1.0's thumb is Metacarpal / Proximal / Distal, VRM 0.x's Proximal /
    // Intermediate / Distal; the other digits are Proximal / Intermediate / Distal in both.
    let digits = [("Thumb", "thumb"), ("Index", "index"), ("Middle", "middle"), ("Ring", "ring"), ("Little", "pinky")];
    let (vrm_digit, digit) = digits.iter().find(|(d, _)| rest.starts_with(d))?;
    let segment = rest.get(vrm_digit.len()..)?;
    let k = match (*vrm_digit, segment, kind) {
        ("Thumb", "Metacarpal", _) => 1,
        ("Thumb", "Proximal", Kind::Vrm1) => 2,
        (_, "Proximal", _) => 1,
        (_, "Intermediate", _) => 2,
        (_, "Distal", _) => 3,
        _ => return None,
    };
    Some(format!("{side}hand{digit}{k}_bb_"))
}

/// The Prism joint name of a Mixamo node (`mixamorig:LeftForeArm`, any `mixamorigN:` prefix).
fn mixamo_name(node: &str) -> Option<String> {
    let (_, bone) = node.split_once(':')?;
    if !node.to_ascii_lowercase().starts_with("mixamorig") {
        return None;
    }
    let fixed = match bone {
        "Hips" => "hips_bb_",
        "Spine" => "spine_bb_",
        "Spine1" => "spine1_bb_",
        "Spine2" => "spine2_bb_",
        "Neck" => "neck_bb_",
        "Head" => "head_bb_",
        _ => "",
    };
    if !fixed.is_empty() {
        return Some(fixed.into());
    }
    let (side, rest) = if let Some(r) = bone.strip_prefix("Left") {
        ("left", r)
    } else {
        ("right", bone.strip_prefix("Right")?)
    };
    let limb = match rest {
        "Shoulder" => Some("shoulder"),
        "Arm" => Some("arm"),
        "ForeArm" => Some("forearm"),
        "Hand" => Some("hand"),
        "UpLeg" => Some("upleg"),
        "Leg" => Some("leg"),
        "Foot" => Some("foot"),
        "ToeBase" => Some("toebase"),
        _ => None,
    };
    if let Some(l) = limb {
        return Some(format!("{side}{l}_bb_"));
    }
    let rest = rest.strip_prefix("Hand")?;
    let digits = [("Thumb", "thumb"), ("Index", "index"), ("Middle", "middle"), ("Ring", "ring"), ("Pinky", "pinky")];
    let (m, digit) = digits.iter().find(|(d, _)| rest.starts_with(d))?;
    let k: u32 = rest.get(m.len()..)?.parse().ok().filter(|k| (1..=3).contains(k))?;
    Some(format!("{side}hand{digit}{k}_bb_"))
}

/// The file's kind and its humanoid bones: (node index, Prism joint name).
pub(super) fn humanoid(json: &Value) -> (Kind, Vec<(usize, String)>) {
    let ext = json.get("extensions");
    let node_of = |v: &Value| v.get("node").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    if let Some(bones) = ext.and_then(|e| e.get("VRMC_vrm")).and_then(|v| v.get("humanoid")).and_then(|h| h.get("humanBones")).and_then(Value::as_object) {
        let list = bones.iter().filter_map(|(bone, v)| Some((node_of(v)?, vrm_name(bone, Kind::Vrm1)?))).collect();
        return (Kind::Vrm1, list);
    }
    if let Some(bones) = ext.and_then(|e| e.get("VRM")).and_then(|v| v.get("humanoid")).and_then(|h| h.get("humanBones")).and_then(Value::as_array) {
        let list = bones.iter().filter_map(|b| Some((node_of(b)?, vrm_name(b.get("bone")?.as_str()?, Kind::Vrm0)?))).collect();
        return (Kind::Vrm0, list);
    }
    let nodes = json.get("nodes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let list = nodes.iter().enumerate().filter_map(|(i, n)| Some((i, mixamo_name(n.get("name")?.as_str()?)?))).collect();
    (Kind::Gltf, list)
}

/// The model's title from its VRM meta, if any.
pub(super) fn title(json: &Value) -> Option<String> {
    let ext = json.get("extensions")?;
    let t = ext.get("VRMC_vrm").and_then(|v| v.get("meta")).and_then(|m| m.get("name")).or_else(|| ext.get("VRM").and_then(|v| v.get("meta")).and_then(|m| m.get("title")))?;
    t.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanoid_names_map_to_prism_joints() {
        assert_eq!(vrm_name("leftUpperArm", Kind::Vrm1).as_deref(), Some("leftarm_bb_"));
        assert_eq!(vrm_name("rightLowerLeg", Kind::Vrm0).as_deref(), Some("rightleg_bb_"));
        assert_eq!(vrm_name("upperChest", Kind::Vrm1).as_deref(), Some("spine2_bb_"));
        assert_eq!(vrm_name("leftThumbMetacarpal", Kind::Vrm1).as_deref(), Some("lefthandthumb1_bb_"));
        assert_eq!(vrm_name("leftThumbProximal", Kind::Vrm1).as_deref(), Some("lefthandthumb2_bb_"));
        assert_eq!(vrm_name("leftThumbProximal", Kind::Vrm0).as_deref(), Some("lefthandthumb1_bb_"));
        assert_eq!(vrm_name("leftThumbIntermediate", Kind::Vrm0).as_deref(), Some("lefthandthumb2_bb_"));
        assert_eq!(vrm_name("rightLittleDistal", Kind::Vrm1).as_deref(), Some("righthandpinky3_bb_"));
        assert_eq!(vrm_name("jaw", Kind::Vrm1), None);
        assert_eq!(mixamo_name("mixamorig:LeftForeArm").as_deref(), Some("leftforearm_bb_"));
        assert_eq!(mixamo_name("mixamorig1:RightHandIndex2").as_deref(), Some("righthandindex2_bb_"));
        assert_eq!(mixamo_name("mixamorig:Spine2").as_deref(), Some("spine2_bb_"));
        assert_eq!(mixamo_name("Armature:Hips"), None);
        assert_eq!(mixamo_name("mixamorig:LeftHandIndex4"), None);
    }

    #[test]
    fn vrm_versions_are_told_apart() {
        let v1 = serde_json::json!({"extensions": {"VRMC_vrm": {"meta": {"name": "Seed"}, "humanoid": {"humanBones": {"hips": {"node": 2}, "head": {"node": 9}, "eye": {"node": 4}}}}}});
        let (k, mut list) = humanoid(&v1);
        list.sort();
        assert_eq!((k, list), (Kind::Vrm1, vec![(2, "hips_bb_".to_string()), (9, "head_bb_".to_string())]));
        assert_eq!(title(&v1).as_deref(), Some("Seed"));
        let v0 = serde_json::json!({"extensions": {"VRM": {"meta": {"title": " Alicia "}, "humanoid": {"humanBones": [{"bone": "leftHand", "node": 5}, {"bone": "x"}]}}}});
        assert_eq!(humanoid(&v0), (Kind::Vrm0, vec![(5, "lefthand_bb_".to_string())]));
        assert_eq!(title(&v0).as_deref(), Some("Alicia"));
        let plain = serde_json::json!({"nodes": [{"name": "mixamorig:Hips"}, {"name": "Cube"}]});
        assert_eq!(humanoid(&plain), (Kind::Gltf, vec![(0, "hips_bb_".to_string())]));
    }
}
