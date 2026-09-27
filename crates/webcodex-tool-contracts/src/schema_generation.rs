//! Shared typed JSON Schema generation and Host-compatible presentation normalization.

use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::JsonSchema;
use serde_json::Value;

pub(crate) fn openapi_schema_generator() -> SchemaGenerator {
    SchemaSettings::openapi3()
        .with(|settings| {
            settings.meta_schema = None;
            settings.inline_subschemas = true;
        })
        .into_generator()
}

pub(crate) fn typed_host_schema<T: JsonSchema>() -> Value {
    let generator = openapi_schema_generator();
    let mut schema = serde_json::to_value(generator.into_root_schema_for::<T>())
        .expect("typed JsonSchema must serialize");
    normalize_host_schema_for_output(&mut schema);
    schema
}

/// Normalize presentation details that are brittle across MCP/OpenAPI hosts.
///
/// This deliberately does not define fields, requiredness, enum values, bounds,
/// or request-only tagged-union behavior. Those remain owned by the typed DTO
/// and the caller-specific projection layer.
pub(crate) fn normalize_host_schema(value: &mut Value) {
    normalize_host_schema_inner(value, false);
}

fn normalize_host_schema_for_output(value: &mut Value) {
    normalize_host_schema_inner(value, true);
}

fn normalize_host_schema_inner(value: &mut Value, preserve_nullable: bool) {
    match value {
        Value::Object(object) => {
            object.remove("title");
            object.remove("format");
            if let Some(description) = object.get_mut("description") {
                if let Some(text) = description.as_str() {
                    *description =
                        Value::String(text.split_whitespace().collect::<Vec<_>>().join(" "));
                }
            }
            let was_nullable = object.remove("nullable") == Some(Value::Bool(true));
            let pure_null = was_nullable
                && object
                    .get("enum")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values.len() == 1 && values.first().is_some_and(Value::is_null)
                    })
                && !object.contains_key("type");
            if pure_null {
                object.clear();
                object.insert("type".to_string(), Value::String("null".to_string()));
                return;
            }
            if was_nullable {
                if let Some(values) = object.get_mut("enum").and_then(Value::as_array_mut) {
                    values.retain(|value| !value.is_null());
                }
            }
            if object.get("default").is_some_and(Value::is_null) {
                object.remove("default");
            }
            if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
                for nested in properties.values_mut() {
                    normalize_host_schema_inner(nested, preserve_nullable);
                }
            }
            if let Some(items) = object.get_mut("items") {
                normalize_host_schema_inner(items, preserve_nullable);
            }
            for keyword in ["oneOf", "anyOf", "allOf"] {
                if let Some(branches) = object.get_mut(keyword).and_then(Value::as_array_mut) {
                    for branch in branches {
                        normalize_host_schema_inner(branch, preserve_nullable);
                    }
                }
            }
            if let Some(additional) = object.get_mut("additionalProperties") {
                if additional.is_object() {
                    normalize_host_schema_inner(additional, preserve_nullable);
                }
            }
            if object.contains_key("properties") {
                object
                    .entry("type".to_string())
                    .or_insert_with(|| Value::String("object".to_string()));
                object
                    .entry("required".to_string())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if preserve_nullable {
                    object
                        .entry("additionalProperties".to_string())
                        .or_insert(Value::Bool(false));
                }
            }
            if was_nullable && preserve_nullable {
                let description = object.remove("description");
                let non_null = Value::Object(std::mem::take(object));
                object.insert(
                    "anyOf".to_string(),
                    Value::Array(vec![non_null, serde_json::json!({"type": "null"})]),
                );
                if let Some(description) = description {
                    object.insert("description".to_string(), description);
                }
            }
        }
        Value::Array(values) => {
            for nested in values {
                normalize_host_schema_inner(nested, preserve_nullable);
            }
        }
        _ => {}
    }
}
