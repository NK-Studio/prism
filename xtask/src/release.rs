//! `cargo xtask release`: a local release conveyor belt.
//!
//! The steps live in `packaging/release/pipeline.json` (what to do, and how to check it was
//! done); progress lives in `log/release/state.json` (local, not committed). Each command prints
//! where the release stands, so a person or an agent can ask "what's next?" and say "done":
//!
//! ```text
//! cargo xtask release start 1.2.0 [--force]   begin a release (--force replaces an unfinished one)
//! cargo xtask release status [--json]         the current step: what to do, how it's checked
//! cargo xtask release check                   run the current step's checks without moving on
//! cargo xtask release done [--note TEXT]      check the current step; if it passes, move on
//! cargo xtask release skip [--note TEXT]      skip the current step (optional steps only)
//! cargo xtask release back                    reopen the previous step
//! ```
//!
//! Checks (`verify` in the pipeline): `cargoVersion` (Cargo.toml has the version), `gitClean`
//! (paths committed), `files` (globs exist; `*` in the file name), `command` (`sh -c` succeeds),
//! `urlJson` (a JSON field at a URL equals a value, fetched with curl).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const PIPELINE: &str = "packaging/release/pipeline.json";
const STATE: &str = "log/release/state.json";

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let json_out = args.contains(&"--json");
    let note = args.iter().position(|a| *a == "--note").and_then(|i| args.get(i + 1)).map(|s| s.to_string());
    match args.first().copied() {
        Some("start") => {
            let version = args.get(1).filter(|v| !v.starts_with("--")).ok_or("usage: cargo xtask release start X.Y.Z")?;
            crate::version::validate(version)?;
            start(root, version, args.contains(&"--force"))?;
            print_status(root, json_out)
        }
        Some("status") | None => print_status(root, json_out),
        Some("check") => {
            let (pipeline, state) = load(root)?;
            let step = current_step(&pipeline, &state).ok_or("the release is finished")?;
            let report = verify(root, step, version_of(&state));
            print_report(&report);
            if report.iter().all(|r| r.1) { Ok(()) } else { Err("checks failed".into()) }
        }
        Some("done") => advance(root, false, note, json_out),
        Some("skip") => advance(root, true, note, json_out),
        Some("back") => back(root, json_out),
        Some(other) => Err(format!("unknown release command `{other}` (start, status, check, done, skip, back)")),
    }
}

fn load_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn save_state(root: &Path, state: &Value) -> Result<(), String> {
    let path = root.join(STATE);
    std::fs::create_dir_all(path.parent().ok_or("bad state path")?).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())? + "\n";
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn load(root: &Path) -> Result<(Value, Value), String> {
    let pipeline = load_json(&root.join(PIPELINE))?;
    let state_path = root.join(STATE);
    if !state_path.exists() {
        return Err("no release in progress: `cargo xtask release start X.Y.Z`".into());
    }
    Ok((pipeline, load_json(&state_path)?))
}

fn steps(pipeline: &Value) -> &[Value] {
    pipeline["steps"].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn version_of(state: &Value) -> &str {
    state["version"].as_str().unwrap_or("")
}

fn now() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn start(root: &Path, version: &str, force: bool) -> Result<(), String> {
    let pipeline = load_json(&root.join(PIPELINE))?;
    if let Ok(old) = load_json(&root.join(STATE))
        && old["finishedAt"].is_null()
        && !force
    {
        return Err(format!("release {} is still in progress (`cargo xtask release status`); pass --force to replace it", version_of(&old)));
    }
    let entries: Vec<Value> = steps(&pipeline).iter().map(|s| json!({"id": s["id"], "status": "pending"})).collect();
    let first = steps(&pipeline).first().map(|s| s["id"].clone()).unwrap_or(Value::Null);
    save_state(root, &json!({"version": version, "startedAt": now(), "current": first, "finishedAt": null, "steps": entries}))
}

fn current_step<'a>(pipeline: &'a Value, state: &Value) -> Option<&'a Value> {
    let id = state["current"].as_str()?;
    steps(pipeline).iter().find(|s| s["id"].as_str() == Some(id))
}

fn fill(text: &str, version: &str) -> String {
    text.replace("{version}", version)
}

/// (check description, passed, detail)
type Report = Vec<(String, bool, String)>;

fn verify(root: &Path, step: &Value, version: &str) -> Report {
    let checks = step["verify"].as_array().cloned().unwrap_or_default();
    checks.iter().map(|c| check(root, c, version)).collect()
}

fn check(root: &Path, c: &Value, version: &str) -> (String, bool, String) {
    let s = |k: &str| fill(c[k].as_str().unwrap_or(""), version);
    match c["kind"].as_str().unwrap_or("") {
        "cargoVersion" => {
            let found = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string()).and_then(|t| crate::version::read(&t));
            match found {
                Ok(v) => (format!("Cargo.toml version is {version}"), v == version, format!("found {v}")),
                Err(e) => ("Cargo.toml version".into(), false, e),
            }
        }
        "gitClean" => {
            let paths: Vec<String> = c["paths"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(|p| fill(p, version)).collect()).unwrap_or_default();
            let out = Command::new("git").current_dir(root).args(["status", "--porcelain", "--"]).args(&paths).output();
            match out {
                Ok(o) if o.status.success() => {
                    let dirty = String::from_utf8_lossy(&o.stdout).trim().to_owned();
                    (format!("committed: {}", paths.join(", ")), dirty.is_empty(), if dirty.is_empty() { "clean".into() } else { dirty })
                }
                Ok(o) => ("git status".into(), false, String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                Err(e) => ("git status".into(), false, e.to_string()),
            }
        }
        "files" => {
            let mut missing = Vec::new();
            let mut found = Vec::new();
            for p in c["paths"].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_str) {
                let pat = fill(p, version);
                match glob_one(root, &pat) {
                    Some(hit) => found.push(hit),
                    None => missing.push(pat),
                }
            }
            let ok = missing.is_empty();
            ("files exist".into(), ok, if ok { found.join(", ") } else { format!("missing: {}", missing.join(", ")) })
        }
        "command" => {
            let cmd = s("run");
            match Command::new("sh").current_dir(root).args(["-c", &cmd]).output() {
                Ok(o) => {
                    let detail = String::from_utf8_lossy(if o.status.success() { &o.stdout } else { &o.stderr }).trim().chars().take(300).collect();
                    (format!("`{cmd}` succeeds"), o.status.success(), detail)
                }
                Err(e) => (format!("`{cmd}`"), false, e.to_string()),
            }
        }
        "urlJson" => {
            let (url, field, want) = (s("url"), s("field"), s("equals"));
            let label = format!("{url} has {field} = {want}");
            match Command::new("curl").args(["-fsS", "--max-time", "20", "-H", "Cache-Control: no-cache", &url]).output() {
                Ok(o) if o.status.success() => match serde_json::from_slice::<Value>(&o.stdout) {
                    Ok(v) => {
                        let got = v[&field].as_str().map(str::to_owned).unwrap_or_else(|| v[&field].to_string());
                        (label, got == want, format!("found {got}"))
                    }
                    Err(e) => (label, false, format!("not JSON: {e}")),
                },
                Ok(o) => (label, false, String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                Err(e) => (label, false, e.to_string()),
            }
        }
        other => (format!("unknown check `{other}`"), false, "fix packaging/release/pipeline.json".into()),
    }
}

/// First existing path for `pattern`, which may have `*` in its last component.
fn glob_one(root: &Path, pattern: &str) -> Option<String> {
    let full = root.join(pattern);
    if !pattern.contains('*') {
        return full.exists().then(|| pattern.to_owned());
    }
    let dir: PathBuf = full.parent()?.to_path_buf();
    let name = full.file_name()?.to_string_lossy().into_owned();
    let mut hits: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| wildcard(&name, n))
        .collect();
    hits.sort();
    hits.first().map(|h| dir.join(h).strip_prefix(root).map(|p| p.display().to_string()).unwrap_or_else(|_| h.clone()))
}

/// `*` matches any run of characters; everything else literally.
fn wildcard(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else { return false };
    if parts.len() == 1 {
        return pattern == name;
    }
    if !name.starts_with(first) || !name.ends_with(last) || name.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

fn print_report(report: &Report) {
    for (what, ok, detail) in report {
        println!("  [{}] {what}{}", if *ok { "ok" } else { "FAIL" }, if detail.is_empty() { String::new() } else { format!(" ({detail})") });
    }
}

fn set_status(state: &mut Value, id: &str, status: &str, note: Option<String>) {
    if let Some(entries) = state["steps"].as_array_mut()
        && let Some(e) = entries.iter_mut().find(|e| e["id"].as_str() == Some(id))
    {
        e["status"] = json!(status);
        e["at"] = json!(now());
        if let Some(n) = note {
            e["note"] = json!(n);
        }
    }
}

fn advance(root: &Path, skip: bool, note: Option<String>, json_out: bool) -> Result<(), String> {
    let (pipeline, mut state) = load(root)?;
    let step = current_step(&pipeline, &state).ok_or("the release is finished")?.clone();
    let id = step["id"].as_str().unwrap_or("").to_owned();
    let version = version_of(&state).to_owned();
    if skip {
        if !step["optional"].as_bool().unwrap_or(false) {
            return Err(format!("`{id}` isn't optional; finish it, then `cargo xtask release done`"));
        }
        set_status(&mut state, &id, "skipped", note);
    } else {
        let report = verify(root, &step, &version);
        print_report(&report);
        if !report.iter().all(|r| r.1) {
            return Err(format!("`{id}` isn't done yet: the checks above failed"));
        }
        set_status(&mut state, &id, "done", note);
    }
    let all = steps(&pipeline);
    let next = all.iter().position(|s| s["id"].as_str() == Some(id.as_str())).and_then(|i| all.get(i + 1));
    match next {
        Some(n) => state["current"] = n["id"].clone(),
        None => {
            state["current"] = Value::Null;
            state["finishedAt"] = json!(now());
        }
    }
    save_state(root, &state)?;
    print_status(root, json_out)
}

fn back(root: &Path, json_out: bool) -> Result<(), String> {
    let (pipeline, mut state) = load(root)?;
    let all = steps(&pipeline);
    let at = match state["current"].as_str() {
        Some(id) => all.iter().position(|s| s["id"].as_str() == Some(id)).unwrap_or(0),
        None => all.len(),
    };
    let prev = at.checked_sub(1).and_then(|i| all.get(i)).ok_or("already at the first step")?;
    let id = prev["id"].as_str().unwrap_or("").to_owned();
    set_status(&mut state, &id, "pending", None);
    state["current"] = json!(id);
    state["finishedAt"] = Value::Null;
    save_state(root, &state)?;
    print_status(root, json_out)
}

fn print_status(root: &Path, json_out: bool) -> Result<(), String> {
    let (pipeline, state) = load(root)?;
    let version = version_of(&state);
    let step = current_step(&pipeline, &state);
    if json_out {
        let current = step.map(|s| {
            let fill_all = |k: &str| s[k].as_array().map(|a| a.iter().filter_map(Value::as_str).map(|t| fill(t, version)).collect::<Vec<_>>()).unwrap_or_default();
            json!({"id": s["id"], "title": s["title"], "optional": s["optional"].as_bool().unwrap_or(false), "do": fill_all("do"), "verify": s["verify"]})
        });
        let out = json!({"version": version, "finished": state["finishedAt"].is_string(), "current": current, "steps": state["steps"]});
        println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
        return Ok(());
    }
    println!("Prism {version} release");
    let entries = state["steps"].as_array().cloned().unwrap_or_default();
    for (i, s) in steps(&pipeline).iter().enumerate() {
        let id = s["id"].as_str().unwrap_or("");
        let status = entries.iter().find(|e| e["id"].as_str() == Some(id)).and_then(|e| e["status"].as_str()).unwrap_or("pending");
        let mark = match status {
            "done" => "✔",
            "skipped" => "–",
            _ if state["current"].as_str() == Some(id) => "▶",
            _ => " ",
        };
        let opt = if s["optional"].as_bool().unwrap_or(false) { " (optional)" } else { "" };
        println!("  {mark} {}. {}{opt}", i + 1, s["title"].as_str().unwrap_or(id));
    }
    match step {
        Some(s) => {
            println!();
            println!("Now: {}", s["title"].as_str().unwrap_or(""));
            for line in s["do"].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_str) {
                println!("  - {}", fill(line, version));
            }
            if s["verify"].as_array().is_some_and(|a| !a.is_empty()) {
                println!("When it's done: `cargo xtask release done` (checks it first).");
            } else {
                println!("When it's done: `cargo xtask release done` (nothing to check automatically).");
            }
            if s["optional"].as_bool().unwrap_or(false) {
                println!("Not needed: `cargo xtask release skip`.");
            }
        }
        None => println!("\nAll done: Prism {version} is released."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard("prism-1.2.0-macos-*.app.zip", "prism-1.2.0-macos-universal.app.zip"));
        assert!(!wildcard("prism-1.2.0-macos-*.app.zip", "prism-1.2.0-macos-universal.dmg"));
        assert!(wildcard("a*b*c", "aXXbYYc"));
        assert!(!wildcard("a*b*c", "ac"));
        assert!(wildcard("exact", "exact"));
    }

    #[test]
    fn the_pipeline_is_valid() {
        let root = crate::root();
        let p = load_json(&root.join(PIPELINE)).unwrap();
        let all = steps(&p);
        assert!(!all.is_empty());
        let mut ids = std::collections::HashSet::new();
        for s in all {
            assert!(ids.insert(s["id"].as_str().unwrap()), "duplicate id");
            assert!(s["title"].is_string() && s["do"].is_array());
            for c in s["verify"].as_array().unwrap() {
                let kind = c["kind"].as_str().unwrap();
                assert!(["cargoVersion", "gitClean", "files", "command", "urlJson"].contains(&kind), "{kind}");
            }
        }
    }
}
