use crate::error::{Result, SkillError};

pub fn validate_skill_name(name: &str) -> Result<()> {
    agt_core::PackageName::try_from(name)
        .map(|_| ())
        .map_err(|source| SkillError::InvalidName {
            name: name.to_string(),
            source: Box::new(source),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_skill_names() {
        for name in ["code-review", "review-1", "a1"] {
            validate_skill_name(name).expect("safe name should pass");
        }
    }

    #[test]
    fn rejects_unsafe_skill_names() {
        for name in [
            "",
            ".",
            "..",
            "my/skill",
            "my.skill",
            "my skill",
            "review_1",
            "A1",
            "-review",
            "review-",
            "review--code",
        ] {
            assert!(validate_skill_name(name).is_err(), "{name} should fail");
        }
    }

    #[test]
    fn enforces_agent_skills_length_boundary() {
        assert!(validate_skill_name(&"a".repeat(64)).is_ok());
        assert!(validate_skill_name(&"a".repeat(65)).is_err());
    }
}
