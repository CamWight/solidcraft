use std::collections::BTreeSet;
use std::sync::Arc;

use solidcore_record::Vec2;
use solidcraft_geom::drawing_sheet::{self, Orientation, PaperSize};
use solidcraft_geom::drawing_view::ProjectedSide;

use super::*;
use crate::drawing::dimension::ModelDimension;
use crate::drawing::table::{MassFormat, PartsList};
use crate::drawing::tests::plate;
use crate::drawing::view::ViewPlace;
use crate::drawing::{SheetComponent, Source};
use crate::drawing_view::{Exploded, Pose, StudyStep, ViewConfiguration, ViewOverrides};

/// The plate drawing's front view and the right view projected from it.
fn front_and_right(d: &Drawing) -> (u64, u64) {
    let front = d.views.iter().find(|v| matches!(v.kind, ViewKind::Base { .. }) && v.sheet == d.sheets[0].id).unwrap().id;
    let right = d.views.iter().find(|v| v.kind == ViewKind::Projected { parent: front, side: ProjectedSide::Right }).unwrap().id;
    (front, right)
}

/// Onto `d`'s first sheet, where the copy was, for its own source.
fn here(d: &Drawing) -> Placement {
    Placement { onto: d.sheets[0].id, by: Vec2::ZERO, source: d.sources[0].id }
}

/// A drawing of another design: one A3 sheet, no views.
fn other() -> Drawing {
    let mut id_source = IdSource::seeded(13066);
    let a3 = drawing_sheet::Sheet::standard(PaperSize::A3, Orientation::Landscape);
    let mut d = Drawing::new("Other", "en", "Sheet 1", a3, None, &BTreeSet::new(), &mut id_source).unwrap();
    let source = d.allocate_id(&mut id_source).unwrap();
    d.sources.push(Source { id: source });
    d
}

/// A parts list of `d`'s design on its first sheet, its id.
fn add_list(d: &mut Drawing, id_source: &mut IdSource) -> u64 {
    let id = d.allocate_id(id_source).unwrap();
    let of = SheetComponent { source: d.sources[0].id, component: 0 };
    let list =
        PartsList { of, view: Default::default(), group: Default::default(), fold: false, purchased_contents: false, mass: MassFormat::default() };
    let kind = Kind::PartsList(list);
    d.tables.add([Arc::new(SheetTable { id, sheet: d.sheets[0].id, table: kind.default_table(), place: kind.default_place(), kind })]);
    id
}

/// A pasted dimension measures in its view's copy and shows no model dimension: the one it was
/// copied from still shows it, so the model dimension shows once.
#[test]
fn a_pasted_dimension_shows_no_model_dimension() {
    let mut d = plate();
    let (front, _) = front_and_right(&d);
    let shown = ModelDimension::Feature { feature: 2, param: "Distance".into() };
    Arc::make_mut(&mut d.dimensions[0]).from_model = Some(shown.clone());
    let (old, from) = (d.dimensions[0].id, d.clone());
    let made = d.paste_records(&from, &from.with_contents(vec![front]), &[], here(&from), &mut IdSource::seeded(1)).unwrap();
    let copy = made.ids.iter().find(|(kind, was, _)| *kind == "dimension" && *was == old).map(|(_, _, id)| *id).unwrap();
    assert_eq!(d.dimension(copy).unwrap().from_model, None);
    assert_eq!(d.dimension(copy).unwrap().view, made.views[0]);
    assert_eq!(d.dimension(old).unwrap().from_model, Some(shown));
    d.validate().unwrap();
}

/// A balloon names its parts list's copy when the list is pasted too; the list itself when the
/// drawing it is pasted into has it; else none, as after a delete.
#[test]
fn a_pasted_balloon_keeps_the_parts_list_its_drawing_has() {
    let mut d = plate();
    let (front, _) = front_and_right(&d);
    let list = add_list(&mut d, &mut IdSource::seeded(2));
    Arc::make_mut(&mut d.balloons[0]).parts_list = Some(list);
    let from = d.clone();
    let views = from.with_contents(vec![front]);
    let balloon_of = |d: &Drawing, made: &Pasted| {
        let copy = made.ids.iter().find(|(kind, ..)| *kind == "balloon").map(|(_, _, id)| *id).unwrap();
        d.balloons.by_id(copy).unwrap().parts_list
    };

    let made = d.paste_records(&from, &views, &[], here(&from), &mut IdSource::seeded(3)).unwrap();
    assert_eq!(balloon_of(&d, &made), Some(list), "the same drawing keeps the list");

    let mut elsewhere = other();
    let place = here(&elsewhere);
    let made = elsewhere.paste_records(&from, &views, &[], place, &mut IdSource::seeded(4)).unwrap();
    assert_eq!(balloon_of(&elsewhere, &made), None, "another drawing hasn't the list");

    let mut elsewhere = other();
    let with_list = OnSheet { tables: vec![list], ..views.clone() };
    let place = here(&elsewhere);
    let made = elsewhere.paste_records(&from, &with_list, &[], place, &mut IdSource::seeded(5)).unwrap();
    assert_eq!(balloon_of(&elsewhere, &made), Some(made.tables[0]), "the list's copy");
    let Kind::PartsList(copied) = &elsewhere.tables.by_id(made.tables[0]).unwrap().kind else { panic!("a parts list") };
    assert_eq!(copied.of.source, elsewhere.sources[0].id);
}

/// A projected view pasted without its parent is a base view that looks as it did, and draws the
/// same state of the assembly: exploded, posed, a configuration's row, as its chain set them.
#[test]
fn a_projected_view_pasted_alone_keeps_its_assembly_state() {
    let mut d = plate();
    let (front, right) = front_and_right(&d);
    let state = ViewOverrides {
        exploded: Some(Exploded::View(9)),
        pose: Some(Pose::Study(StudyStep { id: 5, step: 2 })),
        configuration: Some(ViewConfiguration::Row(4)),
        ..d.view(front).unwrap().settings.clone()
    };
    Arc::make_mut(d.views.iter_mut().find(|v| v.id == front).unwrap()).settings = state;
    let from = d.clone();
    let before = from.resolve_view(right).unwrap();
    let by = Vec2::new(10.0, -5.0);
    let place = Placement { by, ..here(&from) };
    let made = d.paste_records(&from, &from.with_contents(vec![right]), &[], place, &mut IdSource::seeded(6)).unwrap();
    let copy = made.views[0];
    assert!(made.bases.contains(&copy) && made.old_bases.contains(&right) && made.based_across == 0);
    assert!(matches!(d.view(copy).unwrap().kind, ViewKind::Base { .. }));
    let after = d.resolve_view(copy).unwrap();
    assert_eq!(after.exploded.value, Exploded::View(9));
    assert_eq!(after.pose.value, Pose::Study(StudyStep { id: 5, step: 2 }));
    assert_eq!(after.configuration.value, ViewConfiguration::Row(4));
    assert_eq!((after.scale.value, after.visual_style.value), (before.scale.value, before.visual_style.value));
    assert_eq!(d.view(copy).unwrap().place, {
        let at = from.view_centre(right).unwrap() + by;
        ViewPlace::At([at.x, at.y])
    });
    d.validate().unwrap();
}

/// Every record pasted gets an id no record of the drawing had, each a different one, allocated
/// in one go; a copy naming a view its drawing hasn't is refused with the drawing unchanged.
#[test]
fn every_pasted_record_gets_a_new_id_and_a_refusal_changes_nothing() {
    let mut d = plate();
    let had: BTreeSet<u64> = d.ids().collect();
    let from = d.clone();
    let sheets: Vec<u64> = from.sheets.iter().map(|s| s.id).collect();
    let all = sheets.iter().fold(OnSheet::default(), |mut all, id| {
        let on = from.on_sheet(*id);
        all.views.extend(on.views);
        all.dimensions.extend(on.dimensions);
        all.balloons.extend(on.balloons);
        all.notes.extend(on.notes);
        all
    });
    let made = d.paste_records(&from, &all, &sheets, here(&from), &mut IdSource::seeded(7)).unwrap();
    let new: BTreeSet<u64> = made.ids.iter().map(|(_, _, id)| *id).collect();
    assert_eq!(new.len(), made.ids.len());
    assert!(new.is_disjoint(&had));
    assert_eq!(made.sheets.len(), 2);
    assert!(d.sheets.iter().filter(|s| made.sheets.contains(&s.id)).all(|s| s.name.starts_with("Sheet ")), "taken names move on");
    d.validate().unwrap();

    let mut d = from.clone();
    let missing = OnSheet { views: vec![u64::MAX], ..OnSheet::default() };
    assert!(
        d.paste_records(&from, &missing, &[], here(&from), &mut IdSource::seeded(8)).unwrap_err().to_string().starts_with("the copy has no view")
    );
    assert_eq!(d, from);
}
