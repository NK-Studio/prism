//! One playback interpreter shared by the desktop, CLI and MCP.
use crate::actions_cmds::{self, Action};
use crate::{EngineError, Result, Session};
use serde_json::{Value, json};

pub trait Host {
    fn session(&mut self) -> &mut Session;
    fn command(&mut self, id: &str, params: Value) -> Result<Value>;
}
impl Host for Session {
    fn session(&mut self) -> &mut Session {
        self
    }
    fn command(&mut self, id: &str, params: Value) -> Result<Value> {
        self.execute(id, params)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub action: Action,
    pub next: usize,
    pub ran: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Paused {
    pub frames: Vec<Frame>,
    pub reason: Value,
    pub authorizers: Vec<Authorizer>,
}
#[derive(Clone, Copy, Debug)]
pub struct Authorizer(pub fn(&str, &Value) -> Result<()>);
impl PartialEq for Authorizer {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}
/// Retain the initiating request's restrictions across a user/UI resume.
pub fn authorize_step(s: &Session, id: &str, p: &Value) -> Result<()> {
    for auth in &s.actions.authorizers {
        (auth.0)(id, p)?;
    }
    if let Some(auth) = s.authorize {
        auth(id, p)?;
    }
    Ok(())
}
fn bad(msg: &str) -> EngineError {
    EngineError::BadParams { cmd: "actions.play".into(), msg: msg.into() }
}

pub fn run(host: &mut impl Host, p: &Value, resume: bool) -> Result<Value> {
    if host.session().actions.playing > 0 {
        return Err(bad("an action is already playing; insert a named action call directly instead of calling it through a script"));
    }
    let mut accepted_dialog = false;
    let mut override_params = None;
    let mut authorizers = Vec::new();
    let mut frames = if resume {
        if let Some(v) = p.get("params") {
            if !v.is_object() {
                return Err(bad("params must be an object"));
            }
            override_params = Some(v.clone());
        }
        let pending = host.session().actions.paused.take().ok_or_else(|| bad("no paused action"))?;
        accepted_dialog = pending.reason["kind"] == "dialog";
        authorizers = pending.authorizers;
        pending.frames
    } else {
        if host.session().actions.paused.is_some() {
            return Err(bad("resume or cancel the paused action first"));
        }
        let (action, from) = actions_cmds::playback_plan(host.session(), p)?;
        crate::atn_import_cmds::preflight(&host.session().actions.list, &action, from).map_err(|e| bad(&e))?;
        vec![Frame { action, next: from, ran: 0 }]
    };
    let root = frames.first().ok_or_else(|| bad("empty playback stack"))?.action.clone();
    let from = frames.first().map_or(0, |f| f.next);
    let recording = actions_cmds::begin_playback(host.session(), &root)?;
    if let Some(auth) = host.session().authorize {
        let auth = Authorizer(auth);
        if !authorizers.contains(&auth) {
            authorizers.push(auth);
        }
    }
    host.session().actions.authorizers = authorizers.clone();
    let mut budget = 100_000usize;
    let result = loop {
        let Some(frame) = frames.last() else { break json!({"action":root.name,"ran":0}) };
        let Some(step) = frame.action.steps.get(frame.next).cloned() else {
            let count = frame.ran;
            frames.pop();
            if let Some(parent) = frames.last_mut() {
                parent.ran = parent.ran.saturating_add(1);
                continue;
            }
            break json!({"action":root.name,"ran":count});
        };
        let index = frame.next;
        let name = frame.action.name.clone();
        if budget == 0 {
            break failure(&root, &frames, index, &step.id, "action exceeds 100000 playback steps");
        }
        budget -= 1;
        if !step.enabled {
            if let Some(frame) = frames.last_mut() {
                frame.next = frame.next.saturating_add(1);
            }
            continue;
        }
        let params = if accepted_dialog { override_params.take().unwrap_or_else(|| step.params.clone()) } else { step.params.clone() };
        if let Err(e) = authorize_step(host.session(), &step.id, &params) {
            break failure(&root, &frames, index, &step.id, &e.to_string());
        }
        if step.id == "actions.message" || (step.dialog && !accepted_dialog) {
            if recording.is_some() {
                break failure(&root, &frames, index, &step.id, "interactive playback cannot pause while recording another action");
            }
            let reason = if step.id == "actions.message" {
                if let Some(frame) = frames.last_mut() {
                    frame.next = frame.next.saturating_add(1);
                }
                json!({"kind":"stop","action":name,"step":index,"message":params.get("message").and_then(Value::as_str).unwrap_or(""),"allowContinue":params.get("allowContinue").and_then(Value::as_bool).unwrap_or(false)})
            } else {
                json!({"kind":"dialog","action":name,"step":index,"id":step.id,"params":params})
            };
            let result = json!({"action":root.name,"ran":frames.first().map_or(0, |f| f.ran),"paused":reason});
            host.session().actions.paused = Some(Paused { frames: frames.clone(), reason, authorizers: authorizers.clone() });
            break result;
        }
        accepted_dialog = false;
        if step.id == "actions.play" {
            let next = actions_cmds::playback_plan(host.session(), &params);
            match next {
                Ok((child, from)) => {
                    // A settings dialog can change the called action after the root
                    // was checked. Validate its resolved continuation before any child edit.
                    if let Err(error) = crate::atn_import_cmds::preflight(&host.session().actions.list, &child, from) {
                        break failure(&root, &frames, index, &step.id, &error);
                    }
                    if frames.iter().any(|f| f.action.name == child.name) {
                        break failure(&root, &frames, index, &step.id, "recursive action call");
                    }
                    if frames.len() >= 16 {
                        break failure(&root, &frames, index, &step.id, "action nesting exceeds 16 levels");
                    }
                    if let Some(frame) = frames.last_mut() {
                        frame.next = frame.next.saturating_add(1);
                    }
                    frames.push(Frame { action: child, next: from, ran: 0 });
                }
                Err(e) => break failure(&root, &frames, index, &step.id, &e.to_string()),
            }
            continue;
        }
        match host.command(&step.id, params) {
            Ok(value) => {
                if let Some(e) = actions_cmds::nested_failure(&step.id, &value) {
                    break failure(&root, &frames, index, &step.id, &e);
                }
                if let Some(frame) = frames.last_mut() {
                    frame.next = frame.next.saturating_add(1);
                    frame.ran = frame.ran.saturating_add(1);
                }
            }
            Err(e) => break failure(&root, &frames, index, &step.id, &e.to_string()),
        }
    };
    actions_cmds::finish_playback(host.session(), recording, &root, from);
    host.session().actions.authorizers.clear();
    Ok(result)
}
fn failure(root: &Action, frames: &[Frame], step: usize, id: &str, error: &str) -> Value {
    json!({"action":root.name,"ran":frames.first().map_or(0, |f| f.ran),"failed":{"action":frames.last().map(|f| &f.action.name),"step":step,"id":id,"error":error}})
}
