//! Coordination for management operations, including read/modify/write and deletion.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WriteOptions {
    pub expected_revision: Option<String>,
    pub add: BTreeMap<String, Vec<String>>,
    pub remove: BTreeMap<String, Vec<String>>,
    pub clear: Vec<String>,
}
impl WriteOptions {
    pub(super) fn validate(&self, kind: &str, create: bool, data: &mut Value) -> Result<()> {
        let invalid = |field, message| crate::ToolError::invalid(Some(field), message);
        if create
            && (self.expected_revision.is_some()
                || !self.add.is_empty()
                || !self.remove.is_empty()
                || !self.clear.is_empty())
        {
            return Err(invalid(
                "create",
                "Creation cannot include update operations or an expected revision.",
            )
            .into());
        }
        let object = data
            .as_object_mut()
            .ok_or_else(|| invalid("data", "Resource data must be an object."))?;
        for field in self.add.keys().chain(self.remove.keys()) {
            if kind != "agent" || !["skills", "mcps", "builtin_tools"].contains(&field.as_str()) {
                return Err(invalid(
                    field,
                    "This resource field does not support collection operations.",
                )
                .into());
            }
            if object.get(field).is_some_and(|v| !v.is_null()) || self.clear.contains(field) {
                return Err(invalid(
                    field,
                    "Do not combine replacement or clearing with collection operations.",
                )
                .into());
            }
        }
        for field in &self.clear {
            if object.get(field).is_some_and(|v| !v.is_null()) {
                return Err(invalid(field, "Do not set and clear the same field.").into());
            }
            match (kind, field.as_str()) {
                ("agent", "skills" | "mcps" | "builtin_tools") => {
                    object.insert(field.clone(), json!([]));
                }
                ("agent", "description") => {
                    object.insert(field.clone(), json!(""));
                }
                ("model", "max_tokens" | "reasoning_effort") => {
                    object.insert(format!("clear_{field}"), json!(true));
                }
                ("harness", "model" | "reasoning_effort") => {
                    object.insert(field.clone(), Value::Null);
                }
                _ => return Err(invalid(field, "This resource field cannot be cleared.").into()),
            }
        }
        Ok(())
    }
}

/// A stable lock outside resource directories also covers create/delete/category changes.
pub(crate) fn lock(home: &Path) -> Result<File> {
    agt_storage::ensure_dir_under(home, home)?;
    let path = home.join(".resources.lock");
    agt_storage::check_no_symlinks_under(home, &path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

impl Resources {
    pub(super) fn revision(&self, kind: &str, reference: &str) -> Result<String> {
        let (category, name) = resource_ref(kind, reference)?;
        let key = ResourceKey {
            kind: resource_kind(kind)?,
            category: category.to_string(),
            name,
        };
        let root = crate::deletion::resource_dir(&self.home, &key)?;
        let mut hash = Sha256::new();
        hash.update(key.display());
        hash_tree(&self.home, &root, &root, &mut hash)?;
        Ok(format!("{:x}", hash.finalize()))
    }

    pub fn category_create(&self, kind: &str, category: &CategoryName) -> Result<PathBuf> {
        let _guard = lock(&self.home)?;
        Ok(self.categories(kind)?.create(category)?)
    }
    pub fn category_delete(&self, kind: &str, category: &CategoryName) -> Result<PathBuf> {
        let _guard = lock(&self.home)?;
        Ok(self.categories(kind)?.delete_empty(category)?)
    }
}

// Content revisions detect external edits too; never hash masked public summaries.
fn hash_tree(home: &Path, root: &Path, path: &Path, hash: &mut Sha256) -> Result<()> {
    agt_storage::check_no_symlinks_under(home, path)?;
    let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry.file_name() == ".update.lock" {
            continue;
        }
        let path = entry.path();
        agt_storage::check_no_symlinks_under(home, &path)?;
        let name = path.strip_prefix(root)?.to_string_lossy();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if entry.file_type()?.is_dir() {
            hash.update(b"directory");
            hash_tree(home, root, &path, hash)?;
        } else if entry.file_type()?.is_file() {
            let bytes = std::fs::read(&path)?;
            hash.update(b"file");
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        } else {
            bail!("unsupported resource file type");
        }
    }
    Ok(())
}

pub(super) fn resource_kind(kind: &str) -> Result<ResourceKind> {
    Ok(match kind {
        "agent" => ResourceKind::Agent,
        "model" => ResourceKind::Model,
        "provider" => ResourceKind::Provider,
        "prompt" => ResourceKind::Prompt,
        "skill" => ResourceKind::Skill,
        "mcp" => ResourceKind::Mcp,
        "harness" => ResourceKind::Harness,
        _ => bail!("unknown resource kind"),
    })
}
