use serde_json::json;

use super::*;

const EPS: f64 = 1e-9;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// A block 100 x 50 x 20 mm along X, Y and Z, from the origin.
fn block() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 50, "height": 20, "body_name": "Block"}));
    s
}

fn snap(s: &mut Session, p: Value) -> Value {
    run(s, CMD, p)
}

fn refused(s: &mut Session, p: Value) -> String {
    s.execute(CMD, &p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("accepted: {p}"))
}

fn point(v: &Value) -> Vec2 {
    Vec2::new(v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

/// The boundary's size and centre on the sheet.
fn boundary(v: &Value) -> (Vec2, Vec2) {
    let (min, max) = (point(&v["boundary_mm"]["min"]), point(&v["boundary_mm"]["max"]));
    (max - min, (min + max) * 0.5)
}

/// The block's boundary in each view at 1:2, worked by hand: front 50 x 10, top 50 x 25, right
/// 25 x 10, and from an isometric corner (100 + 50)/sqrt(2)/2 by (100 + 50 + 2 x 20)/sqrt(6)/2.
/// The centre lands on the position, and the extents are the model's own millimetres.
#[test]
fn a_blocks_boundary_in_each_view_is_its_size_at_the_scale() {
    let mut s = block();
    let views = [
        (json!("front"), (50.0, 10.0)),
        (json!("top"), (50.0, 25.0)),
        (json!("right"), (25.0, 10.0)),
        (json!("iso"), (150.0 / 2f64.sqrt() / 2.0, 190.0 / 6f64.sqrt() / 2.0)),
        (json!({"direction": [0, 1, 0], "up": [0, 0, 1]}), (50.0, 10.0)),
    ];
    for (view, (w, h)) in views {
        let v = snap(&mut s, json!({"view": view, "scale": "1:2", "position_mm": [100, 120]}));
        let (size, centre) = boundary(&v);
        assert!((size - Vec2::new(w, h)).len() < EPS, "{view}: {size:?}");
        assert!((centre - Vec2::new(100.0, 120.0)).len() < EPS, "{view}: {centre:?}");
        assert_eq!((v["scale"]["text"].as_str(), v["scale"]["fitted"].as_bool()), (Some("1:2"), Some(false)));
    }
    let front = snap(&mut s, json!({"scale": "1:2"}));
    assert_eq!(front["view"]["named"], "front");
    assert_eq!((front["view"]["direction"].clone(), front["view"]["x"].clone()), (json!([0.0, 1.0, 0.0]), json!([1.0, 0.0, 0.0])));
    assert!((point(&front["extents"]["min"]) - Vec2::ZERO).len() < EPS && (point(&front["extents"]["max"]) - Vec2::new(100.0, 20.0)).len() < EPS);
    assert_eq!((front["extents"]["status"].as_str(), front["source"]["bodies"].as_u64()), (Some("approximate"), Some(1)));
    assert_eq!(front["source"]["timeline_at_end"], true);
    let iso = snap(&mut s, json!({"view": "iso", "scale": "1:2"}));
    assert_eq!(iso["view"]["named"], "iso_front_right");
}

/// With no scale the view takes the largest preset scale that fits the sheet's drawing border,
/// centred on the sheet: the 100 x 20 front view on A3 landscape at 2:1 (5:1 would be 500 mm
/// wide), boundary 110..310 x 128.5..168.5. A scale given is used as it is, even when it overhangs.
#[test]
fn without_a_scale_the_view_fits_and_centres_on_its_sheet() {
    let mut s = block();
    let v = snap(&mut s, json!({}));
    assert_eq!((v["scale"]["text"].as_str(), v["scale"]["fitted"].as_bool()), (Some("2:1"), Some(true)));
    assert!((point(&v["boundary_mm"]["min"]) - Vec2::new(110.0, 128.5)).len() < EPS, "{v}");
    assert!((point(&v["boundary_mm"]["max"]) - Vec2::new(310.0, 168.5)).len() < EPS, "{v}");
    let a4 = snap(&mut s, json!({"sheet": {"size": {"standard": "a4"}, "orientation": "portrait"}}));
    assert_eq!(a4["scale"]["text"], "1:1", "A4 portrait has 95 mm to the border each side: 190 mm of room, so 2:1 (200 mm) doesn't fit");
    let near_the_left = snap(&mut s, json!({"position_mm": [80, 148.5]}));
    assert_eq!(near_the_left["scale"]["text"], "1:1", "70 mm to the left border: 140 mm of room");
    let overhanging = snap(&mut s, json!({"scale": "5:1"}));
    assert!((boundary(&overhanging).0 - Vec2::new(500.0, 100.0)).len() < EPS);
    let off_the_border = refused(&mut s, json!({"position_mm": [5, 148.5]}));
    assert!(off_the_border.contains("no preset scale from 50:1 to 1:10000"), "{off_the_border}");
    assert_eq!(a4["sheet"], json!({"size": {"standard": "a4"}, "orientation": "portrait", "dpi": 300.0}), "the sheet, its defaults filled in");
}

/// Scale and position only scale and move the boundary; the margin grows each side by the
/// margin times the scale; the browser's eye counts for nothing; nothing in the design changes.
#[test]
fn scale_position_and_margin_set_the_boundary_and_the_design_is_untouched() {
    let mut s = block();
    let (undo, doc) = (s.undo.len(), s.doc.clone());
    let dirty = s.is_dirty();
    let one = boundary(&snap(&mut s, json!({"scale": "1:1", "position_mm": [0, 0]})));
    let half = boundary(&snap(&mut s, json!({"scale": "1:2", "position_mm": [0, 0]})));
    assert!((one.0 - half.0 * 2.0).len() < EPS && one.1.len() < EPS && half.1.len() < EPS);
    let margin = boundary(&snap(&mut s, json!({"scale": "1:2", "position_mm": [0, 0], "reference_margin_mm": 5})));
    assert!((margin.0 - (half.0 + Vec2::new(5.0, 5.0))).len() < EPS, "2.5 mm a side at 1:2: {margin:?}");
    let first = snap(&mut s, json!({"view": "top"}));
    assert_eq!(snap(&mut s, json!({"view": "top"})), first, "the same call gives the same answer");
    assert_eq!((s.undo.len(), s.is_dirty()), (undo, dirty));
    assert!(std::sync::Arc::ptr_eq(&doc, &s.doc), "nothing in the design changes");
    run(&mut s, "browser.visibility", json!({"items": ["Block"], "visible": false}));
    assert_eq!(snap(&mut s, json!({"view": "top"}))["boundary_mm"], first["boundary_mm"], "a hidden body is still drawn");
}

/// What the snapshot refuses, each with nothing changed: bad settings, the shaded styles until
/// the drawing renderer, the sheet's resolution out of range, an empty design, a position off
/// the sheet's limit, and a view that would reach past the paper's coordinates.
#[test]
fn hostile_snapshots_are_refused() {
    let mut s = block();
    let doc = s.doc.clone();
    let cases = [
        (json!({"scale": "0:1"}), "scale"),
        (json!({"scale": "-1:2"}), "scale"),
        (json!({"scale": "1e400:1"}), "scale"),
        (json!({"scale": "1:2:3"}), "scale"),
        (json!({"scale": 2}), "scale"),
        (json!({"scale": null}), "scale"),
        (json!({"view": "Front"}), "view is"),
        (json!({"view": {"direction": [0, 0, 0], "up": [0, 0, 1]}}), "direction"),
        (json!({"view": {"direction": [1e308, 1e308, 0], "up": [0, 0, 1]}}), "direction"),
        (json!({"view": {"direction": [0, 0, 1], "up": [0, 0, 2]}}), "along the view direction"),
        (json!({"display_style": "shaded"}), "not supported yet"),
        (json!({"reference_style": "shaded_visible"}), "not supported yet"),
        (json!({"display_style": "Shaded"}), "unknown variant"),
        (json!({"sheet": {"dpi": 24.9}}), "dpi"),
        (json!({"sheet": {"dpi": 2400.1}}), "dpi"),
        (json!({"sheet": {"size": {"custom": {"sides_mm": [0, 10]}}}}), "sheet"),
        (json!({"reference_margin_mm": -1}), "reference_margin_mm"),
        (json!({"position_mm": [1e308, 0]}), "point"),
        (json!({"position_mm": [5000.1, 0], "scale": "1:1"}), "position"),
        (json!({"scael": "1:2"}), "scael"),
    ];
    for (p, says) in cases {
        let e = refused(&mut s, p.clone());
        assert!(e.contains(says), "{p}: {e}");
        assert!(std::sync::Arc::ptr_eq(&doc, &s.doc), "{p}");
    }
    let mut big = Session::default();
    run(&mut big, "solid.box", json!({"length": 300, "width": 10, "height": 10}));
    let e = refused(&mut big, json!({"scale": "1000:1"}));
    assert!(e.contains("past ±100000 mm"), "{e}");
    let e = refused(&mut Session::default(), json!({}));
    assert!(e.contains("no bodies"), "{e}");
}

/// A view shows every body the design places, so a design whose assembly walk stops at its depth
/// limit before some occurrences is refused, and one that ends at the limit is drawn: a chain of
/// components one inside the next, one level at a time across the limit.
#[test]
fn a_view_of_an_assembly_past_the_walks_limit_is_refused() {
    let mut s = block();
    let nest = |s: &mut Session, level: u64| {
        let (component, parent) = (1000 + level, if level == 0 { 0 } else { 1000 + level - 1 });
        let doc = s.doc_mut();
        doc.components.push(solidcraft_doc::Component { id: component, name: format!("L{level}"), parent, ..Default::default() });
        let name = format!("L{level}");
        // From JSON, so a field another change adds to occurrences takes its default here.
        let occurrence = json!({"id": 5000 + level, "component": component, "parent": parent, "name": name});
        doc.occurrences.push(serde_json::from_value(occurrence).unwrap());
    };
    // The deepest occurrence's path is MAX_PLACEMENT_DEPTH + 1 long: walked, its children not.
    for level in 0..=MAX_PLACEMENT_DEPTH as u64 {
        nest(&mut s, level);
    }
    assert!(s.doc.placements_complete());
    snap(&mut s, json!({"scale": "1:1"}));
    nest(&mut s, MAX_PLACEMENT_DEPTH as u64 + 1);
    assert!(!s.doc.placements_complete());
    let e = refused(&mut s, json!({"scale": "1:1"}));
    assert!(e.contains("deeper than 64 levels or has more than 100000 placements"), "{e}");
}
