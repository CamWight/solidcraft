use serde_json::json;

use super::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    s.execute(id, &p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("{id} accepted {p}"))
}

/// A box's drawing, open, with its one A3 sheet.
fn drawing_session() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 10}));
    run(&mut s, "drawing.create", json!({}));
    s
}

fn sheets(s: &Session) -> Vec<Arc<Sheet>> {
    s.drawing("test").unwrap().sheets.clone()
}

fn names(s: &Session) -> Vec<String> {
    sheets(s).iter().map(|sheet| sheet.name.clone()).collect()
}

/// A new sheet follows the last, or the one named, with its paper, border and resolution and a
/// name of its own; each add is one undo step.
#[test]
fn a_new_sheet_takes_the_paper_of_the_sheet_it_follows() {
    let mut s = drawing_session();
    let steps = s.undo.len();
    run(&mut s, "drawing.sheet.set", json!({"sheet": "Sheet 1", "dpi": 600, "size": {"standard": "a2"}}));
    let added = run(&mut s, "drawing.sheet.add", json!({}));
    let all = sheets(&s);
    assert_eq!(added["sheet"], json!(ids::encode(all[1].id)));
    assert_eq!((all[1].name.as_str(), all[1].dpi, all[1].sheet), ("Sheet 2", 600.0, all[0].sheet));
    assert_eq!(all[1].border, all[0].border);
    run(&mut s, "drawing.sheet.add", json!({"after": "Sheet 1", "name": "Details", "orientation": "portrait"}));
    assert_eq!(names(&s), ["Sheet 1", "Details", "Sheet 2"]);
    assert_eq!(sheets(&s)[1].sheet.orientation, Orientation::Portrait);
    assert_eq!(s.undo.len(), steps + 3);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&s), ["Sheet 1", "Sheet 2"]);
}

/// A sheet's settings change one at a time, the rest kept; what the sheet's rules refuse is
/// refused with the drawing unchanged.
#[test]
fn a_sheets_settings_change_and_keep_its_rules() {
    let mut s = drawing_session();
    let source = ids::encode(s.drawing("test").unwrap().sources[0].id);
    let id = ids::encode(sheets(&s)[0].id);
    run(
        &mut s,
        "drawing.sheet.set",
        json!({"sheet": id, "name": "Assembly", "documents": {"source": source, "component": 0}, "excluded": {"print": true}}),
    );
    let sheet = sheets(&s)[0].clone();
    assert_eq!(sheet.name, "Assembly");
    assert_eq!(sheet.documents.map(|d| d.component), Some(0));
    assert_eq!((sheet.excluded.count, sheet.excluded.print), (false, true));
    run(&mut s, "drawing.sheet.set", json!({"sheet": "Assembly", "excluded": {"count": true}, "documents": null}));
    let sheet = sheets(&s)[0].clone();
    assert_eq!((sheet.excluded.count, sheet.excluded.print, sheet.documents), (true, true, None), "the other flag kept");
    let before = s.drawing("test").unwrap().clone();
    for (p, says) in [
        (json!({"sheet": "Assembly", "dpi": 24}), "dpi"),
        (json!({"sheet": "Assembly", "size": {"standard": "b7"}}), "`size`"),
        (json!({"sheet": "Assembly", "size": {"custom": {"sides_mm": [0, 100]}}}), "sheet"),
        (json!({"sheet": "Assembly", "orientation": "upright"}), "landscape or portrait"),
        (json!({"sheet": "Assembly", "name": ""}), "name"),
        (json!({"sheet": "Assembly", "documents": {"source": "0000000000001"}}), "documents source"),
        (json!({"sheet": "Assembly", "excluded": {"count": "yes"}}), "true or false"),
        (json!({"sheet": "Nowhere"}), "no sheet `Nowhere`"),
        (json!({}), "`sheet`"),
    ] {
        let e = refused(&mut s, "drawing.sheet.set", p.clone());
        assert!(e.contains(says), "{p}: {e}");
        assert!(Arc::ptr_eq(s.drawing("test").unwrap(), &before), "{p}");
    }
}

/// Sheets reorder by every id once, a name shared by two is refused in favour of the id, and the
/// last sheet can't be deleted.
#[test]
fn sheets_reorder_and_delete_but_one_always_stays() {
    let mut s = drawing_session();
    run(&mut s, "drawing.sheet.add", json!({}));
    run(&mut s, "drawing.sheet.add", json!({"name": "Sheet 1"}));
    let ids: Vec<String> = sheets(&s).iter().map(|sheet| ids::encode(sheet.id)).collect();
    assert!(refused(&mut s, "drawing.sheet.delete", json!({"sheet": "Sheet 1"})).contains("give its id"));
    run(&mut s, "drawing.sheet.reorder", json!({"sheets": [ids[2], ids[0], ids[1]]}));
    assert_eq!(sheets(&s).iter().map(|sheet| ids::encode(sheet.id)).collect::<Vec<_>>(), [ids[2].clone(), ids[0].clone(), ids[1].clone()]);
    for order in [json!([ids[0], ids[1]]), json!([ids[0], ids[0], ids[1]]), json!([ids[0], ids[1], ids[2], "0000000000001"]), json!("all")] {
        let e = refused(&mut s, "drawing.sheet.reorder", json!({"sheets": order}));
        assert!(e.contains("every sheet's id once"), "{order}: {e}");
    }
    run(&mut s, "drawing.sheet.delete", json!({"sheet": ids[0]}));
    run(&mut s, "drawing.sheet.delete", json!({"sheet": ids[1]}));
    assert!(refused(&mut s, "drawing.sheet.delete", json!({"sheet": ids[2]})).contains("at least one sheet"));
    assert_eq!(sheets(&s).len(), 1);
}
