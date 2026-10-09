use super::*;

pub(super) fn write(
    h: &Path,
    category: &CategoryName,
    name: &str,
    create: bool,
    data: Value,
) -> Result<()> {
    if create {
        let d: ProviderCreate = serde_json::from_value(data)?;
        let secret = provider_secret(d.api_key, d.api_key_env)?.ok_or_else(|| {
            crate::ToolError::invalid(
                Some("data.api_key"),
                "Provider creation requires api_key or api_key_env.",
            )
        })?;
        ProviderStore::new(h).create_in(
            category,
            &ProviderConfig::openai_compatible(name, d.base_url, secret)?,
        )?;
    } else {
        let d: ProviderUpdate = serde_json::from_value(data)?;
        let secret = provider_secret(d.api_key, d.api_key_env)?;
        ProviderStore::new(h).update_in(
            category,
            name,
            ProviderConfigUpdate {
                base_url: d.base_url,
                api_key: secret,
            },
        )?;
    }
    Ok(())
}

fn provider_secret(key: Option<String>, env: Option<String>) -> Result<Option<String>> {
    match (key, env) {
        (Some(_), Some(_)) => Err(crate::ToolError::invalid(
            Some("data.api_key"),
            "Use api_key or api_key_env, not both.",
        )
        .into()),
        (Some(key), None) if key.trim().is_empty() => Err(crate::ToolError::invalid(
            Some("data.api_key"),
            "API key must not be empty.",
        )
        .into()),
        (Some(key), None) => Ok(Some(key)),
        (None, Some(env)) => provider_credential(&env).map(Some),
        (None, None) => Ok(None),
    }
}

fn provider_credential(key: &str) -> Result<String> {
    let secret = std::env::var(key.trim()).map_err(|_| crate::ToolError::invalid(
        Some("data.api_key_env"),
        "Provider credential environment variable is unavailable. Set it before starting the Web server, then restart the server.",
    ))?;
    if secret.trim().is_empty() {
        return Err(crate::ToolError::invalid(
            Some("data.api_key_env"),
            "Provider credential environment variable is empty. Set a non-empty API key and restart the Web server.",
        ).into());
    }
    Ok(secret)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderCreate {
    base_url: String,
    api_key: Option<String>,
    api_key_env: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderUpdate {
    base_url: Option<String>,
    api_key: Option<String>,
    api_key_env: Option<String>,
}
