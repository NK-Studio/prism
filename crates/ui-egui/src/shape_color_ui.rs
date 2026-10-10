//! A shape thumbnail opens the native solid-colour picker. Pending colours use a cached
//! document clone; only OK dispatches `shape.edit`, so Cancel never changes history.

use std::sync::Arc;

use photocraft_doc::{Color, Document, Fill, LayerContent, LayerId};
use photocraft_geom::Rect;
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, color_picker_ui};

const TAG: u64 = 1 << 56;

struct Edit {
    dialog: u64,
    document: Arc<Document>,
    revision: u64,
    layer: LayerId,
    original: [f32; 3],
    alpha: f32,
}

struct Preview {
    document: u64,
    revision: u64,
    dialog: u64,
    rgb: [f32; 3],
    key: u64,
    shown: Arc<Document>,
    area: Rect,
}

#[derive(Default)]
pub(crate) struct Runtime {
    edit: Option<Edit>,
    preview: Option<Preview>,
    serial: u64,
}

pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    let current = app.session.active().map(|st| (st.doc.id.0, st.revision));
    let runtime = &mut app.shape_color_runtime;
    if runtime.edit.as_ref().is_some_and(|edit| current != Some((edit.document.id.0, edit.revision)) || !app.ui.dialogs.iter().any(|d| d.id == edit.dialog)) {
        runtime.edit = None;
    }
    // Retain the last footprint on Cancel for the canvas's bounded restoration, but release
    // document pixels as soon as their document/revision is no longer shown.
    if runtime.preview.as_ref().is_some_and(|p| current != Some((p.document, p.revision))) {
        runtime.preview = None;
    }
}

pub(crate) fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__colorPicker").and_then(Value::as_str) == Some("shapeFill")
}

/// Gradients and patterns retain their existing Appearance controls.
pub(crate) fn solid_shape(app: &PhotocraftApp) -> bool {
    app.session
        .active()
        .and_then(|st| st.doc.layer(st.active_layer?))
        .is_some_and(|l| matches!(&l.content, LayerContent::Shape(sh) if matches!(sh.fill, None | Some(Fill::Solid(_)))))
}

fn editable(doc: &Document, layer: LayerId) -> Result<(), String> {
    let path = doc.path_of(layer).ok_or("no shape layer")?;
    let locks = doc.locks_at(&path);
    if locks.all || locks.pixels {
        return Err("the shape layer is locked".into());
    }
    match &doc.layer_at(&path).ok_or("no shape layer")?.content {
        LayerContent::Shape(sh) if matches!(sh.fill, None | Some(Fill::Solid(_))) => Ok(()),
        _ => Err("select a shape with a solid fill".into()),
    }
}

pub(crate) fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no active layer")?;
    editable(&st.doc, layer)?;
    let LayerContent::Shape(sh) = &st.doc.layer(layer).ok_or("no shape layer")?.content else { return Err("no shape layer".into()) };
    let fg = app.session.tools.foreground;
    let color = match sh.fill {
        Some(Fill::Solid(c)) => c,
        _ => Color::rgba(fg[0], fg[1], fg[2], fg[3]),
    };
    let rgb = color.to_rgb();
    if !rgb.iter().chain([color.alpha].iter()).all(|v| v.is_finite()) || !(0.0..=1.0).contains(&color.alpha) {
        return Err("the shape has an invalid fill colour".into());
    }
    let (document, revision) = (st.doc.clone(), st.revision);
    if let Some(previous) = app.shape_color_runtime.edit.take() {
        app.ui.close_dialog(previous.dialog);
    }
    let dialog = color_picker_ui::open_for_command(app, "Color Picker (Solid Color)", rgb, "shape.edit", json!({"layer": layer.0}));
    if let Some(d) = app.ui.dialog_mut(dialog) {
        d.fields.insert("__colorPicker".into(), json!("shapeFill"));
        d.fields.insert("__shapeDialog".into(), json!(dialog));
        d.fields.insert("__document".into(), json!(document.id.0));
        d.fields.insert("__revision".into(), json!(revision));
    }
    // Compare against the displayed colour, preserving exact original float/model/alpha on
    // an unchanged OK, including HDR values that the picker's SDR swatch cannot represent.
    let original = color_picker_ui::parse_hex(&color_picker_ui::hex(rgb)).ok_or("invalid picker colour")?;
    app.shape_color_runtime.edit = Some(Edit { dialog, document, revision, layer, original, alpha: color.alpha });
    Ok(dialog)
}

fn validate<'a>(app: &'a PhotocraftApp, fields: &Map<String, Value>) -> Result<(&'a Edit, [f32; 3]), String> {
    let edit = app.shape_color_runtime.edit.as_ref().ok_or("reopen the shape colour picker")?;
    if !owns(fields) || fields.get("__shapeDialog").and_then(Value::as_u64) != Some(edit.dialog) {
        return Err("reopen the shape colour picker".into());
    }
    let st = app.session.active().ok_or("the shape's document is no longer active")?;
    if st.revision != edit.revision || !Arc::ptr_eq(&st.doc, &edit.document) {
        return Err("the document changed; reopen the shape colour picker".into());
    }
    editable(&st.doc, edit.layer)?;
    let rgb = fields.get("color").and_then(Value::as_str).and_then(color_picker_ui::parse_hex).ok_or("enter a six-digit hex colour")?;
    Ok((edit, rgb))
}

/// Also used by the control channel's authorization check, with the actual command params.
pub(crate) fn params(app: &PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let (edit, rgb) = validate(app, fields)?;
    Ok(json!({"layer": edit.layer.0, "fill": [rgb[0], rgb[1], rgb[2], edit.alpha]}))
}

pub(crate) fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let result = (|| {
        let (edit, rgb) = validate(app, fields)?;
        if rgb == edit.original {
            return Ok(json!({"layer": edit.layer.0, "unchanged": true}));
        }
        let p = params(app, fields)?;
        app.run("shape.edit", p)
    })();
    if let Err(error) = &result {
        app.ui.status = error.clone();
        app.ui.status_error = true;
    }
    result
}

pub(crate) fn display_doc(app: &mut PhotocraftApp, index: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(index) {
        return None;
    }
    let edit = app.shape_color_runtime.edit.as_ref()?;
    let fields = &app.ui.dialogs.iter().find(|d| d.id == edit.dialog)?.fields;
    let (edit, rgb) = validate(app, fields).ok()?;
    if rgb == edit.original {
        return None;
    }
    if let Some(p) = &app.shape_color_runtime.preview
        && (p.dialog, p.rgb) == (edit.dialog, rgb)
    {
        return Some((p.shown.clone(), p.key));
    }
    let mut shown = (*edit.document).clone();
    let LayerContent::Shape(mut shape) = shown.layer(edit.layer)?.content.clone() else { return None };
    shape.fill = Some(Fill::Solid(Color::rgba(rgb[0], rgb[1], rgb[2], edit.alpha)));
    photocraft_engine::vector_cmds::refresh_shape(&shown, &mut shape);
    shown.layer_mut(edit.layer)?.content = LayerContent::Shape(shape);
    let before = photocraft_compose::change_bounds(edit.document.layer(edit.layer)?, shown.bounds()).unwrap_or(shown.bounds());
    let after = photocraft_compose::change_bounds(shown.layer(edit.layer)?, shown.bounds()).unwrap_or(shown.bounds());
    let mut area = before.union(&after);
    // Automation may replace a picker before a frame restores the preceding preview. Keep
    // that footprint too, so a different shape cannot leave the old preview on the canvas.
    if let Some(previous) = &app.shape_color_runtime.preview
        && (previous.document, previous.revision) == (edit.document.id.0, edit.revision)
    {
        area = area.union(&previous.area);
    }
    let shown = Arc::new(shown);
    let (document, revision, dialog) = (edit.document.id.0, edit.revision, edit.dialog);
    let runtime = &mut app.shape_color_runtime;
    runtime.serial = runtime.serial.wrapping_add(1);
    let key = TAG | (runtime.serial & (TAG - 1));
    runtime.preview = Some(Preview { document, revision, dialog, rgb, key, shown: shown.clone(), area });
    Some((shown, key))
}

pub(crate) fn damage(app: &PhotocraftApp, document: u64, revision: u64, from: u64, to: u64) -> Option<Rect> {
    let p = app.shape_color_runtime.preview.as_ref()?;
    let ours = |key| (TAG..TAG * 2).contains(&key);
    (p.document == document && p.revision == revision && (ours(from) || ours(to)) && (from == 0 || ours(from)) && (to == 0 || ours(to))).then_some(p.area)
}

#[cfg(test)]
mod tests;
