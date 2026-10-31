//! What a drawing's annotations draw on paper with: their line, arrowhead and text, in paper
//! millimetres, the same at any view scale. A text is set in the drawing font ([`paper_text`]) and
//! measured there, and one the font can't set leaves the marks and says why.
//!
//! A balloon (#3874 L2) is here too: a circle round its number at its place from its view's centre,
//! with its leader to where its arrow ends, when that was found. A lost one keeps its circle and its
//! number in place, as a lost note keeps its text.
//!
//! A note (#4896 W19i) is its text, centred on its place, with a leader and landing to the side of
//! the text the leader comes from, when what it points at was found. It reads as written, each field
//! visible by its name until the title block's field values are there to fill it.

use std::f64::consts::PI;

use solidcraft_doc::drawing::balloon::Balloon;
use solidcraft_doc::drawing::note::Note;
use solidcraft_geom::cell::{Content, Part, Source};
use solidcraft_geom::paper::annotate::{ArrowShape, LANDING_PER_LINE_WIDTH, Leader, Terminator};
use solidcraft_geom::paper::text::{Font, TextRun};
use solidcraft_geom::paper::{Group, Item, ObjectKind};
use solidcraft_geom::{Loop2, Vec2};
use solidcraft_sketch::{TextStyle, paper_text};

/// An annotation's lines, half as wide as a view's visible ones (ISO 128).
pub(crate) const LINE_MM: f64 = 0.25;
/// The length of an arrowhead.
pub(crate) const ARROW_MM: f64 = 3.0;
/// A filled arrowhead, its line stopped at its base.
pub(crate) const ARROW: Terminator = Terminator::Arrow { shape: ArrowShape::Filled, length_mm: ARROW_MM, angle: PI / 9.0 };
/// The gap between a text and what is beside it.
pub(crate) const GAP_MM: f64 = 1.0;
/// The height of an annotation's capitals.
pub(crate) const TEXT_MM: f64 = 3.5;
/// A balloon's radius, or more when its number needs it.
const BALLOON_MM: f64 = 5.0;

/// A text set once and measured, upright from the sheet's origin, for a layout to place.
pub(crate) struct Text {
    pub(crate) runs: Vec<TextRun>,
    pub(crate) fonts: Vec<Font>,
    /// Half its size.
    pub(crate) half: Vec2,
    /// Why it isn't set, when it can't be.
    pub(crate) why: Option<String>,
}

impl Text {
    /// `text`, its capitals `cap_mm` high.
    pub(crate) fn set(text: Option<&str>, cap_mm: f64) -> Text {
        let unset = |why| Text { runs: Vec::new(), fonts: Vec::new(), half: Vec2::new(0.0, cap_mm * 0.5), why };
        match text.map(|text| paper_text(text, None, Vec2::ZERO, cap_mm, 0.0, &TextStyle::default())) {
            None | Some(Ok(None)) => unset(None),
            Some(Err(why)) => unset(Some(why.to_string())),
            Some(Ok(Some((runs, fonts)))) => {
                let width = runs.iter().map(TextRun::reach).fold(0.0, f64::max);
                // Its lines stand below the first one's baseline.
                let below = runs.iter().map(|run| -run.origin.y).fold(0.0, f64::max);
                Text { runs, fonts, half: Vec2::new(width, cap_mm + below) * 0.5, why: None }
            }
        }
    }
}

/// `runs` of a text `half` in size with capitals `cap_mm` high, centred on `centre` and running along
/// `dir`.
pub(crate) fn placed(runs: Vec<TextRun>, half: Vec2, cap_mm: f64, centre: Vec2, dir: Vec2) -> impl Iterator<Item = Item> {
    let up = dir.perp();
    let start = centre - dir * half.x + up * (half.y - cap_mm);
    runs.into_iter().map(move |run| Item::Text(TextRun { origin: start + dir * run.origin.x + up * run.origin.y, angle: dir.angle(), ..run }))
}

/// An annotation on paper.
pub struct OnPaper {
    pub group: Group,
    /// The fonts its text uses, in no order, the same one perhaps more than once.
    pub fonts: Vec<Font>,
    /// Why part of it isn't there.
    pub why: Option<String>,
}

/// `balloon` showing `item`, its arrow ending at `tip` when that was found, centred on its view as a
/// view's lines are. `why` is what its reader has to say of its number.
pub fn balloon_group(balloon: &Balloon, item: Option<&str>, tip: Option<Vec2>, why: Option<String>) -> OnPaper {
    let centre = Vec2::new(balloon.at[0], balloon.at[1]);
    let text = Text::set(item, TEXT_MM);
    // Round the number's corners, a gap clear of them.
    let radius = BALLOON_MM.max(text.half.len() + GAP_MM);
    let mut items = vec![Item::Stroke { path: Loop2::circle(centre, radius).segs, closed: true, width_mm: LINE_MM }];
    let mut why = why.or(text.why);
    if let Some(tip) = tip {
        match leader_to(tip, centre, radius) {
            Some(leader) => items.extend(leader),
            None => why = why.or_else(|| Some("its arrow ends too near its circle to draw".to_string())),
        }
    }
    items.extend(placed(text.runs, text.half, TEXT_MM, centre, Vec2::X));
    OnPaper { group: Group { object: balloon.id, kind: ObjectKind::Balloon, items }, fonts: text.fonts, why }
}

/// The leader from `tip` to the rim of the circle round `centre`, or none when `tip` is under the
/// circle or nearer it than an arrowhead.
fn leader_to(tip: Vec2, centre: Vec2, radius: f64) -> Option<Vec<Item>> {
    let rim = centre + (tip - centre).normalized()? * radius;
    let leader = Leader { points: vec![tip, rim], landing_mm: None, terminator: ARROW, width_mm: LINE_MM };
    (tip.dist(centre) > radius).then(|| leader.items().ok()).flatten()
}

/// `note`, its leader ending at `tip` when that was found, centred on what it hangs on.
pub fn note_group(note: &Note, tip: Option<Vec2>) -> OnPaper {
    let centre = Vec2::new(note.at[0], note.at[1]);
    let cap_mm = note.cap_mm.unwrap_or(TEXT_MM);
    let text = Text::set(Some(&content_text(&note.content)), cap_mm);
    let mut why = text.why;
    let mut items = Vec::new();
    if let Some(tip) = tip {
        match note_leader(tip, centre, text.half) {
            Some(leader) => items.extend(leader),
            None => why = why.or_else(|| Some("its leader ends too near its text to draw".to_string())),
        }
    }
    items.extend(placed(text.runs, text.half, cap_mm, centre, Vec2::X));
    OnPaper { group: Group { object: note.id, kind: ObjectKind::Note, items }, fonts: text.fonts, why }
}

/// A note's leader from `tip` to the text `half` in size round `centre`: with a landing, to the side
/// of the text it comes from; straight to the nearer side from over or under the text, where a
/// landing would run back through the line.
fn note_leader(tip: Vec2, centre: Vec2, half: Vec2) -> Option<Vec<Item>> {
    let landing = LANDING_PER_LINE_WIDTH * LINE_MM;
    let beside = half.x + GAP_MM;
    let (left, right) = (centre.x - beside, centre.x + beside);
    let (end, landing_mm) = if tip.x < left - landing {
        (left - landing, Some(landing))
    } else if tip.x > right + landing {
        (right + landing, Some(landing))
    } else {
        (if tip.x < centre.x { left } else { right }, None)
    };
    let leader = Leader { points: vec![tip, Vec2::new(end, centre.y)], landing_mm, terminator: ARROW, width_mm: LINE_MM };
    leader.items().ok()
}

/// `content` as it reads for now: its text as written, and a field by its name.
fn content_text(content: &Content) -> String {
    let source = |source: &Source| match source {
        Source::Component => "component",
        Source::Drawing => "drawing",
        Source::Sheet => "sheet",
        Source::Revision => "revision",
        Source::Prompt => "prompt",
        Source::Row => "row",
    };
    let part = |part: &Part| match part {
        Part::Text(text) => text.clone(),
        Part::Field { source: from, key, .. } => format!("<{}.{key}>", source(from)),
    };
    content.0.iter().map(part).collect()
}

#[cfg(test)]
#[path = "drawing_annotation_paper_tests.rs"]
mod tests;
