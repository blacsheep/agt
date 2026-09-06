use crate::error::{AgentError, Result};

pub fn validate_agent_name(name: &str) -> Result<()> {
    let bytes = name.as_bytes();
    let valid = bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.iter().enumerate().skip(1).all(|(index, byte)| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_')
                || (*byte == b'.'
                    && bytes[index - 1].is_ascii_alphanumeric()
                    && bytes.get(index + 1).is_some_and(u8::is_ascii_alphanumeric))
        });

    if valid {
        Ok(())
    } else {
        Err(AgentError::InvalidName {
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_agent_names() {
        for name in [
            "coder",
            "code-reviewer",
            "agent_1",
            "A1",
            "example-model-1.5",
            "agent.v2",
        ] {
            validate_agent_name(name).expect("safe name should pass");
        }
    }

    #[test]
    fn rejects_unsafe_agent_names() {
        for name in [
            "",
            ".",
            "..",
            ".agent",
            "agent.",
            "agent..name",
            "agent-.name",
            "agent._name",
            "-agent",
            "_agent",
            "my/agent",
            "my\\agent",
            "my agent",
        ] {
            assert!(validate_agent_name(name).is_err(), "{name} should fail");
        }
    }
}
