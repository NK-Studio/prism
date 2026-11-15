//! CLIP STUDIO gradient materials into the Gradients panel: a downloaded gradient set
//! (`.colorset`) or gradient sub-tool (`.tool`), by material id or from a file. Reading lives in
//! `prism_io::sut::gradient_material`; this maps CLIP STUDIO's stops onto Prism's presets.
use crate::presets::gradients::{GradientPreset, StopColor};
use crate::presets::Group;
use crate::{EngineError, Result, Session, commands::CommandSpec};
use prism_io::sut::gradient_material::{self, GradientMaterial};
use prism_paint::csp_gradient::{CspGradient, CspStopColor};
use serde_json::{Value, json};

const ID: &str = "gradient.presets.importCsp";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

/// A CLIP STUDIO gradient as a Prism preset, and whether a curve between a drawing colour and
/// another colour had to run straight.
fn preset(name: &str, g: &CspGradient) -> (GradientPreset, bool) {
    let p = g.prism_stops();
    let mut stops: Vec<(f32, StopColor)> = p
        .colors
        .iter()
        .map(|(t, c)| {
            let c = match c {
                CspStopColor::Foreground => StopColor::Foreground,
                CspStopColor::Background => StopColor::Background,
                CspStopColor::Rgb(c) => StopColor::Rgb(*c),
            };
            (*t, c)
        })
        .collect();
    // Two stops of one colour at one place prune to one; a preset needs both ends.
    if let [only] = stops.as_slice() {
        stops = vec![(0.0, only.1), (1.0, only.1)];
    }
    let opacity = if p.opacity.iter().all(|(_, a)| *a >= 1.0) { Vec::new() } else { p.opacity };
    (GradientPreset { name: name.to_string(), stops, opacity }, p.approximated)
}

fn text_param<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(Some).ok_or_else(|| bad(format!("`{key}` must be a non-empty string"))),
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let group_param = text_param(p, "group")?;
    let ctl = prism_raster::Interrupt::NONE;
    // The material, and the group a sub-tool's gradients go to without `group`.
    let (material, fallback): (GradientMaterial, String) = match text_param(p, "uuid")? {
        Some(uuid) => {
            let root = crate::csp_library_cmds::root(ID, p)?;
            let (_, catalog, m) = prism_io::sut::library::import_gradients(&root, uuid, &ctl).map_err(EngineError::Other)?;
            (m, if catalog.trim().is_empty() { "CLIP STUDIO".into() } else { catalog })
        }
        None => {
            if p.get("root").is_some() {
                return Err(bad("`root` goes with `uuid`"));
            }
            let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, ID, "CLIP STUDIO gradient file (.colorset or .tool)")?;
            let m = gradient_material::read(&bytes, &ctl).map_err(bad)?;
            (m, if stem.is_empty() { "CLIP STUDIO".into() } else { stem })
        }
    };
    let mut warnings = material.warnings.clone();
    let mut approximated = 0;
    let mut groups: Vec<(String, Vec<GradientPreset>)> = Vec::new();
    for g in &material.groups {
        let name = group_param.map(str::to_string).or_else(|| g.name.clone()).unwrap_or_else(|| fallback.clone());
        let items: Vec<GradientPreset> = g
            .gradients
            .iter()
            .map(|(n, cg)| {
                let (preset, approx) = preset(n, cg);
                approximated += usize::from(approx);
                preset
            })
            .collect();
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, list)) => list.extend(items),
            None => groups.push((name, items)),
        }
    }
    if approximated > 0 {
        warnings.push(format!("{approximated} gradients curve between a drawing colour and another colour; those segments run straight"));
    }
    // Curves between fixed colours become sampled stops, which match them: nothing to report.
    // Merge: a gradient replaces a same-named one in its group, so importing again updates it.
    let mut imported = Vec::new();
    for (name, items) in groups {
        let index = match s.presets.gradients.iter().position(|g| g.name == name) {
            Some(i) => i,
            None => {
                s.presets.gradients.push(Group::new(&name, Vec::new()));
                s.presets.gradients.len() - 1
            }
        };
        let Some(group) = s.presets.gradients.get_mut(index) else { continue };
        for item in items {
            imported.push(json!({"group": name, "name": item.name}));
            match group.items.iter_mut().find(|g| g.name == item.name) {
                Some(old) => *old = item,
                None => group.items.push(item),
            }
        }
    }
    s.presets_changed();
    Ok(json!({"imported": imported, "count": imported.len(), "warnings": warnings}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Import CLIP STUDIO Gradients…",
        menu: &[],
        shortcut: None,
        params: r#"{"uuid":gradient id from brush.cspLibrary.list gradients?,"root":CLIP STUDIO user data folder? (with uuid),"path":".colorset or .tool file"?,"data":base64 bytes?,"group":string?=the set's name, else the collection or file name} → {imported:[{group,name}], count, warnings} (a same-named gradient in the group is replaced)"#,
        enabled: |_| Ok(()),
        run: import,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gradient blob: 16-byte header, then (rgb, flags, position) stops, straight curves.
    fn blob(stops: &[([u32; 3], u32, u32)]) -> Vec<u8> {
        let mut b: Vec<u8> = [16u32, 28, stops.len() as u32, 16].into_iter().flat_map(u32::to_be_bytes).collect();
        for (rgb, flags, pos) in stops {
            b.extend([rgb[0], rgb[1], rgb[2], u32::MAX, *flags, *pos, 0].into_iter().flat_map(u32::to_be_bytes));
        }
        b
    }
    fn string(s: &str) -> Vec<u8> {
        [vec![1, s.len() as u8], s.as_bytes().to_vec()].concat()
    }
    /// Fields of a typed value record: a count, then key and value pairs.
    fn fields(f: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![1, f.len() as u8];
        for (k, v) in f {
            out.extend(string(k));
            out.extend(v);
        }
        out
    }
    fn text(s: &str) -> Vec<u8> {
        [vec![6], string(s)].concat()
    }
    /// An object's body: the `typename` header, then its fields.
    fn body(f: &[(&str, Vec<u8>)]) -> Vec<u8> {
        [fields(&[("typename", text(""))]), fields(f)].concat()
    }
    fn wrap(tag: u8, b: Vec<u8>) -> Vec<u8> {
        [vec![tag], (b.len() as u32).to_be_bytes().to_vec(), b].concat()
    }
    fn array(values: Vec<Vec<u8>>) -> Vec<u8> {
        wrap(72, [fields(&[("typename", text(""))]), vec![1, values.len() as u8], values.concat()].concat())
    }
    /// A `.colorset` with one set of the given gradients.
    fn colorset(set: &str, items: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let items = items
            .iter()
            .map(|(n, b)| wrap(73, body(&[("gradationData", [vec![7, 2], (b.len() as u16).to_be_bytes().to_vec(), b.clone()].concat()), ("gradationName", text(n))])))
            .collect();
        let set = wrap(73, body(&[("gradationItemArray", array(items)), ("gradationSetName", text(set))]));
        body(&[("gradationSetArray", array(vec![set]))])
    }

    #[test]
    fn a_gradient_set_becomes_a_preset_group_and_reimports_in_place() {
        let red_white = blob(&[([u32::MAX, 0, 0], 0, 0), ([u32::MAX; 3], 0, 0x8000)]);
        let fg_bg = blob(&[([0; 3], 1, 0), ([0; 3], 2, 0x8000)]);
        let file = colorset("Inks", &[("Red", red_white.clone()), ("Live", fg_bg)]);
        let data = prism_paint::tile::b64_encode(&file);
        let mut s = Session::new();
        let r = s.execute(ID, json!({"data": data})).unwrap();
        assert_eq!(r["count"], 2, "{r}");
        let group = s.presets.gradients.iter().find(|g| g.name == "Inks").unwrap();
        assert_eq!(group.items.len(), 2);
        assert_eq!(group.items[0].stops, vec![(0.0, StopColor::Rgb([1.0, 0.0, 0.0])), (1.0, StopColor::Rgb([1.0, 1.0, 1.0]))]);
        assert!(group.items[0].opacity.is_empty());
        assert_eq!(group.items[1].stops, vec![(0.0, StopColor::Foreground), (1.0, StopColor::Background)]);
        // Again, into a named group: same names replace, nothing doubles.
        s.execute(ID, json!({"data": data, "group": "Mine"})).unwrap();
        s.execute(ID, json!({"data": data, "group": "Mine"})).unwrap();
        assert_eq!(s.presets.gradients.iter().find(|g| g.name == "Mine").unwrap().items.len(), 2);
    }

    #[test]
    fn bad_sources_are_errors() {
        let enc = |b: &[u8]| prism_paint::tile::b64_encode(b);
        let mut s = Session::new();
        let before = s.presets.gradients.clone();
        let broken = colorset("Inks", &[("Bad", vec![0, 0, 0, 16])]);
        for p in [
            json!({}),
            json!({"uuid": ""}),
            json!({"uuid": 3}),
            json!({"uuid": "x", "root": "/nonexistent/prism-csp-root"}),
            json!({"data": enc(&broken)}),
            json!({"data": enc(b"not a gradient")}),
            json!({"data": "%%%"}),
            json!({"data": enc(&broken), "root": "/x"}),
            json!({"data": enc(&colorset("Inks", &[])), "group": ""}),
        ] {
            assert!(s.execute(ID, p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.presets.gradients, before);
    }

    #[test]
    #[ignore = "reads the local CLIP STUDIO material folder (PRISM_TEST_CSP_ROOT or the platform's)"]
    fn every_local_downloaded_gradient_imports() {
        let root = std::env::var_os("PRISM_TEST_CSP_ROOT").map(std::path::PathBuf::from).or_else(prism_io::sut::library::default_root).unwrap();
        let mut s = Session::new();
        let listed = s.execute("brush.cspLibrary.list", json!({"root": root.to_string_lossy()})).unwrap();
        for g in listed["gradients"].as_array().unwrap() {
            let r = s.execute(ID, json!({"uuid": g["uuid"], "root": root.to_string_lossy()}));
            eprintln!("{} {} [{}]: {}", g["kind"], g["name"], g["status"], r.as_ref().map_or_else(|e| format!("ERROR {e}"), |v| v.to_string()));
        }
    }
}
