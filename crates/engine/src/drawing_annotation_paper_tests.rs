use solidcraft_doc::drawing::dimension::ModelEdge;
use solidcraft_doc::drawing::note::Anchor;
use solidcraft_geom::paper::Scene;

use super::*;

/// A balloon of a stack's, on `under`.
fn stacked(id: u64, at: BalloonPlace) -> Balloon {
    Balloon { id, view: 8, edge: ModelEdge { body: "B".into(), origin_feature: 1, name: "e".into(), path: vec![1] }, at, parts_list: None }
}

/// Each in a stack touches the one under it, however wide its number makes it.
#[test]
fn a_stack_stands_touching() {
    use std::f64::consts::FRAC_PI_2;
    let first = stacked(1, [60.0, 40.0].into());
    let right = stacked(2, BalloonPlace::On(Stacked { on: 1, angle: 0.0 }));
    let up = stacked(3, BalloonPlace::On(Stacked { on: 2, angle: FRAC_PI_2 }));
    assert!(close(balloon_centre(std::iter::once((&first, Some("1")))), Vec2::new(60.0, 40.0)));
    assert!(close(balloon_centre([(&first, Some("1")), (&right, Some("2"))].into_iter()), Vec2::new(70.0, 40.0)));
    assert!(close(balloon_centre([(&first, Some("1")), (&right, Some("2")), (&up, Some("3"))].into_iter()), Vec2::new(70.0, 50.0)));
    // A number too wide for 5 mm: its own group's circle says how far its centre stands.
    let wide = "12.34.56";
    let at = balloon_centre([(&first, Some("1")), (&right, Some(wide))].into_iter());
    let group = balloon_group(2, at, Some(wide), None, None).group;
    let Some(Item::Stroke { path, .. }) = group.items.first() else { panic!("its circle") };
    let Some(solidcraft_geom::Seg2::Arc { radius, .. }) = path.first() else { panic!("an arc") };
    assert!(*radius > 5.0, "{radius}");
    assert!(close(at, Vec2::new(60.0 + 5.0 + radius, 40.0)), "{at:?} {radius}");
}

fn close(a: Vec2, b: Vec2) -> bool {
    a.dist(b) < 1e-9
}

/// The balloon's circle, its leader's points and its arrowhead's tip, and its number's run.
fn parts(group: &Group) -> (Vec<&Item>, Vec<Vec2>, Option<Vec2>, Option<&TextRun>) {
    let (mut circles, mut line, mut tip, mut run) = (Vec::new(), Vec::new(), None, None);
    for item in &group.items {
        match item {
            Item::Stroke { closed: true, .. } => circles.push(item),
            Item::Stroke { path, .. } => line = path.iter().flat_map(|seg| [seg.start(), seg.end()]).collect(),
            Item::Fill { region } => tip = Some(region.outer.segs[0].start()),
            Item::Text(text) => run = Some(text),
        }
    }
    (circles, line, tip, run)
}

fn valid(on: &OnPaper) {
    let mut scene = Scene::new(Vec2::new(420.0, 297.0), vec![on.group.moved(Vec2::new(200.0, 150.0))]);
    scene.add_fonts(on.fonts.iter().cloned());
    scene.validate().unwrap();
}

/// A number in a circle of 5 at its place, level and in its middle, and a leader from the arrow's end
/// to the rim, along the line to the centre: by hand, (-40, 0) to (60, 40) leaves the rim 5 short.
#[test]
fn a_balloon_is_a_circle_round_its_number_with_a_leader_to_its_arrow() {
    let on = balloon_group(7, Vec2::new(60.0, 40.0), Some("12"), Some(Vec2::new(-40.0, 0.0)), None);
    assert_eq!((on.group.object, on.group.kind, on.why.as_deref()), (7, ObjectKind::Balloon, None));
    valid(&on);
    let (circles, line, tip, run) = parts(&on.group);
    let Item::Stroke { path, width_mm, .. } = circles[0] else { panic!("a circle") };
    assert_eq!(*path, Loop2::circle(Vec2::new(60.0, 40.0), 5.0).segs);
    assert_eq!(*width_mm, LINE_MM);
    assert!(close(tip.unwrap(), Vec2::new(-40.0, 0.0)), "{tip:?}");
    let way = Vec2::new(100.0, 40.0).normalized().unwrap();
    assert!(close(line[0], Vec2::new(-40.0, 0.0) + way * ARROW_MM), "stops at the arrowhead's base: {line:?}");
    assert!(close(*line.last().unwrap(), Vec2::new(60.0, 40.0) - way * 5.0), "{line:?}");
    let run = run.unwrap();
    assert_eq!((run.text.as_str(), run.angle), ("12", 0.0));
    let start = Vec2::new(60.0 - run.reach() * 0.5, 40.0 - TEXT_MM * 0.5);
    assert!(close(run.origin, start), "{:?} for {start:?}", run.origin);
}

/// A number too wide for the least circle gets a larger one, a gap clear of its corners.
#[test]
fn a_wide_number_gets_a_wider_circle() {
    let on = balloon_group(7, Vec2::new(0.0, 0.0), Some("12.34.56"), None, None);
    let run = parts(&on.group).3.unwrap().reach();
    let radius = Vec2::new(run, TEXT_MM).len() * 0.5 + GAP_MM;
    assert!(radius > BALLOON_MM, "{radius}");
    let (circles, ..) = parts(&on.group);
    let Item::Stroke { path, .. } = circles[0] else { panic!("a circle") };
    assert_eq!(*path, Loop2::circle(Vec2::ZERO, radius).segs);
}

/// A lost balloon keeps its circle and number with no leader; no list keeps an empty circle and says
/// why; an arrow under or just outside the circle draws no leader and says so; and a number the font
/// can't set keeps the rest.
#[test]
fn what_it_cant_draw_leaves_the_rest_and_says_why() {
    let lost = balloon_group(7, Vec2::new(0.0, 0.0), Some("3"), None, None);
    assert!(parts(&lost.group).1.is_empty() && parts(&lost.group).3.is_some() && lost.why.is_none());
    let empty = balloon_group(7, Vec2::new(0.0, 0.0), None, Some(Vec2::new(30.0, 0.0)), Some("no list".into()));
    assert!(parts(&empty.group).3.is_none() && parts(&empty.group).2.is_some());
    assert_eq!(empty.why.as_deref(), Some("no list"));
    valid(&empty);
    // 0.1 mm from the centre is under the circle, further from its rim than an arrowhead.
    for tip in [Vec2::ZERO, Vec2::new(0.1, 0.0), Vec2::new(2.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(5.0 + ARROW_MM, 0.0)] {
        let near = balloon_group(7, Vec2::new(0.0, 0.0), Some("3"), Some(tip), None);
        assert!(parts(&near.group).2.is_none() && near.why.as_deref().is_some_and(|why| why.contains("too near")), "{tip:?}: {:?}", near.why);
        assert_eq!(parts(&near.group).0.len(), 1);
    }
    assert!(parts(&balloon_group(7, Vec2::new(0.0, 0.0), Some("3"), Some(Vec2::new(5.0 + ARROW_MM + 0.01, 0.0)), None).group).2.is_some());
    let unset = balloon_group(7, Vec2::new(0.0, 0.0), Some("\u{4e2d}"), Some(Vec2::new(30.0, 0.0)), None);
    assert!(unset.why.as_deref().is_some_and(|why| why.contains("no glyph")) && parts(&unset.group).2.is_some());
}

fn note(at: [f64; 2], text: &str, cap_mm: Option<f64>) -> Note {
    Note { id: 9, anchor: Anchor::Sheet(1), at, content: Content(vec![Part::Text(text.into())]), cap_mm }
}

/// A note is level and centred on its place, at its own height or the default.
#[test]
fn a_note_is_its_text_centred_on_its_place() {
    let on = note_group(&note([100.0, 50.0], "Debur", None), None);
    assert_eq!((on.group.object, on.group.kind, on.why.as_deref()), (9, ObjectKind::Note, None));
    valid(&on);
    let run = parts(&on.group).3.unwrap();
    let start = Vec2::new(100.0 - run.reach() * 0.5, 50.0 - TEXT_MM * 0.5);
    assert!(close(run.origin, start) && run.angle == 0.0, "{:?}", run.origin);
    let tall = note_group(&note([100.0, 50.0], "Debur", Some(7.0)), None);
    let run = parts(&tall.group).3.unwrap();
    assert!(close(run.origin, Vec2::new(100.0 - run.reach() * 0.5, 46.5)), "{:?}", run.origin);
}

/// The landing runs to the side of the text the leader comes from, a gap clear of it, on the text's
/// middle: from the left, from the right, and straight to the nearer side from under the text. Each
/// by hand from the text's measured width.
#[test]
fn a_notes_landing_is_on_the_side_its_leader_comes_from() {
    let at = Vec2::new(100.0, 50.0);
    let width = parts(&note_group(&note([100.0, 50.0], "Debur", None), None).group).3.unwrap().reach();
    let (left, right) = (100.0 - width * 0.5 - GAP_MM, 100.0 + width * 0.5 + GAP_MM);
    let line = |tip: Vec2| {
        let on = note_group(&note([100.0, 50.0], "Debur", None), Some(tip));
        assert_eq!(on.why, None);
        valid(&on);
        assert!(close(parts(&on.group).2.unwrap(), tip), "the arrow's tip");
        parts(&on.group).1
    };
    let from_left = line(Vec2::new(20.0, 10.0));
    assert!(close(*from_left.last().unwrap(), Vec2::new(left, 50.0)), "{from_left:?}");
    assert!(from_left.contains(&Vec2::new(left - 5.0, 50.0)), "a landing 5 long: {from_left:?}");
    let from_right = line(Vec2::new(180.0, 90.0));
    assert!(close(*from_right.last().unwrap(), Vec2::new(right, 50.0)), "{from_right:?}");
    assert!(from_right.contains(&Vec2::new(right + 5.0, 50.0)), "{from_right:?}");
    let under = line(Vec2::new(99.0, 20.0));
    assert!(close(*under.last().unwrap(), Vec2::new(left, 50.0)), "no landing: {under:?}");
    assert!(!under.contains(&Vec2::new(left - 5.0, 50.0)), "{under:?}");
    assert!(close(*line(Vec2::new(101.0, 20.0)).last().unwrap(), Vec2::new(right, 50.0)));
    assert!(close(at, Vec2::new(100.0, 50.0)));
    // Nearer the text than an arrowhead: the text stays and says why.
    let near = note_group(&note([100.0, 50.0], "Debur", None), Some(Vec2::new(left, 50.0)));
    assert!(parts(&near.group).2.is_none() && near.why.as_deref().is_some_and(|why| why.contains("too near")), "{:?}", near.why);
    assert!(parts(&near.group).3.is_some());
}

/// A field shows by its name, never blank, beside the text around it; lines stand a gap clear below.
#[test]
fn a_notes_fields_and_lines_read() {
    let field = Part::Field { source: Source::Sheet, key: "number".into(), format: Default::default() };
    let on = note_group(&Note { content: Content(vec![Part::Text("Sheet ".into()), field]), ..note([0.0, 0.0], "", None) }, None);
    assert_eq!(parts(&on.group).3.unwrap().text, "Sheet <sheet.number>");
    let lines: String = (1..=20).map(|n| format!("Line {n}\n")).collect();
    let on = note_group(&note([0.0, 0.0], lines.trim_end(), None), None);
    let runs: Vec<&TextRun> = on.group.items.iter().filter_map(|item| if let Item::Text(run) = item { Some(run) } else { None }).collect();
    assert_eq!(runs.len(), 20);
    assert!(runs.windows(2).all(|pair| pair[0].origin.y > pair[1].origin.y), "down the page");
    let (top, bottom) = (runs[0].origin.y + TEXT_MM, runs[19].origin.y);
    assert!((top + bottom).abs() < 1e-9, "centred: {top} {bottom}");
    let unset = note_group(&note([0.0, 0.0], "\u{4e2d}", None), Some(Vec2::new(50.0, 0.0)));
    assert!(unset.why.as_deref().is_some_and(|why| why.contains("no glyph")), "{:?}", unset.why);
}

/// Any place and any arrow, at the values that break a layout, draws or says why, without a panic.
#[test]
fn hostile_places_and_arrows_never_panic() {
    let values = [0.0, -0.0, 1e-300, 1e-9, 1.0, -1.0, 5.0, 8.0, 1e5, -1e5, 1e12, f64::MAX, f64::MIN, f64::NAN, f64::INFINITY];
    for x in values {
        for y in values {
            for tip in [Vec2::ZERO, Vec2::new(x, y), Vec2::new(y, 1.0)] {
                for on in [balloon_group(7, Vec2::new(x, y), Some("1"), Some(tip), None), note_group(&note([x, y], "A", None), Some(tip))] {
                    let mut scene = Scene::new(Vec2::new(420.0, 297.0), vec![on.group]);
                    scene.add_fonts(on.fonts);
                    let _ = scene.validate();
                }
            }
        }
    }
}
