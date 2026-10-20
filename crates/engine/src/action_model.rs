//! Prism's independent action model. Legacy command pairs migrate on read.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub fn default_set() -> String {
    "Default Actions".into()
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    #[serde(default = "default_set")]
    pub set: String,
    #[serde(default)]
    pub steps: Vec<ActionStep>,
}
impl Action {
    pub fn new(name: impl Into<String>, steps: Vec<(String, Value)>) -> Self {
        Self { name: name.into(), set: default_set(), steps: steps.into_iter().map(ActionStep::from).collect() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepSource {
    pub format: String,
    pub command: String,
    pub index: usize,
    /// Source-format metadata, kept independently of editable Prism parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActionStep {
    pub id: String,
    pub params: Value,
    pub enabled: bool,
    pub dialog: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<StepSource>,
}
impl From<(String, Value)> for ActionStep {
    fn from((id, params): (String, Value)) -> Self {
        Self { id, params, enabled: true, dialog: false, source: None }
    }
}
impl<'de> Deserialize<'de> for ActionStep {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            Legacy((String, Value)),
            Step {
                id: String,
                params: Value,
                #[serde(default = "yes")]
                enabled: bool,
                #[serde(default)]
                dialog: bool,
                #[serde(default)]
                source: Option<StepSource>,
            },
        }
        Ok(match Input::deserialize(d)? {
            Input::Legacy(pair) => pair.into(),
            Input::Step { id, params, enabled, dialog, source } => Self { id, params, enabled, dialog, source },
        })
    }
}

/// Export a safe batch plan. Interactive steps retain an explicit guard command;
/// batch/droplet consumers must never silently run through a requested pause.
pub fn batch_steps(action: &Action) -> Vec<Value> {
    action
        .steps
        .iter()
        .filter(|s| s.enabled)
        .map(|s| if s.dialog { serde_json::json!(["actions.requireInteraction", {"command":s.id}]) } else { serde_json::json!([s.id, s.params]) })
        .collect()
}
