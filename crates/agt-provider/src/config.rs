use agt_core::ProviderId;
use serde::{Deserialize, Serialize};

use crate::error::{ProviderError, Result};
use crate::name::validate_provider_name;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProviderType {
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderConfig {
    pub name: ProviderId,
    pub provider_type: ProviderType,
    pub base_url: String,
    pub api_key: String,
}

impl ProviderConfig {
    pub fn openai_compatible(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self> {
        let name = name.into();
        validate_provider_name(&name)?;

        let config = Self {
            name: ProviderId::new(name),
            provider_type: ProviderType::OpenAiCompatible,
            base_url: base_url.into(),
            api_key: api_key.into(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<ProviderValidation> {
        validate_provider_name(self.name.as_str())?;

        let mut issues = Vec::new();
        if self.base_url.trim().is_empty() {
            issues.push("base_url 不能为空".to_string());
        }
        if self.api_key.trim().is_empty() {
            issues.push("api_key 不能为空".to_string());
        }

        if issues.is_empty() {
            Ok(ProviderValidation::Ready)
        } else {
            Err(ProviderError::InvalidConfig {
                message: issues.join("；"),
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderValidation {
    Ready,
}

/// A partial update for a persisted provider definition.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfigUpdate {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

impl ProviderConfigUpdate {
    pub fn is_empty(&self) -> bool {
        self.base_url.is_none() && self.api_key.is_none()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ProviderConfigYaml {
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    pub base_url: String,
    pub api_key: String,
}

impl TryFrom<ProviderConfigYaml> for ProviderConfig {
    type Error = ProviderError;

    fn try_from(value: ProviderConfigYaml) -> Result<Self> {
        let config = Self {
            name: ProviderId::new(value.name),
            provider_type: value.provider_type,
            base_url: value.base_url,
            api_key: value.api_key,
        };
        config.validate()?;
        Ok(config)
    }
}

impl From<&ProviderConfig> for ProviderConfigYaml {
    fn from(value: &ProviderConfig) -> Self {
        Self {
            name: value.name.to_string(),
            provider_type: value.provider_type.clone(),
            base_url: value.base_url.clone(),
            api_key: value.api_key.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_yaml_with_type_field() {
        let config = ProviderConfig::openai_compatible(
            "my-provider",
            "https://api.example.com/v1",
            "sk-test1234",
        )
        .expect("valid provider");

        let yaml =
            serde_yaml::to_string(&ProviderConfigYaml::from(&config)).expect("serialize provider");

        assert!(yaml.contains("name: my-provider"));
        assert!(yaml.contains("type: openai-compatible"));
        assert!(yaml.contains("base_url: https://api.example.com/v1"));
        assert!(yaml.contains("api_key: sk-test1234"));
    }

    #[test]
    fn rejects_incomplete_config() {
        let config = ProviderConfig {
            name: ProviderId::new("broken"),
            provider_type: ProviderType::OpenAiCompatible,
            base_url: String::new(),
            api_key: String::new(),
        };

        assert!(config.validate().is_err());
    }
}
