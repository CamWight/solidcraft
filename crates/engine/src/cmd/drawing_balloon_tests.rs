use serde_json::json;
use solidcraft_doc::drawing::view::{View, ViewKind, ViewPlace};
use solidcraft_doc::drawing_view::{ViewDirection, ViewOverrides};
use solidcraft_geom::drawing_view::NamedView;

use super::*;
use crate::schema::tests::conforms;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    let reply = s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    let schema = output_schema_for::<BalloonResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply) || !matches!(id, ADD | INSPECT), "{id}: {reply}");
    reply
}

fn refused(s: &mut Session, cmd: &str, p: Value) -> String {
    s.execute(cmd, &p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("accepted: {p}"))
}

fn saved(s: &Session) -> String {
    s.drawing("test").unwrap().to_json().unwrap()
}

/// A component, Bracket, holding a box 80 x 40 x 30 mm along X, Y and Z from the origin, called
/// Box, placed once where it was made; a 10 mm cube of the design's own, Base, at x = 200; and a
/// drawing of them, open, with a front view at 1:1. The session, the view's id and the Bracket's
/// component and occurrence.
fn placed() -> (Session, u64, [u64; 2]) {
    let mut s = Session::default();
    let made = run(&mut s, "component.create", json!({"name": "Bracket"}));
    let bracket = [made["component"].as_u64().unwrap(), made["occurrence"].as_u64().unwrap()];
    run(&mut s, "solid.box", json!({"length": 80, "width": 40, "height": 30, "body_name": "Box"}));
    run(&mut s, "component.activate", json!({"component": "root"}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [200, 0, 0], "operation": "new", "body_name": "Base"}));
    run(&mut s, "drawing.create", json!({}));
    let mut d = (**s.drawing("test").unwrap()).clone();
    let id = d.allocate_id(&mut ids::IdSource::seeded(3874)).unwrap();
    let (sheet, source) = (d.sheets[0].id, d.sources[0].id);
    let kind = ViewKind::Base { source, view: ViewDirection::Named(NamedView::Front) };
    let settings = ViewOverrides { scale: Some("1:1".parse().unwrap()), ..ViewOverrides::default() };
    d.add_views([Arc::new(View { id, sheet, kind, place: ViewPlace::At([200.0, 150.0]), settings, reference_shown_percent: 0.0 })]);
    d.validate().unwrap();
    s.doc_mut().put_drawing(Arc::new(d));
    (s, id, bracket)
}

/// A balloon on the Box's bottom front edge, through the Bracket, at (60, 40) on paper.
fn add(s: &mut Session, view: u64, occurrence: u64) -> Value {
    run(s, ADD, json!({"view": ids::encode(view), "edge": {"point": [40, 0, 0], "path": [occurrence]}, "at": [60, 40]}))
}

fn ends_at(reply: &Value, p: [f64; 3]) {
    let point: Vec<f64> = reply["point"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    assert!(point.iter().zip(p).all(|(a, b)| (a - b).abs() < 1e-9), "{reply}");
}

/// A balloon numbers the part its edge is placed through, saves the edge by name (any point on it
/// picks the same record), and its arrow ends on the edge's middle, whatever point was picked. An
/// add is one undo step.
#[test]
fn a_balloon_points_at_a_placed_part_and_is_saved_by_name() {
    let (mut s, view, [_, bracket]) = placed();
    let before = saved(&s);
    let reply = add(&mut s, view, bracket);
    assert_eq!((reply["status"].as_str(), reply["exact"].as_bool()), (Some("ok"), Some(true)), "{reply}");
    assert_eq!(reply["occurrence"], json!([bracket]));
    assert_eq!(reply["view"], json!(ids::encode(view)));
    assert_eq!(reply["record"]["edge"]["path"], json!([bracket]));
    assert_eq!(reply["record"]["at"], json!([60.0, 40.0]));
    ends_at(&reply, [40.0, 0.0, 0.0]);
    let elsewhere = run(&mut s, ADD, json!({"view": ids::encode(view), "edge": {"point": [7.5, 0, 0], "path": [bracket]}, "at": [0, 0]}));
    assert_eq!(elsewhere["record"]["edge"], reply["record"]["edge"], "saved by name, not by the point");
    ends_at(&elsewhere, [40.0, 0.0, 0.0]);
    run(&mut s, "edit.undo", json!({}));
    let one = saved(&s);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(saved(&s), before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(saved(&s), one);
}

/// What a balloon can't point at is refused with its reason, and the drawing is left as it was: a
/// view the drawing doesn't have, a body of the design itself (no part), an occurrence that isn't
/// there, a point on no edge, and a place off paper.
#[test]
fn a_balloon_is_refused_what_it_cant_point_at() {
    let (mut s, view, [_, bracket]) = placed();
    let before = saved(&s);
    let v = json!(ids::encode(view));
    let nowhere = ids::encode(99);
    let cases = [
        (json!({"view": nowhere, "edge": {"point": [40, 0, 0], "path": [bracket]}, "at": [0, 0]}), format!("the drawing has no view {nowhere}")),
        (json!({"view": v, "edge": [205, 0, 0], "at": [0, 0]}), "it points at a placed part: its edge's body is placed through an occurrence".into()),
        (json!({"view": v, "edge": {"point": [40, 0, 0], "path": [999]}, "at": [0, 0]}), "no edge at [40, 0, 0]".into()),
        (json!({"view": v, "edge": {"point": [40, 20, 0], "path": [bracket]}, "at": [0, 0]}), "no edge at [40, 20, 0]".into()),
        (json!({"view": v, "edge": {"point": [40, 0, 0], "path": [bracket]}, "at": [5000.001, 0]}), "its place is at most 5000 mm on paper".into()),
    ];
    for (p, why) in cases {
        let e = refused(&mut s, ADD, p.clone());
        assert!(e.contains(&why), "{p}: {e}");
    }
    assert_eq!(saved(&s), before);
}

/// An edit moves a balloon and names what changed, as one undo step; the same place again changes
/// nothing and adds no step. What it points at isn't one of its settings.
#[test]
fn an_edit_moves_a_balloon_and_the_same_place_again_changes_nothing() {
    let (mut s, view, [_, bracket]) = placed();
    let id = add(&mut s, view, bracket)["balloon"].clone();
    let before = saved(&s);
    let moved = run(&mut s, EDIT, json!({"balloon": id, "set": {"at": [-12.5, 80]}}));
    assert_eq!(moved, json!({"balloon": id, "changed": ["at"]}));
    assert_eq!(run(&mut s, INSPECT, json!({"balloon": id}))["record"]["at"], json!([-12.5, 80.0]));
    let after = saved(&s);
    for same in [json!({"at": [-12.5, 80]}), json!({})] {
        assert_eq!(run(&mut s, EDIT, json!({"balloon": id, "set": same}))["changed"], json!([]));
    }
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(saved(&s), before, "one step for the move, none for the same place");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(saved(&s), after);
    let e = refused(&mut s, EDIT, json!({"balloon": id, "set": {"edge": [40, 0, 0]}}));
    assert!(e.contains("edge"), "{e}");
    let e = refused(&mut s, EDIT, json!({"balloon": ids::encode(99), "set": {"at": [0, 0]}}));
    assert!(e.contains("the drawing has no balloon"), "{e}");
    assert_eq!(saved(&s), after);
}

/// Inspect says where a balloon stands with the model now: a notch across its edge splits it, so it
/// lists both pieces and has no middle; undone, it is whole again; an occurrence that is gone is
/// lost, said so, and never moved to another part. The model is changed with the drawing closed.
#[test]
fn inspect_says_where_a_balloon_stands_with_the_model() {
    let (mut s, view, [component, bracket]) = placed();
    let drawing = json!({"drawing": ids::encode(s.drawing("test").unwrap().id)});
    let id = add(&mut s, view, bracket)["balloon"].clone();
    run(&mut s, "drawing.close", json!({}));
    run(&mut s, "component.activate", json!({"component": component}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 5, "height": 5, "corner": [35, -1, -1], "operation": "cut", "targets": ["Box"]}));
    run(&mut s, "component.activate", json!({"component": "root"}));
    run(&mut s, "drawing.open", drawing.clone());
    let split = run(&mut s, INSPECT, json!({"balloon": id}));
    assert_eq!(split["status"], "ambiguous", "{split}");
    assert_eq!(split["candidates"].as_array().map(Vec::len), Some(2), "{split}");
    assert!(split.get("point").is_none(), "no one middle: {split}");
    run(&mut s, "drawing.close", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "drawing.open", drawing);
    let whole = run(&mut s, INSPECT, json!({"balloon": id}));
    assert_eq!(whole["status"], "ok", "{whole}");
    ends_at(&whole, [40.0, 0.0, 0.0]);
    let mut d = (**s.drawing("test").unwrap()).clone();
    let b = d.balloons.first_mut().map(Arc::make_mut).unwrap();
    b.edge.path = vec![999];
    s.doc_mut().put_drawing(Arc::new(d));
    let gone = run(&mut s, INSPECT, json!({"balloon": id}));
    assert_eq!((gone["status"].as_str(), gone["occurrence"].clone()), (Some("lost"), json!([999])), "{gone}");
    assert!(gone["why"].as_str().is_some_and(|why| !why.is_empty()) && gone.get("point").is_none(), "{gone}");
    let e = refused(&mut s, INSPECT, json!({"balloon": ids::encode(99)}));
    assert!(e.contains("the drawing has no balloon"), "{e}");
}

/// A balloon put on a piece of a cut edge is found through the piece's numbered name, so not exact.
/// Once the cut is suppressed, the two faces the name names meet in one whole edge again, found
/// exactly, and the arrow moves to its middle.
#[test]
fn a_balloon_on_a_piece_is_not_exact_until_the_edge_is_whole_again() {
    let (mut s, view, [component, bracket]) = placed();
    let drawing = json!({"drawing": ids::encode(s.drawing("test").unwrap().id)});
    run(&mut s, "drawing.close", json!({}));
    run(&mut s, "component.activate", json!({"component": component}));
    let cut = run(
        &mut s,
        "solid.box",
        json!({"length": 10, "width": 5, "height": 5, "corner": [35, -1, -1], "operation": "cut", "targets": ["Box"]}),
    )["feature"]
        .clone();
    run(&mut s, "component.activate", json!({"component": "root"}));
    run(&mut s, "drawing.open", drawing.clone());
    let piece = run(&mut s, ADD, json!({"view": ids::encode(view), "edge": {"point": [10, 0, 0], "path": [bracket]}, "at": [60, 40]}));
    assert_eq!((piece["status"].as_str(), piece["exact"].as_bool()), (Some("ok"), Some(false)), "{piece}");
    ends_at(&piece, [17.5, 0.0, 0.0]);
    run(&mut s, "drawing.close", json!({}));
    run(&mut s, "component.activate", json!({"component": component}));
    run(&mut s, "timeline.suppress", json!({"feature": cut, "suppressed": true}));
    run(&mut s, "component.activate", json!({"component": "root"}));
    run(&mut s, "drawing.open", drawing);
    let whole = run(&mut s, INSPECT, json!({"balloon": piece["balloon"]}));
    assert_eq!((whole["status"].as_str(), whole["exact"].as_bool()), (Some("ok"), Some(true)), "{whole}");
    ends_at(&whole, [40.0, 0.0, 0.0]);
}

/// Every place a value can go in `v`, as JSON pointers: `v` itself, then each member and item.
fn places(v: &Value, at: String, out: &mut Vec<String>) {
    out.push(at.clone());
    match v {
        Value::Object(o) => o.iter().for_each(|(k, x)| places(x, format!("{at}/{k}"), out)),
        Value::Array(l) => l.iter().enumerate().for_each(|(i, x)| places(x, format!("{at}/{i}"), out)),
        _ => {}
    }
}

/// A hostile value in each place of each command's parameters, at every depth (the edge's pick and
/// path, the place, the edit's settings): never a panic, and the drawing keeps its rules.
#[test]
fn hostile_balloon_params_never_panic() {
    let (mut s, view, [_, bracket]) = placed();
    let id = add(&mut s, view, bracket)["balloon"].clone();
    let v = json!(ids::encode(view));
    let calls = [
        (ADD, json!({"view": v, "edge": {"point": [40, 0, 0], "path": [bracket]}, "at": [1, 2]})),
        (ADD, json!({"view": v, "edge": {"body": "Box", "index": 0, "path": [bracket]}, "at": [1, 2]})),
        (EDIT, json!({"balloon": id, "set": {"at": [3, 4]}})),
        (INSPECT, json!({"balloon": id})),
    ];
    let mut values = crate::tests::hostile_values();
    values.extend([json!({"body": "Box", "index": u64::MAX}), Value::Array(vec![json!(1); 1000]), json!([1e308, 0]), json!("x".repeat(600))]);
    let mut internal = Vec::new();
    for (cmd, template) in calls {
        let mut at = Vec::new();
        places(&template, String::new(), &mut at);
        for (place, value) in at.iter().flat_map(|place| values.iter().map(move |value| (place, value))) {
            let mut p = template.clone();
            if let Some(slot) = p.pointer_mut(place) {
                *slot = value.clone();
            }
            let mut copy = s.scratch();
            if let Err(crate::EngineError::Internal(_, msg)) = copy.execute(cmd, &p) {
                internal.push(format!("{cmd} {p}: {msg}"));
            }
            copy.doc.drawings.iter().for_each(|d| d.validate().unwrap());
        }
    }
    assert!(internal.is_empty(), "balloon commands panicked:\n{}", internal.join("\n"));
}

/// A view lists its balloons. A copied view's balloons are copies with ids of their own, pointing
/// at the same part from the same place, and one undo step takes the copy and them out. A sheet
/// with them on it is refused, naming them, until its contents go with it.
#[test]
fn balloons_go_with_their_view_when_it_is_copied_and_with_its_sheet() {
    let (mut s, view, [_, bracket]) = placed();
    let original = add(&mut s, view, bracket);
    let listed = |s: &mut Session, view: &Value| run(s, "drawing.view.inspect", json!({"view": view}))["balloons"].clone();
    assert_eq!(listed(&mut s, &json!(ids::encode(view))), json!([original["balloon"]]));
    let before = saved(&s);
    let sheet = run(&mut s, "drawing.sheet.add", json!({"name": "Sheet 2"}))["sheet"].clone();
    let reply = run(&mut s, "drawing.view.copy_to_sheet", json!({"view": ids::encode(view), "sheet": sheet, "position_mm": [100, 100]}));
    let copy = reply["view"].clone();
    let copied = listed(&mut s, &copy);
    let [balloon] = copied.as_array().map(Vec::as_slice).unwrap_or_default() else { panic!("one copied balloon: {copied}") };
    assert_ne!(balloon, &original["balloon"]);
    let read = run(&mut s, INSPECT, json!({"balloon": balloon}));
    assert_eq!((read["status"].as_str(), &read["view"]), (Some("ok"), &copy), "{read}");
    assert_eq!((&read["record"]["edge"], &read["record"]["at"]), (&original["record"]["edge"], &original["record"]["at"]));
    assert_eq!(listed(&mut s, &json!(ids::encode(view))), json!([original["balloon"]]), "the original keeps its own");
    let e = refused(&mut s, "drawing.sheet.delete", json!({"sheet": sheet}));
    let named = format!("1 view (`{}`) and 1 balloon (`{}`)", copy.as_str().unwrap(), balloon.as_str().unwrap());
    assert!(e.contains(&named), "{e}");
    run(&mut s, "drawing.sheet.delete", json!({"sheet": sheet, "contents": "delete"}));
    let e = refused(&mut s, INSPECT, json!({"balloon": balloon}));
    assert!(e.contains("the drawing has no balloon"), "{e}");
    for _ in 0..3 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(saved(&s), before);
}

/// A parts list on the drawing's first sheet: its id.
fn list(s: &mut Session, set: Value) -> String {
    let sheet = ids::encode(s.drawing("test").unwrap().sheets[0].id);
    let reply = s.execute("drawing.table.add", &json!({"sheet": sheet, "kind": "parts_list", "set": set})).unwrap();
    reply["table"].as_str().unwrap().to_string()
}

fn read(s: &mut Session, balloon: &str) -> Value {
    run(s, INSPECT, json!({"balloon": balloon}))
}

/// The balloon's group in a sheet's reply, and the reply's word on it.
fn on_sheet(s: &mut Session, balloon: &str) -> (Value, Value) {
    let sheet = ids::encode(s.drawing("test").unwrap().sheets[0].id);
    let reply = s.execute("drawing.scene", &json!({"sheet": sheet})).unwrap();
    let schema = output_schema_for::<crate::schema::drawing_scene::SheetSceneResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    let wanted = ids::decode(balloon).unwrap();
    let group = reply["scene"]["groups"].as_array().unwrap().iter().find(|g| g["object"] == json!(wanted)).cloned();
    let word = reply["balloons"].as_array().unwrap().iter().find(|b| b["balloon"] == json!(balloon)).cloned();
    (group.unwrap_or(Value::Null), word.unwrap())
}

fn items_of<'a>(group: &'a Value, kind: &str) -> Vec<&'a Value> {
    group["items"].as_array().unwrap().iter().filter(|item| item["kind"] == json!(kind)).collect()
}

/// A balloon draws on its sheet after its view: a circle at its place from the view's centre, and a
/// leader to its edge's middle, worked out by hand. The view is centred on what it shows, 0 to 210
/// along X and 0 to 30 up, so at (200, 150) the middle of the bottom edge, (40, 0, 0), is at (135, 135).
/// Its number comes with a list, follows the list, and goes when the list does; moving the view
/// moves it all; an export writes it and says where it stands.
#[test]
fn a_balloon_draws_on_its_sheet_with_its_number_and_its_leader() {
    let (mut s, view, [_, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();

    let (group, word) = on_sheet(&mut s, &balloon);
    assert_eq!((group["kind"].as_str(), word["drawn"].as_bool(), word["status"].as_str()), (Some("balloon"), Some(true), Some("ok")), "{word}");
    assert!(word["why"].as_str().is_some_and(|why| why.contains("no parts list")), "{word}");
    assert!(items_of(&group, "text").is_empty(), "no number yet: {group}");
    let circle = items_of(&group, "stroke").into_iter().find(|item| item["closed"] == json!(true)).unwrap().clone();
    assert_eq!(circle["path"][0]["center"], json!([260.0, 190.0]), "{circle}");
    assert_eq!(circle["path"][0]["radius"], json!(5.0), "{circle}");
    let tip = |group: &Value| items_of(group, "fill")[0]["region"]["outer"]["segs"][0]["a"].clone();
    let at = tip(&group);
    assert!((at[0].as_f64().unwrap() - 135.0).abs() < 1e-9 && (at[1].as_f64().unwrap() - 135.0).abs() < 1e-9, "{at}");

    let table = list(&mut s, json!({}));
    let (group, word) = on_sheet(&mut s, &balloon);
    assert_eq!(items_of(&group, "text")[0]["text"], json!("1"), "{group}");
    assert!(word.get("why").is_none(), "{word}");

    let exported = s.execute("drawing.export", &json!({"sheet": ids::encode(s.drawing("test").unwrap().sheets[0].id)})).unwrap();
    assert_eq!(exported["balloons"][0]["drawn"], json!(true), "{exported}");
    assert!(exported["svg"].as_str().is_some_and(|svg| svg.contains("BALLOON") || svg.contains("balloon")), "its layer in the file");

    s.execute("drawing.table.delete", &json!({"table": table})).unwrap();
    let (group, word) = on_sheet(&mut s, &balloon);
    assert!(items_of(&group, "text").is_empty() && word["why"].as_str().is_some_and(|why| why.contains("no parts list")), "{word}");
}

/// A balloon whose edge is gone stays where it is, with its circle and its number and no leader, and
/// says it is lost; put back, its leader returns. One in a scene it can't fit says so too.
#[test]
fn a_lost_balloon_keeps_its_circle_and_number() {
    let (mut s, view, [_, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();
    list(&mut s, json!({}));
    assert_eq!(items_of(&on_sheet(&mut s, &balloon).0, "fill").len(), 1, "its arrowhead");
    let mut d = (**s.drawing("test").unwrap()).clone();
    let was = d.clone();
    Arc::make_mut(d.balloons.first_mut().unwrap()).edge.name = "gone".into();
    s.doc_mut().put_drawing(Arc::new(d));
    let (group, word) = on_sheet(&mut s, &balloon);
    assert_eq!(word["status"], json!("lost"), "{word}");
    assert_eq!(word["drawn"], json!(true), "{word}");
    assert!(items_of(&group, "fill").is_empty(), "no arrowhead: {group}");
    assert_eq!(items_of(&group, "stroke").len(), 1, "only its circle: {group}");
    assert_eq!(items_of(&group, "text")[0]["text"], json!("1"), "{group}");
    s.doc_mut().put_drawing(Arc::new(was));
    assert_eq!(items_of(&on_sheet(&mut s, &balloon).0, "fill").len(), 1, "back");
}

/// The provider draws a balloon as a build does, and reads it again only when it or what it reads
/// changes: an edit moves it, a new list gives it a number, and a step with nothing changed reads
/// nothing.
#[test]
fn the_provider_keeps_a_balloon_until_what_it_reads_changes() {
    let (mut s, view, [_, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();
    let sheet = s.drawing("test").unwrap().sheets[0].id;
    let settled = |s: &mut Session| {
        crate::drawing_scene::extract(s, sheet).unwrap();
        while crate::drawing_scene::step(s) {}
        crate::drawing_scene::extract(s, sheet).unwrap()
    };
    let first = settled(&mut s);
    let d = s.drawing("test").unwrap().clone();
    assert_eq!(*first.scene, crate::drawing_scene::build(&s, &d, sheet).unwrap().scene, "as a build draws it");
    assert!(first.stale_annotations.is_empty());
    let again = settled(&mut s);
    assert_eq!(again.generation, first.generation, "nothing read again");

    let table = list(&mut s, json!({}));
    let numbered = crate::drawing_scene::extract(&mut s, sheet).unwrap();
    assert_eq!(numbered.stale_annotations, [ids::decode(&table).unwrap(), ids::decode(&balloon).unwrap()], "the list, and what it numbers");
    let numbered = settled(&mut s);
    let d = s.drawing("test").unwrap().clone();
    assert_eq!(*numbered.scene, crate::drawing_scene::build(&s, &d, sheet).unwrap().scene);
    assert_ne!(numbered.generation, first.generation);

    run(&mut s, EDIT, json!({"balloon": balloon, "set": {"at": [70, 50]}}));
    let moved = settled(&mut s);
    assert_ne!(moved.generation, numbered.generation);
    let d = s.drawing("test").unwrap().clone();
    assert_eq!(*moved.scene, crate::drawing_scene::build(&s, &d, sheet).unwrap().scene);
}

/// A balloon and the list it reads give one item, from one rule: a table's row and the balloon's number
/// are the same text. The row carries what the model says of the part, listed once.
#[test]
fn a_balloon_and_its_lists_row_read_one_item() {
    let (mut s, view, [_, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();
    list(&mut s, json!({}));
    let d = s.drawing("test").unwrap().clone();
    let Some(solidcraft_doc::drawing::table::Kind::PartsList(parts)) = d.tables.first().map(|t| t.kind.clone()) else { panic!("a parts list") };
    let from = s.drawn(&d, parts.of.source).unwrap();
    let state = from.model.state();
    let mut tree = solidcraft_doc::bom::Tree::new(&from.doc, &state);
    let listed = solidcraft_doc::bom::list(&mut tree, &parts.request()).unwrap();
    let rows = parts.rows(&listed, from.doc.drawing_units(&d));
    let bracket = rows.iter().find(|row| row.fields.get("name").is_some_and(|name| name == "Bracket")).unwrap();
    assert_eq!(bracket.fields.get("quantity").map(String::as_str), Some("1"));
    assert_eq!(bracket.kind, solidcraft_geom::table::RowKind::Part);
    let number = read(&mut s, &balloon)["item"].clone();
    assert_eq!(number, json!(bracket.fields["item"]), "{bracket:?}");
}

/// A table's inspect names the balloons on each row, for what cross-highlights them.
#[test]
fn a_tables_inspect_names_its_rows_balloons() {
    let (mut s, view, [_, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();
    let table = list(&mut s, json!({}));
    let reply = s.execute("drawing.table.inspect", &json!({"table": table})).unwrap();
    let rows = reply["rows"].as_array().unwrap();
    let on: Vec<&Value> = rows.iter().filter(|row| row.get("balloons").is_some()).collect();
    assert_eq!(on.len(), 1, "{reply}");
    assert_eq!(on[0]["balloons"], json!([balloon]));
}

/// A balloon's number comes from its list, which reads more of the design than the model: a part's
/// properties send the balloon to be read again, and a change of the drawing alone doesn't.
#[test]
fn a_balloon_is_read_again_when_its_lists_design_changes() {
    let (mut s, view, [_, occurrence]) = placed();
    let balloon = ids::decode(add(&mut s, view, occurrence)["balloon"].as_str().unwrap()).unwrap();
    let table = list(&mut s, json!({}));
    let sheet = s.drawing("test").unwrap().sheets[0].id;
    crate::drawing_scene::extract(&mut s, sheet).unwrap();
    while crate::drawing_scene::step(&mut s) {}
    assert!(crate::drawing_scene::extract(&mut s, sheet).unwrap().stale_annotations.is_empty());

    run(&mut s, "drawing.table.edit", json!({"table": table, "set": {"cap_mm": 2.5}}));
    let stale = crate::drawing_scene::extract(&mut s, sheet).unwrap().stale_annotations;
    assert!(!stale.contains(&balloon), "the list's look reads nothing new");
    while crate::drawing_scene::step(&mut s) {}

    let set = json!({"command": "component.metadata.set", "params": {"component": "Bracket", "part_number": "B-1"}});
    run(&mut s, "drawing.source.execute", set);
    assert!(crate::drawing_scene::extract(&mut s, sheet).unwrap().stale_annotations.contains(&balloon));
}

/// A balloon with no list says so, and reads the only one when there is one: the row its part is
/// counted in. Two lists leave it to choose, with the choices listed, until it names one. A list's
/// delete reports the balloon that named it and leaves the balloon. One undo step each.
#[test]
fn a_balloon_reads_its_row_in_a_parts_list() {
    let (mut s, view, [_, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();
    let alone = read(&mut s, &balloon);
    assert_eq!((&alone["row_status"], alone.get("item"), alone.get("parts_list")), (&json!("no_list"), None, None), "{alone}");

    let first = list(&mut s, json!({}));
    let read1 = read(&mut s, &balloon);
    assert_eq!((&read1["row_status"], &read1["item"], &read1["parts_list"]), (&json!("ok"), &json!("1"), &json!(first)), "{read1}");
    assert!(s.drawing("test").unwrap().balloons.by_id(ids::decode(&balloon).unwrap()).unwrap().parts_list.is_none(), "worked out, not saved");

    let second = list(&mut s, json!({"list": {"levels": "top"}}));
    let both = read(&mut s, &balloon);
    assert_eq!(both["row_status"], json!("no_list"), "{both}");
    assert_eq!(both["choices"].as_array().map(Vec::len), Some(2));

    let steps = s.undo.len();
    let named = run(&mut s, EDIT, json!({"balloon": balloon, "set": {"parts_list": second}}));
    assert_eq!((&named["changed"], s.undo.len()), (&json!(["parts_list"]), steps + 1), "{named}");
    assert_eq!(read(&mut s, &balloon)["parts_list"], json!(second));
    run(&mut s, EDIT, json!({"balloon": balloon, "set": {"parts_list": second}}));
    assert_eq!(s.undo.len(), steps + 1, "nothing changed, no step");
    assert!(refused(&mut s, EDIT, json!({"balloon": balloon, "set": {"parts_list": ids::encode(99)}})).contains("parts lists"));

    let gone = s.execute("drawing.table.delete", &json!({"table": second})).unwrap();
    assert_eq!(gone["balloons"], json!([balloon]), "{gone}");
    assert_eq!(read(&mut s, &balloon)["parts_list"], json!(first), "the only one left");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(read(&mut s, &balloon)["parts_list"], json!(second), "the undo brings its link back");
    run(&mut s, EDIT, json!({"balloon": balloon, "set": {"parts_list": null}}));
    assert_eq!(read(&mut s, &balloon)["row_status"], json!("no_list"));

    let on_add = run(
        &mut s,
        ADD,
        json!({"view": ids::encode(view), "edge": {"point": [40, 0, 0], "path": [occurrence]}, "at": [60, 40], "parts_list": first}),
    );
    assert_eq!(on_add["parts_list"], json!(first), "{on_add}");
    refused(&mut s, ADD, json!({"view": ids::encode(view), "edge": {"point": [40, 0, 0], "path": [occurrence]}, "at": [60, 40], "parts_list": "x"}));
}

/// A balloon follows its row: renumbered, it shows the new number. What the model does to the row
/// says what became of it, and never removes the balloon; undoing brings the number back.
#[test]
fn a_balloon_follows_its_row_through_the_model() {
    let (mut s, view, [component, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();
    list(&mut s, json!({}));
    assert_eq!(read(&mut s, &balloon)["item"], json!("1"));
    let drawing = ids::encode(s.open_drawing.unwrap());
    let with_drawing_closed = |s: &mut Session, id: &str, p: Value| {
        s.execute("drawing.close", &json!({})).unwrap();
        s.execute(id, &p).unwrap();
        s.execute("drawing.open", &json!({"drawing": drawing})).unwrap();
    };

    with_drawing_closed(&mut s, "assembly.bom.renumber", json!({"levels": "flat", "start": 7}));
    assert_eq!(read(&mut s, &balloon)["item"], json!("7"), "renumbered");

    with_drawing_closed(&mut s, "occurrence.bom_type", json!({"occurrence": occurrence, "bom_type": "reference"}));
    let reference = read(&mut s, &balloon);
    assert_eq!((&reference["row_status"], reference.get("item")), (&json!("reference"), None), "{reference}");
    assert!(reference["warnings"][0].as_str().is_some_and(|w| w.contains("reference")), "{reference}");
    with_drawing_closed(&mut s, "edit.undo", json!({}));
    assert_eq!(read(&mut s, &balloon)["item"], json!("7"), "undone");

    with_drawing_closed(&mut s, "occurrence.bom_type", json!({"occurrence": occurrence, "bom_type": "phantom"}));
    assert_eq!(read(&mut s, &balloon)["row_status"], json!("ok"), "a phantom's own bodies are a row of their own");
    with_drawing_closed(&mut s, "edit.undo", json!({}));

    with_drawing_closed(&mut s, "component.delete", json!({"component": component}));
    let deleted = read(&mut s, &balloon);
    assert_eq!(deleted["row_status"], json!("not_found"), "{deleted}");
    assert_eq!(s.drawing("test").unwrap().balloons.len(), 1, "the balloon stays");
    with_drawing_closed(&mut s, "edit.undo", json!({}));
    assert_eq!(read(&mut s, &balloon)["item"], json!("7"), "the part back, its number back");
}

/// A list of the component the balloon's part is in reads it below that component; a list of the
/// component a balloon's path doesn't run through, or one deleted since, finds nothing.
#[test]
fn a_balloon_reads_a_components_list_below_it() {
    let (mut s, view, [component, occurrence]) = placed();
    let balloon = add(&mut s, view, occurrence)["balloon"].as_str().unwrap().to_string();
    list(&mut s, json!({"list": {"component": component}}));
    assert_eq!(read(&mut s, &balloon)["row_status"], json!("no_row"), "the assembly listed has no row");
    let drawing = ids::encode(s.open_drawing.unwrap());
    s.execute("drawing.close", &json!({})).unwrap();
    s.execute("component.delete", &json!({"component": component})).unwrap();
    s.execute("drawing.open", &json!({"drawing": drawing})).unwrap();
    assert_eq!(read(&mut s, &balloon)["row_status"], json!("not_found"));
}
