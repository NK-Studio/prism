//! Commands CLIP STUDIO auto actions need that Photoshop has no menu item for: selecting a
//! layer by name, Convert brightness to opacity, a numeric (scale, rotate and move) transform
//! about a reference point and Convert Layer. Each is an ordinary command, usable from actions, MCP and the CLI.
use prism_algo::transform::{Homography, Interp};
use prism_doc::{Document, Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::transform_cmds::{affine_of, composite_over, group_locks, refresh_text, transform_bounds, transform_layer, warp_gray};
use crate::{EngineError, Result, Session};

const BY_NAME: &str = "layer.selectByName";
const BRIGHTNESS: &str = "edit.convertBrightnessToOpacity";
const NUMERIC: &str = "edit.transform.numeric";
const CONVERT: &str = "layer.convert";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|d| d.active_layer).map(|_| ()).ok_or_else(|| "no active layer".into())
}

/// The topmost layer named `name`, searching the whole tree top to bottom (inside collapsed
/// groups too). An exact match wins; otherwise the first match ignoring case.
pub(crate) fn find_by_name(doc: &Document, name: &str) -> Option<LayerId> {
    fn rec(layers: &[Layer], out: &mut Vec<(LayerId, String)>) {
        for l in layers.iter().rev() {
            out.push((l.id, l.name.clone()));
            if let Some(ch) = l.children() {
                rec(ch, out);
            }
        }
    }
    let mut all = Vec::new();
    rec(&doc.layers, &mut all);
    all.iter().find(|(_, n)| n == name).or_else(|| all.iter().find(|(_, n)| n.to_lowercase() == name.to_lowercase())).map(|(id, _)| *id)
}

/// `name` from the params: a non-empty string of at most 255 characters.
pub(crate) fn name_param(cmd: &str, p: &Value, key: &str) -> Result<String> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.chars().count() <= 255)
        .map(str::to_owned)
        .ok_or_else(|| bad(cmd, format!("`{key}` must be a layer name of 1–255 characters")))
}

fn select_by_name(s: &mut Session, p: &Value) -> Result<Value> {
    let name = name_param(BY_NAME, p, "name")?;
    let mode = match p.get("mode") {
        None => "replace",
        Some(v) => v.as_str().filter(|m| matches!(*m, "replace" | "add" | "range")).ok_or_else(|| bad(BY_NAME, "`mode` must be replace, add or range"))?,
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = find_by_name(&st.doc, &name).ok_or_else(|| bad(BY_NAME, format!("no layer named \"{name}\"")))?;
    crate::layer_multi_cmds::select(s, &json!({"layer": id.0, "mode": mode}))?;
    Ok(json!({"layer": id.0}))
}

/// Edit › Convert Brightness to Opacity (CLIP STUDIO): white becomes transparent and darker
/// pixels more opaque; the colour becomes black (CLIP STUDIO's monochrome output). Alpha is the
/// pixel's alpha times one minus its Rec. 601 luma, inside the selection when there is one.
fn brightness_to_opacity(s: &mut Session, p: &Value) -> Result<Value> {
    crate::extra_cmds::map_pixels(s, "Convert Brightness to Opacity", p, |_, px, _| {
        for v in px.iter_mut() {
            let luma = (0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2]).clamp(0.0, 1.0);
            *v = [0.0, 0.0, 0.0, v[3] * (1.0 - luma)];
        }
    })
}

/// The point of `rect` a reference name stands for (CLIP STUDIO's and Photoshop's 3×3 grid).
fn reference_point(rect: [f64; 4], name: &str) -> Option<[f64; 2]> {
    let [x0, y0, x1, y1] = rect;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    Some(match name {
        "topLeft" => [x0, y0],
        "top" => [cx, y0],
        "topRight" => [x1, y0],
        "left" => [x0, cy],
        "center" => [cx, cy],
        "right" => [x1, cy],
        "bottomLeft" => [x0, y1],
        "bottom" => [cx, y1],
        "bottomRight" => [x1, y1],
        _ => return None,
    })
}

fn finite(p: &Value, key: &str, default: f64, range: std::ops::RangeInclusive<f64>) -> Result<f64> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => v.as_f64().filter(|v| v.is_finite() && range.contains(v)).ok_or_else(|| bad(NUMERIC, format!("`{key}` must be a number in {}..{}", range.start(), range.end()))),
    }
}

/// Scale by `scaleX`/`scaleY` % along the frame's own axes, rotate by `angle` degrees (positive is
/// clockwise on screen) about a reference point of the frame, then move by `dx`/`dy`, as one Free
/// Transform step. CLIP STUDIO applies a step's scale before its rotation, observed in its
/// Transform tool property (2026-11-15). `keepOriginal` leaves the source pixels in place under the
/// transformed copy (CLIP STUDIO's Keep original image).
fn numeric_transform(s: &mut Session, p: &Value) -> Result<Value> {
    let sx = finite(p, "scaleX", 100.0, 0.01..=10_000.0)? / 100.0;
    let sy = finite(p, "scaleY", 100.0, 0.01..=10_000.0)? / 100.0;
    let dx = finite(p, "dx", 0.0, -1e6..=1e6)?;
    let dy = finite(p, "dy", 0.0, -1e6..=1e6)?;
    let (sin, cos) = finite(p, "angle", 0.0, -360.0..=360.0)?.to_radians().sin_cos();
    let reference = match p.get("reference") {
        None => "center",
        Some(v) => v.as_str().ok_or_else(|| bad(NUMERIC, "`reference` must be a string"))?,
    };
    let interp = match p.get("interpolation") {
        None => Interp::Bicubic,
        Some(v) => match v.as_str() {
            Some(name @ ("bicubic" | "bilinear" | "nearest")) => Interp::parse(name),
            _ => return Err(bad(NUMERIC, "`interpolation` must be bicubic, bilinear or nearest")),
        },
    };
    let keep = match p.get("keepOriginal") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(NUMERIC, "`keepOriginal` must be a boolean"))?,
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = crate::commands::layer_param(s, p)?;
    let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if keep && layer.surface().is_none() {
        return Err(bad(NUMERIC, "`keepOriginal` needs a pixel layer"));
    }
    let b = transform_bounds(&st.doc, layer);
    if b.is_empty() {
        return Err(EngineError::Other("nothing to transform".into()));
    }
    let rect = [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)];
    let [rx, ry] = reference_point(rect, reference).ok_or_else(|| bad(NUMERIC, "`reference` must be topLeft, top, …, center, …, bottomRight"))?;
    // Image y points down, so this rotation turns clockwise on screen for a positive angle.
    let map = |x: f64, y: f64| {
        let (u, v) = ((x - rx) * sx, (y - ry) * sy);
        [rx + u * cos - v * sin + dx, ry + u * sin + v * cos + dy]
    };
    let quad = [map(rect[0], rect[1]), map(rect[2], rect[1]), map(rect[2], rect[3]), map(rect[0], rect[3])];
    let h = Homography::rect_to_quad(rect, quad).ok_or_else(|| bad(NUMERIC, "the transform collapses the layer"))?;
    if h.inverse().is_none() {
        return Err(bad(NUMERIC, "the transform collapses the layer"));
    }
    let affine = affine_of(&h);
    s.edit("Free Transform", |doc, _| {
        let sel = doc.selection.clone();
        let is_group = doc.layer(id).is_some_and(Layer::is_group);
        let group = group_locks(doc, id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let original = if keep { l.surface().cloned() } else { None };
        transform_layer(if is_group { None } else { sel.as_ref() }, group, l, &h, affine, interp)?;
        if let Some(original) = original {
            let surf = l.surface_mut().ok_or_else(|| bad(NUMERIC, "`keepOriginal` needs a pixel layer"))?;
            let moved = std::mem::replace(surf, original);
            composite_over(surf, &moved);
            surf.prune();
        }
        let snapshot = doc.clone();
        if let Some(l) = doc.layer_mut(id) {
            refresh_text(&snapshot, l);
        }
        if let Some(sel) = &doc.selection {
            doc.selection = Some(warp_gray(sel, &h, Interp::Bilinear)).filter(|s| !s.content_bounds().is_empty());
        }
        Ok(())
    })?;
    Ok(json!({"rect": rect, "quad": quad}))
}

/// Layer › Convert Layer (CLIP STUDIO) for the kinds Prism has: `pixel` rasterizes, `smartObject`
/// wraps the layer (Prism's nearest to an image material layer). `keepOriginal` converts a copy
/// placed above and leaves the source. `name` and `blend` set the result's name and blending
/// mode. Everything is checked before the first change, and it is one history step.
fn convert_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = match p.get("kind").and_then(Value::as_str) {
        Some(k @ ("pixel" | "smartObject")) => k,
        _ => return Err(bad(CONVERT, "`kind` must be pixel or smartObject")),
    };
    let keep = match p.get("keepOriginal") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(CONVERT, "`keepOriginal` must be a boolean"))?,
    };
    let name = match p.get("name") {
        None => None,
        Some(v) => Some(v.as_str().filter(|n| !n.trim().is_empty() && n.chars().count() <= 255).ok_or_else(|| bad(CONVERT, "`name` must be a non-empty string"))?),
    };
    let blend = match p.get("blend") {
        None => None,
        Some(v) => Some(v.as_str().filter(|b| crate::commands::blend_from_str(b).is_some()).ok_or_else(|| bad(CONVERT, "`blend` must be a blending mode name"))?),
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let mut id = crate::commands::layer_param(s, p)?;
    let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let change = match (kind, &layer.content) {
        ("pixel", LayerContent::Raster(_)) | ("smartObject", LayerContent::Smart(_)) => false,
        ("pixel", LayerContent::Group(_) | LayerContent::Adjustment(_)) => {
            return Err(bad(CONVERT, format!("a {} layer cannot become a pixel layer", layer.content.kind_name().to_lowercase())));
        }
        _ => true,
    };
    let key = format!("{CONVERT}#{}", st.revision);
    let layer_of = |v: &Value| v.get("layer").and_then(Value::as_u64).map(LayerId);
    if keep {
        let r = s.execute("layer.duplicate", json!({"layer": id.0, "coalesce": key}))?;
        id = layer_of(&r).unwrap_or(id);
    }
    if change {
        let r = match kind {
            "pixel" => s.execute("layer.rasterize.layer", json!({"layer": id.0, "coalesce": key}))?,
            _ => s.execute("layer.smartObjects.convertToSmartObject", json!({"layer": id.0, "coalesce": key}))?,
        };
        id = layer_of(&r).unwrap_or(id);
    }
    if let Some(name) = name {
        s.execute("layer.renameLayer", json!({"layer": id.0, "name": name, "coalesce": key}))?;
    }
    if let Some(blend) = blend {
        s.execute("layer.setProps", json!({"layer": id.0, "blend": blend, "coalesce": key}))?;
    }
    Ok(json!({"layer": id.0}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: BY_NAME,
            label: "Select Layer by Name",
            menu: &[],
            shortcut: None,
            params: r#"{"name":str,"mode":"replace|add|range"="replace"} → {layer} (the topmost layer with that name, inside collapsed groups too; exact match first, then ignoring case)"#,
            enabled: has_doc,
            journal: true,
            run: select_by_name,
        },
        CommandSpec {
            id: BRIGHTNESS,
            label: "Convert Brightness to Opacity",
            menu: &[],
            shortcut: None,
            params: "{} (the active pixel layer, inside the selection: white becomes transparent, darker pixels more opaque, the colour black)",
            enabled: has_layer,
            journal: true,
            run: brightness_to_opacity,
        },
        CommandSpec {
            id: NUMERIC,
            label: "Numeric Transform",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?,"scaleX":%=100,"scaleY":%=100,"angle":deg=0,"dx":px=0,"dy":px=0,"reference":"topLeft|top|topRight|left|center|right|bottomLeft|bottom|bottomRight"="center","interpolation":"bicubic|bilinear|nearest"="bicubic","keepOriginal":bool=false} (scale along the frame's axes, rotate clockwise by `angle` about the reference point of the Free Transform frame, then move)"#,
            enabled: has_layer,
            journal: true,
            run: numeric_transform,
        },
        CommandSpec {
            id: CONVERT,
            label: "Convert Layer",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?,"kind":"pixel|smartObject","keepOriginal":bool=false,"name":str?,"blend":mode?} → {layer} (CLIP STUDIO's Convert Layer: rasterize, or wrap as a smart object; keepOriginal converts a copy above the source)"#,
            enabled: has_layer,
            journal: true,
            run: convert_layer,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 20, "background": "transparent"})).unwrap();
        s
    }
    fn active(s: &Session) -> LayerId {
        s.active().unwrap().active_layer.unwrap()
    }

    #[test]
    fn select_by_name_finds_the_topmost_match_inside_collapsed_groups() {
        let mut s = session();
        let id = |s: &mut Session, name: &str| LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
        let low = id(&mut s, "Ink");
        let inner = id(&mut s, "Tone");
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = active(&s);
        let d = s.active_mut().unwrap();
        if let Some(prism_doc::LayerContent::Group(g)) = std::sync::Arc::make_mut(&mut d.doc).layer_mut(group).map(|l| &mut l.content) {
            g.expanded = false;
        }
        let high = id(&mut s, "Ink");
        s.execute(BY_NAME, json!({"name": "Tone"})).unwrap();
        assert_eq!(active(&s), inner);
        s.execute(BY_NAME, json!({"name": "Ink"})).unwrap();
        assert_eq!(active(&s), high, "the topmost of two");
        s.execute(BY_NAME, json!({"name": "tone"})).unwrap();
        assert_eq!(active(&s), inner, "case-insensitive fallback");
        s.execute(BY_NAME, json!({"name": "Ink", "mode": "add"})).unwrap();
        assert_eq!(s.active().unwrap().selected_layers().len(), 2);
        assert_ne!(low, high);
        for p in [json!({}), json!({"name": ""}), json!({"name": 3}), json!({"name": "Missing"}), json!({"name": "Ink", "mode": "toggle"})] {
            assert!(s.execute(BY_NAME, p).is_err());
        }
    }

    #[test]
    fn brightness_becomes_black_opacity_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 4, "height": 1, "depth": depth, "background": "transparent"})).unwrap();
            let d = s.active_mut().unwrap();
            let id = d.active_layer.unwrap();
            let surf = std::sync::Arc::make_mut(&mut d.doc).layer_mut(id).unwrap().surface_mut().unwrap();
            surf.write_pixel(0, 0, &[1.0, 1.0, 1.0, 1.0]);
            surf.write_pixel(1, 0, &[0.0, 0.0, 0.0, 1.0]);
            surf.write_pixel(2, 0, &[0.5, 0.5, 0.5, 0.5]);
            surf.write_pixel(3, 0, &[1.0, 0.0, 0.0, 1.0]);
            s.execute(BRIGHTNESS, json!({})).unwrap();
            let surf = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
            let px = |x| surf.pixel(x, 0);
            assert!(px(0)[3] < 0.01, "{depth}: white is transparent");
            assert!((px(1)[3] - 1.0).abs() < 0.01 && px(1)[0] < 0.01, "{depth}: black stays opaque");
            assert!((px(2)[3] - 0.25).abs() < 0.01 && px(2)[0] < 0.01, "{depth}: gray at half alpha");
            assert!((px(3)[3] - 0.701).abs() < 0.01 && px(3)[0] < 0.01, "{depth}: red by luma, now black");
        }
        assert!(Session::new().execute(BRIGHTNESS, json!({})).is_err());
    }

    #[test]
    fn numeric_transform_scales_about_the_reference_and_moves() {
        let mut s = session();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 4, "y": 4, "width": 4, "height": 4})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let r = s.execute(NUMERIC, json!({"scaleX": 200, "scaleY": 50, "reference": "topLeft", "dx": 1, "dy": 2, "interpolation": "nearest"})).unwrap();
        assert_eq!(r["quad"], json!([[5.0, 6.0], [13.0, 6.0], [13.0, 8.0], [5.0, 8.0]]));
        let id = active(&s);
        let b = prism_compose::bounds::content_bounds(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap());
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (5, 6, 13, 8));
        // Keep original: a moved copy over the source.
        s.execute(NUMERIC, json!({"dx": 0, "dy": 4, "keepOriginal": true, "interpolation": "nearest"})).unwrap();
        let b = prism_compose::bounds::content_bounds(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap());
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (5, 6, 13, 12));
        assert!(s.undo());
        for p in [json!({"scaleX": 0}), json!({"scaleX": "x"}), json!({"angle": 400}), json!({"angle": "45"}), json!({"dx": 1e9}), json!({"reference": "middle"}), json!({"interpolation": "cubic"}), json!({"keepOriginal": 1})] {
            assert!(s.execute(NUMERIC, p).is_err());
        }
        let mut empty = session();
        assert!(empty.execute(NUMERIC, json!({})).is_err(), "nothing to transform");
    }

    #[test]
    fn new_layers_go_into_an_active_group_and_deselect_can_be_a_no_op() {
        let mut s = session();
        let group = LayerId(s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap());
        let inside = LayerId(s.execute("layer.new.layer", json!({"into": true})).unwrap()["layer"].as_u64().unwrap());
        let doc = &s.active().unwrap().doc;
        let LayerContent::Group(g) = &doc.layer(group).unwrap().content else { panic!() };
        assert_eq!(g.children.last().map(|l| l.id), Some(inside), "top of the group");
        // A pixel layer active: `into` changes nothing, the layer goes above it.
        let above = LayerId(s.execute("layer.new.layer", json!({"into": true})).unwrap()["layer"].as_u64().unwrap());
        let LayerContent::Group(g) = &s.active().unwrap().doc.layer(group).unwrap().content else { panic!() };
        assert_eq!(g.children.last().map(|l| l.id), Some(above));
        // Without `into` a new group goes above the active group, not inside.
        s.execute(BY_NAME, json!({"name": s.active().unwrap().doc.layer(group).unwrap().name.clone()})).unwrap();
        let sibling = LayerId(s.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap());
        assert!(s.active().unwrap().doc.layers.iter().any(|l| l.id == sibling));
        assert!(s.execute("layer.new.layer", json!({"into": 1})).is_err());

        assert!(s.execute("select.deselect", json!({})).is_err(), "unavailable without a selection");
        let revision = s.active().unwrap().revision;
        s.execute("select.deselect", json!({"ifAny": true})).unwrap();
        assert_eq!(s.active().unwrap().revision, revision, "nothing to do");
        s.execute("select.all", json!({})).unwrap();
        s.execute("select.deselect", json!({"ifAny": true})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
        assert!(Session::new().execute("select.deselect", json!({"ifAny": true})).is_err(), "no document");
    }

    #[test]
    fn convert_layer_rasterizes_or_wraps_in_one_step() {
        let mut s = session();
        s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
        s.execute("select.rect", json!({"x": 2, "y": 2, "width": 4, "height": 4})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let ink = active(&s);
        let count = |s: &Session| s.active().unwrap().doc.walk().len();
        let before = count(&s);
        // Keep the original: a converted copy above, renamed and blended, one undo step.
        let r = s.execute(CONVERT, json!({"kind": "pixel", "keepOriginal": true, "name": "Copy", "blend": "multiply"})).unwrap();
        let copy = LayerId(r["layer"].as_u64().unwrap());
        assert_ne!(copy, ink);
        let doc = &s.active().unwrap().doc;
        let l = doc.layer(copy).unwrap();
        assert_eq!((l.name.as_str(), l.blend), ("Copy", prism_color::BlendMode::Multiply));
        assert!(doc.layer(ink).is_some() && count(&s) == before + 1);
        assert!(s.undo());
        assert_eq!(count(&s), before, "one history step");
        // Smart object in place.
        let r = s.execute(CONVERT, json!({"kind": "smartObject"})).unwrap();
        let so = LayerId(r["layer"].as_u64().unwrap());
        assert!(matches!(s.active().unwrap().doc.layer(so).unwrap().content, LayerContent::Smart(_)));
        // Back to pixels.
        let r = s.execute(CONVERT, json!({"kind": "pixel"})).unwrap();
        assert!(matches!(s.active().unwrap().doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().content, LayerContent::Raster(_)));
        let revision = s.active().unwrap().revision;
        for p in [json!({}), json!({"kind": "vector"}), json!({"kind": "pixel", "keepOriginal": 1}), json!({"kind": "pixel", "name": ""}), json!({"kind": "pixel", "blend": "sparkle"}), json!({"kind": "pixel", "layer": 9999})] {
            assert!(s.execute(CONVERT, p).is_err());
        }
        s.execute("layer.groupLayers", json!({})).unwrap();
        let revision_group = s.active().unwrap().revision;
        assert!(s.execute(CONVERT, json!({"kind": "pixel", "keepOriginal": true})).is_err(), "a group is refused before the copy");
        assert_eq!(s.active().unwrap().revision, revision_group);
        assert!(revision < revision_group);
    }

    #[test]
    fn numeric_rotation_turns_clockwise_after_scaling() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40, "background": "transparent"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        // A tall bar: 4 wide, 20 high, centred on (20, 20).
        s.execute("select.rect", json!({"x": 18, "y": 10, "width": 4, "height": 20})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let r = s.execute(NUMERIC, json!({"angle": 90, "interpolation": "nearest"})).unwrap();
        // Clockwise on screen: the top-left corner swings to the top right.
        let q = r["quad"].as_array().unwrap();
        let (x, y) = (q[0][0].as_f64().unwrap(), q[0][1].as_f64().unwrap());
        assert!((x - 30.0).abs() < 1e-9 && (y - 18.0).abs() < 1e-9, "{q:?}");
        let id = active(&s);
        let b = prism_compose::bounds::content_bounds(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap());
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (10, 18, 30, 22), "lying flat");
        assert!(s.undo());
        // Scale first, along the bar's own axes: halving the height then turning 90° gives a
        // 10-wide bar, not a 2-wide one.
        s.execute(NUMERIC, json!({"scaleY": 50, "angle": -90, "interpolation": "nearest"})).unwrap();
        let b = prism_compose::bounds::content_bounds(s.active().unwrap().doc.layer(id).unwrap().surface().unwrap());
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (15, 18, 25, 22));
    }
}
