//! WIT schema auto-extraction from WASM tool components.
//!
//! Extracts tool metadata (name, description, parameter schema) by briefly
//! instantiating the component with stub host functions and calling the
//! `description()` and `schema()` exports defined in `wit/tool.wit`.
//!
//! This eliminates the need for manually specifying schema in capabilities
//! files — tools self-describe via their WIT exports.

use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use crate::tools::wasm::limits::WasmResourceLimiter;

// Generate separate bindgen bindings scoped to this module.
// These are only used for schema extraction, not for full execution.
wasmtime::component::bindgen!({
    path: "wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});

/// Extracted schema from a WASM tool component's WIT exports.
#[derive(Debug, Clone)]
pub struct WitToolSchema {
    /// Tool name (from the component or file stem).
    pub name: String,
    /// Tool description from calling `description()` export.
    pub description: Option<String>,
    /// Parsed parameter definitions from the JSON Schema returned by `schema()`.
    pub parameters: Vec<WitParameter>,
    /// Raw JSON Schema returned by the `schema()` export.
    pub raw_schema: Option<serde_json::Value>,
}

/// A single parameter extracted from a tool's JSON Schema.
#[derive(Debug, Clone)]
pub struct WitParameter {
    /// Parameter name (JSON property key).
    pub name: String,
    /// WIT type string (e.g., "string", "u32", "option<string>").
    pub wit_type: String,
    /// Mapped JSON Schema type string.
    pub json_schema_type: String,
    /// Whether this parameter is required.
    pub required: bool,
}

/// Minimal store data for schema extraction.
///
/// Implements the `Host` trait with no-op stubs — we only need to instantiate
/// long enough to call `schema()` and `description()`.
struct SchemaStoreData {
    limiter: WasmResourceLimiter,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl SchemaStoreData {
    fn new() -> Self {
        Self {
            limiter: WasmResourceLimiter::new(10 * 1024 * 1024), // 10 MB
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for SchemaStoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// Stub host implementation — all functions return safe defaults or errors.
// This is only used during schema extraction, never for real execution.
impl near::agent::host::Host for SchemaStoreData {
    fn log(&mut self, _level: near::agent::host::LogLevel, _message: String) {
        // No-op
    }

    fn now_millis(&mut self) -> u64 {
        0
    }

    fn workspace_read(&mut self, _path: String) -> Option<String> {
        None
    }

    fn http_request(
        &mut self,
        _method: String,
        _url: String,
        _headers_json: String,
        _body: Option<Vec<u8>>,
        _timeout_ms: Option<u32>,
    ) -> Result<near::agent::host::HttpResponse, String> {
        Err("HTTP not available during schema extraction".to_string())
    }

    fn tool_invoke(&mut self, _alias: String, _params_json: String) -> Result<String, String> {
        Err("Tool invocation not available during schema extraction".to_string())
    }

    fn secret_exists(&mut self, _name: String) -> bool {
        false
    }
}

/// Extract tool schema by instantiating the component and calling its exports.
///
/// Instantiates the WASM component with stub host functions (no I/O, no secrets),
/// then calls `description()` and `schema()` to retrieve tool metadata.
///
/// Returns `None` if the component cannot be instantiated or doesn't export
/// the expected `tool` interface.
pub fn extract_wit_schema(
    engine: &Engine,
    component: &Component,
    name: &str,
) -> Option<WitToolSchema> {
    // Create minimal store with stub host
    let mut store = Store::new(engine, SchemaStoreData::new());

    // Set minimal fuel if consumption is enabled on this engine
    let _ = store.set_fuel(1_000_000);

    // Set epoch deadline to prevent hangs during extraction
    store.epoch_deadline_trap();
    store.set_epoch_deadline(10); // 5 seconds at 500ms ticks

    // Set up resource limiter
    store.limiter(|data| &mut data.limiter);

    // Create linker with stub host functions
    let mut linker = Linker::new(engine);
    if wasmtime_wasi::add_to_linker_sync(&mut linker).is_err() {
        tracing::warn!(
            name = name,
            "Failed to add WASI to linker for schema extraction"
        );
        return None;
    }
    if near::agent::host::add_to_linker(&mut linker, |state| state).is_err() {
        tracing::warn!(
            name = name,
            "Failed to add host functions to linker for schema extraction"
        );
        return None;
    }

    // Instantiate the component
    let instance = match SandboxedTool::instantiate(&mut store, component, &linker) {
        Ok(inst) => inst,
        Err(e) => {
            tracing::debug!(
                name = name,
                error = %e,
                "Could not instantiate component for schema extraction"
            );
            return None;
        }
    };

    // Call description() export
    let description = match instance.near_agent_tool().call_description(&mut store) {
        Ok(desc) if !desc.is_empty() => Some(desc),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(name = name, error = %e, "Failed to call description() export");
            None
        }
    };

    // Call schema() export
    let (raw_schema, parameters) = match instance.near_agent_tool().call_schema(&mut store) {
        Ok(schema_str) => match serde_json::from_str::<serde_json::Value>(&schema_str) {
            Ok(schema_val) => {
                let params = extract_parameters_from_schema(&schema_val);
                (Some(schema_val), params)
            }
            Err(e) => {
                tracing::debug!(
                    name = name,
                    error = %e,
                    "schema() returned invalid JSON"
                );
                (None, Vec::new())
            }
        },
        Err(e) => {
            tracing::debug!(name = name, error = %e, "Failed to call schema() export");
            (None, Vec::new())
        }
    };

    Some(WitToolSchema {
        name: name.to_string(),
        description,
        parameters,
        raw_schema,
    })
}

/// Extract parameter definitions from a JSON Schema object.
///
/// Parses the `properties` and `required` fields of a JSON Schema to
/// produce structured `WitParameter` entries.
fn extract_parameters_from_schema(schema: &serde_json::Value) -> Vec<WitParameter> {
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let required_set: std::collections::HashSet<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    properties
        .iter()
        .map(|(name, prop)| {
            let json_type = prop
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("string");

            let required = required_set.contains(name.as_str());
            let wit_type = json_schema_type_to_wit(json_type, required);

            WitParameter {
                name: name.clone(),
                wit_type,
                json_schema_type: json_type.to_string(),
                required,
            }
        })
        .collect()
}

/// Map JSON Schema type to WIT type string.
fn json_schema_type_to_wit(json_type: &str, required: bool) -> String {
    let base = match json_type {
        "string" => "string",
        "integer" => "s64",
        "number" => "f64",
        "boolean" => "bool",
        "array" => "list<string>",
        "object" => "string", // JSON-encoded object
        _ => "string",
    };

    if required {
        base.to_string()
    } else {
        format!("option<{}>", base)
    }
}

/// Convert a `WitToolSchema` to a JSON Schema `serde_json::Value`.
///
/// If the schema already has a `raw_schema` (from calling the tool's `schema()`
/// export), returns that directly. Otherwise, synthesizes a JSON Schema from
/// the extracted parameters.
pub fn wit_schema_to_json_schema(schema: &WitToolSchema) -> serde_json::Value {
    // If we have the raw schema from the tool, use it directly
    if let Some(raw) = &schema.raw_schema {
        return raw.clone();
    }

    // Synthesize from parameters
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for param in &schema.parameters {
        properties.insert(param.name.clone(), wit_type_to_json_schema(&param.wit_type));
        if param.required {
            required.push(serde_json::Value::String(param.name.clone()));
        }
    }

    let mut schema_obj = serde_json::json!({
        "type": "object",
        "properties": properties,
    });

    if !required.is_empty() {
        schema_obj["required"] = serde_json::Value::Array(required);
    }

    schema_obj
}

/// Convert a WIT type string to a JSON Schema type definition.
///
/// Mapping:
/// - `string` → `{"type": "string"}`
/// - `u32`, `u64`, `s32`, `s64` → `{"type": "integer"}`
/// - `f32`, `f64` → `{"type": "number"}`
/// - `bool` → `{"type": "boolean"}`
/// - `option<T>` → same as T (optionality expressed via `required`)
/// - `list<T>` → `{"type": "array", "items": <T schema>}`
/// - Records → `{"type": "object"}`
pub fn wit_type_to_json_schema(wit_type: &str) -> serde_json::Value {
    let trimmed = wit_type.trim();

    // Handle option<T>
    if let Some(inner) = trimmed
        .strip_prefix("option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return wit_type_to_json_schema(inner);
    }

    // Handle list<T>
    if let Some(inner) = trimmed
        .strip_prefix("list<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return serde_json::json!({
            "type": "array",
            "items": wit_type_to_json_schema(inner)
        });
    }

    match trimmed {
        "string" => serde_json::json!({"type": "string"}),
        "u8" | "u16" | "u32" | "u64" | "s8" | "s16" | "s32" | "s64" => {
            serde_json::json!({"type": "integer"})
        }
        "f32" | "f64" | "float32" | "float64" => {
            serde_json::json!({"type": "number"})
        }
        "bool" => serde_json::json!({"type": "boolean"}),
        _ => serde_json::json!({"type": "object"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wit_type_to_json_schema_primitives() {
        assert_eq!(
            wit_type_to_json_schema("string"),
            serde_json::json!({"type": "string"})
        );
        assert_eq!(
            wit_type_to_json_schema("u32"),
            serde_json::json!({"type": "integer"})
        );
        assert_eq!(
            wit_type_to_json_schema("s64"),
            serde_json::json!({"type": "integer"})
        );
        assert_eq!(
            wit_type_to_json_schema("f64"),
            serde_json::json!({"type": "number"})
        );
        assert_eq!(
            wit_type_to_json_schema("bool"),
            serde_json::json!({"type": "boolean"})
        );
    }

    #[test]
    fn test_wit_type_to_json_schema_option() {
        assert_eq!(
            wit_type_to_json_schema("option<string>"),
            serde_json::json!({"type": "string"})
        );
        assert_eq!(
            wit_type_to_json_schema("option<u32>"),
            serde_json::json!({"type": "integer"})
        );
    }

    #[test]
    fn test_wit_type_to_json_schema_list() {
        assert_eq!(
            wit_type_to_json_schema("list<string>"),
            serde_json::json!({"type": "array", "items": {"type": "string"}})
        );
        assert_eq!(
            wit_type_to_json_schema("list<u32>"),
            serde_json::json!({"type": "array", "items": {"type": "integer"}})
        );
    }

    #[test]
    fn test_json_schema_type_to_wit() {
        assert_eq!(json_schema_type_to_wit("string", true), "string");
        assert_eq!(json_schema_type_to_wit("integer", true), "s64");
        assert_eq!(json_schema_type_to_wit("number", false), "option<f64>");
        assert_eq!(json_schema_type_to_wit("boolean", true), "bool");
        assert_eq!(json_schema_type_to_wit("array", true), "list<string>");
    }

    #[test]
    fn test_extract_parameters_from_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"},
                "limit": {"type": "integer", "description": "Max results"},
                "verbose": {"type": "boolean"}
            },
            "required": ["query"]
        });

        let params = extract_parameters_from_schema(&schema);
        assert_eq!(params.len(), 3);

        let query = params.iter().find(|p| p.name == "query").unwrap();
        assert!(query.required);
        assert_eq!(query.json_schema_type, "string");
        assert_eq!(query.wit_type, "string");

        let limit = params.iter().find(|p| p.name == "limit").unwrap();
        assert!(!limit.required);
        assert_eq!(limit.json_schema_type, "integer");
        assert_eq!(limit.wit_type, "option<s64>");
    }

    #[test]
    fn test_extract_parameters_empty_schema() {
        let schema = serde_json::json!({"type": "object"});
        let params = extract_parameters_from_schema(&schema);
        assert!(params.is_empty());
    }

    #[test]
    fn test_wit_schema_to_json_schema_with_raw() {
        let raw = serde_json::json!({
            "type": "object",
            "properties": {"q": {"type": "string"}},
            "required": ["q"]
        });

        let schema = WitToolSchema {
            name: "test".to_string(),
            description: None,
            parameters: Vec::new(),
            raw_schema: Some(raw.clone()),
        };

        assert_eq!(wit_schema_to_json_schema(&schema), raw);
    }

    #[test]
    fn test_wit_schema_to_json_schema_synthesized() {
        let schema = WitToolSchema {
            name: "test".to_string(),
            description: None,
            parameters: vec![
                WitParameter {
                    name: "input".to_string(),
                    wit_type: "string".to_string(),
                    json_schema_type: "string".to_string(),
                    required: true,
                },
                WitParameter {
                    name: "count".to_string(),
                    wit_type: "option<s64>".to_string(),
                    json_schema_type: "integer".to_string(),
                    required: false,
                },
            ],
            raw_schema: None,
        };

        let json = wit_schema_to_json_schema(&schema);
        assert_eq!(json["type"], "object");
        assert_eq!(json["properties"]["input"]["type"], "string");
        assert_eq!(json["properties"]["count"]["type"], "integer");
        assert_eq!(json["required"], serde_json::json!(["input"]));
    }
}
