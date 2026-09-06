use agt_core::{CategoryName, ProviderId, QualifiedRef};

use crate::error::{ModelError, Result};
use crate::name::validate_provider_side_model_name;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReference {
    provider: QualifiedRef<ProviderId>,
    model: String,
}

impl ModelReference {
    pub fn parse(reference: impl Into<String>) -> Result<Self> {
        Self::parse_in(CategoryName::default(), reference)
    }

    pub fn parse_in(category: CategoryName, reference: impl Into<String>) -> Result<Self> {
        let reference = reference.into();
        let Some((provider, model)) = reference.split_once('/') else {
            return Err(ModelError::InvalidReference { reference });
        };
        if provider.is_empty() || model.is_empty() {
            return Err(ModelError::InvalidReference { reference });
        }

        agt_provider::validate_provider_name(provider).map_err(|_| {
            ModelError::InvalidReference {
                reference: reference.clone(),
            }
        })?;
        validate_provider_side_model_name(model)?;

        Ok(Self {
            provider: QualifiedRef::new(category, ProviderId::new(provider)),
            model: model.to_string(),
        })
    }

    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        Self::new_in(CategoryName::default(), provider, model)
    }

    pub fn new_in(
        category: CategoryName,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self> {
        let provider = provider.into();
        let model = model.into();
        agt_provider::validate_provider_name(&provider).map_err(|_| {
            ModelError::InvalidReference {
                reference: format!("{provider}/{model}"),
            }
        })?;
        validate_provider_side_model_name(&model)?;
        Ok(Self {
            provider: QualifiedRef::new(category, ProviderId::new(provider)),
            model,
        })
    }

    pub fn provider(&self) -> &ProviderId {
        self.provider.name()
    }

    pub fn provider_ref(&self) -> &QualifiedRef<ProviderId> {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

impl std::fmt::Display for ModelReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.provider.category().is_default() {
            write!(f, "{}/{}", self.provider.name(), self.model)
        } else {
            write!(
                f,
                "{}/{}/{}",
                self.provider.category(),
                self.provider.name(),
                self.model
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_provider_model_reference() {
        let reference = ModelReference::parse("my-provider/deepseek-chat").expect("parse");

        assert_eq!(reference.provider().as_str(), "my-provider");
        assert!(reference.provider_ref().category().is_default());
        assert_eq!(reference.model(), "deepseek-chat");
        assert_eq!(reference.to_string(), "my-provider/deepseek-chat");
    }

    #[test]
    fn keeps_explicit_provider_category() {
        let reference = ModelReference::parse_in(
            CategoryName::new("internal").unwrap(),
            "my-provider/deepseek-chat",
        )
        .unwrap();

        assert_eq!(reference.provider_ref().category().as_str(), "internal");
        assert_eq!(reference.to_string(), "internal/my-provider/deepseek-chat");
    }

    #[test]
    fn parses_provider_model_reference_with_slash_in_model_id() {
        let reference = ModelReference::parse("openrouter/openai/gpt-4o").expect("parse");

        assert_eq!(reference.provider().as_str(), "openrouter");
        assert_eq!(reference.model(), "openai/gpt-4o");
        assert_eq!(reference.to_string(), "openrouter/openai/gpt-4o");
    }

    #[test]
    fn rejects_invalid_references() {
        for reference in ["", "provider", "/model", "provider/", "bad name/model"] {
            assert!(
                ModelReference::parse(reference).is_err(),
                "{reference} should fail"
            );
        }
    }
}
