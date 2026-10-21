//! The CLIP STUDIO Material palette's folder tree, built from the listed materials' `folders`
//! (CLIP STUDIO's folder tags: colon paths such as `ダウンロード:Pack-1758445:Brushes`). The
//! system roots are stored in Japanese and shown translated; a download pack's `-<content id>`
//! suffix is hidden. Pure data: the panel draws it.

use serde_json::Value;

/// The virtual folder of favourite materials (never a CLIP STUDIO path: it has no colon form).
pub const FAVORITE: &str = "\u{1}favorite";
/// Folder nesting shown.
const MAX_DEPTH: usize = 8;
/// Folders shown in all.
const MAX_NODES: usize = 4096;

/// CLIP STUDIO's system roots in its palette order, with their English names.
const ROOTS: [(&str, &str); 6] = [
    ("カラーパターン", "Color pattern"),
    ("単色パターン", "Monochromatic pattern"),
    ("漫画素材", "Manga material"),
    ("画像素材", "Image material"),
    ("3D", "3D"),
    ("ダウンロード", "Download"),
];

/// System sub-folders (inside a system root other than Download).
const SUBFOLDERS: [(&str, &str); 11] = [
    ("ブラシ", "Brush"),
    ("イラスト", "Illustration"),
    ("写真", "Picture"),
    ("テクスチャ", "Texture"),
    ("コマ割りテンプレート", "Framing template"),
    ("頭部", "Head"),
    ("体型", "Body type"),
    ("プリミティブ", "Primitive"),
    ("ハンドモデル", "Hand model"),
    ("背景", "Background"),
    ("新規フォルダー", "New folder"),
];

/// One folder: its full colon path (the filter key), what it shows, and its subfolders.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub path: String,
    pub label: String,
    /// Materials in this folder and below.
    pub count: usize,
    pub children: Vec<Node>,
}

/// The English name of a system folder segment (to be translated where drawn), else the segment
/// as stored; a download pack (`Name-1758445` right below Download) loses its content id.
pub fn label(path: &[&str]) -> String {
    let Some((seg, parents)) = path.split_last() else { return String::new() };
    let seg = seg.trim();
    match parents {
        [] => ROOTS.iter().find(|(ja, _)| *ja == seg).map_or(seg, |(_, en)| en).to_string(),
        ["ダウンロード"] => match seg.rsplit_once('-') {
            Some((name, id)) if !name.trim().is_empty() && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => name.trim().to_string(),
            _ => seg.to_string(),
        },
        [root, ..] if *root != "ダウンロード" && ROOTS.iter().any(|(ja, _)| ja == root) => {
            SUBFOLDERS.iter().find(|(ja, _)| *ja == seg).map_or(seg, |(_, en)| en).to_string()
        }
        _ => seg.to_string(),
    }
}

/// Is `seg` one of CLIP STUDIO's system names (so its label goes through the translations)?
pub fn is_system(path: &[&str]) -> bool {
    match path {
        [seg] => ROOTS.iter().any(|(ja, _)| ja == seg),
        [root, .., seg] if *root != "ダウンロード" && ROOTS.iter().any(|(ja, _)| ja == root) => SUBFOLDERS.iter().any(|(ja, _)| ja == seg),
        _ => false,
    }
}

/// The folders of `material` (its `folders` strings).
pub fn folders_of(material: &Value) -> impl Iterator<Item = &str> {
    material["folders"].as_array().into_iter().flatten().filter_map(Value::as_str)
}

/// Is `material` in `folder` (or below it)? `None` is every material.
pub fn contains(folder: Option<&str>, material: &Value) -> bool {
    match folder {
        None => true,
        Some(FAVORITE) => material["favorite"] == true,
        Some(f) => folders_of(material).any(|m| m == f || m.strip_prefix(f).is_some_and(|rest| rest.starts_with(':'))),
    }
}

fn insert(nodes: &mut Vec<Node>, segs: &[&str], depth: usize, total: &mut usize) {
    let Some(&seg) = segs.get(depth) else { return };
    if seg.trim().is_empty() || depth >= MAX_DEPTH {
        return;
    }
    let path = segs.get(..=depth).unwrap_or_default();
    let key = path.join(":");
    let i = match nodes.iter().position(|n| n.path == key) {
        Some(i) => i,
        None if *total < MAX_NODES => {
            *total += 1;
            nodes.push(Node { path: key, label: label(path), count: 0, children: Vec::new() });
            nodes.len() - 1
        }
        None => return,
    };
    if let Some(n) = nodes.get_mut(i) {
        n.count += 1;
        insert(&mut n.children, segs, depth + 1, total);
    }
}

fn sort(nodes: &mut [Node]) {
    nodes.sort_by_key(|n| n.label.to_lowercase());
    for n in nodes {
        sort(&mut n.children);
    }
}

/// The folder tree of `materials`, system roots first in CLIP STUDIO's order, then the user's.
pub fn build<'a>(materials: impl Iterator<Item = &'a Value>) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut total = 0;
    for m in materials {
        // A material counts once per folder even when it is tagged twice below one root.
        let mut seen: Vec<&str> = Vec::new();
        for f in folders_of(m) {
            if seen.contains(&f) {
                continue;
            }
            seen.push(f);
            let segs: Vec<&str> = f.split(':').collect();
            insert(&mut nodes, &segs, 0, &mut total);
        }
    }
    sort(&mut nodes);
    nodes.sort_by_key(|n| ROOTS.iter().position(|(ja, _)| *ja == n.path).unwrap_or(ROOTS.len()));
    nodes
}

/// The node at `path`, searched depth first.
pub fn find<'a>(nodes: &'a [Node], path: &str) -> Option<&'a Node> {
    nodes.iter().find_map(|n| if n.path == path { Some(n) } else { find(&n.children, path) })
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
