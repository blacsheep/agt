use crate::error::{ProviderError, Result};

pub fn validate_provider_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');

    if valid {
        Ok(())
    } else {
        Err(ProviderError::InvalidName {
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_provider_names() {
        for name in ["openai", "my-provider", "provider_1", "A1"] {
            validate_provider_name(name).expect("safe name should pass");
        }
    }

    #[test]
    fn rejects_unsafe_provider_names() {
        for name in ["", ".", "..", "my/provider", "my.provider", "my provider"] {
            assert!(validate_provider_name(name).is_err(), "{name} should fail");
        }
    }
}
