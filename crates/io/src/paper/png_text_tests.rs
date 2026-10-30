//! Text in a sheet PNG (#4890 B2b): a run's glyph outlines, filled nonzero where the walk puts them.

use std::sync::Arc;

use solidcraft_geom::paper::text::{Cluster, Font, FontFlags, FontGlyph, FontMetrics, PlacedGlyph, TextRun};
use solidcraft_geom::paper::{Group, Item, ObjectKind, Scene};
use solidcraft_geom::{Loop2, Vec2};

use super::*;

fn p(x: f64, y: f64) -> Vec2 {
    Vec2::new(x, y)
}

/// Anticlockwise from `min`.
fn rectangle(min: Vec2, max: Vec2) -> Loop2 {
    Loop2::polygon(&[min, p(max.x, min.y), max, p(min.x, max.y)])
}

/// A 20 × 10 mm sheet of `runs` set in `font`; at 254 dpi a millimetre is 10 pixels.
fn sheet(runs: Vec<TextRun>, font: Font) -> Scene {
    let items = runs.into_iter().map(Item::Text).collect();
    Scene { fonts: vec![font], ..Scene::new(p(20.0, 10.0), vec![Group { object: 1, kind: ObjectKind::TitleBlock, items }]) }
}

/// The grey of each pixel, rows top down.
fn greys(scene: &Scene, band_bytes: usize) -> (Vec<u8>, Vec<u8>) {
    let mut out = Vec::new();
    write(&scene.prepare().unwrap(), PngOptions { dpi: 254.0, colour: PngColour::Grey }, &mut out, band_bytes).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(out.clone())).read_info().unwrap();
    let mut samples = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut samples).unwrap();
    (out, samples)
}

/// One glyph made of two overlapping boxes turning the same way, 9 × 7 mm, set at (2, 1): where
/// they overlap is inside both, so nonzero fills it, where even-odd would leave it white.
#[test]
fn a_glyph_is_filled_nonzero_where_the_walk_puts_it() {
    let metrics = FontMetrics {
        ascent: 900,
        descent: -200,
        cap_height: 700,
        bbox: [0, 0, 900, 700],
        italic_angle: 0.0,
        flags: FontFlags::default(),
        stem_v: 80,
    };
    let outline = vec![rectangle(p(0.0, 0.0), p(600.0, 700.0)), rectangle(p(300.0, 0.0), p(900.0, 700.0))];
    let font = Font {
        id: [1; 32],
        bytes: Arc::from(vec![1u8]),
        face: 0,
        postscript_name: "MadeUp".into(),
        num_glyphs: 2,
        units_per_em: 1000,
        metrics,
        glyphs: [(1, FontGlyph { advance: 1000, outline })].into(),
    };
    let run = TextRun {
        font: [1; 32],
        scale: 0.01,
        origin: p(2.0, 1.0),
        angle: 0.0,
        slant: 0.0,
        glyphs: vec![PlacedGlyph { id: 1, x: 0.0, rise: 0.0, advance: 10.0 }],
        text: "A".into(),
        clusters: vec![Cluster { glyphs: 0..1, text: 0..1 }],
        rtl: false,
    };
    let (_, grey) = greys(&sheet(vec![run], font), BAND_BYTES);
    // The glyph covers x 2 to 11 mm and y 1 to 8 mm: columns 20 to 110, rows 20 to 90 from the top.
    let at = |x: usize, y: usize| grey[y * 200 + x];
    for (x, y, value, what) in [
        (35, 55, 0, "the first box alone"),
        (65, 55, 0, "where the boxes overlap"),
        (95, 55, 0, "the second box alone"),
        (115, 55, 255, "right of the glyph"),
        (65, 95, 255, "below the baseline"),
        (65, 15, 255, "above the cap height"),
    ] {
        assert_eq!(at(x, y), value, "{what} at ({x}, {y})");
    }
}

/// Real text in the drawing font, read left to right and right to left, darkens the sheet only
/// within its glyphs' outlines, and gives the same bytes every time. Drawn in bands of a few rows, it
/// is the sheet drawn as one band but for pixels next to a band's edge, where tiny-skia cuts the
/// glyphs' lines: each of those at most a sixteenth of coverage (one step of its 4 x 4 samples) off.
#[test]
fn real_text_lands_inside_its_outlines_whatever_the_band() {
    use solidcraft_sketch::{TextStyle, paper_text};
    for line in ["Ø12 ±0.1 30°", "קו 12"] {
        let (runs, mut fonts) = paper_text(line, None, p(2.0, 3.0), 3.5, 0.0, &TextStyle::default()).unwrap().unwrap();
        let font = fonts.remove(0);
        // The outlines' extent, each piece sampled 16 times along its length.
        let points: Vec<Vec2> = runs
            .iter()
            .flat_map(|r| r.outlines(&font))
            .flat_map(|l| l.segs)
            .flat_map(|s| (0..=16).map(move |i| s.point_at(f64::from(i) / 16.0)))
            .collect();
        let bounds = points.iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |(x0, y0, x1, y1), q| {
            (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y))
        });
        let scene = sheet(runs, font);
        // The sheet is 100 rows: one band.
        let (bytes, grey) = greys(&scene, BAND_BYTES);
        assert_eq!(greys(&scene, BAND_BYTES).0, bytes, "{line}: the same bytes again");
        for rows in [3, 7] {
            let (again, banded) = (greys(&scene, 200 * 4 * rows), greys(&scene, 200 * 4 * rows));
            assert_eq!(again.0, banded.0, "{line}: the same bytes again in bands of {rows} rows");
            for (i, (a, b)) in grey.iter().zip(&banded.1).enumerate().filter(|(_, (a, b))| a != b) {
                // A band's first and last two rows.
                let at = (i / 200) % rows;
                assert!(a.abs_diff(*b) <= 16 && at.min(rows - 1 - at) <= 1, "{line}, bands of {rows} rows: ({}, {}) {a} {b}", i % 200, i / 200);
            }
        }
        let dark: Vec<(usize, usize)> = (0..100).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|&(x, y)| grey[y * 200 + x] < 128).collect();
        assert!(!dark.is_empty(), "{line} draws nothing");
        // A pixel's centre in sheet millimetres, a pixel's width of slack for the curves' control points.
        for (x, y) in dark {
            let (mx, my) = ((x as f64 + 0.5) / 10.0, 10.0 - (y as f64 + 0.5) / 10.0);
            assert!(mx >= bounds.0 - 0.1 && mx <= bounds.2 + 0.1 && my >= bounds.1 - 0.1 && my <= bounds.3 + 0.1, "{line}: ({x}, {y})");
        }
    }
}
