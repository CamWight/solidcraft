//! A title block's text on a sheet (#4890 B3): the built-in block on a new drawing, its values
//! typed over its cells so each test knows its text. Places come from the cell text rule
//! (`geom::cell`), widths from `paper_lines`, which measures what the runs are set from.

use serde_json::{Value, json};
use solidcraft_geom::Vec2;
use solidcraft_geom::paper::text::TextRun;
use solidcraft_geom::paper::{Item, ObjectKind};
use solidcraft_geom::title_block::CellLayout;

use super::*;
use crate::Session;
use crate::drawing_scene::{build, extract, step};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// A block 100 x 50 x 20 mm and a new drawing of it, its one sheet with the built-in title block:
/// the session and the sheet's id.
fn drawing() -> (Session, u64) {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 50, "height": 20, "body_name": "Block"}));
    run(&mut s, "drawing.create", json!({}));
    let sheet = s.drawing("test").unwrap().sheets[0].id;
    (s, sheet)
}

/// `cells` typed over the sheet's cells.
fn typed(s: &mut Session, cells: Value) {
    run(s, "drawing.sheet.set", json!({"sheet": "Sheet 1", "title_block_overrides": cells}));
}

/// The runs of the sheet's title block group, in order, built at once.
fn runs(s: &Session, sheet: u64) -> Vec<TextRun> {
    let d = s.drawing("test").unwrap().clone();
    let scene = build(s, &d, sheet).unwrap().scene;
    let block = scene.groups.iter().find(|g| g.kind == ObjectKind::TitleBlock).unwrap();
    block.items.iter().filter_map(|item| if let Item::Text(run) = item { Some(run.clone()) } else { None }).collect()
}

/// Cell `name` of the sheet's block, placed on the sheet.
fn placed(s: &Session, name: &str) -> CellLayout {
    let d = s.drawing("test").unwrap();
    let sheet = &d.sheets[0];
    let layout = sheet.title_block_layout().unwrap().unwrap();
    sheet.title_block.as_ref().unwrap().cells.iter().zip(layout.cells).find(|(cell, _)| cell.name == name).unwrap().1
}

/// The run whose text is `text`.
fn find<'a>(runs: &'a [TextRun], text: &str) -> &'a TextRun {
    runs.iter().find(|r| r.text == text).unwrap_or_else(|| panic!("no run `{text}` in {:?}", runs.iter().map(|r| &r.text).collect::<Vec<_>>()))
}

fn width(text: &str, cap_mm: f64) -> f64 {
    paper_lines(text, None, cap_mm, None, &TextStyle::default()).unwrap().lines[0].1
}

/// What `drawing.inspect` notes of cell `name`.
fn notes(s: &mut Session, name: &str) -> Value {
    let inspected = run(s, "drawing.inspect", json!({}));
    let cells = inspected["sheets"][0]["title_block_cells"].as_array().unwrap().clone();
    cells.into_iter().find(|c| c["name"] == name).unwrap()["notes"].clone()
}

fn near(a: Vec2, b: Vec2) {
    assert!((a - b).len() < 1e-9, "{a:?} != {b:?}");
}

/// Each label sits at its place, one line; a left-aligned value of one line starts at its box's
/// left side, centred top to bottom; a centred value is centred across its box. The block's lines
/// come first, as the layout draws them, and every run is in the drawing font, which the scene holds.
#[test]
fn labels_and_values_sit_by_the_cell_text_rule() {
    let (mut s, sheet) = drawing();
    typed(&mut s, json!({"title": "Bracket"}));
    let drawn = runs(&s, sheet);
    let title = placed(&s, "title");
    near(find(&drawn, "Title").origin, title.label_at.unwrap());
    let b = title.value_box;
    near(find(&drawn, "Bracket").origin, Vec2::new(b.min.x, b.center().y - 2.5));
    let sheet_cell = placed(&s, "sheet");
    let one_of_one = find(&drawn, "1/1");
    near(
        one_of_one.origin + Vec2::new(width("1/1", 3.5) / 2.0, 0.0),
        Vec2::new(sheet_cell.value_box.center().x, sheet_cell.value_box.center().y - 1.75),
    );
    assert!(drawn.iter().all(|r| r.font == drawn[0].font && r.angle == 0.0 && !r.rtl));
    // Twelve labels and eleven values: the projection symbol's cell has a label and no text.
    assert_eq!(drawn.len(), 23, "{:?}", drawn.iter().map(|r| &r.text).collect::<Vec<_>>());
    let d = s.drawing("test").unwrap().clone();
    let scene = build(&s, &d, sheet).unwrap().scene;
    let group = scene.groups.last().unwrap();
    let record = &d.sheets[0];
    let lines = record.title_block_layout().unwrap().unwrap().paper(sheet, d.projection);
    assert_eq!(group.items[..lines.items.len()], lines.items[..]);
    assert!(group.items[lines.items.len()..].iter().all(|item| matches!(item, Item::Text(_))));
    // The drawing names no font: all of it in the built-in one, listed once.
    assert_eq!(scene.fonts.iter().map(|f| (f.id, f.postscript_name.as_str())).collect::<Vec<_>>(), [(drawn[0].font, "LiberationSans")]);
}

/// A field with no value shows "-"; text typed over a cell takes its place.
#[test]
fn text_typed_over_a_cell_replaces_its_value_and_a_field_with_none_shows_a_dash() {
    let (mut s, sheet) = drawing();
    let number = placed(&s, "identification_number").value_box;
    // The runs in the cell's value box: its value.
    let shown = |s: &Session| -> Vec<(String, Vec2)> {
        runs(s, sheet).into_iter().filter(|r| number.contains(r.origin)).map(|r| (r.text, r.origin)).collect()
    };
    let at = Vec2::new(number.min.x, number.center().y - 2.5);
    assert_eq!(shown(&s), [("-".to_string(), at)]);
    typed(&mut s, json!({"identification_number": "DRW-7"}));
    assert_eq!(shown(&s), [("DRW-7".to_string(), at)]);
}

/// A value that wraps breaks between words to its box's width, its lines running down a pitch apart
/// from the box's top; a line break always breaks. What doesn't fit is shown whole, and
/// `drawing.inspect` notes it: too many characters, too many lines, a word or line too wide.
#[test]
fn a_wrapped_value_breaks_to_its_box_and_what_doesnt_fit_is_noted() {
    let (mut s, sheet) = drawing();
    let owner = placed(&s, "legal_owner").value_box;
    let words = "Tangency Labs and all of its many good friends everywhere";
    typed(&mut s, json!({"legal_owner": words}));
    let wrapped = paper_lines(words, None, 3.5, Some(owner.size().x), &TextStyle::default()).unwrap();
    assert_eq!(wrapped.lines.len(), 2, "{wrapped:?}");
    let drawn = runs(&s, sheet);
    for (row, (line, w)) in wrapped.lines.iter().enumerate() {
        near(find(&drawn, line).origin, Vec2::new(owner.min.x, owner.max.y - 3.5 - row as f64 * wrapped.pitch));
        assert!(*w <= owner.size().x);
    }
    assert_eq!(notes(&mut s, "legal_owner"), Value::Null);
    typed(
        &mut s,
        json!({
            "legal_owner": "One\nTwo\nThree\nFour",
            "title": "i".repeat(26),
            "identification_number": "W".repeat(20),
            "general_tolerances": "Supercalifragilistic".repeat(4),
        }),
    );
    assert_eq!(notes(&mut s, "legal_owner"), json!([{"fit": "height"}]));
    assert_eq!(notes(&mut s, "title"), json!([{"fit": "chars", "count": 26, "max": 25}]));
    assert_eq!(notes(&mut s, "identification_number"), json!([{"fit": "chars", "count": 20, "max": 16}, {"fit": "width"}]));
    assert_eq!(notes(&mut s, "general_tolerances"), json!([{"fit": "width"}]));
    // Shown whole all the same.
    let drawn = runs(&s, sheet);
    for text in ["One", "Four", &"i".repeat(26), &"W".repeat(20), &"Supercalifragilistic".repeat(4)] {
        find(&drawn, text);
    }
}

/// A value in a script the drawing font has no glyphs for is left off the sheet, which is drawn all
/// the same, and `drawing.inspect` says why; a right-to-left one is set as such.
#[test]
fn a_value_the_font_cant_set_is_left_off_and_noted() {
    let (mut s, sheet) = drawing();
    typed(&mut s, json!({"title": "零件", "material": "פלדה 12"}));
    let drawn = runs(&s, sheet);
    find(&drawn, "Title");
    assert!(drawn.iter().all(|r| r.text != "零件"));
    assert!(drawn.iter().any(|r| r.rtl && r.text.contains("פלדה")));
    let refused = notes(&mut s, "title");
    assert!(refused[0]["refused"].as_str().unwrap().contains("has no glyph for"), "{refused}");
    assert_eq!(notes(&mut s, "material"), Value::Null);
}

/// The sheet page follows a change of the drawing's fields at once, and draws no view again for it.
#[test]
fn the_sheet_page_follows_the_drawings_fields() {
    let (mut s, sheet) = drawing();
    extract(&mut s, sheet).unwrap();
    while step(&mut s) {}
    let before = extract(&mut s, sheet).unwrap();
    run(&mut s, "drawing.properties.set", json!({"key": "organization", "value": "Tangency Labs"}));
    let after = extract(&mut s, sheet).unwrap();
    assert!(after.generation > before.generation && after.stale.is_empty());
    let d = s.drawing("test").unwrap().clone();
    assert_eq!(serde_json::to_string(&*after.scene).unwrap(), serde_json::to_string(&build(&s, &d, sheet).unwrap().scene).unwrap());
    find(&runs(&s, sheet), "Tangency Labs");
}
