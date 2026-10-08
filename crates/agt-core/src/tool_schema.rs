//! Tool contracts shared by the framework and execution engines.
//! Validation never coerces arguments, supplies defaults, or retrieves references.
use jsonschema::{ValidationError, Validator};
use serde::Serialize;
use serde_json::Value;

pub struct ToolSchema(Validator);

/// Safe diagnostics: never include argument values or raw validator messages.
#[derive(Clone, Debug, Serialize, thiserror::Error)]
#[error("{code}: {message} (field: {field}, schema: {schema_path})")]
pub struct ContractError {
    pub code: &'static str,
    pub message: &'static str,
    pub field: String,
    pub schema_path: String,
}

impl ToolSchema {
    pub fn compile(schema: &Value) -> Result<Self, ContractError> {
        if !schema.is_object()
            || schema.get("type").is_some_and(|kind| match kind {
                Value::String(kind) => kind != "object",
                Value::Array(kinds) => !kinds.iter().any(|kind| kind == "object"),
                _ => true,
            })
        {
            return Err(ContractError::schema());
        }
        // MCP defaults to 2020-12. Explicit standard dialects are detected by the
        // library; unknown meta-schemas and external references fail offline.
        jsonschema::options()
            .offline()
            .build(schema)
            .map(Self)
            .map_err(|e| {
                let mut error = ContractError::schema();
                error.schema_path = e.instance_path().to_string().chars().take(512).collect();
                error
            })
    }

    pub fn validate(&self, arguments: &Value) -> Result<(), ContractError> {
        if !arguments.is_object() {
            return Err(ContractError::object());
        }
        self.0.validate(arguments).map_err(|e| diagnostic(&e))
    }
}

impl ContractError {
    fn schema() -> Self {
        Self {
            code: "invalid_tool_schema",
            message: "Invalid tool schema: arguments must be objects; use a supported JSON Schema dialect and bundle references locally.",
            field: String::new(),
            schema_path: String::new(),
        }
    }

    pub fn object() -> Self {
        Self {
            code: "invalid_arguments",
            message: "Arguments must be a JSON object.",
            field: String::new(),
            schema_path: String::new(),
        }
    }
}

fn diagnostic(error: &ValidationError<'_>) -> ContractError {
    use jsonschema::error::ValidationErrorKind as E;
    let message = match error.kind() {
        E::Required { .. } => "Required argument is missing.",
        E::Type { .. } => {
            "Argument type does not match the tool schema; no type coercion is performed."
        }
        E::AdditionalProperties { .. } => {
            "Unknown argument; use only fields allowed by the tool schema."
        }
        E::Enum { .. } | E::Constant { .. } => "Argument must match the declared enum or constant.",
        _ => "Argument violates a declared constraint; check the tool schema.",
    };
    let mut field = error.instance_path().to_string();
    if let E::Required { property } = error.kind()
        && let Some(property) = property.as_str()
    {
        field.push('/');
        field.push_str(&property.replace('~', "~0").replace('/', "~1"));
    }
    ContractError {
        code: "invalid_arguments",
        message,
        field: field.chars().take(512).collect(),
        schema_path: error.schema_path().to_string().chars().take(512).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_contract_is_strict_and_preserves_input() {
        let schema = ToolSchema::compile(&json!({
            "type":"object", "properties":{"items":{"type":"array","minItems":1,
            "items":{"$ref":"#/$defs/item"}}}, "required":["items"], "additionalProperties":false,
            "$defs":{"item":{"type":"object","properties":{"count":{"type":"integer","minimum":1,"maximum":7}},"required":["count"],"additionalProperties":false}}
        })).unwrap();
        for args in [
            json!({}),
            json!({"items":[]}),
            json!({"items":[{"count":"2"}]}),
            json!({"items":[{"count":8}]}),
            json!({"items":[{"count":1,"extra":true}]}),
        ] {
            assert!(schema.validate(&args).is_err());
        }
        let args = json!({"items":[{"count":2}]});
        schema.validate(&args).unwrap();
        assert_eq!(args, json!({"items":[{"count":2}]}));
        let error = schema
            .validate(&json!({"items":[{"count":"SECRET"}]}))
            .unwrap_err();
        assert_eq!(error.field, "/items/0/count");
        assert!(!error.to_string().contains("SECRET"));
    }

    #[test]
    fn union_null_and_additional_properties_follow_schema() {
        let schema = ToolSchema::compile(&json!({"type":"object","oneOf":[
            {"properties":{"action":{"const":"wait"},"seconds":{"type":"integer"}},"required":["action","seconds"]},
            {"properties":{"action":{"const":"silent"}},"required":["action"],"not":{"required":["seconds"]}}
        ]})).unwrap();
        assert!(
            schema
                .validate(&json!({"action":"wait","seconds":2}))
                .is_ok()
        );
        assert!(
            schema
                .validate(&json!({"action":"silent","seconds":null}))
                .is_err()
        );
        assert!(
            schema
                .validate(&json!({"action":"silent","extra":true}))
                .is_ok()
        );
    }

    #[test]
    fn invalid_and_external_schemas_fail_at_compile() {
        for schema in [
            json!({"type":"oops"}),
            json!({"type":"array","items":{"type":"string"}}),
            json!({"$ref":"file:///tmp/secret.json"}),
            json!({"$ref":"https://example.com/schema"}),
            json!({"$schema":"https://example.com/custom"}),
        ] {
            assert!(ToolSchema::compile(&schema).is_err());
        }
    }
}
