use agt_core::{ModelId, ProviderId, QualifiedRef};
use serde::{Deserialize, Serialize};

use crate::error::{ModelError, Result};
use crate::name::{validate_model_name, validate_provider_side_model_name};
use crate::reference::ModelReference;

pub const DEFAULT_TEMPERATURE: f32 = 0.7;
pub const DEFAULT_TIMEOUT_SECONDS: u32 = 60;
pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelApiFormat {
    #[default]
    ChatCompletions,
    Responses,
}

impl ModelApiFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat-completions",
            Self::Responses => "responses",
        }
    }
}

impl std::fmt::Display for ModelApiFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ModelApiFormat {
    type Err = ModelError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "chat-completions" | "chat" => Ok(Self::ChatCompletions),
            "responses" | "response" => Ok(Self::Responses),
            _ => Err(ModelError::InvalidConfig {
                message: format!("api_format 无效：{value}，可选值为 chat-completions、responses"),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

impl ReasoningEffort {
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::XHigh,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

impl std::fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ReasoningEffort {
    type Err = ModelError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(Self::None),
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            _ => Err(ModelError::InvalidConfig {
                message: format!(
                    "reasoning_effort 无效：{value}，可选值为 none、minimal、low、medium、high、xhigh"
                ),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelParams {
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub timeout: u32,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub thinking_budget_tokens: Option<u32>,
    pub show_reasoning: bool,
}

impl Default for ModelParams {
    fn default() -> Self {
        Self {
            temperature: DEFAULT_TEMPERATURE,
            max_tokens: None,
            timeout: DEFAULT_TIMEOUT_SECONDS,
            reasoning_effort: None,
            thinking_budget_tokens: None,
            show_reasoning: false,
        }
    }
}

impl ModelParams {
    pub fn validate(&self) -> Result<()> {
        let mut issues = Vec::new();
        if !(0.0..=2.0).contains(&self.temperature) {
            issues.push("temperature 必须在 0.0 到 2.0 之间".to_string());
        }
        if self.timeout == 0 {
            issues.push("timeout 必须大于 0".to_string());
        }
        if self.max_tokens == Some(0) {
            issues.push("max_tokens 必须大于 0，或使用 null 表示不限制".to_string());
        }
        if self.thinking_budget_tokens == Some(0) {
            issues.push("thinking_budget_tokens 必须大于 0，或使用 null 表示不设置".to_string());
        }

        if issues.is_empty() {
            Ok(())
        } else {
            Err(ModelError::InvalidConfig {
                message: issues.join("；"),
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCapabilities {
    pub tool_call: bool,
    pub vision: bool,
    pub context_window: u32,
    pub native_compaction: bool,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            tool_call: true,
            vision: false,
            context_window: DEFAULT_CONTEXT_WINDOW,
            native_compaction: false,
        }
    }
}

impl ModelCapabilities {
    pub fn validate(&self) -> Result<()> {
        if self.context_window == 0 {
            Err(ModelError::InvalidConfig {
                message: "context_window 必须大于 0".to_string(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelConfig {
    pub name: ModelId,
    pub provider: QualifiedRef<ProviderId>,
    pub model: String,
    pub api_format: ModelApiFormat,
    pub params: ModelParams,
    pub capabilities: ModelCapabilities,
}

impl ModelConfig {
    pub fn new(name: impl Into<String>, reference: ModelReference) -> Result<Self> {
        let name = name.into();
        validate_model_name(&name)?;

        let config = Self {
            name: ModelId::new(name),
            provider: reference.provider_ref().clone(),
            model: reference.model().to_string(),
            api_format: ModelApiFormat::default(),
            params: ModelParams::default(),
            capabilities: ModelCapabilities::default(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn reference(&self) -> Result<ModelReference> {
        ModelReference::new_in(
            self.provider.category().clone(),
            self.provider.name().to_string(),
            self.model.clone(),
        )
    }

    pub fn validate(&self) -> Result<()> {
        validate_model_name(self.name.as_str())?;
        agt_provider::validate_provider_name(self.provider.name().as_str()).map_err(|_| {
            ModelError::InvalidConfig {
                message: format!("provider 名称无效：{}", self.provider),
            }
        })?;
        validate_provider_side_model_name(&self.model)?;
        self.params.validate()?;
        self.capabilities.validate()?;
        if self.capabilities.native_compaction && self.api_format != ModelApiFormat::Responses {
            return Err(ModelError::InvalidConfig {
                message: "native_compaction 仅支持 api_format=responses".to_string(),
            });
        }
        Ok(())
    }

    pub fn apply_update(&mut self, update: ModelConfigUpdate) -> Result<()> {
        update.validate()?;
        if let Some(temperature) = update.temperature {
            self.params.temperature = temperature;
        }
        if update.clear_max_tokens {
            self.params.max_tokens = None;
        } else if let Some(max_tokens) = update.max_tokens {
            self.params.max_tokens = Some(max_tokens);
        }
        if let Some(timeout) = update.timeout {
            self.params.timeout = timeout;
        }
        if let Some(api_format) = update.api_format {
            self.api_format = api_format;
        }
        if update.clear_reasoning_effort {
            self.params.reasoning_effort = None;
        } else if let Some(reasoning_effort) = update.reasoning_effort {
            self.params.reasoning_effort = Some(reasoning_effort);
        }
        if update.clear_thinking_budget_tokens {
            self.params.thinking_budget_tokens = None;
        } else if let Some(thinking_budget_tokens) = update.thinking_budget_tokens {
            self.params.thinking_budget_tokens = Some(thinking_budget_tokens);
        }
        if let Some(show_reasoning) = update.show_reasoning {
            self.params.show_reasoning = show_reasoning;
        }
        if let Some(tool_call) = update.tool_call {
            self.capabilities.tool_call = tool_call;
        }
        if let Some(vision) = update.vision {
            self.capabilities.vision = vision;
        }
        if update.reset_context_window {
            self.capabilities.context_window = DEFAULT_CONTEXT_WINDOW;
        } else if let Some(context_window) = update.context_window {
            self.capabilities.context_window = context_window;
        }
        if let Some(native_compaction) = update.native_compaction {
            self.capabilities.native_compaction = native_compaction;
        }

        self.validate()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelConfigUpdate {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub clear_max_tokens: bool,
    pub timeout: Option<u32>,
    pub api_format: Option<ModelApiFormat>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub clear_reasoning_effort: bool,
    pub thinking_budget_tokens: Option<u32>,
    pub clear_thinking_budget_tokens: bool,
    pub show_reasoning: Option<bool>,
    pub tool_call: Option<bool>,
    pub vision: Option<bool>,
    pub context_window: Option<u32>,
    pub reset_context_window: bool,
    pub native_compaction: Option<bool>,
}

impl ModelConfigUpdate {
    pub fn is_empty(&self) -> bool {
        self.temperature.is_none()
            && self.max_tokens.is_none()
            && !self.clear_max_tokens
            && self.timeout.is_none()
            && self.api_format.is_none()
            && self.reasoning_effort.is_none()
            && !self.clear_reasoning_effort
            && self.thinking_budget_tokens.is_none()
            && !self.clear_thinking_budget_tokens
            && self.show_reasoning.is_none()
            && self.tool_call.is_none()
            && self.vision.is_none()
            && self.context_window.is_none()
            && !self.reset_context_window
            && self.native_compaction.is_none()
    }

    pub fn validate(&self) -> Result<()> {
        if self
            .temperature
            .is_some_and(|value| !(0.0..=2.0).contains(&value))
        {
            return Err(ModelError::InvalidConfig {
                message: "temperature 必须在 0.0 到 2.0 之间".to_string(),
            });
        }
        if !self.clear_max_tokens && self.max_tokens == Some(0) {
            return Err(ModelError::InvalidConfig {
                message: "max_tokens 必须大于 0".to_string(),
            });
        }
        if self.timeout == Some(0) {
            return Err(ModelError::InvalidConfig {
                message: "timeout 必须大于 0".to_string(),
            });
        }
        if !self.clear_thinking_budget_tokens && self.thinking_budget_tokens == Some(0) {
            return Err(ModelError::InvalidConfig {
                message: "thinking_budget_tokens 必须大于 0".to_string(),
            });
        }
        if !self.reset_context_window && self.context_window == Some(0) {
            return Err(ModelError::InvalidConfig {
                message: "context_window 必须大于 0".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ModelConfigYaml {
    pub name: String,
    pub provider: QualifiedRef<ProviderId>,
    pub model: String,
    #[serde(default)]
    pub api_format: ModelApiFormat,
    pub params: ModelParamsYaml,
    pub capabilities: ModelCapabilitiesYaml,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ModelParamsYaml {
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default = "default_timeout")]
    pub timeout: u32,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
    #[serde(default)]
    pub show_reasoning: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ModelCapabilitiesYaml {
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    #[serde(default)]
    pub native_compaction: bool,
}

impl TryFrom<ModelConfigYaml> for ModelConfig {
    type Error = ModelError;

    fn try_from(value: ModelConfigYaml) -> Result<Self> {
        let config = Self {
            name: ModelId::new(value.name),
            provider: value.provider,
            model: value.model,
            api_format: value.api_format,
            params: ModelParams {
                temperature: value.params.temperature,
                max_tokens: value.params.max_tokens,
                timeout: value.params.timeout,
                reasoning_effort: value.params.reasoning_effort,
                thinking_budget_tokens: value.params.thinking_budget_tokens,
                show_reasoning: value.params.show_reasoning,
            },
            capabilities: ModelCapabilities {
                tool_call: value.capabilities.tool_call,
                vision: value.capabilities.vision,
                context_window: value.capabilities.context_window,
                native_compaction: value.capabilities.native_compaction,
            },
        };
        config.validate()?;
        Ok(config)
    }
}

impl From<&ModelConfig> for ModelConfigYaml {
    fn from(value: &ModelConfig) -> Self {
        Self {
            name: value.name.to_string(),
            provider: value.provider.clone(),
            model: value.model.clone(),
            api_format: value.api_format.clone(),
            params: ModelParamsYaml {
                temperature: value.params.temperature,
                max_tokens: value.params.max_tokens,
                timeout: value.params.timeout,
                reasoning_effort: value.params.reasoning_effort,
                thinking_budget_tokens: value.params.thinking_budget_tokens,
                show_reasoning: value.params.show_reasoning,
            },
            capabilities: ModelCapabilitiesYaml {
                tool_call: value.capabilities.tool_call,
                vision: value.capabilities.vision,
                context_window: value.capabilities.context_window,
                native_compaction: value.capabilities.native_compaction,
            },
        }
    }
}

fn default_temperature() -> f32 {
    DEFAULT_TEMPERATURE
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_SECONDS
}

fn default_context_window() -> u32 {
    DEFAULT_CONTEXT_WINDOW
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_model_config_with_defaults() {
        let reference = ModelReference::parse("my-provider/deepseek-chat").expect("reference");
        let config = ModelConfig::new("fast-model", reference).expect("config");

        assert_eq!(config.params.temperature, 0.7);
        assert_eq!(config.params.max_tokens, None);
        assert_eq!(config.params.timeout, 60);
        assert_eq!(config.api_format, ModelApiFormat::ChatCompletions);
        assert_eq!(config.params.reasoning_effort, None);
        assert_eq!(config.params.thinking_budget_tokens, None);
        assert!(!config.params.show_reasoning);
        assert!(config.capabilities.tool_call);
        assert!(!config.capabilities.vision);
        assert_eq!(config.capabilities.context_window, 200_000);
        assert!(!config.capabilities.native_compaction);
    }

    #[test]
    fn reasoning_effort_parses_and_round_trips_all_supported_values() {
        for effort in ReasoningEffort::ALL {
            assert_eq!(effort.as_str().parse::<ReasoningEffort>().unwrap(), effort);

            let yaml = serde_yaml::to_string(&effort).expect("serialize effort");
            let parsed = serde_yaml::from_str::<ReasoningEffort>(&yaml).expect("parse effort");
            assert_eq!(parsed, effort);
        }
    }

    #[test]
    fn xhigh_serde_matches_cli_value() {
        let yaml = serde_yaml::to_string(&ReasoningEffort::XHigh).unwrap();

        assert_eq!(yaml.trim(), "xhigh");
        assert_eq!(
            serde_yaml::from_str::<ReasoningEffort>("xhigh").unwrap(),
            ReasoningEffort::XHigh
        );
    }

    #[test]
    fn reasoning_effort_rejects_unknown_value_with_complete_choices() {
        let error = "extreme".parse::<ReasoningEffort>().unwrap_err();
        let message = error.to_string();

        for value in ["none", "minimal", "low", "medium", "high", "xhigh"] {
            assert!(message.contains(value), "missing {value}: {message}");
        }
    }

    #[test]
    fn serializes_yaml_with_params_and_capabilities() {
        let reference = ModelReference::parse("my-provider/deepseek-chat").expect("reference");
        let config = ModelConfig::new("fast-model", reference).expect("config");
        let yaml = serde_yaml::to_string(&ModelConfigYaml::from(&config)).expect("serialize");

        assert!(yaml.contains("name: fast-model"));
        assert!(yaml.contains("provider:\n  category: default\n  name: my-provider"));
        assert!(yaml.contains("model: deepseek-chat"));
        assert!(yaml.contains("api_format: chat-completions"));
        assert!(yaml.contains("temperature: 0.7"));
        assert!(yaml.contains("max_tokens: null"));
        assert!(yaml.contains("timeout: 60"));
        assert!(yaml.contains("reasoning_effort: null"));
        assert!(yaml.contains("thinking_budget_tokens: null"));
        assert!(yaml.contains("show_reasoning: false"));
        assert!(yaml.contains("tool_call: true"));
        assert!(yaml.contains("vision: false"));
        assert!(yaml.contains("context_window: 200000"));
        assert!(yaml.contains("native_compaction: false"));
    }

    #[test]
    fn reads_yaml_without_api_format_as_chat_completions() {
        let yaml: ModelConfigYaml = serde_yaml::from_str(
            r#"
name: fast-model
provider:
  category: default
  name: my-provider
model: deepseek-chat
params:
  temperature: 0.7
  max_tokens: null
  timeout: 60
  reasoning_effort: null
  thinking_budget_tokens: null
  show_reasoning: false
capabilities:
  tool_call: false
  vision: false
  context_window: 200000
"#,
        )
        .expect("parse yaml");
        let config = ModelConfig::try_from(yaml).expect("config");

        assert_eq!(config.api_format, ModelApiFormat::ChatCompletions);
    }

    #[test]
    fn reads_legacy_yaml_without_tool_call_as_false() {
        let yaml: ModelConfigYaml = serde_yaml::from_str(
            r#"
name: fast-model
provider:
  category: default
  name: my-provider
model: deepseek-chat
params:
  temperature: 0.7
  max_tokens: null
  timeout: 60
  reasoning_effort: null
  thinking_budget_tokens: null
  show_reasoning: false
capabilities:
  vision: false
  context_window: 200000
"#,
        )
        .expect("parse legacy yaml");
        let config = ModelConfig::try_from(yaml).expect("config");

        assert!(!config.capabilities.tool_call);
    }

    #[test]
    fn explicit_false_tool_call_round_trips() {
        let reference = ModelReference::parse("my-provider/deepseek-chat").expect("reference");
        let mut config = ModelConfig::new("fast-model", reference).expect("config");
        config
            .apply_update(ModelConfigUpdate {
                tool_call: Some(false),
                ..Default::default()
            })
            .expect("disable tool call");

        let yaml = serde_yaml::to_string(&ModelConfigYaml::from(&config)).expect("serialize");
        let stored = serde_yaml::from_str::<ModelConfigYaml>(&yaml).expect("parse stored yaml");
        let restored = ModelConfig::try_from(stored).expect("restore config");

        assert!(yaml.contains("tool_call: false"));
        assert!(!restored.capabilities.tool_call);
    }

    #[test]
    fn native_compaction_requires_responses_api_format() {
        let reference = ModelReference::parse("my-provider/model").expect("reference");
        let mut config = ModelConfig::new("fast-model", reference).expect("config");
        let error = config
            .apply_update(ModelConfigUpdate {
                native_compaction: Some(true),
                ..Default::default()
            })
            .unwrap_err();
        assert!(error.to_string().contains("native_compaction"));

        config
            .apply_update(ModelConfigUpdate {
                api_format: Some(ModelApiFormat::Responses),
                native_compaction: Some(true),
                ..Default::default()
            })
            .expect("responses compaction");
        assert!(config.capabilities.native_compaction);
    }

    #[test]
    fn applies_partial_update_and_clear_flags() {
        let reference = ModelReference::parse("my-provider/deepseek-chat").expect("reference");
        let mut config = ModelConfig::new("fast-model", reference).expect("config");
        config
            .apply_update(ModelConfigUpdate {
                temperature: Some(0.3),
                max_tokens: Some(4096),
                timeout: Some(120),
                api_format: Some(ModelApiFormat::Responses),
                reasoning_effort: Some(ReasoningEffort::High),
                thinking_budget_tokens: Some(8192),
                show_reasoning: Some(true),
                tool_call: Some(true),
                vision: Some(true),
                context_window: Some(1_000_000),
                ..Default::default()
            })
            .expect("update");

        assert_eq!(config.params.temperature, 0.3);
        assert_eq!(config.params.max_tokens, Some(4096));
        assert_eq!(config.params.timeout, 120);
        assert_eq!(config.api_format, ModelApiFormat::Responses);
        assert_eq!(config.params.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(config.params.thinking_budget_tokens, Some(8192));
        assert!(config.params.show_reasoning);
        assert!(config.capabilities.tool_call);
        assert!(config.capabilities.vision);
        assert_eq!(config.capabilities.context_window, 1_000_000);

        config
            .apply_update(ModelConfigUpdate {
                clear_max_tokens: true,
                clear_reasoning_effort: true,
                clear_thinking_budget_tokens: true,
                ..Default::default()
            })
            .expect("clear");

        assert_eq!(config.params.max_tokens, None);
        assert_eq!(config.params.reasoning_effort, None);
        assert_eq!(config.params.thinking_budget_tokens, None);
    }
}
