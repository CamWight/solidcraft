use serde_json::json;
use solidcraft_doc::drawing::view::{View, ViewKind, ViewPlace};
use solidcraft_doc::drawing_view::{ViewDirection, ViewOverrides};
use solidcraft_geom::Vec3;
use solidcraft_geom::drawing_view::NamedView;

use super::*;
use crate::schema::tests::conforms;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    let reply = s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    let schema = output_schema_for::<DimensionResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply) || !id.starts_with("drawing.dimension"), "{id}: {reply}");
    reply
}

fn refused(s: &mut Session, p: Value) -> String {
    s.execute(ADD, &p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("accepted: {p}"))
}

/// A box 80 x 40 x 30 mm along X, Y and Z from the origin, called Box, and a drawing of it, open,
/// with its front view at 1:1: the session and the view's id.
fn front_view() -> (Session, u64) {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 80, "width": 40, "height": 30, "body_name": "Box"}));
    run(&mut s, "drawing.create", json!({}));
    let mut d = (**s.drawing("test").unwrap()).clone();
    let id = d.allocate_id(&mut ids::IdSource::seeded(4896)).unwrap();
    let (sheet, source) = (d.sheets[0].id, d.sources[0].id);
    let kind = ViewKind::Base { source, view: ViewDirection::Named(NamedView::Front) };
    let settings = ViewOverrides { scale: Some("1:1".parse().unwrap()), ..ViewOverrides::default() };
    d.add_views([Arc::new(View { id, sheet, kind, place: ViewPlace::At([200.0, 150.0]), settings, reference_margin_mm: 0.0 })]);
    d.validate().unwrap();
    s.doc_mut().put_drawing(Arc::new(d));
    (s, id)
}

/// The index of the box's edge from `a` to `b`, as `{body, index}` names it.
fn edge_index(s: &Session, a: [f64; 3], b: [f64; 3]) -> usize {
    let (a, b) = (Vec3::from(a), Vec3::from(b));
    let state = s.model.state();
    let mesh = state.body("Box").unwrap().mesh();
    let ends = |line: &Vec<Vec3>| (line[0], line[line.len() - 1]);
    let joins = |(p, q): (Vec3, Vec3)| (p.dist(a) < 1e-9 && q.dist(b) < 1e-9) || (p.dist(b) < 1e-9 && q.dist(a) < 1e-9);
    mesh.edges.iter().position(|line| joins(ends(line))).unwrap()
}

fn add(s: &mut Session, view: u64, kind: Value) -> Value {
    run(s, ADD, json!({"view": ids::encode(view), "kind": kind}))
}

fn reads(reply: &Value, value: f64, text: &str) {
    assert_eq!((reply["status"].as_str(), reply["text"].as_str()), (Some("ok"), Some(text)), "{reply}");
    assert!((reply["value"].as_f64().unwrap() - value).abs() < 1e-9, "{reply}");
}

/// A dimension added by points or by edges is saved by the edges' names, and reads as the front
/// view shows the box: its bottom front corners 80 apart, its top front edge 80 long, and that
/// edge square to the left front edge.
#[test]
fn a_dimension_is_added_by_points_or_edges_and_saved_by_name() {
    let (mut s, view) = front_view();
    let steps = s.undo.len();
    let by_points = add(&mut s, view, json!({"linear": {"a": [0, 0, 0], "b": [80, 0, 0], "direction": "horizontal"}}));
    reads(&by_points, 80.0, "80.00");
    assert_eq!((by_points["exact"].clone(), by_points["foreshortened"].clone()), (json!(true), json!(false)));
    assert_eq!(s.undo.len(), steps + 1, "one undo step");
    let corner = &by_points["record"]["kind"]["linear"]["a"];
    assert_eq!((corner["body"].as_str(), corner["edges"].as_array().map(Vec::len)), (Some("Box"), Some(3)), "a box corner: three edges");
    assert!(corner.get("point").is_none() && corner.to_string().contains('|'), "names, not the point: {corner}");
    // The same corners by two of their edges each.
    let bottom = edge_index(&s, [0.0, 0.0, 0.0], [80.0, 0.0, 0.0]);
    let (left, right) = (edge_index(&s, [0.0, 0.0, 0.0], [0.0, 0.0, 30.0]), edge_index(&s, [80.0, 0.0, 0.0], [80.0, 0.0, 30.0]));
    let edge = |index: usize| json!({"body": "Box", "index": index});
    let by_edges = add(&mut s, view, json!({"linear": {"a": {"edges": [edge(bottom), edge(left)]}, "b": {"edges": [edge(right), edge(bottom)]}}}));
    reads(&by_edges, 80.0, "80.00");
    // The top front edge's length, by a point on it and by its index, saves the same name.
    let top = edge_index(&s, [0.0, 0.0, 30.0], [80.0, 0.0, 30.0]);
    let at = add(&mut s, view, json!({"length": {"edge": [40, 0, 30]}}));
    let indexed = add(&mut s, view, json!({"length": {"edge": edge(top)}}));
    reads(&at, 80.0, "80.00");
    assert_eq!(at["record"]["kind"], indexed["record"]["kind"]);
    let square = run(&mut s, ADD, json!({"view": ids::encode(view), "kind": {"angle": {"a": [40, 0, 30], "b": [0, 0, 15]}}, "decimals": 1}));
    reads(&square, 90.0, "90.0°");
    // Inspect reads each one back as add answered it.
    let id = by_points["dimension"].clone();
    assert_eq!(run(&mut s, INSPECT, json!({"dimension": id})), by_points);
}

/// An undone add takes the dimension out, and a redo puts it back.
#[test]
fn an_added_dimension_is_one_undo_step() {
    let (mut s, view) = front_view();
    let before = s.drawing("test").unwrap().to_json().unwrap();
    let p = json!({"view": ids::encode(view), "kind": {"length": {"edge": [40, 0, 30]}}, "offset_mm": -12.5, "text": {"at": [0, 40]}});
    let added = run(&mut s, ADD, p);
    let after = s.drawing("test").unwrap().to_json().unwrap();
    assert_eq!((added["record"]["offset_mm"].clone(), added["record"]["text"].clone()), (json!(-12.5), json!({"at": [0.0, 40.0]})));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.drawing("test").unwrap().to_json().unwrap(), before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.drawing("test").unwrap().to_json().unwrap(), after);
}

/// A pick that doesn't name one thing, and a dimension the record would refuse, are refused
/// with the drawing unchanged.
#[test]
fn a_pick_that_names_nothing_or_several_things_is_refused() {
    let (mut s, view) = front_view();
    let before = s.drawing("test").unwrap().to_json().unwrap();
    let v = ids::encode(view);
    let length = |edge: Value| json!({"view": v, "kind": {"length": {"edge": edge}}});
    let linear = |a: Value| json!({"view": v, "kind": {"linear": {"a": a, "b": [80, 0, 0]}}});
    let cases = [
        (length(json!([40, 20, 15])), "no edge at [40, 20, 15]"),
        (length(json!([0, 0, 0])), "[0, 0, 0] is on 3 edges"),
        (length(json!({"body": "Nope", "index": 0})), "no body `Nope`"),
        (length(json!({"body": "Box", "index": 999})), "body `Box` has no edge 999"),
        (length(json!({"point": [40, 0, 30], "index": 1})), "an edge is a point on it"),
        (linear(json!([40, 0, 0])), "no corner at [40, 0, 0]"),
        (linear(json!({"edges": [{"point": [40, 0, 0]}, {"point": [40, 40, 30]}]})), "the corner's edges don't meet"),
        (linear(json!({"edges": [{"point": [40, 0, 0], "path": [1]}]})), "each without a path of its own"),
        (json!({"view": v, "kind": {"length": {"edge": [40, 0, 30]}}, "decimals": 7}), "at most 6 decimals"),
        (json!({"view": v, "kind": {"length": {"edge": [40, 0, 30]}}, "offset_mm": 1e9}), "on paper"),
        (json!({"view": ids::encode(99), "kind": {"length": {"edge": [40, 0, 30]}}}), "the drawing has no view"),
        (json!({"view": v, "kind": {"radius": {"edge": [40, 0, 30]}}}), "radius"),
    ];
    for (p, why) in cases {
        let e = refused(&mut s, p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
    assert_eq!(s.drawing("test").unwrap().to_json().unwrap(), before);
    let e = s.execute(INSPECT, &json!({"dimension": ids::encode(99)})).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("the drawing has no dimension"), "{e}");
}
