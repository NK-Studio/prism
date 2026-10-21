//! SUT tool-property visibility belongs to the preset, never to rendering settings.
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};
use serde_json::{Value, json};

fn bad(cmd: &str, msg: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn index(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    if !p.is_object() {
        return Err(bad(cmd, "parameters must be an object"));
    }
    let name = match p.get("name") {
        None => s.tools.brush_preset.as_deref().ok_or_else(|| bad(cmd, "no selected brush preset"))?,
        Some(Value::String(name)) if !name.trim().is_empty() => name.trim(),
        Some(_) => return Err(bad(cmd, "name must be a non-empty string")),
    };
    s.tools.presets.iter().position(|preset| preset.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(cmd, "no such brush preset"))
}

fn response(s: &Session, i: usize, cmd: &str) -> Result<Value> {
    let preset = s.tools.presets.get(i).ok_or_else(|| bad(cmd, "no such brush preset"))?;
    let metadata = preset.sut.as_ref().ok_or_else(|| bad(cmd, "preset has no SUT display metadata; import its SUT again"))?;
    Ok(json!({"name":preset.name,"original":metadata.original,"custom":metadata.custom,"effective":metadata.effective()}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.sutDisplay.get";
    response(s, index(s, p, cmd)?, cmd)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.sutDisplay.set";
    let i = index(s, p, cmd)?;
    let metadata = s.tools.presets.get(i).and_then(|preset| preset.sut.as_ref()).ok_or_else(|| bad(cmd, "preset has no SUT display metadata"))?;
    let mut display = metadata.effective().clone();
    if let Some(id) = p.get("parameter") {
        let id = id.as_u64().and_then(|id| u32::try_from(id).ok()).ok_or_else(|| bad(cmd, "parameter must be a u32 identifier"))?;
        let visible = p.get("visible").and_then(Value::as_bool).ok_or_else(|| bad(cmd, "visible must be a boolean"))?;
        if !prism_paint::sut::is_known_parameter(id) && !metadata.original.visible.contains(&id) {
            return Err(bad(cmd, "unknown display parameter"));
        }
        if visible && !metadata.original.visible.contains(&id) {
            let source_key =
                if id == 100 { Some("TextureDensity") } else { prism_paint::sut::PARAMETER_KEYS.iter().find(|(known, _)| *known == id).map(|(_, key)| *key) };
            if !metadata.kind().is_verified_brush() || !source_key.is_some_and(|key| metadata.parameters.contains_key(key)) {
                return Err(bad(cmd, "parameter is unavailable for this imported sub-tool"));
            }
        }
        if visible && !display.visible.contains(&id) {
            if display.visible.len() >= prism_paint::sut::MAX_SUT_DISPLAY_ITEMS {
                return Err(bad(cmd, "too many visible parameters"));
            }
            // Restore imported positions; new controls append (the UI uses category order).
            let pos = metadata
                .original
                .visible
                .iter()
                .position(|entry| *entry == id)
                .and_then(|original_pos| {
                    display
                        .visible
                        .iter()
                        .position(|entry| metadata.original.visible.iter().position(|original| original == entry).is_some_and(|p| p > original_pos))
                })
                .unwrap_or(display.visible.len());
            display.visible.insert(pos, id);
        } else if !visible {
            display.visible.retain(|entry| *entry != id);
        }
    } else if p.get("visible").is_some() {
        return Err(bad(cmd, "visible requires parameter"));
    }
    if let Some(categories) = p.get("showCategories") {
        display.show_categories = categories.as_bool().ok_or_else(|| bad(cmd, "showCategories must be a boolean"))?;
    }
    if p.get("parameter").is_none() && p.get("showCategories").is_none() {
        return Err(bad(cmd, "provide parameter and visible, or showCategories"));
    }
    let metadata = s.tools.presets.get_mut(i).and_then(|preset| preset.sut.as_mut()).ok_or_else(|| bad(cmd, "preset has no SUT display metadata"))?;
    metadata.custom = (display != metadata.original).then_some(display);
    s.brush_presets_changed();
    response(s, i, cmd)
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.sutDisplay.reset";
    let i = index(s, p, cmd)?;
    let metadata = s.tools.presets.get_mut(i).and_then(|preset| preset.sut.as_mut()).ok_or_else(|| bad(cmd, "preset has no SUT display metadata"))?;
    metadata.custom = None;
    s.brush_presets_changed();
    response(s, i, cmd)
}

/// Change imported sub-tool settings (the CSP Tool Property of non-brush sub-tools, see
/// `prism_paint::sut_catalog`): `values` maps allowlisted keys to finite numbers up to
/// `MAX_SETTING_VALUE` (32-bit colour channels), or the text keys (font names) to strings of at
/// most `MAX_SETTING_CHARS` characters; `null` removes a value. `gradient` replaces the
/// gradient's stops (`null` removes it).
fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    use prism_paint::csp_gradient::CspGradient;
    use prism_paint::sut::{MAX_SETTING_CHARS, MAX_SETTING_VALUE, clean_text, is_retained_key, is_string_key};
    enum Change {
        Number(Option<f64>),
        Text(Option<String>),
    }
    let cmd = "brush.sutSettings.set";
    let i = index(s, p, cmd)?;
    let empty = serde_json::Map::new();
    let values = match p.get("values") {
        None if p.get("gradient").is_some() => &empty,
        v => v.and_then(Value::as_object).ok_or_else(|| bad(cmd, "values must be an object"))?,
    };
    if values.len() > 64 || (values.is_empty() && p.get("gradient").is_none()) {
        return Err(bad(cmd, "values must hold 1 to 64 settings"));
    }
    let mut changes = Vec::with_capacity(values.len());
    for (key, value) in values {
        let change = if is_string_key(key) {
            Change::Text(match value {
                Value::Null => None,
                Value::String(t) if t.chars().count() <= MAX_SETTING_CHARS && !clean_text(t).is_empty() => Some(clean_text(t)),
                _ => return Err(bad(cmd, "font names must be non-empty strings of at most 256 characters, or null")),
            })
        } else if is_retained_key(key) && key != "PrismTextureScaleFactor" {
            Change::Number(match value {
                Value::Null => None,
                v => Some(v.as_f64().filter(|v| v.is_finite() && v.abs() <= MAX_SETTING_VALUE).ok_or_else(|| bad(cmd, "setting values must be finite numbers or null"))?),
            })
        } else {
            return Err(bad(cmd, "unknown sub-tool setting"));
        };
        changes.push((key.clone(), change));
    }
    let gradient = match p.get("gradient") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => {
            let g: CspGradient = serde_json::from_value(v.clone())
                .map_err(|_| bad(cmd, r#"gradient is {"stops":[{"position":0..1,"color":"foreground"|"background"|{"rgb":[r,g,b]},"opacity":0..1,"curve":[[x,y],…]?},…]}"#))?;
            if !g.is_valid() {
                return Err(bad(cmd, "a gradient needs 2 to 256 stops sorted by position, values in 0..1 and at most 64 curve points sorted by x"));
            }
            Some(Some(g))
        }
    };
    let metadata = s.tools.presets.get_mut(i).and_then(|preset| preset.sut.as_mut()).ok_or_else(|| bad(cmd, "preset has no SUT settings; import its SUT again"))?;
    for (key, change) in changes {
        match change {
            Change::Number(Some(v)) => {
                metadata.parameters.insert(key, v);
            }
            Change::Number(None) => {
                metadata.parameters.remove(&key);
            }
            Change::Text(Some(t)) => {
                metadata.strings.insert(key, t);
            }
            Change::Text(None) => {
                metadata.strings.remove(&key);
            }
        }
    }
    if let Some(g) = gradient {
        metadata.gradient = g;
    }
    let mut out = json!({"values": metadata.parameters, "strings": metadata.strings, "gradient": metadata.gradient});
    s.brush_presets_changed();
    out["name"] = json!(s.tools.presets.get(i).map(|p| p.name.clone()).unwrap_or_default());
    Ok(out)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "brush.sutSettings.set",
            label: "Change Imported Sub-tool Settings",
            menu: &[],
            shortcut: None,
            params: r#"{"name":preset?=selected,"values":{key:number|null, fontKey:string|null}?,"gradient":{"stops":[{"position":0..1,"color":"foreground"|"background"|{"rgb":[r,g,b]},"opacity":0..1,"curve":[[x,y],…]?},…]}|null?} → {name,values,strings,gradient}"#,
            enabled: |_| Ok(()),
            run: settings,
            journal: true,
        },
        CommandSpec {
            id: "brush.sutDisplay.get",
            label: "Get SUT Tool Settings Display",
            menu: &[],
            shortcut: None,
            params: r#"{"name":preset?=selected} → {name,original,custom,effective}"#,
            enabled: |_| Ok(()),
            run: get,
            journal: false,
        },
        CommandSpec {
            id: "brush.sutDisplay.set",
            label: "Customize SUT Tool Settings Display",
            menu: &[],
            shortcut: None,
            params: r#"{"name":preset?=selected,"parameter":u32?,"visible":bool?,"showCategories":bool?} → {name,original,custom,effective}"#,
            enabled: |_| Ok(()),
            run: set,
            journal: true,
        },
        CommandSpec {
            id: "brush.sutDisplay.reset",
            label: "Restore Imported SUT Display",
            menu: &[],
            shortcut: None,
            params: r#"{"name":preset?=selected} → {name,original,custom,effective}"#,
            enabled: |_| Ok(()),
            run: reset,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_paint::{
        BrushPreset,
        sut::{SutBrushMetadata, SutDisplay},
    };

    pub(crate) fn sample() -> BrushPreset {
        BrushPreset {
            name: "SUT sample".into(),
            brush: Default::default(),
            builtin: false,
            group: "SUT".into(),
            sut: Some(SutBrushMetadata {
                original: SutDisplay { visible: vec![1000, 10, 0xffff], show_categories: false },
                custom: None,
                input_op: 1,
                output_op: 2,
                parameters: Default::default(),
                node_icon: 0,
                texture_name: "Texture".into(),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn sub_tool_settings_change_allowlisted_values_only() {
        let mut s = Session::new();
        s.tools.presets.push(sample());
        let r = s.execute("brush.sutSettings.set", json!({"name":"SUT sample","values":{"FillCloseGap":10,"FillUseCloseGap":1}})).unwrap();
        assert_eq!(r["values"], json!({"FillCloseGap":10.0,"FillUseCloseGap":1.0}));
        s.execute("brush.sutSettings.set", json!({"name":"SUT sample","values":{"FillCloseGap":null}})).unwrap();
        let p = s.tools.presets.iter().find(|p| p.name == "SUT sample").unwrap();
        assert_eq!(p.sut.as_ref().unwrap().parameters.get("FillCloseGap"), None);
        // Hostile input fails without changing anything.
        for bad in [
            json!({"name":"SUT sample","values":{"ArbitraryColumn":1}}),
            json!({"name":"SUT sample","values":{"PrismTextureScaleFactor":1}}),
            json!({"name":"SUT sample","values":{"FillCloseGap":"5"}}),
            json!({"name":"SUT sample","values":{"FillCloseGap":1e300}}),
            json!({"name":"SUT sample","values":{"TextEdgeColorRed":4294967296.0}}),
            json!({"name":"SUT sample","values":{"TextFontName":1}}),
            json!({"name":"SUT sample","values":{}}),
            json!({"name":"SUT sample","values":[1]}),
            json!({"name":"SUT sample"}),
            json!({"name":"missing","values":{"FillCloseGap":1}}),
            json!({"name":7,"values":{"FillCloseGap":1}}),
            json!(null),
        ] {
            assert!(s.execute("brush.sutSettings.set", bad.clone()).is_err(), "{bad}");
        }
        let p = s.tools.presets.iter().find(|p| p.name == "SUT sample").unwrap();
        assert_eq!(p.sut.as_ref().unwrap().parameters.len(), 1);
        // Every family's catalogued keys are settable, 32-bit colour channels included.
        let r = s.execute("brush.sutSettings.set", json!({"name":"SUT sample","values":{"SelectCreateType":2,"TextEdgeColorRed":4294967295u32,"SpuitReferType":0}})).unwrap();
        assert_eq!(r["values"]["TextEdgeColorRed"], json!(4294967295.0));
        assert_eq!(r["values"]["SelectCreateType"], json!(2.0));
        // Font names are text; gradients replace the stops; hostile ones change nothing.
        let r = s.execute("brush.sutSettings.set", json!({"name":"SUT sample","values":{"TextFontName":" Noto Sans KR ","TextRubyFontName":"Meiryo"}})).unwrap();
        assert_eq!(r["strings"], json!({"TextFontName":"Noto Sans KR","TextRubyFontName":"Meiryo"}));
        let stops = json!({"stops":[{"position":0.0,"color":"foreground","opacity":1.0,"curve":[[0.0,0.0],[1.0,0.0]]},{"position":1.0,"color":{"rgb":[1.0,0.5,0.0]},"opacity":0.5}]});
        let r = s.execute("brush.sutSettings.set", json!({"name":"SUT sample","gradient":stops})).unwrap();
        assert_eq!(r["gradient"]["stops"][1]["color"], json!({"rgb":[1.0,0.5,0.0]}));
        let before = s.tools.presets.clone();
        for bad in [
            json!({"name":"SUT sample","values":{"TextFontName":""}}),
            json!({"name":"SUT sample","values":{"TextFontName":"x".repeat(300)}}),
            json!({"name":"SUT sample","values":{"TextFontName":["a"]}}),
            json!({"name":"SUT sample","values":{"FillCloseGap":1},"gradient":{"stops":[]}}),
            json!({"name":"SUT sample","gradient":{"stops":[{"position":0.0,"color":"foreground","opacity":1.0}]}}),
            json!({"name":"SUT sample","gradient":{"stops":[{"position":1.0,"color":"foreground","opacity":1.0},{"position":0.0,"color":"background","opacity":1.0}]}}),
            json!({"name":"SUT sample","gradient":{"stops":[{"position":0.0,"color":"pink","opacity":1.0},{"position":1.0,"color":"background","opacity":1.0}]}}),
            json!({"name":"SUT sample","gradient":{"stops":[{"position":0.0,"color":"foreground","opacity":2.0},{"position":1.0,"color":"background","opacity":1.0}]}}),
            json!({"name":"SUT sample","gradient":{"stops":[{"position":0.0,"color":{"rgb":[1e40,0,0]},"opacity":1.0},{"position":1.0,"color":"background","opacity":1.0}]}}),
            json!({"name":"SUT sample","gradient":7}),
        ] {
            assert!(s.execute("brush.sutSettings.set", bad.clone()).is_err(), "{bad}");
            assert_eq!(s.tools.presets, before, "{bad}");
        }
        let r = s.execute("brush.sutSettings.set", json!({"name":"SUT sample","values":{"TextRubyFontName":null},"gradient":null})).unwrap();
        assert_eq!((r["strings"].clone(), r["gradient"].clone()), (json!({"TextFontName":"Noto Sans KR"}), Value::Null));
        // A preset without SUT settings is refused.
        s.tools.presets.push(BrushPreset { sut: None, name: "Plain".into(), ..sample() });
        assert!(s.execute("brush.sutSettings.set", json!({"name":"Plain","values":{"FillCloseGap":1}})).is_err());
    }

    #[test]
    fn sut_visibility_is_preset_state_and_selection_survives_edits() {
        let mut s = Session::new();
        s.tools.presets.push(sample());
        s.execute("tools.setBrush", json!({"preset":"sut SAMPLE"})).unwrap();
        s.execute("tools.setBrush", json!({"size":15,"opacity":0.9})).unwrap();
        let brush = s.tools.brush.clone();
        let r = s.execute("brush.sutDisplay.set", json!({"parameter":10,"visible":false,"showCategories":true})).unwrap();
        assert_eq!(r["effective"]["visible"], json!([1000, 0xffff]));
        assert_eq!(s.tools.brush, brush);
        s.execute("brush.presets.save", json!({"name":"Copy"})).unwrap();
        assert_eq!(s.tools.brush_preset.as_deref(), Some("Copy"));
        s.execute("brush.presets.rename", json!({"name":"Copy","newName":"Renamed"})).unwrap();
        assert_eq!(s.tools.brush_preset.as_deref(), Some("Renamed"));
        s.execute("brush.presets.move", json!({"name":"Renamed","group":"Other"})).unwrap();
        s.execute("brush.sutDisplay.reset", json!({})).unwrap();
        let r = s.execute("brush.sutDisplay.get", json!({})).unwrap();
        assert!(r["custom"].is_null());
        assert_eq!(s.tools.brush, brush);
        s.execute("brush.presets.deleteGroup", json!({"group":"Other"})).unwrap();
        assert_eq!(s.tools.brush_preset, None);
        s.execute("tools.setBrush", json!({"preset":"SUT sample","reset":true})).unwrap();
        assert_eq!(s.tools.brush_preset, None);
    }

    #[test]
    fn sut_display_rejects_bad_inputs_atomically() {
        let mut s = Session::new();
        s.tools.presets.push(sample());
        s.execute("tools.setBrush", json!({"preset":"SUT sample"})).unwrap();
        let before = s.tools.presets.clone();
        for p in [
            json!(null),
            json!({}),
            json!({"name":3}),
            json!({"parameter":-1,"visible":true}),
            json!({"parameter":u64::MAX,"visible":true}),
            json!({"parameter":999999,"visible":true}),
            json!({"parameter":10,"visible":false,"showCategories":3}),
            json!({"parameter":10,"visible":1}),
            json!({"visible":true}),
        ] {
            assert!(s.execute("brush.sutDisplay.set", p).is_err());
            assert_eq!(s.tools.presets, before);
        }
        assert!(s.execute("tools.setBrush", json!({"reset":true,"brush":{"size":"invalid"}})).is_err());
        assert_eq!(s.tools.brush_preset.as_deref(), Some("SUT sample"));
        s.execute("brush.sutDisplay.set", json!({"parameter":0xffff,"visible":false})).unwrap();
        assert!(s.execute("brush.sutDisplay.get", json!({"name":"missing"})).is_err());
    }

    #[test]
    fn newly_shown_rows_require_verified_tool_and_source_parameter() {
        let mut s = Session::new();
        let mut preset = sample();
        let metadata = preset.sut.as_mut().unwrap();
        metadata.parameters.insert("TextureDensity".into(), 100.0);
        s.tools.presets.push(preset);
        s.execute("tools.setBrush", json!({"preset":"SUT sample"})).unwrap();
        assert!(s.execute("brush.sutDisplay.set", json!({"parameter":100,"visible":true})).is_err());
        let metadata = s.tools.presets.iter_mut().find(|p| p.name == "SUT sample").unwrap().sut.as_mut().unwrap();
        metadata.input_op = 10;
        metadata.output_op = 10;
        assert!(s.execute("brush.sutDisplay.set", json!({"parameter":1050,"visible":true})).is_err());
        s.execute("brush.sutDisplay.set", json!({"parameter":100,"visible":true})).unwrap();
        s.execute("brush.sutDisplay.set", json!({"parameter":110,"visible":true})).unwrap();
    }
}
