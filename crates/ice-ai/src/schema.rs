//! Mirrors `packages/ai/src/utils/validation.ts` — JSON-Schema validation of tool
//! arguments plus a hand-written `coerce_with_json_schema` layer that tolerates
//! LLM-sloppy JSON (stringified numbers, `"true"`, null on optional fields, …).
//!
//! The TS source uses TypeBox's `Compile`/`Check`/`Value.Convert`; the Rust port
//! replaces TypeBox with the `jsonschema` crate (`JSONSchema::compile` +
//! `is_valid` + `validate`) and native compatibility coercion rules. The
//! public entry point is [`validate_tool_arguments`], which mirrors TS
//! `validateToolArguments`: coerce → validate,
//! returning the coerced value or an `AiError::Schema`.

use crate::error::AiError;
use crate::types::{Tool, ToolCall};
use jsonschema::paths::PathChunk;
use jsonschema::JSONSchema;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

// ----------------------------------------------------------------------------
// Schema-view helpers — a typed view over the opaque `serde_json::Value` schema
// ----------------------------------------------------------------------------

fn schema_types(schema: &Value) -> Vec<String> {
    let Some(obj) = schema.as_object() else {
        return Vec::new();
    };
    match obj.get("type") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

fn matches_json_type(value: &Value, ty: &str) -> bool {
    match ty {
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

// ----------------------------------------------------------------------------
// Validator cache — compiled schemas are keyed by serialized content and kept
// in a bounded FIFO so dynamically changing tool definitions cannot grow this
// process-global cache without limit.
// ----------------------------------------------------------------------------

const VALIDATOR_CACHE_CAPACITY: usize = 256;

#[derive(Default)]
struct ValidatorCache {
    validators: HashMap<String, Arc<JSONSchema>>,
    insertion_order: VecDeque<String>,
}

static VALIDATOR_CACHE: Mutex<Option<ValidatorCache>> = Mutex::new(None);

/// Compile-or-fetch a validator for a schema, preserving compilation failures
/// so the execution boundary can reject invalid schemas instead of treating
/// them as permissive.
fn get_sub_schema_validator(schema: &Value) -> Result<Arc<JSONSchema>, String> {
    let key = schema.to_string();
    {
        let guard = VALIDATOR_CACHE.lock().unwrap();
        if let Some(validator) = guard.as_ref().and_then(|cache| cache.validators.get(&key)) {
            return Ok(validator.clone());
        }
    }

    // Compile without holding the process-wide cache lock so independent
    // schemas and cache hits are not blocked by a costly compilation.
    // jsonschema's default resolver handles `$ref` inside self-contained
    // schemas. Compilation errors are retained for the caller; only valid
    // validators are cached.
    let validator = Arc::new(JSONSchema::compile(schema).map_err(|error| error.to_string())?);

    let mut guard = VALIDATOR_CACHE.lock().unwrap();
    let cache = guard.get_or_insert_with(ValidatorCache::default);
    // Another thread may have compiled and inserted this schema while this
    // thread was compiling. Reuse its entry to keep the FIFO key unique.
    if let Some(existing) = cache.validators.get(&key) {
        return Ok(existing.clone());
    }
    if cache.validators.len() >= VALIDATOR_CACHE_CAPACITY {
        if let Some(expired) = cache.insertion_order.pop_front() {
            cache.validators.remove(&expired);
        }
    }
    cache.insertion_order.push_back(key.clone());
    cache.validators.insert(key, validator.clone());
    Ok(validator)
}

// ----------------------------------------------------------------------------
// coercePrimitiveByType — faithful port of the TS table
// ----------------------------------------------------------------------------

/// Convert a finite floating-point value to JSON without saturating it to an
/// integer at Rust's numeric bounds. Preserve ordinary integral values as
/// integer JSON numbers where they fit exactly.
fn json_number_from_f64(value: f64) -> Option<Value> {
    if value.fract() == 0.0 {
        // The upper bounds are exclusive because `i64::MAX as f64` and
        // `u64::MAX as f64` round up to 2^63 and 2^64 respectively.
        if value >= i64::MIN as f64 && value < 9_223_372_036_854_775_808.0 {
            return Some(Value::from(value as i64));
        }
        if value >= 0.0 && value < 18_446_744_073_709_551_616.0 {
            return Some(Value::from(value as u64));
        }
    }
    serde_json::Number::from_f64(value).map(Value::Number)
}

/// Convert integer strings without lossy or saturating casts. Exact signed and
/// unsigned integer text is handled directly. Decimal/exponent forms are
/// parsed as exact decimal values and accepted only within JavaScript's
/// safe-integer range.
fn coerce_integer_string(value: &str) -> Option<Value> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = value.parse::<i64>() {
        return Some(Value::from(parsed));
    }
    if let Ok(parsed) = value.parse::<u64>() {
        return Some(Value::from(parsed));
    }
    parse_safe_integer_decimal(value).map(Value::from)
}

fn parse_safe_integer_decimal(value: &str) -> Option<i64> {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

    let (negative, unsigned) = if let Some(rest) = value.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = value.strip_prefix('+') {
        (false, rest)
    } else {
        (false, value)
    };
    let (mantissa, exponent) = match unsigned.find(|character| character == 'e' || character == 'E')
    {
        Some(index) => {
            let (mantissa, suffix) = unsigned.split_at(index);
            if suffix[1..].contains('e') || suffix[1..].contains('E') {
                return None;
            }
            (mantissa, suffix[1..].parse::<i64>().ok()?)
        }
        None => (unsigned, 0),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    if (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let digits = format!("{whole}{fraction}");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some(0);
    }

    let decimal_position = i64::try_from(whole.len()).ok()?.checked_add(exponent)?;
    if decimal_position <= 0 {
        return None;
    }

    let integer_digits = if decimal_position < i64::try_from(digits.len()).ok()? {
        let position = usize::try_from(decimal_position).ok()?;
        if digits[position..].bytes().any(|byte| byte != b'0') {
            return None;
        }
        digits[..position].to_string()
    } else {
        let zero_count =
            usize::try_from(decimal_position - i64::try_from(digits.len()).ok()?).ok()?;
        if digits.len().saturating_add(zero_count) > 16 {
            return None;
        }
        format!("{digits}{}", "0".repeat(zero_count))
    };
    let normalized = integer_digits.trim_start_matches('0');
    if normalized.is_empty() {
        return Some(0);
    }
    if normalized.len() > 16 {
        return None;
    }
    let magnitude = normalized.parse::<u64>().ok()?;
    if magnitude > MAX_SAFE_INTEGER {
        return None;
    }
    let value = i64::try_from(magnitude).ok()?;
    Some(if negative { -value } else { value })
}

/// Coerce a primitive value to match a JSON-Schema primitive type. Mirrors TS
/// `coercePrimitiveByType`. Returns the coerced value (possibly == input when
/// no coercion applies).
fn coerce_primitive_by_type(value: &Value, ty: &str) -> Value {
    match ty {
        "number" => match value {
            Value::Null => Value::from(0),
            Value::String(s) if !s.trim().is_empty() => {
                if let Ok(parsed) = s.trim().parse::<f64>() {
                    if parsed.is_finite() {
                        return json_number_from_f64(parsed).unwrap_or_else(|| value.clone());
                    }
                }
                value.clone()
            }
            Value::Bool(b) => Value::from(if *b { 1 } else { 0 }),
            _ => value.clone(),
        },
        "integer" => match value {
            Value::Null => Value::from(0),
            Value::String(s) => coerce_integer_string(s).unwrap_or_else(|| value.clone()),
            Value::Bool(b) => Value::from(if *b { 1 } else { 0 }),
            _ => value.clone(),
        },
        "boolean" => match value {
            Value::Null => Value::Bool(false),
            Value::String(s) => match s.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => value.clone(),
            },
            Value::Number(n) => {
                if n.as_f64() == Some(1.0) {
                    Value::Bool(true)
                } else if n.as_f64() == Some(0.0) {
                    Value::Bool(false)
                } else {
                    value.clone()
                }
            }
            _ => value.clone(),
        },
        "string" => match value {
            Value::Null => Value::String(String::new()),
            Value::Number(n) => Value::String(n.to_string()),
            Value::Bool(b) => Value::String(b.to_string()),
            _ => value.clone(),
        },
        "null" => match value {
            Value::String(s) if s.is_empty() => Value::Null,
            Value::Number(n) if n.as_f64() == Some(0.0) => Value::Null,
            Value::Bool(false) => Value::Null,
            _ => value.clone(),
        },
        _ => value.clone(),
    }
}

// ----------------------------------------------------------------------------
// applySchemaObjectCoercion / applySchemaArrayCoercion
// ----------------------------------------------------------------------------

fn apply_schema_object_coercion(value: &mut Map<String, Value>, schema: &Value) {
    let Some(sobj) = schema.as_object() else {
        return;
    };
    let properties = sobj.get("properties").and_then(|p| p.as_object());
    let defined_keys: HashSet<&str> = properties
        .map(|p| p.keys().map(|k| k.as_str()).collect())
        .unwrap_or_default();

    if let Some(props) = properties {
        for (key, prop_schema) in props {
            if let Some(child) = value.get_mut(key) {
                let coerced = coerce_with_json_schema(child.clone(), prop_schema);
                *child = coerced;
            }
        }
    }

    if let Some(addl) = sobj.get("additionalProperties") {
        if let Some(addl_obj) = addl.as_object() {
            let addl_value = Value::Object(addl_obj.clone());
            for (key, child) in value.iter_mut() {
                if defined_keys.contains(key.as_str()) {
                    continue;
                }
                let coerced = coerce_with_json_schema(child.clone(), &addl_value);
                *child = coerced;
            }
        }
    }
}

fn apply_schema_array_coercion(value: &mut Vec<Value>, schema: &Value) {
    let Some(sobj) = schema.as_object() else {
        return;
    };
    match sobj.get("items") {
        Some(Value::Array(items)) => {
            for (i, item_schema) in items.iter().enumerate() {
                if i >= value.len() {
                    continue;
                }
                let coerced = coerce_with_json_schema(value[i].clone(), item_schema);
                value[i] = coerced;
            }
        }
        Some(items) if items.is_object() => {
            for entry in value.iter_mut() {
                let coerced = coerce_with_json_schema(entry.clone(), items);
                *entry = coerced;
            }
        }
        _ => {}
    }
}

// ----------------------------------------------------------------------------
// coerceWithUnionSchema
// ----------------------------------------------------------------------------

fn coerce_with_union_schema(value: &Value, schemas: &[Value]) -> Value {
    // Pass 1: if any member already validates, keep as-is.
    for schema in schemas {
        if let Ok(v) = get_sub_schema_validator(schema) {
            if v.is_valid(value) {
                return value.clone();
            }
        }
    }
    // Pass 2: clone + coerce against each member; return first that validates.
    for schema in schemas {
        let candidate = coerce_with_json_schema(value.clone(), schema);
        if let Ok(v) = get_sub_schema_validator(schema) {
            if v.is_valid(&candidate) {
                return candidate;
            }
        }
    }
    value.clone()
}

// ----------------------------------------------------------------------------
// coerceWithJsonSchema — the recursive entry point
// ----------------------------------------------------------------------------

/// Recursively coerce a value to conform to a JSON-Schema. Mirrors TS
/// `coerceWithJsonSchema`. Handles `allOf`/`anyOf`/`oneOf`, primitive-type
/// coercion, object-property recursion, and array-item recursion.
pub fn coerce_with_json_schema(value: Value, schema: &Value) -> Value {
    let mut next = value;

    if let Some(Value::Array(all_of)) = schema.as_object().and_then(|o| o.get("allOf")) {
        for nested in all_of {
            next = coerce_with_json_schema(next, nested);
        }
    }

    if let Some(Value::Array(any_of)) = schema.as_object().and_then(|o| o.get("anyOf")) {
        next = coerce_with_union_schema(&next, any_of);
    }

    if let Some(Value::Array(one_of)) = schema.as_object().and_then(|o| o.get("oneOf")) {
        next = coerce_with_union_schema(&next, one_of);
    }

    let types = schema_types(schema);
    let matches_union_member = types.len() > 1 && types.iter().any(|t| matches_json_type(&next, t));
    if !types.is_empty() && !matches_union_member {
        for ty in &types {
            let candidate = coerce_primitive_by_type(&next, ty);
            if candidate != next {
                next = candidate;
                break;
            }
        }
    }

    if types.iter().any(|t| t == "object") {
        if let Value::Object(ref mut map) = next {
            apply_schema_object_coercion(map, schema);
        }
    }

    if types.iter().any(|t| t == "array") {
        if let Value::Array(ref mut arr) = next {
            apply_schema_array_coercion(arr, schema);
        }
    }

    next
}

// Optional nulls are deliberately not removed before coercion. The shared
// coercion pass handles them according to their property schemas, matching the
// current TypeScript validator's plain-JSON-schema path.

// ----------------------------------------------------------------------------
// validateToolArguments — public entry point
// ----------------------------------------------------------------------------

/// Validate (and coerce) tool-call arguments against the tool's parameter
/// schema. Mirrors TS `validateToolArguments`:
///
/// 1. `coerce_with_json_schema` — LLM-sloppy-JSON tolerance.
/// 2. `JSONSchema::is_valid` — authoritative check.
///
/// Returns the coerced arguments on success, or `AiError::Schema` with a
/// formatted message (path + message per error + the received JSON) on failure.
pub fn validate_tool_arguments(tool: &Tool, tool_call: &ToolCall) -> Result<Value, AiError> {
    let args = tool_call.arguments.clone();

    // Coerce. TS only skips the custom coerce pass when the schema
    // carries the TypeBox Kind symbol; schemars produces plain JSON Schemas, so
    // the Rust port always runs it.
    let coerced = coerce_with_json_schema(args.clone(), tool.parameters.as_value());

    // Authoritative validation.
    let validator = get_sub_schema_validator(tool.parameters.as_value()).map_err(|diagnostic| {
        AiError::Schema {
            message: format!(
                "Could not compile schema for tool {:?}: {diagnostic}\n\nTool schema:\n{}\n\nReceived arguments:\n{}",
                tool_call.name,
                serde_json::to_string_pretty(tool.parameters.as_value())
                    .unwrap_or_else(|_| "<unprintable>".into()),
                serde_json::to_string_pretty(&tool_call.arguments)
                    .unwrap_or_else(|_| "<unprintable>".into())
            ),
        }
    })?;

    let check_value = if coerced != args { &coerced } else { &args };
    if validator.is_valid(check_value) {
        return Ok(check_value.clone());
    }

    let errors: Vec<String> = collect_validation_errors(&validator, check_value);
    let errors_block = if errors.is_empty() {
        "Unknown validation error".to_string()
    } else {
        errors
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    Err(AiError::Schema {
        message: format!(
            "Validation failed for tool {:?}:\n{errors_block}\n\nReceived arguments:\n{}",
            tool_call.name,
            serde_json::to_string_pretty(&tool_call.arguments)
                .unwrap_or_else(|_| "<unprintable>".into())
        ),
    })
}

/// Find a tool by name in a slice and validate its call. Mirrors TS
/// `validateToolCall`. Returns `Err(AiError::Schema)` if the tool isn't found.
pub fn validate_tool_call(tools: &[Tool], tool_call: &ToolCall) -> Result<Value, AiError> {
    let tool = tools
        .iter()
        .find(|t| t.name == tool_call.name)
        .ok_or_else(|| AiError::Schema {
            message: format!("Tool {:?} not found", tool_call.name),
        })?;
    validate_tool_arguments(tool, tool_call)
}

/// Collect human-readable validation errors with JSON-pointer paths, mirroring
/// the TS `validator.Errors(...).map(formatValidationPath)` output. Uses
/// `JSONSchema::validate`, whose `Err` variant is an iterator over all errors.
fn collect_validation_errors(validator: &JSONSchema, value: &Value) -> Vec<String> {
    let Err(errors) = validator.validate(value) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for error in errors {
        // Build a dotted path from the JSONPointer chunks (Property/Index),
        // matching the TS `formatValidationPath` output; root → "root".
        let dotted: String = error
            .instance_path
            .iter()
            .map(|chunk| match chunk {
                PathChunk::Property(p) => p.as_ref().to_string(),
                PathChunk::Index(i) => i.to_string(),
                PathChunk::Keyword(k) => k.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".");
        let label = if dotted.is_empty() {
            "root".to_string()
        } else {
            dotted
        };
        out.push(format!("{label}: {error}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Tool, ToolCall, ToolCallType};
    use serde_json::json;

    fn tool(name: &str, schema: Value) -> Tool {
        Tool {
            name: name.to_string(),
            description: String::new(),
            parameters: crate::types::Schema::new(schema),
            constrained_sampling: None,
        }
    }

    fn call(args: Value) -> ToolCall {
        ToolCall {
            kind: ToolCallType,
            id: "id".to_string(),
            name: "t".to_string(),
            arguments: args,
            thought_signature: None,
            namespace: None,
        }
    }

    #[test]
    fn coerce_string_number_to_number() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
        );
        let tc = call(json!({"n":"42"}));
        let v = validate_tool_arguments(&t, &tc).unwrap();
        assert_eq!(v["n"], json!(42));
    }

    #[test]
    fn coerce_string_integer_to_integer() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
        );
        let tc = call(json!({"n":"7"}));
        let v = validate_tool_arguments(&t, &tc).unwrap();
        assert_eq!(v["n"], json!(7));
    }

    #[test]
    fn coerce_string_bool_to_boolean() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"b":{"type":"boolean"}},"required":["b"]}),
        );
        let tc = call(json!({"b":"true"}));
        let v = validate_tool_arguments(&t, &tc).unwrap();
        assert_eq!(v["b"], json!(true));
    }

    #[test]
    fn coerce_number_to_string() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"s":{"type":"string"}},"required":["s"]}),
        );
        let tc = call(json!({"s":42}));
        let v = validate_tool_arguments(&t, &tc).unwrap();
        assert_eq!(v["s"], json!("42"));
    }

    #[test]
    fn optional_null_is_coerced_not_dropped() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"opt":{"type":"string"}},"required":[]}),
        );
        let tc = call(json!({"opt":null}));
        let v = validate_tool_arguments(&t, &tc).unwrap();
        assert_eq!(v["opt"], json!(""));
    }

    #[test]
    fn union_keeps_valid_member() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"x":{"oneOf":[{"type":"string"},{"type":"number"}]}},"required":["x"]}),
        );
        let tc = call(json!({"x":5}));
        let v = validate_tool_arguments(&t, &tc).unwrap();
        assert_eq!(v["x"], json!(5));
    }

    #[test]
    fn union_coerces_to_member() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"x":{"oneOf":[{"type":"boolean"},{"type":"string"}]}},"required":["x"]}),
        );
        let tc = call(json!({"x":1}));
        let v = validate_tool_arguments(&t, &tc).unwrap();
        assert_eq!(v["x"], json!(true));
    }

    #[test]
    fn rejects_uncoercible() {
        let t = tool(
            "t",
            json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
        );
        let tc = call(json!({"n":"not-a-number"}));
        assert!(validate_tool_arguments(&t, &tc).is_err());
    }

    #[test]
    fn validate_tool_call_finds_named_tool() {
        let tools = vec![
            tool("other", json!({"type":"object"})),
            tool(
                "t",
                json!({"type":"object","properties":{"n":{"type":"number"}},"required":["n"]}),
            ),
        ];
        let tc = ToolCall {
            kind: ToolCallType,
            id: "id".into(),
            name: "t".into(),
            arguments: json!({"n":"9"}),
            thought_signature: None,
            namespace: None,
        };
        let v = validate_tool_call(&tools, &tc).unwrap();
        assert_eq!(v["n"], json!(9));
    }

    #[test]
    fn validate_tool_call_missing_tool_errors() {
        let tools = vec![tool("other", json!({"type":"object"}))];
        let tc = ToolCall {
            kind: ToolCallType,
            id: "id".into(),
            name: "absent".into(),
            arguments: json!({}),
            thought_signature: None,
            namespace: None,
        };
        let err = validate_tool_call(&tools, &tc).unwrap_err();
        assert!(matches!(err, AiError::Schema { .. }));
    }

    #[test]
    fn concurrent_validator_compilation_keeps_cache_entries_unique() {
        const THREADS: usize = 8;
        let schema = json!({"type":"object","title":"concurrent-cache-test"});
        let barrier = Arc::new(std::sync::Barrier::new(THREADS));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let schema = schema.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let validator = get_sub_schema_validator(&schema).unwrap();
                    assert!(validator.is_valid(&json!({})));
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("validator thread should succeed");
        }

        let guard = VALIDATOR_CACHE.lock().unwrap();
        let cache = guard
            .as_ref()
            .expect("validator cache should be initialized");
        let unique_keys: HashSet<&String> = cache.insertion_order.iter().collect();
        assert_eq!(
            unique_keys.len(),
            cache.insertion_order.len(),
            "concurrent insertion must not duplicate FIFO cache keys"
        );
        assert_eq!(cache.validators.len(), cache.insertion_order.len());
        assert!(cache.validators.len() <= VALIDATOR_CACHE_CAPACITY);
    }
}
