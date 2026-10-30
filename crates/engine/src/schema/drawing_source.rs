//! Contracts and result types for the commands in `cmd/drawing_source.rs` (#3890).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What `drawing.source.execute` takes.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct ExecuteParams {
    /// The source's id, as `drawing.inspect` lists it. Left out, the design the drawing is in.
    #[serde(default, deserialize_with = "super::present", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub source: Option<String>,
    /// A model command's id.
    pub command: String,
    /// What it takes.
    #[serde(default)]
    pub params: Value,
}

/// What `drawing.source.execute` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecuteResult {
    pub source: String,
    pub command: String,
    /// What the command answered.
    pub result: Value,
}

/// What `drawing.source.add` takes.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct AddParams {
    /// A saved .sc design's path. It is read before anything changes.
    pub path: String,
}

/// What `drawing.source.relink` and `drawing.source.locate` take.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct RepointParams {
    /// The source's id, as `drawing.sources` lists it.
    pub source: String,
    /// A saved .sc design's path. It is read before anything changes.
    pub path: String,
}

/// What `drawing.source.remove` takes.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct RemoveParams {
    /// The source's id, as `drawing.sources` lists it.
    pub source: String,
}

/// A source and where it stands.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceResult {
    pub source: String,
    /// The path the drawing keeps: none for the design the drawing is in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file it is read from now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// `up_to_date`, `pending`, `missing`, `unreadable`, `not_allowed` or `newer`.
    pub state: String,
    /// Why it isn't open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What the change wants checked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `drawing.sources` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SourcesResult {
    pub sources: Vec<SourceResult>,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

/// What `drawing.source.remove` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RemoveResult {
    pub removed: String,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}
