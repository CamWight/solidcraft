use std::path::PathBuf;

use serde_json::{Value, json};
use solidcraft_doc::drawing::Source;
use solidcraft_doc::{Document, Model};
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
