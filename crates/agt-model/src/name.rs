use crate::error::{ModelError, Result};

pub fn validate_model_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.');

    if valid {
        Ok(())
    } else {
        Err(ModelError::InvalidName {
            name: name.to_string(),
        })
    }
}

pub(crate) fn validate_provider_side_model_name(name: &str) -> Result<()> {
    let valid = !name.is_empty() && !name.chars().any(|ch| ch == '\\' || ch.is_whitespace());

    if valid {
        Ok(())
    } else {
        Err(ModelError::InvalidProviderModelName {
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_model_aliases() {
        for name in ["fast", "fast-model", "model_1", "A1", "gpt-4.1"] {
            validate_model_name(name).expect("safe name should pass");
        }
    }

    #[test]
    fn rejects_unsafe_model_aliases() {
        for name in ["", ".", "..", "my/model", "my\\model", "my model"] {
            assert!(validate_model_name(name).is_err(), "{name} should fail");
        }
    }

    #[test]
    fn provider_side_model_allows_common_model_names() {
        for name in [
            "deepseek-chat",
            "gpt-4.1",
            "claude-3-5-sonnet",
            "qwen3:32b",
            "openai/gpt-4o",
        ] {
            validate_provider_side_model_name(name).expect("provider-side model should pass");
        }
    }
}
