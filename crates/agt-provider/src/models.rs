use std::net::IpAddr;

use serde::Deserialize;

use crate::config::ProviderConfig;
use crate::error::{ProviderError, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderModel {
    pub id: String,
    pub owned_by: Option<String>,
}

/// OpenAI-compatible and Anthropic list `data[].id`; Gemini lists `models[].name`.
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Option<Vec<OpenAiModelItem>>,
    models: Option<Vec<GeminiModelItem>>,
}

#[derive(Debug, Deserialize)]
struct GeminiModelItem {
    name: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
}

pub fn list_openai_compatible_models(config: &ProviderConfig) -> Result<Vec<ProviderModel>> {
    let url = models_url(&config.base_url)?;
    let client = build_http_client(&url)?;
    // A provider has no API format of its own, so try each vendor's credentials in
    // turn: OpenAI-compatible bearer, then Anthropic, then Gemini.
    let attempts: [&dyn Fn() -> reqwest::blocking::RequestBuilder; 3] = [
        &|| client.get(url.clone()).bearer_auth(&config.api_key),
        &|| {
            client
                .get(url.clone())
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
        },
        &|| {
            let mut url = url.clone();
            url.set_query(Some("pageSize=1000"));
            client.get(url).header("x-goog-api-key", &config.api_key)
        },
    ];
    let mut status = None;
    for attempt in attempts {
        let response = attempt().send().map_err(ProviderError::http)?;
        if response.status().is_success() {
            let text = response.text().map_err(ProviderError::http)?;
            return parse_models_response(&text);
        }
        let code = response.status();
        status = Some(code);
        if !matches!(code.as_u16(), 400 | 401 | 403) {
            break;
        }
    }
    Err(ProviderError::Remote {
        message: format!(
            "/models 接口返回 HTTP {}。该 provider 可能不支持模型列表接口，请手动使用 agt model add-alias",
            status.map(|s| s.to_string()).unwrap_or_default()
        ),
    })
}

pub fn models_url(base_url: &str) -> Result<reqwest::Url> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    reqwest::Url::parse(&url).map_err(|source| ProviderError::InvalidConfig {
        message: format!("base_url 无法拼接 /models：{source}"),
    })
}

pub fn parse_models_response(text: &str) -> Result<Vec<ProviderModel>> {
    let invalid = |detail: String| ProviderError::Remote {
        message: format!("/models 响应不是可识别的模型列表格式：{detail}"),
    };
    let response: ModelsResponse =
        serde_json::from_str(text).map_err(|source| invalid(source.to_string()))?;
    let mut models = match (response.data, response.models) {
        (Some(data), _) => data
            .into_iter()
            .map(|item| ProviderModel {
                id: item.id,
                owned_by: item.owned_by,
            })
            .collect::<Vec<_>>(),
        (None, Some(models)) => models
            .into_iter()
            .map(|item| ProviderModel {
                id: item.name.trim_start_matches("models/").to_owned(),
                owned_by: None,
            })
            .collect(),
        (None, None) => return Err(invalid("missing data or models".into())),
    };
    models.retain(|item| !item.id.trim().is_empty());
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
        assert!(parse_models_response(r#"{"items":[]}"#).is_err());
    }

    #[test]
    fn parses_gemini_models_response() {
        let models = parse_models_response(
            r#"{"models":[{"name":"models/gemini-2.5-pro"},{"name":"models/gemini-2.5-flash"}]}"#,
        )
        .expect("parse");
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["gemini-2.5-flash", "gemini-2.5-pro"]
        );
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
