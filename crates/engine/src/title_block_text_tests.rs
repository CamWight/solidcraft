//! A title block's text on a sheet (#4890 B3): the built-in block on a new drawing, its values
//! typed over its cells so each test knows its text. Places come from the cell text rule
//! (`geom::cell`), widths from `paper_lines`, which measures what the runs are set from.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use solidcore_record::Vec2;
use solidcraft_geom::cell::{MAX_TEXT_BYTES, PaperText};
use solidcraft_geom::paper::text::TextRun;
use solidcraft_geom::paper::{Item, ObjectKind};
use solidcraft_geom::prelude::*;
use solidcraft_geom::title_block::CellLayout;
use solidcraft_sketch::paper_lines;

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
    paper_lines(text, None, "", cap_mm, None, &TextStyle::default()).unwrap().lines[0].1
}

/// What `drawing.inspect` lists of the sheet's cells.
fn cells(s: &mut Session) -> Vec<Value> {
    let inspected = run(s, "drawing.inspect", json!({}));
    inspected["sheets"][0]["title_block_cells"].as_array().unwrap().clone()
}

/// What `drawing.inspect` notes of cell `name`.
fn notes(s: &mut Session, name: &str) -> Value {
    cells(s).into_iter().find(|c| c["name"] == name).unwrap()["notes"].clone()
}

fn near(a: Vec2, b: Vec2) {
    assert!(a.dist(b) < 1e-9, "{a:?} != {b:?}");
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
    // Each label, and each value's lines in its box, which read as its text: the general tolerances'
    // note breaks between words. The projection symbol's cell has a label and no text, and a field
    // with no value shows nothing.
    let mut values = 0;
    for cell in cells(&mut s).into_iter().filter(|c| c["text"] != "") {
        let value_box = placed(&s, cell["name"].as_str().unwrap()).value_box;
        let lines: Vec<&str> = drawn.iter().filter(|r| value_box.contains(r.origin)).map(|r| r.text.as_str()).collect();
        assert_eq!(lines.join(" "), cell["text"], "{}", cell["name"]);
        values += lines.len();
    }
    assert_eq!(drawn.len(), 18 + values, "{:?}", drawn.iter().map(|r| &r.text).collect::<Vec<_>>());
    let d = s.drawing("test").unwrap().clone();
    let scene = build(&s, &d, sheet).unwrap().scene;
    let group = scene.groups.last().unwrap();
    let record = &d.sheets[0];
    let lines = solidcraft_doc::drawing::projection_symbol::title_block_paper(&record.title_block_layout().unwrap().unwrap(), sheet, d.projection);
    assert_eq!(group.items[..lines.items.len()], lines.items[..]);
    assert!(group.items[lines.items.len()..].iter().all(|item| matches!(item, Item::Text(_))));
    // The drawing names no font: all of it in the built-in one, listed once.
    assert_eq!(scene.fonts.iter().map(|f| (f.id, f.postscript_name.as_str())).collect::<Vec<_>>(), [(drawn[0].font, "LiberationSans")]);
}

/// A field with no value shows nothing; text typed over a cell takes its place.
#[test]
fn text_typed_over_a_cell_replaces_its_value_and_a_field_with_none_shows_nothing() {
    let (mut s, sheet) = drawing();
    let number = placed(&s, "identification_number").value_box;
    // The runs in the cell's value box: its value.
    let shown = |s: &Session| -> Vec<(String, Vec2)> {
        runs(s, sheet).into_iter().filter(|r| number.contains(r.origin)).map(|r| (r.text, r.origin)).collect()
    };
    let at = Vec2::new(number.min.x, number.center().y - 2.5);
    assert_eq!(shown(&s), []);
    typed(&mut s, json!({"identification_number": "DRW-7"}));
    assert_eq!(shown(&s), [("DRW-7".to_string(), at)]);
}

/// The built-in block's Material and Finish cells read the part's own properties with no text typed
/// over them: a part's, and a single-part design's own (a design has no component to hold them); text
/// typed over a cell still wins, and a part with only a free `finish` property shows that.
#[test]
fn the_material_and_finish_cells_read_the_part_and_a_single_part_designs_own() {
    let text = |s: &mut Session, name: &str| cells(s).into_iter().find(|c| c["name"] == name).unwrap()["text"].clone();
    let document = |s: &mut Session, component: u64| {
        let source = solidcraft_doc::ids::encode(s.drawing("test").unwrap().sources[0].id);
        run(s, "drawing.sheet.set", json!({"sheet": "Sheet 1", "documents": {"source": source, "component": component}}));
    };
    // A single-part design.
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 50, "height": 20, "body_name": "Block"}));
    run(&mut s, "document.metadata.set", json!({"values": {"material": "5052-H32, 0.090 in", "finish": "Powder coat, RAL 9005, matt"}}));
    run(&mut s, "drawing.create", json!({}));
    document(&mut s, 0);
    assert_eq!(text(&mut s, "material"), json!("5052-H32, 0.090 in"));
    assert_eq!(text(&mut s, "finish"), json!("Powder coat, RAL 9005, matt"));
    typed(&mut s, json!({"finish": "See note 3"}));
    assert_eq!((text(&mut s, "finish"), text(&mut s, "material")), (json!("See note 3"), json!("5052-H32, 0.090 in")), "an override wins");
    // A part of an assembly: its finish property, and the free property when it has none.
    let mut s = Session::default();
    for (name, extra) in [
        ("Plate", json!({"finish": "Passiv\u{e9}, #4 brushed", "material": "304, 2 mm"})),
        ("Cover", json!({"properties": {"finish": "Zinc plate"}})),
    ] {
        run(&mut s, "component.create", json!({"name": name}));
        run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
        run(&mut s, "component.activate", json!({"component": 0}));
        let mut p = json!({"component": name});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        run(&mut s, "component.metadata.set", p);
    }
    run(&mut s, "drawing.create", json!({}));
    let id = |s: &Session, name: &str| s.doc.components.iter().find(|c| c.name == name).unwrap().id;
    let (plate, cover) = (id(&s, "Plate"), id(&s, "Cover"));
    document(&mut s, plate);
    assert_eq!((text(&mut s, "finish"), text(&mut s, "material")), (json!("Passiv\u{e9}, #4 brushed"), json!("304, 2 mm")));
    document(&mut s, cover);
    assert_eq!(text(&mut s, "finish"), json!("Zinc plate"), "the free property still shows");
}

/// A cell that shows a logo shows it in place of its value: its label stays, nothing typed over the
/// cell or read by its field is drawn over the picture, and `drawing.inspect` lists no text for it.
/// The typed text is kept, not drawn: with the logo taken off, it shows again.
#[test]
fn a_logo_cell_keeps_its_label_and_draws_no_value() {
    let (mut s, sheet) = drawing();
    typed(&mut s, json!({"legal_owner": "Acme Ltd"}));
    let texts = |s: &Session| runs(s, sheet).into_iter().map(|r| r.text).collect::<Vec<_>>();
    assert!(texts(&s).iter().any(|t| t == "Acme Ltd") && texts(&s).iter().any(|t| t == "Legal owner"));
    let dir = std::env::temp_dir().join(format!("solidcraft-title-block-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("logo.png");
    std::fs::write(&file, crate::drawing_pictures::tests::png(320, 80)).unwrap();
    run(&mut s, "drawing.title_block.logo", json!({"cell": "legal_owner", "file": file.to_string_lossy()}));
    std::fs::remove_dir_all(&dir).unwrap();
    let shown = texts(&s);
    assert!(!shown.iter().any(|t| t == "Acme Ltd") && shown.iter().any(|t| t == "Legal owner"), "{shown:?}");
    assert!(!cells(&mut s).iter().any(|cell| cell["name"] == "legal_owner"));
    // The picture is in the block's group, under its lines.
    let d = s.drawing("test").unwrap().clone();
    let block = build(&s, &d, sheet).unwrap().scene.groups.into_iter().find(|g| g.kind == ObjectKind::TitleBlock).unwrap();
    assert!(matches!(block.items.first(), Some(Item::Picture(_))), "{:?}", block.items.first());
    run(&mut s, "drawing.title_block.logo", json!({"cell": "legal_owner", "file": null}));
    assert!(texts(&s).iter().any(|t| t == "Acme Ltd"), "{:?}", texts(&s));
    assert!(cells(&mut s).iter().any(|cell| cell["name"] == "legal_owner" && cell["text"] == "Acme Ltd"));
}

/// A value that wraps breaks between words to its box's width, its lines running down a pitch apart
/// from the box's top: a part's tolerances note on a new drawing. A line break always breaks. What
/// doesn't fit is shown whole, and `drawing.inspect` notes it: too many characters, too many lines,
/// a word or line too wide.
#[test]
fn a_wrapped_value_breaks_to_its_box_and_what_doesnt_fit_is_noted() {
    let (mut s, sheet) = drawing();
    let tolerances = placed(&s, "general_tolerances").value_box;
    let note = cells(&mut s).into_iter().find(|c| c["name"] == "general_tolerances").unwrap()["text"].as_str().unwrap().to_string();
    let wrapped = paper_lines(&note, None, "", 2.5, Some(tolerances.size().x), &TextStyle::default()).unwrap();
    assert!(wrapped.lines.len() > 1, "{wrapped:?}");
    let drawn = runs(&s, sheet);
    for (row, (line, w)) in wrapped.lines.iter().enumerate() {
        near(find(&drawn, line).origin, Vec2::new(tolerances.min.x, tolerances.max.y - 2.5 - row as f64 * wrapped.pitch));
        assert!(*w <= tolerances.size().x);
    }
    assert_eq!(notes(&mut s, "general_tolerances"), Value::Null);
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

/// A value with a character no font has is left off the sheet, which is drawn all the same, and
/// `drawing.inspect` says why; a right-to-left one is set as such. The character is a noncharacter,
/// which no font maps (an installed icon font may fill the private-use area).
#[test]
fn a_value_the_font_cant_set_is_left_off_and_noted() {
    let (mut s, sheet) = drawing();
    typed(&mut s, json!({"title": "A \u{fdd0}", "material": "פלדה 12"}));
    let drawn = runs(&s, sheet);
    find(&drawn, "Title");
    assert!(drawn.iter().all(|r| !r.text.contains('\u{fdd0}')));
    assert!(drawn.iter().any(|r| r.rtl && r.text.contains("פלדה")));
    let refused = notes(&mut s, "title");
    assert!(refused[0]["refused"].as_str().unwrap().contains("has no glyph for"), "{refused}");
    assert_eq!(notes(&mut s, "material"), Value::Null);
}

/// The sheet page follows a change of the design it documents at once (its name, in the title
/// cell), and draws no view again for it.
#[test]
fn the_sheet_page_follows_the_documented_designs_name() {
    let (mut s, sheet) = drawing();
    let source = solidcraft_doc::ids::encode(s.drawing("test").unwrap().sources[0].id);
    run(&mut s, "drawing.sheet.set", json!({"sheet": "Sheet 1", "documents": {"source": source, "component": 0}}));
    extract(&mut s, sheet).unwrap();
    while step(&mut s) {}
    let before = extract(&mut s, sheet).unwrap();
    // Model commands wait for the drawing to close, so the design is changed as they would: the
    // scene keys on the design itself, whatever changes it.
    s.doc_mut().name = "Plate".into();
    let after = extract(&mut s, sheet).unwrap();
    assert!(after.generation > before.generation && after.stale.is_empty());
    let block = after.scene.groups.iter().find(|g| g.kind == ObjectKind::TitleBlock).unwrap();
    assert!(block.items.iter().any(|item| matches!(item, Item::Text(run) if run.text == "Plate")));
}

/// A value in a script the drawing's font lacks: the window's sheet leaves it to the search's thread
/// and `step` keeps asking, until it is the sheet an export draws, whichever fonts this computer has.
/// A second one comes while the first's font is on the sheet: no new face, and it still comes. The
/// drawing's language chooses among the fonts for Han, as it does for an export, and
/// `drawing.inspect` names the font that set it.
#[test]
fn a_value_in_another_script_ends_as_an_export_draws_it() {
    let (mut s, sheet) = drawing();
    let settled = |s: &mut Session| {
        let started = std::time::Instant::now();
        let mut answer = extract(s, sheet).unwrap();
        while step(s) || answer.waiting_for_fonts {
            assert!(started.elapsed().as_secs() < 120, "the search for fonts never answered");
            std::thread::sleep(std::time::Duration::from_millis(10));
            answer = extract(s, sheet).unwrap();
        }
        let window = extract(s, sheet).unwrap();
        let d = s.drawing("test").unwrap().clone();
        assert_eq!(serde_json::to_string(&*window.scene).unwrap(), serde_json::to_string(&build(s, &d, sheet).unwrap().scene).unwrap());
    };
    // Characters asked for nowhere else, so the search hasn't answered yet.
    typed(&mut s, json!({"title": "\u{9f8d}"}));
    assert!(extract(&mut s, sheet).unwrap().waiting_for_fonts, "the window's sheet leaves it to the search");
    settled(&mut s);
    typed(&mut s, json!({"title": "\u{9f8d}", "material": "\u{9f9c}"}));
    assert!(extract(&mut s, sheet).unwrap().waiting_for_fonts, "the window's sheet leaves it to the search");
    settled(&mut s);
    let refused = notes(&mut s, "title");
    assert_eq!(notes(&mut s, "material").is_null(), refused.is_null(), "the same font has both, or none has");
    let fallbacks = run(&mut s, "drawing.inspect", json!({}));
    assert_eq!(fallbacks["font_fallbacks"].as_array().unwrap().is_empty(), !refused.is_null(), "{fallbacks}");
    for language in ["ja", "zh-CN"] {
        run(&mut s, "drawing.set", json!({"language": language}));
        settled(&mut s);
        let plain = TextStyle::default();
        if let Ok(Some((own, _))) = solidcraft_sketch::paper_text("\u{9f8d}", None, language, Vec2::ZERO, 3.5, 0.0, &plain) {
            assert_eq!(find(&runs(&s, sheet), "\u{9f8d}").font, own[0].font, "{language}");
        }
    }
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

/// The built-in block's labels print in the drawing's language (#19884), on the window's sheet as
/// on an export's, and a change of the language reprints them. A label someone typed prints as
/// typed in every language, and a word no catalog has prints its English.
#[test]
fn the_built_in_labels_print_in_the_drawings_language_and_typed_ones_as_typed() {
    crate::paper_words::install_test_catalogs();
    let (mut s, sheet) = drawing();
    let window = |s: &mut Session| {
        let scene = extract(s, sheet).unwrap().scene;
        let block = scene.groups.iter().find(|g| g.kind == ObjectKind::TitleBlock).unwrap().clone();
        block.items.into_iter().filter_map(|item| if let Item::Text(run) = item { Some(run.text) } else { None }).collect::<Vec<_>>()
    };
    assert!(window(&mut s).iter().any(|text| text == "Title"), "English by default");
    run(&mut s, "drawing.set", json!({"language": "fr"}));
    let drawn = runs(&s, sheet);
    near(find(&drawn, "Titre").origin, placed(&s, "title").label_at.unwrap());
    find(&drawn, "Propriétaire légal");
    find(&drawn, "Approved by");
    assert!(!drawn.iter().any(|r| r.text == "Title" || r.text == "Legal owner"));
    assert_eq!(window(&mut s), drawn.iter().map(|r| r.text.clone()).collect::<Vec<_>>(), "the window's sheet is the export's");
    run(&mut s, "drawing.set", json!({"language": "de"}));
    let reprinted = window(&mut s);
    assert!(reprinted.iter().any(|text| text == "Titel") && reprinted.iter().any(|text| text == "Rechtlicher Eigentümer"), "{reprinted:?}");
    assert!(!reprinted.iter().any(|text| text == "Titre"));
    // A label typed over the built-in one is kept as typed, saved as a plain string, in each language.
    let mut block = serde_json::to_value(s.drawing("test").unwrap().sheets[0].title_block.as_ref().unwrap()).unwrap();
    let title = block["cells"].as_array_mut().unwrap().iter_mut().find(|cell| cell["name"] == "title").unwrap();
    assert_eq!(title["label"]["text"], json!({"message": "drawing.paper.cell.title"}));
    title["label"]["text"] = json!("Benennung");
    run(&mut s, "drawing.sheet.set", json!({"sheet": "Sheet 1", "title_block": block}));
    for language in ["de", "fr", "en"] {
        run(&mut s, "drawing.set", json!({"language": language}));
        near(find(&runs(&s, sheet), "Benennung").origin, placed(&s, "title").label_at.unwrap());
    }
    let saved = serde_json::to_value(s.drawing("test").unwrap().sheets[0].title_block.as_ref().unwrap()).unwrap();
    assert_eq!(saved["cells"].as_array().unwrap().iter().find(|cell| cell["name"] == "title").unwrap()["label"]["text"], "Benennung");
}

/// A drawing saved by 0.49.0 prints exactly as before (#19884): its block's labels, and the note and
/// document type its part template typed over their cells, were saved as English text. So they read
/// as typed text, print in English whatever the drawing's language, and save again byte for byte. It
/// is still the built-in block to the sheet panel.
#[test]
fn a_sheet_saved_before_built_in_words_prints_and_saves_as_it_did() {
    use solidcraft_doc::drawing::template::{PART_NOTE, PART_TYPE};

    crate::paper_words::install_test_catalogs();
    let (mut s, sheet) = drawing();
    let english = crate::paper_words::in_english(&solidcraft_geom::title_block::default_block());
    let old = serde_json::to_string(&english).unwrap();
    assert!(old.contains(r#""text":"Title""#) && !old.contains("drawing.paper"), "a 0.49.0 block: {old}");
    let words = |id: &str| crate::paper_words::english(id).unwrap();
    let old_cells = json!({"document_type": words(PART_TYPE), "general_tolerances": words(PART_NOTE)});
    let old_block = serde_json::from_str::<Value>(&old).unwrap();
    run(&mut s, "drawing.sheet.set", json!({"sheet": "Sheet 1", "title_block": old_block, "title_block_overrides": old_cells.clone()}));
    let before = runs(&s, sheet);
    run(&mut s, "drawing.set", json!({"language": "fr"}));
    let after = runs(&s, sheet);
    find(&after, "Title");
    find(&after, "Part drawing");
    assert!(!after.iter().any(|r| r.text == "Titre" || r.text == "Plan de pièce" || r.text.starts_with("Sauf")));
    assert_eq!(after, before, "the same runs in French as in English");
    let record = s.drawing("test").unwrap().sheets[0].clone();
    assert_eq!(serde_json::to_string(record.title_block.as_ref().unwrap()).unwrap(), old);
    assert_eq!(serde_json::to_value(&record.title_block_overrides).unwrap(), old_cells, "saved as the strings 0.49.0 wrote");
    assert_eq!(crate::paper_words::in_english(record.title_block.as_ref().unwrap()), english);
}

/// A built-in template's general tolerances and document type print in the drawing's language
/// (#19884) on a drawing from each built-in template, and a change of the language reprints them, on
/// the window's sheet as on an export's. A note someone typed over the cell prints as typed in every
/// language and saves as plain text; the document type beside it stays the built-in message.
#[test]
fn a_templates_note_and_type_print_in_the_drawings_language_and_typed_ones_as_typed() {
    use solidcraft_doc::drawing::template::{self, ASSEMBLY_NOTE, ASSEMBLY_TYPE};

    crate::paper_words::install_test_catalogs();
    let (mut s, _) = drawing();
    let shown = |s: &mut Session, name: &str| cells(s).into_iter().find(|c| c["name"] == name).unwrap()["text"].clone();
    // The open drawing's first sheet's block, its runs one space apart: a note wraps to several runs.
    let sheet = |s: &Session| s.drawing("test").unwrap().sheets[0].id;
    let printed = |s: &Session| runs(s, sheet(s)).into_iter().map(|r| r.text).collect::<Vec<_>>().join(" ");
    for name in template::BUILT_INS {
        run(&mut s, "drawing.close", json!({}));
        run(&mut s, "drawing.create", json!({"template": name, "name": name, "language": "fr"}));
        let (note, kind) = if template::assembly(name) == Some(true) {
            let note =
                "Sauf indication contraire : positions d'assemblage ±0.5 mm, angles ±1°. Les tolérances des pièces figurent sur les plans de pièces.";
            (note, "Plan d'assemblage")
        } else {
            ("Sauf indication contraire : linéaire ±0.2 mm, angles ±0.5°. Casser les arêtes vives 0.2 mm.", "Plan de pièce")
        };
        assert_eq!((shown(&mut s, "general_tolerances"), shown(&mut s, "document_type")), (json!(note), json!(kind)), "{name}");
        let text = printed(&s);
        assert!(text.contains(note) && text.contains(kind) && !text.contains("Unless stated"), "{name}: {text}");
    }
    let window = |s: &mut Session| {
        let at = sheet(s);
        let scene = extract(s, at).unwrap().scene;
        let block = scene.groups.iter().find(|g| g.kind == ObjectKind::TitleBlock).unwrap().clone();
        block.items.into_iter().filter_map(|item| if let Item::Text(run) = item { Some(run.text) } else { None }).collect::<Vec<_>>().join(" ")
    };
    assert_eq!(window(&mut s), printed(&s), "the window's sheet is the export's");
    // The last drawing made, an assembly's: back in English, then a note typed over the built-in one.
    assert!(template::BUILT_INS.last().is_some_and(|name| template::assembly(name) == Some(true)));
    let last = (ASSEMBLY_NOTE, ASSEMBLY_TYPE);
    let english = |id: &str| json!(crate::paper_words::english(id).unwrap());
    run(&mut s, "drawing.set", json!({"language": "en"}));
    assert_eq!((shown(&mut s, "general_tolerances"), shown(&mut s, "document_type")), (english(last.0), english(last.1)), "reprinted");
    assert!(window(&mut s).contains(english(last.1).as_str().unwrap()), "reprinted on the window's sheet");
    typed(&mut s, json!({"general_tolerances": "ISO 2768-mK"}));
    for language in ["fr", "de", "en"] {
        run(&mut s, "drawing.set", json!({"language": language}));
        assert_eq!(shown(&mut s, "general_tolerances"), json!("ISO 2768-mK"), "{language}");
        assert!(printed(&s).contains("ISO 2768-mK"), "{language}");
    }
    run(&mut s, "drawing.set", json!({"language": "fr"}));
    assert_eq!(shown(&mut s, "document_type"), json!("Plan d'assemblage"), "the type beside it is still built in");
    let saved = serde_json::to_value(&s.drawing("test").unwrap().sheets[0].title_block_overrides).unwrap();
    assert_eq!(saved, json!({"document_type": {"message": last.1}, "general_tolerances": "ISO 2768-mK"}));
}

/// An untitled design's Title cell prints its default name in the drawing's language (#19884), as the
/// design's tab shows it, and so does the identification number, which reads the name without a part
/// number; a change of the language reprints it. A name someone typed prints as typed, even one that is
/// another language's word for Untitled, or Untitled in other capitals.
#[test]
fn an_untitled_designs_title_prints_in_the_drawings_language_and_a_typed_name_as_typed() {
    crate::paper_words::install_test_catalogs();
    let (mut s, sheet) = drawing();
    let source = solidcraft_doc::ids::encode(s.drawing("test").unwrap().sources[0].id);
    run(&mut s, "drawing.sheet.set", json!({"sheet": "Sheet 1", "documents": {"source": source, "component": 0}}));
    assert_eq!(s.doc.name, "Untitled");
    let count = |s: &Session, text: &str| runs(s, sheet).iter().filter(|r| r.text == text).count();
    assert_eq!(count(&s, "Untitled"), 2, "English: the title and the identification number");
    for (language, printed) in [("fr", "Sans titre"), ("de", "Ohne Titel"), ("en", "Untitled")] {
        run(&mut s, "drawing.set", json!({"language": language}));
        assert_eq!(count(&s, printed), 2, "{language}");
        let title = cells(&mut s).into_iter().find(|c| c["name"] == "title").unwrap();
        assert_eq!(title["text"], printed, "drawing.inspect lists what the sheet prints");
    }
    for typed in ["Sans titre", "untitled"] {
        s.doc_mut().name = typed.into();
        for language in ["de", "fr"] {
            run(&mut s, "drawing.set", json!({"language": language}));
            assert_eq!(count(&s, typed), 2, "{language}: `{typed}` is typed");
            assert_eq!(count(&s, "Ohne Titel") + count(&s, "Untitled"), 0, "{language}: `{typed}`");
        }
    }
}

/// A word a drawing prints that someone can type over (#19884, plan §5): a title block cell's label,
/// or the text typed over its value, by the cell's name; a table column's heading, by the table's id
/// and the column's name; a view label's words before its letter, or before its scale.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum WordField {
    Label(String),
    Override(String),
    Heading(u64, String),
    ViewWords(u64),
    ScaleWords(u64),
}

impl WordField {
    /// Its kind, as the property test sweeps them.
    fn kind(&self) -> usize {
        match self {
            WordField::Label(_) => 0,
            WordField::Override(_) => 1,
            WordField::Heading(..) => 2,
            WordField::ViewWords(_) => 3,
            WordField::ScaleWords(_) => 4,
        }
    }
}

/// The languages of the paper words' pending catalog (`assets/locales/pending/drawing-paper-text.json`),
/// as a drawing's `language` names them.
const PAPER_LANGUAGES: [&str; 11] = ["en", "ar", "de", "fr", "he", "ja", "ko", "ru", "vi", "zh", "zh-TW"];

/// Every word of `d`'s first sheet, its tables and its views, as saved.
fn word_fields(d: &Drawing) -> BTreeMap<WordField, PaperText> {
    let mut fields = BTreeMap::new();
    let sheet = &d.sheets[0];
    for cell in sheet.title_block.iter().flat_map(|block| &block.cells) {
        if let Some(label) = &cell.label {
            fields.insert(WordField::Label(cell.name.clone()), label.text.clone());
        }
    }
    for (cell, text) in &sheet.title_block_overrides {
        fields.insert(WordField::Override(cell.clone()), text.clone());
    }
    for table in d.tables.iter() {
        for column in &table.table.columns {
            fields.insert(WordField::Heading(table.id, column.name.clone()), column.heading.clone());
        }
    }
    for view in &d.views {
        if let Some(label) = &view.label {
            fields.insert(WordField::ViewWords(view.id), label.words.clone());
            fields.insert(WordField::ScaleWords(view.id), label.scale_words.clone());
        }
    }
    fields
}

/// Types `text` over `field` of the open drawing by the command a user's edit runs: the title block
/// as `drawing.sheet.set` takes it, a cell's text, a table's columns as `drawing.table.edit` takes
/// them, a view label's part. Whether it was taken; a refusal is never an internal error.
fn type_word(s: &mut Session, field: &WordField, text: &str) -> bool {
    let d = s.drawing("test").unwrap().clone();
    let (id, p) = match field {
        WordField::Label(cell) => {
            let mut block = serde_json::to_value(d.sheets[0].title_block.as_ref().unwrap()).unwrap();
            let named = block["cells"].as_array_mut().unwrap().iter_mut().find(|c| c["name"] == cell.as_str()).unwrap();
            named["label"]["text"] = json!(text);
            ("drawing.sheet.set", json!({"sheet": "Sheet 1", "title_block": block}))
        }
        WordField::Override(cell) => {
            let cells: serde_json::Map<String, Value> = [(cell.clone(), json!(text))].into_iter().collect();
            ("drawing.sheet.set", json!({"sheet": "Sheet 1", "title_block_overrides": cells}))
        }
        WordField::Heading(table, column) => {
            let mut columns = serde_json::to_value(&d.tables.by_id(*table).unwrap().table.columns).unwrap();
            let named = columns.as_array_mut().unwrap().iter_mut().find(|c| c["name"] == column.as_str()).unwrap();
            named["heading"] = json!(text);
            ("drawing.table.edit", json!({"table": solidcraft_doc::ids::encode(*table), "set": {"columns": columns}}))
        }
        WordField::ViewWords(view) => ("drawing.view.edit", json!({"view": solidcraft_doc::ids::encode(*view), "set": {"label": {"words": text}}})),
        WordField::ScaleWords(view) => {
            ("drawing.view.edit", json!({"view": solidcraft_doc::ids::encode(*view), "set": {"label": {"scale_words": text}}}))
        }
    };
    let r = s.execute(id, &p);
    assert!(!matches!(r, Err(crate::EngineError::Internal(..))), "{id} {field:?}: {r:?}");
    r.is_ok()
}

/// Whether the drawing's own rule refuses `text` over `field` of `d`: a label, a heading and a view
/// label's words are one line of at most 4096 bytes without a control character
/// (`cell::check_label`); the texts typed over a block's cells, the built-in messages' ids among
/// them, hold 4096 bytes in all, with no control character but a line break (`cell::check_text`).
fn may_refuse(d: &Drawing, field: &WordField, text: &str) -> bool {
    match field {
        WordField::Override(cell) => {
            let others: usize = d.sheets[0].title_block_overrides.iter().filter(|(name, _)| *name != cell).map(|(_, over)| over.saved_bytes()).sum();
            others + text.len() > MAX_TEXT_BYTES || text.chars().any(|c| c.is_control() && c != '\n')
        }
        _ => text.len() > MAX_TEXT_BYTES || text.chars().any(char::is_control),
    }
}

/// What `field` of `d`, saved as `saved`, prints in `language`, read where the sheet reads it: a
/// label and a scale's words by [`crate::paper_words::resolve`], which laying out the title block and
/// a view label call; a cell's text as [`shown`] gives it; a heading from its table's layout; a view's
/// words from its label's text ([`Drawing::label_text`]).
fn printed_word(s: &Session, d: &Drawing, field: &WordField, saved: &PaperText, language: &str) -> String {
    let words = |text: &PaperText| crate::paper_words::resolve(language, text).into_owned();
    match field {
        WordField::Label(_) | WordField::ScaleWords(_) => words(saved),
        WordField::Override(cell) => {
            let mut local = d.clone();
            local.language = language.to_string();
            let state = s.model.state();
            let mut tree = Tree::new(&s.doc, &state);
            shown(s, &mut tree, &local, 0).into_iter().find(|shown| shown.name == *cell).unwrap().text
        }
        WordField::Heading(table, column) => {
            let table = &d.tables.by_id(*table).unwrap().table;
            let printed: Vec<_> = table.columns.iter().filter(|c| c.printed).collect();
            // A column the table doesn't print (a parts list's ballooned column) has no heading on paper.
            let Some(at) = printed.iter().position(|c| c.name == *column) else { return words(saved) };
            let laid = table.layout(&[], Vec2::ZERO, &|_, _| 1.0, &|heading| crate::paper_words::resolve(language, heading)).unwrap();
            laid.heading.cells[at].lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join("\n")
        }
        WordField::ViewWords(view) => {
            let letter = &d.view(*view).unwrap().label.as_ref().unwrap().letter;
            let Some(text) = d.label_text(*view, &|text| crate::paper_words::resolve(language, text)) else { return words(saved) };
            // Its first line is its words and letter (`VIEW A`), or its letter alone.
            let first = text.split('\n').next().unwrap_or_default();
            first.strip_suffix(&format!(" {letter}")).unwrap_or(if first == letter.as_str() { "" } else { first }).to_string()
        }
    }
}

/// The hostile shapes of text someone can type: empty and blank text, line breaks and other control
/// characters, every built-in message's id and English and near misses of it, the words of every
/// installed catalog (the pseudo-language's wrapped ones too), each script the catalog's languages
/// write, combining marks and format characters, and text at the 4096-byte limit and over it.
/// Sorted, once each, so the generator picks the same every run.
fn hostile_words() -> Vec<String> {
    let blank = ["", " ", "   ", "\u{a0}", "\u{3000}", "\n", "a\nb", "\r\n", "\t", "\u{0}", "\u{7}", "\u{1b}[31mred", "\u{7f}", "\u{85}"];
    // A message's saved form, and an id no catalog has, typed as text.
    let saved = ["{\"message\": \"drawing.paper.cell.title\"}", "{\"message\":\"x\"}", "drawing.paper.no_such_word"];
    // Latin, Cyrillic, Arabic, Hebrew, Chinese, Japanese, Korean and Vietnamese script, and two
    // directions in one text.
    let scripts = ["Werkstoff", "Matière", "Материал", "ИЗМ.", "المادة", "חומר", "材料", "材質", "재료", "Vật liệu", "Part חומר 12", "مادة 42 mm"];
    // Combining marks, joiners, a direction mark and override, a byte order mark, an emoji sequence.
    let marks = ["Mate\u{301}riau", "T\u{336}i\u{337}tle", "Title\u{200d}", "\u{200c}", "\u{200f}Title", "\u{202e}eltiT", "\u{feff}Title", "👩‍🔧"];
    let mut words: BTreeSet<String> = blank.into_iter().chain(saved).chain(scripts).chain(marks).map(str::to_string).collect();
    for (id, english) in crate::paper_words::MESSAGES {
        words.extend([id.to_string(), english.to_string(), english.to_lowercase(), format!(" {english}")]);
    }
    for catalog in solidcraft_ui_text::locales() {
        words.extend(catalog.messages().values().cloned());
    }
    words.extend([MAX_TEXT_BYTES, MAX_TEXT_BYTES + 1].map(|bytes| "W".repeat(bytes)));
    words.extend([MAX_TEXT_BYTES / 3, MAX_TEXT_BYTES / 3 + 1].map(|chars| "材".repeat(chars)));
    words.into_iter().collect()
}

/// Each of `languages` in turn by `drawing.set`, as Drawing Settings changes it: every word in
/// `typed` is saved as typed text and prints as typed; every other word is still the one the drawing
/// was `made` with, and a built-in message prints in the language, its English wrapped in the
/// pseudo-language. Then the drawing saved and opened again has every word as it was.
fn every_word_checked(s: &mut Session, made: &BTreeMap<WordField, PaperText>, typed: &BTreeMap<WordField, String>, languages: &[&str]) {
    use crate::paper_words::{PSEUDO, PSEUDO_CLOSE, PSEUDO_OPEN, english};

    for language in languages {
        run(s, "drawing.set", json!({"language": language}));
        let d = s.drawing("test").unwrap().clone();
        assert_eq!(d.language, *language);
        let saved = word_fields(&d);
        assert!(typed.keys().all(|field| saved.contains_key(field)), "no word typed is lost");
        for (field, now) in &saved {
            let printed = printed_word(s, &d, field, now, language);
            let Some(text) = typed.get(field) else {
                assert_eq!(Some(now), made.get(field), "{field:?}: nobody typed it");
                let expected = match now.message() {
                    Some(id) if *language == PSEUDO => format!("{PSEUDO_OPEN}{}{PSEUDO_CLOSE}", english(id).unwrap()),
                    _ => crate::paper_words::resolve(language, now).into_owned(),
                };
                assert_eq!(printed, expected, "{field:?} in {language}");
                continue;
            };
            assert_eq!(*now, PaperText::typed(text.as_str()), "{field:?}");
            assert_eq!(printed, *text, "{field:?} in {language}");
        }
    }
    let d = s.drawing("test").unwrap().clone();
    let reopened = Drawing::from_json(&d.to_json().unwrap()).unwrap();
    assert_eq!(word_fields(&reopened), word_fields(&d), "saved and opened, every word as it was");
}

/// Text someone types over a drawing's words prints exactly as typed in every language of the paper
/// words' catalog, and only built-in messages translate (#19884, plan §5). Every hostile shape
/// ([`hostile_words`]) is typed over every kind of word (a title block label, a cell's text, a table
/// heading, a view label's words and its scale's words), each over a field of its kind a seeded
/// generator picks, then random pairs of them; before an edit the generator may change the drawing's
/// language. An edit is refused only by its field's own rule, and a refused edit changes nothing. A
/// word taken is saved as typed text, never as a message, even when it is a message's id, English or
/// translation, and prints exactly as typed in every language. Every so often, and at the end, each
/// language is set by `drawing.set` and every word read again: typed words as typed, the rest the
/// built-in messages the drawing was made with, printed in that language; then saved and opened.
#[test]
fn typed_words_print_as_typed_in_every_language_and_only_built_in_ones_translate() {
    crate::paper_words::install_test_catalogs();
    let (mut s, sheet) = drawing();
    let on = solidcraft_doc::ids::encode(sheet);
    run(&mut s, "drawing.table.add", json!({"sheet": on, "kind": "parts_list"}));
    run(&mut s, "drawing.table.add", json!({"sheet": on, "kind": "revision"}));
    let front = run(&mut s, "drawing.view.base", json!({"sheet": "Sheet 1", "scale": "1:2", "position_mm": [150.0, 150.0]}))["view"].clone();
    run(&mut s, "drawing.view.projected", json!({"parent": front, "side": "above", "place": {"at": [150.0, 230.0]}}));
    let made = word_fields(s.drawing("test").unwrap());
    // Each kind's fields; a cell's text can be typed over any text cell, one with none typed yet too.
    let cells: Vec<String> = {
        let d = s.drawing("test").unwrap();
        let state = s.model.state();
        let mut tree = Tree::new(&s.doc, &state);
        shown(&s, &mut tree, d, 0).into_iter().map(|cell| cell.name).collect()
    };
    let mut kinds: [Vec<WordField>; 5] = Default::default();
    for field in made.keys().cloned().chain(cells.into_iter().map(WordField::Override)) {
        let kind = &mut kinds[field.kind()];
        if !kind.contains(&field) {
            kind.push(field);
        }
    }
    assert!(kinds.iter().all(|fields| !fields.is_empty()), "every kind of word: {kinds:?}");
    let d = s.drawing("test").unwrap().clone();
    let lettered = |field: &WordField| matches!(field, WordField::ViewWords(view) if d.label_text(*view, &|text| crate::paper_words::resolve("en", text)).is_some());
    assert!(kinds[3].iter().any(lettered), "a view label printed with its letter");

    let mut state = 19884_u64;
    let mut next = move |n: usize| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((state >> 33) % n.max(1) as u64) as usize
    };
    let languages: Vec<&str> = PAPER_LANGUAGES.into_iter().chain([crate::paper_words::PSEUDO]).collect();
    let texts = hostile_words();
    let mut cases: Vec<(usize, String)> = texts.iter().flat_map(|text| (0..kinds.len()).map(move |kind| (kind, text.clone()))).collect();
    for _ in 0..80 {
        let kind = next(kinds.len());
        let pair = format!("{}{}", texts[next(texts.len())], texts[next(texts.len())]);
        cases.push((kind, pair));
    }
    let mut typed: BTreeMap<WordField, String> = BTreeMap::new();
    let (mut taken, mut refused) = (0, 0);
    for (case, (kind, text)) in cases.iter().enumerate() {
        if case > 0 && case.is_multiple_of(150) {
            every_word_checked(&mut s, &made, &typed, &languages);
        }
        if next(3) == 0 {
            run(&mut s, "drawing.set", json!({"language": languages[next(languages.len())]}));
        }
        let fields = &kinds[*kind];
        let field = &fields[next(fields.len())];
        let before = (**s.drawing("test").unwrap()).clone();
        let refusable = may_refuse(&before, field, text);
        let took = type_word(&mut s, field, text);
        assert_eq!(took, !refusable, "{field:?} {text:?} in {}: refused only by its rule", before.language);
        let d = s.drawing("test").unwrap().clone();
        if !took {
            assert!(*d == before, "{field:?} {text:?}: a refused edit changes nothing");
            refused += 1;
            continue;
        }
        taken += 1;
        typed.insert(field.clone(), text.clone());
        let saved = word_fields(&d);
        assert_eq!(saved[field], PaperText::typed(text.as_str()), "{field:?} {text:?}: saved as typed text, never as a message");
        for language in &languages {
            assert_eq!(printed_word(&s, &d, field, &saved[field], language), *text, "{field:?} {text:?} in {language}");
        }
    }
    every_word_checked(&mut s, &made, &typed, &languages);
    assert!(taken > cases.len() / 2 && refused > 0, "{taken} taken, {refused} refused of {}", cases.len());
}
