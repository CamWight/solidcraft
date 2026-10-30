//! What a drawing's annotations draw on paper with: their line, arrowhead and text, in paper
//! millimetres, the same at any view scale. A text is set in the drawing font ([`paper_text`]) and
//! measured there, and one the font can't set leaves the marks and says why.
//!
//! A balloon (#3874 L2) is here too: a circle round its number at its place from its view's centre,
//! with its leader to where its arrow ends, when that was found. A lost one keeps its circle and its
//! number in place, as a lost note keeps its text.

use std::f64::consts::PI;

use solidcraft_doc::drawing::balloon::Balloon;
use solidcraft_geom::paper::annotate::{ArrowShape, Leader, Terminator};
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
    pub(crate) fn set(text: Option<&str>) -> Text {
        let unset = |why| Text { runs: Vec::new(), fonts: Vec::new(), half: Vec2::new(0.0, TEXT_MM * 0.5), why };
        match text.map(|text| paper_text(text, None, Vec2::ZERO, TEXT_MM, 0.0, &TextStyle::default())) {
            None | Some(Ok(None)) => unset(None),
            Some(Err(why)) => unset(Some(why.to_string())),
            Some(Ok(Some((runs, fonts)))) => {
                let width = runs.iter().map(TextRun::reach).fold(0.0, f64::max);
                // Its lines stand below the first one's baseline.
                let below = runs.iter().map(|run| -run.origin.y).fold(0.0, f64::max);
                Text { runs, fonts, half: Vec2::new(width, TEXT_MM + below) * 0.5, why: None }
            }
        }
    }
}

/// `runs` of a text `half` in size, centred on `centre` and running along `dir`.
pub(crate) fn placed(runs: Vec<TextRun>, half: Vec2, centre: Vec2, dir: Vec2) -> impl Iterator<Item = Item> {
    let up = dir.perp();
    let start = centre - dir * half.x + up * (half.y - TEXT_MM);
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
    let text = Text::set(item);
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
    items.extend(placed(text.runs, text.half, centre, Vec2::X));
    OnPaper { group: Group { object: balloon.id, kind: ObjectKind::Balloon, items }, fonts: text.fonts, why }
}

/// The leader from `tip` to the rim of the circle round `centre`, or none when `tip` is under the
/// circle or nearer it than an arrowhead.
fn leader_to(tip: Vec2, centre: Vec2, radius: f64) -> Option<Vec<Item>> {
    let rim = centre + (tip - centre).normalized()? * radius;
    let leader = Leader { points: vec![tip, rim], landing_mm: None, terminator: ARROW, width_mm: LINE_MM };
    (tip.dist(centre) > radius).then(|| leader.items().ok()).flatten()
}

#[cfg(test)]
#[path = "drawing_annotation_paper_tests.rs"]
mod tests;
