use super::*;

pub(super) fn write(
    h: &Path,
    category: &CategoryName,
    name: &str,
    create: bool,
    data: Value,
    options: &WriteOptions,
) -> Result<()> {
    let d: AgentWrite = serde_json::from_value(data)?;
    let mutate = |config: &mut AgentConfig| -> agt_agent::Result<()> {
        apply_agent(config, &d)
            .and_then(|()| apply_mounts(config, options))
            .map_err(|error| agt_agent::AgentError::InvalidConfig {
                message: error.to_string(),
            })
    };
    if create {
        PromptStore::new(h).ensure_default_prompt()?;
        let mut config = if let Some(harness) = &d.harness {
            let (c, n) = crate::named_ref(harness)?;
            AgentConfig::external(name, QualifiedRef::new(c, HarnessId::new(n)))?
        } else {
            let model = d
                .model
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("model or harness required"))?;
            let (_, leaf) = crate::named_ref(model)?;
            AgentConfig::new(name, leaf)?
        };
        mutate(&mut config)?;
        AgentStore::new(h).create_in(category, &config)?;
    } else {
        AgentStore::new(h).update_in(category, name, mutate)?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentWrite {
    harness: Option<String>,
    model: Option<String>,
    prompt: Option<String>,
    skills: Option<Vec<String>>,
    mcps: Option<Vec<String>>,
    builtin_tools: Option<Vec<BuiltinTool>>,
    description: Option<String>,
}
fn apply_agent(config: &mut AgentConfig, d: &AgentWrite) -> Result<()> {
    if d.model.is_some() && d.harness.is_some() {
        bail!("choose model or harness, not both");
    }
    if let Some(value) = &d.harness {
        let (c, n) = crate::named_ref(value)?;
        config.harness = Some(QualifiedRef::new(c, HarnessId::new(n)));
        config.model = None;
    }
    if let Some(value) = &d.model {
        let (c, n) = crate::named_ref(value)?;
        config.model = Some(QualifiedRef::new(c, ModelId::new(n)));
        config.harness = None;
    }
    if let Some(value) = &d.prompt {
        let (c, n) = crate::named_ref(value)?;
        config.prompt = QualifiedRef::new(c, PromptId::new(n));
    }
    if let Some(values) = &d.skills {
        config.skills = values
            .iter()
            .map(|v| {
                let (c, n) = resource_ref("skill", v)?;
                Ok(ResourceRef::persisted_in(c, n.parse()?))
            })
            .collect::<Result<_>>()?;
    }
    if let Some(values) = &d.mcps {
        config.mcps = values
            .iter()
            .map(|v| {
                let (c, n) = crate::named_ref(v)?;
                Ok(QualifiedRef::new(c, McpId::new(n)))
            })
            .collect::<Result<_>>()?;
    }
    if let Some(values) = &d.builtin_tools {
        config.builtin_tools = values.clone();
    }
    if let Some(value) = &d.description {
        config.description = value.clone();
    }
    Ok(())
}

fn apply_mounts(config: &mut AgentConfig, options: &WriteOptions) -> Result<()> {
    update_collection(&mut config.skills, "skills", options, |value| {
        let (c, n) = resource_ref("skill", value)?;
        Ok(ResourceRef::persisted_in(c, n.parse()?))
    })?;
    update_collection(&mut config.mcps, "mcps", options, |value| {
        let (c, n) = crate::named_ref(value)?;
        Ok(QualifiedRef::new(c, McpId::new(n)))
    })?;
    update_collection(
        &mut config.builtin_tools,
        "builtin_tools",
        options,
        |value| Ok(serde_json::from_value(json!(value))?),
    )
}

fn update_collection<T: PartialEq>(
    values: &mut Vec<T>,
    field: &str,
    options: &WriteOptions,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<()> {
    let additions = options
        .add
        .get(field)
        .into_iter()
        .flatten()
        .map(|v| parse(v))
        .collect::<Result<Vec<_>>>()?;
    let removals = options
        .remove
        .get(field)
        .into_iter()
        .flatten()
        .map(|v| parse(v))
        .collect::<Result<Vec<_>>>()?;
    if additions.iter().any(|v| removals.contains(v)) {
        return Err(crate::ToolError::invalid(
            Some(field),
            "Do not add and remove the same reference.",
        )
        .into());
    }
    for value in additions {
        if !values.contains(&value) {
            values.push(value);
        }
    }
    for value in removals {
        let index = values.iter().position(|v| *v == value).ok_or_else(|| {
            crate::ToolError::invalid(
                Some(field),
                "Cannot remove a reference that is not attached.",
            )
        })?;
        values.remove(index);
    }
    Ok(())
}
