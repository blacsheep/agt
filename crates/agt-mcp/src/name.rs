use crate::error::{McpError, Result};

pub fn validate_mcp_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');

    if valid {
        Ok(())
    } else {
        Err(McpError::InvalidName {
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_mcp_names() {
        for name in ["filesystem", "git_server", "A1"] {
            validate_mcp_name(name).expect("safe name should pass");
        }
    }

    #[test]
    fn rejects_unsafe_mcp_names() {
        for name in ["", ".", "..", "my/mcp", "my.mcp", "my mcp"] {
            assert!(validate_mcp_name(name).is_err(), "{name} should fail");
        }
    }
}
