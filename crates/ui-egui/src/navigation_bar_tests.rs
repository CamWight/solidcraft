use super::*;
use crate::navigation_bar_view::measure;
use egui::{Rect, pos2, vec2};
use serde_json::json;

#[test]
fn slot_catalogue_uses_registered_commands_and_existing_icons() {
    let mut ids = std::collections::BTreeSet::new();
    for slot in CATALOGUE {
        assert!(ids.insert(slot.id), "duplicate catalogue id {}", slot.id);
        match slot.command {
            Some(command) => assert!(solidcraft_engine::find_command(command).is_some(), "{}", slot.id),
            None => assert!(["projection", "display", "separator"].contains(&slot.id)),
        }
        if slot.kind != Kind::Separator {
            assert!(!crate::icon_svg::resolve(slot.icon).placeholder, "{}: {}", slot.id, slot.icon);
        }
    }
}

#[test]
fn default_slots_and_canvas_availability_preserve_navigation() {
    let bar = NavigationBar::default();
    assert_eq!(bar.slots, ["orbit", "pan", "zoom", "separator", "fit", "look_at", "home", "separator", "visual_style", "display"]);
    let flat = NavCaps { orbit: false, pick: false };
    for caps in [NavCaps::MODEL, NavCaps::RENDER, flat] {
        let shown: Vec<_> = CATALOGUE.iter().filter(|slot| slot.available(caps)).map(|slot| slot.id).collect();
        for id in ["orbit", "spin", "look_at", "projection", "visual_style"] {
            assert_eq!(shown.contains(&id), caps.orbit, "{caps:?}: {id}");
        }
        for id in ["pan", "zoom", "fit", "home", "display", "grid", "ground_shadow", "ambient_occlusion"] {
            assert!(shown.contains(&id), "{caps:?}: {id}");
        }
    }
    let mut app = crate::SolveApp::new(Default::default(), Default::default());
    app.run("sketch.create", json!({"plane":"XY"})).unwrap();
    assert!(find("visual_style").unwrap().available(canvas_caps(&app)), "the previous bar kept style available in a sketch");
}

#[test]
fn measured_bar_drops_labels_then_overflows_in_order_without_compass_overlap() {
    let prefs = NavigationBar { labels: Labels::Labels, ..Default::default() };
    for width in [5.0, 39.0, 46.0, 90.0, 160.0, 250.0, 310.0, 700.0, 1400.0] {
        for height in [30.0, 80.0, 130.0, 240.0, 720.0] {
            let canvas = Rect::from_min_size(pos2(20.0, 30.0), vec2(width, height));
            let bar = measure(canvas, &prefs, NavCaps::MODEL, |_| 50.0);
            let layout = crate::viewport_layout::Layout::with_content(canvas, canvas, &[], 1.0, crate::prefs::NavigatorStyle::Cube, bar.rect);
            assert!(!layout.axis_view.intersect(bar.rect).is_positive());
            if !bar.rect.is_positive() {
                continue;
            }
            assert!(canvas.contains_rect(bar.rect));
            assert_eq!(bar.rect.height(), prefs.size.points() + 4.0);
            let mut previous = bar.rect.left();
            for (_, rect) in &bar.slots {
                assert!(bar.rect.contains_rect(*rect));
                assert!(rect.left() >= previous);
                previous = rect.right();
            }
            if bar.more.is_positive() {
                assert!(!bar.labels);
                assert!(bar.rect.contains_rect(bar.more));
                assert!(bar.more.left() >= previous);
            }
            let order: Vec<_> = bar.slots.iter().map(|(slot, _)| slot.id).chain(bar.overflow.iter().map(|slot| slot.id)).collect();
            assert_eq!(order, prefs.slots);
        }
    }
    let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1400.0, 720.0));
    assert!(measure(wide, &prefs, NavCaps::MODEL, |_| 50.0).labels);
    let middle = Rect::from_min_size(pos2(0.0, 0.0), vec2(310.0, 720.0));
    let bar = measure(middle, &prefs, NavCaps::MODEL, |_| 50.0);
    assert!(!bar.labels && bar.overflow.is_empty(), "text disappears before slots fold");
}

#[test]
fn measured_bar_honours_size_position_and_ignores_unknown_slots() {
    let canvas = Rect::from_min_size(pos2(20.0, 30.0), vec2(700.0, 500.0));
    for size in [Size::Compact, Size::Regular] {
        for position in [Position::Bottom, Position::BottomLeft, Position::BottomRight] {
            let prefs: NavigationBar = serde_json::from_value(json!({"slots":["future", "pan", "fit"], "size":size, "position":position})).unwrap();
            let bar = measure(canvas, &prefs, NavCaps::MODEL, |_| 0.0);
            assert_eq!(bar.slots.iter().map(|(slot, _)| slot.id).collect::<Vec<_>>(), ["pan", "fit"]);
            assert!(bar.slots.iter().all(|(_, rect)| rect.height() == size.points()));
            match position {
                Position::Bottom => assert_eq!(bar.rect.center().x, canvas.center().x),
                Position::BottomLeft => assert_eq!(bar.rect.left(), canvas.left() + 5.0),
                Position::BottomRight => assert_eq!(bar.rect.right(), canvas.right() - 5.0),
            }
        }
    }
}

#[test]
fn navigation_slot_visuals_never_use_focus_as_a_fill() {
    use crate::ui_kit::{NavigationFill, navigation_visual};
    for kind in [Kind::Action, Kind::Tool, Kind::Toggle, Kind::Menu] {
        for keyboard_focus in [false, true] {
            assert_eq!(
                navigation_visual(kind, false, false, false, keyboard_focus),
                (NavigationFill::None, keyboard_focus),
                "{kind:?}: idle or released"
            );
            for (hovered, pressed) in [(true, false), (false, true), (true, true)] {
                assert_eq!(
                    navigation_visual(kind, false, hovered, pressed, keyboard_focus),
                    (NavigationFill::Hover, keyboard_focus),
                    "{kind:?}: hover or press"
                );
            }
            let active_fill = if kind == Kind::Action { NavigationFill::None } else { NavigationFill::Active };
            assert_eq!(
                navigation_visual(kind, true, false, false, keyboard_focus),
                (active_fill, keyboard_focus),
                "{kind:?}: active tool/toggle or open menu"
            );
        }
    }
}

#[test]
fn navigation_slot_pointer_clicks_release_focus_and_leave_no_fill() {
    use crate::theme::Tokens;
    use crate::ui_kit::{NavigationButton, navigation_button};
    use egui::{Color32, Event, RawInput, Shape};

    fn fill(output: &egui::FullOutput, bounds: Rect) -> Option<Color32> {
        let mut fill = None;
        for shape in &output.shapes {
            crate::test_support::each_shape(&shape.shape, &mut |shape| {
                if let Shape::Rect(rect) = shape
                    && rect.rect == bounds
                    && rect.fill != Color32::TRANSPARENT
                {
                    fill = Some(rect.fill);
                }
            });
        }
        fill
    }

    for kind in [Kind::Action, Kind::Tool, Kind::Toggle] {
        let ctx = crate::fonts::test_context();
        let bounds = Rect::from_min_size(pos2(20.0, 30.0), vec2(26.0, 26.0));
        let draw = |events, selected| {
            let mut focused = false;
            let mut clicked = false;
            let output = crate::test_support::frame(
                &ctx,
                RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 400.0))),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| {
                    let response = navigation_button(
                        ui,
                        bounds,
                        NavigationButton { id: "regression", label: "Fit", icon: "fit", selected, kind, labels: false, menu: false },
                    );
                    focused = response.has_focus();
                    clicked = response.clicked();
                },
            );
            (output, focused, clicked)
        };
        draw(vec![], false);
        draw(vec![Event::PointerMoved(bounds.center())], false);
        let pressed = draw(crate::test_support::pointer(bounds.center(), true), false);
        assert_eq!(fill(&pressed.0, bounds), Some(Tokens::get().hover), "{kind:?}: pressed");
        let released = draw(crate::test_support::pointer(bounds.center(), false), false);
        assert!(released.2, "{kind:?}: real pointer activation");
        assert!(!released.1, "{kind:?}: pointer activation must surrender focus");
        let away = draw(vec![Event::PointerMoved(pos2(200.0, 200.0))], false);
        assert_eq!(fill(&away.0, bounds), None, "{kind:?}: no lingering fill");
        if kind != Kind::Action {
            let active = draw(vec![], true);
            assert_eq!(fill(&active.0, bounds), Some(Tokens::get().accent_soft));
            draw(crate::test_support::pointer(bounds.center(), true), true);
            let released = draw(crate::test_support::pointer(bounds.center(), false), true);
            assert!(released.2 && !released.1);
            let off = draw(vec![Event::PointerMoved(pos2(200.0, 200.0))], false);
            assert_eq!(fill(&off.0, bounds), None, "{kind:?}: tool/toggle released");
        }
    }
}
