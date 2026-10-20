use super::*;

pub(super) fn write(
    h: &Path,
    category: &CategoryName,
    name: &str,
    create: bool,
    data: Value,
) -> Result<()> {
    if create {
        let d: ModelCreate = serde_json::from_value(data)?;
        if d.params.provider.is_some() || d.params.model.is_some() {
            return Err(crate::ToolError::invalid(
                Some("params"),
                "Give provider and model at the top level of data, not inside params.",
            )
            .into());
        }
        let (cat, provider) = crate::named_ref(&d.provider)?;
        let mut config = ModelConfig::new(name, ModelReference::new_in(cat, provider, d.model)?)?;
        config.apply_update(d.params)?;
        ModelStore::new(h).create_in(category, &config)?;
    } else {
        ModelStore::new(h).update_in(
            category,
            name,
            serde_json::from_value::<ModelConfigUpdate>(data)?,
        )?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelCreate {
    provider: String,
    model: String,
    #[serde(default)]
    params: ModelConfigUpdate,
}
