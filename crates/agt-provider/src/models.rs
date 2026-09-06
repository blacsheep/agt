use std::net::IpAddr;

use serde::Deserialize;

use crate::config::ProviderConfig;
use crate::error::{ProviderError, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderModel {
    pub id: String,
    pub owned_by: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
}

pub fn list_openai_compatible_models(config: &ProviderConfig) -> Result<Vec<ProviderModel>> {
    let url = models_url(&config.base_url)?;
    let response = build_http_client(&url)?
        .get(url)
        .bearer_auth(&config.api_key)
        .send()
        .map_err(ProviderError::http)?;

    let status = response.status();
    if !status.is_success() {
        return Err(ProviderError::Remote {
            message: format!(
                "/models 接口返回 HTTP {status}。该 provider 可能不支持模型列表接口，请手动使用 agt model add-alias"
            ),
        });
    }

    let text = response.text().map_err(ProviderError::http)?;
    parse_models_response(&text)
}

pub fn models_url(base_url: &str) -> Result<reqwest::Url> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    reqwest::Url::parse(&url).map_err(|source| ProviderError::InvalidConfig {
        message: format!("base_url 无法拼接 /models：{source}"),
    })
}

pub fn parse_models_response(text: &str) -> Result<Vec<ProviderModel>> {
    let response: OpenAiModelsResponse =
        serde_json::from_str(text).map_err(|source| ProviderError::Remote {
            message: format!("/models 响应不是 OpenAI-compatible 格式：{source}"),
        })?;

    let mut models = response
        .data
        .into_iter()
        .filter(|item| !item.id.trim().is_empty())
        .map(|item| ProviderModel {
            id: item.id,
            owned_by: item.owned_by,
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(models)
}

fn build_http_client(url: &reqwest::Url) -> Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder();
    if should_bypass_proxy(url) {
        builder = builder.no_proxy();
    }
    builder.build().map_err(ProviderError::http)
}

fn should_bypass_proxy(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host.parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_models_url_from_versioned_base_url() {
        let url = models_url("https://api.example.com/v1").expect("url");

        assert_eq!(url.as_str(), "https://api.example.com/v1/models");
    }

    #[test]
    fn parses_openai_compatible_models_response() {
        let models = parse_models_response(
            r#"{"object":"list","data":[{"id":"deepseek-chat","object":"model","owned_by":"deepseek"},{"id":"openai/gpt-4o"}]}"#,
        )
        .expect("parse");

        assert_eq!(
            models,
            vec![
                ProviderModel {
                    id: "deepseek-chat".to_string(),
                    owned_by: Some("deepseek".to_string()),
                },
                ProviderModel {
                    id: "openai/gpt-4o".to_string(),
                    owned_by: None,
                },
            ]
        );
    }

    #[test]
    fn rejects_invalid_models_response() {
        assert!(parse_models_response(r#"{"models":[]}"#).is_err());
    }

    #[test]
    fn bypasses_proxy_for_loopback_models_url() {
        let localhost = models_url("http://localhost:11434/v1").expect("localhost url");
        let loopback = models_url("http://127.0.0.1:8000/v1").expect("loopback url");
        let remote = models_url("https://api.example.com/v1").expect("remote url");

        assert!(should_bypass_proxy(&localhost));
        assert!(should_bypass_proxy(&loopback));
        assert!(!should_bypass_proxy(&remote));
    }
}
