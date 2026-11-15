//! Import CLIP STUDIO auto actions without executing them. Steps with a Prism equivalent
//! (`laf_mapping`) become ordinary action steps; the rest are kept as placeholders that refuse
//! playback until disabled or replaced, as Photoshop action import does.
use crate::{
    EngineError, Result, Session,
    actions_cmds::{Action, ActionStep},
    commands::CommandSpec,
};
use serde_json::{Value, json};
const ID: &str = "actions.importLaf";
/// A CLIP STUDIO step Prism cannot run yet, kept so the action can be inspected and edited.
pub(crate) const UNSUPPORTED: &str = "actions.unsupportedLafStep";
fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}
fn name(v: &Value, key: &str) -> Result<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.chars().count() <= 255)
        .map(str::to_owned)
        .ok_or_else(|| bad(format!("invalid `{key}`")))
}
fn flag(v: &Value, key: &str) -> Result<bool> {
    match v.get(key).and_then(Value::as_u64) {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(bad(format!("unsupported `{key}`"))),
    }
}

/// Compiled actions, and what differs from CLIP STUDIO (deduplicated).
struct Compiled {
    actions: Vec<Action>,
    notes: Vec<String>,
}

/// `name`, or `name (2)`, `name (3)` … when an earlier action of the same LAF has it (CLIP
/// STUDIO allows repeats, such as separator actions named with dashes).
fn unique_name(taken: &[Action], name: &str) -> Result<String> {
    if !taken.iter().any(|a| a.name == name) {
        return Ok(name.to_string());
    }
    (2..=taken.len() + 1)
        .map(|n| format!("{name} ({n})"))
        .find(|n| n.chars().count() <= 255 && !taken.iter().any(|a| &a.name == n))
        .ok_or_else(|| bad("LAF action names collide"))
}

fn compile(v: &Value) -> Result<Compiled> {
    let source = v
        .get("actionArray")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= 256)
        .ok_or_else(|| bad("LAF has no actions (or more than 256)"))?;
    let set: String = v.get("actionSetName").and_then(Value::as_str).filter(|s| !s.trim().is_empty() && s.chars().count() <= 255).unwrap_or("CLIP STUDIO").into();
    let mut actions: Vec<Action> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for a in source {
        if a.get("typename").and_then(Value::as_str) != Some("typeAction") {
            return Err(bad("invalid LAF action"));
        }
        let name = unique_name(&actions, &name(a, "actionName")?)?;
        // An action without steps is a label CLIP STUDIO users put between real actions.
        let items = a.get("actionItemArray").and_then(Value::as_array).filter(|a| a.len() <= 4096).ok_or_else(|| bad("invalid LAF action steps"))?;
        let mut steps: Vec<ActionStep> = Vec::new();
        for (source_index, item) in items.iter().enumerate() {
            if item.get("typename").and_then(Value::as_str) != Some("typeActionItem") {
                return Err(bad("invalid LAF action step"));
            }
            let enabled = flag(item, "actionItemEnable")?;
            let dialog = flag(item, "actionItemShowSettings")?;
            let kind = name_of_step(item)?;
            let params = item.get("actionItemParameter").and_then(Value::as_object).ok_or_else(|| bad("missing LAF action parameters"))?;
            let source = |details: Option<Value>| crate::action_model::StepSource { format: "laf".into(), command: kind.into(), index: source_index, details };
            match crate::laf_mapping::map(kind, params) {
                Ok(mapped) => {
                    if let Some(note) = mapped.note.filter(|n| !notes.contains(n)) {
                        notes.push(note);
                    }
                    for (part, (id, params)) in mapped.steps.into_iter().enumerate() {
                        let mut step: ActionStep = (id, params).into();
                        step.enabled = enabled;
                        step.dialog = dialog && part == 0;
                        step.source = Some(source(None));
                        steps.push(step);
                    }
                }
                // Kept with its source parameters, so the action keeps its shape and the step
                // can be disabled or replaced; playback refuses it before changing anything.
                Err(reason) => steps.push(ActionStep {
                    id: UNSUPPORTED.into(),
                    params: json!({"kind": kind, "reason": reason}),
                    enabled,
                    dialog: false,
                    source: Some(source(Some(json!({"parameters": params, "dialog": dialog})))),
                }),
            }
        }
        actions.push(Action { name, set: set.clone(), steps });
    }
    Ok(Compiled { actions, notes })
}
fn name_of_step(v: &Value) -> Result<&str> {
    v.get("actionItemKind")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty() && k.chars().count() <= 255)
        .ok_or_else(|| bad("missing or invalid action kind"))
}
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    if s.actions.recording.is_some() || s.actions.playing > 0 || s.actions.paused.is_some() {
        return Err(bad("stop action recording or playback before importing"));
    }
    let replace = match p.get("replace") {
        None => false,
        Some(Value::Bool(v)) => *v,
        _ => return Err(bad("`replace` must be a boolean")),
    };
    // A downloaded set by its CLIP STUDIO material id, or a file / bytes.
    let bytes = match p.get("uuid") {
        Some(v) => {
            if p.get("path").is_some() || p.get("data").is_some() {
                return Err(bad("pass one of `uuid`, `path` or `data`"));
            }
            let uuid = v.as_str().map(str::trim).filter(|u| !u.is_empty() && u.len() <= 128).ok_or_else(|| bad("`uuid` must be an auto-action id from brush.cspLibrary.list"))?;
            let root = crate::csp_library_cmds::root(ID, p)?;
            prism_io::sut::library::action_bytes(&root, uuid).map_err(bad)?.1
        }
        None => crate::preset_import_cmds::file_bytes_with_limit(p, ID, ".laf file", 8 << 20)?.0,
    };
    let source = prism_io::laf::read_laf(&bytes, &prism_raster::Interrupt::NONE).map_err(bad)?;
    let Compiled { actions, notes } = compile(&source)?;
    if !replace && actions.iter().any(|a| s.actions.list.iter().any(|old| old.name == a.name)) {
        return Err(bad("an action with that name exists; pass `replace:true` to replace it"));
    }
    let names: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    let mut result = s.actions.list.clone();
    for a in actions {
        if let Some(old) = result.iter_mut().find(|old| old.name == a.name) {
            *old = a;
        } else {
            if result.len() >= 4096 {
                return Err(bad("action library exceeds 4096 actions"));
            }
            result.push(a);
        }
    }
    // Match PresetStore::flush_actions, so an import never leaves a library it cannot save.
    let saved = serde_json::to_vec_pretty(&crate::actions_cmds::ActionsFile { version: 2, actions: result.clone() }).map_err(|e| bad(e.to_string()))?;
    if saved.len() as u64 > crate::preset_store::MAX_ACTIONS_BYTES {
        return Err(bad("imported action library exceeds the persistence size limit"));
    }
    let mut unsupported = 0usize;
    let mut imported = Vec::new();
    for name in &names {
        let Some(a) = result.iter().find(|a| &a.name == name) else { continue };
        let blocked: Vec<&str> = a.steps.iter().filter(|st| st.enabled && st.id == UNSUPPORTED).filter_map(|st| st.params.get("kind").and_then(Value::as_str)).collect();
        unsupported += a.steps.iter().filter(|st| st.id == UNSUPPORTED).count();
        let mut entry = json!({"name": a.name, "set": a.set, "steps": a.steps.len(), "unsupportedEnabled": blocked.len(), "ready": blocked.is_empty()});
        if !blocked.is_empty() {
            let mut kinds: Vec<&str> = Vec::new();
            for k in blocked {
                if !kinds.contains(&k) {
                    kinds.push(k);
                }
            }
            entry["unsupportedKinds"] = json!(kinds);
        }
        if let Err(reason) = crate::atn_import_cmds::preflight(&result, a, 0) {
            entry["ready"] = json!(false);
            entry["blockedReason"] = json!(reason);
        }
        imported.push(entry);
    }
    s.actions.list = result;
    s.actions.touch();
    let mut warnings = vec!["CLIP STUDIO steps were mapped to Prism commands; pixel-identical results are not guaranteed. Step enable and dialog flags were preserved.".to_string()];
    if unsupported > 0 {
        warnings.insert(
            0,
            format!("{unsupported} CLIP STUDIO steps have no Prism equivalent yet and were kept as placeholders. Disable or replace those steps before playing their actions."),
        );
    }
    warnings.extend(notes);
    Ok(json!({"format":"laf","count":imported.len(),"imported":imported,"unsupported":unsupported,"warnings":warnings}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: ID,
            label: "Import CLIP STUDIO Auto Actions…",
            menu: &[],
            shortcut: None,
            params: r#"{"path":".laf file"?,"data":base64 bytes?,"uuid":auto-action id from brush.cspLibrary.list actions?,"root":CLIP STUDIO user data folder? (with uuid),"replace":bool=false} → {count, imported:[{name,set,steps,unsupportedEnabled,unsupportedKinds?,ready,blockedReason?}], unsupported, warnings}"#,
            enabled: |_| Ok(()),
            run: import,
            journal: false,
        },
        CommandSpec {
            id: UNSUPPORTED,
            label: "Unsupported CLIP STUDIO Step",
            menu: &[],
            shortcut: None,
            params: "Imported CLIP STUDIO step with no Prism equivalent yet; disable or replace it before playback.",
            enabled: |_| Ok(()),
            run: |_, p| {
                Err(bad(format!(
                    "unsupported CLIP STUDIO step {}: {}",
                    p.get("kind").and_then(Value::as_str).unwrap_or("unknown"),
                    p.get("reason").and_then(Value::as_str).unwrap_or("disable or replace this step")
                )))
            },
            journal: false,
        },
        CommandSpec {
            id: "edit.clearOutsideSelection",
            label: "Clear Outside Selection",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?,"target":"mask"?} Clear the inverse selection in one undo step, preserving the selection. On an adjustment or fill layer (or with target "mask") it hides the outside in the layer mask, adding one that reveals all first."#,
            enabled: |_| Ok(()),
            run: clear_outside,
            journal: true,
        },
    ]
}

fn clear_outside(s: &mut Session, p: &Value) -> Result<Value> {
    let id = crate::commands::layer_param(s, p)?;
    let bg = s.tools.background;
    // Adjustment and fill layers have no pixels: like Photoshop's Delete with such a layer
    // targeted (and CLIP STUDIO's Colored Outline action), the mask takes the edit.
    let on_mask = crate::commands::is_mask_target(p)
        || s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| matches!(l.content, prism_doc::LayerContent::Adjustment(_) | prism_doc::LayerContent::Fill(_)));
    if on_mask {
        return clear_mask_outside(s, id);
    }
    s.edit("Clear Outside Selection", |doc, _| {
        let selection = doc.selection.as_ref().ok_or_else(|| bad("no selection"))?;
        let bounds = doc.bounds();
        let area = i64::from(bounds.width()).checked_mul(i64::from(bounds.height())).ok_or_else(|| bad("canvas too large"))?;
        if !(0..=64_000_000).contains(&area) {
            return Err(bad("Clear Outside Selection is limited to 64 million pixels"));
        }
        let mut inverse = prism_raster::Surface::new(selection.format());
        // Work in bounded row strips rather than allocating an entire canvas buffer.
        let mut y = bounds.y0;
        while y < bounds.y1 {
            let end = y.saturating_add(32).min(bounds.y1);
            let strip = prism_geom::Rect { x0: bounds.x0, y0: y, x1: bounds.x1, y1: end };
            let data: Vec<_> = selection.read_region(strip).into_iter().map(|v| 1.0 - v).collect();
            inverse.write_region(strip, &data);
            y = end;
        }
        crate::edit_cmds::clear_area(doc, id, bounds, Some(&inverse), bg)
    })?;
    Ok(Value::Null)
}

/// Hide the layer mask outside the selection: inside the canvas the mask becomes its value times
/// the selection, everywhere else hidden. A layer without a mask gets one (revealing all) first.
fn clear_mask_outside(s: &mut Session, id: prism_doc::LayerId) -> Result<Value> {
    s.edit("Clear Outside Selection", |doc, _| {
        let selection = doc.selection.clone().ok_or_else(|| bad("no selection"))?;
        let bounds = doc.bounds();
        let area = i64::from(bounds.width()).checked_mul(i64::from(bounds.height())).ok_or_else(|| bad("canvas too large"))?;
        if !(0..=64_000_000).contains(&area) {
            return Err(bad("Clear Outside Selection is limited to 64 million pixels"));
        }
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let mask = l.mask.get_or_insert_with(prism_doc::LayerMask::reveal_all);
        let mut out = prism_raster::Surface::with_default(mask.surface.format(), &[0.0]);
        let mut y = bounds.y0;
        while y < bounds.y1 {
            let end = y.saturating_add(32).min(bounds.y1);
            let strip = prism_geom::Rect { x0: bounds.x0, y0: y, x1: bounds.x1, y1: end };
            let (old, sel) = (mask.surface.read_region(strip), selection.read_region(strip));
            if old.len() != sel.len() {
                return Err(bad("the layer mask and the selection have different formats"));
            }
            let data: Vec<f32> = old.iter().zip(&sel).map(|(m, v)| m * v).collect();
            out.write_region(strip, &data);
            y = end;
        }
        out.prune();
        mask.surface = out;
        Ok(())
    })?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "PRISM_TEST_CSP_MATERIALS is a JSON array of two local tool paths and one LAF path"]
    fn local_materials_import_paint_execute_and_persist() {
        let paths: Vec<String> = serde_json::from_str(&std::env::var("PRISM_TEST_CSP_MATERIALS").unwrap()).unwrap();
        assert_eq!(paths.len(), 3);
        let originals: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
        // Both files are named material_0.tool: default group replacement must not
        // silently remove the pen when the second material is opened.
        let mut opened = Session::new();
        for path in &paths[..2] {
            opened.execute("brush.presets.importSut", json!({"path":path})).unwrap();
        }
        let imported: Vec<_> = opened.tools.presets.iter().filter(|p| p.sut.is_some()).collect();
        assert_eq!(imported.len(), 2);
        assert_ne!(imported[0].group, imported[1].group);
        let dir = std::env::temp_dir().join(format!("prism-raw-material-test-{}", std::process::id()));
        let mut s = Session::new();
        assert!(s.attach_preset_store(crate::preset_store::open_dir(&dir)).is_empty());
        let pen = s.execute("brush.presets.importSut", json!({"path":paths[0],"group":"Material test","select":true})).unwrap();
        assert_eq!(pen["brushes"], 1);
        let fill = s.execute("brush.presets.importSut", json!({"path":paths[1],"group":"Material test","replace":false})).unwrap();
        assert_eq!(fill["brushes"], 0);
        assert!(fill["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Enclose and fill")));
        s.execute("file.new", json!({"width":128,"height":128,"background":"transparent"})).unwrap();
        s.execute("paint.stroke", json!({"points":[[20,64,0.3],[100,64,1.0]]})).unwrap();
        assert!(prism_compose::flatten(&s.active().unwrap().doc).px.iter().any(|p| p[3] > 0.0));
        let action = s.execute(ID, json!({"path":paths[2]})).unwrap();
        assert_eq!(action["count"], 1);
        s.execute("actions.play", json!({"action":0})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
        let expected = s.actions.list.clone();
        s.sync_preset_store();
        let mut reopened = Session::new();
        let warnings = reopened.attach_preset_store(crate::preset_store::open_dir(&dir));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(reopened.actions.list, expected);
        assert_eq!(reopened.tools.presets.iter().filter(|p| p.group == "Material test").count(), 2);
        for original in s.tools.presets.iter().filter(|p| p.group == "Material test") {
            let loaded = reopened.tools.presets.iter().find(|p| p.group == original.group && p.name == original.name).unwrap();
            assert_eq!(loaded.sut, original.sut);
        }
        for (path, before) in paths.iter().zip(originals) {
            assert_eq!(std::fs::read(path).unwrap(), before);
        }
        std::fs::remove_dir_all(&dir).unwrap();
        eprintln!("{}", json!({"pen":pen,"fill":fill,"action":action}));
    }
    fn source(kinds: &[&str]) -> Value {
        json!({"actionArray":[{"typename":"typeAction","actionName":"Synthetic cleanup","actionItemArray":kinds.iter().map(|k|json!({"typename":"typeActionItem","actionItemKind":k,"actionItemEnable":1,"actionItemShowSettings":0,"actionItemParameter":if *k=="layeractionselectfromlayerstatusnew" { json!({"typename":"","referselected":0}) } else { json!({"typename":""}) }})).collect::<Vec<_>>()}]})
    }
    /// A LAF value stream for `v` (the reader's format, independent of it): objects carry their
    /// `typename` in a header, arrays an empty one.
    fn encode(v: &Value) -> Vec<u8> {
        fn int(n: u64) -> Vec<u8> {
            let b = n.to_be_bytes();
            let skip = b.iter().take_while(|x| **x == 0).count();
            [vec![(8 - skip) as u8], b[skip..].to_vec()].concat()
        }
        fn string(s: &str) -> Vec<u8> {
            [int(s.len() as u64), s.as_bytes().to_vec()].concat()
        }
        fn fields(m: &[(&str, &Value)]) -> Vec<u8> {
            let mut out = int(m.len() as u64);
            for (k, v) in m {
                out.extend(string(k));
                out.extend(value(v));
            }
            out
        }
        fn object(o: &serde_json::Map<String, Value>) -> Vec<u8> {
            let name = o.get("typename").cloned().unwrap_or(json!(""));
            let body: Vec<(&str, &Value)> = o.iter().filter(|(k, _)| k.as_str() != "typename").map(|(k, v)| (k.as_str(), v)).collect();
            [fields(&[("typename", &name)]), fields(&body)].concat()
        }
        fn framed(tag: u8, payload: Vec<u8>) -> Vec<u8> {
            [vec![tag], (payload.len() as u32).to_be_bytes().to_vec(), payload].concat()
        }
        fn value(v: &Value) -> Vec<u8> {
            match v {
                Value::String(s) => [vec![6], string(s)].concat(),
                Value::Number(n) if n.as_u64().is_some() => [vec![17], int(n.as_u64().unwrap())].concat(),
                Value::Number(n) if n.as_i64().is_some() => [vec![20], int(n.as_i64().unwrap().unsigned_abs())].concat(),
                Value::Number(n) => [vec![21, 8], n.as_f64().unwrap().to_be_bytes().to_vec()].concat(),
                Value::Array(a) => framed(72, [fields(&[("typename", &json!(""))]), int(a.len() as u64), a.iter().flat_map(value).collect()].concat()),
                Value::Object(o) => framed(73, object(o)),
                other => panic!("not encodable: {other}"),
            }
        }
        object(v.as_object().unwrap())
    }
    fn import_value(s: &mut Session, v: &Value) -> Result<Value> {
        s.execute(ID, json!({"data": prism_paint::tile::b64_encode(&encode(v))}))
    }
    fn set(actions: Value) -> Value {
        json!({"typename":"typeActionSet","actionSetName":"Synthetic set","actionArray":actions})
    }
    fn action(name: &str, items: Vec<Value>) -> Value {
        json!({"typename":"typeAction","actionName":name,"actionItemArray":items})
    }
    fn item(kind: &str, params: Value) -> Value {
        let mut p = params;
        p["typename"] = json!("");
        json!({"typename":"typeActionItem","actionItemKind":kind,"actionItemEnable":1,"actionItemShowSettings":0,"actionItemParameter":p})
    }
    #[test]
    fn unknown_steps_are_kept_as_placeholders_that_refuse_playback() {
        let mut s = Session::new();
        let v = set(json!([action("Tone", vec![item("layeractionsetcompositemode", json!({"compositemode": 2})), item("layeractionsettoneline", json!({"line": 30.0}))])]));
        let r = import_value(&mut s, &v).unwrap();
        assert_eq!(r["unsupported"], 1);
        assert_eq!(r["imported"][0]["ready"], false);
        assert_eq!(r["imported"][0]["unsupportedKinds"], json!(["layeractionsettoneline"]));
        let a = &s.actions.list[0];
        assert_eq!((a.steps[0].id.as_str(), a.steps[1].id.as_str()), ("layer.setProps", UNSUPPORTED));
        assert_eq!(a.steps[1].source.as_ref().unwrap().details.as_ref().unwrap()["parameters"]["line"], 30.0);
        // Playback refuses before the first step changes the document.
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let blend = |s: &Session| s.active().unwrap().doc.layers[0].blend;
        let before = blend(&s);
        assert!(s.execute("actions.play", json!({"action": 0})).is_err());
        assert_eq!(blend(&s), before);
        // Disabled, the rest runs.
        s.execute("actions.configure", json!({"action": 0, "step": 1, "enabled": false})).unwrap();
        s.execute("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(blend(&s), prism_color::BlendMode::Multiply);
    }
    #[test]
    fn label_actions_and_repeated_names_import() {
        let mut s = Session::new();
        let v = set(json!([action("----", vec![]), action("Run", vec![item("selectactiondeselect", json!({}))]), action("----", vec![])]));
        let r = import_value(&mut s, &v).unwrap();
        assert_eq!(r["count"], 3);
        let names: Vec<_> = s.actions.list.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["----", "Run", "---- (2)"]);
        assert!(s.actions.list.iter().all(|a| a.set == "Synthetic set"));
        assert!(import_value(&mut s, &v).is_err(), "existing names need replace");
        let mut v2 = v.clone();
        v2["actionArray"][1]["actionItemArray"][0]["actionItemShowSettings"] = json!(1);
        s.execute(ID, json!({"data": prism_paint::tile::b64_encode(&encode(&v2)), "replace": true})).unwrap();
        assert!(s.actions.list[1].steps[0].dialog);
        assert_eq!(s.actions.list.len(), 3);
    }
    #[test]
    fn malformed_params_and_sources_are_errors() {
        let mut s = Session::new();
        let nowhere = std::env::temp_dir().join("prism-no-csp-folder-here").to_string_lossy().into_owned();
        for p in [
            json!({}),
            json!({"data":"bad"}),
            json!({"path":3}),
            json!({"replace":0}),
            json!({"uuid":3}),
            json!({"uuid":""}),
            json!({"uuid":"x".repeat(200)}),
            json!({"uuid":"a","path":"x.laf"}),
            json!({"uuid":"a","root":nowhere}),
            json!({"uuid":"a","root":""}),
        ] {
            assert!(s.execute(ID, p).is_err());
            assert!(s.actions.list.is_empty());
        }
        for bad in [
            set(json!([])),
            set(json!([{"typename":"typeOther","actionName":"x","actionItemArray":[]}])),
            set(json!([action("", vec![])])),
            set(json!([action("x", vec![json!({"typename":"typeActionItem","actionItemKind":"selectactiondeselect","actionItemEnable":2,"actionItemShowSettings":0,"actionItemParameter":{"typename":""}})])])),
            set(json!([action("x", vec![json!({"typename":"typeActionItem","actionItemKind":"","actionItemEnable":1,"actionItemShowSettings":0,"actionItemParameter":{"typename":""}})])])),
        ] {
            assert!(import_value(&mut s, &bad).is_err(), "{bad}");
            assert!(s.actions.list.is_empty());
        }
    }
    #[test]
    #[ignore = "reads the local CLIP STUDIO material folder (PRISM_TEST_CSP_ROOT or the platform's)"]
    fn every_local_downloaded_laf_imports() {
        let root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(std::path::PathBuf::from).or_else(prism_io::sut::library::default_root).unwrap();
        let mut stack = vec![root.join("CLIPStudioCommon").join("Material")];
        let (mut files, mut actions, mut ready, mut kinds) = (0, 0, 0, std::collections::BTreeMap::<String, usize>::new());
        let mut played: Vec<(String, Option<String>)> = Vec::new();
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "laf") {
                    let mut s = Session::new();
                    let r = s.execute(ID, json!({"path": p.to_string_lossy()})).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                    files += 1;
                    for a in r["imported"].as_array().unwrap() {
                        actions += 1;
                        ready += usize::from(a["ready"] == true);
                        if a["ready"] == true && a["steps"].as_u64() > Some(0) {
                            // Play on a small illustration: white paper, a drawn layer, and the
                            // copy on the clipboard that copy/paste actions expect.
                            let mut play = Session::new();
                            play.actions.list = s.actions.list.clone();
                            play.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
                            play.execute("layer.new.layer", json!({"name": "Layer 1"})).unwrap();
                            play.execute("select.rect", json!({"x": 16, "y": 16, "width": 24, "height": 20})).unwrap();
                            play.execute("edit.fill", json!({"contents": "color", "color": "#c04020"})).unwrap();
                            play.execute("select.deselect", json!({})).unwrap();
                            play.execute("edit.copy", json!({})).ok();
                            let name = a["name"].as_str().unwrap();
                            let at = play.actions.list.iter().position(|x| x.name == name).unwrap();
                            // A failing step stops playback and is reported in `failed`, not as an error.
                            let result = play.execute("actions.play", json!({"action": at})).map_err(|e| e.to_string()).and_then(|r| match r.get("failed") {
                                Some(f) if f.is_object() => Err(format!("step {} {}: {}", f["step"], f["id"], f["error"])),
                                _ => Ok(r),
                            });
                            eprintln!("  play {name}: {}", result.as_ref().map_or_else(|e| format!("ERROR {e}"), |_| "ok".into()));
                            if let Some(out) = std::env::var_os("PRISM_LAF_CHECK_OUT") {
                                let path = std::path::Path::new(&out).join(format!("{:03}.png", played.len()));
                                eprintln!("    -> {}", path.display());
                                play.execute("file.saveACopy", json!({"path": path.to_string_lossy(), "layers": false})).ok();
                            }
                            played.push((name.to_string(), result.err()));
                        }
                        for k in a["unsupportedKinds"].as_array().into_iter().flatten() {
                            *kinds.entry(k.as_str().unwrap().to_string()).or_default() += 1;
                        }
                    }
                    eprintln!("{}: {}", p.display(), serde_json::to_string(&r["imported"]).unwrap());
                }
            }
        }
        eprintln!("{files} files, {actions} actions, {ready} ready; blocking kinds by action: {kinds:?}");
        // The same sets through the library listing, by material id.
        let mut s = Session::new();
        let listed = s.execute("brush.cspLibrary.list", json!({"root": root.to_string_lossy()})).unwrap();
        let ids: Vec<&str> = listed["actions"].as_array().unwrap().iter().filter(|a| a["status"] == "ready").filter_map(|a| a["uuid"].as_str()).collect();
        assert_eq!(ids.len(), files, "every downloaded LAF is listed as a ready auto action");
        for id in ids {
            s.execute(ID, json!({"uuid": id, "root": root.to_string_lossy(), "replace": true})).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let failed: Vec<_> = played.iter().filter(|(_, e)| e.is_some()).collect();
        eprintln!("played {} ready actions, {} failed: {failed:#?}", played.len(), failed.len());
        assert!(files > 0);
    }
    #[test]
    fn source_steps_remain_individually_editable_and_disabled_steps_are_retained() {
        let mut v = source(&["layeractionselectfromlayerstatusnew", "editactionclearoutofselectarea", "selectactiondeselect"]);
        v["actionSetName"] = json!("Imported set");
        v["actionArray"][0]["actionItemArray"][1]["actionItemEnable"] = json!(0);
        v["actionArray"][0]["actionItemArray"][1]["actionItemShowSettings"] = json!(1);
        let actions = compile(&v).unwrap().actions;
        let a = &actions[0];
        assert_eq!(a.set, "Imported set");
        assert_eq!(a.steps.len(), 3);
        assert_eq!(a.steps[1].id, "edit.clearOutsideSelection");
        assert!(!a.steps[1].enabled);
        assert!(a.steps[1].dialog);
        assert_eq!(a.steps[1].source.as_ref().unwrap().index, 1);
    }
    #[test]
    fn clear_outside_on_an_adjustment_layer_masks_it() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_none(), "no selection, no mask");
        s.execute("select.rect", json!({"x": 2, "y": 2, "width": 3, "height": 3})).unwrap();
        s.execute("edit.clearOutsideSelection", json!({})).unwrap();
        let d = s.active().unwrap();
        let mask = d.doc.layer(id).unwrap().mask.as_ref().unwrap();
        assert_eq!((mask.value(3, 3), mask.value(0, 0), mask.value(7, 7), mask.value(-5, 40)), (1.0, 0.0, 0.0, 0.0));
        assert!(d.doc.selection.is_some(), "the selection stays");
        assert!(s.undo());
        assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_none(), "one undo step");
        // An existing mask keeps what it hid inside the selection.
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 2, "height": 2})).unwrap();
        s.execute("layer.layerMask.hideSelection", json!({})).ok();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        s.execute("edit.clearOutsideSelection", json!({})).unwrap();
        let mask = s.active().unwrap().doc.layer(id).unwrap().mask.clone().unwrap();
        assert_eq!((mask.value(3, 3), mask.value(5, 5)), (1.0, 0.0));
        s.execute("select.deselect", json!({})).unwrap();
        assert!(s.execute("edit.clearOutsideSelection", json!({})).is_err(), "no selection");
    }

    #[test]
    fn clear_outside_is_one_undo_step_and_preserves_selection_on_failure() {
        let mut s = Session::new();
        assert!(s.execute("edit.clearOutsideSelection", json!({})).is_err());
        s.execute("file.new", json!({"width":8,"height":8,"background":"transparent"})).unwrap();
        assert!(s.execute("edit.clearOutsideSelection", json!({})).is_err());
        s.execute("select.rect", json!({"x":2,"y":2,"width":2,"height":2})).unwrap();
        let selected = s.active().unwrap().doc.selection.clone();
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute("edit.clearOutsideSelection", json!({"layer":999999})).is_err());
        assert_eq!(s.active().unwrap().doc.selection, selected);
        assert_eq!(s.active().unwrap().history.past_len(), history);
        s.execute("edit.clearOutsideSelection", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.selection, selected);
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.selection, selected);
    }
    #[test]
    #[ignore = "24 MP action migration benchmark; run explicitly with --nocapture"]
    fn clear_outside_24mp_comparison() {
        for atomic in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":6000,"height":4000,"background":"transparent"})).unwrap();
            s.execute("edit.fill", json!({"color":"#336699"})).unwrap();
            s.execute("select.rect", json!({"x":1000,"y":1000,"width":4000,"height":2000})).unwrap();
            let start = std::time::Instant::now();
            if atomic {
                s.execute("edit.clearOutsideSelection", json!({})).unwrap();
            } else {
                for id in ["select.inverse", "edit.clear", "select.inverse"] {
                    s.execute(id, json!({})).unwrap();
                }
            }
            eprintln!("24MP clear outside atomic={atomic}: {:?}", start.elapsed());
            let d = s.active().unwrap();
            let surface = d.doc.layers[0].surface().unwrap();
            assert_eq!(surface.pixel(0, 0)[3], 0.0);
            assert_eq!(surface.pixel(2000, 2000)[3], 1.0);
            assert_eq!(d.doc.selection.as_ref().unwrap().sample_channel(2000, 2000, 0), 1.0);
        }
    }
    #[test]
    fn cleanup_runs_at_all_depths_and_preserves_rgb() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
            let d = s.active_mut().unwrap();
            let id = d.active_layer.unwrap();
            let surf = std::sync::Arc::make_mut(&mut d.doc).layer_mut(id).unwrap().surface_mut().unwrap();
            surf.write_pixel(2, 2, &[0.2, 0.4, 0.6, 0.5]);
            surf.write_pixel(3, 2, &[0.2, 0.4, 0.6, 1.0]);
            s.actions.list = compile(&source(&["layeractionselectfromlayerstatusnew", "editactionclearoutofselectarea", "selectactiondeselect"])).unwrap().actions;
            s.execute("actions.play", json!({"action":0})).unwrap();
            let d = s.active().unwrap();
            let surf = d.doc.layer(id).unwrap().surface().unwrap();
            let p = surf.pixel(2, 2);
            assert!((p[3] - 0.25).abs() < 0.015, "{depth}: {p:?}");
            assert!((p[0] - 0.2).abs() < 0.015);
            assert!((surf.pixel(3, 2)[3] - 1.0).abs() < 0.01);
            assert!(d.doc.selection.is_none());
        }
    }
}
