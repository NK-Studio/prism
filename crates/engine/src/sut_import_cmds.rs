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

    fn collect_suts(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() && depth < 8 {
                collect_suts(&path, depth + 1, out);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("sut")) {
                out.push(path);
            }
        }
    }

    fn cell(s: &str) -> String {
        s.replace('|', "\\|").replace('\n', " ")
    }

    /// Local folder of user-owned brushes; never bundled or committed. Prints a Markdown report
    /// (also written to PRISM_TEST_SUT_REPORT when set). Import errors are reported; panics fail.
    #[test]
    #[ignore = "set PRISM_TEST_SUT_DIR to a local folder of .sut brushes"]
    fn local_sut_folder_report() {
        let dir = std::path::PathBuf::from(std::env::var("PRISM_TEST_SUT_DIR").unwrap());
        let mut files = Vec::new();
        collect_suts(&dir, 0, &mut files);
        files.sort();
        assert!(!files.is_empty(), "no .sut files under {}", dir.display());
        let mut rows = Vec::new();
        let mut warning_counts = std::collections::BTreeMap::<String, usize>::new();
        let (mut ok, mut failed, mut panicked, mut blank, mut unpersisted) = (0, 0, 0, 0, 0);
        for path in &files {
            let rel = path.strip_prefix(&dir).unwrap_or(path).display().to_string();
            let started = std::time::Instant::now();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let store = std::env::temp_dir().join(format!("prism-sut-folder-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&store);
                let mut s = Session::new();
                let _ = s.attach_preset_store(crate::preset_store::open_dir(&store));
                let result = s.execute("brush.presets.importSut", json!({"path":path,"select":true}))?;
                s.execute("file.new", json!({"width":256,"height":256,"depth":8,"background":"transparent"}))?;
                s.execute("paint.stroke", json!({"points":[[30,128,0.2],[128,100,0.7],[226,128,1.0]]}))?;
                let painted = s.active().map(|d| prism_compose::flatten(&d.doc).px.iter().filter(|p| p[3] > 0.0).count()).unwrap_or(0);
                // Persisted: the library survives a restart with the same brush.
                s.sync_preset_store();
                let mut restarted = Session::new();
                let _ = restarted.attach_preset_store(crate::preset_store::open_dir(&store));
                let name = result["imported"][0].as_str().unwrap_or_default();
                let imported = prism_paint::presets::find(&s.tools.presets, name).map(|p| p.brush.clone());
                let persisted = imported.is_some() && prism_paint::presets::find(&restarted.tools.presets, name).map(|p| p.brush.clone()) == imported;
                let _ = std::fs::remove_dir_all(&store);
                Ok::<_, EngineError>((result, s.tools.brush.clone(), painted, persisted))
            }));
            let ms = started.elapsed().as_millis();
            let row = match outcome {
                Ok(Ok((result, brush, painted, persisted))) => {
                    ok += 1;
                    let tips = match &brush.tip {
                        prism_paint::TipShape::Round => "round".to_string(),
                        prism_paint::TipShape::Sampled(t) => format!("1 ({}×{})", t.width, t.height),
                        prism_paint::TipShape::Sequence(ts) => {
                            let sizes: Vec<_> = ts.iter().take(4).map(|t| format!("{}×{}", t.width, t.height)).collect();
                            format!("{} ({}{})", ts.len(), sizes.join(", "), if ts.len() > 4 { ", …" } else { "" })
                        }
                    };
                    let warnings: Vec<String> =
                        result["warnings"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(str::to_string)).collect();
                    for w in &warnings {
                        *warning_counts.entry(w.clone()).or_default() += 1;
                    }
                    if painted == 0 {
                        blank += 1;
                    }
                    let name = result["imported"][0].as_str().unwrap_or("?");
                    if !persisted {
                        unpersisted += 1;
                    }
                    let status = if painted == 0 {
                        "⚠️ blank stroke"
                    } else if !persisted {
                        "⚠️ not persisted"
                    } else {
                        "✅"
                    };
                    let texture = if brush.texture.enabled { "yes" } else { "" };
                    format!(
                        "| {status} | {} | {} | {tips} | {texture} | {:.1} | {painted} | {} | {ms} |",
                        cell(&rel),
                        cell(name),
                        brush.size,
                        warnings.len()
                    )
                }
                Ok(Err(e)) => {
                    failed += 1;
                    format!("| ❌ {} | {} | | | | | | | {ms} |", cell(&e.to_string()), cell(&rel))
                }
                Err(p) => {
                    panicked += 1;
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    format!("| 💥 PANIC {} | {} | | | | | | | {ms} |", cell(&msg), cell(&rel))
                }
            };
            rows.push(row);
        }
        let mut report = format!(
            "# SUT folder report\n\n{} files: {ok} imported ({blank} painted nothing, {unpersisted} not persisted), {failed} failed, {panicked} panicked\n\n\
             | status | file | brush | tips | texture | size | painted px | warnings | ms |\n|---|---|---|---|---|---|---|---|---|\n",
            files.len()
        );
        for r in &rows {
            report.push_str(r);
            report.push('\n');
        }
        if !warning_counts.is_empty() {
            report.push_str("\n## Warnings by frequency\n\n| files | warning |\n|---|---|\n");
            let mut by_count: Vec<_> = warning_counts.into_iter().collect();
            by_count.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            for (w, n) in by_count {
                report.push_str(&format!("| {n} | {} |\n", cell(&w)));
            }
        }
        println!("{report}");
        if let Ok(out) = std::env::var("PRISM_TEST_SUT_REPORT") {
            std::fs::write(&out, &report).unwrap();
            println!("report written to {out}");
        }
        assert_eq!(panicked, 0, "importing or painting a SUT panicked; see the report");
    }
}
