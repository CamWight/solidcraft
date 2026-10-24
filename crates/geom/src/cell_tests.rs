use serde_json::json;

use super::*;
use crate::bspline::tests::Lcg;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Aabb2 {
    Aabb2 { min: Vec2::new(x0, y0), max: Vec2::new(x1, y1) }
}

fn text(s: &str) -> Part {
    Part::Text(s.to_string())
}

fn field(source: Source, key: &str) -> Part {
    Part::Field { source, key: key.to_string(), format: Format::default() }
}

/// The title cell's value box on an A3 sheet: the cell (230, 10)-(342, 24) under a 2.5 mm label.
const TITLE_BOX: Aabb2 = Aabb2 { min: Vec2::new(231.0, 11.0), max: Vec2::new(341.0, 19.5) };

#[test]
fn the_label_and_the_value_box_inset_the_cell_by_the_padding() {
    let cell = rect(230.0, 10.0, 342.0, 24.0);
    // The label's baseline 1 + 2.5 mm below the top; the value box's top 1 mm below that.
    assert_eq!(label_at(cell, 2.5), Vec2::new(231.0, 20.5));
    assert_eq!(value_box(cell, Some(2.5)), TITLE_BOX);
    assert_eq!(value_box(cell, None), rect(231.0, 11.0, 341.0, 23.0));
}

#[test]
fn one_line_sits_centred_in_its_value_box_at_the_left_middle_or_right() {
    // The box's centre is 15.25 high: the 5 mm line's baseline 2.5 below it.
    for (align, x) in [(HAlign::Left, 231.0), (HAlign::Centre, 286.0), (HAlign::Right, 341.0)] {
        assert_eq!(place_lines(TITLE_BOX, 1, 5.0, 8.0, align, false), Ok((Vec2::new(x, 12.75), None)), "{align:?}");
    }
}

#[test]
fn wrapped_lines_and_several_lines_run_down_from_the_top_at_the_pitch() {
    let square = rect(0.0, 0.0, 10.0, 10.0);
    // A fake font's pitch of 1.6 cap heights: at 2.5 mm the baselines are 2.5 and 6.5 below the top.
    let (first, warning) = place_lines(square, 2, 2.5, 4.0, HAlign::Left, true).unwrap();
    assert_eq!((first, warning), (Vec2::new(0.0, 7.5), None));
    assert_eq!(first.y - 4.0, 10.0 - 6.5);
    // Two lines without wrapping start at the top too; one line that wraps isn't centred.
    assert_eq!(place_lines(square, 2, 2.5, 4.0, HAlign::Left, false), Ok((Vec2::new(0.0, 7.5), None)));
    assert_eq!(place_lines(square, 1, 2.5, 4.0, HAlign::Right, true), Ok((Vec2::new(10.0, 7.5), None)));
    assert_eq!(place_lines(square, 1, 2.5, 4.0, HAlign::Right, false), Ok((Vec2::new(10.0, 3.75), None)));
}

#[test]
fn lines_whose_last_baseline_is_below_the_box_warn_height() {
    // 14.5 mm high: four lines at 2.5 and a pitch of 4 end exactly on the bottom; a fifth doesn't fit.
    let tall = rect(311.0, 45.0, 379.0, 59.5);
    for wrap in [false, true] {
        assert_eq!(place_lines(tall, 4, 2.5, 4.0, HAlign::Left, wrap), Ok((Vec2::new(311.0, 57.0), None)));
        assert_eq!(place_lines(tall, 5, 2.5, 4.0, HAlign::Left, wrap), Ok((Vec2::new(311.0, 57.0), Some(Warning::Height))));
    }
    // Within 1e-9 mm of the bottom is on it.
    let raised = |by: f64| Aabb2 { min: Vec2::new(311.0, 45.0 + by), ..tall };
    assert_eq!(place_lines(raised(0.5e-9), 4, 2.5, 4.0, HAlign::Left, true).unwrap().1, None);
    assert_eq!(place_lines(raised(2e-9), 4, 2.5, 4.0, HAlign::Left, true).unwrap().1, Some(Warning::Height));
    // One centred line fits while its cap height does: 5 mm in a 5 mm box, not 5.1.
    let low = rect(0.0, 0.0, 10.0, 5.0);
    assert_eq!(place_lines(low, 1, 5.0, 8.0, HAlign::Left, false), Ok((Vec2::new(0.0, 0.0), None)));
    assert_eq!(place_lines(low, 1, 5.1, 8.0, HAlign::Left, false).unwrap().1, Some(Warning::Height));
    // A flat box holds no line; no lines never overflow; a vast count still answers.
    assert_eq!(place_lines(rect(0.0, 1.0, 10.0, 1.0), 1, 1.0, 1.6, HAlign::Left, false).unwrap().1, Some(Warning::Height));
    assert_eq!(place_lines(rect(0.0, 1.0, 10.0, 1.0), 0, 1.0, 1.6, HAlign::Left, false).unwrap().1, None);
    assert_eq!(place_lines(tall, usize::MAX, 2.5, 4.0, HAlign::Left, true).unwrap().1, Some(Warning::Height));
}

#[test]
fn a_hostile_value_box_cap_or_pitch_is_refused() {
    const BOX: Invalid = Invalid("a value box is finite and not inside out");
    const SIZES: Invalid = Invalid("a cap height and a line pitch are finite and above 0");
    for value_box in [
        Aabb2::EMPTY,
        rect(10.0, 0.0, 9.0, 5.0),
        rect(0.0, 5.0, 10.0, 4.999),
        rect(f64::NAN, 0.0, 10.0, 5.0),
        rect(0.0, 0.0, f64::INFINITY, 5.0),
        rect(0.0, f64::NEG_INFINITY, 10.0, 5.0),
    ] {
        assert_eq!(place_lines(value_box, 1, 2.5, 4.0, HAlign::Left, false), Err(BOX), "{value_box:?}");
    }
    for v in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(place_lines(TITLE_BOX, 1, v, 4.0, HAlign::Left, false), Err(SIZES), "cap {v}");
        assert_eq!(place_lines(TITLE_BOX, 1, 2.5, v, HAlign::Left, false), Err(SIZES), "pitch {v}");
    }
    place_lines(TITLE_BOX, 1, f64::MIN_POSITIVE, f64::MIN_POSITIVE, HAlign::Left, false).unwrap();
}

#[test]
fn a_count_warns_above_its_maximum_in_characters_not_bytes() {
    let title = Fit { max_chars: Some(25), wide_max_chars: Some(30), wrap: false };
    let chars = |count, max| Some(Warning::Chars { count, max });
    let repeat = |c: &str, n: usize| c.repeat(n);
    assert_eq!(title.warn_chars(&repeat("0", 25)), None);
    assert_eq!(title.warn_chars(&repeat("0", 26)), chars(26, 25));
    assert_eq!(title.warn_chars(&repeat("0", 40)), chars(40, 25));
    // 'é' is two bytes and one character.
    assert_eq!(title.warn_chars(&repeat("é", 25)), None);
    assert_eq!(title.warn_chars(&repeat("é", 40)), chars(40, 25));
    // A text with a wide character is held to the wide count, whatever else it holds.
    assert_eq!(title.warn_chars(&repeat("字", 30)), None);
    assert_eq!(title.warn_chars(&repeat("字", 31)), chars(31, 30));
    assert_eq!(title.warn_chars(&format!("A{}", repeat("字", 29))), None);
    assert_eq!(title.warn_chars(&format!("{}字", repeat("A", 30))), chars(31, 30));
    // Line breaks don't count.
    assert_eq!(title.warn_chars(&format!("{}\n", repeat("0", 25))), None);
    assert_eq!(title.warn_chars(""), None);
    // No wide count: the count holds for every text. No count: nothing warns.
    let plain = Fit { max_chars: Some(25), ..Fit::default() };
    assert_eq!(plain.warn_chars(&repeat("字", 26)), chars(26, 25));
    let wide_only = Fit { wide_max_chars: Some(30), ..Fit::default() };
    assert_eq!(wide_only.warn_chars(&repeat("0", 1000)), None);
    assert_eq!(wide_only.warn_chars(&repeat("字", 31)), chars(31, 30));
    assert_eq!(Fit::default().warn_chars(&repeat("字", 1000)), None);
    let none = Fit { max_chars: Some(0), ..Fit::default() };
    assert_eq!(none.warn_chars("a"), chars(1, 0));
    assert_eq!(none.warn_chars("\n"), None);
}

#[test]
fn wide_characters_are_those_of_the_wide_scripts_blocks() {
    for c in ['字', '한', 'カ', 'ひ', 'Ａ', '。', 'ꀀ', '\u{1100}', '\u{115F}', '\u{303E}', '\u{20000}', '\u{3FFFD}', '\u{FFE6}'] {
        assert!(is_wide(c), "{c:?} U+{:04X}", u32::from(c));
    }
    for c in ['A', 'é', 'Ω', 'Я', 'א', '\u{1160}', '\u{303F}', '\u{3040}', '\u{FF61}', '\u{FFE7}', '\u{3FFFE}', '😀', '\n'] {
        assert!(!is_wide(c), "{c:?} U+{:04X}", u32::from(c));
    }
}

#[test]
fn content_has_at_most_64_parts_and_4096_bytes_of_text() {
    Content(vec![text("a"); MAX_PARTS]).validate().unwrap();
    assert_eq!(Content(vec![text(""); MAX_PARTS + 1]).validate(), Err(Invalid("a cell has at most 64 parts")));
    const LONG: Invalid = Invalid("a cell's text is at most 4096 bytes");
    Content(vec![text(&"a".repeat(4096))]).validate().unwrap();
    assert_eq!(Content(vec![text(&"a".repeat(4097))]).validate(), Err(LONG));
    // Bytes, not characters: 2048 'é' are 4096 bytes.
    Content(vec![text(&"é".repeat(2048))]).validate().unwrap();
    assert_eq!(Content(vec![text(&"é".repeat(2048)), text("a")]).validate(), Err(LONG));
    // Keys and units count too.
    let unit =
        |unit: &str| Part::Field { source: Source::Component, key: "k".repeat(2047), format: Format { unit: Some(unit.into()), decimals: None } };
    Content(vec![text(&"a".repeat(2048)), unit("g")]).validate().unwrap();
    assert_eq!(Content(vec![text(&"a".repeat(2048)), unit("kg")]).validate(), Err(LONG));
    assert_eq!(Content(vec![text(&"a".repeat(2048)), field(Source::Drawing, &"k".repeat(2049))]).validate(), Err(LONG));
    // Counted first: 65 parts of bad text report the count.
    assert_eq!(Content(vec![text("\t"); MAX_PARTS + 1]).validate(), Err(Invalid("a cell has at most 64 parts")));
    Content::default().validate().unwrap();
}

#[test]
fn content_has_no_control_character_but_a_line_break() {
    const CONTROL: Invalid = Invalid("a cell's text has no control character but a line break");
    Content(vec![text("Anodised\nblack"), text("\n")]).validate().unwrap();
    for c in ['\t', '\r', '\0', '\u{1b}', '\u{7f}', '\u{85}', '\u{9f}'] {
        let s = format!("a{c}b");
        assert_eq!(Content(vec![text(&s)]).validate(), Err(CONTROL), "{c:?}");
        assert_eq!(Content(vec![field(Source::Sheet, &s)]).validate(), Err(CONTROL), "{c:?}");
        let format = Format { unit: Some(s), decimals: Some(1) };
        assert_eq!(Content(vec![Part::Field { source: Source::Component, key: "mass".into(), format }]).validate(), Err(CONTROL), "{c:?}");
    }
}

#[test]
fn parts_read_and_write_as_externally_tagged_json() {
    let content = Content(vec![
        text("Sheet "),
        field(Source::Sheet, "number"),
        Part::Field { source: Source::Component, key: "mass".into(), format: Format { unit: Some("g".into()), decimals: Some(1) } },
    ]);
    let value = serde_json::to_value(&content).unwrap();
    assert_eq!(
        value,
        json!([
            {"text": "Sheet "},
            {"field": {"source": "sheet", "key": "number", "format": {"unit": null, "decimals": null}}},
            {"field": {"source": "component", "key": "mass", "format": {"unit": "g", "decimals": 1}}},
        ])
    );
    assert_eq!(serde_json::from_value::<Content>(value).unwrap(), content);
    // Without a format, the caller's settings.
    let read = |v: serde_json::Value| serde_json::from_value::<Content>(json!([v]));
    assert_eq!(read(json!({"field": {"source": "revision", "key": "date"}})).unwrap(), Content(vec![field(Source::Revision, "date")]));
    for (name, source) in [
        ("component", Source::Component),
        ("drawing", Source::Drawing),
        ("sheet", Source::Sheet),
        ("revision", Source::Revision),
        ("prompt", Source::Prompt),
    ] {
        assert_eq!(serde_json::to_value(source).unwrap(), json!(name));
    }
    for bad in [
        json!({"field": {"source": "customer", "key": "name"}}),
        json!({"field": {"source": "Component", "key": "name"}}),
        json!({"field": {"source": "sheet", "key": "number", "scale": 1}}),
        json!({"field": {"source": "sheet", "key": "number", "format": {"date": "ymd"}}}),
        json!({"field": {"source": "component", "key": "mass", "format": {"decimals": 256}}}),
        json!({"field": {"source": "component", "key": "mass", "format": {"decimals": -1}}}),
        json!({"field": {"source": "component", "key": "mass", "format": {"decimals": 1.5}}}),
        json!({"field": {"source": "sheet"}}),
        json!({"field": {"source": "sheet", "key": 1}}),
        json!({"expr": "a + 1"}),
        json!({"text": "a", "field": {"source": "sheet", "key": "number"}}),
        json!({"kind": "text", "text": "a"}),
        json!("Sheet"),
    ] {
        assert!(read(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn an_override_holds_two_texts_each_checked_by_the_text_rule() {
    let typed = Override { text: "Bracket, left".into(), source_was: "Bracket".into() };
    typed.validate().unwrap();
    let value = serde_json::to_value(&typed).unwrap();
    assert_eq!(value, json!({"text": "Bracket, left", "source_was": "Bracket"}));
    assert_eq!(serde_json::from_value::<Override>(value).unwrap(), typed);
    // Each holds up to 4096 bytes on its own.
    Override { text: "a".repeat(4096), source_was: "b".repeat(4096) }.validate().unwrap();
    for bad in [
        Override { text: "a".repeat(4097), ..typed.clone() },
        Override { source_was: "a".repeat(4097), ..typed.clone() },
        Override { text: "a\tb".into(), ..typed.clone() },
        Override { source_was: "\r".into(), ..typed.clone() },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
    for bad in [json!({"text": "a"}), json!({"text": "a", "source_was": "b", "drifted": true})] {
        assert!(serde_json::from_value::<Override>(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn alignments_projections_and_fits_have_snake_case_names() {
    for (align, name) in [(HAlign::Left, "left"), (HAlign::Centre, "centre"), (HAlign::Right, "right")] {
        assert_eq!(serde_json::to_value(align).unwrap(), json!(name));
        assert_eq!(serde_json::from_value::<HAlign>(json!(name)).unwrap(), align);
    }
    assert_eq!(HAlign::default(), HAlign::Left);
    for (projection, name) in [(Projection::First, "first"), (Projection::Third, "third")] {
        assert_eq!(serde_json::to_value(projection).unwrap(), json!(name));
        assert_eq!(serde_json::from_value::<Projection>(json!(name)).unwrap(), projection);
    }
    for bad in ["center", "Left", "middle", ""] {
        assert!(serde_json::from_value::<HAlign>(json!(bad)).is_err(), "{bad}");
    }
    for bad in ["First", "first_angle", "second", ""] {
        assert!(serde_json::from_value::<Projection>(json!(bad)).is_err(), "{bad}");
    }
    let fit = Fit { max_chars: Some(25), wide_max_chars: Some(30), wrap: true };
    let value = serde_json::to_value(fit).unwrap();
    assert_eq!(value, json!({"max_chars": 25, "wide_max_chars": 30, "wrap": true}));
    assert_eq!(serde_json::from_value::<Fit>(value).unwrap(), fit);
    assert_eq!(serde_json::from_value::<Fit>(json!({})).unwrap(), Fit::default());
    assert!(serde_json::from_value::<Fit>(json!({"padding_mm": 1.0})).is_err());
    assert!(serde_json::from_value::<Fit>(json!({"max_chars": -1})).is_err());
    assert_eq!(MISSING, "-");
}

/// A whole number below `n` from `rng`.
fn pick(rng: &mut Lcg, n: usize) -> usize {
    ((rng.unit() * n as f64) as usize).min(n - 1)
}

/// Random content, texts and boxes, hostile ones among them: validation agrees with the rule worked
/// out independently, the count agrees with one by hand, and nothing panics.
#[test]
fn fuzzed_content_texts_and_boxes_never_panic_and_follow_the_rules() {
    const CHARS: [char; 14] = ['a', 'Z', '0', ' ', '\n', '\t', '\r', '\u{7f}', '\u{85}', 'é', '字', '한', '😀', '\0'];
    const NUMBERS: [f64; 10] = [0.0, -1.0, 1e-300, 2.5, 5.0, 14.5, 1e5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
    let string = |rng: &mut Lcg, len: usize| -> String { (0..len).map(|_| CHARS[pick(rng, CHARS.len())]).collect() };
    let number = |rng: &mut Lcg| if pick(rng, 4) == 0 { NUMBERS[pick(rng, NUMBERS.len())] } else { rng.unit() * 200.0 - 50.0 };
    let mut rng = Lcg(0x5eed);
    let mut accepted = 0;
    for round in 0..3000 {
        let lengths = [0, 1, 7, 60, 1100, 4097];
        let mut parts = Vec::new();
        for i in 0..[0, 1, 3, 64, 65][round % 5] {
            let len = lengths[(round + i) % lengths.len()];
            parts.push(match i % 3 {
                0 => Part::Text(string(&mut rng, len)),
                1 => Part::Field { source: Source::Drawing, key: string(&mut rng, len % 40), format: Format::default() },
                _ => {
                    let format = Format { unit: Some(string(&mut rng, len % 5)), decimals: Some(3) };
                    Part::Field { source: Source::Component, key: "mass".into(), format }
                }
            });
        }
        let content = Content(parts);
        let strings: Vec<&str> = content
            .0
            .iter()
            .flat_map(|p| match p {
                Part::Text(t) => vec![t.as_str()],
                Part::Field { key, format, .. } => [Some(key.as_str()), format.unit.as_deref()].into_iter().flatten().collect(),
            })
            .collect();
        let bytes: usize = strings.iter().map(|s| s.len()).sum();
        let clean = strings.iter().all(|s| s.chars().all(|c| c == '\n' || !c.is_control()));
        let valid = content.0.len() <= MAX_PARTS && bytes <= MAX_TEXT_BYTES && clean;
        assert_eq!(content.validate().is_ok(), valid, "round {round}: {} parts, {bytes} bytes, clean {clean}", content.0.len());
        accepted += usize::from(valid);
        let back: Content = serde_json::from_value(serde_json::to_value(&content).unwrap()).unwrap();
        assert_eq!(back, content);
        let fit = Fit { max_chars: Some(30), wide_max_chars: Some(20), wrap: false };
        for s in &strings {
            let count = s.chars().filter(|&c| c != '\n').count();
            let max = if s.chars().any(is_wide) { 20 } else { 30 };
            assert_eq!(fit.warn_chars(s), (count > max as usize).then_some(Warning::Chars { count, max }), "{s:?}");
        }
        let (x0, y0, x1, y1, cap, pitch) =
            (number(&mut rng), number(&mut rng), number(&mut rng), number(&mut rng), number(&mut rng), number(&mut rng));
        let lines = [0, 1, 2, 5, 1000, usize::MAX][round % 6];
        let value_box = rect(x0, y0, x1, y1);
        let sane = [x0, y0, x1, y1, cap, pitch].iter().all(|v| v.is_finite()) && x0 <= x1 && y0 <= y1 && cap > 0.0 && pitch > 0.0;
        let placed = place_lines(value_box, lines, cap, pitch, [HAlign::Left, HAlign::Centre, HAlign::Right][round % 3], round % 2 == 0);
        assert_eq!(placed.is_ok(), sane, "{value_box:?} {cap} {pitch}");
        if let Ok((at, warning)) = placed {
            assert!(at.is_finite() && at.x >= x0 && at.x <= x1 && at.y <= y1, "{at:?} in {value_box:?}");
            if lines == 1 && warning.is_none() {
                assert!(at.y >= y0 - EPS, "{at:?} in {value_box:?}");
            }
        }
    }
    assert!(accepted > 500, "{accepted}");
}
