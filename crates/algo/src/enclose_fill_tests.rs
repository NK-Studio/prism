use super::*;

const W: usize = 40;
const H: usize = 30;
const INK: [u8; 4] = [0, 0, 0, 255];
const OPEN: [u8; 4] = [0, 0, 0, 0];

fn area() -> Rect {
    Rect::new(0, 0, W as i32, H as i32)
}

/// Transparent canvas with black square outlines (one pixel wide) at `boxes` (x0, y0, x1, y1
/// inclusive), each with an optional gap of `gap` pixels in its top edge.
fn canvas(boxes: &[(usize, usize, usize, usize, usize)]) -> Vec<[u8; 4]> {
    let mut img = vec![OPEN; W * H];
    for &(x0, y0, x1, y1, gap) in boxes {
        for x in x0..=x1 {
            let mid = (x0 + x1) / 2;
            if gap == 0 || x < mid || x >= mid + gap {
                img[y0 * W + x] = INK;
            }
            img[y1 * W + x] = INK;
        }
        for y in y0..=y1 {
            img[y * W + x0] = INK;
            img[y * W + x1] = INK;
        }
    }
    img
}

fn rect_lasso(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

fn hard() -> EncloseOptions {
    EncloseOptions { anti_alias: false, ..Default::default() }
}

#[test]
fn fills_enclosed_area_but_not_the_surroundings() {
    let img = canvas(&[(5, 5, 15, 15, 0)]);
    let r = enclose_region(&img, area(), &rect_lasso(2.0, 2.0, 20.0, 20.0), &hard()).unwrap();
    assert_eq!(r.areas, 1);
    assert_eq!(r.region.at(10, 10), 1.0, "inside the outline");
    assert_eq!(r.region.at(5, 10), 1.0, "the fill runs under the line itself");
    assert_eq!(r.region.at(4, 10), 0.0, "but not past it");
    assert_eq!(r.region.at(3, 3), 0.0, "the ring between outline and lasso touches the lasso");
}

#[test]
fn areas_that_leak_out_of_the_lasso_stay_unfilled() {
    // The lasso cuts through the second box: its inside continues outside the lasso.
    let img = canvas(&[(2, 2, 10, 10, 0), (20, 2, 35, 20, 0)]);
    let r = enclose_region(&img, area(), &rect_lasso(0.0, 0.0, 26.0, 25.0), &hard()).unwrap();
    assert_eq!(r.areas, 2, "the first box, and the second box's lines inside the lasso");
    assert_eq!(r.region.at(6, 6), 1.0);
    assert_eq!(r.region.at(22, 10), 0.0, "the open inside continues outside the lasso");
    assert_eq!(r.region.at(20, 10), 1.0, "a line inside the lasso is filled up to it");
    assert_eq!(r.region.at(30, 2), 0.0, "and not past it");
}

#[test]
fn close_gap_seals_small_openings() {
    let img = canvas(&[(5, 5, 20, 20, 2)]);
    let lasso = rect_lasso(1.0, 1.0, 25.0, 25.0);
    let open = enclose_region(&img, area(), &lasso, &hard()).unwrap();
    assert_eq!(open.region.at(12, 12), 0.0, "the gap leaks to the lasso: only the lines are filled");
    let r = enclose_region(&img, area(), &lasso, &EncloseOptions { close_gap: 3.0, ..hard() }).unwrap();
    assert_eq!(r.areas, 1);
    assert_eq!(r.region.at(12, 12), 1.0);
    assert_eq!(r.region.at(6, 6), 1.0, "the fill reaches the lines");
    assert_eq!(r.region.at(5, 10), 1.0, "and runs under them");
    assert_eq!(r.region.at(12, 5), 0.0, "the outside soaks into the sealed gap");
    assert_eq!(r.region.at(3, 3), 0.0, "outside the outline stays empty");
}

#[test]
fn shrink_pulls_back_from_the_outside_and_grow_changes_nothing() {
    // An anti-aliased left edge: grey, black, grey (x = 4, 5, 6); the inside starts at x = 7.
    let mut img = canvas(&[(5, 5, 15, 15, 0)]);
    for y in 5..=15 {
        img[y * W + 4] = [127, 127, 127, 255];
        img[y * W + 6] = [127, 127, 127, 255];
    }
    let lasso = rect_lasso(2.0, 2.0, 20.0, 20.0);
    let plain = enclose_region(&img, area(), &lasso, &hard()).unwrap();
    assert_eq!(plain.region.at(4, 10), 1.0, "the whole line is covered");
    assert_eq!(plain.region.at(3, 10), 0.0);
    let grown = enclose_region(&img, area(), &lasso, &EncloseOptions { expand: 3.0, ..hard() }).unwrap();
    assert_eq!(grown, plain, "growing never enters the open outside");
    let shrunk = enclose_region(&img, area(), &lasso, &EncloseOptions { expand: -2.0, ..hard() }).unwrap();
    assert_eq!(shrunk.region.at(5, 10), 0.0, "two pixels off the outside");
    assert_eq!(shrunk.region.at(6, 10), 1.0);
    assert_eq!(shrunk.region.at(10, 10), 1.0);
}

#[test]
fn shrink_ignores_the_lasso_and_canvas_edges() {
    // A solid ink block from the canvas edge, cut by the lasso's right edge (x = 12): a shrink
    // measured from the open outside leaves the cut and the canvas edge alone.
    let mut img = vec![OPEN; W * H];
    for y in 5..=15 {
        for x in 0..=20 {
            img[y * W + x] = INK;
        }
    }
    let r = enclose_region(&img, area(), &rect_lasso(-5.0, 2.0, 12.0, 20.0), &EncloseOptions { expand: -1.0, ..hard() }).unwrap();
    assert_eq!(r.region.at(11, 10), 1.0, "next to the lasso cut");
    assert_eq!(r.region.at(12, 10), 0.0, "outside the lasso");
    assert_eq!(r.region.at(0, 10), 1.0, "on the canvas edge");
    assert_eq!(r.region.at(5, 5), 0.0, "next to the open outside");
    assert_eq!(r.region.at(5, 6), 1.0);
}

#[test]
fn a_lone_line_inside_the_lasso_is_filled() {
    // CLIP STUDIO PAINT fills a stroke that encloses nothing: open space reaches all around it,
    // but nothing reaches through it.
    let mut img = vec![OPEN; W * H];
    for x in 5..25 {
        img[10 * W + x] = INK;
    }
    let r = enclose_region(&img, area(), &rect_lasso(2.0, 2.0, 30.0, 20.0), &hard()).unwrap();
    assert_eq!(r.areas, 1);
    assert_eq!(r.region.at(10, 10), 1.0);
    assert_eq!(r.region.at(10, 9), 0.0);
    assert_eq!(r.region.bbox, Rect::new(5, 10, 25, 11));
}

#[test]
fn expansion_never_leaks_through_a_sealed_gap() {
    let img = canvas(&[(5, 5, 20, 20, 2)]);
    let o = EncloseOptions { close_gap: 3.0, expand: 10.0, ..hard() };
    let r = enclose_region(&img, area(), &rect_lasso(1.0, 1.0, 25.0, 25.0), &o).unwrap();
    assert_eq!(r.region.at(12, 3), 0.0, "nothing above the gap");
    assert_eq!(r.region.at(5, 10), 1.0, "the line itself is reached");
}

#[test]
fn white_paper_counts_as_open_and_tolerance_decides_line_art() {
    let mut img = canvas(&[(5, 5, 15, 15, 0)]);
    for p in img.iter_mut().filter(|p| p[3] == 0) {
        *p = [255, 255, 255, 255];
    }
    let lasso = rect_lasso(2.0, 2.0, 20.0, 20.0);
    assert_eq!(enclose_region(&img, area(), &lasso, &hard()).unwrap().region.at(10, 10), 1.0);
    // Light grey lines are open at a wide margin: nothing is enclosed then.
    let grey: Vec<[u8; 4]> = img.iter().map(|p| if p[0] == 0 { [200, 200, 200, 255] } else { *p }).collect();
    assert!(enclose_region(&grey, area(), &lasso, &EncloseOptions { tolerance: 80.0, ..hard() }).is_none());
    assert!(enclose_region(&grey, area(), &lasso, &EncloseOptions { tolerance: 20.0, ..hard() }).is_some());
}

#[test]
fn lasso_past_the_canvas_edge_fills_areas_closed_by_the_edge() {
    // A line across the canvas: the area below it is closed by the canvas edges.
    let mut img = vec![OPEN; W * H];
    for x in 0..W {
        img[20 * W + x] = INK;
    }
    let r = enclose_region(&img, area(), &rect_lasso(-10.0, 10.0, W as f32 + 10.0, H as f32 + 10.0), &hard()).unwrap();
    assert_eq!(r.region.at(5, 25), 1.0);
    assert_eq!(r.region.at(5, 15), 0.0, "above the line reaches the lasso's top edge");
    assert!(r.region.bbox.x0 >= 0 && r.region.bbox.x1 <= W as i32 && r.region.bbox.y1 <= H as i32, "clipped to the canvas");
}

#[test]
fn anti_aliasing_softens_only_the_edge() {
    let img = canvas(&[(5, 5, 15, 15, 0)]);
    let r = enclose_region(&img, area(), &rect_lasso(2.0, 2.0, 20.0, 20.0), &EncloseOptions::default()).unwrap();
    assert_eq!(r.region.at(10, 10), 1.0);
    let edge = r.region.at(5, 10);
    assert!((0.5..1.0).contains(&edge), "edge pixel {edge}");
    let fringe = r.region.at(4, 10);
    assert!(fringe > 0.0 && fringe < 0.5, "fringe pixel {fringe}");
}

#[test]
fn degenerate_input_is_rejected_without_panicking() {
    let img = canvas(&[(5, 5, 15, 15, 0)]);
    let o = EncloseOptions { tolerance: f32::NAN, close_gap: f32::INFINITY, expand: f32::NEG_INFINITY, ..Default::default() };
    assert!(enclose_region(&img, area(), &[(0.0, 0.0), (1.0, 1.0)], &o).is_none());
    assert!(enclose_region(&img, area(), &[(f32::NAN, 0.0), (1.0, f32::INFINITY), (2.0, 2.0)], &o).is_none());
    assert!(enclose_region(&img, area(), &rect_lasso(1e30, 1e30, 2e30, 2e30), &o).is_none());
    assert!(enclose_region(&img[..10], area(), &rect_lasso(2.0, 2.0, 20.0, 20.0), &o).is_none(), "image of the wrong size");
    assert!(enclose_region(&[], Rect::EMPTY, &rect_lasso(2.0, 2.0, 20.0, 20.0), &o).is_none());
    let _ = enclose_region(&img, area(), &rect_lasso(-1e30, -1e30, 1e30, 1e30), &o);
}
