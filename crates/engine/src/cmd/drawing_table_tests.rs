use serde_json::json;

use super::*;
use crate::schema::tests::conforms;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    let reply = s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    let schema = output_schema_for::<TableResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply) || id == DELETE, "{id}: {reply}");
    reply
}

/// `id` refuses `p`, leaving the drawing and the undo steps as they were: why.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let (record, steps) = (s.drawing("test").ok().cloned(), s.undo.len());
    let message = s.execute(id, &p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("{id} accepted {p}"));
    assert_eq!((s.drawing("test").ok().cloned(), s.undo.len()), (record, steps), "{id} {p}");
    message
}

/// A table can be selected and deleted as what else is on a sheet is, in one undo step.
#[test]
fn a_table_is_selected_and_deleted_as_other_objects_are() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let table = run(&mut s, ADD, json!({"sheet": sheet, "kind": "revision"}))["table"].clone();
    let item = json!({"type": "drawing", "drawing": ids::encode(s.drawing("test").unwrap().id), "object": table});
    s.execute("select.set", &json!({"items": [item]})).unwrap();
    let gone = s.execute("selection.delete", &json!({"items": [item]})).unwrap();
    assert_eq!(gone["tables"], json!([table]));
    assert!(s.drawing("test").unwrap().tables.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.drawing("test").unwrap().tables.len(), 1);
}

/// A plate and a bolt placed twice in the design, its Bolt inside a component, and a drawing, open.
fn assembly() -> Session {
    let mut s = Session::default();
    for (name, part) in [("Plate", "P-100"), ("Bolt", "B-200")] {
        s.execute("component.create", &json!({"name": name})).unwrap();
        s.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10, "body_name": name})).unwrap();
        s.execute("component.activate", &json!({"component": "root"})).unwrap();
        s.execute("component.metadata.set", &json!({"component": name, "part_number": part})).unwrap();
    }
    s.execute("occurrence.copy", &json!({"component": "Bolt", "translate": [0, 50, 0]})).unwrap();
    s.execute("drawing.create", &json!({})).unwrap();
    s
}

fn sheet_of(s: &Session) -> String {
    ids::encode(s.drawing("test").unwrap().sheets[0].id)
}

fn id_of(reply: &Value) -> u64 {
    ids::decode(reply["table"].as_str().unwrap()).unwrap()
}

/// What a sheet's reply says of table `id`, and the box round its lines.
fn on_sheet(s: &mut Session, id: u64) -> (Value, [f64; 4]) {
    let reply = s.execute("drawing.scene", &json!({"sheet": sheet_of(s)})).unwrap();
    let schema = output_schema_for::<crate::schema::drawing_scene::SheetSceneResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    let word = reply["tables"].as_array().unwrap().iter().find(|t| t["table"] == json!(ids::encode(id))).cloned().unwrap();
    let group = reply["scene"]["groups"].as_array().unwrap().iter().find(|g| g["object"] == json!(id));
    let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for point in
        group.into_iter().flat_map(|g| g["items"].as_array().unwrap()).filter(|i| i["kind"] == "stroke").flat_map(|i| i["path"].as_array().unwrap())
    {
        for end in ["a", "b"] {
            let (x, y) = (point[end][0].as_f64().unwrap(), point[end][1].as_f64().unwrap());
            bounds = [bounds[0].min(x), bounds[1].min(y), bounds[2].max(x), bounds[3].max(y)];
        }
    }
    (word, bounds)
}

fn inspected(s: &mut Session, id: u64) -> Value {
    let reply = s.execute(INSPECT, &json!({"table": ids::encode(id)})).unwrap();
    let schema = output_schema_for::<InspectResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    reply
}

/// A drawing space for the paper: the corners the tables hang from, from the record's own layout.
fn space(s: &Session) -> solidcraft_geom::Aabb2 {
    let d = s.drawing("test").unwrap();
    crate::drawing_table_paper::drawing_space(&d.sheets[0])
}

/// A parts list draws at the drawing space's top right, its top two rows and heading 7 mm each by the
/// defaults; a revision table at the bottom left. Each is one kind of object in the scene, says where
/// it stands, and an export says the same.
#[test]
fn a_table_draws_where_its_record_says() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let list = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"})));
    let revisions = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "revision"})));
    let drawing = space(&s);

    let (word, [x0, y0, x1, y1]) = on_sheet(&mut s, list);
    assert_eq!((word["status"].as_str(), word["drawn"].as_bool()), (Some("ok"), Some(true)), "{word}");
    assert!((x1 - drawing.max.x).abs() < 1e-9 && (y1 - drawing.max.y).abs() < 1e-9, "{x1} {y1} {drawing:?}");
    assert!((y1 - y0 - 3.0 * 7.0).abs() < 1e-9, "a heading and two rows: {y0} {y1}");
    assert!(x1 - x0 > 0.0);

    let (word, [x0, y0, ..]) = on_sheet(&mut s, revisions);
    assert_eq!(word["drawn"], json!(true), "{word}");
    assert!((x0 - drawing.min.x).abs() < 1e-9 && (y0 - drawing.min.y).abs() < 1e-9, "{x0} {y0} {drawing:?}");

    let exported = s.execute("drawing.export", &json!({"sheet": sheet})).unwrap();
    assert_eq!(exported["tables"].as_array().map(Vec::len), Some(2), "{exported}");
    assert!(exported["svg"].as_str().is_some_and(|svg| svg.contains("PARTS_LIST") || svg.contains("parts_list")), "its layer in the file");
}

/// A change of paper takes an anchored table along and leaves a free one where it was. An offset moves
/// it, a heading at the bottom keeps the same corner of the outline, and a free place names its top
/// left. The bytes are the same on a second draw.
#[test]
fn a_tables_place_follows_the_paper_and_its_record() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let table = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"})));
    let free = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list", "set": {"place": {"at": [30, 200]}}})));
    let (_, [_, _, x1, y1]) = on_sheet(&mut s, table);
    let was_free = on_sheet(&mut s, free).1;
    assert!((was_free[0] - 30.0).abs() < 1e-9 && (was_free[3] - 200.0).abs() < 1e-9, "{was_free:?}");
    let first = s.execute("drawing.scene", &json!({"sheet": sheet})).unwrap();
    assert_eq!(first, s.execute("drawing.scene", &json!({"sheet": sheet})).unwrap(), "the same bytes");

    run(&mut s, EDIT, json!({"table": ids::encode(table), "set": {"place": {"anchor": {"corner": "top_right", "offset_mm": [-5, -7]}}}}));
    let (_, [_, _, moved_x, moved_y]) = on_sheet(&mut s, table);
    assert!((moved_x - (x1 - 5.0)).abs() < 1e-9 && (moved_y - (y1 - 7.0)).abs() < 1e-9, "{moved_x} {moved_y}");

    run(&mut s, EDIT, json!({"table": ids::encode(table), "set": {"heading": "bottom"}}));
    let (_, [_, _, bottom_x, bottom_y]) = on_sheet(&mut s, table);
    assert!((bottom_x - moved_x).abs() < 1e-9 && (bottom_y - moved_y).abs() < 1e-9, "the same corner: {bottom_x} {bottom_y}");

    let wider = s.drawing("test").unwrap().sheets[0].sheet.size_mm().unwrap();
    let mut d = (**s.drawing("test").unwrap()).clone();
    Arc::make_mut(&mut d.sheets[0]).sheet = solidcraft_geom::drawing_sheet::Sheet::standard(
        solidcraft_geom::drawing_sheet::PaperSize::A2,
        solidcraft_geom::drawing_sheet::Orientation::Landscape,
    );
    s.doc_mut().put_drawing(Arc::new(d));
    let (_, [_, _, on_a2, _]) = on_sheet(&mut s, table);
    let grown = s.drawing("test").unwrap().sheets[0].sheet.size_mm().unwrap().x - wider.x;
    assert!((on_a2 - (bottom_x + grown)).abs() < 1.0, "it follows the paper: {on_a2} {bottom_x} {grown}");
    assert_eq!(on_sheet(&mut s, free).1, was_free, "a free one stays");
}

/// One that can't be laid out says why and draws nothing; the rest of the sheet still draws. Here the
/// text it would hold is past a table's 1 MiB: 300 revision rows, each with a 4096 byte column.
#[test]
fn a_table_that_cant_be_laid_out_says_why_and_the_sheet_still_draws() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let good = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"})));
    let bad = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "revision"})));
    let mut d = (**s.drawing("test").unwrap()).clone();
    let revisions = Arc::make_mut(&mut d.revisions);
    for row in 0..300 {
        revisions.add(None, 1_000_000 + row).unwrap();
    }
    let long = solidcraft_geom::cell::Content(vec![solidcraft_geom::cell::Part::Text("x".repeat(4096))]);
    let at = d.tables.index_of(bad).unwrap();
    Arc::make_mut(d.tables.get_mut(at).unwrap()).table.columns[0].content = long;
    s.doc_mut().put_drawing(Arc::new(d));
    let (word, _) = on_sheet(&mut s, bad);
    assert_eq!((word["status"].as_str(), word["drawn"].as_bool()), (Some("failed"), Some(false)), "{word}");
    assert!(word["why"].as_str().is_some_and(|why| why.contains("1 MiB")), "{word}");
    assert_eq!(on_sheet(&mut s, good).0["drawn"], json!(true));
    let reply = inspected(&mut s, bad);
    assert_eq!(reply["status"], "failed");
    assert!(reply["why"].as_str().is_some_and(|why| why.contains("1 MiB")), "{reply}");
    assert!(reply.get("outline_mm").is_none());
}

/// A table's words stand in its cells: each line on its baseline, its left, middle or right at the anchor
/// the layout gives as its column aligns. The sheet carries their fonts, and each word is in the export.
#[test]
fn a_tables_words_stand_where_their_cells_say() {
    use solidcraft_geom::cell::HAlign;
    use solidcraft_geom::paper::Item;

    let mut s = assembly();
    let sheet = s.drawing("test").unwrap().sheets[0].id;
    let table = id_of(&run(&mut s, ADD, json!({"sheet": ids::encode(sheet), "kind": "parts_list"})));
    let mut words = 0;
    for (align, along) in [(HAlign::Left, 0.0), (HAlign::Centre, 0.5), (HAlign::Right, 1.0)] {
        let mut d = (**s.drawing("test").unwrap()).clone();
        let at = d.tables.index_of(table).unwrap();
        Arc::make_mut(d.tables.get_mut(at).unwrap()).table.columns.iter_mut().for_each(|column| column.align = align);
        s.doc_mut().put_drawing(Arc::new(d));
        let d = s.drawing("test").unwrap().clone();
        let scene = crate::drawing_scene::build(&s, &d, sheet).unwrap().scene;
        let space = crate::drawing_table_paper::drawing_space(&d.sheets[0]);
        let placed = crate::drawing_table_paper::place(&s, &d, &d.tables[0], space).unwrap();
        let group = scene.groups.iter().find(|g| g.object == table).unwrap();
        let mut runs: Vec<_> = group.items.iter().filter_map(|item| if let Item::Text(run) = item { Some(run) } else { None }).collect();
        let lines: Vec<_> =
            std::iter::once(&placed.layout.heading).chain(&placed.layout.rows).flat_map(|row| &row.cells).flat_map(|c| &c.lines).collect();
        assert!(lines.iter().any(|line| line.text == "Bolt"), "{lines:?}");
        assert_eq!(lines.len(), runs.len(), "{align:?}");
        words = lines.len();
        for line in lines {
            let at = runs.iter().position(|run| run.text == line.text && (run.origin.y - line.at.y).abs() < 1e-9).unwrap();
            let run = runs.swap_remove(at);
            assert!((run.origin.x + run.reach() * along - line.at.x).abs() < 1e-9, "{align:?} {line:?} {:?}", run.origin);
            assert_eq!(run.angle, 0.0);
        }
        assert!(!scene.fonts.is_empty());
        assert_eq!(scene, crate::drawing_scene::build(&s, &d, sheet).unwrap().scene, "the same bytes on a redraw");
    }
    let exported = s.execute("drawing.export", &json!({"sheet": ids::encode(sheet)})).unwrap();
    let svg = exported["svg"].as_str().unwrap();
    let group = svg.split(&format!("data-object=\"{table}\"")).nth(1).unwrap().split("</g>").next().unwrap();
    assert!(group.matches("<path fill=\"#000\" stroke=\"none\"").count() >= words, "its words are in the file: {group}");
}

/// The provider draws a table as a build does, keeps it until what it reads changes, and draws it again
/// when the table, the model or its revision rows do.
#[test]
fn the_provider_keeps_a_table_until_what_it_reads_changes() {
    let mut s = assembly();
    let sheet_id = s.drawing("test").unwrap().sheets[0].id;
    let sheet = ids::encode(sheet_id);
    let table = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"})));
    let settled = |s: &mut Session| {
        crate::drawing_scene::extract(s, sheet_id).unwrap();
        while crate::drawing_scene::step(s) {}
        crate::drawing_scene::extract(s, sheet_id).unwrap()
    };
    let same_as_a_build = |s: &Session, scene: &crate::drawing_scene::SheetView| {
        let d = s.drawing("test").unwrap().clone();
        assert_eq!(*scene.scene, crate::drawing_scene::build(s, &d, sheet_id).unwrap().scene);
    };
    let first = settled(&mut s);
    same_as_a_build(&s, &first);
    assert!(first.scene.groups.iter().any(|g| g.object == table));
    assert_eq!(settled(&mut s).generation, first.generation, "nothing read again");

    run(&mut s, EDIT, json!({"table": ids::encode(table), "set": {"cap_mm": 2.5}}));
    let waiting = crate::drawing_scene::extract(&mut s, sheet_id).unwrap();
    assert_eq!(waiting.stale_annotations, [table], "the table to read");
    let edited = settled(&mut s);
    assert_ne!(edited.generation, first.generation);
    same_as_a_build(&s, &edited);

    s.execute(
        "drawing.source.execute",
        &json!({"command": "component.metadata.set", "params": {"component": "Bolt", "part_number": "B-200-LONG-NAME"}}),
    )
    .unwrap();
    let renamed = settled(&mut s);
    assert_ne!(renamed.generation, edited.generation, "a part's number names nothing the model holds");
    same_as_a_build(&s, &renamed);

    let revisions = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "revision"})));
    let waiting = crate::drawing_scene::extract(&mut s, sheet_id).unwrap();
    assert_eq!(waiting.stale_annotations, [revisions], "a change of the drawing alone leaves the list as it read");
}

/// Inspect gives what a table placed shows: its outline, each row with its key and rectangle, a cell an
/// override changed with the value it replaced, a filter that leaves nothing, and a word wider than its
/// column. It changes nothing.
#[test]
fn inspect_says_where_a_table_is_and_what_it_shows() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let table = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"})));
    let before = s.drawing("test").unwrap().to_json().unwrap();
    let steps = s.undo.len();
    let reply = inspected(&mut s, table);
    assert_eq!((reply["status"].as_str(), reply["listed"].as_u64()), (Some("ok"), Some(2)), "{reply}");
    assert_eq!(s.drawing("test").unwrap().to_json().unwrap(), before);
    assert_eq!(s.undo.len(), steps);
    let bounds = on_sheet(&mut s, table).1;
    let outline: Vec<f64> = reply["outline_mm"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    assert!(outline.iter().zip(bounds).all(|(a, b)| (a - b).abs() < 1e-9), "{outline:?} {bounds:?}");
    let rows = reply["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let rect = |row: &Value, at: usize| row["rect_mm"][at].as_f64().unwrap();
    assert!((rect(&rows[0], 3) - (outline[3] - 7.0)).abs() < 1e-9 && (rect(&rows[1], 3) - (outline[3] - 14.0)).abs() < 1e-9, "{reply}");

    let key = rows[0]["key"].as_str().unwrap().to_string();
    let override_ = json!([{"row": key, "column": "name", "text": "By hand"}]);
    run(&mut s, EDIT, json!({"table": ids::encode(table), "set": {"overrides": override_}}));
    let reply = inspected(&mut s, table);
    assert_eq!(reply["overrides"][0], json!({"row": key, "column": "name", "text": "By hand", "replaced": "Bolt"}), "{reply}");

    let filter = json!({"text": [{"column": "name", "contains": "no such part"}]});
    run(&mut s, EDIT, json!({"table": ids::encode(table), "set": {"filter": filter, "overrides": []}}));
    let reply = inspected(&mut s, table);
    assert_eq!((reply["listed"].as_u64(), reply.get("rows")), (Some(2), None), "the filter leaves nothing: {reply}");

    let mut columns = s.drawing("test").unwrap().tables.by_id(table).unwrap().table.columns.clone();
    columns[2].width_mm = Some(5.0);
    let columns = serde_json::to_value(&columns).unwrap();
    run(&mut s, EDIT, json!({"table": ids::encode(table), "set": {"filter": {}, "columns": columns}}));
    let reply = inspected(&mut s, table);
    assert!(reply["warnings"].as_array().unwrap().iter().any(|w| w["column"] == "name" && w["warning"] == "width"), "{reply}");
}

/// The default mass column reads a weighed list, in its list's unit: a 10 mm cube of the default
/// material weighs its volume times its density. Without one, the list isn't weighed and shows none.
#[test]
fn a_mass_column_reads_a_weighed_list() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let table = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list", "set": {"list": {"mass": {"unit": "g", "decimals": 3}}}})));
    let d = s.drawing("test").unwrap().clone();
    let record = d.tables.by_id(table).unwrap();
    let placed = crate::drawing_table_paper::place(&s, &d, record, space(&s)).unwrap();
    let mass = placed.rows[0].fields.get("mass").unwrap();
    assert!(mass.parse::<f64>().unwrap() > 0.0 && mass.split('.').nth(1).map(str::len) == Some(3), "{mass}");

    let mut columns = record.table.columns.clone();
    columns.retain(|column| column.name != "mass");
    run(&mut s, EDIT, json!({"table": ids::encode(table), "set": {"columns": serde_json::to_value(&columns).unwrap()}}));
    let d = s.drawing("test").unwrap().clone();
    let placed = crate::drawing_table_paper::place(&s, &d, d.tables.by_id(table).unwrap(), space(&s)).unwrap();
    assert_eq!(placed.rows[0].fields.get("mass"), None, "not weighed");
}

/// The new parts list starts as the plan says: top right, headings on top, QTY, rows of 7 mm, listing
/// the design. A revision table starts at the bottom left. Each is one undo step, and reads back.
#[test]
fn a_new_table_has_its_defaults_and_saves() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let steps = s.undo.len();
    let list = run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"}));
    assert_eq!(s.undo.len(), steps + 1);
    let record = &list["record"];
    assert_eq!(record["place"], json!({"anchor": {"corner": "top_right", "offset_mm": [0.0, 0.0]}}), "{record}");
    assert_eq!((&record["table"]["heading"], &record["table"]["row_height_mm"]), (&json!("top"), &json!(7.0)));
    assert_eq!(record["table"]["columns"][1]["heading"], json!("QTY"));
    assert_eq!(record["kind"]["parts_list"]["of"]["component"], json!(0));
    assert_eq!(record["kind"]["parts_list"]["mass"], json!({"unit": "kg", "decimals": 2}));
    let revisions = run(&mut s, ADD, json!({"sheet": "Sheet 1", "kind": "revision"}));
    assert_eq!(revisions["record"]["place"]["anchor"]["corner"], json!("bottom_left"), "{revisions}");
    assert_eq!(revisions["record"]["kind"], json!("revision"));
    assert_eq!(revisions.get("warnings"), None);

    let d = s.drawing("test").unwrap().clone();
    assert_eq!(d.tables.len(), 2);
    assert_eq!(Drawing::from_json(&d.to_json().unwrap()).unwrap(), *d);
    let listed = s.execute("drawing.inspect", &json!({})).unwrap();
    assert_eq!(listed["sheets"][0]["tables"].as_array().map(Vec::len), Some(2), "{listed}");

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.drawing("test").unwrap().tables.len(), 1);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(*s.drawing("test").unwrap().clone(), *d);
}

/// A list's sheet documents the component it starts with.
#[test]
fn a_new_list_lists_what_its_sheet_documents() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let bolt = s.doc.find_component("Bolt").unwrap();
    let source = ids::encode(s.drawing("test").unwrap().sources[0].id);
    s.execute("drawing.sheet.set", &json!({"sheet": sheet, "documents": {"source": source, "component": bolt}})).unwrap();
    let list = run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"}));
    assert_eq!(list["record"]["kind"]["parts_list"]["of"]["component"], json!(bolt), "{list}");
    let named = run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list", "set": {"list": {"component": "root"}}}));
    assert_eq!(named["record"]["kind"]["parts_list"]["of"]["component"], json!(0), "{named}");
}

/// Placing a list saves what its view shows, once: the same numbers, one numbering however many lists
/// show the view, and the step after it doesn't save them again. A list of another view numbers that
/// view. An undo puts back the drawing and keeps the numbers, as a drawing's step puts back only the
/// drawing; no model is evaluated for it.
#[test]
fn placing_a_list_keeps_the_numbers_its_view_shows() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    // A model read, so with the drawing closed for it.
    let shown = |s: &mut Session| {
        let drawing = ids::encode(s.open_drawing.unwrap());
        s.execute("drawing.close", &json!({})).unwrap();
        let rows = s.execute("assembly.bom", &json!({"levels": "flat"})).unwrap()["rows"].clone();
        s.execute("drawing.open", &json!({"drawing": drawing})).unwrap();
        rows
    };
    let before = shown(&mut s);
    assert!(s.doc.assembly.item_numbers.is_empty());
    let (revision, model) = (s.revision, s.model.state());
    run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"}));
    assert!(Arc::ptr_eq(&model, &s.model.state()), "the model isn't evaluated");
    assert!(s.revision > revision);
    assert_eq!(s.doc.assembly.item_numbers.len(), 1);
    let rows = |v: &Value| v.as_array().unwrap().iter().map(|r| (r["item"].clone(), r["part_number"].clone())).collect::<Vec<_>>();
    assert_eq!(rows(&shown(&mut s)), rows(&before), "what it showed");

    let numbers = s.doc.assembly.item_numbers.clone();
    let second = run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"}));
    assert_eq!(s.doc.assembly.item_numbers, numbers, "one numbering for one view");
    let (one, two) = (s.drawing("test").unwrap().tables[0].kind.clone(), s.drawing("test").unwrap().tables[1].kind.clone());
    assert_eq!(one, two, "{second}");

    run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list", "set": {"list": {"levels": "top"}}}));
    assert_eq!(s.doc.assembly.item_numbers.len(), 2);
    let edited = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"})));
    run(&mut s, EDIT, json!({"table": ids::encode(edited), "set": {"list": {"levels": "all"}}}));
    assert_eq!(s.doc.assembly.item_numbers.len(), 3, "an edit to another view numbers that one");

    let numbers = s.doc.assembly.item_numbers.clone();
    while !s.drawing("test").unwrap().tables.is_empty() {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(s.doc.assembly.item_numbers, numbers, "the design's, not the drawing's");
}

/// A revision table numbers nothing, and a list a renumbered view keeps its numbers.
#[test]
fn a_saved_numbering_stays_as_it_is() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    run(&mut s, ADD, json!({"sheet": sheet, "kind": "revision"}));
    assert!(s.doc.assembly.item_numbers.is_empty());
    s.execute("drawing.close", &json!({})).unwrap();
    s.execute("assembly.bom.renumber", &json!({"levels": "flat", "start": 10, "step": 10})).unwrap();
    let numbers = s.doc.assembly.item_numbers.clone();
    let drawing = ids::encode(s.doc.drawings[0].id);
    s.execute("drawing.open", &json!({"drawing": drawing})).unwrap();
    run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"}));
    assert_eq!(s.doc.assembly.item_numbers, numbers);
}

/// An edit changes what it names and nothing else, one step, none when nothing changes. A table moves
/// between sheets; a change of paper moves nothing in the record. A sheet's delete counts its tables
/// and takes them, leaving the others'.
#[test]
fn a_table_edits_moves_and_goes_with_its_sheet() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let id = id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"})));
    let table = ids::encode(id);
    let set = |s: &mut Session, set: Value| run(s, EDIT, json!({"table": table, "set": set}));

    let edited = set(&mut s, json!({"heading": "bottom", "row_height_mm": 9.0, "place": {"at": [20.0, 30.0]}}));
    assert_eq!((&edited["record"]["table"]["heading"], &edited["record"]["place"]), (&json!("bottom"), &json!({"at": [20.0, 30.0]})), "{edited}");
    assert_eq!(edited["record"]["table"]["columns"][1]["heading"], json!("QTY"), "the rest stays");
    let steps = s.undo.len();
    set(&mut s, json!({"heading": "bottom"}));
    assert_eq!(s.undo.len(), steps, "nothing changed, no step");

    let filter = json!({"only": "parts", "text": [{"column": "name", "contains": "bolt"}]});
    let overrides = json!([{"row": "a-key", "column": "name", "text": "Typed"}]);
    let edited = set(&mut s, json!({"filter": filter, "overrides": overrides}));
    assert_eq!((&edited["record"]["table"]["filter"], &edited["record"]["table"]["overrides"]), (&filter, &overrides), "{edited}");

    let before = s.drawing("test").unwrap().tables.by_id(id).unwrap().clone();
    s.execute("drawing.sheet.set", &json!({"sheet": sheet, "size": {"standard": "a1"}})).unwrap();
    assert_eq!(*s.drawing("test").unwrap().tables.by_id(id).unwrap(), before, "a change of paper moves nothing");

    let other = s.execute("drawing.sheet.add", &json!({"name": "Other"})).unwrap()["sheet"].as_str().unwrap().to_string();
    let moved = set(&mut s, json!({"sheet": "Other"}));
    assert_eq!(moved["sheet"], json!(other), "{moved}");
    let held = s.execute("drawing.sheet.delete", &json!({"sheet": other})).unwrap_err().to_string();
    assert!(held.contains("1 table"), "{held}");
    s.execute("drawing.sheet.delete", &json!({"sheet": other, "contents": "delete"})).unwrap();
    assert!(s.drawing("test").unwrap().tables.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    let id = s.drawing("test").unwrap().tables[0].id;
    let deleted = run(&mut s, DELETE, json!({"table": ids::encode(id)}));
    assert_eq!(deleted, json!({"table": ids::encode(id)}));
    assert!(s.drawing("test").unwrap().tables.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.drawing("test").unwrap().tables.len(), 1);
}

/// A table's id is drawn from the ids the drawing shares, and a damaged file's table is refused.
#[test]
fn hostile_tables_are_refused_with_nothing_changed() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let table = ids::encode(id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"}))));
    let revisions = ids::encode(id_of(&run(&mut s, ADD, json!({"sheet": sheet, "kind": "revision"}))));
    for bad in [
        json!({"sheet": "no such", "kind": "revision"}),
        json!({"sheet": sheet, "kind": "chart"}),
        json!({"sheet": sheet}),
        json!({"sheet": sheet, "kind": "revision", "set": {"list": {"levels": "flat"}}}),
        json!({"sheet": sheet, "kind": "parts_list", "set": {"list": {"component": "no such"}}}),
    ] {
        refused(&mut s, ADD, bad);
    }
    let long = "x".repeat(70_000);
    for bad in [
        json!({"columns": []}),
        json!({"columns": "x"}),
        json!({"columns": [{"name": "A b", "heading": "A", "content": []}]}),
        json!({"columns": [{"name": "a", "heading": long, "content": []}]}),
        json!({"heading": "sideways"}),
        json!({"row_height_mm": 0.0}),
        json!({"row_height_mm": -7.0}),
        json!({"row_height_mm": 1e300}),
        json!({"cap_mm": 8.0}),
        json!({"filter": {"text": [{"column": "nope", "contains": "x"}]}}),
        json!({"overrides": [{"row": "a", "column": "nope", "text": "x"}]}),
        json!({"place": {"at": [1e300, 0.0]}}),
        json!({"place": {"at": [0.0]}}),
        json!({"place": {"anchor": {"corner": "middle"}}}),
        json!({"place": {"anchor": {"corner": "top_left", "offset_mm": [-6000.0, 0.0]}}}),
        json!({"sheet": "no such"}),
        json!({"list": {"levels": "some"}}),
        json!({"list": {"group_by": "colour"}}),
        json!({"list": {"group_children": "yes"}}),
        json!({"list": {"mass": {"unit": "stone", "decimals": 2}}}),
        json!({"list": {"mass": {"unit": "kg", "decimals": 7}}}),
        json!({"list": {"component": [1]}}),
    ] {
        refused(&mut s, EDIT, json!({"table": table, "set": bad}));
    }
    assert!(refused(&mut s, EDIT, json!({"table": revisions, "set": {"list": {}}})).contains("parts list"));
    refused(&mut s, EDIT, json!({"table": "0000000000000", "set": {}}));
    refused(&mut s, DELETE, json!({"table": "0000000000000"}));
    s.drawing("test").unwrap().validate().unwrap();
}

/// Too large a design to list still places the list, says so, and saves no numbering.
#[test]
fn a_view_that_cant_be_numbered_still_places_its_list() {
    let mut s = assembly();
    let sheet = sheet_of(&s);
    let mut doc = (*s.doc).clone();
    let Some(first) = doc.occurrences.first().cloned() else { panic!("an occurrence") };
    doc.occurrences =
        (0..=solidcraft_doc::MAX_PLACEMENTS as u64).map(|i| solidcraft_doc::Occurrence { id: 1_000_000 + i, ..first.clone() }).collect();
    s.doc = Arc::new(doc);
    let reply = run(&mut s, ADD, json!({"sheet": sheet, "kind": "parts_list"}));
    assert!(reply["warnings"][0].as_str().is_some_and(|w| w.contains("can still move")), "{reply}");
    assert!(s.doc.assembly.item_numbers.is_empty());
    assert_eq!(s.drawing("test").unwrap().tables.len(), 1);
}
