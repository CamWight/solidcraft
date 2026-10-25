//! Navigation controls share kit button, divider and keycap chrome.
use crate::{icons, navigation_bar::Kind, theme::Tokens};
use egui::{Align2, FontId, Rect, Response, Sense, Ui, pos2, vec2};

pub(crate) struct NavigationButton<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub icon: &'a str,
    pub selected: bool,
    pub kind: Kind,
    pub labels: bool,
    pub menu: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavigationFill {
    None,
    Hover,
    Active,
}

/// Focus is an outline, never a latched fill. Actions have no persistent active state.
pub(crate) fn navigation_visual(kind: Kind, active: bool, hovered: bool, pressed: bool, keyboard_focus: bool) -> (NavigationFill, bool) {
    let fill = if active && matches!(kind, Kind::Tool | Kind::Toggle | Kind::Menu) {
        NavigationFill::Active
    } else if hovered || pressed {
        NavigationFill::Hover
    } else {
        NavigationFill::None
    };
    (fill, keyboard_focus)
}

pub(crate) fn navigation_button(ui: &mut Ui, rect: Rect, button: NavigationButton<'_>) -> Response {
    let t = Tokens::get();
    let response = ui.interact(rect, ui.make_persistent_id(("nav_slot", button.id)), Sense::click());
    // egui has no focus-visible flag here: pointer activation must not retain focus.
    let pressed = response.is_pointer_button_down_on();
    if pressed || response.clicked_by(egui::PointerButton::Primary) {
        response.surrender_focus();
    }
    let (fill, focus_ring) = navigation_visual(button.kind, button.selected, response.hovered(), pressed, response.has_focus());
    match fill {
        NavigationFill::None => {}
        NavigationFill::Hover => {
            ui.painter().rect_filled(rect, t.radius_sm, t.hover);
        }
        NavigationFill::Active => {
            ui.painter().rect_filled(rect, t.radius_sm, t.accent_soft);
        }
    }
    if focus_ring {
        ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    let side = rect.height();
    let icon = Rect::from_center_size(pos2(rect.left() + side * 0.5, rect.center().y), vec2(side - t.space_8, side - t.space_8));
    icons::paint_ink(ui.painter(), icon, button.icon, t.icon);
    if button.labels {
        ui.painter().text(
            pos2(rect.left() + side + t.space_4, rect.center().y),
            Align2::LEFT_CENTER,
            button.label,
            FontId::proportional(t.font_label),
            t.text,
        );
    }
    if button.menu {
        icons::paint_ink(
            ui.painter(),
            Rect::from_center_size(pos2(rect.right() - t.space_6, rect.center().y), vec2(t.space_8, t.space_8)),
            "chevron",
            t.icon,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), button.selected, button.label));
    response
}

pub(crate) fn navigation_separator(ui: &Ui, rect: Rect) {
    let t = Tokens::get();
    ui.painter().vline(rect.center().x, rect.shrink(t.space_6).y_range(), t.tertiary_border(t.border));
}

pub(crate) fn shortcut_keycap(ui: &mut Ui, shortcut: &str) {
    let t = Tokens::get();
    egui::Frame::NONE
        .fill(t.panel_header)
        .stroke(t.tertiary_border(t.border))
        .corner_radius(t.radius_sm.saturating_sub(1))
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(shortcut).monospace().color(t.text_dim));
        });
}
