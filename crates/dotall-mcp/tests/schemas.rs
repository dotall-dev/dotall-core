use dotall_mcp::params::{
    ApplyParams, CapabilitiesParams, DepsParams, DiffParams, DiscardParams, EditParams, InitParams,
    ReadParams, RevertParams, StagedParams, StatusParams,
};
use dotall_mcp::response::{JsonResult, ToolResponse};

#[test]
fn edit_schema_requires_stale_write_guard_and_operations() {
    let schema = schemars::schema_for!(EditParams);
    let json = serde_json::to_value(schema).expect("schema JSON");
    let required = json["required"].as_array().expect("required");

    assert!(required.iter().any(|value| value == "expected_source_hash"));
    assert!(required.iter().any(|value| value == "operations"));
    assert!(required.iter().any(|value| value == "actor_id"));
}

#[test]
fn edit_schema_has_no_auto_apply_flag() {
    let json = serde_json::to_value(schemars::schema_for!(EditParams)).expect("schema JSON");
    let properties = json["properties"].as_object().expect("properties");

    assert!(!properties.contains_key("stage_only"));
    assert!(!properties.contains_key("auto_apply"));
}

#[test]
fn read_schema_exposes_budget_and_continuation() {
    let json = serde_json::to_value(schemars::schema_for!(ReadParams)).expect("schema JSON");
    let properties = json["properties"].as_object().expect("properties");

    assert!(properties.contains_key("max_tokens"));
    assert!(properties.contains_key("continuation"));
}

#[test]
fn capabilities_schema_requires_file() {
    let json =
        serde_json::to_value(schemars::schema_for!(CapabilitiesParams)).expect("schema JSON");
    let required = json["required"].as_array().expect("required");

    assert!(required.iter().any(|value| value == "file"));
}

#[test]
fn apply_schema_exposes_transaction_and_all_fields() {
    let json = serde_json::to_value(schemars::schema_for!(ApplyParams)).expect("schema JSON");
    let properties = json["properties"].as_object().expect("properties");

    assert!(properties.contains_key("transaction_id"));
    assert!(properties.contains_key("all"));
}

#[test]
fn tool_response_schema_is_tagged_envelope() {
    let json =
        serde_json::to_value(schemars::schema_for!(ToolResponse<JsonResult>)).expect("schema JSON");

    let variants = json
        .get("oneOf")
        .or_else(|| json.get("anyOf"))
        .and_then(|value| value.as_array())
        .expect("tagged ToolResponse schema should expose variant list");

    assert!(variants.len() >= 2);

    let serialized = serde_json::to_string(&json).expect("schema string");
    assert!(serialized.contains("success"));
    assert!(serialized.contains("error"));
    assert!(serialized.contains("next_actions"));
}

#[test]
fn edit_params_round_trip_without_stage_only() {
    let params = EditParams {
        file: "book.xlsx".into(),
        expected_source_hash: "hash".into(),
        transaction_id: Some("tx".into()),
        actor_id: "agent".into(),
        operations: vec![dotall_mcp::params::OperationParam {
            kind: "set_cell_formula".into(),
            payload: serde_json::json!({"sheet": "Sheet1", "address": "A1", "formula": "=1+1"}),
        }],
    };

    let json = serde_json::to_string(&params).expect("serialize");
    assert!(!json.contains("stage_only"));
    let restored: EditParams = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.file, "book.xlsx");
    assert_eq!(restored.operations.len(), 1);
}

#[test]
fn workspace_and_file_param_schemas_exist() {
    for schema in [
        schemars::schema_for!(InitParams),
        schemars::schema_for!(StatusParams),
        schemars::schema_for!(StagedParams),
        schemars::schema_for!(DiscardParams),
        schemars::schema_for!(DiffParams),
        schemars::schema_for!(RevertParams),
        schemars::schema_for!(DepsParams),
    ] {
        let json = serde_json::to_value(schema).expect("schema JSON");
        assert_eq!(json["type"], "object");
    }
}
