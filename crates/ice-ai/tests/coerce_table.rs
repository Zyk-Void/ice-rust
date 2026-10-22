//! Integration tests for `coerce_with_json_schema` + `validate_tool_arguments`.
//! Mirrors the table-driven cases in `validation.ts`'s test block. Lives in
//! `tests/` so it exercises the crate's public API exactly the way `pi-agent`
//! and `pi-tools` will.

use ice_ai::schema::{coerce_with_json_schema, validate_tool_arguments, validate_tool_call};
use ice_ai::{AiError, Schema, Tool, ToolCall, ToolCallType};
use serde_json::{json, Value};

fn tool(name: &str, schema: Value) -> Tool {
    Tool {
        name: name.to_string(),
        description: String::new(),
        parameters: Schema::new(schema),
        constrained_sampling: None,
    }
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        kind: ToolCallType,
        id: "id".to_string(),
        name: name.to_string(),
        arguments: args,
        thought_signature: None,
        namespace: None,
    }
}

// --- primitive coercions -----------------------------------------------------

#[test]
fn string_to_number() {
    let v = coerce_with_json_schema(json!("42"), &json!({"type":"number"}));
    assert_eq!(v, json!(42));
}

#[test]
fn string_to_integer() {
    let v = coerce_with_json_schema(json!("7"), &json!({"type":"integer"}));
    assert_eq!(v, json!(7));
}

#[test]
fn string_true_to_boolean() {
    let v = coerce_with_json_schema(json!("true"), &json!({"type":"boolean"}));
    assert_eq!(v, json!(true));
}

#[test]
fn number_one_to_boolean() {
    let v = coerce_with_json_schema(json!(1), &json!({"type":"boolean"}));
    assert_eq!(v, json!(true));
}

#[test]
fn number_to_string() {
    let v = coerce_with_json_schema(json!(42), &json!({"type":"string"}));
    assert_eq!(v, json!("42"));
}

#[test]
fn bool_to_string() {
    let v = coerce_with_json_schema(json!(true), &json!({"type":"string"}));
    assert_eq!(v, json!("true"));
}

#[test]
fn null_to_number_is_zero() {
    let v = coerce_with_json_schema(json!(null), &json!({"type":"number"}));
    assert_eq!(v, json!(0));
}

#[test]
fn null_to_boolean_is_false() {
    let v = coerce_with_json_schema(json!(null), &json!({"type":"boolean"}));
    assert_eq!(v, json!(false));
}

#[test]
fn null_to_string_is_empty() {
    let v = coerce_with_json_schema(json!(null), &json!({"type":"string"}));
    assert_eq!(v, json!(""));
}

#[test]
fn empty_string_to_null() {
    let v = coerce_with_json_schema(json!(""), &json!({"type":"null"}));
    assert_eq!(v, json!(null));
}

// --- object / array recursion ------------------------------------------------

#[test]
fn object_property_coercion() {
    let v = coerce_with_json_schema(
        json!({"n":"42","b":"true"}),
        &json!({"type":"object","properties":{"n":{"type":"number"},"b":{"type":"boolean"}}}),
    );
    assert_eq!(v, json!({"n":42,"b":true}));
}

#[test]
fn array_item_coercion() {
    let v = coerce_with_json_schema(
        json!(["1", "2", "3"]),
        &json!({"type":"array","items":{"type":"integer"}}),
    );
    assert_eq!(v, json!([1, 2, 3]));
}

#[test]
fn nested_object_array() {
    let v = coerce_with_json_schema(
        json!({"items":[{"n":"1"},{"n":"2"}]}),
        &json!({"type":"object","properties":{"items":{"type":"array","items":{"type":"object","properties":{"n":{"type":"number"}}}}}}),
    );
    assert_eq!(v, json!({"items":[{"n":1},{"n":2}]}));
}

// --- optional-null coercion (via the public entry point) ----------------------

#[test]
fn optional_and_required_nulls_are_coerced_without_deletion() {
    // Optional and required string properties both receive null.
    let t = tool(
        "t",
        json!({"type":"object","properties":{"opt":{"type":"string"},"req":{"type":"string"}},"required":["req"]}),
    );
    let tc = call("t", json!({"opt":null,"req":null}));
    let v = validate_tool_arguments(&t, &tc).unwrap();
    // Both properties remain present and use the documented null-to-string
    // coercion; optional nulls are not deleted before coercion.
    assert_eq!(v["opt"], json!(""));
    assert_eq!(v["req"], json!(""));
}

// --- unions ------------------------------------------------------------------

#[test]
fn union_keeps_already_valid_member() {
    let v = coerce_with_json_schema(
        json!(5),
        &json!({"oneOf":[{"type":"string"},{"type":"number"}]}),
    );
    assert_eq!(v, json!(5));
}

#[test]
fn union_coerces_to_validating_member() {
    let v = coerce_with_json_schema(
        json!(1),
        &json!({"oneOf":[{"type":"boolean"},{"type":"string"}]}),
    );
    assert_eq!(v, json!(true));
}

// --- validation failures -----------------------------------------------------

#[test]
fn uncoercible_value_rejected() {
    let t = tool(
        "t",
        json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
    );
    let tc = call("t", json!({"n":"not-a-number"}));
    let err = validate_tool_arguments(&t, &tc).unwrap_err();
    let AiError::Schema { message } = err else {
        panic!("expected a schema error, got {err}");
    };
    assert!(message.contains("Validation failed for tool \"t\""));
    assert!(message.contains("n:"), "property path missing: {message}");
    assert!(
        message.contains("not-a-number"),
        "received arguments missing: {message}"
    );
    assert!(message.contains("Received arguments:"));
}

#[test]
fn integer_coercion_preserves_exact_u64_values() {
    let t = tool(
        "t",
        json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
    );
    let tc = call("t", json!({"n":"9223372036854775808"}));

    let coerced = validate_tool_arguments(&t, &tc).unwrap();
    assert_eq!(coerced["n"], json!(9_223_372_036_854_775_808_u64));
}

#[test]
fn integer_exponent_coercion_keeps_exact_values() {
    let coerced = coerce_with_json_schema(json!("1e3"), &json!({"type":"integer"}));
    assert_eq!(coerced, json!(1000));
}

#[test]
fn integer_coercion_rejects_values_outside_u64_range() {
    let t = tool(
        "t",
        json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
    );
    let tc = call("t", json!({"n":"18446744073709551616"}));

    assert!(validate_tool_arguments(&t, &tc).is_err());
}

#[test]
fn integer_coercion_rejects_lossy_decimal_strings() {
    let t = tool(
        "t",
        json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
    );
    let tc = call("t", json!({"n":"1.0000000000000001"}));

    assert!(validate_tool_arguments(&t, &tc).is_err());
}

#[test]
fn large_number_coercion_does_not_saturate_to_i64() {
    let t = tool(
        "t",
        json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
    );
    let tc = call("t", json!({"n":"1e100"}));

    let coerced = validate_tool_arguments(&t, &tc).unwrap();
    assert_eq!(coerced["n"], json!(1e100));
}

#[test]
fn missing_required_rejected() {
    let t = tool(
        "t",
        json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
    );
    let tc = call("t", json!({}));
    assert!(validate_tool_arguments(&t, &tc).is_err());
}

#[test]
fn validate_tool_call_missing_tool_errors() {
    let tools = vec![tool("other", json!({"type":"object"}))];
    let tc = call("absent", json!({}));
    let err = validate_tool_call(&tools, &tc).unwrap_err();
    assert!(matches!(err, AiError::Schema { .. }));
}

#[test]
fn invalid_schema_fails_closed_with_tool_and_schema_diagnostics() {
    let schema = json!({"type":"not-a-json-schema-type"});
    let t = tool("broken_schema_tool", schema.clone());
    let tc = call("broken_schema_tool", json!({"payload":"must not pass"}));

    let err = validate_tool_arguments(&t, &tc).unwrap_err();
    let AiError::Schema { message } = err else {
        panic!("expected a schema error, got {err}");
    };

    assert!(message.contains("broken_schema_tool"));
    assert!(message.contains("not-a-json-schema-type"));
    assert!(message.contains("must not pass"));
}

#[test]
fn validate_tool_call_dispatches_to_named_tool() {
    let tools = vec![
        tool("other", json!({"type":"object"})),
        tool(
            "t",
            json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
        ),
    ];
    let tc = call("t", json!({"n":"9"}));
    let v = validate_tool_call(&tools, &tc).unwrap();
    assert_eq!(v["n"], json!(9));
}
