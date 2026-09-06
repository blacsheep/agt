use serde_json::Value;

pub const REDACTED_VALUE: &str = "<redacted>";

const SENSITIVE_CONTAINERS: &[&str] = &["env", "headers", "http_headers", "env_http_headers"];
const SENSITIVE_KEY_PARTS: &[&str] = &["key", "token", "secret", "password", "authorization"];

pub fn mask_sensitive_json(value: &Value) -> Value {
    mask_value(value, None)
}

fn mask_value(value: &Value, key: Option<&str>) -> Value {
    if key.is_some_and(is_sensitive_key) {
        return Value::String(REDACTED_VALUE.to_string());
    }

    match value {
        Value::Object(map) => {
            let masked = map
                .iter()
                .map(|(key, value)| {
                    let next = if is_sensitive_container(key) {
                        mask_container(value)
                    } else {
                        mask_value(value, Some(key))
                    };
                    (key.clone(), next)
                })
                .collect();
            Value::Object(masked)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| mask_value(item, None)).collect())
        }
        _ => value.clone(),
    }
}

fn mask_container(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.keys()
                .map(|key| (key.clone(), Value::String(REDACTED_VALUE.to_string())))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|_| Value::String(REDACTED_VALUE.to_string()))
                .collect(),
        ),
        Value::Null => Value::Null,
        _ => Value::String(REDACTED_VALUE.to_string()),
    }
}

fn is_sensitive_container(key: &str) -> bool {
    SENSITIVE_CONTAINERS
        .iter()
        .any(|part| key.eq_ignore_ascii_case(part))
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SENSITIVE_KEY_PARTS.iter().any(|part| key.contains(part))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn masks_env_headers_and_secret_like_keys() {
        let value = json!({
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "env": {
                        "API_KEY": "secret",
                        "NORMAL": "value"
                    },
                    "headers": {
                        "Authorization": "Bearer token"
                    },
                    "http_headers": {
                        "X-Account": "literal-secret"
                    },
                    "env_http_headers": {
                        "X-Workspace": "MCP_WORKSPACE"
                    },
                    "nested": {
                        "access_token": "token",
                        "url": "https://example.com"
                    }
                }
            }
        });

        let masked = mask_sensitive_json(&value);

        assert_eq!(
            masked["mcpServers"]["filesystem"]["env"]["API_KEY"],
            REDACTED_VALUE
        );
        assert_eq!(
            masked["mcpServers"]["filesystem"]["env"]["NORMAL"],
            REDACTED_VALUE
        );
        assert_eq!(
            masked["mcpServers"]["filesystem"]["headers"]["Authorization"],
            REDACTED_VALUE
        );
        assert_eq!(
            masked["mcpServers"]["filesystem"]["http_headers"]["X-Account"],
            REDACTED_VALUE
        );
        assert_eq!(
            masked["mcpServers"]["filesystem"]["env_http_headers"]["X-Workspace"],
            REDACTED_VALUE
        );
        assert_eq!(
            masked["mcpServers"]["filesystem"]["nested"]["access_token"],
            REDACTED_VALUE
        );
        assert_eq!(
            masked["mcpServers"]["filesystem"]["nested"]["url"],
            "https://example.com"
        );
    }
}
