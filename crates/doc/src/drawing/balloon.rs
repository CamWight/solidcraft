//! A drawing's balloons (W26, #3874): each points at a part placed in its view's model, by an edge
//! of it, to number that part on the drawing. A balloon saves what it points at by name, never a
//! picked point, as a dimension does ([`super::dimension`]): when the model changes it follows the
//! part, or says what it lost, and it never moves to another part.
//!
//! - **The part** it numbers is the occurrence its edge's body is placed through: the full path,
//!   from the top. A body of the design itself is not a placed part, so no balloon points at one.
//! - **The arrow** ends on the middle of its edge, whichever way the kernel runs the edge.
//! - **The balloon** sits at a place on paper, in millimetres from its view's centre, so it moves
//!   with the view and not with the model; or **on another balloon**, touching it in a stack. Each
//!   in a stack keeps its own edge, but only the first draws its arrow. When one goes, the first of
//!   those on it takes its place, and its arrow, if it had one, points at its own edge.
//!
//! The number a balloon shows comes from a parts list, worked out and never saved: the one it names,
//! else its sheet's only one, else the drawing's only one ([`Drawing::parts_list_for`]). It keeps the
//! balloon whatever becomes of the row.
//!
//! The rules, beside the record's: a balloon is on one of the drawing's views; its edge reference
//! keeps a dimension's rules, with an occurrence path of 1 to [`crate::MAX_PLACEMENT_DEPTH`] ids; its place
//! is on paper ([`is_paper_coordinate`]); the list it names is one of the drawing's parts lists; at
//! most [`MAX_BALLOONS`] balloons, listed by id. A stack is on one view, at most
//! [`MAX_STACK_BALLOONS`] balloons, and none stands on itself, however far round.
//!
//! [`Document::resolve_balloons`] finds each balloon's edge again as a dimension's is found
//! ([`super::reference::Status`]): by name, strictly, and what it can't decide it says.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use solidcraft_geom::drawing_view::is_paper_coordinate;
use solidcraft_geom::paper::MAX_SIDE_MM;
use solidcraft_geom::table::Ballooned;

use super::by_id::Record;
use super::reference::{Located, Lookup, ModelEdge, check_edge};
use super::table::{Kind, PartsList, SheetTable};
use super::{Drawing, require};
use crate::{DocError, Document, Model, Result, ids};

/// The most balloons a drawing may have.
pub const MAX_BALLOONS: usize = 10_000;

/// The most balloons in a stack.
pub const MAX_STACK_BALLOONS: usize = 64;

/// Where a balloon stands: at its own place, or on another balloon.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BalloonPlace {
    /// Its centre, in millimetres of paper from its view's centre.
    At([f64; 2]),
    On(Stacked),
}

impl From<[f64; 2]> for BalloonPlace {
    fn from(at: [f64; 2]) -> BalloonPlace {
        BalloonPlace::At(at)
    }
}

/// A place touching another balloon.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stacked {
    #[serde(with = "crate::ids")]
    pub on: u64,
    /// The way from that balloon's centre to this one's, in radians anticlockwise from the paper's +x.
    pub angle: f64,
}

/// One balloon of a drawing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Balloon {
    #[serde(with = "crate::ids")]
    pub id: u64,
    /// The view it points into and moves with.
    #[serde(with = "crate::ids")]
    pub view: u64,
    /// The edge its arrow points at. Its occurrence path is the part the balloon numbers.
    pub edge: ModelEdge,
    /// Where it stands.
    pub at: BalloonPlace,
    /// The parts list it names, when it names one.
    #[serde(default, with = "crate::ids::option", skip_serializing_if = "Option::is_none")]
    pub parts_list: Option<u64>,
}

impl Balloon {
    /// The part it numbers: the occurrences its edge's body is placed through, from the top.
    pub fn occurrence(&self) -> &[u64] {
        &self.edge.path
    }
}

impl Record for Balloon {
    fn id(&self) -> u64 {
        self.id
    }
}

impl Drawing {
    /// The parts list balloon `b` reads, by its id: the one it names, else its sheet's only one, else
    /// the drawing's only one. With none to go by, the lists it could name, by id.
    pub fn parts_list_for(&self, b: &Balloon) -> std::result::Result<(u64, PartsList), Vec<u64>> {
        let list = |t: &SheetTable| match t.kind {
            Kind::PartsList(list) => Some((t.id, list)),
            Kind::Revision => None,
        };
        let lists: Vec<(u64, u64, PartsList)> = self.tables.iter().filter_map(|t| list(t).map(|(id, l)| (id, t.sheet, l))).collect();
        if let Some(named) = lists.iter().find(|(id, ..)| Some(*id) == b.parts_list) {
            return Ok((named.0, named.2));
        }
        let sheet = self.view(b.view).map(|v| v.sheet);
        let mut on_sheet = lists.iter().filter(|(_, on, _)| Some(*on) == sheet);
        if let (Some(only), None) = (on_sheet.next(), on_sheet.next()) {
            return Ok((only.0, only.2));
        }
        match lists.as_slice() {
            [only] => Ok((only.0, only.2)),
            _ => Err(lists.iter().map(|(id, ..)| *id).collect()),
        }
    }

    /// The balloons table `t` shows the rows of: those that read it, where its filter looks for them
    /// ([`solidcraft_geom::table::Filter::ballooned`]). None without that filter.
    pub fn balloons_shown_by(&self, t: &SheetTable) -> Vec<&Arc<Balloon>> {
        let Some(scope) = t.table.filter.ballooned else { return Vec::new() };
        let in_scope = |b: &Balloon| scope == Ballooned::Drawing || self.view(b.view).is_some_and(|v| v.sheet == t.sheet);
        self.balloons.iter().filter(|b| in_scope(b) && self.parts_list_for(b).is_ok_and(|(list, _)| list == t.id)).collect()
    }

    /// The balloons `b` stands on, from its stack's first to the one under it: none for one at its
    /// own place. A stack that goes round or past its size stops, as its rules refuse one.
    pub fn stacked_under(&self, b: &Balloon) -> Vec<&Arc<Balloon>> {
        let mut under = Vec::new();
        let mut place = b.at;
        while let BalloonPlace::On(Stacked { on, .. }) = place {
            let next = self.balloons.index_of(on).and_then(|at| self.balloons.get(at));
            let Some(next) = next.filter(|_| under.len() < MAX_STACK_BALLOONS) else { break };
            under.push(next);
            place = next.at;
        }
        under.reverse();
        under
    }

    /// The balloons of `b`'s stack, by id: itself alone when none stands on another with it.
    pub fn stack_of(&self, b: &Balloon) -> Vec<u64> {
        let first = |one: &Balloon| self.stacked_under(one).first().map_or(one.id, |under| under.id);
        let root = first(b);
        self.balloons.iter().filter(|one| first(one) == root).map(|one| one.id).collect()
    }

    /// Before `gone` (by id) are taken out: hands each one's place to the first of those on it, and
    /// the others of those on it stand on that one.
    pub(super) fn hand_on_stacks(&mut self, gone: &[u64]) {
        let mut on_it: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for b in self.balloons.iter() {
            if let BalloonPlace::On(Stacked { on, .. }) = b.at {
                on_it.entry(on).or_default().push(b.id);
            }
        }
        for id in gone {
            let Some(place) = self.balloons.by_id(*id).map(|b| b.at) else { continue };
            let mut above = on_it.remove(id).unwrap_or_default();
            let next = (!above.is_empty()).then(|| above.remove(0));
            if let BalloonPlace::On(Stacked { on, .. }) = place
                && let Some(beside) = on_it.get_mut(&on)
            {
                beside.retain(|one| one != id);
                beside.extend(next);
                beside.sort_unstable();
            }
            let Some(next) = next else { continue };
            for (one, new) in std::iter::once((next, Some(place))).chain(above.iter().map(|one| (*one, None))) {
                let at = self.balloons.index_of(one);
                let Some(balloon) = at.and_then(|at| self.balloons.get_mut(at)).map(Arc::make_mut) else { continue };
                balloon.at = match (new, balloon.at) {
                    (Some(place), _) => place,
                    (None, BalloonPlace::On(stacked)) => BalloonPlace::On(Stacked { on: next, ..stacked }),
                    (None, at) => at,
                };
            }
            on_it.entry(next).or_default().extend(above);
        }
    }

    /// The balloons' rules (the module's notes). The views and the tables are checked first, so they
    /// are in id order here.
    pub(super) fn check_balloons(&self) -> Result<()> {
        self.balloons.check(MAX_BALLOONS, "balloon", |b| self.check_balloon(b))?;
        self.check_stacks()
    }

    /// One walk down from each balloon on another, at most a stack's size long.
    fn check_stacks(&self) -> Result<()> {
        let mut sizes: BTreeMap<u64, usize> = BTreeMap::new();
        for b in self.balloons.iter().filter(|b| matches!(b.at, BalloonPlace::On(_))) {
            let name = ids::encode(b.id);
            let (mut under, mut hops) = (&**b, 0);
            while let BalloonPlace::On(Stacked { on, .. }) = under.at {
                hops += 1;
                require(hops < MAX_STACK_BALLOONS, || {
                    format!("balloon {name} stands in a stack that goes round or is over {MAX_STACK_BALLOONS} balloons")
                })?;
                let next = self.balloons.by_id(on);
                under =
                    next.ok_or_else(|| DocError::Invalid(format!("balloon {name} stands on {}, which the drawing doesn't have", ids::encode(on))))?;
                require(under.view == b.view, || format!("balloon {name} stands on a balloon of another view"))?;
            }
            let size = sizes.entry(under.id).or_insert(1);
            *size += 1;
            require(*size <= MAX_STACK_BALLOONS, || format!("a stack is at most {MAX_STACK_BALLOONS} balloons"))?;
        }
        Ok(())
    }

    fn check_balloon(&self, b: &Balloon) -> Result<()> {
        require(self.view(b.view).is_some(), || format!("it is on view {}, which the drawing doesn't have", ids::encode(b.view)))?;
        if let Some(list) = b.parts_list {
            let named = || format!("it names table {}, which isn't one of the drawing's parts lists", ids::encode(list));
            require(self.tables.by_id(list).is_some_and(|t| matches!(t.kind, Kind::PartsList(_))), named)?;
        }
        check_edge(&b.edge)?;
        require(!b.edge.path.is_empty(), || "it points at a placed part: its edge's body is placed through an occurrence".into())?;
        match b.at {
            BalloonPlace::At([x, y]) => {
                require(is_paper_coordinate(x) && is_paper_coordinate(y), || format!("its place is at most {MAX_SIDE_MM} mm on paper"))
            }
            BalloonPlace::On(Stacked { angle, .. }) => {
                require(angle.is_finite() && angle.abs() <= std::f64::consts::TAU, || "its angle is at most a full turn".into())
            }
        }
    }
}

impl Document {
    /// The balloons `wanted` of `drawing` (one of this design's drawings) as `model` (this
    /// design's) has them now: each one's edge found again by name, and the middle of it. A body is
    /// looked up and its edge names worked out once for the call, however many balloons name it.
    /// `Err` only for an id the drawing doesn't have.
    pub fn resolve_balloons(&self, model: &Model, drawing: &Drawing, wanted: &[u64]) -> Result<Vec<Located>> {
        let mut lookup = Lookup::new(self, model);
        wanted
            .iter()
            .map(|id| {
                let b = drawing.balloons.by_id(*id).ok_or_else(|| DocError::Unknown(format!("balloon {}", ids::encode(*id))))?;
                Ok(lookup.middle(&b.edge))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "balloon_tests.rs"]
mod tests;
