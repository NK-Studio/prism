use super::*;

fn n(value: u32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}
fn unicode(text: &str) -> Vec<u8> {
    let units: Vec<_> = text.encode_utf16().chain([0]).collect();
    [n(units.len() as u32), units.iter().flat_map(|v| v.to_be_bytes()).collect()].concat()
}
fn id(text: &str) -> Vec<u8> {
    [n(if text.len() == 4 { 0 } else { text.len() as u32 }), text.as_bytes().to_vec()].concat()
}
fn class(name: &str) -> Vec<u8> {
    [unicode(""), id(name)].concat()
}
fn descriptor(items: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut bytes = [class("null"), n(items.len() as u32)].concat();
    for (key, value) in items {
        bytes.extend(id(key));
        bytes.extend(value);
    }
    bytes
}
fn typed(tag: &[u8; 4], bytes: Vec<u8>) -> Vec<u8> {
    [tag.to_vec(), bytes].concat()
}
fn step(event: &str, enabled: bool, dialog: bool, data: Option<Vec<u8>>) -> Vec<u8> {
    let event_bytes = if event.len() == 4 {
        [b"long".to_vec(), event.as_bytes().to_vec()].concat()
    } else {
        [b"TEXT".to_vec(), n(event.len() as u32), event.as_bytes().to_vec()].concat()
    };
    [
        vec![1, u8::from(enabled), u8::from(dialog), 2],
        event_bytes,
        n(event.len() as u32),
        event.as_bytes().to_vec(),
        n(if data.is_some() { u32::MAX } else { 0 }),
        data.unwrap_or_default(),
    ]
    .concat()
}
fn action(name: &str, steps: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = [vec![0, 3, 1, 0, 0, 2], unicode(name), vec![1], n(steps.len() as u32)].concat();
    for step in steps {
        bytes.extend(step);
    }
    bytes
}
fn file(set: &str, actions: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = [n(16), unicode(set), vec![1], n(actions.len() as u32)].concat();
    for action in actions {
        bytes.extend(action);
    }
    bytes
}
fn load(session: &mut Session, bytes: &[u8]) -> Result<Value> {
    session.execute("actions.importAtn", json!({"data":prism_paint::tile::b64_encode(bytes)}))
}
fn document(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"depth":depth,"background":"transparent"})).unwrap();
    let d = s.active_mut().unwrap();
    let layer = d.active_layer.unwrap();
    std::sync::Arc::make_mut(&mut d.doc).layer_mut(layer).unwrap().surface_mut().unwrap().write_pixel(1, 1, &[0.2, 0.4, 0.6, 1.0]);
    s
}
fn pixel(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(1, 1)
}
fn named_call(set: &str, action: &str) -> Vec<u8> {
    let refs = [n(2), b"name".to_vec(), class("Actn"), unicode(action), b"name".to_vec(), class("ASet"), unicode(set)].concat();
    step("Ply ", true, false, Some(descriptor(vec![("null", typed(b"obj ", refs))])))
}

#[test]
fn inline_import_only_adds_definitions_and_preserves_source_and_flags() {
    let radius = descriptor(vec![("Rds ", typed(b"UntF", [b"#Pxl".to_vec(), 2.0f64.to_be_bytes().to_vec()].concat()))]);
    let bytes =
        file("$$$/Test/Set=Synthetic ATN", vec![action("Example", vec![step("Invr", true, true, None), step("GsnB", false, false, Some(radius.clone()))])]);
    let mut s = document(8);
    let before = pixel(&s);
    let imported = load(&mut s, &bytes).unwrap();
    assert_eq!(imported["count"], 1);
    assert_eq!(imported["unsupported"], 0);
    assert_eq!(pixel(&s), before);
    assert!(s.actions.paused.is_none());
    let a = &s.actions.list[0];
    assert_eq!(a.set, "Synthetic ATN");
    assert!(a.steps[0].enabled && a.steps[0].dialog);
    assert!(!a.steps[1].enabled && !a.steps[1].dialog);
    assert_eq!(a.steps[1].params["radius"], 2.0);
    let source = a.steps[1].source.as_ref().unwrap();
    assert_eq!((source.format.as_str(), source.command.as_str(), source.index), ("atn", "GsnB", 1));
    let details = source.details.as_ref().unwrap();
    assert_eq!(details["descriptor"], prism_paint::tile::b64_encode(&radius));
    assert_eq!(details["shortcutIndex"], 3);
    assert_eq!(details["shiftKey"], true);
    assert_eq!(details["dialogOptions"], 2);
    let roundtrip: Vec<Action> = serde_json::from_slice(&serde_json::to_vec(&s.actions.list).unwrap()).unwrap();
    assert_eq!(roundtrip, s.actions.list);
    let paused = s.execute("actions.play", json!({"action":"Example"})).unwrap();
    assert_eq!(paused["paused"]["kind"], "dialog");
    assert_eq!(pixel(&s), before);
    let resumed = s.execute("actions.resume", json!({})).unwrap();
    assert_eq!(resumed["ran"], 1);
    assert_ne!(pixel(&s), before);
}

#[test]
fn malformed_imports_and_duplicate_conflicts_are_atomic_and_replace_is_explicit() {
    let original = file("Test", vec![action("Existing", vec![step("Invr", true, false, None)])]);
    let mut s = Session::new();
    load(&mut s, &original).unwrap();
    let before = s.actions.list.clone();
    for p in [json!({}), json!({"data":"%%%"}), json!({"data":prism_paint::tile::b64_encode(&original),"replace":3})] {
        assert!(s.execute("actions.importAtn", p).is_err());
        assert_eq!(s.actions.list, before);
    }
    let conflicting = file("Test", vec![action("New First", vec![]), action("Existing", vec![step("Dstt", true, false, None)])]);
    assert!(load(&mut s, &conflicting).is_err());
    assert_eq!(s.actions.list, before);
    let duplicates = file("Test", vec![action("Duplicate", vec![]), action("Duplicate", vec![])]);
    assert!(load(&mut s, &duplicates).is_err());
    assert_eq!(s.actions.list, before);
    let replaced = s.execute("actions.importAtn", json!({"data":prism_paint::tile::b64_encode(&conflicting),"replace":true})).unwrap();
    assert_eq!(replaced["count"], 2);
    assert_eq!(s.actions.list.len(), 2);
    assert_eq!(s.actions.list.iter().find(|a| a.name == "Existing").unwrap().steps[0].id, "image.adjustments.desaturate");
}

#[test]
fn unsupported_step_blocks_before_edits_but_disabled_step_is_skipped() {
    let bytes = file("Test", vec![action("Blocked", vec![step("Invr", true, false, None), step("notVerified", true, false, None)])]);
    let mut s = document(8);
    let before = pixel(&s);
    let result = load(&mut s, &bytes).unwrap();
    assert_eq!(result["unsupported"], 1);
    assert_eq!(result["imported"][0]["ready"], false);
    assert_eq!(s.actions.list[0].steps[1].id, UNSUPPORTED);
    assert!(s.actions.list[0].steps[1].enabled);
    let error = s.execute("actions.play", json!({"action":"Blocked"})).unwrap_err();
    assert!(error.to_string().contains("notVerified"));
    assert_eq!(pixel(&s), before);
    assert_eq!(s.actions.playing, 0);
    let plan = s.execute("actions.get", json!({"action":"Blocked"})).unwrap();
    assert_eq!(plan["steps"].as_array().unwrap().len(), 1);
    assert_eq!(plan["steps"][0][0], UNSUPPORTED);
    s.execute("actions.configure", json!({"action":"Blocked","step":1,"enabled":false})).unwrap();
    let played = s.execute("actions.play", json!({"action":"Blocked"})).unwrap();
    assert_eq!(played["ran"], 1);
    assert_ne!(pixel(&s), before);
}

#[test]
fn nested_imported_call_preflights_before_parent_edits_and_preserves_call_semantics() {
    let bytes = file(
        "Test",
        vec![
            action("Parent", vec![step("Invr", true, false, None), named_call("Test", "Child")]),
            action("Child", vec![step("Invr", true, false, None), step("notVerified", true, false, None)]),
        ],
    );
    let mut s = document(16);
    let imported = load(&mut s, &bytes).unwrap();
    assert_eq!(imported["imported"][0]["ready"], false);
    assert_eq!(imported["imported"][0]["unsupportedEnabled"], 0);
    assert!(imported["imported"][0]["blockedReason"].as_str().unwrap().contains("Child"));
    assert_eq!(s.actions.list[0].steps[1].id, "actions.play");
    let before = pixel(&s);
    assert!(s.execute("actions.play", json!({"action":"Parent"})).is_err());
    assert_eq!(pixel(&s), before);
    s.execute("actions.configure", json!({"action":"Child","step":1,"enabled":false})).unwrap();
    let played = s.execute("actions.play", json!({"action":"Parent"})).unwrap();
    assert_eq!(played["ran"], 2);
    let after = pixel(&s);
    assert!(before.iter().zip(after).all(|(a, b)| (a - b).abs() < 0.0001));
}

#[test]
fn supported_inversion_executes_at_8_16_and_32_bits() {
    let bytes = file("Test", vec![action("Invert", vec![step("Invr", true, false, None)])]);
    for depth in [8, 16, 32] {
        let mut s = document(depth);
        load(&mut s, &bytes).unwrap();
        let played = s.execute("actions.play", json!({"action":"Invert"})).unwrap();
        assert_eq!(played["ran"], 1, "{depth}: {played}");
        let p = pixel(&s);
        for (actual, expected) in p.iter().zip([0.8, 0.6, 0.4, 1.0]) {
            assert!((actual - expected).abs() < 0.01, "{depth}: {p:?}");
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert!((pixel(&s)[0] - 0.2).abs() < 0.01);
    }
}

#[test]
fn unmapped_parameters_remain_visible_instead_of_silently_changing_the_operation() {
    let bytes = file(
        "Test",
        vec![action(
            "Unknown settings",
            vec![
                step("Invr", true, false, Some(descriptor(vec![("extra", typed(b"bool", vec![1]))]))),
                step("GsnB", false, true, Some(descriptor(vec![("Rds ", typed(b"UntF", [b"#Prc".to_vec(), 2.0f64.to_be_bytes().to_vec()].concat()))]))),
            ],
        )],
    );
    let mut s = Session::new();
    let result = load(&mut s, &bytes).unwrap();
    assert_eq!(result["unsupported"], 2);
    assert_eq!(result["imported"][0]["unsupportedEnabled"], 1);
    assert!(s.actions.list[0].steps.iter().all(|s| s.id == UNSUPPORTED));
    assert!(!s.actions.list[0].steps[1].enabled && s.actions.list[0].steps[1].dialog);
    let saved = serde_json::to_value(&s.actions.list).unwrap();
    let restored: Vec<Action> = serde_json::from_value(saved).unwrap();
    assert_eq!(restored, s.actions.list);
}

#[test]
#[ignore = "reads PRISM_TEST_ATN_FILES JSON array of local files; never executes installed action definitions"]
fn installed_atn_import_coverage_and_registered_commands() {
    let paths: Vec<String> = serde_json::from_str(&std::env::var("PRISM_TEST_ATN_FILES").unwrap()).unwrap();
    assert!(!paths.is_empty());
    let mut total = (0usize, 0usize, 0usize, 0usize);
    for path in paths {
        let original = std::fs::read(&path).unwrap();
        let mut s = Session::new();
        let report = s.execute("actions.importAtn", json!({"path":path})).unwrap_or_else(|e| panic!("{path}: {e}"));
        let steps: Vec<_> = s.actions.list.iter().flat_map(|a| &a.steps).collect();
        let unsupported = steps.iter().filter(|s| s.id == UNSUPPORTED).count();
        let mapped = steps.len() - unsupported;
        let ready = s.actions.list.iter().filter(|a| preflight(&s.actions.list, a, 0).is_ok()).count();
        for step in &steps {
            assert!(crate::commands::find(&step.id).is_some(), "{path}: unregistered {}", step.id);
        }
        assert_eq!(report["unsupported"], unsupported);
        assert!(s.active().is_none());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        println!("{path}: {} actions, {mapped} mapped steps, {unsupported} unsupported steps, {ready} ready actions", s.actions.list.len());
        total.0 += s.actions.list.len();
        total.1 += mapped;
        total.2 += unsupported;
        total.3 += ready;
    }
    assert!(total.1 > 0 && total.2 > 0 && total.3 > 0);
    println!("total: {} actions, {} mapped steps, {} unsupported steps, {} ready actions", total.0, total.1, total.2, total.3);
}

#[test]
fn duplicate_names_across_sets_keep_distinct_nested_targets_and_persist() {
    let mut bytes = file("First", vec![action("Shared", vec![step("Invr", true, false, None)]), action("Caller", vec![named_call("Second", "Shared")])]);
    bytes.extend(file("Second", vec![action("Shared", vec![step("Dstt", true, false, None)])]).into_iter().skip(4));
    let directory = std::env::temp_dir().join(format!("prism-atn-persistence-{}", std::process::id()));
    let mut s = Session::new();
    assert!(s.attach_preset_store(crate::preset_store::open_dir(&directory)).is_empty());
    let report = load(&mut s, &bytes).unwrap();
    assert_eq!(report["count"], 3);
    assert_eq!(s.actions.list[0].name, "First / Shared");
    assert_eq!(s.actions.list[1].steps[0].params["action"], "Second / Shared");
    assert_eq!(s.actions.list[2].name, "Second / Shared");
    s.sync_preset_store();
    let mut restored = Session::new();
    assert!(restored.attach_preset_store(crate::preset_store::open_dir(&directory)).is_empty());
    assert_eq!(restored.actions.list, s.actions.list);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn repeated_source_metadata_is_bounded_before_library_mutation() {
    let bytes = file(&"S".repeat(255), vec![action(&"A".repeat(255), vec![step("Invr", true, false, None); 40_000])]);
    assert!(bytes.len() < 4 << 20);
    let mut s = Session::new();
    let error = load(&mut s, &bytes).unwrap_err();
    assert!(error.to_string().contains("persistence size limit"));
    assert!(s.actions.list.is_empty());
}

#[test]
fn import_checks_the_pretty_json_size_used_by_persistence() {
    // Compute a small-file / many-steps boundary from the actual persisted schema,
    // keeping this regression stable when provenance gains another field.
    let one_step = step("Invr", true, false, None);
    let mut probe = Session::new();
    load(&mut probe, &file("Sizes", vec![action("Bounded", vec![one_step.clone()])])).unwrap();
    let mut expected = crate::actions_cmds::ActionsFile { version: 2, actions: probe.actions.list.clone() };
    let first = expected.actions[0].steps[0].clone();
    let one_len = serde_json::to_vec_pretty(&expected).unwrap().len();
    expected.actions[0].steps.push(first.clone());
    let per_step = serde_json::to_vec_pretty(&expected).unwrap().len() - one_len;
    let limit = crate::preset_store::MAX_ACTIONS_BYTES as usize;
    let count = limit / per_step + 128;
    assert!(count < 100_000);
    expected.actions[0].steps.resize(count, first);
    for (index, item) in expected.actions[0].steps.iter_mut().enumerate() {
        item.source.as_mut().unwrap().index = index;
    }
    assert!(serde_json::to_vec(&expected).unwrap().len() < limit, "compact representation fits");
    assert!(serde_json::to_vec_pretty(&expected).unwrap().len() > limit, "the actual saved representation does not fit");
    drop(expected);
    drop(probe);
    let bytes = file("Sizes", vec![action("Bounded", vec![one_step; count])]);
    assert!(bytes.len() < 4 << 20);
    let mut s = Session::new();
    let error = load(&mut s, &bytes).unwrap_err();
    assert!(error.to_string().contains("persistence size limit"), "{error}");
    assert!(s.actions.list.is_empty());
}

#[test]
fn action_count_across_sets_is_rejected_before_merging_empty_actions() {
    let actions = (0..4096).map(|i| action(&format!("Empty {i}"), vec![])).collect();
    let mut bytes = file("First", actions);
    bytes.extend(file("Second", vec![action("One more", vec![])]).into_iter().skip(4));
    assert!(bytes.len() < 256 << 10);
    let mut s = Session::new();
    let error = load(&mut s, &bytes).unwrap_err();
    assert!(error.to_string().contains("4096"), "{error}");
    assert!(s.actions.list.is_empty());
}

#[test]
fn nested_float_start_uses_the_same_step_index_as_playback() {
    let bytes = file("Test", vec![action("Child", vec![step("notVerified", true, false, None), step("Invr", true, false, None)])]);
    let mut s = document(16);
    load(&mut s, &bytes).unwrap();
    s.actions.list.push(Action::new("Caller", vec![("actions.play".into(), json!({"action":"Child","from":1.0}))]));
    let before = pixel(&s);
    let played = s.execute("actions.play", json!({"action":"Caller"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert!(played.get("failed").is_none(), "{played}");
    assert_ne!(pixel(&s), before);
    assert!(s.actions.list[0].steps[0].enabled, "from skips the source step without changing its flag");
}

#[test]
fn dialog_override_preflights_the_new_child_before_running_its_first_edit() {
    let bytes = file("Test", vec![action("Safe", vec![]), action("Unsafe", vec![step("Invr", true, false, None), step("notVerified", true, false, None)])]);
    let mut s = document(32);
    load(&mut s, &bytes).unwrap();
    let mut caller = Action::new("Caller", vec![("actions.play".into(), json!({"action":"Safe"}))]);
    caller.steps[0].dialog = true;
    s.actions.list.push(caller);
    let before = pixel(&s);
    let paused = s.execute("actions.play", json!({"action":"Caller"})).unwrap();
    assert_eq!(paused["paused"]["kind"], "dialog");
    assert_eq!(pixel(&s), before);
    let resumed = s.execute("actions.resume", json!({"params":{"action":"Unsafe"}})).unwrap();
    assert!(resumed["failed"]["error"].as_str().unwrap().contains("notVerified"), "{resumed}");
    assert_eq!(pixel(&s), before, "even the new child's first inversion must not run");
    assert!(s.actions.paused.is_none());
    assert_eq!(s.actions.playing, 0);
}
