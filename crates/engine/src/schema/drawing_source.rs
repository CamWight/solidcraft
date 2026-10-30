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
