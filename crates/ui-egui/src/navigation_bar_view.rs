//! Measured single-row navigation bar. Commands own all view changes.
use crate::navigation::NavCaps;
use crate::navigation_bar::{self, Idle, Kind, Labels, NavigationBar, Position, Slot};
use crate::theme::Tokens;
use crate::ui_kit::{self, Container, Menu, MenuItem, Surface};
use crate::{SolveApp, viewport::NavMode};
use egui::{Align2, FontId, Rect, pos2, vec2};
use serde_json::json;

pub(crate) struct BarLayout {
    pub rect: Rect,
    pub slots: Vec<(&'static Slot, Rect)>,
    pub overflow: Vec<&'static Slot>,
    pub more: Rect,
    pub labels: bool,
}

/// Drop labels before moving a suffix into More. All rectangles use logical UI points.
pub(crate) fn measure(content: Rect, prefs: &NavigationBar, caps: NavCaps, label_width: impl Fn(&Slot) -> f32) -> BarLayout {
    let mut result = BarLayout { rect: Rect::NOTHING, slots: Vec::new(), overflow: Vec::new(), more: Rect::NOTHING, labels: false };
    let button = prefs.size.points();
    let height = button + 4.0;
    if !content.is_finite() || content.width() < button + 20.0 || content.height() < height + 7.0 {
        return result;
    }
    let slots: Vec<_> =
        prefs.slots.iter().take(navigation_bar::MAX_SLOTS).filter_map(|id| navigation_bar::find(id)).filter(|s| s.available(caps)).collect();
    if slots.is_empty() {
        return result;
    }
    let width = |slot: &Slot, labels| {
        if slot.kind == Kind::Separator {
            10.0
        } else if slot.kind == Kind::Menu {
            button + 12.0
        } else if labels {
            button + label_width(slot).max(0.0) + 12.0
        } else {
            button
        }
    };
    let total = |labels| slots.iter().map(|slot| width(slot, labels) + 4.0).sum::<f32>() + 6.0;
    let available = content.width() - 10.0;
    result.labels = prefs.labels == Labels::Labels && total(true) <= available;
    let mut visible = slots.len();
    let mut row_width = total(result.labels);
    if row_width > available {
        visible = 0;
        row_width = button + 10.0;
        for slot in &slots {
            let next = row_width + width(slot, false) + 4.0;
            if next > available {
                break;
            }
            row_width = next;
            visible += 1;
        }
        result.overflow.extend(slots.iter().skip(visible).copied());
    }
    let left = match prefs.position {
        Position::Bottom => content.center().x - row_width * 0.5,
        Position::BottomLeft => content.left() + 5.0,
        Position::BottomRight => content.right() - row_width - 5.0,
    };
    result.rect = Rect::from_min_size(pos2(left, content.bottom() - height - 7.0), vec2(row_width, height));
    let mut x = left + 5.0;
    for slot in slots.iter().take(visible) {
        let w = width(slot, result.labels);
        result.slots.push((*slot, Rect::from_min_size(pos2(x, result.rect.top() + 2.0), vec2(w, button))));
        x += w + 4.0;
    }
    if !result.overflow.is_empty() {
        result.more = Rect::from_min_size(pos2(x, result.rect.top() + 2.0), vec2(button, button));
    }
    result
}

pub(crate) fn measured(app: &SolveApp, ctx: &egui::Context, content: Rect) -> BarLayout {
    let t = Tokens::get();
    measure(content, &app.preferences.navigation_bar, navigation_bar::canvas_caps(app), |slot| {
        ctx.fonts_mut(|fonts| fonts.layout_no_wrap(app.tr(slot.label).to_owned(), FontId::proportional(t.font_label), t.text).size().x)
    })
}

fn selected(app: &SolveApp, id: &str) -> bool {
    match id {
        "orbit" => app.viewport.nav == Some(NavMode::Orbit),
        "pan" => app.viewport.nav == Some(NavMode::Pan),
        "zoom" => app.viewport.nav == Some(NavMode::Zoom),
        "spin" => app.viewport.spin,
        "projection" => app.ui.perspective,
        "grid" => app.ui.show_grid,
        "ground_shadow" => app.ui.ground_shadow,
        "ambient_occlusion" => app.preferences.ambient_occlusion,
        _ => false,
    }
}

/// #6143 will replace this single compatibility path with a registered command.
fn toggle_projection(app: &mut SolveApp) {
    app.ui.perspective = !app.ui.perspective;
}

fn activate(app: &mut SolveApp, slot: &Slot) {
    match slot.id {
        "look_at" => crate::viewport::look_at_selection(app),
        "projection" => toggle_projection(app),
        _ => {
            if let Some(command) = slot.command {
                let params = match slot.kind {
                    Kind::Tool => json!({"tool": if selected(app, slot.id) { "none" } else { slot.id }}),
                    _ if slot.id == "ambient_occlusion" => json!({"enabled": !app.preferences.ambient_occlusion}),
                    _ => json!({}),
                };
                let _ = app.run(command, params);
            }
        }
    }
}

fn tooltip(app: &SolveApp, response: egui::Response, slot: &Slot) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.horizontal(|ui| {
            ui.label(app.tr(slot.label));
            if let Some(key) = slot.command.and_then(|id| crate::keymap::display_for(app, id)) {
                ui_kit::shortcut_keycap(ui, &key);
            }
        });
    })
}

pub(crate) fn show(app: &mut SolveApp, ui: &mut egui::Ui, rect: Rect) {
    if !app.ui.show_navigation_bar {
        return;
    }
    let Some(layout) = app.viewport.layout.as_ref().filter(|l| l.navigation.is_positive()) else { return };
    let bar = measured(app, ui.ctx(), layout.content);
    if !bar.rect.is_positive() {
        return;
    }
    crate::scenario::publish_rect("nav:outer", bar.rect);
    let menu_id = ui.id().with("navigation_bar_menu");
    let mut open = ui.data(|d| d.get_temp::<String>(menu_id)).unwrap_or_default();
    let active = ui.rect_contains_pointer(bar.rect)
        || ui.rect_contains_pointer(crate::viewport::axis_rect(app, rect))
        || app.viewport.axis_drag.is_some()
        || app.viewport.nav.is_some()
        || app.viewport.primary_nav.is_some()
        || app.viewport.orbit_from.is_some()
        || (app.viewport.mouse.is_some() && ui.input(|i| i.pointer.middle_down()))
        || app.viewport.modifier_nav.is_some()
        || app.viewport.spin
        || !open.is_empty();
    let opacity = match app.preferences.navigation_bar.idle {
        _ if active => 1.0,
        Idle::Always => 1.0,
        Idle::Fade => 0.45,
        Idle::Hide => 0.0,
    };
    let mut anchor = bar.more;
    ui.scope(|ui| {
        ui.set_opacity(opacity);
        let t = Tokens::get();
        let surface = Container::new(ui.ctx(), Surface::Navigation).frame(ui.ctx(), t.overlay, crate::frame::Side::Left);
        Container::paint_frame(ui.painter(), surface, bar.rect.shrink(surface.stroke.width));
        for (slot, bounds) in &bar.slots {
            crate::scenario::publish_target(&format!("nav:slot:{}", slot.id), *bounds);
            if slot.kind == Kind::Separator {
                ui_kit::navigation_separator(ui, *bounds);
                continue;
            }
            let response = ui_kit::navigation_button(
                ui,
                *bounds,
                ui_kit::NavigationButton {
                    id: slot.id,
                    kind: slot.kind,
                    icon: slot.icon,
                    label: app.tr(slot.label),
                    selected: selected(app, slot.id) || open == slot.id,
                    labels: bar.labels && slot.kind != Kind::Menu,
                    menu: slot.kind == Kind::Menu,
                },
            );
            if tooltip(app, response, slot).clicked() {
                if slot.kind == Kind::Menu {
                    open = if open == slot.id { String::new() } else { slot.id.to_owned() };
                } else {
                    activate(app, slot);
                }
            }
            if slot.id == "display" {
                crate::scenario::publish_handle("nav:view", bounds.center());
            }
            if open == slot.id {
                anchor = *bounds;
            }
        }
        if bar.more.is_positive() {
            let response = ui_kit::navigation_button(
                ui,
                bar.more,
                ui_kit::NavigationButton {
                    id: "more",
                    kind: Kind::Menu,
                    icon: "chevron",
                    label: app.tr("navbar.more"),
                    selected: open == "more" || bar.overflow.iter().any(|slot| slot.id == open),
                    labels: false,
                    menu: false,
                },
            )
            .on_hover_text(app.tr("navbar.more"));
            crate::scenario::publish_target("nav:more", response.rect);
            if response.clicked() {
                open = if open == "more" { String::new() } else { "more".to_owned() };
            }
        }
    });
    if !open.is_empty() && anchor.is_positive() {
        let child_popup_open = egui::Popup::is_any_open(ui.ctx());
        let ctx = ui.ctx().clone();
        let menu = Container::new(&ctx, Surface::Popup)
            .area(menu_id.with("popup"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(anchor.right_top() - vec2(0.0, 6.0))
            .show(&ctx, |ui| {
                Menu::show(ui, 240.0, |ui| match open.as_str() {
                    "visual_style" => {
                        if style_menu(app, ui) {
                            open.clear();
                        }
                    }
                    "display" => display_menu(app, ui),
                    "more" => overflow_menu(app, ui, &bar.overflow, &mut open),
                    _ => open.clear(),
                });
            });
        let outside = !child_popup_open
            && ui.input(|i| {
                i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !menu.response.rect.contains(p) && !bar.rect.contains(p))
            });
        if outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            open.clear();
        }
    }
    ui.data_mut(|d| d.insert_temp(menu_id, open));
}

fn style_menu(app: &mut SolveApp, ui: &mut egui::Ui) -> bool {
    let mut chosen = false;
    for (index, style) in ["shaded_with_edges", "shaded", "wireframe", "shaded_with_hidden_edges"].into_iter().enumerate() {
        let label = format!("navbar.style.{style}");
        let mut item = MenuItem::new(style, app.tr(&label), "style");
        item.checked = Some(app.ui.visual_style == index as u8);
        let response = item.show(ui);
        crate::scenario::publish_target(&format!("nav:style:{style}"), response.rect);
        if response.clicked() {
            let _ = app.run("view.visual_style", json!({"style":style}));
            chosen = true;
        }
    }
    chosen
}

fn overflow_menu(app: &mut SolveApp, ui: &mut egui::Ui, slots: &[&Slot], open: &mut String) {
    for slot in slots {
        if slot.kind == Kind::Separator {
            let start = ui.next_widget_position();
            Menu::separator(ui);
            let bounds = Rect::from_min_max(start, pos2(start.x + ui.available_width(), ui.next_widget_position().y));
            crate::scenario::publish_target(&format!("nav:slot:{}", slot.id), bounds);
            continue;
        }
        let mut item = MenuItem::new(slot.id, app.tr(slot.label), slot.icon);
        if matches!(slot.kind, Kind::Tool | Kind::Toggle) {
            item.checked = Some(selected(app, slot.id));
        }
        item.submenu = slot.kind == Kind::Menu;
        let key = slot.command.and_then(|id| crate::keymap::display_for(app, id)).unwrap_or_default();
        item.shortcut = &key;
        let response = item.show(ui);
        crate::scenario::publish_target(&format!("nav:slot:{}", slot.id), response.rect);
        if slot.id == "display" {
            crate::scenario::publish_handle("nav:view", response.rect.center());
        }
        if tooltip(app, response, slot).clicked() {
            if slot.kind == Kind::Menu {
                *open = slot.id.to_owned();
            } else {
                activate(app, slot);
                open.clear();
            }
        }
    }
}

fn display_menu(app: &mut SolveApp, ui: &mut egui::Ui) {
    Menu::heading(ui, app.tr("navbar.display"));
    ui.label(app.tr("ui.graphics_preset"));
    crate::prefs::graphics_preset_selector(app, ui);
    Menu::separator(ui);
    for id in ["grid", "ground_shadow", "ambient_occlusion", "projection"] {
        let Some(slot) = navigation_bar::find(id).filter(|slot| slot.available(navigation_bar::canvas_caps(app))) else { continue };
        let mut item = MenuItem::new(slot.id, app.tr(slot.label), slot.icon);
        item.checked = Some(selected(app, slot.id));
        let response = item.show(ui);
        crate::scenario::publish_target(&format!("view:{}", slot.id), response.rect);
        if response.clicked() {
            activate(app, slot);
        }
    }
    let tool = ui_kit::checkbox(ui, crate::i18n::tr_label("Show tool at cursor"), &mut app.ui.tool_cursor);
    crate::scenario::publish_handle("view:tool_cursor", tool.rect.center());
    Menu::separator(ui);
    let cursor = &mut app.ui.agent_cursor;
    let show = ui_kit::checkbox(ui, crate::i18n::tr_label("Show Agent Cursor"), &mut cursor.show);
    crate::scenario::publish_handle("view:agent_cursor", show.rect.center());
    ui.add_enabled_ui(cursor.show, |ui| {
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr_label("Speed"));
            crate::agent_cursor::speed_slider(ui, &mut cursor.speed);
        });
        ui_kit::checkbox(ui, crate::i18n::tr_label("Follow camera (Look At sketches)"), &mut cursor.follow_camera);
    });
    if crate::agent_chat::shown(app) {
        Menu::separator(ui);
        let mut open = app.agent_chat.open;
        let response = ui_kit::checkbox(ui, &crate::agent_chat::panel_tip(), &mut open);
        crate::scenario::publish_handle("view:agent_chat", response.rect.center());
        if response.changed() {
            app.agent_chat.open = open;
            app.preferences.agent_chat_minimized &= !open;
        }
    }
}
