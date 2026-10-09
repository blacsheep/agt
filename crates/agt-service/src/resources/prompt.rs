use super::*;

pub(super) fn write(
    h: &Path,
    category: &CategoryName,
    name: &str,
    create: bool,
    data: Value,
) -> Result<()> {
    let d: Text = serde_json::from_value(data)?;
    if create {
        PromptStore::new(h).create_in(category, name, &d.text)?;
    } else {
        PromptStore::new(h).update_in(category, name, &d.text)?;
    }
    Ok(())
}
