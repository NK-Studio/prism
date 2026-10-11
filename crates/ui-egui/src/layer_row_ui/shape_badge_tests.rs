//! Vector-kind overlay follows the actual layer content, without taking thumbnail input.

use egui::{Rect, Shape, pos2, vec2};
use egui_kittest::Harness;
use prism_doc::LayerId;
use serde_json::json;

use crate::{
    PrismApp,
    theme::{ThemeKind, Tokens},
};

fn badges(h: &Harness<'_, PrismApp>, layer: u64) -> Vec<Rect> {
    let row = super::recorded(&h.ctx).into_iter().find(|r| r.layer == layer).expect("visible layer").row;
    let size = if Tokens::get(&h.ctx).pro { 24.0 } else { 34.0 };
    let thumb = Rect::from_min_size(pos2(row.left() + 34.0, row.center().y - size / 2.0), vec2(size, size));
    let [background, _] = Tokens::shape_thumbnail_badge();
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Rect(r) if r.fill == background && r.stroke.width == 0.0 && thumb.contains_rect(r.rect) => Some(r.rect),
            _ => None,
        })
        .collect()
}

fn harness(kind: &str, theme: ThemeKind, scale: f32) -> (Harness<'static, PrismApp>, u64) {
    let mut s = prism_engine::Session::new();
    s.execute("file.new", json!({"width":160,"height":120})).unwrap();
    let id =
        s.execute("shape.create", json!({"kind":kind,"rect":[20,20,100,70],"from":[20,20],"to":[120,90],"fill":"#000000"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = Harness::builder().with_size(vec2(1000.0, 720.0)).with_pixels_per_point(scale).with_max_steps(64).build_eframe(move |cc| {
        PrismApp::setup_context(&cc.egui_ctx, theme);
        PrismApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":theme.id(),"dockWidth":250,"dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    (h, id)
}

#[test]
fn shapes_keep_badge_during_selection_hover_and_masks_in_all_themes_and_scales() {
    for theme in ThemeKind::ALL {
        for scale in [1.0, 2.0] {
            for kind in ["rectangle", "roundedRect", "ellipse", "polygon", "line"] {
                let (mut h, id) = harness(kind, theme, scale);
                let tiles = badges(&h, id);
                assert_eq!(tiles.len(), 1, "{theme:?} {scale} {kind}");
                let tile = tiles[0];
                let [background, foreground] = Tokens::shape_thumbnail_badge();
                let contrast = |c: egui::Color32| c.r() as f32;
                assert!(contrast(foreground) - contrast(background) > 100.0);
                let anchors = h
                    .output()
                    .shapes
                    .iter()
                    .filter(|s| matches!(&s.shape, Shape::Rect(r) if r.fill == background && r.stroke.color == foreground && tile.contains_rect(r.rect)))
                    .count();
                assert_eq!(anchors, 4, "four vector anchor points");
                let history = h.state().session.active().unwrap().history.past_len();
                h.state_mut().run("layer.new.layer", json!({"name":"Raster"})).unwrap();
                h.run_steps(3);
                let unselected = badges(&h, id);
                assert_eq!(unselected.len(), 1);
                assert_eq!(unselected[0].size(), tile.size());
                h.hover_at(unselected[0].center());
                h.run_steps(3);
                assert_eq!(badges(&h, id), unselected, "unselected and hovered retains tile");
                assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1, "painting overlay never edits history");
                h.state_mut().run("layer.layerMask.revealAll", json!({"layer":id})).unwrap();
                h.run_steps(3);
                assert_eq!(badges(&h, id), unselected, "pixel mask does not replace shape overlay");
            }
        }
    }
}

#[test]
fn rasterization_removes_badge_and_undo_restores_it() {
    let (mut h, id) = harness("rectangle", ThemeKind::ProMedium, 1.0);
    assert_eq!(badges(&h, id).len(), 1);
    h.state_mut().run("layer.rasterize.shape", json!({"layer":id})).unwrap();
    h.run_steps(4);
    assert!(badges(&h, id).is_empty());
    assert!(matches!(h.state().session.active().unwrap().doc.layer(LayerId(id)).unwrap().content, prism_doc::LayerContent::Raster(_)));
    assert!(h.state_mut().session.undo());
    h.run_steps(4);
    assert_eq!(badges(&h, id).len(), 1);
}

#[test]
fn shape_badge_stays_inside_narrow_thumbnails_and_skips_clipped_rows() {
    let mut s = prism_engine::Session::new();
    s.execute("file.new", json!({"width":32,"height":32})).unwrap();
    let id = s.execute("shape.create", json!({"kind":"rectangle","rect":[0,0,10,10]})).unwrap()["layer"].as_u64().unwrap();
    let shape = s.active().unwrap().doc.layer(LayerId(id)).unwrap().clone();
    for side in [14.0, 24.0, 34.0] {
        let thumb = Rect::from_min_size(pos2(20.0, 20.0), vec2(side, side));
        let mut h = Harness::builder().with_size(vec2(120.0, 120.0)).build_ui({
            let shape = shape.clone();
            move |ui| {
                super::shape_thumb_badge(ui, &shape, thumb);
                super::shape_thumb_badge(ui, &shape, thumb.translate(vec2(0.0, 1000.0)));
            }
        });
        h.run_steps(2);
        let [bg, _] = Tokens::shape_thumbnail_badge();
        let tiles: Vec<_> = h
            .output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == bg && r.stroke.width == 0.0 => Some(r.rect),
                _ => None,
            })
            .collect();
        assert_eq!(tiles.len(), 1);
        assert!(thumb.contains_rect(tiles[0]));
        assert_eq!(tiles[0].right_bottom(), thumb.right_bottom());
    }
}
