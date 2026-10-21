use super::*;
use serde_json::json;

fn listing() -> Vec<Value> {
    vec![
        json!({"name": "Isometric ruler", "folders": ["ダウンロード:Isometric ruler and grid-1758445:Isometric ruler and grid"], "favorite": true}),
        json!({"name": "Isometric grid", "folders": ["ダウンロード:Isometric ruler and grid-1758445:Isometric ruler and grid"]}),
        json!({"name": "Tip", "folders": ["画像素材:ブラシ", "自作ブラシ:茂み"]}),
        json!({"name": "Loose"}),
    ]
}

#[test]
fn system_roots_come_first_translated_and_packs_lose_their_content_id() {
    let l = listing();
    let tree = build(l.iter());
    let labels: Vec<&str> = tree.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["Image material", "Download", "自作ブラシ"]);
    let download = find(&tree, "ダウンロード").unwrap();
    assert_eq!(download.count, 2);
    let pack = &download.children[0];
    assert_eq!((pack.label.as_str(), pack.path.as_str()), ("Isometric ruler and grid", "ダウンロード:Isometric ruler and grid-1758445"));
    assert_eq!(find(&tree, "画像素材:ブラシ").unwrap().label, "Brush");
    assert!(is_system(&["画像素材", "ブラシ"]) && is_system(&["ダウンロード"]) && !is_system(&["ダウンロード", "ブラシ"]) && !is_system(&["自作ブラシ"]));
    // Names that only look like a content id keep their dash.
    assert_eq!(label(&["ダウンロード", "Line-art"]), "Line-art");
    assert_eq!(label(&["ダウンロード", "-123"]), "-123");
}

#[test]
fn a_folder_holds_its_subfolders_materials_and_favourites_are_virtual() {
    let l = listing();
    let names = |f: Option<&str>| l.iter().filter(|m| contains(f, m)).map(|m| m["name"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(names(None).len(), 4);
    assert_eq!(names(Some("ダウンロード")), ["Isometric ruler", "Isometric grid"]);
    assert_eq!(names(Some("自作ブラシ")), ["Tip"]);
    assert!(names(Some("自作")).is_empty(), "a prefix is not a parent folder");
    assert_eq!(names(Some(FAVORITE)), ["Isometric ruler"]);
}

#[test]
fn hostile_folder_data_is_bounded() {
    let deep = (0..50).map(|i| format!("f{i}")).collect::<Vec<_>>().join(":");
    let many: Vec<Value> = (0..5000).map(|i| json!({"folders": [format!("user{i}")]})).collect();
    let odd = [json!({"folders": [deep, "", ":::", 3, null, "a::b"]}), json!({"folders": "x"})];
    let tree = build(odd.iter());
    let mut depth = 0;
    let mut n = tree.iter().find(|n| n.path == "f0");
    while let Some(node) = n {
        depth += 1;
        n = node.children.first();
    }
    assert_eq!(depth, 8);
    assert!(build(many.iter()).len() <= 4096);
}
