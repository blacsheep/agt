use crate::error::{PromptError, Result};

pub fn validate_prompt_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');

    if valid {
        Ok(())
    } else {
        Err(PromptError::InvalidName {
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_prompt_names() {
        for name in ["default", "code-review", "team_prompt", "A1"] {
            validate_prompt_name(name).expect("safe prompt name");
        }
    }

    #[test]
    fn rejects_unsafe_prompt_names() {
        for name in ["", ".", "..", "my/prompt", "my.prompt", "my prompt"] {
            assert!(validate_prompt_name(name).is_err(), "{name} should fail");
        }
    }
}
