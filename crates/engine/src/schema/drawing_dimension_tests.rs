use super::{AddParams, DimensionSet, EditParams, InspectParams};
use crate::schema::input_schema_for;
use crate::schema::tests::assert_keys;

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<AddParams>(), &["decimals", "kind", "offset_mm", "override_text", "text", "view"]);
    assert_keys(input_schema_for::<InspectParams>(), &["dimension"]);
    assert_keys(input_schema_for::<EditParams>(), &["dimension", "set"]);
    assert_keys(
        input_schema_for::<DimensionSet>(),
        &["decimals", "diameter", "direction", "offset_mm", "other_side", "override_text", "reading", "text"],
    );
}
