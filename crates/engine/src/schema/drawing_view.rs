//! Contracts and result types for the commands in `cmd/drawing_view.rs`.
//!
//! The view's own settings are `solidcraft_doc::drawing_view::ViewSettings`, which a saved view
//! (W12) keeps too. The contract lists the same fields, each read by that type's own reader, and
//! `the_contract_lists_the_view_settings` keeps the two lists the same.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solidcraft_doc::drawing_view::{TangentEdges, ViewDirection, ViewSettings};
use solidcraft_geom::drawing_sheet::{Orientation, PaperSize, SheetSize};
use solidcraft_geom::drawing_view::{NamedView, Scale};
use solidcraft_render::VisualStyle;

use super::Point2;

/// The lowest and highest picture resolution a sheet takes, in dots per inch.
pub const DPI_RANGE: std::ops::RangeInclusive<f64> = 25.0..=2400.0;

/// What `drawing.view.snapshot` takes: the view's settings, the page it fits on, and where its
/// centre goes.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct SnapshotParams {
    pub view: ViewParam,
    #[serde(deserialize_with = "super::present")]
    #[schemars(with = "ScaleParam")]
    pub scale: Option<ScaleParam>,
    pub display_style: StyleParam,
    pub reference_style: StyleParam,
    /// Whether reference parts hide the model's lines behind them (false when not given).
    pub reference_hides_model: bool,
    /// Model millimetres added on every side of the view's extents, so reference parts around
    /// the model show inside its boundary: 0 to 100000 (0 when not given).
    pub reference_margin_mm: f64,
    pub tangent_edges: TangentEdgesParam,
    pub sheet: SheetParam,
    /// Where the view's centre goes on the sheet, in paper millimetres from its lower-left
    /// corner (the sheet's centre when not given).
    #[serde(deserialize_with = "super::present")]
    #[schemars(with = "Point2")]
    pub position_mm: Option<Point2>,
}

impl SnapshotParams {
    pub fn settings(&self) -> ViewSettings {
        ViewSettings {
            view: self.view.0,
            scale: self.scale.map(|s| s.0),
            display_style: self.display_style.0,
            reference_style: self.reference_style.0,
            reference_hides_model: self.reference_hides_model,
            reference_margin_mm: self.reference_margin_mm,
            tangent_edges: self.tangent_edges.0,
        }
    }
}

// Each parameter type below reads as the type it wraps, so the contract accepts exactly what the
// view's settings do; it is there for its schema.

/// Which way the view looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct ViewParam(pub ViewDirection);

impl JsonSchema for ViewParam {
    fn schema_name() -> Cow<'static, str> {
        "DrawingView".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let names: Vec<Value> = NamedView::ALL.iter().filter_map(|v| serde_json::to_value(v).ok()).chain([json!("iso")]).collect();
        let vector = json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3});
        json_schema!({
            "description": "Which way the view looks: a named view (front when not given; iso is iso_front_right), or {direction, up, origin?}: the line of sight, the way up the paper (not along the direction) and the point the view's plane passes through (the world origin when not given).",
            "anyOf": [
                {"type": "string", "enum": names},
                {"type": "object", "properties": {"direction": vector, "up": vector, "origin": vector}, "required": ["direction", "up"], "additionalProperties": false}
            ]
        })
    }
}

/// A drawing scale, paper:model.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct ScaleParam(pub Scale);

impl JsonSchema for ScaleParam {
    fn schema_name() -> Cow<'static, str> {
        "DrawingScale".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The scale as paper:model text, such as \"1:2\" or \"5:1\", from 1:100000 to 1000:1. When not given, the largest ISO 5455 scale at which the view fits the sheet's drawing area.",
            "type": "string",
            "pattern": "^[0-9]+(\\.[0-9]+)?:[0-9]+(\\.[0-9]+)?$"
        })
    }
}

/// How the model draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct StyleParam(pub VisualStyle);

impl JsonSchema for StyleParam {
    fn schema_name() -> Cow<'static, str> {
        "VisualStyle".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let names: Vec<Value> = VisualStyle::ALL.iter().filter_map(|s| serde_json::to_value(s).ok()).collect();
        json_schema!({
            "description": "How the model draws: visible (the default), visible_hidden, shaded, shaded_visible or shaded_visible_hidden.",
            "type": "string",
            "enum": names
        })
    }
}

/// Tangent edges hidden or shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct TangentEdgesParam(pub TangentEdges);

impl JsonSchema for TangentEdgesParam {
    fn schema_name() -> Cow<'static, str> {
        "TangentEdges".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"description": "Edges where faces meet smoothly: hide (the default) or show.", "type": "string", "enum": ["hide", "show"]})
    }
}

/// The page the view fits on: its size, the way it lies and its picture resolution.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SheetParam {
    pub size: SheetSize,
    pub orientation: Orientation,
    pub dpi: f64,
}

impl Default for SheetParam {
    fn default() -> Self {
        SheetParam { size: SheetSize::Standard(PaperSize::A3), orientation: Orientation::Landscape, dpi: 300.0 }
    }
}

impl JsonSchema for SheetParam {
    fn schema_name() -> Cow<'static, str> {
        "SnapshotSheet".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let papers: Vec<Value> = PaperSize::ALL.iter().filter_map(|p| serde_json::to_value(p).ok()).collect();
        let sides = json!({"type": "array", "items": {"type": "number", "exclusiveMinimum": 0}, "minItems": 2, "maxItems": 2});
        let standard = json!({"type": "object", "properties": {"standard": {"type": "string", "enum": papers}}, "required": ["standard"], "additionalProperties": false});
        let custom = json!({
            "type": "object",
            "properties": {"custom": {"type": "object", "properties": {"sides_mm": sides}, "required": ["sides_mm"], "additionalProperties": false}},
            "required": ["custom"],
            "additionalProperties": false
        });
        json_schema!({
            "description": "The page the view fits and centres on (A3 landscape at 300 dpi when not given).",
            "type": "object",
            "properties": {
                "size": {"anyOf": [standard, custom]},
                "orientation": {"type": "string", "enum": ["landscape", "portrait"]},
                "dpi": {"type": "number", "minimum": *DPI_RANGE.start(), "maximum": *DPI_RANGE.end()}
            },
            "additionalProperties": false
        })
    }
}

/// A view's extents on its own plane, in model millimetres.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExtentsResult {
    pub min: [f64; 2],
    pub max: [f64; 2],
    /// How far the display mesh may lie inside a curved face: the largest display tolerance.
    pub error_mm: f64,
    /// `approximate`: measured on the display mesh.
    pub status: &'static str,
    pub provenance: &'static str,
}

/// A box on the sheet, in paper millimetres.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BoxResult {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// The scale the view was drawn at.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ScaleResult {
    pub text: String,
    pub factor: f64,
    /// Whether the scale was worked out to fit the sheet rather than given.
    pub fitted: bool,
}

/// The view's basis as resolved: its name when it has one, the line of sight and the paper axes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ViewResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub named: Option<String>,
    pub direction: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub origin: [f64; 3],
}

/// What the view was drawn from.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceResult {
    pub bodies: usize,
    /// Whether the timeline marker is at the end; otherwise the view shows the rolled-back model.
    pub timeline_at_end: bool,
}

/// What `drawing.view.snapshot` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SnapshotResult {
    pub view: ViewResult,
    pub scale: ScaleResult,
    pub position_mm: [f64; 2],
    /// The sheet the view is placed on, its defaults filled in.
    pub sheet: SheetParam,
    pub display_style: String,
    pub reference_style: String,
    pub extents: ExtentsResult,
    /// The view's box on the sheet: `position_mm` at its centre, the extents at the scale.
    pub boundary_mm: BoxResult,
    pub source: SourceResult,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "drawing_view_tests.rs"]
mod tests;
