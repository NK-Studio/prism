//! Character models for 3D figure layers: VRM (0.x and 1.0) and glTF 2.0 (`.glb`, `.gltf`)
//! read into a [`Model`] — a skinned mesh in rest world space with its textures — and the Prism
//! skeleton it is posed with.
//!
//! - Units become centimetres and every model faces +Z with its left on +X (Prism's figure
//!   convention; VRM 0.x models face −Z and are turned half a turn).
//! - The skeleton is every skin joint and humanoid bone with their ancestors, under a
//!   `root_group_` joint that places the figure. Humanoid bones take Prism's (CLIP STUDIO's)
//!   joint names ([`humanoid`]); other nodes are `node:<name>`. Scale is baked out: joints keep
//!   only offsets and rotations.
//! - Vertices are baked to the rest pose (each weighted joint's rest transform × its inverse bind
//!   matrix), so posing needs only each joint's rigid change from rest.
use prism_algo::figure3d::mesh::{AlphaMode, MAX_TEXTURE, MAX_TRIANGLES, MAX_VERTICES, Material, Model, Texture};
use prism_doc::figure::{Joint, MAX_DEPTH, MAX_JOINTS, MAX_NAME};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

mod gltf;
mod humanoid;

pub use gltf::MAX_FILE;
use gltf::{IDENTITY, M4, Result, apply, apply_dir, arr, floats, index, local, mul, rotation_of};
pub use humanoid::Kind;

/// A character model read from a file.
#[derive(Clone, Debug)]
pub struct ModelFile {
    pub name: String,
    pub kind: Kind,
    pub model: Model,
    /// How many humanoid bones were found (0: no CLIP STUDIO poses apply).
    pub humanoid: usize,
}

/// Whether `name` looks like a model file Prism reads.
pub fn is_model_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".vrm") || n.ends_with(".glb") || n.ends_with(".gltf")
}

/// Whether `bytes` start like a binary glTF (`.glb`, `.vrm`).
pub fn is_glb(bytes: &[u8]) -> bool {
    bytes.starts_with(b"glTF")
}

/// A self-contained copy of a model file: a `.gltf` whose buffers or images sit in files beside
/// it (`dir`) gets them inlined as `data:` URIs, so the figure keeps everything it needs. Binary
/// files (`.glb`, `.vrm`) come back as they are.
pub fn self_contained(bytes: &[u8], dir: Option<&Path>) -> std::result::Result<Vec<u8>, String> {
    if is_glb(bytes) {
        return Ok(bytes.to_vec());
    }
    let mut json: Value = serde_json::from_slice(bytes).map_err(|e| format!("not a glTF file ({e})"))?;
    for key in ["buffers", "images"] {
        let Some(list) = json.get_mut(key).and_then(Value::as_array_mut) else { continue };
        for item in list {
            let Some(uri) = item.get("uri").and_then(Value::as_str).filter(|u| !u.starts_with("data:")).map(str::to_string) else { continue };
            let data = gltf::uri_bytes(&uri, dir)?;
            if let Some(o) = item.as_object_mut() {
                o.insert("uri".into(), Value::String(format!("data:application/octet-stream;base64,{}", prism_paint::tile::b64_encode(&data))));
            }
        }
    }
    let out = serde_json::to_vec(&json).map_err(|e| e.to_string())?;
    if out.len() > MAX_FILE {
        return Err("the model file exceeds 256 MiB".into());
    }
    Ok(out)
}

/// Read a `.vrm`, `.glb` or `.gltf`. `dir` is the file's folder, for `.gltf` files that keep
/// buffers or images beside them.
pub fn read(bytes: &[u8], fallback_name: &str, dir: Option<&Path>, ctl: &prism_raster::Interrupt) -> std::result::Result<ModelFile, String> {
    ctl.check().map_err(|e| e.to_string())?;
    let doc = gltf::parse(bytes, dir)?;
    let json = &doc.json;
    let (kind, bones) = humanoid::humanoid(json);
    let nodes = arr(json, "nodes");
    if nodes.len() > 200_000 {
        return Err("the model has too many nodes".into());
    }
    let n = nodes.len();
    // Parents (a node may have one), then world matrices from the roots down.
    let mut parent: Vec<Option<usize>> = vec![None; n];
    for (i, node) in nodes.iter().enumerate() {
        for c in arr(node, "children").iter().filter_map(|c| c.as_u64().and_then(|c| usize::try_from(c).ok())) {
            let slot = parent.get_mut(c).ok_or("a node has a missing child")?;
            if slot.is_some() || c == i {
                return Err("a node has two parents".into());
            }
            *slot = Some(i);
        }
    }
    let mut world: Vec<Option<M4>> = vec![None; n];
    let mut depth = vec![0usize; n];
    let mut stack: Vec<usize> = (0..n).filter(|i| parent.get(*i).is_some_and(Option::is_none)).collect();
    while let Some(i) = stack.pop() {
        let Some(node) = nodes.get(i) else { continue };
        let (pw, pd) = match parent.get(i).copied().flatten() {
            Some(p) => (world.get(p).copied().flatten().unwrap_or(IDENTITY), depth.get(p).copied().unwrap_or(0) + 1),
            None => (IDENTITY, 0),
        };
        if pd > 256 {
            return Err("the model's nodes nest too deep".into());
        }
        if let Some(w) = world.get_mut(i) {
            *w = Some(mul(&pw, &local(node)));
        }
        if let Some(d) = depth.get_mut(i) {
            *d = pd;
        }
        for c in arr(node, "children").iter().filter_map(|c| c.as_u64().and_then(|c| usize::try_from(c).ok())) {
            stack.push(c);
        }
    }
    let world_of = |i: usize| world.get(i).copied().flatten().unwrap_or(IDENTITY);
    // glTF metres → centimetres; VRM 0.x turned to face +Z.
    let turn: M4 = if kind == Kind::Vrm0 { [-100.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 0.0, -100.0, 0.0, 0.0, 0.0, 0.0, 1.0] } else { [100.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 0.0, 100.0, 0.0, 0.0, 0.0, 0.0, 1.0] };
    let to_prism = |m: &M4| mul(&turn, m);

    // The skeleton: skin joints and humanoid bones, with their ancestors.
    let human: HashMap<usize, String> = bones.into_iter().collect();
    let mut wanted: HashSet<usize> = human.keys().copied().collect();
    for skin in arr(json, "skins") {
        wanted.extend(arr(skin, "joints").iter().filter_map(|j| j.as_u64().and_then(|j| usize::try_from(j).ok())));
    }
    for i in wanted.clone() {
        let mut p = parent.get(i).copied().flatten();
        let mut guard = 0;
        while let Some(q) = p {
            if !wanted.insert(q) || guard > 256 {
                break;
            }
            guard += 1;
            p = parent.get(q).copied().flatten();
        }
    }
    wanted.retain(|i| *i < n);
    if wanted.len() + 1 > MAX_JOINTS {
        return Err(format!("the model has more than {MAX_JOINTS} bones"));
    }
    let mut used_names: HashSet<String> = HashSet::from(["root_group_".to_string()]);
    let mut names: HashMap<usize, String> = HashMap::new();
    let mut order: Vec<usize> = wanted.iter().copied().collect();
    order.sort_unstable();
    for i in order {
        let base = human.get(&i).cloned().unwrap_or_else(|| {
            let raw: String = nodes.get(i).and_then(|v| v.get("name")).and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_NAME - 16).collect();
            if raw.is_empty() { format!("node:{i}") } else { format!("node:{raw}") }
        });
        let name = if used_names.contains(&base) { format!("{base}#{i}") } else { base };
        used_names.insert(name.clone());
        names.insert(i, name);
    }
    fn build(i: usize, nodes: &[Value], wanted: &HashSet<usize>, names: &HashMap<usize, String>, world: &dyn Fn(usize) -> M4, parent_world: &M4, d: usize) -> Result<Joint> {
        if d > MAX_DEPTH {
            return Err("the model's bones nest too deep".into());
        }
        let w = world(i);
        let (pq, wq) = (rotation_of(parent_world), rotation_of(&w));
        let (pp, wp) = (apply(parent_world, [0.0; 3]), apply(&w, [0.0; 3]));
        let inv = [pq[0], -pq[1], -pq[2], -pq[3]];
        let offset = qrot(inv, [wp[0] - pp[0], wp[1] - pp[1], wp[2] - pp[2]]);
        let rotation = prism_doc::figure::normalize_quat(qmul(inv, wq));
        let mut children = Vec::new();
        for c in arr(nodes.get(i).unwrap_or(&Value::Null), "children").iter().filter_map(|c| c.as_u64().and_then(|c| usize::try_from(c).ok())) {
            if wanted.contains(&c) {
                children.push(build(c, nodes, wanted, names, world, &w, d + 1)?);
            }
        }
        Ok(Joint { name: names.get(&i).cloned().unwrap_or_default(), rotation, offset, children })
    }
    let prism_world = |i: usize| to_prism(&world_of(i));
    // `root_group_` sits at the origin unturned: the top joints hang off that frame.
    let root_frame: M4 = IDENTITY;
    let mut tops: Vec<usize> = wanted.iter().copied().filter(|i| parent.get(*i).copied().flatten().is_none_or(|p| !wanted.contains(&p))).collect();
    tops.sort_unstable();
    let mut root = Joint { name: "root_group_".into(), rotation: [1.0, 0.0, 0.0, 0.0], offset: [0.0; 3], children: Vec::new() };
    for t in tops {
        root.children.push(build(t, nodes, &wanted, &names, &prism_world, &root_frame, 1)?);
    }
    let mut joint_names: Vec<String> = vec!["root_group_".into()];
    let mut joint_index: HashMap<usize, u16> = HashMap::new();
    let mut ordered: Vec<(usize, &String)> = names.iter().map(|(i, s)| (*i, s)).collect();
    ordered.sort_unstable();
    for (i, s) in ordered {
        joint_index.insert(i, u16::try_from(joint_names.len()).map_err(|_| "too many bones")?);
        joint_names.push(s.clone());
    }
    // The joint a rigid (unskinned) mesh node follows: itself or its nearest skeleton ancestor.
    let follow = |mut i: usize| -> u16 {
        for _ in 0..512 {
            if let Some(j) = joint_index.get(&i) {
                return *j;
            }
            match parent.get(i).copied().flatten() {
                Some(p) => i = p,
                None => break,
            }
        }
        0
    };

    let mut model = Model { joints: joint_names, rest: root, ..Default::default() };
    let mut materials: HashMap<Option<usize>, u16> = HashMap::new();
    let mut textures: HashMap<usize, Option<usize>> = HashMap::new();
    for (ni, node) in nodes.iter().enumerate() {
        ctl.check().map_err(|e| e.to_string())?;
        let Some(mesh) = index(node, "mesh").and_then(|m| arr(json, "meshes").get(m)) else { continue };
        let skin = index(node, "skin").and_then(|s| arr(json, "skins").get(s));
        // Each skin joint's rest transform × inverse bind matrix (glTF space).
        let binds: Vec<(M4, u16)> = match skin {
            Some(s) => {
                let joints: Vec<usize> = arr(s, "joints").iter().filter_map(|j| j.as_u64().and_then(|j| usize::try_from(j).ok())).collect();
                let ibm = match index(s, "inverseBindMatrices") {
                    Some(a) => doc.read(a, 16)?,
                    None => Vec::new(),
                };
                joints
                    .iter()
                    .enumerate()
                    .map(|(k, j)| {
                        let m: M4 = ibm.get(k * 16..k * 16 + 16).and_then(|s| <[f64; 16]>::try_from(s).ok()).unwrap_or(IDENTITY);
                        (mul(&world_of(*j), &m), joint_index.get(j).copied().unwrap_or(0))
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        let rigid = (world_of(ni), follow(ni));
        for prim in arr(mesh, "primitives") {
            if prim.get("mode").and_then(Value::as_u64).unwrap_or(4) != 4 {
                continue;
            }
            let attrs = prim.get("attributes").unwrap_or(&Value::Null);
            let Some(pos_a) = index(attrs, "POSITION") else { continue };
            let pos = doc.read(pos_a, 3)?;
            let count = pos.len() / 3;
            let base = model.positions.len();
            if base + count > MAX_VERTICES {
                return Err("the model has too many vertices".into());
            }
            let normals = match index(attrs, "NORMAL") {
                Some(a) => Some(doc.read(a, 3)?),
                None => None,
            };
            let uvs = match index(attrs, "TEXCOORD_0") {
                Some(a) => doc.read(a, 2)?,
                None => vec![0.0; count * 2],
            };
            let (joints, weights) = match (skin, index(attrs, "JOINTS_0"), index(attrs, "WEIGHTS_0")) {
                (Some(_), Some(j), Some(w)) => (doc.read(j, 4)?, doc.read(w, 4)?),
                _ => (Vec::new(), Vec::new()),
            };
            for v in 0..count {
                let p = [pos.get(v * 3).copied().unwrap_or(0.0), pos.get(v * 3 + 1).copied().unwrap_or(0.0), pos.get(v * 3 + 2).copied().unwrap_or(0.0)];
                let nrm = normals.as_ref().map(|nv| [nv.get(v * 3).copied().unwrap_or(0.0), nv.get(v * 3 + 1).copied().unwrap_or(0.0), nv.get(v * 3 + 2).copied().unwrap_or(1.0)]);
                let mut influence = [(0u16, 0.0f32); 4];
                let (mut wp, mut wn, mut total) = ([0.0; 3], [0.0; 3], 0.0);
                if !joints.is_empty() {
                    for k in 0..4 {
                        let w = weights.get(v * 4 + k).copied().unwrap_or(0.0);
                        let ji = joints.get(v * 4 + k).copied().unwrap_or(0.0) as usize;
                        let Some((m, joint)) = binds.get(ji) else { continue };
                        if w <= 0.0 {
                            continue;
                        }
                        let q = apply(m, p);
                        wp = [wp[0] + q[0] * w, wp[1] + q[1] * w, wp[2] + q[2] * w];
                        if let Some(nv) = nrm {
                            let d = apply_dir(m, nv);
                            wn = [wn[0] + d[0] * w, wn[1] + d[1] * w, wn[2] + d[2] * w];
                        }
                        if let Some(slot) = influence.get_mut(k) {
                            *slot = (*joint, w as f32);
                        }
                        total += w;
                    }
                }
                let (wp, wn) = if total > 1e-9 {
                    let s = 1.0 / total;
                    for slot in &mut influence {
                        slot.1 = (f64::from(slot.1) * s) as f32;
                    }
                    ([wp[0] * s, wp[1] * s, wp[2] * s], wn)
                } else {
                    influence = [(rigid.1, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)];
                    (apply(&rigid.0, p), nrm.map_or([0.0; 3], |nv| apply_dir(&rigid.0, nv)))
                };
                model.positions.push(apply(&turn, wp));
                model.normals.push(normalize(apply_dir(&turn, wn)));
                model.uvs.push([uvs.get(v * 2).copied().unwrap_or(0.0) as f32, uvs.get(v * 2 + 1).copied().unwrap_or(0.0) as f32]);
                model.influences.push(influence);
            }
            let indices: Vec<usize> = match index(prim, "indices") {
                Some(a) => doc.read(a, 1)?.into_iter().map(|i| i as usize).collect(),
                None => (0..count).collect(),
            };
            let mat = index(prim, "material");
            let mi = match materials.get(&mat) {
                Some(m) => *m,
                None => {
                    let m = material(&doc, mat, &mut textures, &mut model.textures)?;
                    let id = u16::try_from(model.materials.len()).map_err(|_| "the model has too many materials")?;
                    model.materials.push(m);
                    materials.insert(mat, id);
                    id
                }
            };
            for t in indices.chunks_exact(3) {
                if t.iter().any(|i| *i >= count) {
                    return Err("a triangle refers to a missing vertex".into());
                }
                if model.triangles.len() >= MAX_TRIANGLES {
                    return Err("the model has too many triangles".into());
                }
                let at = |k: usize| u32::try_from(base + t.get(k).copied().unwrap_or(0)).unwrap_or(0);
                model.triangles.push([at(0), at(1), at(2)]);
                model.triangle_material.push(mi);
            }
            if normals.is_none() {
                smooth_normals(&mut model, base);
            }
        }
    }
    if model.triangles.is_empty() {
        return Err("the model has no triangles to draw".into());
    }
    model.validate()?;
    let name = humanoid::title(json).unwrap_or_else(|| fallback_name.trim().to_string());
    let name: String = name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let name = if name.is_empty() { "Character".to_string() } else { name };
    model.title = name.clone();
    Ok(ModelFile { name, kind, model, humanoid: human.len() })
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-12 && l.is_finite() { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 0.0, 1.0] }
}

fn qmul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [aw, ax, ay, az] = a;
    let [bw, bx, by, bz] = b;
    [aw * bw - ax * bx - ay * by - az * bz, aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw]
}

fn qrot(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let p = qmul(qmul(q, [0.0, v[0], v[1], v[2]]), [q[0], -q[1], -q[2], -q[3]]);
    [p[1], p[2], p[3]]
}

/// Area-weighted vertex normals for a primitive that has none (vertices from `base` on).
fn smooth_normals(model: &mut Model, base: usize) {
    let mut acc = vec![[0.0f64; 3]; model.positions.len().saturating_sub(base)];
    for t in &model.triangles {
        let [a, b, c] = t.map(|i| i as usize);
        if a < base || b < base || c < base {
            continue;
        }
        let (Some(pa), Some(pb), Some(pc)) = (model.positions.get(a), model.positions.get(b), model.positions.get(c)) else { continue };
        let (u, v) = ([pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]], [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        for i in [a, b, c] {
            if let Some(s) = acc.get_mut(i - base) {
                *s = [s[0] + n[0], s[1] + n[1], s[2] + n[2]];
            }
        }
    }
    for (k, s) in acc.into_iter().enumerate() {
        if let Some(n) = model.normals.get_mut(base + k) {
            *n = normalize(s);
        }
    }
}

/// A glTF material (or the default one): base colour factor and texture, alpha mode.
fn material(doc: &gltf::Doc, i: Option<usize>, decoded: &mut HashMap<usize, Option<usize>>, textures: &mut Vec<Texture>) -> Result<Material> {
    let Some(m) = i.and_then(|i| arr(&doc.json, "materials").get(i)) else { return Ok(Material::default()) };
    let pbr = m.get("pbrMetallicRoughness").unwrap_or(&Value::Null);
    let factor = floats::<4>(pbr, "baseColorFactor").unwrap_or([1.0; 4]).map(|v| v.clamp(0.0, 1.0) as f32);
    // VRM 0.x MToon keeps the main texture in its material properties when glTF's is missing.
    let tex_index = pbr.get("baseColorTexture").and_then(|t| index(t, "index")).or_else(|| {
        let name = m.get("name").and_then(Value::as_str)?;
        let props = doc.json.get("extensions")?.get("VRM")?.get("materialProperties")?.as_array()?;
        let p = props.iter().find(|p| p.get("name").and_then(Value::as_str) == Some(name))?;
        index(p.get("textureProperties")?, "_MainTex")
    });
    let texture = match tex_index.and_then(|t| arr(&doc.json, "textures").get(t)).and_then(|t| index(t, "source")) {
        Some(img) => match decoded.get(&img) {
            Some(t) => *t,
            None => {
                let t = image(doc, img).map(|tex| {
                    textures.push(tex);
                    textures.len() - 1
                });
                decoded.insert(img, t);
                t
            }
        },
        None => None,
    };
    let alpha = match m.get("alphaMode").and_then(Value::as_str) {
        Some("MASK") => AlphaMode::Mask(m.get("alphaCutoff").and_then(Value::as_f64).unwrap_or(0.5).clamp(0.0, 1.0) as f32),
        Some("BLEND") => AlphaMode::Blend,
        _ => AlphaMode::Opaque,
    };
    Ok(Material { color: factor, texture, alpha })
}

/// Decode glTF image `i` (PNG or JPEG) to RGBA8; `None` when it can't be read (the material
/// keeps its colour factor).
fn image(doc: &gltf::Doc, i: usize) -> Option<Texture> {
    let img = arr(&doc.json, "images").get(i)?;
    let bytes: Vec<u8> = match index(img, "bufferView") {
        Some(v) => doc.view_bytes(v).ok()?.to_vec(),
        None => {
            let uri = img.get("uri")?.as_str()?;
            uri.strip_prefix("data:").and_then(|r| r.split_once(";base64,")).and_then(|(_, d)| gltf::base64(d).ok())?
        }
    };
    let decoded = prism_codecs::decode(&bytes).ok()?;
    let (w, h) = (decoded.width(), decoded.height());
    if w == 0 || h == 0 || w > MAX_TEXTURE || h > MAX_TEXTURE {
        return None;
    }
    let rgba = decoded.to_rgba8();
    (rgba.len() == w as usize * h as usize * 4).then_some(Texture { width: w, height: h, rgba })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
