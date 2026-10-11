//! Clip Studio Paint brush import, shared by the native file picker and automation.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "brush.presets.importSut".into(), msg: msg.into() }
}
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    for k in ["replace", "select"] {
        if p.get(k).is_some_and(|v| !v.is_boolean()) {
            return Err(bad(format!("`{k}` must be a boolean")));
        }
    }
    let params = p.clone();
    let apply = p.clone();
    crate::jobs::run(
        s,
        "Import CSP Brush",
        false,
        move |ctx| {
            let (bytes, stem) = crate::preset_import_cmds::file_bytes_with_limit(&params, "brush.presets.importSut", ".sut file", 64 << 20)?;
            let group = match params.get("group") {
                Some(v) => v
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && s.chars().count() <= 255)
                    .ok_or_else(|| bad("`group` must be a non-empty string of at most 255 characters"))?
                    .to_string(),
                None if !stem.is_empty() => stem,
                None => "Clip Studio Paint".into(),
            };
            ctx.check()?;
            let imp = ctx
                .stage(0.0, 1.0, "Reading CSP brush", |ctl| prism_io::sut::read_sut(&bytes, &group, ctl))
                .map_err(|e| if ctx.cancelled() { EngineError::Cancelled } else { bad(e) })?;
            Ok((group, imp))
        },
        move |s, (group, imp)| {
            let mut result = crate::preset_import_cmds::add_abr_presets(s, &apply, group, imp)?;
            if let Some(r) = result.as_object_mut() {
                r.remove("version");
                r.insert("format".into(), json!("sut"));
            }
            Ok(result)
        },
    )
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "brush.presets.importSut",
        label: "Import Clip Studio Paint Brush…",
        menu: &[],
        shortcut: None,
        params: r#"{"path":".sut file"?,"data":base64 bytes?,"group":string?=file name,"replace":bool=true,"select":bool=false}"#,
        enabled: |_| Ok(()),
        run: import,
        journal: true,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_import_keeps_library_and_brush() {
        let mut s = Session::new();
        let presets = s.tools.presets.clone();
        let brush = s.tools.brush.clone();
        for p in [
            json!({}),
            json!({"path":""}),
            json!({"data":22}),
            json!({"data":"AAAA"}),
            json!({"data":"%%%"}),
            json!({"path":"/nonexistent.sut"}),
            json!({"select":4}),
            json!({"data":"AAAA","group":2}),
        ] {
            assert!(s.execute("brush.presets.importSut", p).is_err());
            assert_eq!(s.tools.presets, presets);
            assert_eq!(s.tools.brush, brush);
        }
    }
    /// Local user-owned oracle; never bundled or committed.
    #[test]
    #[ignore = "set PRISM_TEST_SUT to a local brush"]
    fn local_sut_import_paints_and_persists() {
        let path = std::env::var("PRISM_TEST_SUT").unwrap();
        let dir = std::env::temp_dir().join(format!("prism-sut-oracle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Session::new();
        assert!(s.attach_preset_store(crate::preset_store::open_dir(&dir)).is_empty());
        let n = s.tools.presets.len();
        let result = s.execute("brush.presets.importSut", json!({"path":path,"select":true})).unwrap();
        assert_eq!(result["count"], 1);
        assert_eq!(result["format"], "sut");
        assert_eq!(result["imported"][0], "세모네모펜");
        assert_eq!(s.tools.brush.size, 17.0);
        let prism_paint::TipShape::Sequence(tips) = &s.tools.brush.tip else { panic!("expected both tips") };
        assert_eq!(tips.len(), 2);
        assert_eq!((tips[0].width, tips[0].height), (190, 148));
        assert_eq!((tips[1].width, tips[1].height), (124, 125));
        for depth in [8, 16, 32] {
            s.execute("file.new", json!({"width":128,"height":128,"depth":depth,"background":"transparent"})).unwrap();
            s.execute("paint.stroke", json!({"points":[[20,64,0.3],[100,64,1.0]]})).unwrap();
            let doc = &s.active().unwrap().doc;
            let image = prism_compose::flatten(doc);
            assert!(image.px.iter().any(|p| p[3] > 0.0 && p[0] < 0.5));
        }
        s.execute("brush.presets.importSut", json!({"path":path})).unwrap();
        assert_eq!(s.tools.presets.len(), n + 1);
        s.sync_preset_store();
        let mut restarted = Session::new();
        assert!(restarted.attach_preset_store(crate::preset_store::open_dir(&dir)).is_empty());
        let name = result["imported"][0].as_str().unwrap();
        assert_eq!(prism_paint::presets::find(&restarted.tools.presets, name).unwrap().brush.tip, s.tools.brush.tip);
        std::fs::remove_dir_all(&dir).unwrap();
        let json = serde_json::to_string(&s.tools.brush).unwrap();
        assert_eq!(serde_json::from_str::<prism_paint::BrushSettings>(&json).unwrap(), s.tools.brush);
    }
}
