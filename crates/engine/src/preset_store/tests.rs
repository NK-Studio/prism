use super::*;
use prism_psd::abr::{AbrSample, LegacyBrush, LegacyTip, write_v12};
use serde_json::json;
use std::path::PathBuf;

/// A fresh, empty directory under the system temp dir (tests never write anywhere else).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pc-preset-store-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        TempDir(d)
    }
    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = DirBackend::new(&self.0).list().unwrap().into_iter().map(|(n, _)| n).collect();
        v.sort();
        v
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A session with the store in `dir` attached.
fn session(dir: &TempDir) -> (Session, Vec<String>) {
    let mut s = Session::new();
    let w = s.attach_preset_store(open_dir(&dir.0));
    (s, w)
}

fn tip8(w: u32, h: u32, seed: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| ((x * 7 + y * 13 + seed) % 256) as f32 / 255.0)
}

fn tip16(w: u32, h: u32, seed: u32) -> GrayTile {
    let data = (0..w * h).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 16) as u16).collect();
    GrayTile { width: w, height: h, data }
}

fn sampled(name: &str, group: &str, tip: GrayTile) -> BrushPreset {
    let brush = BrushSettings { size: 42.0, spacing: 0.1, tip: TipShape::Sampled(tip), ..Default::default() };
    BrushPreset { name: name.into(), brush, builtin: false, group: group.into(), sut: None }
}

fn user(s: &Session) -> Vec<BrushPreset> {
    s.tools.presets.iter().filter(|p| !p.builtin).cloned().collect()
}

#[test]
fn sessions_have_no_store_by_default() {
    let mut s = Session::new();
    assert!(s.preset_store.is_none(), "headless sessions must stay hermetic");
    s.execute("brush.presets.save", json!({"name": "Mine"})).unwrap();
    assert!(s.preset_store.is_none());
}

#[test]
fn tip_codec_round_trips_8_and_16_bit() {
    let t8 = tip8(37, 21, 3);
    let t16 = tip16(40, 33, 9);
    let e8 = encode_tip(&t8);
    let e16 = encode_tip(&t16);
    assert_eq!(e8[6], 8, "8-bit sourced tips are stored at 8 bits");
    assert_eq!(e16[6], 16);
    assert_eq!(decode_tip(&e8).unwrap(), t8);
    assert_eq!(decode_tip(&e16).unwrap(), t16);
}

#[test]
fn tip_decoder_rejects_garbage() {
    let good = encode_tip(&tip16(16, 16, 1));
    for n in 0..good.len() {
        assert!(decode_tip(&good[..n]).is_err(), "truncated at {n}");
    }
    let mut bad = good.clone();
    bad[6] = 12;
    assert!(decode_tip(&bad).is_err());
    // A header claiming a huge bitmap is refused before allocating.
    let mut huge = good.clone();
    huge[7..11].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_tip(&huge).is_err());
    let mut big = good.clone();
    big[7..11].copy_from_slice(&MAX_TIP_SIDE.to_le_bytes());
    big[11..15].copy_from_slice(&MAX_TIP_SIDE.to_le_bytes());
    assert!(decode_tip(&big).is_err());
    let mut rng = 0x1234_5678u32;
    for _ in 0..200 {
        let len = (rng % 64) as usize;
        let v: Vec<u8> = (0..len)
            .map(|_| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                rng as u8
            })
            .collect();
        let _ = decode_tip(&v);
        let mut w = TIP_MAGIC.to_vec();
        w.extend_from_slice(&v);
        let _ = decode_tip(&w);
    }
}

#[test]
fn group_with_sampled_8_and_16_bit_tips_round_trips() {
    let dir = TempDir::new("roundtrip");
    let (mut s, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert!(dir.files().is_empty(), "attaching an empty store writes nothing");
    let mut a = sampled("Eight", "Inks", tip8(64, 48, 1));
    a.brush.dual_brush.tip = TipShape::Sampled(tip8(12, 12, 5));
    a.brush.texture.pattern = Pattern::Tile(tip16(32, 32, 7));
    let b = sampled("Sixteen", "Inks", tip16(50, 70, 2));
    let mut c = sampled("Shared tip", "Other", tip8(64, 48, 1));
    c.brush.size = 9.0;
    s.tools.presets.extend([a, b, c]);
    s.brush_presets_changed();
    s.sync_preset_store();
    let files = dir.files();
    assert_eq!(files.iter().filter(|f| f.ends_with(".pcbrushes")).count(), 2, "{files:?}");
    // Four distinct bitmaps: the 64×48 tip is shared by two presets.
    assert_eq!(files.iter().filter(|f| f.starts_with("tips/")).count(), 4, "{files:?}");
    assert!(files.contains(&INDEX_FILE.to_string()));

    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(user(&t), user(&s));
    let order: Vec<String> = user(&t).into_iter().map(|p| p.name).collect();
    assert_eq!(order, ["Eight", "Sixteen", "Shared tip"]);
    // Nothing changed: reloading rewrote nothing.
    let before: Vec<_> = DirBackend::new(&dir.0).list().unwrap();
    let mut t = t;
    t.brush_presets_changed();
    t.sync_preset_store();
    assert_eq!(DirBackend::new(&dir.0).list().unwrap(), before);
}

#[test]
fn commands_persist_and_delete_removes_from_disk() {
    let dir = TempDir::new("commands");
    let (mut s, _) = session(&dir);
    s.tools.brush.tip = TipShape::Sampled(tip16(20, 20, 4));
    s.execute("brush.presets.save", json!({"name": "Scribble"})).unwrap();
    assert_eq!(dir.files().iter().filter(|f| f.starts_with("tips/")).count(), 1);
    let (t, _) = session(&dir);
    let p = prism_paint::presets::find(&t.tools.presets, "Scribble").expect("saved preset reloads");
    assert_eq!(p.brush.tip, s.tools.brush.tip);

    // Rename through the Preset Manager, then delete.
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Scribble", "newName": "Doodle"})).unwrap();
    let (t, _) = session(&dir);
    assert!(prism_paint::presets::find(&t.tools.presets, "Doodle").is_some());
    assert!(prism_paint::presets::find(&t.tools.presets, "Scribble").is_none());
    s.execute("brush.presets.delete", json!({"name": "Doodle"})).unwrap();
    assert_eq!(dir.files(), [INDEX_FILE], "the group file and its tip are gone");
    let (t, _) = session(&dir);
    assert!(prism_paint::presets::find(&t.tools.presets, "Doodle").is_none());
}

#[test]
fn sampled_preset_preserves_four_independent_pressure_curves_on_disk() {
    use prism_paint::{Control, CspTextureMode, Dynamic, PressureCurve};

    let dir = TempDir::new("pressure-curves");
    let (mut s, warnings) = session(&dir);
    assert!(warnings.is_empty(), "{warnings:?}");
    let dynamic = |points: [[f64; 2]; 3], minimum| Dynamic {
        control: Control::PenPressure,
        minimum,
        pressure_curve: Some(PressureCurve::new(points.to_vec()).unwrap()),
        ..Default::default()
    };
    // Original synthetic coordinates and bitmaps model four separately imported effectors.
    let mut preset = sampled("Four responses", "Synthetic SUT", tip16(17, 13, 9));
    preset.brush.shape_dynamics.enabled = true;
    preset.brush.shape_dynamics.size = dynamic([[0.0, 0.0], [0.25, 0.7], [1.0, 1.0]], 0.15);
    preset.brush.transfer.enabled = true;
    preset.brush.transfer.opacity = dynamic([[0.0, 0.2], [0.5, 0.3], [1.0, 0.9]], 0.25);
    preset.brush.transfer.flow = dynamic([[0.0, 1.0], [0.75, 0.6], [1.0, 0.1]], 0.35);
    preset.brush.texture.enabled = true;
    preset.brush.texture.each_tip = true;
    preset.brush.texture.csp_mode = Some(CspTextureMode::Outline);
    preset.brush.texture.pattern = Pattern::Tile(tip8(11, 7, 3));
    preset.brush.texture.depth_jitter = dynamic([[0.0, 0.4], [0.5, 0.8], [1.0, 0.5]], 0.45);
    preset.brush.csp_spray = Some(prism_paint::CspSpray {
        size: 2.5,
        count: 9,
        bias: -0.75,
        size_dynamic: dynamic([[0.0, 0.1], [0.4, 0.8], [1.0, 1.0]], 0.2),
        count_dynamic: dynamic([[0.0, 0.8], [0.7, 0.2], [1.0, 0.1]], 0.3),
        ..Default::default()
    });
    s.tools.presets.push(preset.clone());
    s.brush_presets_changed();
    s.sync_preset_store();
    assert_eq!(dir.files().iter().filter(|name| name.ends_with(".pcbrushes")).count(), 1);
    drop(s);

    let (mut restored, warnings) = session(&dir);
    assert!(warnings.is_empty(), "{warnings:?}");
    let loaded = prism_paint::presets::find(&restored.tools.presets, &preset.name).expect("saved sampled preset reloads");
    assert_eq!(loaded, &preset, "all effectors, floors, CSP texture modes and sampled assets must survive a fresh session");
    // Preset selection retains the current session's smoothing setting (BrushSettings::picked_over).
    let mut expected_selection = preset.brush.clone();
    expected_selection.smoothing = restored.tools.brush.smoothing.clone();
    restored.execute("tools.setBrush", json!({"preset": preset.name})).unwrap();
    assert_eq!(restored.tools.brush, expected_selection);
}

#[test]
fn imported_abr_group_persists() {
    let dir = TempDir::new("abr");
    let (mut s, _) = session(&dir);
    let tip = AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: (0..24).map(|i| i * 10).collect() };
    let abr = write_v12(
        2,
        &[
            LegacyBrush { name: "Grit".into(), spacing: 25, anti_alias: true, tip: LegacyTip::Sampled(tip) },
            LegacyBrush {
                name: "Round 9".into(),
                spacing: 10,
                anti_alias: true,
                tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
            },
        ],
        true,
    )
    .unwrap();
    let data = prism_paint::tile::b64_encode(&abr);
    s.execute("brush.presets.importAbr", json!({"data": data, "group": "Legacy Set"})).unwrap();
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    let grp: Vec<&BrushPreset> = t.tools.presets.iter().filter(|p| p.group == "Legacy Set").collect();
    assert_eq!(grp.len(), 2);
    assert_eq!(user(&t), user(&s));
    // Deleting every preset of the group removes its file and tips.
    let names: Vec<String> = grp.iter().map(|p| p.name.clone()).collect();
    for n in names {
        s.execute("brush.presets.delete", json!({"name": n})).unwrap();
    }
    assert!(!dir.files().iter().any(|f| f.ends_with(".pcbrushes") || f.starts_with("tips/")), "{:?}", dir.files());
}

#[test]
fn deleted_builtins_stay_deleted_and_overrides_replace_them() {
    let dir = TempDir::new("builtins");
    let (mut s, _) = session(&dir);
    let first = s.tools.presets[0].name.clone();
    let second = s.tools.presets[1].name.clone();
    s.execute("brush.presets.delete", json!({"name": first})).unwrap();
    s.tools.brush.size = 77.0;
    s.execute("brush.presets.save", json!({"name": second})).unwrap();
    let (t, _) = session(&dir);
    assert!(prism_paint::presets::find(&t.tools.presets, &first).is_none());
    let p = prism_paint::presets::find(&t.tools.presets, &second).unwrap();
    assert_eq!((p.builtin, p.brush.size), (false, 77.0));
    assert_eq!(t.tools.presets.iter().filter(|p| p.name.eq_ignore_ascii_case(&second)).count(), 1);
    // No built-in is ever written.
    let n = dir.files().iter().filter(|f| f.ends_with(".pcbrushes")).count();
    assert_eq!(n, 1);
}

#[test]
fn startup_with_many_groups_loads_them_all() {
    let dir = TempDir::new("many");
    let (mut s, _) = session(&dir);
    for g in 0..25 {
        for i in 0..4 {
            s.tools.presets.push(sampled(&format!("P{g}-{i}"), &format!("Group {g}"), tip8(16 + i, 16, g)));
        }
    }
    s.brush_presets_changed();
    s.sync_preset_store();
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(user(&t).len(), 100);
    let groups: Vec<String> = user(&t).iter().map(|p| p.group.clone()).fold(Vec::new(), |mut v, g| {
        if !v.contains(&g) {
            v.push(g);
        }
        v
    });
    assert_eq!(groups, (0..25).map(|g| format!("Group {g}")).collect::<Vec<_>>(), "group order survives");
}

#[test]
fn reordering_presets_and_groups_persists() {
    let names = |s: &Session| s.tools.presets.iter().map(|p| (p.name.clone(), p.group.clone())).collect::<Vec<_>>();
    let dir = TempDir::new("reorder");
    let (mut s, _) = session(&dir);
    for (n, g) in [("Ink 1", "Inks"), ("Ink 2", "Inks"), ("Wash", "Washes")] {
        s.execute("brush.presets.save", json!({"name": n})).unwrap();
        s.tools.presets.iter_mut().find(|p| p.name == n).unwrap().group = g.into();
    }
    s.brush_presets_changed();
    s.sync_preset_store();
    // Untouched order: no order list is written (the default order is implied).
    let (t, _) = session(&dir);
    assert_eq!(names(&t), names(&s));
    // Reorder a built-in within its group, a user preset across groups, and whole groups.
    let builtin_group = s.tools.presets[0].group.clone();
    let last_builtin = s.tools.presets.iter().rfind(|p| p.builtin && p.group == builtin_group).unwrap().name.clone();
    let first = s.tools.presets[0].name.clone();
    s.execute("brush.presets.move", json!({"name": last_builtin, "before": first})).unwrap();
    s.execute("brush.presets.move", json!({"name": "Ink 2", "group": "Washes", "index": 0})).unwrap();
    s.execute("brush.presets.moveGroup", json!({"group": "Washes", "index": 0})).unwrap();
    s.execute("brush.presets.moveGroup", json!({"group": "Inks", "before": builtin_group})).unwrap();
    assert_eq!(s.tools.presets[0].name, "Ink 2");
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(names(&t), names(&s), "the panel order survives a restart");
    assert!(t.tools.presets.iter().find(|p| p.name == last_builtin).unwrap().builtin, "reordering keeps a built-in built-in");
    // Renaming and deleting a group persist too.
    s.execute("brush.presets.renameGroup", json!({"group": "Inks", "newName": "Pens"})).unwrap();
    s.execute("brush.presets.deleteGroup", json!({"group": "Washes"})).unwrap();
    let (t, _) = session(&dir);
    assert_eq!(names(&t), names(&s));
    assert!(t.tools.presets.iter().all(|p| p.group != "Washes" && p.group != "Inks"));
}

#[test]
fn corrupt_and_oversized_files_are_skipped_with_a_warning() {
    let dir = TempDir::new("corrupt");
    let (mut s, _) = session(&dir);
    s.tools.presets.push(sampled("Good", "Fine", tip8(8, 8, 1)));
    s.tools.presets.push(sampled("Lost", "Broken tip", tip16(9, 9, 2)));
    s.brush_presets_changed();
    s.sync_preset_store();
    let be = DirBackend::new(&dir.0);
    be.write("junk-00000000.pcbrushes", b"{not json").unwrap();
    be.write("wrong-00000000.pcbrushes", br#"{"format":"other","version":1,"group":"x","presets":[]}"#).unwrap();
    // Corrupt the 16-bit tip of "Broken tip".
    let h = tip_hash(&tip16(9, 9, 2));
    be.write(&tip_file(&h), b"PCTIP1\x10garbage").unwrap();
    let (mut t, w) = session(&dir);
    assert_eq!(w.len(), 4, "{w:?}");
    assert!(prism_paint::presets::find(&t.tools.presets, "Good").is_some());
    assert!(prism_paint::presets::find(&t.tools.presets, "Lost").is_none());
    // The broken files are left alone for the user (never deleted by a sync).
    t.execute("brush.presets.save", json!({"name": "Another"})).unwrap();
    let files = dir.files();
    assert!(files.contains(&"junk-00000000.pcbrushes".to_string()) && files.contains(&tip_file(&h)), "{files:?}");

    // Oversized files are refused without reading them whole.
    let mem = MemBackend::default();
    mem.files.lock().unwrap().insert("big-00000000.pcbrushes".into(), vec![b' '; MAX_GROUP_BYTES as usize + 1]);
    mem.files.lock().unwrap().insert(INDEX_FILE.into(), b"[1,2".to_vec());
    let o = open(Box::new(mem));
    assert!(o.presets.is_empty());
    assert_eq!(o.warnings.len(), 2, "{:?}", o.warnings);
}

#[test]
fn write_failures_are_warnings() {
    let dir = TempDir::new("readonly");
    // The store "directory" is a file: every write fails.
    std::fs::write(&dir.0, b"not a dir").unwrap();
    let mut s = Session::new();
    s.attach_preset_store(open_dir(&dir.0));
    s.tools.brush.tip = TipShape::Sampled(tip8(4, 4, 0));
    s.execute("brush.presets.save", json!({"name": "Nope"})).unwrap();
    let w = s.preset_store.as_mut().unwrap().take_warnings();
    assert!(!w.is_empty());
    let _ = std::fs::remove_file(&dir.0);
}

#[test]
fn backends_refuse_paths_outside_the_store() {
    let mem = MemBackend::default();
    for n in ["../x.pcbrushes", "a/b.pcbrushes", "tips/../x", ".hidden", "", "tips/"] {
        assert!(mem.write(n, b"x").is_err(), "{n}");
    }
    assert!(group_file_name("../../etc/passwd").ends_with(".pcbrushes"));
    assert!(valid_name(&group_file_name("../../etc/passwd")));
    assert!(valid_name(&group_file_name("")));
    assert!(valid_name(&group_file_name("日本語 ブラシ")));
    assert_ne!(group_file_name("a/b"), group_file_name("a_b"));
}

/// `cargo test --release -p prism-engine --lib preset_store::tests::bench_load_500 -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_load_500_sampled_presets() {
    let dir = TempDir::new("bench");
    let (mut s, _) = session(&dir);
    for i in 0..500u32 {
        // A mix of 8-bit (most .abr tips) and 16-bit tips, 64..319 px.
        let side = 64 + (i * 37) % 256;
        let t = if i % 4 == 0 { tip16(side, side, i) } else { tip8(side, side, i) };
        s.tools.presets.push(sampled(&format!("Brush {i}"), &format!("Set {}", i / 50), t));
    }
    s.brush_presets_changed();
    let t0 = std::time::Instant::now();
    s.sync_preset_store();
    let write = t0.elapsed();
    let bytes = s.preset_store.as_ref().unwrap().bytes();
    let t0 = std::time::Instant::now();
    let o = open_dir(&dir.0);
    let load = t0.elapsed();
    assert_eq!(o.presets.len(), 500);
    let t0 = std::time::Instant::now();
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Brush 3", "newName": "Renamed"})).unwrap();
    let rename = t0.elapsed();
    eprintln!("500 sampled presets: {:.1} MB on disk; write {write:?}, load {load:?}, rename {rename:?}", bytes as f64 / 1e6);
}

#[test]
fn a_store_directory_that_does_not_exist_yet_opens_without_warnings() {
    // Windows reports a file under a missing directory as "path not found" (os error 3), not
    // "file not found"; both mean an empty store, as on a first launch.
    let parent = TempDir::new("missing-store");
    let dir = parent.0.join("not-created-yet");
    assert!(!dir.exists());
    let opened = open_dir(&dir);
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    assert!(opened.actions.is_empty());
    let err = DirBackend::new(&dir).read(ACTIONS_FILE, MAX_ACTIONS_BYTES).unwrap_err();
    assert!(missing_file(&err), "{err}");
}

#[test]
fn multi_tip_brush_survives_store_restart() {
    let dir = TempDir::new("multi-tip");
    let (mut s, warnings) = session(&dir);
    assert!(warnings.is_empty());
    let brush =
        BrushSettings { tip: TipShape::Sequence(vec![tip8(20, 16, 1), tip16(12, 12, 2)]), size: 17.0, csp_tip_scale: Some([1.0, 2.4]), ..Default::default() };
    s.tools.brush = brush.clone();
    s.execute("brush.presets.save", json!({"name":"Multi-tip"})).unwrap();
    s.sync_preset_store();
    let (t, warnings) = session(&dir);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(prism_paint::presets::find(&t.tools.presets, "Multi-tip").unwrap().brush.tip, brush.tip);
    assert_eq!(prism_paint::presets::find(&t.tools.presets, "Multi-tip").unwrap().brush.csp_tip_scale, brush.csp_tip_scale);
}

#[test]
fn colour_tips_and_tip_order_are_stored_as_tip_files() {
    use prism_paint::{CspTipColor, CspTipOrder};
    let dir = TempDir::new("colour-tips");
    let (mut s, warnings) = session(&dir);
    assert!(warnings.is_empty());
    let rgb = |w: u32, h: u32| CspTipColor { width: w, height: h, rgb: (0..w * h * 3).map(|i| (i * 37 % 256) as u8).collect() };
    let mut preset = sampled("Colour tips", "CSP", tip8(20, 16, 1));
    preset.brush.tip = TipShape::Sequence(vec![tip8(20, 16, 1), tip16(12, 12, 2), tip8(9, 5, 3)]);
    preset.brush.csp_tip_colors = vec![Some(rgb(20, 16)), None, Some(rgb(9, 5))];
    preset.brush.csp_tip_order = Some(CspTipOrder::Reverse);
    // Colours that do not fit their size are dropped, not saved as a broken tip.
    let mut broken = sampled("Broken colours", "CSP", tip8(4, 4, 9));
    broken.brush.csp_tip_colors = vec![Some(CspTipColor { width: 4, height: 4, rgb: vec![1, 2, 3] })];
    s.tools.presets.extend([preset.clone(), broken]);
    s.brush_presets_changed();
    s.sync_preset_store();
    let files = dir.files();
    // Three tips, the 12×12 one 16-bit; two colour tips of three 8-bit planes each.
    assert_eq!(files.iter().filter(|f| f.starts_with("tips/")).count(), 4 + 6, "{files:?}");
    let group = files.iter().find(|f| f.ends_with(".pcbrushes")).unwrap();
    let json = std::fs::read_to_string(dir.0.join(group)).unwrap();
    assert!(!json.contains("cspTipColors") && json.contains("tipColors") && json.contains("\"sequence\""), "bitmaps live in tip files");
    let (t, warnings) = session(&dir);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(prism_paint::presets::find(&t.tools.presets, "Colour tips").unwrap().brush, preset.brush);
    assert!(prism_paint::presets::find(&t.tools.presets, "Broken colours").unwrap().brush.csp_tip_colors.iter().all(Option::is_none));
    // A missing colour plane skips the group with a warning, like a missing tip.
    let green = rgb(9, 5).rgb.iter().skip(1).step_by(3).map(|v| u16::from(*v) * 257).collect();
    std::fs::remove_file(dir.0.join(tip_file(&tip_hash(&GrayTile { width: 9, height: 5, data: green })))).unwrap();
    let (u, warnings) = session(&dir);
    assert!(warnings.iter().any(|w| w.contains("missing or unreadable tips")), "{warnings:?}");
    assert!(prism_paint::presets::find(&u.tools.presets, "Colour tips").is_none());
}

#[test]
fn sut_display_metadata_survives_store_restart_and_restore() {
    use prism_paint::sut::{SutBrushMetadata, SutDisplay};
    let dir = TempDir::new("sut-display");
    let (mut s, warnings) = session(&dir);
    assert!(warnings.is_empty());
    let mut preset = sampled("SUT sample", "SUT", tip8(7, 9, 1));
    preset.sut = Some(SutBrushMetadata {
        original: SutDisplay { visible: vec![1000, 10], show_categories: false },
        custom: None,
        input_op: 1,
        output_op: 2,
        parameters: [
            ("Opacity".into(), 90.0),
            ("PrismTextureScaleFactor".into(), 2.0),
            ("FillReferMultiLayer".into(), 1.0),
            ("FillColorMargin".into(), 20.0),
            ("FillContinuousPixel".into(), 1.0),
            ("FillAntiAlias".into(), 1.0),
            ("GradationShapeKind".into(), 2.0),
            ("GradationDither".into(), 1.0),
            ("GradationReverseSaved".into(), 0.0),
            ("TextFontSize".into(), 12.0),
            ("TextVertical".into(), 1.0),
        ]
        .into(),
        node_icon: 0,
        texture_name: "Paper".into(),
    });
    s.tools.presets.push(preset);
    s.execute("brush.sutDisplay.set", json!({"name":"SUT sample","parameter":10,"visible":false,"showCategories":true})).unwrap();
    s.sync_preset_store();
    let expected = prism_paint::presets::find(&s.tools.presets, "SUT sample").unwrap().clone();
    let (mut restored, warnings) = session(&dir);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(prism_paint::presets::find(&restored.tools.presets, "SUT sample"), Some(&expected));
    restored.execute("tools.setBrush", json!({"preset":"SUT sample"})).unwrap();
    restored.execute("brush.sutDisplay.reset", json!({})).unwrap();
    restored.sync_preset_store();
    let (again, warnings) = session(&dir);
    assert!(warnings.is_empty());
    let metadata = prism_paint::presets::find(&again.tools.presets, "SUT sample").unwrap().sut.as_ref().unwrap();
    assert!(metadata.custom.is_none());
    assert_eq!(metadata.original.visible, [1000, 10]);
}

#[test]
fn malformed_sut_metadata_keeps_valid_brush_and_bounds_presentation() {
    let dir = TempDir::new("sut-corrupt");
    let (mut s, _) = session(&dir);
    s.execute("brush.presets.save", json!({"name":"Keep brush"})).unwrap();
    s.sync_preset_store();
    let file = dir.files().into_iter().find(|f| f.ends_with(GROUP_EXT)).unwrap();
    let backend = DirBackend::new(&dir.0);
    let mut group: serde_json::Value = serde_json::from_slice(&backend.read(&file, MAX_GROUP_BYTES).unwrap()).unwrap();
    group["presets"][0]["sut"] = json!({"original":{"visible":[1000]}});
    backend.write(&file, &serde_json::to_vec(&group).unwrap()).unwrap();
    let opened = open_dir(&dir.0);
    assert_eq!(opened.presets.len(), 1);
    assert_eq!(opened.presets[0].name, "Keep brush");
    assert!(opened.presets[0].sut.is_none());
    assert!(opened.warnings.iter().any(|warning| warning.contains("SUT display metadata")));
    let mut warnings = Vec::new();
    let metadata = read_sut_metadata(
        Some(json!({
            "original":{"visible":(0..700).collect::<Vec<_>>(),"show_categories":false},
            "input_op":10,"output_op":10,"parameters":{"Opacity":90,"Arbitrary":2,"BrushSize":1e20},
            "texture_name":"a".repeat(2048)
        })),
        "Bounded",
        &mut warnings,
    )
    .unwrap();
    assert_eq!(metadata.original.visible.len(), prism_paint::sut::MAX_SUT_DISPLAY_ITEMS);
    assert_eq!(metadata.texture_name.len(), 1024);
    assert_eq!(metadata.parameters.len(), 1);
    assert!(!warnings.is_empty());
}
