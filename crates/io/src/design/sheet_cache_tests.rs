use std::collections::BTreeMap;
use std::io::Write as _;

use serde_json::{Value, json};
use solidcraft_doc::drawing::{Drawing, Source};
use solidcraft_doc::ids::IdSource;
use solidcraft_geom::drawing_sheet::{self, Orientation, PaperSize};
use solidcraft_geom::paper::Item;

use super::*;

/// A design with two drawings.
fn design() -> Document {
    let mut ids = IdSource::seeded(3890);
    let mut doc = Document::new("Plate");
    for name in ["Assembly", "Details"] {
        let a3 = drawing_sheet::Sheet::standard(PaperSize::A3, Orientation::Landscape);
        let taken = doc.drawings.iter().map(|d| d.id).collect();
        let mut d = Drawing::new(name, "en", "Sheet 1", a3, None, &taken, &mut ids).unwrap();
        let id = d.allocate_id(&mut ids).unwrap();
        d.sources.push(Source { id });
        doc.drawings.push(Arc::new(d));
    }
    doc
}

fn group(view: u64) -> Group {
    let items: Vec<Item> = Vec::new();
    Group { object: view, kind: ObjectKind::View, items }
}

fn view(id: u64) -> SavedView {
    let boundary = Aabb2 { min: Vec2 { x: -50.0, y: -10.0 }, max: Vec2 { x: 50.0, y: 10.0 } };
    SavedView { view: id, key: format!("key {id}"), group: Arc::new(group(id)), boundary, origin_mm: Vec2 { x: -25.0, y: -5.0 } }
}

fn caches(doc: &Document, keep_from: Option<&str>) -> SheetCaches {
    SheetCaches { lines: vec![SheetLines { drawing: doc.drawings[0].id, views: vec![view(7), view(9)] }], keep_from: keep_from.map(String::from) }
}

fn saved(doc: &Document, sheets: &SheetCaches, budget: usize) -> Vec<u8> {
    write_container(doc, DesignCompression::Fast, None, None, None, sheets, budget).unwrap()
}

fn read_from(bytes: &[u8], drawing: u64, document: &str) -> Option<ReadLines> {
    read(&mut PackageReader::new(Cursor::new(bytes)).ok()?, drawing, document).ok()
}

fn parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let names: Vec<String> = zip.file_names().map(String::from).collect();
    names
        .into_iter()
        .map(|name| {
            let mut bytes = Vec::new();
            zip.by_name(&name).unwrap().read_to_end(&mut bytes).unwrap();
            (name, bytes)
        })
        .collect()
}

/// A container with `parts`, as a save would write them.
fn with_parts(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Lines read back as they went in, current for their `document.json` and only for it, and a design
/// with a cache reads the same as one without. `document.json` doesn't change for them.
#[test]
fn lines_round_trip_and_change_no_document() {
    let doc = design();
    let bytes = saved(&doc, &caches(&doc, None), MAX_DESIGN_BYTES);
    let digest_of = design_bytes_digest(&bytes).unwrap();
    let read = read_from(&bytes, doc.drawings[0].id, &digest_of).unwrap();
    assert_eq!(read.current, env!("SOLIDCRAFT_SOURCE_COMMIT") != "unknown");
    assert_eq!(read.views, vec![view(7), view(9)]);
    assert!(!read_from(&bytes, doc.drawings[0].id, "another").unwrap().current);
    assert!(read_from(&bytes, doc.drawings[1].id, &digest_of).is_none(), "the other drawing has none");

    let plain = saved(&doc, &SheetCaches::default(), MAX_DESIGN_BYTES);
    assert_eq!(parts(&bytes)["document.json"], parts(&plain)["document.json"]);
    assert_eq!(design_bytes_digest(&plain), Some(digest_of));
    assert_eq!(crate::read_design(&bytes).unwrap(), crate::read_design(&plain).unwrap());
    assert!(!crate::write_design(&doc).windows(FOLDER.len()).any(|w| w == FOLDER.as_bytes()), "plain JSON never has them");
}

/// Lines that don't fit are left out, whole, and the save holds; a drawing the design doesn't have
/// and an empty list write nothing; more views than a cache holds are none.
#[test]
fn lines_that_dont_fit_are_left_out() {
    let doc = design();
    let sheets = caches(&doc, None);
    let names = |bytes: &[u8]| parts(bytes).keys().filter(|n| n.starts_with(FOLDER)).count();
    assert_eq!(names(&saved(&doc, &sheets, MAX_DESIGN_BYTES)), 3);
    assert_eq!(names(&saved(&doc, &sheets, 1)), 0, "no room");

    let stranger = SheetCaches { lines: vec![SheetLines { drawing: 1, views: vec![view(7)] }], keep_from: None };
    assert_eq!(names(&saved(&doc, &stranger, MAX_DESIGN_BYTES)), 0);
    let empty = SheetCaches { lines: vec![SheetLines { drawing: doc.drawings[0].id, views: Vec::new() }], keep_from: None };
    assert_eq!(names(&saved(&doc, &empty, MAX_DESIGN_BYTES)), 0);
    let too_many =
        SheetCaches { lines: vec![SheetLines { drawing: doc.drawings[0].id, views: (0..=MAX_VIEWS as u64).map(view).collect() }], keep_from: None };
    assert_eq!(names(&saved(&doc, &too_many, MAX_DESIGN_BYTES)), 0);
}

/// A save keeps each other drawing's entries from the file it came from, byte for byte once read;
/// lines given replace a drawing's old ones; a deleted drawing's go.
#[test]
fn a_save_keeps_the_other_drawings_lines_from_its_file() {
    let folder_on_disk = std::env::temp_dir().join(format!("sheet-cache-{}", std::process::id()));
    std::fs::create_dir_all(&folder_on_disk).unwrap();
    let path = folder_on_disk.join("old.solidcraft");
    let path_text = path.to_string_lossy().to_string();
    let doc = design();
    std::fs::write(&path, saved(&doc, &caches(&doc, None), MAX_DESIGN_BYTES)).unwrap();
    let old = parts(&std::fs::read(&path).unwrap());
    let old_lines =
        |p: &BTreeMap<String, Vec<u8>>| p.iter().filter(|(n, _)| n.starts_with(FOLDER)).map(|(n, b)| (n.clone(), b.clone())).collect::<Vec<_>>();

    let kept = saved(&doc, &SheetCaches { lines: Vec::new(), keep_from: Some(path_text.clone()) }, MAX_DESIGN_BYTES);
    assert_eq!(old_lines(&parts(&kept)), old_lines(&old));

    let other = SheetCaches { lines: vec![SheetLines { drawing: doc.drawings[1].id, views: vec![view(11)] }], keep_from: Some(path_text.clone()) };
    let both = parts(&saved(&doc, &other, MAX_DESIGN_BYTES));
    assert_eq!(both.keys().filter(|n| n.starts_with(FOLDER)).count(), 5, "its own and the kept ones");

    let renewed = SheetCaches { lines: vec![SheetLines { drawing: doc.drawings[0].id, views: vec![view(9)] }], keep_from: Some(path_text.clone()) };
    let renewed = parts(&saved(&doc, &renewed, MAX_DESIGN_BYTES));
    assert_eq!(renewed.keys().filter(|n| n.starts_with(FOLDER)).count(), 2, "the given lines, not the old ones as well");

    let mut less = doc.clone();
    less.drawings.remove(0);
    let gone = parts(&saved(&less, &SheetCaches { lines: Vec::new(), keep_from: Some(path_text.clone()) }, MAX_DESIGN_BYTES));
    assert_eq!(gone.keys().filter(|n| n.starts_with(FOLDER)).count(), 0, "a deleted drawing's go");

    let missing = SheetCaches { lines: Vec::new(), keep_from: Some(format!("{path_text}.none")) };
    assert_eq!(parts(&saved(&doc, &missing, MAX_DESIGN_BYTES)).keys().filter(|n| n.starts_with(FOLDER)).count(), 0, "no file: a miss");
    assert_eq!(design_file_digest(&path_text), design_bytes_digest(&std::fs::read(&path).unwrap()));
    assert!(read_sheet_lines(&path_text, doc.drawings[0].id, "x").is_some());
    assert!(read_sheet_lines(&format!("{path_text}.none"), doc.drawings[0].id, "x").is_none());
    std::fs::remove_dir_all(&folder_on_disk).unwrap();
}

/// Anything wrong in a cache is a miss, for the whole manifest or the one view, and the design still
/// opens: other formats, other builds (shown, not current), wrong ids and kinds, NaN, huge sizes.
#[test]
fn a_damaged_cache_is_a_miss_never_a_failed_open() {
    let doc = design();
    let id = doc.drawings[0].id;
    let good = parts(&saved(&doc, &caches(&doc, None), MAX_DESIGN_BYTES));
    let digest_of = digest(&good["document.json"]);
    let name = format!("{}manifest.json", folder(id));
    let seven = format!("{}views/{}.json", folder(id), ids::encode(7));
    let changed = |name: &str, bytes: Vec<u8>| {
        let mut p = good.clone();
        p.insert(name.to_string(), bytes);
        with_parts(&p)
    };
    let manifest = |change: &dyn Fn(&mut Value)| {
        let mut v: Value = serde_json::from_slice(&good[&name]).unwrap();
        change(&mut v);
        changed(&name, serde_json::to_vec(&v).unwrap())
    };
    let opens = |bytes: &[u8]| assert!(crate::read_design(bytes).is_ok(), "the design still opens");

    for bad in [
        changed(&name, b"not json".to_vec()),
        changed(&name, vec![0; MAX_MANIFEST + 1]),
        manifest(&|v| v["format"] = json!("solidcraft-sheet-cache/2")),
        manifest(&|v| v["extra"] = json!(1)),
        manifest(&|v| v["views"] = json!(vec![json!(["x", "k"]); MAX_VIEWS + 1])),
    ] {
        assert_eq!(read_from(&bad, id, &digest_of), None);
        opens(&bad);
    }
    let other_build = manifest(&|v| v["build"] = json!("another/build"));
    let read = read_from(&other_build, id, &digest_of).unwrap();
    assert!(!read.current && read.views.len() == 2, "shown, and drawn again");
    opens(&other_build);

    let view_json = |change: &dyn Fn(&mut Value)| {
        let mut v: Value = serde_json::from_slice(&good[&seven]).unwrap();
        change(&mut v);
        changed(&seven, serde_json::to_vec(&v).unwrap())
    };
    for bad in [
        changed(&seven, b"{".to_vec()),
        view_json(&|v| v["group"]["object"] = json!(8)),
        view_json(&|v| v["group"]["kind"] = json!("drawing_border")),
        view_json(&|v| v["boundary"] = json!([0.0, 0.0, 1e400, 0.0])),
        view_json(&|v| v["origin_mm"] = json!(["x", 0.0])),
        view_json(&|v| v["extra"] = json!(1)),
    ] {
        let read = read_from(&bad, id, &digest_of).unwrap();
        assert_eq!(read.views, vec![view(9)], "that view's miss alone");
        opens(&bad);
    }
    let mut missing = good.clone();
    missing.remove(&seven);
    assert_eq!(read_from(&with_parts(&missing), id, &digest_of).unwrap().views, vec![view(9)]);
    assert_eq!(read_from(b"not a container", id, &digest_of), None);
}

/// Fuzz: any change to any cache entry's bytes is a miss or some lines, never a panic or a failed open.
#[test]
fn mutated_entries_never_fail_an_open() {
    let doc = design();
    let id = doc.drawings[0].id;
    let good = parts(&saved(&doc, &caches(&doc, None), MAX_DESIGN_BYTES));
    let digest_of = digest(&good["document.json"]);
    let names: Vec<&String> = good.keys().filter(|n| n.starts_with(FOLDER)).collect();
    let mut state = 3890u64;
    let mut next = move || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    for _ in 0..2_000 {
        let name = names[next() % names.len()].clone();
        let mut bytes = good[&name].clone();
        for _ in 0..1 + next() % 8 {
            let at = next() % bytes.len();
            match next() % 3 {
                0 => bytes[at] = (next() % 256) as u8,
                1 => bytes.truncate(at.max(1)),
                _ => bytes.insert(at, b"\"0123456789{}[],:Ee-".get(next() % 20).copied().unwrap_or(b'0')),
            }
        }
        let mut p = good.clone();
        p.insert(name, bytes);
        let file = with_parts(&p);
        let _ = read_from(&file, id, &digest_of);
        assert!(crate::read_design(&file).is_ok());
    }
}
