use std::collections::BTreeSet;

use serde_json::Value;
use solidcraft_geom::drawing_sheet::{self, Orientation, PaperSize};

use super::*;
use crate::drawing::OnSheet;
use crate::ids::IdSource;

const RECT: [f64; 4] = [10.0, 20.0, 60.0, 45.0];

/// A PNG's header, saying these pixels: all a cap reads.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b
}

/// A JPEG's start, saying these pixels.
fn jpeg(w: u16, h: u16) -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8];
    b.extend(h.to_be_bytes());
    b.extend(w.to_be_bytes());
    b.extend([0; 12]);
    b
}

/// A drawing on one A3 sheet with one image of `bytes`: the drawing and the image's id.
fn pictured(bytes: &[u8]) -> (Drawing, u64) {
    let mut id_source = IdSource::seeded(5426);
    let a3 = drawing_sheet::Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let mut d = Drawing::new("Pictured", "en", "Sheet 1", a3, None, &BTreeSet::new(), &mut id_source).unwrap();
    let id = d.allocate_id(&mut id_source).unwrap();
    d.add_images([Arc::new(SheetImage::from_file(id, d.sheets[0].id, RECT, bytes).unwrap())]);
    d.validate().unwrap();
    (d, id)
}

fn file(d: &Drawing) -> Value {
    serde_json::from_str(&d.to_json().unwrap()).unwrap()
}

fn refused(d: &Drawing, edit: impl FnOnce(&mut Value)) -> String {
    let mut v = file(d);
    edit(&mut v);
    Drawing::from_json(&v.to_string()).map_or_else(|e| e.to_string(), |_| "read".into())
}

#[test]
fn an_image_is_saved_with_its_bytes_and_reads_back_the_same() {
    let bytes = png(640, 480);
    let (d, id) = pictured(&bytes);
    let saved = d.to_json().unwrap();
    let read = Drawing::from_json(&saved).unwrap();
    assert_eq!(read, d);
    assert_eq!(read.to_json().unwrap(), saved);
    let image = read.image(id).unwrap();
    assert_eq!((image.format.as_str(), image.pixels, image.rect_mm), ("png", [640, 480], RECT));
    assert_eq!(base64_decode(&image.data).unwrap(), bytes);
    assert!(read.has_object(id) && read.image(id + 1).is_none());
}

#[test]
fn a_drawing_without_images_saves_nothing_for_them_and_older_ones_read() {
    let (mut d, id) = pictured(&png(1, 1));
    d.remove_objects(&OnSheet { images: vec![id], ..Default::default() });
    let v = file(&d);
    assert!(v.get("images").is_none(), "{v}");
    assert_eq!(Drawing::from_json(&v.to_string()).unwrap(), d);
}

#[test]
fn an_image_goes_with_its_sheet_and_ids_stay_apart() {
    let (mut d, id) = pictured(&png(1, 1));
    let sheet = d.sheets[0].id;
    assert_eq!(d.on_sheet(sheet).images, [id]);
    assert!(!d.on_sheet(sheet).is_empty());
    let taken = d.ids().collect::<BTreeSet<_>>();
    assert!(taken.contains(&id));
    d.remove_objects(&d.on_sheet(sheet));
    assert!(d.images.is_empty());
}

#[test]
fn the_caps_are_read_from_the_file_before_anything_decodes_it() {
    let make = |b: &[u8]| SheetImage::from_file(1, 2, RECT, b).map(|i| i.pixels).map_err(|e| e.to_string());
    assert_eq!(make(&png(8_000, 8_000)), Ok([8_000, 8_000]));
    assert_eq!(make(&jpeg(8_000, 8_000)), Ok([8_000, 8_000]));
    // The limits, one over, and sizes whose product would overflow 32 bits.
    for (w, h) in [(8_001, 8_000), (64_000_001, 1), (1, 64_000_001), (65_536, 65_536), (u32::MAX, u32::MAX)] {
        assert!(make(&png(w, h)).is_err_and(|e| e.contains("pixels")), "{w} by {h}");
    }
    assert!(make(&jpeg(u16::MAX, u16::MAX)).is_err_and(|e| e.contains("pixels")));
    let not_images: [&[u8]; 6] = [b"", b"GIF89a", b"\x89PNG", &png(0, 5), &png(5, 0), &jpeg(0, 5)];
    for not_image in not_images {
        assert!(make(not_image).is_err_and(|e| e.contains("PNG or JPEG")), "{not_image:?}");
    }
    let mut full = png(10, 10);
    full.resize(MAX_IMAGE_BYTES, 0);
    assert_eq!(make(&full), Ok([10, 10]));
    full.push(0);
    assert!(make(&full).is_err_and(|e| e.contains("MiB")));
}

#[test]
fn a_file_that_lies_is_refused_on_reading() {
    let (d, _) = pictured(&png(10, 10));
    let image = |v: &mut Value| v["images"][0].take();
    let with = |key: &'static str, value: Value| {
        move |v: &mut Value| {
            let mut i = image(v);
            i[key] = value;
            v["images"][0] = i;
        }
    };
    assert!(refused(&d, with("pixels", serde_json::json!([1, 1]))).contains("disagree"));
    assert!(refused(&d, with("format", "jpeg".into())).contains("disagree"));
    assert!(refused(&d, with("format", "gif".into())).contains("PNG or a JPEG"));
    assert!(refused(&d, with("data", "!!!".into())).contains("base64"));
    assert!(refused(&d, with("data", base64_encode(&png(9_000, 9_000)).into())).contains("pixels"));
    assert!(refused(&d, with("data", "".into())).contains("PNG or JPEG"));
    assert!(refused(&d, with("pixels", serde_json::json!([0, 10]))).contains("pixels"));
    assert!(refused(&d, with("pixels", serde_json::json!([65_536, 65_536]))).contains("pixels"));
    assert!(refused(&d, with("sheet", ids::encode(1 << 41).into())).contains("doesn't have"));
    assert!(refused(&d, with("path", "C:/secret.png".into())).contains("path"), "an unknown field is damage");
}

#[test]
fn a_rectangle_is_on_paper_and_has_a_size() {
    let (d, _) = pictured(&png(10, 10));
    for rect in [
        [60.0, 20.0, 10.0, 45.0],
        [10.0, 45.0, 60.0, 20.0],
        [10.0, 20.0, 10.0, 45.0],
        [10.0, 20.0, 60.0, 20.0],
        [1e300, 20.0, 2e300, 45.0],
        [f64::NAN; 4],
    ] {
        let mut bad = d.clone();
        Arc::make_mut(&mut bad.images[0]).rect_mm = rect;
        assert!(bad.validate().is_err_and(|e| e.to_string().contains("rectangle")), "{rect:?}");
    }
}

#[test]
fn a_drawings_images_keep_their_counts() {
    let (d, id) = pictured(&png(1, 1));
    let again = |d: &mut Drawing, n: u64, data: &str| {
        let mut i = (*d.images[0]).clone();
        i.id = id + n;
        i.data = data.into();
        d.add_images([Arc::new(i)]);
    };
    // One file's cap and the cap between them, checked on every edit from the text's length alone.
    let mut big = d.clone();
    Arc::make_mut(&mut big.images[0]).data = "A".repeat(encoded(MAX_IMAGE_BYTES) + 1);
    assert!(big.validate().is_err_and(|e| e.to_string().contains("its file")));
    let mut many = d.clone();
    let full = "A".repeat(encoded(MAX_IMAGE_BYTES));
    Arc::make_mut(&mut many.images[0]).data.clone_from(&full);
    again(&mut many, 1, &full);
    let room = encoded(MAX_IMAGES_BYTES) - 2 * full.len();
    again(&mut many, 2, &"A".repeat(room));
    many.validate().unwrap();
    again(&mut many, 3, "A");
    assert!(many.validate().is_err_and(|e| e.to_string().contains("between them")));
    let mut crowd = d.clone();
    (1..=MAX_IMAGES as u64).for_each(|n| again(&mut crowd, n, "AAAA"));
    assert!(crowd.validate().is_err_and(|e| e.to_string().contains("at most")));
    let mut twins = d.clone();
    let twin = twins.images[0].clone();
    twins.images.push(twin);
    assert!(twins.validate().is_err());
}
