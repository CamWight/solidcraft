use serde_json::{Value, json};

use super::{OccurrenceBomTypeParams, OccurrenceBomTypeResult};
use crate::Session;
use crate::schema::tests::{assert_keys, assert_strict_keeps_todays_errors, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<OccurrenceBomTypeParams>(), &["bom_type", "occurrence", "path"]);
}

/// A design with one placed component, `Part:1`.
fn placed(level: ParamCheck) -> (Session, u64) {
    let mut s = Session { param_check: level, ..Session::default() };
    s.execute("component.create", &json!({"name": "Part"})).unwrap_or_else(|e| panic!("{e}"));
    let id = s.doc.occurrences.first().map(|o| o.id).unwrap_or_default();
    (s, id)
}

#[test]
fn strict_keeps_todays_errors() {
    assert_strict_keeps_todays_errors(&[
        ("occurrence.bom_type", json!({"occurrence": true, "bom_type": "normal"})),
        ("occurrence.bom_type", json!({"occurrence": null, "bom_type": "normal"})),
        ("occurrence.bom_type", json!({"occurrence": [1], "bom_type": "normal"})),
    ]);
    // With an occurrence there, a malformed path or type is refused in the handler's words.
    let (_, id) = placed(ParamCheck::Off);
    for p in [
        json!({"occurrence": id, "path": [], "bom_type": "normal"}),
        json!({"occurrence": id, "path": "1", "bom_type": "normal"}),
        json!({"occurrence": id, "path": null, "bom_type": "normal"}),
        json!({"occurrence": id, "path": vec![id; 65], "bom_type": "normal"}),
        json!({"occurrence": id, "path": [-1], "bom_type": "normal"}),
        json!({"occurrence": id, "bom_type": "bought"}),
        json!({"occurrence": id, "bom_type": "Normal"}),
        json!({"occurrence": id, "bom_type": 3}),
    ] {
        let error = |level| placed(level).0.execute("occurrence.bom_type", &p).err().map(|e| e.to_string());
        let off = error(ParamCheck::Off);
        assert!(off.is_some(), "{p} should fail today");
        assert_eq!(error(ParamCheck::Strict), off, "{p}");
    }
}

#[test]
fn replies_match_the_output_schema() {
    let schema = output_schema_for::<OccurrenceBomTypeResult>();
    let schema = schema.as_value();
    let (mut s, id) = placed(ParamCheck::Warn);
    for p in [
        json!({"occurrence": id, "bom_type": "purchased"}),
        json!({"occurrence": "Part:1", "path": [id], "bom_type": "reference"}),
        json!({"occurrence": id.to_string(), "bom_type": "same_as_component"}),
        json!({"occurrence": id, "bom_type": null}),
    ] {
        let r = s.execute("occurrence.bom_type", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
        assert!(r.get("param_warnings").is_none(), "{p}: {r}");
        assert!(conforms(schema, schema, &r), "{r} doesn't match {schema}");
    }
    let r = s.execute("occurrence.bom_type", &json!({"occurrence": id, "bom_type": "phantom", "type": "x"})).unwrap_or_else(|e| panic!("{e}"));
    assert!(r.get("param_warnings").is_some(), "{r}");
    assert!(conforms(schema, schema, &r), "{r} doesn't match {schema}");
    let ok = json!({"occurrence": id, "bom_type": "phantom", "effective": "phantom", "changed": false});
    assert!(conforms(schema, schema, &ok));
    for (k, v) in [("occurrence", json!("1")), ("changed", json!(1)), ("effective", Value::Null)] {
        let mut bad = ok.clone();
        bad[k] = v;
        assert!(!conforms(schema, schema, &bad), "{bad}");
    }
}
