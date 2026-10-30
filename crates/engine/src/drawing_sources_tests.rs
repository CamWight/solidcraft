use std::path::PathBuf;

use serde_json::{Value, json};
use solidcraft_doc::drawing::Source;
use solidcraft_doc::drawing::view::{View, ViewKind, ViewPlace};
use solidcraft_doc::drawing_view::{ViewDirection, ViewOverrides};
use solidcraft_doc::{Document, Model};
use solidcraft_geom::drawing_view::NamedView;
use solidcraft_io::vfs::PathScope;

use super::*;

/// A folder of this test's own.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("solidcraft-drawing-sources-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A design of one box saved as `name` in `dir`: its file.
fn saved_box(dir: &std::path::Path, name: &str, size: f64) -> String {
    let mut part = Session::default();
    run(&mut part, "solid.box", json!({"length": size, "width": size, "height": size}));
    let file = dir.join(name).to_string_lossy().into_owned();
    run(&mut part, "file.save", json!({"path": file}));
    file
}

/// A session with a box and its drawing, open, which also draws each of `paths`: their source ids.
fn drawing_of(paths: &[&str]) -> (Session, Vec<u64>) {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run(&mut s, "drawing.create", json!({}));
    let mut d = (**s.drawing("test").unwrap()).clone();
    let added: Vec<u64> = paths.iter().map(|_| d.allocate_id(&mut s.ids).unwrap()).collect();
    for (id, path) in added.iter().zip(paths) {
        d.sources.push(Source { id: *id, path: Some(path.to_string()) });
        d.validate().unwrap();
    }
    s.doc_mut().put_drawing(Arc::new(d));
    (s, added)
}

/// Where the source `id` of the drawing open stands.
fn standing(s: &Session, id: u64) -> Result<(), SourceState> {
    let d = s.drawing("test").unwrap().clone();
    s.drawn(&d, id).map(|_| ()).map_err(|why| why.state)
}

/// Each way a source can't be opened has its word, and asks the disk nothing more than the rule lets it.
#[test]
fn each_way_a_design_cant_be_opened_has_its_word() {
    let dir = folder("words");
    let at = |name: &str| dir.join(name).to_string_lossy().into_owned();
    std::fs::write(at("plate.step"), b"ISO-10303-21;").unwrap();
    std::fs::write(at("corrupt.sc"), b"PK\x03\x04 not a design").unwrap();
    std::fs::create_dir_all(at("folder.sc")).unwrap();
    let mut newer: Value = serde_json::from_slice(&solidcraft_io::write_design(&Document::new("Newer"))).unwrap();
    newer["format"] = json!("solidcraft/99999");
    std::fs::write(at("newer.sc"), newer.to_string()).unwrap();
    let cases = [
        ("missing.sc", SourceState::Missing),
        ("folder.sc", SourceState::Unreadable),
        ("plate.step", SourceState::Unreadable),
        ("corrupt.sc", SourceState::Unreadable),
        ("newer.sc", SourceState::Newer),
    ];
    for (name, word) in cases {
        let (mut s, ids) = drawing_of(&[&at(name)]);
        assert_eq!(standing(&s, ids[0]), Err(SourceState::Pending), "{name}: nothing is read until it is needed");
        s.open_sources();
        assert_eq!(standing(&s, ids[0]), Err(word), "{name}");
        assert_eq!(s.linked.designs.len(), 1, "{name}");
    }
    assert_eq!(SourceState::NotAllowed.word(), "not_allowed");

    // Network locations and devices are judged from the text: not one call asks for them.
    let mut refused = vec![(r"\\.\COM1\plate.sc".to_string(), SourceState::Unreadable)];
    if cfg!(windows) {
        refused.push((r"\\evil.example\share\plate.sc".to_string(), SourceState::NotAllowed));
    }
    for (path, word) in refused {
        let (mut s, ids) = drawing_of(&[&path]);
        let calls = vfs::file_calls();
        s.open_sources();
        assert_eq!(vfs::file_calls(), calls, "{path}: a file call was made");
        assert_eq!(standing(&s, ids[0]), Err(word), "{path}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A file outside the folders an agent may read isn't read; it says so, and the drawing still works.
#[test]
fn a_design_outside_the_allowed_folders_says_so() {
    let (dir, allowed) = (folder("scope"), folder("scope-allowed"));
    let file = saved_box(&dir, "plate.sc", 20.0);
    let (mut s, ids) = drawing_of(&[&file]);
    vfs::scoped(Some(PathScope::new([allowed.clone()])), || s.open_sources());
    assert_eq!(standing(&s, ids[0]), Err(SourceState::NotAllowed));
    run(&mut s, "drawing.sheet.add", json!({}));
    for d in [dir, allowed] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

/// A source that names the design it is in draws that design: nothing is opened a second time.
#[test]
fn a_source_naming_the_design_itself_is_that_design() {
    let dir = folder("itself");
    let path = dir.join("a.sc").to_string_lossy().into_owned();
    let (mut s, ids) = drawing_of(&[&path]);
    run(&mut s, "file.save", json!({"path": path}));
    let calls = vfs::file_calls();
    s.open_sources();
    assert_eq!(vfs::file_calls(), calls, "the design isn't read again");
    assert!(s.linked.designs.is_empty());
    let d = s.drawing("test").unwrap().clone();
    assert!(std::ptr::eq(s.drawn(&d, ids[0]).unwrap(), &s));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A file is opened once however many sources name it, read-only and apart from the design. A drawing
/// in it that draws this design opens nothing more. Closing the drawing lets it go, and undo or a removed
/// source drops what no longer has a source.
#[test]
fn each_file_opens_once_and_only_while_it_is_needed() {
    let dir = folder("once");
    let plate = saved_box(&dir, "plate.sc", 20.0);
    let path = dir.join("a.sc").to_string_lossy().into_owned();
    // B's drawing draws A.
    let mut b = Session::default();
    run(&mut b, "solid.box", json!({"length": 5, "width": 5, "height": 5}));
    run(&mut b, "drawing.create", json!({"source": path}));
    let b_file = dir.join("b.sc").to_string_lossy().into_owned();
    run(&mut b, "file.save", json!({"path": b_file}));

    // A's drawing draws B, twice, and the plate.
    let (mut s, ids) = drawing_of(&[&b_file, &b_file, &plate]);
    run(&mut s, "file.save", json!({"path": path}));
    let design = s.doc.clone();
    s.open_sources();
    assert_eq!(s.linked.designs.len(), 2, "each file once");
    assert!(Arc::ptr_eq(&s.doc, &design), "opening one changes nothing here");
    let d = s.drawing("test").unwrap().clone();
    let from_b = s.drawn(&d, ids[0]).unwrap();
    assert!(std::ptr::eq(from_b, s.drawn(&d, ids[1]).unwrap()));
    assert!(from_b.read_only && from_b.linked.designs.is_empty(), "what it draws isn't opened in turn");

    let saved = solidcraft_io::design_source_paths(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved, ["b.sc", "plate.sc"]);

    // A scratch copy has them too, for what runs off the window's thread.
    let scratch = s.scratch();
    assert!(scratch.drawn(&d, ids[2]).is_ok());

    let mut fewer = (*d).clone();
    fewer.sources.retain(|source| source.id != ids[2]);
    s.doc_mut().put_drawing(Arc::new(fewer));
    s.open_sources();
    assert_eq!(s.linked.designs.len(), 1);
    run(&mut s, "drawing.close", json!({}));
    assert!(s.linked.designs.is_empty(), "closing the drawing lets them go");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A design saved with its model opens without evaluating it, as `doc.open` does.
#[test]
fn a_warm_design_is_not_evaluated() {
    let dir = folder("warm");
    let file = saved_box(&dir, "plate.sc", 20.0);
    let bytes = std::fs::read(&file).unwrap();
    let mut reader = solidcraft_io::DesignContainerReader::new(Cursor::new(&bytes)).unwrap();
    let saved = reader.geometry().expect("the file keeps its model");
    let doc = reader.read_document().unwrap();
    assert!(Model::adopt(&Arc::new(doc), &saved).is_ok());
    let (mut s, ids) = drawing_of(&[&file]);
    s.open_sources();
    assert_eq!(standing(&s, ids[0]), Ok(()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A front view at 1:1 of `source`, on the first sheet at `at`: its id.
fn view_of(s: &mut Session, source: u64, at: [f64; 2]) -> u64 {
    let mut d = (**s.drawing("test").unwrap()).clone();
    let id = d.allocate_id(&mut s.ids).unwrap();
    let sheet = d.sheets[0].id;
    let kind = ViewKind::Base { source, view: ViewDirection::Named(NamedView::Front) };
    let settings = ViewOverrides { scale: Some("1:1".parse().unwrap()), ..ViewOverrides::default() };
    d.add_views([Arc::new(View { id, sheet, kind, place: ViewPlace::At(at), settings, reference_margin_mm: 0.0 })]);
    d.validate().unwrap();
    s.doc_mut().put_drawing(Arc::new(d));
    id
}

/// How wide view `view` is on paper.
fn width(s: &Session, view: u64) -> f64 {
    let d = s.drawing("test").unwrap().clone();
    let box_mm = crate::drawing_view::on_sheet(s, &d, view, "test").unwrap().boundary;
    box_mm.max.x - box_mm.min.x
}

/// The provider's sheet once everything on it has been drawn.
fn all_drawn(s: &mut Session, sheet: u64) -> crate::drawing_scene::SheetView {
    crate::drawing_scene::extract(s, sheet).unwrap();
    let mut steps = 0;
    while crate::drawing_scene::step(s) {
        steps += 1;
        assert!(steps < 100, "the provider never settles");
    }
    crate::drawing_scene::extract(s, sheet).unwrap()
}

/// One sheet draws two designs: each view has its own design's size, and a dimension or a note on a view
/// reads that view's design. When a design goes, its view keeps its last lines and says it is lost, and
/// the other is left as it is.
#[test]
fn views_draw_their_own_sources_and_a_lost_one_keeps_its_lines() {
    let dir = folder("two");
    let plate = saved_box(&dir, "plate.sc", 40.0);
    let (mut s, linked) = drawing_of(&[&plate]);
    let d = s.drawing("test").unwrap().clone();
    let (own, sheet) = (d.sources[0].id, d.sheets[0].id);
    let small = view_of(&mut s, own, [100.0, 150.0]);
    let big = view_of(&mut s, linked[0], [250.0, 150.0]);
    s.open_sources();
    assert!((width(&s, small) - 10.0).abs() < 1e-6 && (width(&s, big) - 40.0).abs() < 1e-6);

    let across = json!({"linear": {"a": [0, 0, 0], "b": [40, 0, 0], "direction": "horizontal"}});
    let reply = run(&mut s, "drawing.dimension.add", json!({"view": ids::encode(big), "kind": across}));
    assert_eq!(reply["status"], "ok", "{reply}");
    assert!((reply["value"].as_f64().unwrap() - 40.0).abs() < 1e-9, "{reply}");
    let far = json!({"anchor": {"corner": {"view": ids::encode(big), "corner": [40, 0, 40]}}, "at": [30, 30], "content": "Far corner"});
    assert_eq!(run(&mut s, "drawing.note.add", far)["status"], "ok");
    let on_small = json!({"view": ids::encode(small), "kind": {"linear": {"a": [0, 0, 0], "b": [40, 0, 0], "direction": "horizontal"}}});
    assert!(s.execute("drawing.dimension.add", &on_small).is_err(), "the other design has no such corner");

    let built = crate::drawing_scene::build(&s, s.drawing("test").unwrap(), sheet).unwrap();
    assert!(built.views.iter().all(|v| v.status == crate::drawing_scene::Status::Drawn), "{:?}", built.views);
    let before = all_drawn(&mut s, sheet);
    assert!(before.views.iter().all(|v| v.status == crate::drawing_scene::Status::Drawn), "{:?}", before.views);
    let groups = before.scene.groups.len();

    std::fs::remove_file(&plate).unwrap();
    s.linked.close();
    s.open_sources();
    let after = all_drawn(&mut s, sheet);
    let status = |view: u64| after.views.iter().find(|v| v.view == view).map(|v| v.status.clone()).unwrap();
    assert_eq!(status(small), crate::drawing_scene::Status::Drawn);
    assert!(matches!(status(big), crate::drawing_scene::Status::Lost(why) if why.contains("not found")));
    assert!(after.scene.groups.iter().any(|g| g.object == big), "its last lines stay");
    assert_eq!(after.scene.groups.len(), groups - 1, "and its lost dimension, centred on what it lost, isn't drawn");
    assert!(after.dimensions.iter().all(|one| matches!(&one.status, solidcraft_doc::drawing::dimension::Status::Lost(_))));

    // Never drawn and missing: a placeholder to pick, and no lines. The sheet is still drawn.
    let (mut fresh, linked) = drawing_of(&[&plate]);
    let sheet = fresh.drawing("test").unwrap().sheets[0].id;
    let lost = view_of(&mut fresh, linked[0], [250.0, 150.0]);
    fresh.open_sources();
    let view = all_drawn(&mut fresh, sheet);
    assert!(matches!(&view.views[0].status, crate::drawing_scene::Status::Lost(_)));
    assert!(view.objects.iter().any(|o| o.object == lost), "{:?}", view.objects);
    std::fs::remove_dir_all(&dir).unwrap();
}
