//! Import supported CLIP STUDIO auto actions without executing them.
use crate::{
    EngineError, Result, Session,
    actions_cmds::{Action, ActionStep},
    commands::CommandSpec,
};
use serde_json::{Value, json};
const ID: &str = "actions.importLaf";
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
fn compile(v: &Value) -> Result<Vec<Action>> {
    let source = v
        .get("actionArray")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= 256)
        .ok_or_else(|| bad("LAF has no actions (or more than 256)"))?;
    let mut actions: Vec<Action> = Vec::new();
    for a in source {
        if a.get("typename").and_then(Value::as_str) != Some("typeAction") {
            return Err(bad("invalid LAF action"));
        }
        let name = name(a, "actionName")?;
        if actions.iter().any(|a| a.name == name) {
            return Err(bad("duplicate action names in LAF"));
        }
        let items = a.get("actionItemArray").and_then(Value::as_array).filter(|a| a.len() <= 4096).ok_or_else(|| bad("invalid LAF action steps"))?;
        let mut steps: Vec<ActionStep> = Vec::new();
        for (source_index, item) in items.iter().enumerate() {
            if item.get("typename").and_then(Value::as_str) != Some("typeActionItem") {
                return Err(bad("invalid LAF action step"));
            }
            let enabled = flag(item, "actionItemEnable")?;
            let dialog = flag(item, "actionItemShowSettings")?;
            let first = steps.len();
            let kind = name_of_step(item)?;
            let params = item.get("actionItemParameter").and_then(Value::as_object).ok_or_else(|| bad("missing LAF action parameters"))?;
            let allowed = match kind {
                "layeractionselectfromlayerstatusnew" => &["typename", "referselected"][..],
                "editactionclearoutofselectarea" | "selectactiondeselect" => &["typename"][..],
                _ => return Err(bad(format!("{name}: unsupported CLIP STUDIO action `{kind}`; no actions were imported"))),
            };
            if params.keys().any(|k| !allowed.contains(&k.as_str())) || params.get("typename").and_then(Value::as_str) != Some("") {
                return Err(bad(format!("{name}: unsupported parameters for `{kind}`")));
            }
            match kind {
                "layeractionselectfromlayerstatusnew" => {
                    if params.get("referselected").and_then(Value::as_u64) != Some(0) {
                        return Err(bad("loading transparency from multiple selected layers is unsupported"));
                    }
                    steps.push(("select.loadSelection".into(), json!({"channel":"transparency","operation":"new"})).into());
                }
                "editactionclearoutofselectarea" => {
                    steps.push(("edit.clearOutsideSelection".into(), json!({})).into());
                }
                "selectactiondeselect" => steps.push(("select.deselect".into(), json!({})).into()),
                _ => return Err(bad("unsupported LAF action")),
            }
            for (part, step) in steps.iter_mut().skip(first).enumerate() {
                step.enabled = enabled;
                step.dialog = dialog && part == 0;
                step.source = Some(crate::action_model::StepSource { format: "laf".into(), command: kind.into(), index: source_index, details: None });
            }
        }
        if steps.is_empty() {
            return Err(bad(format!("{name}: no enabled action steps")));
        }
        actions.push(Action {
            name,
            set: v.get("actionSetName").and_then(Value::as_str).filter(|s| !s.is_empty() && s.chars().count() <= 255).unwrap_or("CLIP STUDIO").into(),
            steps,
        });
    }
    Ok(actions)
}
fn name_of_step(v: &Value) -> Result<&str> {
    v.get("actionItemKind").and_then(Value::as_str).ok_or_else(|| bad("missing action kind"))
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
    let (bytes, _) = crate::preset_import_cmds::file_bytes_with_limit(p, ID, ".laf file", 8 << 20)?;
    let source = prism_io::laf::read_laf(&bytes, &prism_raster::Interrupt::NONE).map_err(bad)?;
    let actions = compile(&source)?;
    if !replace && actions.iter().any(|a| s.actions.list.iter().any(|old| old.name == a.name)) {
        return Err(bad("an action with that name exists; pass `replace:true` to replace it"));
    }
    let imported: Vec<_> = actions.iter().map(|a| json!({"name":a.name,"steps":a.steps.len()})).collect();
    for a in actions {
        if let Some(old) = s.actions.list.iter_mut().find(|old| old.name == a.name) {
            *old = a;
        } else {
            s.actions.list.push(a);
        }
    }
    s.actions.touch();
    Ok(
        json!({"format":"laf","count":imported.len(),"imported":imported,"warnings":["CLIP STUDIO steps were mapped to Prism commands; pixel-identical results are not guaranteed. Step enable and dialog flags were preserved."]}),
    )
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: ID,
            label: "Import CLIP STUDIO Auto Actions…",
            menu: &[],
            shortcut: None,
            params: r#"{"path":".laf file"?,"data":base64 bytes?,"replace":bool=false}"#,
            enabled: |_| Ok(()),
            run: import,
            journal: false,
        },
        CommandSpec {
            id: "edit.clearOutsideSelection",
            label: "Clear Outside Selection",
            menu: &[],
            shortcut: None,
            params: "{layer:id?}. Clear the inverse selection in one undo step, preserving the selection.",
            enabled: |_| Ok(()),
            run: clear_outside,
            journal: true,
        },
    ]
}

fn clear_outside(s: &mut Session, p: &Value) -> Result<Value> {
    let id = crate::commands::layer_param(s, p)?;
    let bg = s.tools.background;
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
    #[test]
    fn unknown_steps_reject_and_interactive_flags_are_preserved() {
        assert!(compile(&source(&["selectactiondeselect", "unknown"])).is_err());
        let mut v = source(&["selectactiondeselect"]);
        v["actionArray"][0]["actionItemArray"][0]["actionItemShowSettings"] = json!(1);
        assert!(compile(&v).unwrap()[0].steps[0].dialog);
        let mut s = Session::new();
        for p in [json!({}), json!({"data":"bad"}), json!({"path":3}), json!({"replace":0})] {
            assert!(s.execute(ID, p).is_err());
            assert!(s.actions.list.is_empty());
        }
    }
    #[test]
    fn source_steps_remain_individually_editable_and_disabled_steps_are_retained() {
        let mut v = source(&["layeractionselectfromlayerstatusnew", "editactionclearoutofselectarea", "selectactiondeselect"]);
        v["actionSetName"] = json!("Imported set");
        v["actionArray"][0]["actionItemArray"][1]["actionItemEnable"] = json!(0);
        v["actionArray"][0]["actionItemArray"][1]["actionItemShowSettings"] = json!(1);
        let actions = compile(&v).unwrap();
        let a = &actions[0];
        assert_eq!(a.set, "Imported set");
        assert_eq!(a.steps.len(), 3);
        assert_eq!(a.steps[1].id, "edit.clearOutsideSelection");
        assert!(!a.steps[1].enabled);
        assert!(a.steps[1].dialog);
        assert_eq!(a.steps[1].source.as_ref().unwrap().index, 1);
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
            s.actions.list = compile(&source(&["layeractionselectfromlayerstatusnew", "editactionclearoutofselectarea", "selectactiondeselect"])).unwrap();
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
