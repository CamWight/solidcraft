use solidcraft_doc::drawing::reference::ModelEdge;
use solidcraft_geom::paper::Scene;

use super::*;

fn balloon(at: [f64; 2]) -> Balloon {
    Balloon { id: 7, view: 8, edge: ModelEdge { body: "B".into(), origin_feature: 1, name: "e".into(), path: vec![1] }, at, parts_list: None }
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
            Item::Picture(_) => {}
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
    let on = balloon_group(&balloon([60.0, 40.0]), Some("12"), Some(Vec2::new(-40.0, 0.0)), None);
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
    let on = balloon_group(&balloon([0.0, 0.0]), Some("12.34.56"), None, None);
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
    let lost = balloon_group(&balloon([0.0, 0.0]), Some("3"), None, None);
    assert!(parts(&lost.group).1.is_empty() && parts(&lost.group).3.is_some() && lost.why.is_none());
    let empty = balloon_group(&balloon([0.0, 0.0]), None, Some(Vec2::new(30.0, 0.0)), Some("no list".into()));
    assert!(parts(&empty.group).3.is_none() && parts(&empty.group).2.is_some());
    assert_eq!(empty.why.as_deref(), Some("no list"));
    valid(&empty);
    // 0.1 mm from the centre is under the circle, further from its rim than an arrowhead.
    for tip in [Vec2::ZERO, Vec2::new(0.1, 0.0), Vec2::new(2.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(5.0 + ARROW_MM, 0.0)] {
        let near = balloon_group(&balloon([0.0, 0.0]), Some("3"), Some(tip), None);
        assert!(parts(&near.group).2.is_none() && near.why.as_deref().is_some_and(|why| why.contains("too near")), "{tip:?}: {:?}", near.why);
        assert_eq!(parts(&near.group).0.len(), 1);
    }
    assert!(parts(&balloon_group(&balloon([0.0, 0.0]), Some("3"), Some(Vec2::new(5.0 + ARROW_MM + 0.01, 0.0)), None).group).2.is_some());
    let unset = balloon_group(&balloon([0.0, 0.0]), Some("\u{4e2d}"), Some(Vec2::new(30.0, 0.0)), None);
    assert!(unset.why.as_deref().is_some_and(|why| why.contains("no glyph")) && parts(&unset.group).2.is_some());
}

/// Any place and any arrow, at the values that break a layout, draws or says why, without a panic.
#[test]
fn hostile_places_and_arrows_never_panic() {
    let values = [0.0, -0.0, 1e-300, 1e-9, 1.0, -1.0, 5.0, 8.0, 1e5, -1e5, 1e12, f64::MAX, f64::MIN, f64::NAN, f64::INFINITY];
    for x in values {
        for y in values {
            for tip in [Vec2::ZERO, Vec2::new(x, y), Vec2::new(y, 1.0)] {
                let on = balloon_group(&balloon([x, y]), Some("1"), Some(tip), None);
                let mut scene = Scene::new(Vec2::new(420.0, 297.0), vec![on.group]);
                scene.add_fonts(on.fonts);
                let _ = scene.validate();
            }
        }
    }
}
