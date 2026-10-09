use super::*;

pub(super) fn write(
    h: &Path,
    category: &CategoryName,
    name: &str,
    create: bool,
    data: Value,
) -> Result<()> {
    if create {
        let d: SkillCreate = serde_json::from_value(data)?;
        SkillStore::new(h).create_in(category, SkillCreateOptions::new(name, d.description)?)?;
    } else {
        let d: Text = serde_json::from_value(data)?;
        let package: PackageRef = name.parse()?;
        SkillStore::new(h).update_in(category, &package, &d.text)?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillCreate {
    description: String,
}
