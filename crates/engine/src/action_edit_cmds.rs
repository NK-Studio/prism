//! Action options and step editing, available to every command client.
use crate::actions_cmds::{self, ActionStep};
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
fn bad(msg: &str) -> EngineError {
    EngineError::BadParams { cmd: "actions.configure".into(), msg: msg.into() }
}
pub(crate) fn valid_name(v: &Value) -> Result<String> {
    v.as_str().filter(|s| !s.trim().is_empty() && s.chars().count() <= 255).map(str::to_owned).ok_or_else(|| bad("name must contain 1–255 characters"))
}
fn configure(s: &mut Session, p: &Value) -> Result<Value> {
    actions_cmds::not_playing(s, "actions.configure")?;
    let idx = actions_cmds::resolve(&s.actions.list, p, "actions.configure")?;
    let mut action = s.actions.list.get(idx).cloned().ok_or_else(|| bad("no such action"))?;
    if let Some(step) = p.get("step") {
        let n = step.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("step must be a non-negative integer"))?;
        // Validate against a private copy including live recording entries before mutation.
        action.steps.extend(actions_cmds::pending_steps(s, idx).cloned().map(ActionStep::from));
        let item = action.steps.get_mut(n).ok_or_else(|| bad("step out of range"))?;
        for (key, flag) in [("enabled", &mut item.enabled), ("dialog", &mut item.dialog)] {
            if let Some(v) = p.get(key) {
                *flag = v.as_bool().ok_or_else(|| bad("step flags must be boolean"))?;
            }
        }
        if let Some(params) = p.get("params") {
            if !params.is_object() {
                return Err(bad("params must be an object"));
            }
            item.params = params.clone();
        }
        if s.actions.recording.is_some_and(|(_, i)| i == idx) {
            s.actions.recording = Some((s.journal.len(), idx));
        }
    } else {
        if let Some(set) = p.get("set") {
            action.set = valid_name(set)?;
        }
        // Names stay globally unique to preserve existing shortcuts, nested calls and droplets.
        if let Some(name) = p.get("name") {
            let name = valid_name(name)?;
            if name != action.name {
                return Err(bad("renaming an action is not yet supported; existing named calls must remain valid"));
            }
        }
    }
    let result = json!({"name":action.name,"set":action.set,"steps":action.steps.len()});
    let slot = s.actions.list.get_mut(idx).ok_or_else(|| bad("no such action"))?;
    *slot = action;
    s.actions.touch();
    Ok(result)
}
fn insert_stop(s: &mut Session, p: &Value) -> Result<Value> {
    actions_cmds::not_playing(s, "actions.insertStop")?;
    let idx = actions_cmds::resolve(&s.actions.list, p, "actions.insertStop")?;
    let message = p.get("message").and_then(Value::as_str).filter(|m| m.len() <= 4096).ok_or_else(|| bad("message must be a string up to 4096 bytes"))?;
    let allow = p.get("allowContinue").map_or(Ok(false), |v| v.as_bool().ok_or_else(|| bad("allowContinue must be boolean")))?;
    let len = s.actions.list.get(idx).map_or(0, |a| a.steps.len()).saturating_add(actions_cmds::pending_steps(s, idx).count());
    let at = match p.get("after") {
        None => len,
        Some(v) => {
            v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < len).and_then(|n| n.checked_add(1)).ok_or_else(|| bad("after is out of range"))?
        }
    };
    actions_cmds::flush_recording(s)?;
    let a = s.actions.list.get_mut(idx).ok_or_else(|| bad("no such action"))?;
    a.steps.insert(at, ("actions.message".into(), json!({"message":message,"allowContinue":allow})).into());
    s.actions.touch();
    Ok(json!({"step":at}))
}
fn cancel(s: &mut Session, _: &Value) -> Result<Value> {
    if s.actions.playing > 0 {
        return Err(bad("cannot cancel synchronous playback"));
    }
    Ok(json!({"cancelled":s.actions.paused.take().is_some()}))
}
fn interaction(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("this action requires interactive playback; run it from the Actions panel"))
}
fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    actions_cmds::not_playing(s, "actions.duplicate")?;
    let idx = actions_cmds::resolve(&s.actions.list, p, "actions.duplicate")?;
    if s.actions.recording.is_some() {
        return Err(bad("stop recording before duplicating"));
    }
    let mut copy = s.actions.list.get(idx).cloned().ok_or_else(|| bad("no such action"))?;
    if let Some(step) = p.get("step") {
        let n = step.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad("step must be an integer"))?;
        let item = copy.steps.get(n).cloned().ok_or_else(|| bad("step out of range"))?;
        copy.steps.insert(n.saturating_add(1), item);
        let slot = s.actions.list.get_mut(idx).ok_or_else(|| bad("no such action"))?;
        *slot = copy;
        s.actions.touch();
        return Ok(json!({"action":idx,"step":n.saturating_add(1)}));
    }
    copy.name = valid_name(p.get("name").ok_or_else(|| bad("new action name required"))?)?;
    if s.actions.list.iter().any(|a| a.name == copy.name) {
        return Err(bad("action name already exists"));
    }
    if let Some(set) = p.get("set") {
        copy.set = valid_name(set)?;
    }
    let index = s.actions.list.len();
    s.actions.list.push(copy);
    s.actions.touch();
    Ok(json!({"index":index}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec("actions.duplicate", "Duplicate Action or Step", r#"{action:name|index,step:index?} or {action,name:str,set:str?}"#, duplicate),
        spec(
            "actions.configure",
            "Action Options",
            r#"{action:name|index,set:str?} or {action,step:index,enabled:bool?,dialog:bool?,params:object?}. Edits flags and recorded settings."#,
            configure,
        ),
        spec(
            "actions.insertStop",
            "Insert Stop",
            r#"{action:name|index,after:index?,message:str,allowContinue:bool=false}. Insert after a selected step or append."#,
            insert_stop,
        ),
        spec("actions.resume", "Resume Action", r#"{params:object?}. Continue a paused action; params replaces settings for this execution only."#, |s, p| {
            crate::action_playback::run(s, p, true)
        }),
        spec("actions.cancel", "Cancel Action Playback", "{} → {cancelled:bool}", cancel),
        spec("actions.message", "Stop", "Internal stop step. Use actions.insertStop.", interaction),
        spec("actions.requireInteraction", "Action Requires Settings", "Internal guard for interactive batch steps.", interaction),
    ]
}

fn spec(id: &'static str, label: &'static str, params: &'static str, run: fn(&mut Session, &Value) -> Result<Value>) -> CommandSpec {
    CommandSpec { id, label, params, menu: &[], shortcut: None, enabled: |_| Ok(()), run, journal: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions_cmds::{Action, ActionsFile};
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.actions.list.push(Action::new("Child", vec![("layer.new.layer".into(), json!({"name":"child"}))]));
        s.actions
            .list
            .push(Action::new("Parent", vec![("actions.play".into(), json!({"action":"Child"})), ("layer.new.layer".into(), json!({"name":"after"}))]));
        s
    }
    #[test]
    fn nested_modal_resume_does_not_repeat_and_keeps_recorded_parameters() {
        let mut s = session();
        s.execute("actions.configure", json!({"action":"Child","step":0,"dialog":true})).unwrap();
        let p = s.execute("actions.play", json!({"action":"Parent"})).unwrap();
        assert_eq!(p["paused"]["kind"], "dialog");
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert!(s.execute("actions.move", json!({"action":0,"to":1})).is_err());
        let p = s.execute("actions.resume", json!({"params":{"name":"changed once"}})).unwrap();
        assert!(p.get("failed").is_none(), "{p}");
        assert_eq!(s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.as_str()).collect::<Vec<_>>(), ["changed once", "after"]);
        assert_eq!(s.actions.list[0].steps[0].params["name"], "child");
        assert!(s.actions.paused.is_none());
        assert!(s.execute("actions.resume", json!({})).is_err());
    }
    #[test]
    fn stop_in_nested_action_preserves_parent_continuation_and_allows_manual_work() {
        let mut s = session();
        s.execute("actions.insertStop", json!({"action":"Child","message":"Inspect","allowContinue":true})).unwrap();
        let p = s.execute("actions.play", json!({"action":"Parent"})).unwrap();
        assert_eq!(p["paused"]["kind"], "stop");
        assert_eq!(s.active().unwrap().doc.layers.len(), 2);
        s.execute("layer.new.layer", json!({"name":"manual"})).unwrap();
        s.execute("actions.resume", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.as_str()).collect::<Vec<_>>(), ["child", "manual", "after"]);
    }
    #[test]
    fn disabled_steps_keep_flags_through_move_duplicate_delete_and_v1_migration() {
        let old = json!({"version":1,"actions":[{"name":"Legacy","steps":[["unknown",{}],["layer.new.layer",{}]]}]});
        let file: ActionsFile = serde_json::from_value(old).unwrap();
        let mut s = session();
        s.actions.list = file.actions;
        assert_eq!(s.actions.list[0].set, "Default Actions");
        s.execute("actions.configure", json!({"action":0,"step":0,"enabled":false,"dialog":true})).unwrap();
        s.execute("actions.configure", json!({"action":0,"set":"Retouch"})).unwrap();
        s.execute("actions.move", json!({"action":0,"step":0,"to":1})).unwrap();
        s.execute("actions.duplicate", json!({"action":0,"step":1})).unwrap();
        s.execute("actions.delete", json!({"action":0,"step":1})).unwrap();
        let saved = serde_json::to_value(ActionsFile { version: 2, actions: s.actions.list.clone() }).unwrap();
        let restored: ActionsFile = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.actions, s.actions.list);
        assert!(!s.actions.list[0].steps[1].enabled);
        assert!(s.actions.list[0].steps[1].dialog);
        let r = s.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(r["ran"], 1);
        assert!(r.get("failed").is_none());
        assert_eq!(s.execute("actions.get", json!({"action":0})).unwrap()["steps"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn resumed_steps_are_authorized_and_bad_edits_are_atomic() {
        let mut s = session();
        s.execute("actions.configure", json!({"action":0,"step":0,"dialog":true})).unwrap();
        let before = s.actions.list.clone();
        for id in ["actions.configure", "actions.insertStop", "actions.duplicate"] {
            for p in [json!({}), json!({"action":0,"step":-1})] {
                // insertStop / duplicate reject absent required fields, configure rejects invalid flags.
                assert!(s.execute(id, p).is_err(), "{id}");
                assert_eq!(s.actions.list, before);
            }
        }
        assert!(s.execute("actions.configure", json!({"action":0,"step":0,"enabled":false,"dialog":"yes"})).is_err());
        assert_eq!(s.actions.list, before);
        s.execute("actions.play", json!({"action":1})).unwrap();
        assert!(s.execute("actions.resume", json!({"params":3})).is_err());
        assert!(s.actions.paused.is_some());
        s.authorize = Some(|id, _| if id == "layer.new.layer" { Err(EngineError::Other("denied resumed layer".into())) } else { Ok(()) });
        let r = s.execute("actions.resume", json!({})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("denied resumed"));
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert_eq!(s.actions.playing, 0);
        assert!(s.actions.playback_stack.is_empty());
    }

    #[test]
    fn originating_policy_survives_unrestricted_resume() {
        let mut s = session();
        s.execute("actions.insertStop", json!({"action":0,"message":"wait"})).unwrap();
        s.execute("actions.move", json!({"action":0,"step":1,"to":0})).unwrap();
        s.authorize = Some(|id, _| if id == "layer.new.layer" { Err(EngineError::Other("origin denied".into())) } else { Ok(()) });
        assert!(s.execute("actions.play", json!({"action":0})).unwrap().get("paused").is_some());
        s.authorize = None; // A control request has ended; the user clicks Resume.
        let r = s.execute("actions.resume", json!({})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("origin denied"));
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    }

    #[test]
    fn scripts_cannot_run_past_an_interactive_child_or_hide_nested_failure() {
        let mut s = session();
        s.execute("actions.insertStop", json!({"action":0,"message":"wait"})).unwrap();
        s.execute("actions.move", json!({"action":0,"step":1,"to":0})).unwrap();
        let script = json!({"steps":[["actions.play",{"action":"Child"}],["layer.new.layer",{"name":"script-after"}]]});
        s.actions.list[1].steps[0] = ("file.scripts.browse".into(), script.clone()).into();
        let r = s.execute("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(r.get("failed").is_some(), "{r}");
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert!(s.actions.paused.is_none());
        let r = s.execute("file.scripts.browse", script).unwrap();
        assert_eq!(r["ok"], false);
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert!(s.actions.paused.is_none());
    }
    #[test]
    fn resumed_wrapper_commands_keep_origin_policy_and_manual_scripts_keep_existing_pause() {
        let mut s = session();
        s.actions.list[0].steps = vec![("file.automate.conditionalModeChange".into(), json!({"to":"grayscale"})).into()];
        s.execute("actions.insertStop", json!({"action":0,"message":"wait"})).unwrap();
        s.execute("actions.move", json!({"action":0,"step":1,"to":0})).unwrap();
        s.authorize = Some(|id, _| if id == "image.mode.grayscale" { Err(EngineError::Other("denied mode".into())) } else { Ok(()) });
        s.execute("actions.play", json!({"action":0})).unwrap();
        s.authorize = None;
        let listed = s.execute("file.scripts.browse", json!({"steps":[["actions.list",{}]]})).unwrap();
        assert_eq!(listed["ok"], true, "{listed}");
        let failed = s.execute("file.scripts.browse", json!({"steps":[["file.scripts.browse",{"steps":[["layer.delete",{"layer":999999}]]}]]})).unwrap();
        assert_eq!(failed["ok"], false, "{failed}");
        assert!(s.actions.paused.is_some());
        let resumed = s.execute("file.scripts.browse", json!({"steps":[["actions.resume",{}],["layer.new.layer",{}]]})).unwrap();
        assert_eq!(resumed["ok"], false, "{resumed}");
        assert_eq!(s.active().unwrap().doc.mode, prism_color::ColorMode::Rgb);
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert!(s.actions.authorizers.is_empty());
    }
}
