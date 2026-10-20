//! Conservative Photoshop-event to Prism-command translation. Unknown parameters are not dropped.
use prism_io::atn::{Descriptor, ReferenceItem as Ref, Step, Value as D};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
type Result<T> = std::result::Result<T, String>;
fn keys(d: Option<&Descriptor>, allowed: &[&str]) -> Result<()> {
    let mut seen = HashSet::new();
    if let Some(d) = d {
        for (key, _) in &d.items {
            if !allowed.iter().any(|a| key.is(a)) || !seen.insert(key.as_bytes()) {
                return Err(format!("unsupported or repeated parameter `{}`", String::from_utf8_lossy(key.as_bytes())));
            }
        }
    }
    Ok(())
}
fn get<'a>(d: Option<&'a Descriptor>, key: &str) -> Result<&'a D> {
    d.and_then(|d| d.get(key)).ok_or_else(|| format!("missing recorded parameter `{key}`"))
}
fn number(v: &D, unit: &[u8; 4], min: f64, max: f64) -> Result<f64> {
    let n = match v {
        D::Integer(n) => f64::from(*n),
        D::Double(n) => *n,
        D::UnitFloat { unit: u, value } if u == unit => *value,
        _ => return Err("unsupported numeric type or units".into()),
    };
    if !n.is_finite() || !(min..=max).contains(&n) {
        return Err("recorded number is outside Prism's supported range".into());
    }
    Ok(n)
}
fn text(v: &D) -> Result<String> {
    match v {
        D::Text(s) => Ok(crate::atn_import_cmds::display_name(&s.to_string_lossy())),
        _ => Err("expected text".into()),
    }
}
fn boolean(v: &D) -> Result<bool> {
    if let D::Boolean(b) = v { Ok(*b) } else { Err("expected boolean".into()) }
}
fn enum_is(v: &D, kind: &str, value: &str) -> bool {
    matches!(v,D::Enumerated{type_id,value:v} if type_id.is(kind)&&v.is(value))
}
fn current_layer(v: &D) -> bool {
    matches!(v,D::Reference(r) if matches!(r.as_slice(),[Ref::Enumerated{class,type_id,value}] if class.class_id.is("Lyr ")&&type_id.is("Ordn")&&value.is("Trgt")))
}
fn selection(v: &D) -> bool {
    matches!(v,D::Reference(r) if matches!(r.as_slice(),[Ref::Property{class,key}] if class.class_id.is("Chnl")&&key.is("fsel")))
}
fn empty(id: &str, d: Option<&Descriptor>) -> Result<(String, Value)> {
    keys(d, &[])?;
    Ok((id.into(), json!({})))
}

pub(crate) fn map(step: &Step, names: &HashMap<(String, String), String>) -> Result<(String, Value)> {
    let event = String::from_utf8_lossy(step.event.as_bytes());
    let d = step.descriptor.as_ref();
    let simple = match event.as_ref() {
        "invert" | "Invr" => Some("image.adjustments.invert"),
        "desaturate" | "Dstt" => Some("image.adjustments.desaturate"),
        "copyEvent" | "copy" => Some("edit.copy"),
        "copyMerged" | "CpyM" => Some("edit.copyMerged"),
        "paste" | "past" => Some("edit.paste"),
        "cut" | "cut " => Some("edit.cut"),
        "clearEvent" | "Cler" => Some("edit.clear"),
        "inverse" | "Invs" => Some("select.inverse"),
        "flattenImage" | "FltI" => Some("layer.flattenImage"),
        "mergeVisible" | "MrgV" => Some("layer.mergeVisible"),
        "findEdges" | "FndE" => Some("filter.stylize.findEdges"),
        "solarize" | "Slrz" => Some("filter.stylize.solarize"),
        _ => None,
    };
    if let Some(id) = simple {
        return empty(id, d);
    }
    match event.as_ref() {
        "gaussianBlur" | "GsnB" | "highPass" | "HghP" | "median" | "Mdn " | "feather" | "Fthr" => {
            keys(d, &["Rds "])?;
            let (id, min, max) = match event.as_ref() {
                "gaussianBlur" | "GsnB" => ("filter.blur.gaussianBlur", 0.1, 1000.0),
                "highPass" | "HghP" => ("filter.other.highPass", 0.1, 1000.0),
                "median" | "Mdn " => ("filter.noise.median", 1.0, 500.0),
                _ => ("select.modify.feather", 0.1, 1000.0),
            };
            Ok((id.into(), json!({"radius":number(get(d,"Rds ")?,b"#Pxl",min,max)?})))
        }
        "motionBlur" | "MtnB" => {
            keys(d, &["Angl", "Dstn"])?;
            Ok((
                "filter.blur.motionBlur".into(),
                json!({"angle":number(get(d,"Angl")?,b"#Ang",-360.0,360.0)?,"distance":number(get(d,"Dstn")?,b"#Pxl",1.0,2000.0)?}),
            ))
        }
        "unsharpMask" | "UnsM" => {
            keys(d, &["Amnt", "Rds ", "Thsh"])?;
            Ok((
                "filter.sharpen.unsharpMask".into(),
                json!({"amount":number(get(d,"Amnt")?,b"#Prc",1.0,500.0)?,"radius":number(get(d,"Rds ")?,b"#Pxl",0.1,1000.0)?,"threshold":number(get(d,"Thsh")?,b"#Nne",0.0,255.0)?}),
            ))
        }
        "thresholdClassEvent" | "Thrs" => {
            keys(d, &["Lvl "])?;
            Ok(("image.adjustments.threshold".into(), json!({"level":number(get(d,"Lvl ")?,b"#Nne",1.0,255.0)?})))
        }
        "brightnessEvent" | "BrgC" => {
            keys(d, &["Brgh", "Cntr", "useLegacy"])?;
            let legacy = d.and_then(|d| d.get("useLegacy")).map(boolean).transpose()?.unwrap_or(true);
            Ok((
                "image.adjustments.brightnessContrast".into(),
                json!({"brightness":number(get(d,"Brgh")?,b"#Nne",if legacy {-100.0} else {-150.0},if legacy {100.0} else {150.0})?,"contrast":number(get(d,"Cntr")?,b"#Nne",if legacy {-100.0} else {-50.0},100.0)?,"legacy":legacy}),
            ))
        }
        "convertMode" | "CnvM" => {
            keys(d, &["T   "])?;
            let D::Class(c) = get(d, "T   ")? else { return Err("only an explicit color-mode class is supported".into()) };
            let id = match c.class_id.as_bytes() {
                b"RGBM" => "image.mode.rgb",
                b"Grys" => "image.mode.grayscale",
                b"CMYM" => "image.mode.cmyk",
                b"LbCM" => "image.mode.lab",
                _ => return Err("unsupported color-mode class".into()),
            };
            Ok((id.into(), json!({})))
        }
        "set" | "setd" => {
            keys(d, &["null", "T   "])?;
            if !selection(get(d, "null")?) {
                return Err("only set selection All/None is supported".into());
            }
            let v = get(d, "T   ")?;
            let id = if enum_is(v, "Ordn", "Al  ") {
                "select.all"
            } else if enum_is(v, "Ordn", "None") {
                "select.deselect"
            } else {
                return Err("unsupported selection source".into());
            };
            Ok((id.into(), json!({})))
        }
        "make" | "Mk  " => {
            keys(d, &["null", "Usng"])?;
            let D::Reference(r) = get(d, "null")? else {
                return Err("expected layer class reference".into());
            };
            if !matches!(r.as_slice(),[Ref::Class(c)] if c.class_id.is("Lyr ")) {
                return Err("only a new raster layer is supported".into());
            }
            let mut p = json!({});
            if let Some(v) = d.and_then(|d| d.get("Usng")) {
                let D::Descriptor(using) = v else {
                    return Err("unsupported new-layer settings".into());
                };
                if !using.class_id.is("Lyr ") {
                    return Err("unsupported new-layer class".into());
                }
                keys(Some(using), &["Nm  "])?;
                if let Some(name) = using.get("Nm  ") {
                    p["name"] = json!(text(name)?);
                }
            }
            Ok(("layer.new.layer".into(), p))
        }
        "duplicate" | "Dplc" | "delete" | "Dlt " => {
            keys(d, &["null"])?;
            if !current_layer(get(d, "null")?) {
                return Err("only the current layer target is supported".into());
            }
            Ok((if matches!(event.as_ref(), "duplicate" | "Dplc") { "layer.duplicate" } else { "layer.delete" }.into(), json!({})))
        }
        "stop" | "Stop" => {
            keys(d, &["Msge", "Cntn"])?;
            let message = text(get(d, "Msge")?)?;
            if message.len() > 4096 {
                return Err("stop message exceeds 4096 bytes".into());
            }
            let allow = d.and_then(|d| d.get("Cntn")).map(boolean).transpose()?.unwrap_or(false);
            Ok(("actions.message".into(), json!({"message":message,"allowContinue":allow})))
        }
        "play" | "Ply " => {
            keys(d, &["null"])?;
            let D::Reference(r) = get(d, "null")? else {
                return Err("expected named action reference".into());
            };
            let [Ref::Name { class: ac, name: an }, Ref::Name { class: sc, name: sn }] = r.as_slice() else {
                return Err("only named action/set references are supported".into());
            };
            if !ac.class_id.is("Actn") || !sc.class_id.is("ASet") {
                return Err("unsupported action reference classes".into());
            }
            let key = (crate::atn_import_cmds::display_name(&sn.to_string_lossy()), crate::atn_import_cmds::display_name(&an.to_string_lossy()));
            let name = names.get(&key).ok_or("called action is not in this imported file")?;
            Ok(("actions.play".into(), json!({"action":name})))
        }
        "fill" | "Fl  " => fill(d),
        _ => Err(format!("event `{event}` has no verified Prism mapping")),
    }
}
fn fill(d: Option<&Descriptor>) -> Result<(String, Value)> {
    keys(d, &["Usng", "Opct", "Md  ", "PrsT", "Clr "])?;
    let v = get(d, "Usng")?;
    let mut p = json!({});
    let contents = [("FrgC", "foreground"), ("BckC", "background"), ("Blck", "black"), ("Wht ", "white"), ("Gry ", "gray"), ("Clr ", "color")]
        .iter()
        .find_map(|(code, kind)| enum_is(v, "FlCn", code).then_some(*kind))
        .ok_or("unsupported fill contents")?;
    p["contents"] = json!(contents);
    if let Some(opacity) = d.and_then(|d| d.get("Opct")) {
        p["opacity"] = json!(number(opacity, b"#Prc", 0.0, 100.0)?);
    }
    if let Some(mode) = d.and_then(|d| d.get("Md  ")) {
        let mode = [("Nrml", "normal"), ("Mltp", "multiply"), ("Scrn", "screen"), ("Ovrl", "overlay"), ("Drkn", "darken"), ("Lghn", "lighten")]
            .iter()
            .find_map(|(code, name)| enum_is(mode, "BlnM", code).then_some(*name))
            .ok_or("unsupported fill blend mode")?;
        p["mode"] = json!(mode);
    }
    if let Some(preserve) = d.and_then(|d| d.get("PrsT")) {
        p["preserveTransparency"] = json!(boolean(preserve)?);
    }
    if contents == "color" {
        let D::Descriptor(color) = get(d, "Clr ")? else {
            return Err("expected RGB color descriptor".into());
        };
        if !color.class_id.is("RGBC") {
            return Err("only explicit RGB fill colors are supported".into());
        }
        keys(Some(color), &["Rd  ", "Grn ", "Bl  "])?;
        p["color"] = json!([
            number(get(Some(color), "Rd  ")?, b"#Nne", 0.0, 255.0)? / 255.0,
            number(get(Some(color), "Grn ")?, b"#Nne", 0.0, 255.0)? / 255.0,
            number(get(Some(color), "Bl  ")?, b"#Nne", 0.0, 255.0)? / 255.0,
            1.0
        ]);
    } else if d.and_then(|d| d.get("Clr ")).is_some() {
        return Err("unexpected color for non-color fill".into());
    }
    Ok(("edit.fill".into(), p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_io::atn::{Class, Id, UnicodeString};

    fn id(value: &str) -> Id {
        Id::Str(value.as_bytes().to_vec())
    }
    fn text_value(value: &str) -> D {
        D::Text(UnicodeString(value.encode_utf16().collect()))
    }
    fn class(value: &str) -> Class {
        Class { name: UnicodeString(vec![]), class_id: id(value) }
    }
    fn descriptor(class: &str, items: Vec<(&str, D)>) -> Descriptor {
        Descriptor { name: UnicodeString(vec![]), class_id: id(class), items: items.into_iter().map(|(key, value)| (id(key), value)).collect() }
    }
    fn step(event: &str, items: Vec<(&str, D)>) -> Step {
        Step {
            expanded: false,
            enabled: true,
            dialog: false,
            dialog_options: 0,
            event: id(event),
            name: event.into(),
            name_bytes: event.as_bytes().to_vec(),
            descriptor: Some(descriptor("null", items)),
            descriptor_bytes: vec![],
        }
    }
    fn unit(unit: &[u8; 4], value: f64) -> D {
        D::UnitFloat { unit: *unit, value }
    }
    fn enumerated(kind: &str, value: &str) -> D {
        D::Enumerated { type_id: id(kind), value: id(value) }
    }
    fn check(event: &str, items: Vec<(&str, D)>, expected: &str, params: Value) {
        let (command, mapped) = map(&step(event, items), &HashMap::new()).unwrap();
        assert_eq!(command, expected, "{event}");
        assert!(crate::commands::find(&command).is_some(), "mapped command is registered: {command}");
        assert_eq!(mapped, params, "{event}");
    }
    fn selection_reference() -> D {
        D::Reference(vec![Ref::Property { class: class("Chnl"), key: id("fsel") }])
    }
    fn current_layer_reference() -> D {
        D::Reference(vec![Ref::Enumerated { class: class("Lyr "), type_id: id("Ordn"), value: id("Trgt") }])
    }

    #[test]
    fn simple_events_map_to_registered_commands_without_dropping_parameters() {
        for (events, command) in [
            (["invert", "Invr"], "image.adjustments.invert"),
            (["desaturate", "Dstt"], "image.adjustments.desaturate"),
            (["copyEvent", "copy"], "edit.copy"),
            (["copyMerged", "CpyM"], "edit.copyMerged"),
            (["paste", "past"], "edit.paste"),
            (["cut", "cut "], "edit.cut"),
            (["clearEvent", "Cler"], "edit.clear"),
            (["inverse", "Invs"], "select.inverse"),
            (["flattenImage", "FltI"], "layer.flattenImage"),
            (["mergeVisible", "MrgV"], "layer.mergeVisible"),
            (["findEdges", "FndE"], "filter.stylize.findEdges"),
            (["solarize", "Slrz"], "filter.stylize.solarize"),
        ] {
            for event in events {
                check(event, vec![], command, json!({}));
                assert!(map(&step(event, vec![("unknown", D::Boolean(true))]), &HashMap::new()).is_err());
            }
        }
        let mut no_descriptor = step("Invr", vec![]);
        no_descriptor.event = Id::Code(*b"Invr");
        no_descriptor.descriptor = None;
        assert_eq!(map(&no_descriptor, &HashMap::new()).unwrap().0, "image.adjustments.invert");
        assert!(map(&step("notACommand", vec![]), &HashMap::new()).is_err());
    }

    #[test]
    fn numeric_filter_parameters_keep_recorded_units_and_ranges() {
        for (event, command) in
            [("GsnB", "filter.blur.gaussianBlur"), ("HghP", "filter.other.highPass"), ("Mdn ", "filter.noise.median"), ("Fthr", "select.modify.feather")]
        {
            check(event, vec![("Rds ", unit(b"#Pxl", 3.5))], command, json!({"radius":3.5}));
            for bad in [unit(b"#Prc", 3.5), D::Double(f64::NAN), D::Double(f64::INFINITY), D::Double(-1.0), D::Text(UnicodeString(vec![]))] {
                assert!(map(&step(event, vec![("Rds ", bad)]), &HashMap::new()).is_err(), "{event}");
            }
        }
        assert!(map(&step("Fthr", vec![("Rds ", D::Double(0.0))]), &HashMap::new()).is_err());
        assert!(map(&step("GsnB", vec![("Rds ", D::Double(1001.0))]), &HashMap::new()).is_err());
        assert!(map(&step("GsnB", vec![]), &HashMap::new()).is_err());
        assert!(map(&step("GsnB", vec![("Rds ", D::Integer(2)), ("Rds ", D::Integer(3))]), &HashMap::new()).is_err());
        check("MtnB", vec![("Angl", unit(b"#Ang", -45.0)), ("Dstn", unit(b"#Pxl", 8.0))], "filter.blur.motionBlur", json!({"angle":-45.0,"distance":8.0}));
        check(
            "UnsM",
            vec![("Amnt", unit(b"#Prc", 150.0)), ("Rds ", unit(b"#Pxl", 2.0)), ("Thsh", D::Integer(5))],
            "filter.sharpen.unsharpMask",
            json!({"amount":150.0,"radius":2.0,"threshold":5.0}),
        );
        check("Thrs", vec![("Lvl ", D::Integer(128))], "image.adjustments.threshold", json!({"level":128.0}));
    }

    #[test]
    fn brightness_modes_do_not_silently_clamp_modern_contrast() {
        check(
            "BrgC",
            vec![("Brgh", D::Integer(50)), ("Cntr", D::Integer(-90))],
            "image.adjustments.brightnessContrast",
            json!({"brightness":50.0,"contrast":-90.0,"legacy":true}),
        );
        check(
            "brightnessEvent",
            vec![("Brgh", D::Integer(150)), ("Cntr", D::Integer(-50)), ("useLegacy", D::Boolean(false))],
            "image.adjustments.brightnessContrast",
            json!({"brightness":150.0,"contrast":-50.0,"legacy":false}),
        );
        assert!(map(&step("BrgC", vec![("Brgh", D::Integer(0)), ("Cntr", D::Integer(-51)), ("useLegacy", D::Boolean(false))]), &HashMap::new()).is_err());
        assert!(map(&step("BrgC", vec![("Brgh", D::Integer(0)), ("Cntr", D::Integer(0)), ("useLegacy", D::Integer(1))]), &HashMap::new()).is_err());
    }

    #[test]
    fn fill_maps_percent_opacity_rgb_channels_and_preserve_transparency() {
        let rgb = || D::Descriptor(descriptor("RGBC", vec![("Rd  ", D::Double(255.0)), ("Grn ", D::Double(127.5)), ("Bl  ", D::Double(0.0))]));
        check(
            "Fl  ",
            vec![
                ("Usng", enumerated("FlCn", "Clr ")),
                ("Clr ", rgb()),
                ("Opct", unit(b"#Prc", 25.0)),
                ("Md  ", enumerated("BlnM", "Mltp")),
                ("PrsT", D::Boolean(true)),
            ],
            "edit.fill",
            json!({"contents":"color","color":[1.0,0.5,0.0,1.0],"opacity":25.0,"mode":"multiply","preserveTransparency":true}),
        );
        for (code, contents) in [("FrgC", "foreground"), ("BckC", "background"), ("Blck", "black"), ("Wht ", "white"), ("Gry ", "gray")] {
            check("fill", vec![("Usng", enumerated("FlCn", code))], "edit.fill", json!({"contents":contents}));
        }
        assert!(map(&step("Fl  ", vec![("Usng", enumerated("FlCn", "FrgC")), ("Clr ", rgb())]), &HashMap::new()).is_err());
        assert!(map(&step("Fl  ", vec![("Usng", enumerated("FlCn", "Clr ")), ("Clr ", D::Descriptor(descriptor("CMYC", vec![])))]), &HashMap::new()).is_err());
        assert!(map(&step("Fl  ", vec![("Usng", enumerated("FlCn", "FrgC")), ("Opct", unit(b"#Pxl", 25.0))]), &HashMap::new()).is_err());
        assert!(map(&step("Fl  ", vec![("Usng", enumerated("FlCn", "FrgC")), ("Md  ", enumerated("BlnM", "unverified"))]), &HashMap::new()).is_err());
    }

    #[test]
    fn layer_selection_and_mode_references_are_explicit_and_conservative() {
        for (code, command) in [("RGBM", "image.mode.rgb"), ("Grys", "image.mode.grayscale"), ("CMYM", "image.mode.cmyk"), ("LbCM", "image.mode.lab")] {
            check("CnvM", vec![("T   ", D::Class(class(code)))], command, json!({}));
        }
        assert!(map(&step("CnvM", vec![("T   ", D::Class(class("RGBM"))), ("Mrge", D::Boolean(true))]), &HashMap::new()).is_err());
        assert!(map(&step("CnvM", vec![("T   ", D::Class(class("BtmM")))]), &HashMap::new()).is_err());
        for (value, command) in [("Al  ", "select.all"), ("None", "select.deselect")] {
            check("setd", vec![("null", selection_reference()), ("T   ", enumerated("Ordn", value))], command, json!({}));
        }
        assert!(map(&step("setd", vec![("null", current_layer_reference()), ("T   ", enumerated("Ordn", "None"))]), &HashMap::new()).is_err());
        check(
            "Mk  ",
            vec![("null", D::Reference(vec![Ref::Class(class("Lyr "))])), ("Usng", D::Descriptor(descriptor("Lyr ", vec![("Nm  ", text_value("Raster"))])))],
            "layer.new.layer",
            json!({"name":"Raster"}),
        );
        assert!(map(&step("Mk  ", vec![("null", D::Reference(vec![Ref::Class(class("Dcmn"))]))]), &HashMap::new()).is_err());
        for (event, command) in [("Dplc", "layer.duplicate"), ("Dlt ", "layer.delete")] {
            check(event, vec![("null", current_layer_reference())], command, json!({}));
            assert!(map(&step(event, vec![("null", D::Reference(vec![Ref::Index { class: class("Lyr "), value: 2 }]))]), &HashMap::new()).is_err());
        }
    }

    #[test]
    fn nested_calls_resolve_import_names_and_stops_preserve_continue_policy() {
        check(
            "Stop",
            vec![("Msge", text_value("Inspect result")), ("Cntn", D::Boolean(true))],
            "actions.message",
            json!({"message":"Inspect result","allowContinue":true}),
        );
        let target = || {
            D::Reference(vec![
                Ref::Name { class: class("Actn"), name: UnicodeString("Child".encode_utf16().collect()) },
                Ref::Name { class: class("ASet"), name: UnicodeString("Set".encode_utf16().collect()) },
            ])
        };
        let names = HashMap::from([(("Set".into(), "Child".into()), "Set / Child".into())]);
        let (command, params) = map(&step("Ply ", vec![("null", target())]), &names).unwrap();
        assert_eq!(command, "actions.play");
        assert!(crate::commands::find(&command).is_some());
        assert_eq!(params, json!({"action":"Set / Child"}));
        assert!(map(&step("Ply ", vec![("null", target())]), &HashMap::new()).is_err(), "unresolved external actions are unsupported");
        assert!(map(&step("Ply ", vec![("null", target()), ("Cntn", D::Boolean(true))]), &names).is_err());
    }
}
