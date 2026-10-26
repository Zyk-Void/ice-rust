//! Closed MCP form subset: no references, nested objects, or remote resolution.
use super::*;
use jsonschema::JSONSchema;

pub(super) struct Schema {
    pub fields: Vec<Field>,
}
pub(super) struct Field {
    pub name: String,
    pub title: String,
    pub description: String,
    pub required: bool,
    pub default: Option<Value>,
    pub multiple: bool,
    /// Raw values remain separate from their sanitized display labels.
    pub options: Vec<(Value, String)>,
    kind: String,
    validator: JSONSchema,
}

fn keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("Expected a schema object")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported schema keyword: {key}"));
    }
    Ok(())
}
fn choices(schema: &Value) -> Result<Vec<(Value, String)>, String> {
    if ["enum", "oneOf", "anyOf"]
        .iter()
        .filter(|key| schema.get(**key).is_some())
        .count()
        > 1
        || schema.get("enumNames").is_some() && schema.get("enum").is_none()
    {
        return Err("Conflicting enum choices".into());
    }
    let options = if let Some(values) = schema.get("enum") {
        let values = values.as_array().ok_or("enum must be an array")?;
        if let Some(names) = schema.get("enumNames") {
            if names.as_array().is_none_or(|names| {
                names.len() != values.len() || names.iter().any(|v| !v.is_string())
            }) {
                return Err("enumNames must match enum".into());
            }
        }
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let text = value.as_str().ok_or("enum values must be strings")?;
                Ok((
                    value.clone(),
                    schema["enumNames"][index].as_str().unwrap_or(text).into(),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else if let Some(options) = schema.get("oneOf").or_else(|| schema.get("anyOf")) {
        options
            .as_array()
            .ok_or("choices must be an array")?
            .iter()
            .map(|option| {
                keys(option, &["const", "title"])?;
                let text = option["const"]
                    .as_str()
                    .ok_or("choice const must be a string")?;
                Ok((json!(text), option["title"].as_str().unwrap_or(text).into()))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        return Ok(Vec::new());
    };
    let unique: HashSet<_> = options.iter().map(|(value, _)| value.to_string()).collect();
    if options.is_empty() || options.len() > 256 || unique.len() != options.len() {
        return Err("Options must contain 1–256 distinct strings".into());
    }
    Ok(options)
}

impl Schema {
    pub fn parse(value: &Value) -> Result<Self, String> {
        keys(
            value,
            &[
                "type",
                "properties",
                "required",
                "additionalProperties",
                "title",
                "description",
                "$schema",
            ],
        )?;
        if value["type"] != "object" {
            return Err("requestedSchema.type must be object".into());
        }
        let properties = value["properties"]
            .as_object()
            .ok_or("Missing properties")?;
        if properties.len() > 64 {
            return Err("At most 64 fields are supported".into());
        }
        if value
            .get("additionalProperties")
            .is_some_and(|v| v != false)
        {
            return Err("additionalProperties must be false".into());
        }
        let required = match value.get("required") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or("required must be an array")?
                .iter()
                .map(|v| v.as_str().ok_or("required names must be strings"))
                .collect::<Result<Vec<_>, _>>()?,
        };
        if required.iter().any(|name| !properties.contains_key(*name))
            || required.iter().collect::<HashSet<_>>().len() != required.len()
        {
            return Err("required contains duplicate or undeclared fields".into());
        }
        let mut fields = Vec::new();
        for (name, schema) in properties {
            let kind = schema["type"].as_str().ok_or("Missing field type")?;
            let mut allowed = vec!["type", "title", "description", "default"];
            allowed.extend(match kind {
                "string" => vec![
                    "enum",
                    "enumNames",
                    "oneOf",
                    "minLength",
                    "maxLength",
                    "format",
                    "pattern",
                ],
                "number" | "integer" => vec!["minimum", "maximum"],
                "boolean" => vec![],
                "array" => vec!["items", "minItems", "maxItems", "uniqueItems"],
                _ => return Err(format!("Unsupported field type: {kind}")),
            });
            keys(schema, &allowed)?;
            for key in ["title", "description"] {
                if schema.get(key).is_some_and(|v| !v.is_string()) {
                    return Err(format!("{key} must be a string"));
                }
            }
            if schema.get("format").is_some_and(|v| {
                !matches!(v.as_str(), Some("email" | "uri" | "date" | "date-time"))
            }) {
                return Err("Unsupported string format".into());
            }
            let multiple = kind == "array";
            let options = if multiple {
                let items = &schema["items"];
                keys(
                    items,
                    &[
                        "type",
                        "enum",
                        "enumNames",
                        "oneOf",
                        "anyOf",
                        "title",
                        "description",
                    ],
                )?;
                if items.get("type").is_some_and(|v| v != "string") {
                    return Err("Array items must be strings".into());
                }
                let options = choices(items)?;
                if options.is_empty() {
                    return Err("Arrays require enum choices".into());
                }
                options
            } else if kind == "boolean" {
                vec![(json!(true), "Yes".into()), (json!(false), "No".into())]
            } else if kind == "string" {
                choices(schema)?
            } else {
                Vec::new()
            };
            // Every accepted keyword has local semantics. No $ref or identifiers can
            // reach this compiler, so it cannot resolve a URL or read a local file.
            let mut normalized = schema.clone();
            if multiple {
                normalized["items"]["type"] = json!("string");
            }
            let validator = JSONSchema::options()
                .should_validate_formats(true)
                .compile(&normalized)
                .map_err(|e| e.to_string())?;
            let field = Field {
                name: name.clone(),
                title: schema["title"].as_str().unwrap_or(name).into(),
                description: schema["description"].as_str().unwrap_or("").into(),
                required: required.contains(&name.as_str()),
                default: schema.get("default").cloned(),
                multiple,
                options,
                kind: kind.into(),
                validator,
            };
            if let Some(default) = &field.default {
                if !field.validator.is_valid(default) {
                    return Err(format!("Invalid default for {name}"));
                }
            }
            fields.push(field);
        }
        Ok(Self { fields })
    }
}
impl Field {
    pub fn validate(&self, value: Value) -> Result<Value, String> {
        let value = match (self.kind.as_str(), value) {
            ("number" | "integer", Value::String(text)) => {
                serde_json::from_str::<Value>(text.trim())
                    .ok()
                    .filter(Value::is_number)
                    .ok_or("Enter a finite JSON number")?
            }
            (_, value) => value,
        };
        self.validator.validate(&value).map_err(|mut errors| {
            errors
                .next()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "Invalid value".into())
        })?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coercion_and_constraints() {
        for (schema, good, bad) in [
            (
                json!({"type":"integer","minimum":1,"maximum":5}),
                json!("3"),
                json!("1.5"),
            ),
            (
                json!({"type":"number","minimum":0}),
                json!("1.5"),
                json!("NaN"),
            ),
            (
                json!({"type":"string","minLength":2,"maxLength":5,"pattern":"^[a-z]+$"}),
                json!("ab"),
                json!("A1"),
            ),
            (
                json!({"type":"string","enum":["one","two"]}),
                json!("one"),
                json!("bad"),
            ),
            (
                json!({"type":"string","format":"date"}),
                json!("2024-02-29"),
                json!("2025-02-29"),
            ),
            (json!({"type":"boolean"}), json!(false), json!("false")),
            (
                json!({"type":"array","items":{"type":"string","enum":["a","b"]},"minItems":1}),
                json!(["a"]),
                json!([]),
            ),
        ] {
            let parsed =
                Schema::parse(&json!({"type":"object","properties":{"x":schema},"required":["x"]}))
                    .unwrap();
            assert!(parsed.fields[0].required);
            assert!(parsed.fields[0].validate(good).is_ok());
            assert!(parsed.fields[0].validate(bad).is_err());
        }
    }
    #[test]
    fn rejects_remote_nested_and_malformed_schemas() {
        for schema in [
            json!({"type":"object"}),
            json!({"type":"string","$ref":"https://example.com"}),
            json!({"type":"array","items":{"$ref":"file:///etc/passwd"}}),
            json!({"type":"string","pattern":"["}),
            json!({"type":"boolean","default":"yes"}),
            json!({"type":"string","enum":[]}),
        ] {
            assert!(Schema::parse(&json!({"type":"object","properties":{"x":schema}})).is_err());
        }
        assert!(
            Schema::parse(&json!({"type":"object","properties":{},"required":["missing"]}))
                .is_err()
        );
    }
}
