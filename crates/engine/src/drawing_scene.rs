//! The sheet scene (#4890 W10, #3890 W15): everything drawn on one sheet of a drawing as one paper
//! scene ([`Scene`]), for the sheet page and every export, so the screen and the files draw the
//! same thing. In paint order: the sheet's drawing border ([`SheetLayout::paper`]), then each view
//! on the sheet by id, its visible lines as [`paper_group`] makes them. Text, the title block,
//! tables, hidden lines and shaded views each join as one more kind of group.
//!
//! [`build`] draws a sheet at once, each view the one way: its lines worked out centred on the view,
//! then moved to its centre, so the same lines on the same centre are the same bytes however they
//! got there. It reads the model as it is: no model is evaluated.
//!
//! [`SheetLayout::paper`]: solidcraft_geom::drawing_sheet::SheetLayout::paper

use std::borrow::Borrow;

use solidcraft_doc::drawing::view::ResolvedView;
use solidcraft_doc::drawing::{Drawing, Sheet};
use solidcraft_doc::drawing_view::TangentEdges;
use solidcraft_doc::{DocError, ids};
use solidcraft_geom::paper::{Group, Item, MAX_RECORDS, Scene};
use solidcraft_geom::{Loop2, Region2, Seg2, Vec2};

use crate::drawing_view::{CHORD_MM, MIN_MODEL_CHORD_MM, ShownSet, Stats, Unmeasured, paper_group};
use crate::params::bad;
use crate::{EngineError, Result, Session};

/// The command the sheet scene answers for, in its errors.
pub const CMD: &str = "drawing.scene";

/// What became of one view of a sheet scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Its visible lines are in the scene.
    Drawn,
    /// It has nothing to measure, so it draws nothing: the sheet page shows its placeholder.
    Placeholder(Unmeasured),
    /// It can't be drawn yet, or its lines are more than a paper scene holds: why. The sheet's
    /// other views are drawn all the same.
    Failed(String),
}

impl Status {
    /// Its name in replies.
    pub fn name(&self) -> &'static str {
        match self {
            Status::Drawn => "drawn",
            Status::Placeholder(_) => "placeholder",
            Status::Failed(_) => "failed",
        }
    }
}

/// One view of a sheet scene.
#[derive(Clone, Debug)]
pub struct ViewState {
    pub view: u64,
    pub status: Status,
    /// What working out its lines cost, when they were.
    pub stats: Option<Stats>,
}

/// One sheet drawn at once ([`build`]).
#[derive(Clone, Debug)]
pub struct SheetScene {
    pub scene: Scene,
    /// Each view on the sheet, in id order.
    pub views: Vec<ViewState>,
}

/// Sheet `sheet` of `drawing`, one of `s`'s design's drawings, drawn at once: its border, then each
/// of its views by id. A view with nothing to measure, or one that can't be drawn, is left out and
/// says why; the sheet is refused only when it can't be drawn at all.
pub fn build(s: &Session, drawing: &Drawing, sheet: u64) -> Result<SheetScene> {
    let record = sheet_of(drawing, sheet)?;
    let on = views_on(drawing, sheet)?;
    // Worked out once for the sheet: every view draws from the same one. Each view's lines are
    // placed as they are drawn, so at most one view's are held beside the scene.
    let shown = crate::drawing_view::shown(s);
    let drawn = on.iter().map(|(id, view, centre)| (*id, *centre, draw(s, &shown, *id, view)));
    let (scene, views) = assemble(record, drawn)?;
    Ok(SheetScene { scene, views })
}

/// A view's lines for a sheet scene, centred on the view, or why it has none.
#[derive(Clone, Debug)]
struct Drawn {
    group: Option<Group>,
    status: Status,
    stats: Option<Stats>,
}

/// View `id`, drawing as `view` does, of the bodies `shown` holds: its visible lines, sampled at
/// [`CHORD_MM`] on paper, or coarser past 100:1, where that would ask the mesh for chords finer than
/// [`MIN_MODEL_CHORD_MM`]; centred on the view ([`assemble`] moves them to its centre). Lines past
/// the paper scene's [`MAX_RECORDS`] are refused here, so no view holds more than a scene takes.
fn draw(s: &Session, shown: &std::result::Result<ShownSet, Unmeasured>, id: u64, view: &ResolvedView) -> Drawn {
    let none = |status| Drawn { group: None, status, stats: None };
    // A style that draws hidden lines is refused here too, so the lines below are visible ones.
    if let Some(why) = crate::drawing_view::not_drawable(view.display_style.value, view.reference_style.value, true) {
        return none(Status::Failed(why.into()));
    }
    // Every body the design places, until saved views have `source` and `hide` (V4) to keep a part.
    let measured = shown.as_ref().map_err(|why| *why).and_then(|set| crate::drawing_view::measure(set, &view.basis).map(|e| (set, e)));
    let (set, extents) = match measured {
        Ok(measured) => measured,
        Err(why) => return none(Status::Placeholder(why)),
    };
    let around = extents.model.inflated(view.reference_margin_mm);
    let tangent_edges = view.tangent_edges.value == TangentEdges::Show;
    // The margin keeps the mesh's chord at or above its floor once divided back by the factor.
    let chord_mm = CHORD_MM.max(MIN_MODEL_CHORD_MM * view.scale.value.factor() * (1.0 + 1e-9));
    let lines = crate::drawing_view::frame(view.basis, view.scale.value, &around, Vec2::ZERO, CMD)
        .and_then(|frame| crate::drawing_view::lines(s, set, &frame, false, tangent_edges, chord_mm, CMD));
    match lines {
        Ok(lines) => {
            let group = paper_group(&lines, id);
            if group.records() > MAX_RECORDS {
                return none(Status::Failed(format!("its lines are more than a paper scene's {MAX_RECORDS} records: choose a smaller scale")));
            }
            Drawn { group: Some(group), status: Status::Drawn, stats: Some(lines.stats) }
        }
        Err(e) => none(Status::Failed(reason(e))),
    }
}

/// The scene of sheet `record`: its border, then each of `views` (its id, centre and lines, in id
/// order) moved to its centre. A view that would take the scene past a paper scene's limits is left
/// out and says why, and the views after it are still drawn when they fit.
fn assemble<D: Borrow<Drawn>>(record: &Sheet, views: impl Iterator<Item = (u64, Vec2, D)>) -> Result<(Scene, Vec<ViewState>)> {
    let size_mm = record.sheet.size_mm().map_err(|e| bad(CMD, e.to_string()))?;
    let border = match &record.border {
        Some(border) => border.layout(&record.sheet).map_err(|e| bad(CMD, e.to_string()))?.paper(record.id),
        None => Vec::new(),
    };
    let mut records = border.iter().map(Group::records).fold(0usize, usize::saturating_add);
    let mut scene = Scene { size_mm, groups: border };
    // Each view's group is counted onto the scene and checked alone on the sheet first, so one past
    // the limits leaves out only itself, and checking costs one pass over each group.
    let mut alone = Scene { size_mm, groups: Vec::with_capacity(1) };
    let mut states = Vec::new();
    for (view, centre, drawn) in views {
        let drawn: &Drawn = drawn.borrow();
        let mut status = drawn.status.clone();
        if let Some(group) = &drawn.group {
            let more = group.records();
            if records.saturating_add(more) > MAX_RECORDS {
                status = Status::Failed(format!(
                    "with its lines the sheet is more than a paper scene's {MAX_RECORDS} records: put it on another sheet, or choose a smaller scale"
                ));
            } else {
                alone.groups.push(moved(group, centre));
                match alone.validate() {
                    Ok(()) => {
                        records += more;
                        scene.groups.append(&mut alone.groups);
                    }
                    Err(e) => {
                        alone.groups.clear();
                        status = Status::Failed(format!("{e}: choose a smaller scale"));
                    }
                }
            }
        }
        states.push(ViewState { view, status, stats: drawn.stats });
    }
    scene.validate().map_err(|e| bad(CMD, e.to_string()))?;
    Ok((scene, states))
}

/// `group` moved `by` on paper: each point plus `by`, exactly, so the same lines moved to the same
/// centre are the same bytes however they got there.
fn moved(group: &Group, by: Vec2) -> Group {
    let segs = |path: &[Seg2]| -> Vec<Seg2> {
        let shift = |seg: &Seg2| match *seg {
            // An arc's centre moves; its radius and its angles stay.
            Seg2::Arc { center, radius, start, sweep } => Seg2::Arc { center: center + by, radius, start, sweep },
            _ => seg.map_controls(|p| p + by),
        };
        path.iter().map(shift).collect()
    };
    let ring = |l: &Loop2| Loop2 { segs: segs(&l.segs) };
    let items = group.items.iter().map(|item| match item {
        Item::Stroke { path, closed, width_mm } => Item::Stroke { path: segs(path), closed: *closed, width_mm: *width_mm },
        Item::Fill { region } => Item::Fill { region: Region2 { outer: ring(&region.outer), holes: region.holes.iter().map(ring).collect() } },
    });
    Group { object: group.object, kind: group.kind, items: items.collect() }
}

/// Sheet `sheet` of `d`.
fn sheet_of(d: &Drawing, sheet: u64) -> Result<&Sheet> {
    d.sheets.iter().map(|s| &**s).find(|s| s.id == sheet).ok_or_else(|| bad(CMD, format!("the drawing has no sheet {}", ids::encode(sheet))))
}

/// The views on sheet `sheet` of `d`, in id order, each as it draws and with its centre.
fn views_on(d: &Drawing, sheet: u64) -> Result<Vec<(u64, ResolvedView, Vec2)>> {
    let fail = |e: DocError| bad(CMD, e.to_string());
    let mut on: Vec<u64> = d.views.iter().filter(|v| v.sheet == sheet).map(|v| v.id).collect();
    on.sort_unstable();
    on.into_iter()
        .map(|id| -> Result<(u64, ResolvedView, Vec2)> { Ok((id, d.resolve_view(id).map_err(fail)?, d.view_centre(id).map_err(fail)?)) })
        .collect()
}

/// `e`'s message without the command's name: a view's reason in a sheet scene.
fn reason(e: EngineError) -> String {
    match e {
        EngineError::BadParams { msg, .. } => msg,
        e => e.to_string(),
    }
}

#[cfg(test)]
#[path = "drawing_scene_tests.rs"]
mod tests;
