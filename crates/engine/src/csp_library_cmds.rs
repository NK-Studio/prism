//! Brushes downloaded into an installed CLIP STUDIO PAINT, imported without a SUT export.
//! Read-only: see `prism_io::sut::library` for what is read and what is never touched.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use prism_io::sut::library::{self, Status};
use serde_json::{Value, json};
use std::path::PathBuf;

const LIST: &str = "brush.cspLibrary.list";
const IMPORT: &str = "brush.cspLibrary.import";
/// Brushes imported by one command.
const MAX_IMPORT: usize = 64;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// `root` when given, else the platform's default CLIP STUDIO user data folder.
fn root(cmd: &str, p: &Value) -> Result<PathBuf> {
    match p.get("root") {
        Some(v) => v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).ok_or_else(|| bad(cmd, "`root` must be a non-empty folder path")),
        None => library::default_root()
            .ok_or_else(|| EngineError::Other("CLIP STUDIO user data folder not found; is CLIP STUDIO installed? Pass `root` to choose the folder".into())),
    }
}

fn list(_s: &mut Session, p: &Value) -> Result<Value> {
    let root = root(LIST, p)?;
    let lib = library::list(&root).map_err(EngineError::Other)?;
    let brushes: Vec<Value> = lib
        .brushes
        .iter()
        .map(|b| {
            let source = match b.status {
                Status::Ready(source) => Value::from(source.id()),
                _ => Value::Null,
            };
            let thumbnail = b.thumbnail.as_ref().map(|p| p.display().to_string());
            json!({ "uuid": b.uuid, "name": b.name, "kind": b.kind, "catalog": b.catalog, "status": b.status.id(), "source": source, "guidance": b.status.guidance(), "thumbnail": thumbnail, "tags": b.tags })
        })
        .collect();
    let ready = lib.brushes.iter().filter(|b| matches!(b.status, Status::Ready(_))).count();
    let catalogs: Vec<Value> =
        lib.catalogs.iter().map(|c| json!({ "name": c.name, "thumbnail": c.thumbnail.as_ref().map(|p| p.display().to_string()), "tags": c.tags })).collect();
    Ok(
        json!({ "root": lib.root.display().to_string(), "brushes": brushes, "catalogs": catalogs, "count": brushes.len(), "ready": ready, "notices": lib.notices }),
    )
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let uuids: Vec<String> = p
        .get("uuids")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= MAX_IMPORT)
        .ok_or_else(|| bad(IMPORT, format!("`uuids` must be an array of 1 to {MAX_IMPORT} brush ids from {LIST}")))?
        .iter()
        .map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty() && s.len() <= 128).map(str::to_string))
        .collect::<Option<_>>()
        .ok_or_else(|| bad(IMPORT, "every `uuids` entry must be a non-empty brush id string"))?;
    for k in ["replace", "select"] {
        if p.get(k).is_some_and(|v| !v.is_boolean()) {
            return Err(bad(IMPORT, format!("`{k}` must be a boolean")));
        }
    }
    let group = match p.get("group") {
        Some(v) => Some(
            v.as_str()
                .map(str::trim)
                .filter(|g| !g.is_empty() && g.chars().count() <= 255)
                .ok_or_else(|| bad(IMPORT, "`group` must be a non-empty string of at most 255 characters"))?
                .to_string(),
        ),
        None => None,
    };
    let root = root(IMPORT, p)?;
    let apply = p.clone();
    crate::jobs::run(
        s,
        "Import CLIP STUDIO Brushes",
        false,
        move |ctx| {
            let mut done: Vec<(String, prism_io::abr_map::AbrImport)> = Vec::new();
            let mut failed = Vec::new();
            let n = uuids.len() as f32;
            for (i, uuid) in uuids.iter().enumerate() {
                ctx.check()?;
                let result =
                    ctx.stage(i as f32 / n, (i + 1) as f32 / n, "Reading CLIP STUDIO brush", |ctl| library::import(&root, uuid, group.as_deref(), ctl));
                match result {
                    Ok(imported) => done.push(imported),
                    Err(_) if ctx.cancelled() => return Err(EngineError::Cancelled),
                    Err(error) => failed.push(json!({ "uuid": uuid, "error": error })),
                }
            }
            if done.is_empty() {
                let reasons: Vec<&str> = failed.iter().filter_map(|f| f["error"].as_str()).collect();
                return Err(EngineError::Other(format!("no CLIP STUDIO brush was imported: {}", reasons.join("; "))));
            }
            Ok((done, failed))
        },
        move |s, (done, failed)| {
            let replace = apply.get("replace").and_then(Value::as_bool).unwrap_or(true);
            let mut select = apply.get("select").and_then(Value::as_bool).unwrap_or(false);
            let (mut imported, mut warnings, mut notes, mut groups) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut brushes = 0u64;
            for (group, imp) in done {
                // Re-importing a brush replaces that brush, not the rest of its collection.
                if replace {
                    s.tools.presets.retain(|x| x.builtin || x.group != group || !imp.presets.iter().any(|n| n.name == x.name));
                }
                let names: Vec<String> = imp.presets.iter().map(|x| x.name.clone()).collect();
                warnings.extend(imp.warnings.iter().map(|w| format!("{}: {w}", names.join(", "))));
                notes.extend(imp.notes.iter().map(|w| format!("{}: {w}", names.join(", "))));
                let r = crate::preset_import_cmds::add_abr_presets(s, &json!({ "replace": false, "select": select }), group.clone(), imp)?;
                select = false;
                imported.extend(r["imported"].as_array().cloned().unwrap_or_default());
                brushes += r["brushes"].as_u64().unwrap_or(0);
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            Ok(
                json!({ "format": "csp-library", "imported": imported, "count": imported.len(), "brushes": brushes, "groups": groups, "warnings": warnings, "notes": notes, "failed": failed }),
            )
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: LIST,
            label: "List CLIP STUDIO Downloaded Brushes",
            menu: &[],
            shortcut: None,
            params: r#"{"root":"CLIP STUDIO user data folder"?=platform default} → {root, brushes:[{uuid,name,kind,catalog,status,source,guidance,thumbnail:path?,tags:[user tag]}], catalogs:[{name,thumbnail:path?,tags:[user tag]}], count, ready, notices}"#,
            enabled: |_| Ok(()),
            run: list,
            journal: false,
        },
        CommandSpec {
            id: IMPORT,
            label: "Import from CLIP STUDIO…",
            menu: &[],
            shortcut: None,
            params: r#"{"uuids":[brush id from brush.cspLibrary.list],"root":string?,"group":string?=collection name,"replace":bool=true (replace a same-named brush in its group),"select":bool=false}"#,
            enabled: |_| Ok(()),
            run: import,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_params_fail_without_changing_the_library() {
        let mut s = Session::new();
        let presets = s.tools.presets.clone();
        let brush = s.tools.brush.clone();
        let many: Vec<String> = (0..=MAX_IMPORT).map(|i| format!("id-{i}")).collect();
        for p in [
            json!({}),
            json!({"uuids": []}),
            json!({"uuids": "id"}),
            json!({"uuids": [1]}),
            json!({"uuids": [""]}),
            json!({"uuids": many}),
            json!({"uuids": ["id"], "select": 2}),
            json!({"uuids": ["id"], "group": ""}),
            json!({"uuids": ["id"], "root": ""}),
            json!({"uuids": ["id"], "root": "/nonexistent/prism-csp-root"}),
        ] {
            assert!(s.execute(IMPORT, p.clone()).is_err(), "{p}");
            assert_eq!(s.tools.presets, presets);
            assert_eq!(s.tools.brush, brush);
        }
        for p in [json!({"root": 3}), json!({"root": ""}), json!({"root": "/nonexistent/prism-csp-root"})] {
            assert!(s.execute(LIST, p).is_err());
        }
    }

    /// Local, user-owned CLIP STUDIO data; never bundled or committed.
    #[test]
    #[ignore = "reads the local CLIP STUDIO user data folder (PRISM_TEST_CSP_ROOT or the default)"]
    fn local_library_imports_ready_brushes_into_the_library() {
        let mut s = Session::new();
        let mut p = json!({});
        if let Some(root) = std::env::var_os("PRISM_TEST_CSP_ROOT") {
            p["root"] = json!(root.to_string_lossy());
        }
        let listed = s.execute(LIST, p.clone()).unwrap();
        let ready: Vec<Value> = listed["brushes"].as_array().unwrap().iter().filter(|b| b["status"] == "ready").map(|b| b["uuid"].clone()).collect();
        assert!(!ready.is_empty());
        p["uuids"] = json!(ready);
        p["select"] = json!(true);
        let r = s.execute(IMPORT, p.clone()).unwrap();
        assert_eq!(r["count"].as_u64(), Some(ready.len() as u64));
        let n = s.tools.presets.len();
        s.execute(IMPORT, p).unwrap();
        assert_eq!(s.tools.presets.len(), n, "re-importing replaces the same brushes");
        s.execute("file.new", json!({"width":128,"height":128,"depth":8,"background":"transparent"})).unwrap();
        s.execute("paint.stroke", json!({"points":[[20,64,0.3],[100,64,1.0]]})).unwrap();
    }
}
