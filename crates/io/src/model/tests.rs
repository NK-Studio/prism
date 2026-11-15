use super::*;
use prism_algo::figure3d::mesh::skin;
use prism_doc::figure::Figure;
use prism_raster::Interrupt;
use serde_json::json;

/// A small GLB writer for synthetic test models.
struct Glb {
    json: Value,
    bin: Vec<u8>,
}

impl Glb {
    fn new() -> Glb {
        Glb { json: json!({"asset": {"version": "2.0"}, "buffers": [{}], "bufferViews": [], "accessors": []}), bin: Vec::new() }
    }

    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let views = self.json["bufferViews"].as_array_mut().unwrap();
        views.push(json!({"buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len()}));
        self.bin.extend_from_slice(bytes);
        views.len() - 1
    }

    fn floats(&mut self, data: &[f32], kind: &str) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let comps = match kind {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => 16,
        };
        let view = self.view(&bytes);
        let acc = self.json["accessors"].as_array_mut().unwrap();
        acc.push(json!({"bufferView": view, "componentType": 5126, "count": data.len() / comps, "type": kind}));
        acc.len() - 1
    }

    fn ints(&mut self, data: &[u16], kind: &str, count: usize) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes);
        let acc = self.json["accessors"].as_array_mut().unwrap();
        acc.push(json!({"bufferView": view, "componentType": 5123, "count": count, "type": kind}));
        acc.len() - 1
    }

    fn bytes(mut self) -> Vec<u8> {
        self.json["buffers"][0]["byteLength"] = json!(self.bin.len());
        let mut text = serde_json::to_vec(&self.json).unwrap();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let total = 12 + 8 + text.len() + 8 + self.bin.len();
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((text.len() as u32).to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(text);
        out.extend((self.bin.len() as u32).to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes());
        out.extend(self.bin);
        out
    }
}

fn png_1x1(rgba: [u8; 4]) -> Vec<u8> {
    let im = prism_codecs::Image::from_raw(1, 1, prism_codecs::ChannelLayout::Rgba, prism_codecs::SampleType::U8, rgba.to_vec()).unwrap();
    prism_codecs::encode(&im, prism_codecs::Format::Png, &prism_codecs::EncodeOptions::default()).unwrap()
}

/// An armature scaled down 100× holding hips (1 m up) and chest (50 cm above it); a 20 cm wide
/// quad from the hips to the chest skinned half to each; a red texture; a VRM humanoid map.
fn character(vrm: &str) -> Vec<u8> {
    let mut g = Glb::new();
    let pos = g.floats(&[-0.1, 1.0, 0.0, 0.1, 1.0, 0.0, -0.1, 1.5, 0.0, 0.1, 1.5, 0.0], "VEC3");
    let nrm = g.floats(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0], "VEC3");
    let uv = g.floats(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0], "VEC2");
    let joints = g.ints(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0], "VEC4", 4);
    let weights = g.floats(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], "VEC4");
    let idx = g.ints(&[0, 1, 3, 0, 3, 2], "SCALAR", 6);
    // Inverse bind matrices (glTF space, the armature's scale included): undo each joint's
    // rest translation (100 and 150 armature units) and the armature's 0.01 scale.
    let ibm = g.floats(&[100.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, -100.0, 0.0, 1.0, 100.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, -150.0, 0.0, 1.0], "MAT4");
    let png = png_1x1([255, 0, 0, 255]);
    let img_view = g.view(&png);
    g.json["nodes"] = json!([
        {"name": "Armature", "scale": [0.01, 0.01, 0.01], "children": [1, 3]},
        {"name": "J_Hips", "translation": [0.0, 100.0, 0.0], "children": [2]},
        {"name": "J_Chest", "translation": [0.0, 50.0, 0.0]},
        {"name": "Body", "mesh": 0, "skin": 0}
    ]);
    g.json["scenes"] = json!([{"nodes": [0]}]);
    g.json["skins"] = json!([{"joints": [1, 2], "inverseBindMatrices": ibm}]);
    g.json["meshes"] = json!([{"primitives": [{"attributes": {"POSITION": pos, "NORMAL": nrm, "TEXCOORD_0": uv, "JOINTS_0": joints, "WEIGHTS_0": weights}, "indices": idx, "material": 0}]}]);
    g.json["materials"] = json!([{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "baseColorFactor": [1.0, 1.0, 1.0, 1.0]}}]);
    g.json["textures"] = json!([{"source": 0}]);
    g.json["images"] = json!([{"bufferView": img_view, "mimeType": "image/png"}]);
    g.json["extensions"] = match vrm {
        "1" => json!({"VRMC_vrm": {"meta": {"name": "Tester"}, "humanoid": {"humanBones": {"hips": {"node": 1}, "chest": {"node": 2}}}}}),
        "0" => json!({"VRM": {"meta": {"title": "Old Tester"}, "humanoid": {"humanBones": [{"bone": "hips", "node": 1}, {"bone": "chest", "node": 2}]}}}),
        _ => json!({}),
    };
    g.bytes()
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn a_vrm_1_character_reads_in_centimetres_with_prism_bone_names() {
    let f = read(&character("1"), "file", None, &Interrupt::NONE).unwrap();
    assert_eq!((f.name.as_str(), f.kind, f.humanoid), ("Tester", Kind::Vrm1, 2));
    let m = &f.model;
    assert_eq!(m.rest.name, "root_group_");
    let armature = &m.rest.children[0];
    assert_eq!(armature.name, "node:Armature");
    let hips = m.rest.find("hips_bb_").unwrap();
    let chest = m.rest.find("spine1_bb_").unwrap();
    // The armature's 0.01 scale is baked out: the bones sit 100 and 150 cm up.
    assert!(close(hips.offset, [0.0, 100.0, 0.0]), "{:?}", hips.offset);
    assert!(close(chest.offset, [0.0, 50.0, 0.0]), "{:?}", chest.offset);
    assert!(close(m.positions[0], [-10.0, 100.0, 0.0]) && close(m.positions[3], [10.0, 150.0, 0.0]), "{:?}", m.positions);
    assert_eq!(m.triangles, vec![[0, 1, 3], [0, 3, 2]]);
    assert_eq!(m.textures[0].rgba, vec![255, 0, 0, 255]);
    assert_eq!(m.materials[0].texture, Some(0));
    // Skinning through the Prism skeleton: turning the chest moves only the top edge.
    let mut fig = Figure { root: m.rest.clone(), ..Default::default() };
    fig.root.find_mut("spine1_bb_").unwrap().rotation = [std::f64::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f64::consts::FRAC_1_SQRT_2];
    let (posed, _) = skin(m, &fig);
    assert!(close(posed[0], m.positions[0]));
    assert!(close(posed[2], [0.0, 140.0, 0.0]), "the chest's left vertex swings under it: {:?}", posed[2]);
}

#[test]
fn vrm_0_characters_turn_to_face_forward() {
    let f = read(&character("0"), "file", None, &Interrupt::NONE).unwrap();
    assert_eq!((f.name.as_str(), f.kind), ("Old Tester", Kind::Vrm0));
    // Half a turn about Y: x flips.
    assert!(close(f.model.positions[0], [10.0, 100.0, 0.0]), "{:?}", f.model.positions[0]);
    let fig = Figure { root: f.model.rest.clone(), ..Default::default() };
    let posed = prism_algo::figure3d::skeleton::pose(&fig);
    let hips = posed.iter().find(|p| p.name == "hips_bb_").unwrap();
    assert!(close(hips.pos, [0.0, 100.0, 0.0]));
    // Plain glTF without a humanoid keeps node names and faces as it was.
    let plain = read(&character("none"), "plain", None, &Interrupt::NONE).unwrap();
    assert_eq!((plain.name.as_str(), plain.kind, plain.humanoid), ("plain", Kind::Gltf, 0));
    assert!(plain.model.rest.find("node:J_Hips").is_some());
}

#[test]
fn malformed_models_are_errors_never_panics() {
    let good = character("1");
    let mut cases: Vec<Vec<u8>> = vec![Vec::new(), b"glTF".to_vec(), b"{}".to_vec(), b"{\"asset\":{\"version\":\"1.0\"}}".to_vec(), good[..40].to_vec()];
    // Every truncation of the binary chunk, and byte flips in the header.
    for cut in [good.len() / 2, good.len() - 3, 30] {
        cases.push(good[..cut].to_vec());
    }
    let mut wrong_version = good.clone();
    wrong_version[4] = 1;
    cases.push(wrong_version);
    for case in &cases {
        assert!(read(case, "x", None, &Interrupt::NONE).is_err());
    }
    // A triangle past the vertices, an accessor past its view, a cycle of nodes.
    let tamper = |f: &dyn Fn(&mut Value)| {
        let bytes = character("1");
        let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut json: Value = serde_json::from_slice(&bytes[20..20 + len]).unwrap();
        f(&mut json);
        let mut g = Glb { json, bin: bytes[20 + len + 8..].to_vec() };
        g.bin.truncate(g.bin.len());
        g.bytes()
    };
    for bad in [
        tamper(&|j| j["accessors"][5]["count"] = json!(9)),
        tamper(&|j| j["accessors"][0]["count"] = json!(999_999)),
        tamper(&|j| j["nodes"][2]["children"] = json!([1])),
        tamper(&|j| j["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(77)),
        tamper(&|j| j["meshes"] = json!([])),
    ] {
        assert!(read(&bad, "x", None, &Interrupt::NONE).is_err());
    }
    // A missing texture leaves the colour factor.
    let no_tex = tamper(&|j| j["images"][0]["bufferView"] = json!(0));
    let f = read(&no_tex, "x", None, &Interrupt::NONE).unwrap();
    assert_eq!(f.model.materials[0].texture, None);
    assert!(is_model_name("a.VRM") && is_model_name("b.glb") && !is_model_name("c.png"));
}

#[test]
fn gltf_json_with_data_uris_reads_too() {
    // The same model as JSON, its buffer as a base64 data URI.
    let bytes = character("1");
    let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut json: Value = serde_json::from_slice(&bytes[20..20 + len]).unwrap();
    let bin = &bytes[20 + len + 8..];
    let b64: String = {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in bin.chunks(3) {
            let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
            for k in 0..4 {
                s.push(if k <= c.len() { T[((n >> (18 - 6 * k)) & 63) as usize] as char } else { '=' });
            }
        }
        s
    };
    json["buffers"][0]["uri"] = json!(format!("data:application/octet-stream;base64,{b64}"));
    let text = serde_json::to_vec(&json).unwrap();
    let f = read(&text, "x", None, &Interrupt::NONE).unwrap();
    assert_eq!(f.model.positions.len(), 4);
    // External files need the folder, and never reach outside it.
    json["buffers"][0]["uri"] = json!("../secret.bin");
    let text = serde_json::to_vec(&json).unwrap();
    assert!(read(&text, "x", None, &Interrupt::NONE).is_err());
    assert!(read(&text, "x", Some(std::path::Path::new(".")), &Interrupt::NONE).is_err());
}

/// Local look at real models in `corpus/models` (not committed; `PRISM_TEST_MODELS` to point
/// elsewhere): each one's stats, and front and three-quarter renders in `$PRISM_LOOK_OUT`
/// (default `target/models`).
#[test]
#[ignore = "reads local model files and writes PNGs to look at"]
fn local_models_render() {
    use prism_algo::figure3d::{RenderOptions, frame, orbit, render};
    use prism_doc::figure::FigureScene;
    use std::sync::Arc;
    let dir = std::path::PathBuf::from(std::env::var_os("PRISM_TEST_MODELS").unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/models").into()));
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_LOOK_OUT").unwrap_or_else(|| "target/models".into()));
    std::fs::create_dir_all(&out).unwrap();
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !is_model_name(&name) {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let t = std::time::Instant::now();
        let f = read(&bytes, &name, path.parent(), &Interrupt::NONE).unwrap_or_else(|e| panic!("{name}: {e}"));
        let m = &f.model;
        eprintln!(
            "{name}: {:?} \"{}\" read in {:?}: {} vertices, {} triangles, {} joints, {} humanoid, {} textures, {} materials",
            f.kind,
            f.name,
            t.elapsed(),
            m.positions.len(),
            m.triangles.len(),
            m.joints.len(),
            f.humanoid,
            m.textures.len(),
            m.materials.len()
        );
        let models = vec![Some(Arc::new(m.clone()))];
        for (k, yaw) in [0.0f64, 35.0].into_iter().enumerate() {
            let mut scene = FigureScene { figures: vec![prism_doc::figure::Figure { name: f.name.clone(), root: m.rest.clone(), ..Default::default() }], ..Default::default() };
            frame(&mut scene, &models, [300.0, 400.0], 720.0, true).unwrap();
            orbit(&mut scene, yaw, 0.0);
            let r = prism_geom::Rect::new(0, 0, 600, 800);
            let t = std::time::Instant::now();
            let s = render(&scene, &models, prism_color::PixelFormat::RGBA8, r, RenderOptions::default(), &Interrupt::NONE).unwrap();
            eprintln!("  render {k}: {:?}", t.elapsed());
            let px: Vec<u8> = s.read_region(r).iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            let mut onto_white = px.clone();
            for p in onto_white.chunks_mut(4) {
                let a = f32::from(p[3]) / 255.0;
                for v in p.iter_mut().take(3) {
                    *v = (f32::from(*v) * a + 255.0 * (1.0 - a)).round() as u8;
                }
                p[3] = 255;
            }
            let im = prism_codecs::Image::from_raw(600, 800, prism_codecs::ChannelLayout::Rgba, prism_codecs::SampleType::U8, onto_white).unwrap();
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            std::fs::write(out.join(format!("{stem}_{k}.png")), prism_codecs::encode(&im, prism_codecs::Format::Png, &prism_codecs::EncodeOptions::default()).unwrap()).unwrap();
        }
        seen += 1;
    }
    assert!(seen > 0, "no models in {}", dir.display());
}

/// Local look: the first local VRM in `corpus/models` posed with the local CLIP STUDIO poses (a
/// whole-figure pose, then a hand pose), beside Prism's mannequin in the same pose.
#[test]
#[ignore = "reads local model files and CLIP STUDIO poses and writes PNGs to look at"]
fn local_models_posed() {
    use crate::sut::library::{default_root, import_pose_any, list};
    use crate::sut::pose::Pose;
    use prism_algo::figure3d::hand::{Side, apply_hand_pose};
    use prism_algo::figure3d::retarget::retarget;
    use prism_algo::figure3d::{RenderOptions, frame, orbit, render};
    use prism_doc::figure::{Figure, FigureScene};
    use std::sync::Arc;
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/models"));
    let out = std::path::PathBuf::from(std::env::var_os("PRISM_LOOK_OUT").unwrap_or_else(|| "target/models".into()));
    std::fs::create_dir_all(&out).unwrap();
    let vrm = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vrm"))).expect("a .vrm in corpus/models");
    let file = read(&std::fs::read(&vrm).unwrap(), "vrm", None, &Interrupt::NONE).unwrap();
    let root = default_root().unwrap();
    let lib = list(&root).unwrap();
    let mut body = None;
    let mut hand = None;
    for p in &lib.poses {
        match import_pose_any(&root, &p.uuid, &Interrupt::NONE) {
            Ok((_, Pose::Body(f))) if body.is_none() => body = Some(f),
            Ok((_, Pose::Hand(h))) if p.name.contains("Peace 1") => hand = Some(h),
            _ => {}
        }
    }
    let body = body.expect("a whole-figure pose");
    let mut character = Figure { name: file.name.clone(), root: file.model.rest.clone(), ..Default::default() };
    retarget(&body, &mut character, &file.model.rest).unwrap();
    if let Some(h) = &hand {
        let from = if h.left { Side::Left } else { Side::Right };
        for side in [Side::Left, Side::Right] {
            apply_hand_pose(&mut character, side, &h.hand, from, [false; 5]).unwrap();
        }
    }
    let mut mannequin = body.clone();
    mannequin.root.offset = [90.0, 0.0, 0.0];
    mannequin.root.rotation = [1.0, 0.0, 0.0, 0.0];
    let models = vec![Some(Arc::new(file.model.clone())), None];
    for (k, yaw) in [0.0f64, 40.0, -50.0].into_iter().enumerate() {
        let mut scene = FigureScene { figures: vec![character.clone(), mannequin.clone()], ..Default::default() };
        frame(&mut scene, &models, [500.0, 400.0], 720.0, false).unwrap();
        orbit(&mut scene, yaw, 5.0);
        let r = prism_geom::Rect::new(0, 0, 1000, 800);
        let s = render(&scene, &models, prism_color::PixelFormat::RGBA8, r, RenderOptions::default(), &Interrupt::NONE).unwrap();
        let mut px: Vec<u8> = s.read_region(r).iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        for p in px.chunks_mut(4) {
            let a = f32::from(p[3]) / 255.0;
            for v in p.iter_mut().take(3) {
                *v = (f32::from(*v) * a + 255.0 * (1.0 - a)).round() as u8;
            }
            p[3] = 255;
        }
        let im = prism_codecs::Image::from_raw(1000, 800, prism_codecs::ChannelLayout::Rgba, prism_codecs::SampleType::U8, px).unwrap();
        std::fs::write(out.join(format!("posed_{k}.png")), prism_codecs::encode(&im, prism_codecs::Format::Png, &prism_codecs::EncodeOptions::default()).unwrap()).unwrap();
    }
}
