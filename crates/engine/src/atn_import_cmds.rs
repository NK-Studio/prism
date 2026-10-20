//! Import Photoshop action definitions; importing never runs them.
use crate::action_model::{Action, ActionStep, StepSource};
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub(crate) const UNSUPPORTED: &str = "actions.unsupportedAtnStep";
fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "actions.importAtn".into(), msg: message.into() }
}
pub(crate) fn display_name(raw: &str) -> String {
    if raw.starts_with("$$$/") { raw.split_once('=').map_or(raw, |(_, name)| name).to_string() } else { raw.to_string() }
}
fn valid_name(raw: &str) -> Result<String> {
    let name = display_name(raw);
    if name.trim().is_empty() || name.chars().count() > 255 {
        return Err(bad("ATN names must contain 1–255 characters"));
    }
    Ok(name)
}
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    crate::actions_cmds::not_playing(s, "actions.importAtn")?;
    if s.actions.recording.is_some() {
        return Err(bad("stop recording before importing actions"));
    }
    let replace = match p.get("replace") {
        None => false,
        Some(Value::Bool(b)) => *b,
        _ => return Err(bad("replace must be boolean")),
    };
    let (bytes, _) = crate::preset_import_cmds::file_bytes_with_limit(p, "actions.importAtn", ".atn file", 4 << 20)?;
    let sets = prism_io::atn::read_atn(&bytes, &prism_raster::Interrupt::NONE).map_err(bad)?;
    let mut source_keys = HashSet::new();
    let mut counts = HashMap::<String, usize>::new();
    for set in &sets {
        let set_name = valid_name(&set.name)?;
        for action in &set.actions {
            let name = valid_name(&action.name)?;
            if !source_keys.insert((set_name.clone(), name.clone())) {
                return Err(bad("duplicate action names within an ATN set"));
            }
            if source_keys.len() > 4096 {
                return Err(bad("ATN exceeds 4096 actions"));
            }
            *counts.entry(name).or_default() += 1;
        }
    }
    if source_keys.is_empty() {
        return Err(bad("ATN contains no actions"));
    }
    let mut names = HashMap::new();
    let mut used = HashSet::new();
    for (set, name) in &source_keys {
        let collision = counts.get(name).copied().unwrap_or(0) > 1 || s.actions.list.iter().any(|a| a.name == *name && a.set != *set);
        let target = if collision { format!("{set} / {name}") } else { name.clone() };
        let target = valid_name(&target)?;
        if !used.insert(target.clone()) {
            return Err(bad("ATN action names collide after set qualification"));
        }
        names.insert((set.clone(), name.clone()), target);
    }
    let mut actions = Vec::new();
    let mut imported = Vec::new();
    let mut unsupported = 0usize;
    let mut output_bytes = 0u64;
    for set in sets {
        let set_name = valid_name(&set.name)?;
        for source in set.actions {
            let source_name = valid_name(&source.name)?;
            let target = names.get(&(set_name.clone(), source_name)).cloned().ok_or_else(|| bad("missing action name"))?;
            let mut steps = Vec::new();
            let mut blocked = 0usize;
            for (index, step) in source.steps.iter().enumerate() {
                let event = String::from_utf8_lossy(step.event.as_bytes()).into_owned();
                let (id, params) = match crate::atn_mapping::map(step, &names) {
                    Ok(mapped) => mapped,
                    Err(reason) => {
                        unsupported += 1;
                        blocked += usize::from(step.enabled);
                        (UNSUPPORTED.into(), json!({"event":event,"name":display_name(&step.name),"reason":reason}))
                    }
                };
                let compiled = ActionStep {
                    id,
                    params,
                    enabled: step.enabled,
                    dialog: step.dialog,
                    source: Some(StepSource {
                        format: "atn".into(),
                        command: event,
                        index,
                        details: Some(json!({
                            "name":step.name,"nameBytes":prism_paint::tile::b64_encode(&step.name_bytes),
                            "descriptor":prism_paint::tile::b64_encode(&step.descriptor_bytes),
                            "eventKind":match &step.event { prism_io::atn::Id::Code(_) => "long", prism_io::atn::Id::Str(_) => "TEXT" },
                            "expanded":step.expanded,"dialogOptions":step.dialog_options,
                            "actionName":source.name,"setName":set.name,"actionExpanded":source.expanded,"setExpanded":set.expanded,"shortcutIndex":source.index,
                            "shiftKey":source.shift_key,"commandKey":source.command_key,"color":source.color
                        })),
                    }),
                };
                // Small source records repeat action/set metadata. Bound that expansion
                // incrementally instead of constructing an enormous library before rejecting it.
                let size = serde_json::to_vec(&compiled).map_err(|e| bad(e.to_string()))?.len() as u64;
                output_bytes = output_bytes.saturating_add(size);
                if output_bytes > crate::preset_store::MAX_ACTIONS_BYTES {
                    return Err(bad("expanded ATN steps exceed the persistence size limit"));
                }
                steps.push(compiled);
            }
            imported.push(json!({"name":target,"set":set_name,"steps":steps.len(),"unsupportedEnabled":blocked,"ready":blocked==0}));
            actions.push(Action { name: target, set: set_name.clone(), steps });
        }
    }
    let mut result = s.actions.list.clone();
    for action in actions {
        if let Some(old) = result.iter_mut().find(|a| a.name == action.name) {
            if !replace || old.set != action.set {
                return Err(bad(format!("action `{}` already exists; use replace:true for the same set", action.name)));
            }
            *old = action;
        } else {
            if result.len() >= 4096 {
                return Err(bad("action library exceeds 4096 actions"));
            }
            result.push(action);
        }
    }
    if result.len() > 4096 {
        return Err(bad("action library exceeds 4096 actions"));
    }
    // Match PresetStore::flush_actions exactly, including indentation overhead.
    let saved = serde_json::to_vec_pretty(&crate::actions_cmds::ActionsFile { version: 2, actions: result.clone() }).map_err(|e| bad(e.to_string()))?;
    if saved.len() as u64 > crate::preset_store::MAX_ACTIONS_BYTES {
        return Err(bad("imported action library exceeds the persistence size limit"));
    }
    for entry in &mut imported {
        if let Some(action) = entry.get("name").and_then(Value::as_str).and_then(|name| result.iter().find(|a| a.name == name))
            && let Err(reason) = preflight(&result, action, 0)
        {
            entry["ready"] = json!(false);
            entry["blockedReason"] = json!(reason);
        }
    }
    s.actions.list = result;
    s.actions.touch();
    let mut warnings = vec!["Photoshop commands use Prism implementations; pixel-identical results are not guaranteed.".to_string()];
    if unsupported > 0 {
        warnings.insert(0, format!("{unsupported} unsupported Photoshop steps retained. Disable or replace those steps before playing their actions."));
    }
    Ok(json!({"format":"atn","count":imported.len(),"imported":imported,"unsupported":unsupported,"warnings":warnings}))
}

/// Refuse known unsupported imported steps before changing the document, including direct child calls.
pub(crate) fn preflight(list: &[Action], root: &Action, from: usize) -> std::result::Result<(), String> {
    let mut pending = vec![(root, from)];
    let mut seen = HashSet::new();
    // Import readiness checks visit many roots. Avoid a full library scan for
    // every named edge in a long or shared call graph.
    let by_name: HashMap<_, _> = list.iter().enumerate().map(|(index, action)| (action.name.as_str(), index)).collect();
    let mut budget = 100_000usize;
    while let Some((action, start)) = pending.pop() {
        if !seen.insert((action.name.as_str(), start)) {
            continue;
        }
        for (index, step) in action.steps.iter().enumerate().skip(start).filter(|(_, step)| step.enabled) {
            if budget == 0 {
                return Err("action preflight exceeds 100000 steps".into());
            }
            budget -= 1;
            if step.id == UNSUPPORTED {
                return Err(format!(
                    "{} step {}: unsupported Photoshop command {}: {}. Disable or replace this step before playback.",
                    action.name,
                    index + 1,
                    step.params.get("event").and_then(Value::as_str).unwrap_or(""),
                    step.params.get("reason").and_then(Value::as_str).unwrap_or("unsupported")
                ));
            }
            if step.id == "actions.play" {
                let index = if let Some(name) = step.params.get("action").and_then(Value::as_str) {
                    by_name.get(name).copied()
                } else {
                    crate::actions_cmds::resolve(list, &step.params, "actions.play").ok()
                };
                if let Some(child) = index.and_then(|index| list.get(index)) {
                    let start = crate::actions_cmds::from_step(&step.params, child.steps.len()).map_err(|e| e.to_string())?;
                    pending.push((child, start));
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "actions.importAtn",
            label: "Import Photoshop Actions…",
            menu: &[],
            shortcut: None,
            params: r#"{path:'.atn file'|data:base64,replace:bool=false}. Import definitions; unsupported steps are retained and block playback."#,
            enabled: |_| Ok(()),
            run: import,
            journal: false,
        },
        CommandSpec {
            id: UNSUPPORTED,
            label: "Unsupported Photoshop Step",
            menu: &[],
            shortcut: None,
            params: "Imported unsupported step; disable or replace it before playback.",
            enabled: |_| Ok(()),
            run: |_, p| {
                Err(bad(format!(
                    "unsupported Photoshop step: {}: {}",
                    p.get("event").and_then(Value::as_str).unwrap_or("unknown"),
                    p.get("reason").and_then(Value::as_str).unwrap_or("disable or replace this step")
                )))
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
