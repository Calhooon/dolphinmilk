//! Schema conversion — x402 manifest input schemas → JSON Schema (OpenAI function calling).
//!
//! x402 manifests describe endpoint inputs in several formats:
//!   Variant A: `{ "contentType": "application/json", "schema": { "prompt": { "type": "string", "required": true } } }`
//!   Variant B: `{ "query": { "type": "string", "required": true } }`  (flat, no wrapper)
//!   Variant C (freetext): `{ "category": "OVERALL|POLITICS|... (default: OVERALL)" }` (string descriptions)
//!
//! This module converts any format into standard JSON Schema suitable for
//! OpenAI function calling. Freetext strings are parsed for pipe-delimited enums,
//! parenthetical defaults, and numeric ranges.

use serde_json::{json, Value};

/// Convert an x402 manifest endpoint's `input` field to a JSON Schema object
/// with `type: "object"`, `properties`, and `required` array.
///
/// Handles both Variant A (wrapped in `"schema"` key) and Variant B (flat properties).
pub fn manifest_input_to_json_schema(input: &Value) -> Value {
    if input.is_null() || (input.is_object() && input.as_object().unwrap().is_empty()) {
        return json!({
            "type": "object",
            "properties": {}
        });
    }

    // Determine which variant we're dealing with:
    // Variant A: has "contentType" or "schema" key
    // Variant B: properties are at the top level
    let props_value = if input.get("schema").is_some() || input.get("contentType").is_some() {
        // Variant A — look inside "schema" key, or fallback to "properties"
        input
            .get("schema")
            .or_else(|| input.get("properties"))
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else if input.get("properties").is_some() {
        // Standard JSON Schema style — has a "properties" key at top level
        input
            .get("properties")
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else {
        // Variant B — the object itself contains the property definitions
        input.clone()
    };

    let props_obj = match props_value.as_object() {
        Some(o) => o,
        None => {
            return json!({
                "type": "object",
                "properties": {}
            })
        }
    };

    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for (name, def) in props_obj {
        // Skip meta-keys that aren't property definitions
        if name == "type" || name == "required" || name == "contentType" || name == "schema" {
            continue;
        }

        let prop_schema = convert_property(def);

        // Check if this property is required
        if def
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            required.push(json!(name));
        }

        properties.insert(name.clone(), prop_schema);
    }

    let mut schema = json!({
        "type": "object",
        "properties": Value::Object(properties),
    });

    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }

    schema
}

/// Parse a freetext string property description into a JSON Schema object.
///
/// Handles patterns like:
///   - `"OVERALL|POLITICS|SPORTS|... (default: OVERALL)"` → enum + default
///   - `"1-50 (default: 25)"` → integer with min/max + default
///   - `"PNL|VOL (default: PNL)"` → enum + default
///   - `"some description"` → string with description
fn parse_freetext_property(text: &str) -> Value {
    let mut prop = serde_json::Map::new();
    let text = text.trim();

    // Extract "(default: ...)" suffix if present
    let (main_part, default_val) = if let Some(paren_start) = text.rfind("(default:") {
        let after = &text[paren_start + 9..]; // skip "(default:"
        let default_str = after.trim_end_matches(')').trim();
        let main = text[..paren_start].trim();
        (main, Some(default_str.to_string()))
    } else if let Some(paren_start) = text.rfind("(default :") {
        let after = &text[paren_start + 10..];
        let default_str = after.trim_end_matches(')').trim();
        let main = text[..paren_start].trim();
        (main, Some(default_str.to_string()))
    } else {
        (text, None)
    };

    // Check if it's pipe-delimited enum values: "A|B|C"
    if main_part.contains('|') {
        let values: Vec<&str> = main_part.split('|').map(|s| s.trim()).collect();
        // Only treat as enum if all parts are non-empty and look like identifiers/values
        if values.iter().all(|v| !v.is_empty()) {
            prop.insert("type".into(), json!("string"));
            let enum_vals: Vec<Value> = values.iter().map(|v| json!(v)).collect();
            prop.insert("enum".into(), Value::Array(enum_vals));
            if let Some(ref def_val) = default_val {
                prop.insert("default".into(), json!(def_val));
            }
            return Value::Object(prop);
        }
    }

    // Check if it's a numeric range: "1-50" (digits-digits)
    if let Some(dash_pos) = main_part.find('-') {
        // Only match N-M where N and M are positive integers and dash isn't at position 0
        if dash_pos > 0 {
            let left = &main_part[..dash_pos].trim();
            let right = &main_part[dash_pos + 1..].trim();
            if let (Ok(min), Ok(max)) = (left.parse::<i64>(), right.parse::<i64>()) {
                if min < max {
                    prop.insert("type".into(), json!("integer"));
                    prop.insert("minimum".into(), json!(min));
                    prop.insert("maximum".into(), json!(max));
                    if let Some(ref def_val) = default_val {
                        if let Ok(def_num) = def_val.parse::<i64>() {
                            prop.insert("default".into(), json!(def_num));
                        } else {
                            prop.insert("default".into(), json!(def_val));
                        }
                    }
                    return Value::Object(prop);
                }
            }
        }
    }

    // Fallback: treat as string with the original text as description
    prop.insert("type".into(), json!("string"));
    prop.insert("description".into(), json!(text));
    if let Some(ref def_val) = default_val {
        prop.insert("default".into(), json!(def_val));
    }
    Value::Object(prop)
}

/// Convert a single property definition from manifest format to JSON Schema.
fn convert_property(def: &Value) -> Value {
    // Handle freetext string descriptions (Variant C):
    // e.g., "OVERALL|POLITICS|SPORTS|... (default: OVERALL)"
    if let Some(text) = def.as_str() {
        return parse_freetext_property(text);
    }

    let mut prop = serde_json::Map::new();

    // Handle type conversion
    if let Some(type_str) = def.get("type").and_then(|v| v.as_str()) {
        match type_str {
            "string[]" => {
                prop.insert("type".into(), json!("array"));
                prop.insert("items".into(), json!({"type": "string"}));
            }
            "number[]" | "integer[]" => {
                let inner = type_str.trim_end_matches("[]");
                prop.insert("type".into(), json!("array"));
                prop.insert("items".into(), json!({"type": inner}));
            }
            "string | string[]" | "string|string[]" => {
                prop.insert(
                    "oneOf".into(),
                    json!([
                        {"type": "string"},
                        {"type": "array", "items": {"type": "string"}}
                    ]),
                );
            }
            "number | number[]" | "number|number[]" => {
                prop.insert(
                    "oneOf".into(),
                    json!([
                        {"type": "number"},
                        {"type": "array", "items": {"type": "number"}}
                    ]),
                );
            }
            "object" => {
                prop.insert("type".into(), json!("object"));
                // If the property has nested properties, convert them too
                if let Some(nested_props) = def.get("properties").and_then(|p| p.as_object()) {
                    let mut converted = serde_json::Map::new();
                    for (k, v) in nested_props {
                        converted.insert(k.clone(), convert_property(v));
                    }
                    prop.insert("properties".into(), Value::Object(converted));
                }
            }
            other => {
                // Normalize non-standard types to valid JSON Schema types.
                // OpenAI only accepts: string, number, integer, boolean, array, object, null.
                let normalized = match other {
                    "string" | "number" | "integer" | "boolean" | "null" | "array" | "object" => {
                        other.to_string()
                    }
                    "int" | "int32" | "int64" | "i32" | "i64" => "integer".to_string(),
                    "float" | "double" | "f32" | "f64" => "number".to_string(),
                    "bool" => "boolean".to_string(),
                    // Anything else (e.g. "AtomicBEEF (base64)", "binary") → string with description
                    _ => "string".to_string(),
                };
                prop.insert("type".into(), json!(normalized));
            }
        }
    }

    // Pass through standard JSON Schema fields
    if let Some(desc) = def.get("description") {
        // If the type was normalized from a non-standard type, append the original format
        let original_type = def.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let is_normalized = !matches!(
            original_type,
            "" | "string"
                | "number"
                | "integer"
                | "boolean"
                | "null"
                | "array"
                | "object"
                | "string[]"
                | "number[]"
                | "integer[]"
                | "string | string[]"
                | "string|string[]"
                | "number | number[]"
                | "number|number[]"
                | "int"
                | "int32"
                | "int64"
                | "i32"
                | "i64"
                | "float"
                | "double"
                | "f32"
                | "f64"
                | "bool"
        );
        if is_normalized {
            let desc_str = desc.as_str().unwrap_or("");
            prop.insert(
                "description".into(),
                json!(format!("{desc_str} (format: {original_type})")),
            );
        } else {
            prop.insert("description".into(), desc.clone());
        }
    }
    if let Some(default) = def.get("default") {
        prop.insert("default".into(), default.clone());
    }
    if let Some(enum_vals) = def.get("enum") {
        prop.insert("enum".into(), enum_vals.clone());
    }
    if let Some(format) = def.get("format") {
        prop.insert("format".into(), format.clone());
    }
    if let Some(min) = def.get("min") {
        prop.insert("minimum".into(), min.clone());
    }
    if let Some(max) = def.get("max") {
        prop.insert("maximum".into(), max.clone());
    }
    if let Some(minimum) = def.get("minimum") {
        prop.insert("minimum".into(), minimum.clone());
    }
    if let Some(maximum) = def.get("maximum") {
        prop.insert("maximum".into(), maximum.clone());
    }
    if let Some(items) = def.get("items") {
        // Only set if not already set by type conversion
        if !prop.contains_key("items") {
            prop.insert("items".into(), items.clone());
        }
    }
    // OpenAI requires arrays to have an `items` schema. Add a default if missing.
    if prop.get("type").and_then(|v| v.as_str()) == Some("array") && !prop.contains_key("items") {
        prop.insert("items".into(), json!({"type": "object"}));
    }
    if let Some(max_items) = def.get("maxItems") {
        prop.insert("maxItems".into(), max_items.clone());
    }

    Value::Object(prop)
}

/// Format a single endpoint's input schema as a human-readable string for LLM consumption.
///
/// Output format:
/// ```text
/// Input:
///   - prompt (string, REQUIRED): Text description of the image
///   - resolution (string, default: "2K", enum: [1K, 2K, 4K]): Resolution tier
/// ```
pub fn format_input_schema(input: &Value) -> String {
    let schema = manifest_input_to_json_schema(input);

    let props = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) if !p.is_empty() => p,
        _ => return String::new(),
    };

    let required_set: Vec<String> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut lines = vec!["    Input:".to_string()];

    for (name, def) in props {
        let mut parts = Vec::new();

        // Type
        if let Some(type_val) = def.get("type").and_then(|v| v.as_str()) {
            if type_val == "array" {
                if let Some(items_type) = def
                    .get("items")
                    .and_then(|i| i.get("type"))
                    .and_then(|t| t.as_str())
                {
                    parts.push(format!("{items_type}[]"));
                } else {
                    parts.push("array".to_string());
                }
            } else {
                parts.push(type_val.to_string());
            }
        } else if def.get("oneOf").is_some() {
            parts.push("string | string[]".to_string());
        }

        // Required
        if required_set.contains(&name.to_string()) {
            parts.push("REQUIRED".to_string());
        }

        // Default
        if let Some(default) = def.get("default") {
            parts.push(format!("default: {default}"));
        }

        // Enum
        if let Some(enum_vals) = def.get("enum").and_then(|e| e.as_array()) {
            let vals: Vec<String> = enum_vals
                .iter()
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .collect();
            parts.push(format!("enum: [{}]", vals.join(", ")));
        }

        // Min/max
        if let Some(min) = def.get("minimum") {
            parts.push(format!("min: {min}"));
        }
        if let Some(max) = def.get("maximum") {
            parts.push(format!("max: {max}"));
        }

        let type_info = if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join(", "))
        };

        // Description
        let desc = def
            .get("description")
            .and_then(|d| d.as_str())
            .map(|d| format!(": {d}"))
            .unwrap_or_default();

        lines.push(format!("      - {name}{type_info}{desc}"));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variant_a_with_schema_key() {
        let input = json!({
            "contentType": "application/json",
            "schema": {
                "prompt": {"type": "string", "required": true, "description": "Image prompt"},
                "resolution": {"type": "string", "default": "2K", "enum": ["1K", "2K", "4K"]}
            }
        });
        let schema = manifest_input_to_json_schema(&input);
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["prompt"]["type"] == "string");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("prompt")));
        assert!(!schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("resolution")));
        assert_eq!(schema["properties"]["resolution"]["default"], "2K");
        assert!(schema["properties"]["resolution"]["enum"].is_array());
    }

    #[test]
    fn test_variant_b_flat() {
        let input = json!({
            "query": {"type": "string", "required": true},
            "max_results": {"type": "integer", "default": 10}
        });
        let schema = manifest_input_to_json_schema(&input);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["query"]["type"], "string");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("query")));
        assert_eq!(schema["properties"]["max_results"]["default"], 10);
    }

    #[test]
    fn test_empty_input() {
        let schema = manifest_input_to_json_schema(&json!({}));
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].as_object().unwrap().is_empty());
    }

    #[test]
    fn test_null_input() {
        let schema = manifest_input_to_json_schema(&Value::Null);
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn test_string_array_type() {
        let input = json!({
            "tags": {"type": "string[]", "description": "List of tags"}
        });
        let schema = manifest_input_to_json_schema(&input);
        let tags = &schema["properties"]["tags"];
        assert_eq!(tags["type"], "array");
        assert_eq!(tags["items"]["type"], "string");
    }

    #[test]
    fn test_string_or_array_type() {
        let input = json!({
            "query": {"type": "string | string[]"}
        });
        let schema = manifest_input_to_json_schema(&input);
        let query = &schema["properties"]["query"];
        assert!(query["oneOf"].is_array());
        let one_of = query["oneOf"].as_array().unwrap();
        assert_eq!(one_of.len(), 2);
    }

    #[test]
    fn test_min_max_conversion() {
        let input = json!({
            "count": {"type": "integer", "min": 1, "max": 100}
        });
        let schema = manifest_input_to_json_schema(&input);
        assert_eq!(schema["properties"]["count"]["minimum"], 1);
        assert_eq!(schema["properties"]["count"]["maximum"], 100);
    }

    #[test]
    fn test_required_extraction() {
        let input = json!({
            "a": {"type": "string", "required": true},
            "b": {"type": "string", "required": false},
            "c": {"type": "string"}
        });
        let schema = manifest_input_to_json_schema(&input);
        let req = schema["required"].as_array().unwrap();
        assert_eq!(req.len(), 1);
        assert!(req.contains(&json!("a")));
    }

    #[test]
    fn test_format_input_schema_output() {
        let input = json!({
            "contentType": "application/json",
            "schema": {
                "prompt": {"type": "string", "required": true, "description": "Text description"},
                "resolution": {"type": "string", "default": "2K", "enum": ["1K", "2K", "4K"]}
            }
        });
        let output = format_input_schema(&input);
        assert!(output.contains("Input:"));
        assert!(output.contains("prompt"));
        assert!(output.contains("REQUIRED"));
        assert!(output.contains("Text description"));
        assert!(output.contains("resolution"));
        assert!(output.contains("default: \"2K\""));
        assert!(output.contains("enum: [1K, 2K, 4K]"));
    }

    #[test]
    fn test_format_empty_input() {
        let output = format_input_schema(&json!({}));
        assert!(output.is_empty());
    }

    #[test]
    fn test_properties_key_passthrough() {
        // Standard JSON Schema style with "properties" at top level
        let input = json!({
            "properties": {
                "url": {"type": "string", "required": true}
            }
        });
        let schema = manifest_input_to_json_schema(&input);
        assert_eq!(schema["properties"]["url"]["type"], "string");
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("url")));
    }

    #[test]
    fn test_unknown_type_normalized_to_string() {
        let input = json!({
            "data": {"type": "binary", "description": "Raw data"}
        });
        let schema = manifest_input_to_json_schema(&input);
        // Non-standard types are normalized to "string" for OpenAI compatibility
        assert_eq!(schema["properties"]["data"]["type"], "string");
        // Original type info preserved in description
        assert!(schema["properties"]["data"]["description"]
            .as_str()
            .unwrap()
            .contains("binary"));
    }

    #[test]
    fn test_atomic_beef_type_normalized() {
        let input = json!({
            "payment": {"type": "AtomicBEEF (base64)", "description": "Payment transaction"}
        });
        let schema = manifest_input_to_json_schema(&input);
        assert_eq!(schema["properties"]["payment"]["type"], "string");
        assert!(schema["properties"]["payment"]["description"]
            .as_str()
            .unwrap()
            .contains("AtomicBEEF"));
    }

    // --- Freetext string parsing tests (Variant C) ---

    #[test]
    fn test_freetext_pipe_delimited_enum() {
        let input = json!({
            "category": "OVERALL|POLITICS|SPORTS|CRYPTO|CULTURE|ECONOMICS|TECH|FINANCE"
        });
        let schema = manifest_input_to_json_schema(&input);
        let cat = &schema["properties"]["category"];
        assert_eq!(cat["type"], "string");
        let enum_vals = cat["enum"].as_array().unwrap();
        assert_eq!(enum_vals.len(), 8);
        assert!(enum_vals.contains(&json!("OVERALL")));
        assert!(enum_vals.contains(&json!("FINANCE")));
    }

    #[test]
    fn test_freetext_enum_with_default() {
        let input = json!({
            "category": "OVERALL|POLITICS|SPORTS (default: OVERALL)"
        });
        let schema = manifest_input_to_json_schema(&input);
        let cat = &schema["properties"]["category"];
        assert_eq!(cat["type"], "string");
        assert_eq!(cat["default"], "OVERALL");
        let enum_vals = cat["enum"].as_array().unwrap();
        assert_eq!(enum_vals.len(), 3);
        assert!(enum_vals.contains(&json!("OVERALL")));
    }

    #[test]
    fn test_freetext_range_pattern() {
        let input = json!({
            "limit": "1-50 (default: 25)"
        });
        let schema = manifest_input_to_json_schema(&input);
        let limit = &schema["properties"]["limit"];
        assert_eq!(limit["type"], "integer");
        assert_eq!(limit["minimum"], 1);
        assert_eq!(limit["maximum"], 50);
        assert_eq!(limit["default"], 25);
    }

    #[test]
    fn test_freetext_plain_string() {
        let input = json!({
            "query": "Search query text"
        });
        let schema = manifest_input_to_json_schema(&input);
        let query = &schema["properties"]["query"];
        assert_eq!(query["type"], "string");
        assert_eq!(query["description"], "Search query text");
    }

    #[test]
    fn test_freetext_mixed_with_objects() {
        // Some fields are freetext strings, others are structured objects
        let input = json!({
            "category": "OVERALL|POLITICS|SPORTS (default: OVERALL)",
            "limit": "1-50 (default: 25)",
            "order_by": "PNL|VOL (default: PNL)",
            "verbose": {"type": "boolean", "default": false}
        });
        let schema = manifest_input_to_json_schema(&input);
        // Freetext enum
        assert_eq!(schema["properties"]["category"]["type"], "string");
        assert!(schema["properties"]["category"]["enum"].is_array());
        assert_eq!(schema["properties"]["category"]["default"], "OVERALL");
        // Freetext range
        assert_eq!(schema["properties"]["limit"]["type"], "integer");
        assert_eq!(schema["properties"]["limit"]["minimum"], 1);
        assert_eq!(schema["properties"]["limit"]["maximum"], 50);
        // Freetext enum
        assert_eq!(schema["properties"]["order_by"]["type"], "string");
        let ob_enum = schema["properties"]["order_by"]["enum"].as_array().unwrap();
        assert!(ob_enum.contains(&json!("PNL")));
        assert!(ob_enum.contains(&json!("VOL")));
        assert_eq!(schema["properties"]["order_by"]["default"], "PNL");
        // Structured object
        assert_eq!(schema["properties"]["verbose"]["type"], "boolean");
        assert_eq!(schema["properties"]["verbose"]["default"], false);
    }

    #[test]
    fn test_format_freetext_schema() {
        let input = json!({
            "category": "OVERALL|POLITICS|SPORTS (default: OVERALL)",
            "limit": "1-50 (default: 25)"
        });
        let output = format_input_schema(&input);
        assert!(output.contains("Input:"));
        assert!(output.contains("category"));
        assert!(output.contains("enum: [OVERALL, POLITICS, SPORTS]"));
        assert!(output.contains("default: \"OVERALL\""));
        assert!(output.contains("limit"));
        assert!(output.contains("integer"));
        assert!(output.contains("min: 1"));
        assert!(output.contains("max: 50"));
        assert!(output.contains("default: 25"));
    }

    #[test]
    fn test_freetext_polymirror_full() {
        // Real-world polymirror manifest format
        let input = json!({
            "category": "OVERALL|POLITICS|SPORTS|CRYPTO|CULTURE|ECONOMICS|TECH|FINANCE (default: OVERALL)",
            "limit": "1-50 (default: 25)",
            "order_by": "PNL|VOL (default: PNL)",
            "time_period": "DAY|WEEK|MONTH|ALL (default: WEEK)"
        });
        let schema = manifest_input_to_json_schema(&input);

        // category
        let cat = &schema["properties"]["category"];
        assert_eq!(cat["type"], "string");
        assert_eq!(cat["enum"].as_array().unwrap().len(), 8);
        assert_eq!(cat["default"], "OVERALL");

        // time_period
        let tp = &schema["properties"]["time_period"];
        assert_eq!(tp["type"], "string");
        let tp_enum = tp["enum"].as_array().unwrap();
        assert_eq!(tp_enum.len(), 4);
        assert!(tp_enum.contains(&json!("DAY")));
        assert!(tp_enum.contains(&json!("WEEK")));
        assert_eq!(tp["default"], "WEEK");

        // limit
        let lim = &schema["properties"]["limit"];
        assert_eq!(lim["type"], "integer");
        assert_eq!(lim["minimum"], 1);
        assert_eq!(lim["maximum"], 50);
        assert_eq!(lim["default"], 25);

        // order_by
        let ob = &schema["properties"]["order_by"];
        assert_eq!(ob["type"], "string");
        assert_eq!(ob["enum"].as_array().unwrap().len(), 2);
        assert_eq!(ob["default"], "PNL");

        // Verify the formatted output is usable by an LLM
        let output = format_input_schema(&input);
        assert!(output.contains("OVERALL"));
        assert!(output.contains("POLITICS"));
        assert!(output.contains("DAY"));
        assert!(output.contains("WEEK"));
        assert!(output.contains("PNL"));
        assert!(output.contains("VOL"));
    }
}
