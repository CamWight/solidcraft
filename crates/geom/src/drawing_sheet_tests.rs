use super::*;

const EPS: f64 = 1e-9;

fn iso_layout(size: PaperSize, orientation: Orientation) -> SheetLayout {
    DrawingBorderProfile::iso().layout(&Sheet::standard(size, orientation)).unwrap()
}

fn lengths(fields: &[Field]) -> Vec<f64> {
    fields.iter().map(|f| f.to_mm - f.from_mm).collect()
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < EPS)
}

#[test]
fn every_size_is_its_published_millimetres() {
    use PaperSize::*;
    let expected = [
        (A0, 841.0, 1189.0),
        (A1, 594.0, 841.0),
        (A2, 420.0, 594.0),
        (A3, 297.0, 420.0),
        (A4, 210.0, 297.0),
        (A5, 148.0, 210.0),
        (Letter, 215.9, 279.4),
        (Legal, 215.9, 355.6),
        (AnsiA, 215.9, 279.4),
        (AnsiB, 279.4, 431.8),
        (AnsiC, 431.8, 558.8),
        (AnsiD, 558.8, 863.6),
        (AnsiE, 863.6, 1117.6),
        (AnsiF, 711.2, 1016.0),
        (ArchA, 228.6, 304.8),
        (ArchB, 304.8, 457.2),
        (ArchC, 457.2, 609.6),
        (ArchD, 609.6, 914.4),
        (ArchE, 914.4, 1219.2),
        (ArchE1, 762.0, 1066.8),
    ];
    assert_eq!(expected.len(), PaperSize::ALL.len());
    for (size, short, long) in expected {
        let (s, l) = size.sides_mm();
        assert!((s - short).abs() < EPS && (l - long).abs() < EPS, "{size:?}: {s} x {l}");
    }
}

#[test]
fn landscape_is_the_default_and_either_orientation_turns_every_size() {
    let sheet: Sheet = serde_json::from_value(serde_json::json!({"size": {"standard": "a3"}})).unwrap();
    assert_eq!(sheet.orientation, Orientation::Landscape);
    assert_eq!(sheet.size_mm().unwrap(), Vec2::new(420.0, 297.0));
    for size in PaperSize::ALL {
        let across = Sheet::standard(size, Orientation::Landscape).size_mm().unwrap();
        let upright = Sheet::standard(size, Orientation::Portrait).size_mm().unwrap();
        assert!(across.x > across.y && upright.x < upright.y, "{size:?}");
        assert_eq!(across, Vec2::new(upright.y, upright.x));
    }
    // A custom sheet's sides may come in either order; the orientation lays it.
    for sides_mm in [[500.0, 700.0], [700.0, 500.0]] {
        let sheet = Sheet { size: SheetSize::Custom { sides_mm }, orientation: Orientation::Portrait };
        assert_eq!(sheet.size_mm().unwrap(), Vec2::new(500.0, 700.0));
    }
}

#[test]
fn a3_landscape_has_a_390_by_277_drawing_border_and_8_by_6_fields() {
    let layout = iso_layout(PaperSize::A3, Orientation::Landscape);
    assert_eq!(layout.sheet_border, Aabb2 { min: Vec2::ZERO, max: Vec2::new(420.0, 297.0) });
    assert_eq!(layout.drawing_border, Aabb2 { min: Vec2::new(20.0, 10.0), max: Vec2::new(410.0, 287.0) });
    assert_eq!(layout.drawing_border_line_mm, 0.7);
    // Columns: 50 mm out from the centre line at 210; the left corner takes the 40 mm left over.
    assert!(close(&lengths(&layout.columns), &[40.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 50.0]));
    assert_eq!(layout.columns.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), ["1", "2", "3", "4", "5", "6", "7", "8"]);
    assert!(layout.columns.iter().any(|f| f.to_mm == 210.0));
    // Rows: 148.5 above and below 10 and 287, so two of 50 and a 38.5 mm corner each way.
    assert!(close(&lengths(&layout.rows), &[38.5, 50.0, 50.0, 50.0, 50.0, 38.5]));
    assert_eq!(layout.rows.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), ["A", "B", "C", "D", "E", "F"]);
    let (top, bottom) = (layout.rows.first().unwrap(), layout.rows.last().unwrap());
    assert_eq!((top.from_mm, top.to_mm), (248.5, 287.0));
    assert_eq!((bottom.from_mm, bottom.to_mm), (10.0, 48.5));
    assert_eq!(layout.label_sides, Sides::ALL);
}

#[test]
fn a3_landscape_has_iso_5457s_grid_border_centring_and_trimming_marks() {
    let layout = iso_layout(PaperSize::A3, Orientation::Landscape);
    // 5 mm outside the drawing border (20, 10)-(410, 287), the left side too.
    assert_eq!(layout.grid_border, Aabb2 { min: Vec2::new(15.0, 5.0), max: Vec2::new(415.0, 292.0) });
    // On the centre lines x = 210 and y = 148.5, from the grid border to 10 mm inside the drawing border.
    let mark = |a: (f64, f64), b: (f64, f64)| [Vec2::new(a.0, a.1), Vec2::new(b.0, b.1)];
    assert_eq!(
        layout.centring_marks,
        [
            mark((210.0, 5.0), (210.0, 20.0)),
            mark((210.0, 292.0), (210.0, 277.0)),
            mark((15.0, 148.5), (30.0, 148.5)),
            mark((415.0, 148.5), (400.0, 148.5))
        ]
    );
    // At each corner, 10 along the bottom or top edge by 5 up or down, and 5 by 10 along the side.
    let rect = |x0, y0, x1, y1| Aabb2 { min: Vec2::new(x0, y0), max: Vec2::new(x1, y1) };
    assert_eq!(
        layout.trimming_marks,
        [
            rect(0.0, 0.0, 10.0, 5.0),
            rect(0.0, 0.0, 5.0, 10.0),
            rect(410.0, 0.0, 420.0, 5.0),
            rect(415.0, 0.0, 420.0, 10.0),
            rect(0.0, 292.0, 10.0, 297.0),
            rect(0.0, 287.0, 5.0, 297.0),
            rect(410.0, 292.0, 420.0, 297.0),
            rect(415.0, 287.0, 420.0, 297.0),
        ]
    );
    // On the sheet's centre lines, not the drawing border's: a deeper bottom margin leaves them at 148.5.
    let sheet = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let iso = DrawingBorderProfile::iso();
    let deep = DrawingBorderProfile { margins: Margins { bottom_mm: 30.0, ..iso.margins }, ..iso }.layout(&sheet).unwrap();
    assert!(deep.centring_marks.iter().skip(2).all(|[a, b]| a.y == 148.5 && b.y == 148.5), "{:?}", deep.centring_marks);
    // A profile can leave the marks out.
    let plain = DrawingBorderProfile { centring_mark_mm: None, trimming_mark_mm: None, ..iso }.layout(&sheet).unwrap();
    assert!(plain.centring_marks.is_empty() && plain.trimming_marks.is_empty());
    assert_eq!(plain.grid_border, layout.grid_border);
}

/// On every standard size either way, ISO 5457's marks are where the standard puts them.
#[test]
fn every_iso_sheet_has_its_marks_on_its_centre_lines_and_in_its_corners() {
    for size in PaperSize::ALL {
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            let layout = iso_layout(size, orientation);
            let (size_mm, drawing, grid) = (layout.sheet_border.max, layout.drawing_border, layout.grid_border);
            let centre = Vec2::new(size_mm.x / 2.0, size_mm.y / 2.0);
            // Bottom, top, left and right.
            let expected = [
                [Vec2::new(centre.x, grid.min.y), Vec2::new(centre.x, drawing.min.y + 10.0)],
                [Vec2::new(centre.x, grid.max.y), Vec2::new(centre.x, drawing.max.y - 10.0)],
                [Vec2::new(grid.min.x, centre.y), Vec2::new(drawing.min.x + 10.0, centre.y)],
                [Vec2::new(grid.max.x, centre.y), Vec2::new(drawing.max.x - 10.0, centre.y)],
            ];
            assert_eq!(layout.centring_marks.len(), 4);
            for (mark, [from, to]) in layout.centring_marks.iter().zip(expected) {
                assert!(mark[0].dist(from) < EPS && mark[1].dist(to) < EPS, "{size:?} {orientation:?}: {mark:?}");
            }
            assert_eq!(layout.trimming_marks.len(), 8);
            for (i, r) in layout.trimming_marks.iter().enumerate() {
                assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= size_mm.x && r.max.y <= size_mm.y, "{size:?}: {r:?}");
                let on_corner = (r.min.x == 0.0 || r.max.x == size_mm.x) && (r.min.y == 0.0 || r.max.y == size_mm.y);
                let shape = if i % 2 == 0 { Vec2::new(10.0, 5.0) } else { Vec2::new(5.0, 10.0) };
                assert!(on_corner && r.size().dist(shape) < EPS, "{size:?} {orientation:?}: {r:?}");
                // In the margins: up to the grid reference border, never across it.
                assert!(!r.inflated(-EPS).meets(&grid), "{size:?} {orientation:?}: {r:?}");
            }
        }
    }
}

#[test]
fn the_iso_rule_gives_the_standard_field_counts_for_a0_to_a4_either_way() {
    // ISO 5457's table, long side by short side.
    for (size, long, short) in
        [(PaperSize::A0, 24, 16), (PaperSize::A1, 16, 12), (PaperSize::A2, 12, 8), (PaperSize::A3, 8, 6), (PaperSize::A4, 6, 4)]
    {
        let across = iso_layout(size, Orientation::Landscape);
        assert_eq!((across.columns.len(), across.rows.len()), (long, short), "{size:?} landscape");
        let upright = iso_layout(size, Orientation::Portrait);
        assert_eq!((upright.columns.len(), upright.rows.len()), (short, long), "{size:?} portrait");
    }
    // A1 across: 400.5 left of the centre line and 410.5 right of it; eight fields each way.
    let a1 = lengths(&iso_layout(PaperSize::A1, Orientation::Landscape).columns);
    assert!((a1.first().unwrap() - 50.5).abs() < EPS && (a1.last().unwrap() - 60.5).abs() < EPS, "{a1:?}");
}

/// The label sides are worked out for each sheet, so one profile labels every size as ISO 5457 does.
#[test]
fn one_iso_profile_labels_a4_on_its_top_and_right_and_every_other_size_all_round() {
    let iso = DrawingBorderProfile::iso();
    let top_right = Sides { top: true, bottom: false, left: false, right: true };
    let bottom_left = Sides { top: false, bottom: true, left: true, right: false };
    let fixed = DrawingBorderProfile { label_sides: LabelSides::Fixed(bottom_left), ..iso };
    for size in PaperSize::ALL {
        for orientation in [Orientation::Portrait, Orientation::Landscape] {
            let sheet = Sheet::standard(size, orientation);
            let expected = if size == PaperSize::A4 { top_right } else { Sides::ALL };
            assert_eq!(iso.layout(&sheet).unwrap().label_sides, expected, "{size:?} {orientation:?}");
            assert_eq!(fixed.layout(&sheet).unwrap().label_sides, bottom_left, "{size:?} {orientation:?}");
        }
    }
}

#[test]
fn an_equal_division_cuts_the_drawing_border_evenly() {
    let sheet = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let profile = DrawingBorderProfile { grid: GridRule::Equal { columns: 8, rows: 6 }, ..DrawingBorderProfile::iso() };
    let layout = profile.layout(&sheet).unwrap();
    assert!(close(&lengths(&layout.columns), &[48.75; 8]));
    assert!(close(&lengths(&layout.rows), &[277.0 / 6.0; 6]));
    assert!((277.0 / 6.0 - 46.166_666_666_7_f64).abs() < 1e-9);
    let none = DrawingBorderProfile { grid: GridRule::None, ..profile }.layout(&sheet).unwrap();
    assert!(none.columns.is_empty() && none.rows.is_empty());
    assert_eq!(none.drawing_border, layout.drawing_border);
}

#[test]
fn rows_skip_i_and_o_and_letters_go_on_past_z() {
    let a0 = iso_layout(PaperSize::A0, Orientation::Landscape);
    let rows: String = a0.rows.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(rows, "ABCDEFGHJKLMNPQR");
    assert_eq!(letters(0, true), "A");
    assert_eq!(letters(23, true), "Z");
    assert_eq!(letters(24, true), "AA");
    assert_eq!(letters(25, true), "AB");
    assert_eq!(letters(24 + 24 * 24 - 1, true), "ZZ");
    assert_eq!(letters(24 + 24 * 24, true), "AAA");
    assert_eq!(letters(8, false), "I");
    assert_eq!(letters(25, false), "Z");
    assert_eq!(letters(26, false), "AA");
    assert_eq!(letters(usize::MAX, true).len(), letters(usize::MAX - 1, true).len());
    for i in 0..2000 {
        assert!(!letters(i, true).contains(['I', 'O']), "{i}");
    }
    let sheet = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let every = DrawingBorderProfile { skip_i_and_o: false, grid: GridRule::Equal { columns: 1, rows: 10 }, ..DrawingBorderProfile::iso() };
    let rows: String = every.layout(&sheet).unwrap().rows.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(rows, "ABCDEFGHIJ");
}

#[test]
fn hostile_sizes_and_profiles_are_refused() {
    let custom = |a: f64, b: f64| Sheet { size: SheetSize::Custom { sides_mm: [a, b] }, orientation: Orientation::Landscape };
    for (a, b) in [(0.0, 100.0), (-1.0, 100.0), (f64::NAN, 100.0), (100.0, f64::INFINITY), (5_000.000_001, 100.0), (f64::MIN_POSITIVE, f64::MAX)] {
        assert!(custom(a, b).size_mm().is_err(), "{a} x {b}");
    }
    assert_eq!(custom(5000.0, 5000.0).size_mm().unwrap(), Vec2::new(5000.0, 5000.0));
    let a3 = Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let iso = DrawingBorderProfile::iso();
    let with_margins =
        |left_mm, right_mm, top_mm, bottom_mm| DrawingBorderProfile { margins: Margins { left_mm, right_mm, top_mm, bottom_mm }, ..iso };
    // Refused on its own, with no sheet at all: validate() refuses it, and so layout() does too.
    for profile in [
        with_margins(-1.0, 10.0, 10.0, 10.0),
        with_margins(f64::NAN, 10.0, 10.0, 10.0),
        with_margins(20.0, f64::INFINITY, 10.0, 10.0),
        with_margins(20.0, 10.0, 5000.1, 10.0),
        DrawingBorderProfile { drawing_border_line_mm: 0.0, ..iso },
        DrawingBorderProfile { drawing_border_line_mm: -0.7, ..iso },
        DrawingBorderProfile { drawing_border_line_mm: f64::NAN, ..iso },
        DrawingBorderProfile { drawing_border_line_mm: 10.000_001, ..iso },
        DrawingBorderProfile { grid: GridRule::FromCentre { field_mm: 0.999 }, ..iso },
        DrawingBorderProfile { grid: GridRule::FromCentre { field_mm: 1000.1 }, ..iso },
        DrawingBorderProfile { grid: GridRule::FromCentre { field_mm: f64::NAN }, ..iso },
        DrawingBorderProfile { grid: GridRule::FromCentre { field_mm: f64::INFINITY }, ..iso },
        DrawingBorderProfile { grid: GridRule::Equal { columns: 0, rows: 6 }, ..iso },
        DrawingBorderProfile { grid: GridRule::Equal { columns: 8, rows: 0 }, ..iso },
        DrawingBorderProfile { grid: GridRule::Equal { columns: 201, rows: 1 }, ..iso },
        DrawingBorderProfile { grid: GridRule::Equal { columns: u32::MAX, rows: u32::MAX }, ..iso },
        // The grid reference border lies between the sheet border and the drawing border.
        DrawingBorderProfile { grid_border_mm: -0.1, ..iso },
        DrawingBorderProfile { grid_border_mm: f64::NAN, ..iso },
        DrawingBorderProfile { grid_border_mm: 10.000_001, trimming_mark_mm: None, ..iso },
        // On every side: ISO's 5 mm is wider than a 4 mm bottom margin.
        DrawingBorderProfile { trimming_mark_mm: None, ..with_margins(20.0, 10.0, 10.0, 4.0) },
        // A centring mark reaches above 0 inside the drawing border.
        DrawingBorderProfile { centring_mark_mm: Some(0.0), ..iso },
        DrawingBorderProfile { centring_mark_mm: Some(-10.0), ..iso },
        DrawingBorderProfile { centring_mark_mm: Some(f64::NAN), ..iso },
        DrawingBorderProfile { centring_mark_mm: Some(f64::INFINITY), ..iso },
        // A trimming mark is finite, above 0 and no wider than it is long...
        DrawingBorderProfile { trimming_mark_mm: Some([4.0, 5.0]), ..iso },
        DrawingBorderProfile { trimming_mark_mm: Some([10.0, 0.0]), ..iso },
        DrawingBorderProfile { trimming_mark_mm: Some([f64::NAN, 5.0]), ..iso },
        DrawingBorderProfile { trimming_mark_mm: Some([10.0, f64::NAN]), ..iso },
        DrawingBorderProfile { trimming_mark_mm: Some([f64::INFINITY, 5.0]), ..iso },
        // ...and fits between the sheet border and the grid reference border: ISO's 5 mm fill its
        // 10 - 5 mm exactly.
        DrawingBorderProfile { grid_border_mm: 5.000_001, ..iso },
        DrawingBorderProfile { grid_border_mm: 0.0, ..with_margins(2.0, 2.0, 2.0, 2.0) },
    ] {
        assert!(profile.validate().is_err(), "validated {profile:?}");
        assert!(profile.layout(&a3).is_err(), "laid out {profile:?}");
    }
    // Fine on its own, but not on an A3 sheet.
    for profile in [
        with_margins(210.0, 210.0, 10.0, 10.0),
        with_margins(20.0, 10.0, 148.0, 148.5),
        // A millimetre of drawing space, but the centre line at 210 lies outside it.
        with_margins(200.0, 219.0, 10.0, 10.0),
        // A centring mark past the centre line, 138.5 mm inside the drawing border's bottom.
        DrawingBorderProfile { centring_mark_mm: Some(138.500_001), ..iso },
        // Or a drawing border its centre lines miss.
        DrawingBorderProfile { grid: GridRule::Equal { columns: 2, rows: 2 }, ..with_margins(200.0, 219.0, 10.0, 10.0) },
        // A trimming mark longer than half the 297 mm side.
        DrawingBorderProfile { trimming_mark_mm: Some([148.500_001, 5.0]), ..iso },
    ] {
        assert_eq!(profile.validate(), Ok(()), "{profile:?}");
        assert!(profile.layout(&a3).is_err(), "laid out {profile:?}");
    }
    // The same space with an equal division and no centring marks is fine, and so are the limits
    // themselves.
    let off_centre =
        DrawingBorderProfile { grid: GridRule::Equal { columns: 2, rows: 2 }, centring_mark_mm: None, ..with_margins(200.0, 219.0, 10.0, 10.0) };
    assert!((off_centre.layout(&a3).unwrap().drawing_border.max.x - 201.0).abs() < EPS);
    for profile in [
        DrawingBorderProfile { drawing_border_line_mm: 10.0, ..iso },
        DrawingBorderProfile { grid: GridRule::FromCentre { field_mm: 1.0 }, ..iso },
        DrawingBorderProfile { grid: GridRule::FromCentre { field_mm: 1000.0 }, ..iso },
        DrawingBorderProfile { grid: GridRule::Equal { columns: 200, rows: 200 }, ..iso },
        DrawingBorderProfile { grid_border_mm: 0.0, trimming_mark_mm: None, ..with_margins(0.0, 0.0, 0.0, 0.0) },
        DrawingBorderProfile { grid_border_mm: 10.0, trimming_mark_mm: None, ..iso },
        DrawingBorderProfile { centring_mark_mm: Some(138.5), ..iso },
        // ISO's own trimming marks fill their strip exactly, and so do these.
        iso,
        DrawingBorderProfile { grid_border_mm: 0.0, trimming_mark_mm: Some([10.0, 2.0]), ..with_margins(2.0, 2.0, 2.0, 2.0) },
        DrawingBorderProfile { trimming_mark_mm: Some([148.5, 5.0]), ..iso },
    ] {
        profile.layout(&a3).unwrap();
    }
}

#[test]
fn names_round_trip_and_unknown_fields_are_refused() {
    for (size, name) in
        [(PaperSize::A0, "a0"), (PaperSize::A5, "a5"), (PaperSize::Letter, "letter"), (PaperSize::AnsiA, "ansi_a"), (PaperSize::ArchE1, "arch_e1")]
    {
        assert_eq!(serde_json::to_value(size).unwrap(), serde_json::json!(name));
    }
    for size in PaperSize::ALL {
        let back: PaperSize = serde_json::from_value(serde_json::to_value(size).unwrap()).unwrap();
        assert_eq!(back, size);
    }
    let sheet = Sheet { size: SheetSize::Custom { sides_mm: [500.0, 700.0] }, orientation: Orientation::Portrait };
    let value = serde_json::to_value(sheet).unwrap();
    assert_eq!(value, serde_json::json!({"size": {"custom": {"sides_mm": [500.0, 700.0]}}, "orientation": "portrait"}));
    assert_eq!(serde_json::from_value::<Sheet>(value).unwrap(), sheet);
    let profile = DrawingBorderProfile::iso();
    let value = serde_json::to_value(profile).unwrap();
    assert_eq!(value["grid"], serde_json::json!({"from_centre": {"field_mm": 50.0}}));
    assert_eq!(value["label_sides"], serde_json::json!("iso"));
    assert_eq!(serde_json::to_value(GridRule::None).unwrap(), serde_json::json!("none"));
    assert_eq!(serde_json::from_value::<DrawingBorderProfile>(value.clone()).unwrap(), profile);
    for other in [
        DrawingBorderProfile { grid: GridRule::None, ..profile },
        DrawingBorderProfile { grid: GridRule::Equal { columns: 8, rows: 6 }, ..profile },
        DrawingBorderProfile { label_sides: LabelSides::Fixed(Sides::ALL), ..profile },
    ] {
        assert_eq!(serde_json::from_value::<DrawingBorderProfile>(serde_json::to_value(other).unwrap()).unwrap(), other);
    }
    for bad in [
        serde_json::json!({"size": {"standard": "a6"}}),
        serde_json::json!({"size": {"standard": "a3"}, "scale": 1}),
        serde_json::json!({"size": {"custom": {"sides_mm": [1, 2], "units": "in"}}}),
        serde_json::json!({"size": {"custom": {"sides_mm": [1, 2, 3]}}}),
        serde_json::json!({"size": {"standard": "a3"}, "orientation": "upside_down"}),
    ] {
        assert!(serde_json::from_value::<Sheet>(bad.clone()).is_err(), "{bad}");
    }
    // An unknown field anywhere in a profile is refused, in the grid's `none` form too.
    for (key, bad) in [
        ("title_block", serde_json::json!(true)),
        ("grid", serde_json::json!({"rule": "none", "columns": 8})),
        ("grid", serde_json::json!({"none": {"columns": 8}})),
        ("grid", serde_json::json!({"from_centre": {"field_mm": 50.0, "columns": 8}})),
        ("grid", serde_json::json!({"from_centre": {"field_mm": 50.0}, "columns": 8})),
        ("label_sides", serde_json::json!({"fixed": {"top": true, "bottom": true, "left": true, "right": true, "middle": true}})),
    ] {
        let mut extra = value.clone();
        extra[key] = bad.clone();
        assert!(serde_json::from_value::<DrawingBorderProfile>(extra).is_err(), "{key}: {bad}");
    }
}

/// For every accepted sheet and profile, the fields tile the drawing border exactly, top to bottom
/// and left to right, every field is longer than nothing and nothing is NaN.
#[test]
fn fields_tile_every_drawing_border_exactly() {
    let mut sheets: Vec<Sheet> =
        PaperSize::ALL.into_iter().flat_map(|size| [Orientation::Landscape, Orientation::Portrait].map(|o| Sheet::standard(size, o))).collect();
    for sides_mm in [[1.5, 1.5], [5000.0, 5000.0], [5000.0, 1.0], [17.3, 4999.9], [100.0, 101.0], [123.456, 789.012]] {
        for orientation in [Orientation::Landscape, Orientation::Portrait] {
            sheets.push(Sheet { size: SheetSize::Custom { sides_mm }, orientation });
        }
    }
    let margins = [
        Margins { left_mm: 20.0, right_mm: 10.0, top_mm: 10.0, bottom_mm: 10.0 },
        Margins { left_mm: 0.0, right_mm: 0.0, top_mm: 0.0, bottom_mm: 0.0 },
        Margins { left_mm: 5.0, right_mm: 5.0, top_mm: 5.0, bottom_mm: 5.0 },
        Margins { left_mm: 33.3, right_mm: 0.1, top_mm: 7.7, bottom_mm: 19.0 },
    ];
    let grids = [
        GridRule::FromCentre { field_mm: 50.0 },
        GridRule::FromCentre { field_mm: 1.0 },
        GridRule::FromCentre { field_mm: 7.5 },
        GridRule::FromCentre { field_mm: 33.3 },
        GridRule::FromCentre { field_mm: 1000.0 },
        GridRule::Equal { columns: 1, rows: 1 },
        GridRule::Equal { columns: 3, rows: 7 },
        GridRule::Equal { columns: 200, rows: 200 },
    ];
    let mut accepted = 0;
    for sheet in &sheets {
        for margins in margins {
            for grid in grids {
                let profile = DrawingBorderProfile {
                    margins,
                    drawing_border_line_mm: 0.7,
                    grid,
                    skip_i_and_o: true,
                    label_sides: LabelSides::Fixed(Sides::ALL),
                    grid_border_mm: 0.0,
                    centring_mark_mm: None,
                    trimming_mark_mm: None,
                };
                let Ok(layout) = profile.layout(sheet) else { continue };
                accepted += 1;
                let (drawing, size_mm) = (layout.drawing_border, layout.sheet_border.max);
                assert!(drawing.min.x >= 0.0 && drawing.min.y >= 0.0 && drawing.max.x <= size_mm.x && drawing.max.y <= size_mm.y);
                let check = |fields: &[Field], lo: f64, hi: f64, falling: bool| {
                    let (first, last) = (fields.first().unwrap(), fields.last().unwrap());
                    let (start, end) = if falling { (last.from_mm, first.to_mm) } else { (first.from_mm, last.to_mm) };
                    assert_eq!((start, end), (lo, hi), "{sheet:?} {profile:?}");
                    for f in fields {
                        assert!(f.from_mm.is_finite() && f.to_mm.is_finite() && f.to_mm - f.from_mm > EPS, "{f:?} {sheet:?} {profile:?}");
                    }
                    for w in fields.windows(2) {
                        let (a, b) = (&w[0], &w[1]);
                        if falling { assert_eq!(a.from_mm, b.to_mm) } else { assert_eq!(a.to_mm, b.from_mm) }
                    }
                    let total: f64 = fields.iter().map(|f| f.to_mm - f.from_mm).sum();
                    assert!((total - (hi - lo)).abs() < 1e-6, "{total} vs {}", hi - lo);
                    let mut labels: Vec<&str> = fields.iter().map(|f| f.label.as_str()).collect();
                    labels.sort_unstable();
                    labels.dedup();
                    assert_eq!(labels.len(), fields.len());
                };
                check(&layout.columns, drawing.min.x, drawing.max.x, false);
                check(&layout.rows, drawing.min.y, drawing.max.y, true);
                if let GridRule::FromCentre { field_mm } = grid {
                    // Inner fields are exactly the field length, and the centre lines are edges.
                    let centre = size_mm.x / 2.0;
                    assert!(layout.columns.iter().any(|f| f.to_mm == centre));
                    let inner = layout.columns.len().saturating_sub(2);
                    for f in layout.columns.iter().skip(1).take(inner) {
                        assert!((f.to_mm - f.from_mm - field_mm).abs() < 1e-9, "{f:?}");
                    }
                }
            }
        }
    }
    assert!(accepted > 1000, "{accepted}");
}

#[test]
fn rounding_past_a_whole_number_of_fields_leaves_no_sliver() {
    // 420 mm across with the centre line at 210 and the drawing border to 410: the right half is
    // four fields of 50. The left margin puts the left half at 150 mm, three fields, give or take
    // rounding noise.
    let sheet = Sheet { size: SheetSize::Custom { sides_mm: [420.0, 297.0] }, orientation: Orientation::Landscape };
    for hair in [0.0, 1e-12, 1e-10, -1e-10] {
        let margins = Margins { left_mm: 60.0 - hair, right_mm: 10.0, top_mm: 10.0, bottom_mm: 10.0 };
        let layout = DrawingBorderProfile { margins, ..DrawingBorderProfile::iso() }.layout(&sheet).unwrap();
        let widths = lengths(&layout.columns);
        // The noise joins the corner field: no field of a hair's width before it.
        assert_eq!(widths.len(), 7, "{hair}: {widths:?}");
        assert!(close(&widths, &[50.0; 7]), "{hair}: {widths:?}");
        assert_eq!(layout.columns.first().unwrap().from_mm, 60.0 - hair);
    }
}
