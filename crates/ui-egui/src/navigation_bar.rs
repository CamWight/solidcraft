//! Stored navigation slots and their catalogue. Painting lives in `navigation_bar_view`.
use crate::{SolveApp, navigation::NavCaps};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_SLOTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Action,
    Tool,
    Toggle,
    Menu,
    Separator,
}

pub(crate) struct Slot {
    pub id: &'static str,
    pub kind: Kind,
    pub icon: &'static str,
    pub label: &'static str,
    pub command: Option<&'static str>,
    orbit: bool,
}

macro_rules! slot {
    ($id:literal, $kind:ident, $icon:literal, $command:expr, $orbit:literal) => {
        Slot { id: $id, kind: Kind::$kind, icon: $icon, label: concat!("navbar.", $id), command: $command, orbit: $orbit }
    };
}

pub(crate) const CATALOGUE: &[Slot] = &[
    slot!("orbit", Tool, "orbit", Some("view.tool"), true),
    slot!("pan", Tool, "pan", Some("view.tool"), false),
    slot!("zoom", Tool, "zoom", Some("view.tool"), false),
    slot!("spin", Toggle, "spin", Some("view.spin"), true),
    slot!("fit", Action, "fit", Some("view.fit"), false),
    slot!("look_at", Action, "lookat", Some("view.normal_to"), true),
    slot!("home", Action, "home", Some("view.home"), false),
    slot!("visual_style", Menu, "style", Some("view.visual_style"), true),
    slot!("projection", Toggle, "perspective", None, true),
    slot!("display", Menu, "settings", None, false),
    slot!("grid", Toggle, "style", Some("view.grid"), false),
    slot!("ground_shadow", Toggle, "style", Some("view.ground_shadow"), false),
    slot!("ambient_occlusion", Toggle, "settings", Some("view.ambient_occlusion"), false),
    slot!("separator", Separator, "", None, false),
];

pub(crate) fn find(id: &str) -> Option<&'static Slot> {
    CATALOGUE.iter().find(|slot| slot.id == id)
}

impl Slot {
    pub(crate) fn available(&self, caps: NavCaps) -> bool {
        !self.orbit || caps.orbit
    }
}

pub(crate) fn canvas_caps(app: &SolveApp) -> NavCaps {
    if crate::dialogs_sheet::flat_active(app) || crate::render_workspace::result_active(app) {
        NavCaps { orbit: false, pick: false }
    } else if crate::render_workspace::active(app) {
        NavCaps::RENDER
    } else {
        NavCaps::MODEL
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    #[default]
    Bottom,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    #[default]
    Compact,
    Regular,
}

impl Size {
    // Logical points: egui's UI zoom applies the preference's scale to these dimensions.
    pub(crate) fn points(self) -> f32 {
        match self {
            Self::Compact => 26.0,
            Self::Regular => 32.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Labels {
    #[default]
    Icons,
    Labels,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Idle {
    Always,
    #[default]
    Fade,
    Hide,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NavigationBar {
    pub slots: Vec<String>,
    pub position: Position,
    pub size: Size,
    pub labels: Labels,
    pub idle: Idle,
}

impl Default for NavigationBar {
    fn default() -> Self {
        Self {
            slots: ["orbit", "pan", "zoom", "separator", "fit", "look_at", "home", "separator", "visual_style", "display"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            position: Position::default(),
            size: Size::default(),
            labels: Labels::default(),
            idle: Idle::default(),
        }
    }
}

impl<'de> Deserialize<'de> for NavigationBar {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let mut bar = Self::default();
        if let Some(ids) = value.get("slots").and_then(Value::as_array) {
            let mut seen = std::collections::BTreeSet::new();
            let mut separators = 0;
            bar.slots = ids
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| {
                    if *id == "separator" {
                        if separators == 4 {
                            return false;
                        }
                        separators += 1;
                        true
                    } else {
                        seen.insert(*id)
                    }
                })
                .take(MAX_SLOTS)
                .map(str::to_owned)
                .collect();
        }
        if let Some(v) = value.get("position") {
            bar.position = serde_json::from_value(v.clone()).unwrap_or_default();
        }
        if let Some(v) = value.get("size") {
            bar.size = serde_json::from_value(v.clone()).unwrap_or_default();
        }
        if let Some(v) = value.get("labels") {
            bar.labels = serde_json::from_value(v.clone()).unwrap_or_default();
        }
        if let Some(v) = value.get("idle") {
            bar.idle = serde_json::from_value(v.clone()).unwrap_or_default();
        }
        Ok(bar)
    }
}

/// Interactive callers get errors for malformed values; saved files recover field by field.
pub(crate) fn validate(value: &Value) -> Result<(), String> {
    let object = value.as_object().ok_or("`navigation_bar` must be an object")?;
    for (key, value) in object {
        let valid = match key.as_str() {
            "slots" => value.as_array().is_some_and(|ids| ids.iter().all(Value::is_string)),
            "position" => serde_json::from_value::<Position>(value.clone()).is_ok(),
            "size" => serde_json::from_value::<Size>(value.clone()).is_ok(),
            "labels" => serde_json::from_value::<Labels>(value.clone()).is_ok(),
            "idle" => serde_json::from_value::<Idle>(value.clone()).is_ok(),
            _ => return Err(format!("unknown navigation_bar field `{key}`")),
        };
        if !valid {
            return Err(format!("invalid navigation_bar `{key}`"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "navigation_bar_tests.rs"]
mod tests;
