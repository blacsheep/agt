use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayDomain {
    Provider,
    Model,
    Skill,
    Mcp,
    Kb,
    Prompt,
    Agent,
    Env,
    Conversation,
    Orchestra,
}

impl GatewayDomain {
    pub const ALL: [Self; 10] = [
        Self::Provider,
        Self::Model,
        Self::Skill,
        Self::Mcp,
        Self::Kb,
        Self::Prompt,
        Self::Agent,
        Self::Env,
        Self::Conversation,
        Self::Orchestra,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Model => "model",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
            Self::Kb => "kb",
            Self::Prompt => "prompt",
            Self::Agent => "agent",
            Self::Env => "env",
            Self::Conversation => "conversation",
            Self::Orchestra => "orchestra",
        }
    }
}

impl fmt::Display for GatewayDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GatewayDomain {
    type Err = GatewayDomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "provider" => Ok(Self::Provider),
            "model" => Ok(Self::Model),
            "skill" => Ok(Self::Skill),
            "mcp" => Ok(Self::Mcp),
            "kb" => Ok(Self::Kb),
            "prompt" => Ok(Self::Prompt),
            "agent" => Ok(Self::Agent),
            "env" => Ok(Self::Env),
            "conversation" => Ok(Self::Conversation),
            "orchestra" => Ok(Self::Orchestra),
            _ => Err(GatewayDomainError {
                value: value.to_string(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("gateway domain 无效：{value}")]
pub struct GatewayDomainError {
    value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_domains_round_trip_through_stable_names() {
        for domain in GatewayDomain::ALL {
            assert_eq!(domain.as_str().parse::<GatewayDomain>().unwrap(), domain);
            assert_eq!(
                serde_json::to_string(&domain).unwrap(),
                format!("\"{domain}\"")
            );
        }
    }

    #[test]
    fn rejects_unknown_domain() {
        assert!("resource".parse::<GatewayDomain>().is_err());
    }
}
