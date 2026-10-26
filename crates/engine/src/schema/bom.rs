//! Contracts and result types for the commands in `cmd/bom.rs`.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use solidcraft_doc::BomType;

use super::check::whole;
use super::types::{IdOrName, present};

/// Let one occurrence count as another bill of materials type in every bill of materials, in
/// place of its component's, or as its component again. An occurrence inside a subassembly
/// belongs to its definition, so the type holds wherever the subassembly is placed. One undo step.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct OccurrenceBomTypeParams {
    /// The occurrence: its id, or its name.
    pub occurrence: IdOrName,
    /// Its placement, checked when given: the occurrence ids from the design down to it.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "OccurrencePath")]
    pub path: Option<OccurrencePath>,
    /// The type it counts as, or `same_as_component` (or null) for its component's.
    #[serde(deserialize_with = "bom_type")]
    #[schemars(schema_with = "bom_type_schema")]
    pub bom_type: Option<BomType>,
}

/// An occurrence path, read by the handlers' own reader.
#[derive(Debug)]
pub struct OccurrencePath(pub Vec<u64>);

impl<'de> Deserialize<'de> for OccurrencePath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::cmd::path_ids(&Value::deserialize(d)?).map(OccurrencePath).map_err(|e| D::Error::custom(whole(e)))
    }
}

impl JsonSchema for OccurrencePath {
    fn schema_name() -> Cow<'static, str> {
        "OccurrencePath".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Occurrence ids from the design down to the placement.",
            "type": "array",
            "items": {"type": "integer", "minimum": 0},
            "minItems": 1,
            "maxItems": solidcraft_doc::MAX_PLACEMENT_DEPTH
        })
    }
}

/// `bom_type`, read by the handler's own reader.
fn bom_type<'de, D: Deserializer<'de>>(d: D) -> Result<Option<BomType>, D::Error> {
    crate::cmd::occurrence_bom_type(Some(&Value::deserialize(d)?)).map_err(|e| D::Error::custom(whole(e)))
}

/// The five types' words, `same_as_component`, or null.
fn bom_type_schema(_: &mut SchemaGenerator) -> Schema {
    let mut words: Vec<Value> = BomType::ALL.iter().map(|t| Value::from(t.as_str())).collect();
    words.push(Value::from(crate::cmd::SAME_AS_COMPONENT));
    json_schema!({
        "description": "normal, phantom, reference, purchased or inseparable; same_as_component or null: the component's.",
        "anyOf": [{"type": "string", "enum": words}, {"type": "null"}]
    })
}

/// What `occurrence.bom_type` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OccurrenceBomTypeResult {
    /// The occurrence's id.
    pub occurrence: u64,
    /// Its own type now, or `same_as_component`.
    pub bom_type: String,
    /// The type it counts as: its own, else its component's.
    pub effective: String,
    /// Whether this call changed it (an undo step).
    pub changed: bool,
    /// In the warn parameter-check mode: what the call's parameters got wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_warnings: Vec<String>,
}

#[cfg(test)]
#[path = "bom_tests.rs"]
mod tests;
