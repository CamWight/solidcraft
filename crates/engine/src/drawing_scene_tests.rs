use std::sync::Arc;

use serde_json::{Value, json};
use solidcraft_doc::MAX_PLACEMENT_DEPTH;
use solidcraft_doc::drawing::view::{View, ViewKind, ViewPlace};
use solidcraft_doc::drawing_view::{ViewDirection, ViewOverrides};
use solidcraft_geom::drawing_view::{NamedView, ProjectedSide};
use solidcraft_geom::paper::ObjectKind;

use super::*;
use crate::schema::drawing_scene::SheetSceneResult;
use crate::schema::output_schema_for;
use crate::schema::tests::conforms;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// A block 100 x 50 x 20 mm along X, Y and Z from the origin, and a drawing of it on A3 with the
/// ISO border: the front view at 1:2 centred at (150, 150), and the view above it, the top view in
/// third angle, lined up with it at y 220. The session, the sheet and the two views' ids.
fn front_and_top() -> (Session, u64, [u64; 2]) {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 50, "height": 20, "body_name": "Block"}));
    run(&mut s, "drawing.create", json!({}));
    let mut d = (**s.drawing("test").unwrap()).clone();
    let mut id_source = solidcraft_doc::ids::IdSource::seeded(4890);
    let (sheet, source) = (d.sheets[0].id, d.sources[0].id);
    let mut add = |d: &mut Drawing, kind: ViewKind, place: ViewPlace, settings: ViewOverrides| {
        let id = d.allocate_id(&mut id_source).unwrap();
        d.views.push(Arc::new(View { id, sheet, kind, place, settings, reference_margin_mm: 0.0 }));
        id
    };
    let half = ViewOverrides { scale: Some("1:2".parse().unwrap()), ..ViewOverrides::default() };
    let front = add(&mut d, ViewKind::Base { source, view: ViewDirection::Named(NamedView::Front) }, ViewPlace::At([150.0, 150.0]), half);
    let top = add(&mut d, ViewKind::Projected { parent: front, side: ProjectedSide::Above }, ViewPlace::Aligned(220.0), ViewOverrides::default());
    d.validate().unwrap();
    s.doc_mut().put_drawing(Arc::new(d));
    (s, sheet, [front, top])
}

/// Component `L{level}`, placed inside `L{level - 1}` (the design itself at level 0).
fn nest(s: &mut Session, level: u64) {
    let (component, parent) = (1000 + level, if level == 0 { 0 } else { 1000 + level - 1 });
    let doc = s.doc_mut();
    doc.components.push(solidcraft_doc::Component { id: component, name: format!("L{level}"), parent, ..Default::default() });
    let name = format!("L{level}");
    // From JSON, so a field another change adds to occurrences takes its default here.
    let occurrence = json!({"id": 5000 + level, "component": component, "parent": parent, "name": name});
    doc.occurrences.push(serde_json::from_value(occurrence).unwrap());
}

/// Sheet `sheet` of the drawing open, built at once.
fn built(s: &Session, sheet: u64) -> SheetScene {
    let d = s.drawing("test").unwrap().clone();
    build(s, &d, sheet).unwrap()
}

/// The scene as its JSON text: what byte for byte compares.
fn bytes(scene: &Scene) -> String {
    serde_json::to_string(scene).unwrap()
}

/// The drawing border's groups of sheet `sheet` of the drawing open, as its sheet layout draws them.
fn border(s: &Session, sheet: u64) -> Vec<Group> {
    let d = s.drawing("test").unwrap();
    let record = d.sheets.iter().find(|x| x.id == sheet).unwrap();
    record.border.as_ref().unwrap().layout(&record.sheet).unwrap().paper(sheet)
}

/// The box round the ends of `group`'s stroke pieces, and how long they are in all.
fn reach(group: &Group) -> (Vec2, Vec2, f64) {
    let (mut lo, mut hi, mut length) = (Vec2::new(f64::INFINITY, f64::INFINITY), Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY), 0.0);
    for item in &group.items {
        let Item::Stroke { path, .. } = item else { continue };
        for seg in path {
            for p in [seg.start(), seg.end()] {
                lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
                hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
            }
            length += seg.length();
        }
    }
    (lo, hi, length)
}

/// Whether `a` and `b` are the same strokes in the same order, their pieces' ends within `tol`.
fn same_within(a: &Group, b: &Group, tol: f64) -> bool {
    let near = |s: &Seg2, t: &Seg2| s.start().dist(t.start()) <= tol && s.end().dist(t.end()) <= tol;
    a.items.len() == b.items.len()
        && a.items.iter().zip(&b.items).all(|pair| match pair {
            (Item::Stroke { path: p, closed: c, width_mm: w }, Item::Stroke { path: q, closed: d, width_mm: v }) => {
                c == d && w == v && p.len() == q.len() && p.iter().zip(q).all(|(s, t)| near(s, t))
            }
            _ => false,
        })
}

/// The scene is the sheet's three border groups as its layout draws them, byte for byte, then the
/// two views in id order: the front view round its 50 x 10 mm outline centred at (150, 150), 120 mm
/// of line in all, the top view round 50 x 25 mm centred at (150, 220), 150 mm. The command answers
/// the same scene, matching its schema, and the SVG writer takes it with the border's three groups
/// before the views'.
#[test]
fn a_sheet_is_its_border_then_its_views_by_id() {
    let (mut s, sheet, [front, top]) = front_and_top();
    let SheetScene { scene, views } = built(&s, sheet);
    let border = border(&s, sheet);
    assert_eq!(border.len(), 3);
    assert_eq!(serde_json::to_string(&scene.groups[..3]).unwrap(), serde_json::to_string(&border).unwrap());
    let order = if front < top { [front, top] } else { [top, front] };
    let tail: Vec<(u64, ObjectKind)> = scene.groups[3..].iter().map(|g| (g.object, g.kind)).collect();
    assert_eq!(tail, order.map(|id| (id, ObjectKind::View)));
    for (id, lo, hi, length) in [(front, [125.0, 145.0], [175.0, 155.0], 120.0), (top, [125.0, 207.5], [175.0, 232.5], 150.0)] {
        let (min, max, drawn) = reach(scene.groups.iter().find(|g| g.object == id).unwrap());
        assert!(min.dist(Vec2::from(lo)) < 1e-6 && max.dist(Vec2::from(hi)) < 1e-6 && (drawn - length).abs() < 1e-6, "{id}: {min:?} {max:?} {drawn}");
    }
    let statuses: Vec<(u64, Status)> = views.iter().map(|v| (v.view, v.status.clone())).collect();
    assert_eq!(statuses, order.map(|id| (id, Status::Drawn)));

    let reply = run(&mut s, CMD, json!({"sheet": "Sheet 1"}));
    let schema = output_schema_for::<SheetSceneResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    assert_eq!(reply["scene"], serde_json::to_value(&scene).unwrap());
    let replied: Vec<(Value, Value)> = reply["views"].as_array().unwrap().iter().map(|v| (v["view"].clone(), v["status"].clone())).collect();
    assert_eq!(replied, order.map(|id| (json!(ids::encode(id)), json!("drawn"))));

    let (svg, _) = solidcraft_io::paper::paper_svg(&scene.prepare().unwrap()).unwrap();
    let svg = String::from_utf8(svg).unwrap();
    let borders: Vec<usize> = svg.match_indices("data-kind=\"drawing_border\"").map(|(at, _)| at).collect();
    let first_view = svg.find("data-kind=\"view\"").unwrap();
    assert!(borders.len() == 3 && borders.iter().all(|at| *at < first_view), "{svg}");
}

/// Each view's group is the snapshot's for the same view, scale and centre: the same strokes in the
/// same order, their ends within rounding. The scene works a view's lines out centred on it and
/// then moves them; the snapshot maps them to the centre in one step.
#[test]
fn each_view_draws_as_its_snapshot_within_rounding() {
    let (mut s, sheet, [front, top]) = front_and_top();
    let scene = built(&s, sheet).scene;
    // The snapshot works on the model, so with no drawing open.
    run(&mut s, "drawing.close", json!({}));
    for (id, named, at) in [(front, "front", [150.0, 150.0]), (top, "top", [150.0, 220.0])] {
        let v = run(&mut s, "drawing.view.snapshot", json!({"view": named, "scale": "1:2", "position_mm": at, "scene": true}));
        let snapped: Scene = serde_json::from_value(v["scene"].clone()).unwrap();
        let ours = scene.groups.iter().find(|g| g.object == id).unwrap();
        assert!(same_within(ours, &snapped.groups[0], 1e-9), "{id}: {ours:?} {:?}", snapped.groups[0]);
    }
}

/// A sheet with no views is its border alone.
#[test]
fn a_sheet_with_no_views_is_its_border() {
    let (mut s, _, _) = front_and_top();
    let second = run(&mut s, "drawing.sheet.add", json!({}))["sheet"].as_str().and_then(ids::decode).unwrap();
    let SheetScene { scene, views } = built(&s, second);
    assert_eq!((serde_json::to_string(&scene.groups).unwrap(), views.len()), (serde_json::to_string(&border(&s, second)).unwrap(), 0));
}

/// A view in a style the paper scene can't draw yet fails with the snapshot's reason, and a view
/// with nothing to measure is a placeholder; each draws nothing, and the sheet's other views and
/// its border are drawn all the same. The command says why for each.
#[test]
fn a_view_that_cant_be_drawn_leaves_out_only_itself() {
    let (mut s, sheet, [front, top]) = front_and_top();
    let status = |s: &Session, id: u64| built(s, sheet).views.into_iter().find(|v| v.view == id).map(|v| v.status);
    for (style, why) in [
        ("shaded", "not supported yet: shaded views need the drawing renderer (#5624)"),
        ("visible_hidden", "not supported yet: hidden lines in the paper scene need dashed lines (#4890 A5)"),
    ] {
        run(&mut s, "drawing.view.edit", json!({"view": ids::encode(top), "set": {"display_style": style}}));
        assert_eq!((status(&s, front), status(&s, top)), (Some(Status::Drawn), Some(Status::Failed(why.into()))), "{style}");
        let objects: Vec<u64> = built(&s, sheet).scene.groups.iter().map(|g| g.object).collect();
        assert_eq!(objects, [sheet, sheet, sheet, front], "{style}");
    }
    let reply = run(&mut s, CMD, json!({"sheet": ids::encode(sheet)}));
    let schema = output_schema_for::<SheetSceneResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    let failed = reply["views"].as_array().unwrap().iter().find(|v| v["view"] == ids::encode(top)).unwrap().clone();
    assert_eq!((&failed["status"], failed["reason"].as_str().map(|r| r.contains("dashed lines"))), (&json!("failed"), Some(true)), "{reply}");
    // Past the walk limit the front view has nothing to measure.
    for level in 0..=MAX_PLACEMENT_DEPTH as u64 + 1 {
        nest(&mut s, level);
    }
    assert_eq!(status(&s, front), Some(Status::Placeholder(Unmeasured::PastWalkLimit)));
    assert_eq!(built(&s, sheet).scene.groups.len(), 3);
}

/// The command evaluates no model.
#[test]
fn drawing_a_sheet_evaluates_no_model() {
    let (mut s, sheet, _) = front_and_top();
    let before = s.evaluations;
    run(&mut s, CMD, json!({"sheet": ids::encode(sheet)}));
    assert_eq!(s.evaluations, before);
}

/// An unknown sheet and another drawing's sheet are refused, and so is a sheet with no drawing open.
#[test]
fn unknown_and_other_drawings_sheets_are_refused() {
    let (mut s, sheet, _) = front_and_top();
    let refused = |s: &mut Session, named: &str| s.execute(CMD, &json!({"sheet": named})).unwrap_err().to_string();
    assert!(refused(&mut s, "Sheet 9").contains("no sheet `Sheet 9`"));
    run(&mut s, "drawing.create", json!({"name": "Second"}));
    let first = ids::encode(sheet);
    assert!(refused(&mut s, &first).contains(&format!("no sheet `{first}`")));
    run(&mut s, "drawing.close", json!({}));
    assert!(refused(&mut s, &first).contains("open a drawing first"));
}

/// A view whose lines would take the sheet past a paper scene's records fails and is left out;
/// a view after it that fits is still drawn, up to the limit exactly.
#[test]
fn a_view_past_the_sheets_records_leaves_out_only_itself() {
    let (s, sheet, _) = front_and_top();
    let d = s.drawing("test").unwrap().clone();
    let record = d.sheets.iter().find(|x| x.id == sheet).unwrap();
    // One stroke of `n` pieces: n + 2 records with its group.
    let lines = |n: usize| {
        let path = (0..n).map(|i| Seg2::Line { a: Vec2::new(i as f64 * 1e-3, 0.0), b: Vec2::new((i + 1) as f64 * 1e-3, 0.0) }).collect();
        let items = vec![Item::Stroke { path, closed: false, width_mm: 0.5 }];
        Drawn { group: Some(Group { object: n as u64, kind: ObjectKind::View, items }), status: Status::Drawn, stats: None }
    };
    let left = MAX_RECORDS - border(&s, sheet).iter().map(Group::records).sum::<usize>();
    // The first leaves 20 records; the second needs 32; the third takes the last 20.
    let views = [lines(left - 22), lines(30), lines(18)];
    let centre = Vec2::new(100.0, 100.0);
    let (scene, states) = assemble(record, views.iter().enumerate().map(|(i, v)| (i as u64, centre, v))).unwrap();
    let statuses: Vec<&'static str> = states.iter().map(|v| v.status.name()).collect();
    assert_eq!(statuses, ["drawn", "failed", "drawn"]);
    assert_eq!((scene.groups.len(), scene.groups.iter().map(Group::records).sum::<usize>()), (5, MAX_RECORDS));
    assert!(matches!(&states[1].status, Status::Failed(why) if why.contains("put it on another sheet")), "{:?}", states[1]);
}

/// A view at 200:1 asks the mesh for chords no finer than its floor, so it is drawn rather than
/// refused for a chord it has no setting for.
#[test]
fn a_view_past_100_to_1_is_drawn_with_a_coarser_chord() {
    let (mut s, sheet, [front, _]) = front_and_top();
    run(&mut s, "drawing.view.edit", json!({"view": ids::encode(front), "set": {"scale": "200:1"}}));
    let scene = built(&s, sheet);
    let front = scene.views.iter().find(|v| v.view == front).unwrap();
    assert_eq!(front.status, Status::Drawn, "{front:?}");
}
