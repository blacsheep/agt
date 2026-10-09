use super::*;

pub(super) fn write(
    h: &Path,
    category: &CategoryName,
    name: &str,
    create: bool,
    data: Value,
) -> Result<()> {
    let config = if create {
        data
    } else {
        let mut old = serde_json::to_value(
            agt_harness::HarnessStore::new(h)
                .read_in(category, name)?
                .config,
        )?;
        for (key, value) in data
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("data must be an object"))?
        {
            old[key] = value.clone();
        }
        old
    };
    agt_harness::HarnessStore::new(h).write_in(
        category,
        name,
        &serde_json::from_value(config)?,
        create,
    )?;
    Ok(())
}
