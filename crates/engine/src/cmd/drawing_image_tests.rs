use std::path::PathBuf;

use serde_json::json;
use solidcraft_doc::canvas::base64_decode;
use solidcraft_doc::drawing::image::MAX_IMAGE_PIXELS;
use solidcraft_io::vfs::{self, PathScope};

use super::*;
use crate::schema::tests::conforms;

const RECT: [f64; 4] = [10.0, 20.0, 60.0, 45.0];

/// A PNG's header, saying these pixels.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b
}

/// A folder of this test's own, holding `bytes` as `logo.png`: the folder and the file.
fn on_disk(test: &str, bytes: &[u8]) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("solidcraft-drawing-image-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("logo.png");
    std::fs::write(&file, bytes).unwrap();
    (dir, file.to_string_lossy().into_owned())
}

/// A session with a box and a drawing of it, open, on the sheet it returns.
fn drawing() -> (Session, String) {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 80, "width": 40, "height": 30})).unwrap();
    s.execute("drawing.create", &json!({})).unwrap();
    let sheet = ids::encode(s.drawing("test").unwrap().sheets[0].id);
    (s, sheet)
}

fn refused(s: &mut Session, p: Value) -> String {
    s.execute(ADD, &p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("accepted: {p}"))
}

#[test]
fn an_image_is_read_saved_without_its_path_and_one_undo_step() {
    let bytes = png(640, 480);
    let (dir, file) = on_disk("saved", &bytes);
    let (mut s, sheet) = drawing();
    let reply = s.execute(ADD, &json!({"sheet": sheet, "file": file, "rect_mm": RECT})).unwrap();
    let schema = output_schema_for::<AddResult>();
    assert!(conforms(schema.as_value(), schema.as_value(), &reply), "{reply}");
    assert_eq!((&reply["format"], &reply["pixels"]), (&json!("png"), &json!([640, 480])), "{reply}");

    // The original file is gone: the design has its bytes, and doesn't know where they came from.
    std::fs::remove_dir_all(&dir).unwrap();
    let record = s.drawing("test").unwrap().clone();
    let id = ids::decode(reply["image"].as_str().unwrap()).unwrap();
    assert_eq!(base64_decode(&record.image(id).unwrap().data).unwrap(), bytes);
    assert!(!record.to_json().unwrap().contains("logo"));

    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.drawing("test").unwrap().images.is_empty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.drawing("test").unwrap().image(id).is_some());

    let item = json!({"type": "drawing", "drawing": ids::encode(record.id), "object": ids::encode(id)});
    s.execute("selection.delete", &json!({"items": [item]})).unwrap();
    assert!(s.drawing("test").unwrap().images.is_empty());
}

#[test]
fn what_isnt_an_image_or_breaks_a_cap_is_refused_and_changes_nothing() {
    let (dir, bomb) = on_disk("refused", &png(100_000, 100_000));
    let (mut s, sheet) = drawing();
    let before = s.drawing("test").unwrap().clone();
    let side = (MAX_IMAGE_PIXELS as f64).sqrt() as u32 + 1;
    let text = dir.join("words.png");
    std::fs::write(&text, b"not a picture").unwrap();
    let big = dir.join("big.png");
    std::fs::write(&big, vec![0; MAX_IMAGE_BYTES + 1]).unwrap();
    let over = dir.join("over.png");
    std::fs::write(&over, png(side, side)).unwrap();
    let path = |p: &PathBuf| p.to_string_lossy().into_owned();
    let dir_text = path(&dir);
    let missing = path(&dir.join("missing.png"));
    let files: [&str; 10] = [&bomb, &path(&text), &path(&big), &path(&over), &dir_text, &missing, "", " ", "a\0b", &"x".repeat(4096)];
    for file in files {
        let why = refused(&mut s, json!({"sheet": sheet, "file": file, "rect_mm": RECT}));
        assert!(!why.is_empty(), "{file:?}");
    }
    let (good_dir, good) = on_disk("refused-good", &png(4, 4));
    assert!(refused(&mut s, json!({"sheet": "no such sheet", "file": good, "rect_mm": RECT})).contains("sheet"));
    for rect in [[60, 20, 10, 45], [10, 20, 10, 45], [0, 0, 1_000_000_000, 1]] {
        assert!(refused(&mut s, json!({"sheet": sheet, "file": good, "rect_mm": rect})).contains("rectangle"), "{rect:?}");
    }
    assert_eq!(**s.drawing("test").unwrap(), *before);
    for d in [dir, good_dir] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

/// A file outside the folders an agent may read is refused by the same check every other read takes.
#[test]
fn a_file_outside_the_allowed_folders_is_refused() {
    let (dir, file) = on_disk("scope", &png(4, 4));
    let (allowed, _) = on_disk("scope-allowed", &png(4, 4));
    let (mut s, sheet) = drawing();
    let p = json!({"sheet": sheet, "file": file, "rect_mm": RECT});
    let answer = vfs::scoped(Some(PathScope::new([allowed.clone()])), || s.execute(ADD, &p));
    assert!(answer.is_err(), "{answer:?}");
    assert!(s.drawing("test").unwrap().images.is_empty());
    for d in [dir, allowed] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

/// A drawing that couldn't be saved again is refused when the image is added, not at the save.
#[test]
fn an_image_a_drawing_has_no_room_for_is_refused() {
    let mut full = png(4, 4);
    full.resize(MAX_IMAGE_BYTES, 0);
    let (dir, file) = on_disk("room", &full);
    let (mut s, sheet) = drawing();
    let p = json!({"sheet": sheet, "file": file, "rect_mm": RECT});
    s.execute(ADD, &p).unwrap();
    s.execute(ADD, &p).unwrap();
    assert!(refused(&mut s, p).contains("between them"));
    s.drawing("test").unwrap().to_json().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
