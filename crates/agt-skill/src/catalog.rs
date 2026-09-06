use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agt_core::{PackageRef, ResourceSourceSelector};

use crate::{Result, SkillDocument, SkillError, SkillMetadata, StoredSkillMetadata};

pub const CATALOG_MAX_BYTES: usize = 32 * 1024;
pub const SKILL_DOCUMENT_MAX_BYTES: u64 = 256 * 1024;
pub const FILE_WALK_MAX_DEPTH: usize = 8;
pub const FILE_WALK_MAX_VISITED_ENTRIES: usize = 1024;
pub const FILE_WALK_MAX_RETURNED_PATHS: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkillCatalogBudgetReport {
    pub max_bytes: usize,
    pub rendered_bytes: usize,
    pub included: usize,
    pub omitted: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillCatalogEntry {
    pub package: PackageRef,
    pub root: PathBuf,
    pub path: PathBuf,
    pub source: ResourceSourceSelector,
    pub metadata: SkillMetadata,
    frozen_content: Option<SkillContent>,
}

impl SkillCatalogEntry {
    pub fn new(
        package: PackageRef,
        root: PathBuf,
        path: PathBuf,
        source: ResourceSourceSelector,
        metadata: SkillMetadata,
    ) -> Self {
        Self {
            package,
            root,
            path,
            source,
            metadata,
            frozen_content: None,
        }
    }

    pub fn freeze(mut self) -> Result<Self> {
        self.frozen_content = Some(load_entry(&self)?);
        Ok(self)
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen_content.is_some()
    }
}

impl From<StoredSkillMetadata> for SkillCatalogEntry {
    fn from(value: StoredSkillMetadata) -> Self {
        Self::new(
            value.package,
            value.dir,
            value.path,
            ResourceSourceSelector::Persisted,
            value.metadata,
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SkillCatalogSnapshot {
    entries: BTreeMap<PackageRef, SkillCatalogEntry>,
}

impl SkillCatalogSnapshot {
    pub fn from_persisted(entries: impl IntoIterator<Item = StoredSkillMetadata>) -> Result<Self> {
        Self::new(entries.into_iter().map(SkillCatalogEntry::from))
    }

    pub fn new(entries: impl IntoIterator<Item = SkillCatalogEntry>) -> Result<Self> {
        let mut mapped = BTreeMap::new();
        for entry in entries {
            validate_entry(&entry)?;
            let package = entry.package.clone();
            if mapped.insert(package.clone(), entry).is_some() {
                return Err(SkillError::InvalidConfig {
                    message: format!("catalog 包含重复 package：{package}"),
                });
            }
        }
        Ok(Self { entries: mapped })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, package: &PackageRef) -> Option<&SkillCatalogEntry> {
        self.entries.get(package)
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = &SkillCatalogEntry> {
        self.entries.values()
    }

    pub fn refs(&self) -> impl ExactSizeIterator<Item = &PackageRef> {
        self.entries.keys()
    }

    pub fn load(&self, package: &PackageRef) -> Result<SkillContent> {
        let entry = self
            .entries
            .get(package)
            .ok_or_else(|| SkillError::NotFound {
                name: package.to_string(),
            })?;
        match &entry.frozen_content {
            Some(content) => Ok(content.clone()),
            None => load_entry(entry),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillContent {
    pub package: PackageRef,
    pub root: PathBuf,
    pub path: PathBuf,
    pub body: String,
    pub files: SkillFileList,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SkillFileList {
    pub paths: Vec<String>,
    pub visited_entries: usize,
    pub truncated: bool,
}

fn validate_entry(entry: &SkillCatalogEntry) -> Result<()> {
    let declared = entry.metadata.package_ref()?;
    if declared != entry.package {
        return Err(SkillError::InvalidConfig {
            message: format!(
                "catalog package {} 与 metadata 声明 {} 不一致",
                entry.package, declared
            ),
        });
    }
    if entry.path != entry.root.join(crate::config::SKILL_FILE) {
        return Err(SkillError::InvalidConfig {
            message: format!(
                "catalog SKILL.md 路径 {} 不位于 root {}",
                entry.path.display(),
                entry.root.display()
            ),
        });
    }
    Ok(())
}

fn load_entry(entry: &SkillCatalogEntry) -> Result<SkillContent> {
    let root_metadata =
        fs::symlink_metadata(&entry.root).map_err(|source| SkillError::io(&entry.root, source))?;
    if !root_metadata.file_type().is_dir() {
        return Err(SkillError::UnsupportedFileType {
            path: entry.root.clone(),
        });
    }
    let metadata =
        fs::symlink_metadata(&entry.path).map_err(|source| SkillError::io(&entry.path, source))?;
    if !metadata.file_type().is_file() {
        return Err(SkillError::UnsupportedFileType {
            path: entry.path.clone(),
        });
    }
    if metadata.len() > SKILL_DOCUMENT_MAX_BYTES {
        return Err(SkillError::LimitExceeded {
            path: entry.path.clone(),
            message: format!(
                "SKILL.md {} bytes，超过 runtime 上限 {} bytes",
                metadata.len(),
                SKILL_DOCUMENT_MAX_BYTES
            ),
        });
    }
    let text =
        fs::read_to_string(&entry.path).map_err(|source| SkillError::io(&entry.path, source))?;
    let document = SkillDocument::parse(&entry.path, &text)?;
    let declared = document.metadata.package_ref()?;
    if declared != entry.package {
        return Err(SkillError::InvalidConfig {
            message: format!(
                "snapshot package {} 与读取到的 SKILL.md 声明 {} 不一致",
                entry.package, declared
            ),
        });
    }
    Ok(SkillContent {
        package: entry.package.clone(),
        root: entry.root.clone(),
        path: entry.path.clone(),
        body: document.body,
        files: walk_skill_files(&entry.root)?,
    })
}

fn walk_skill_files(root: &Path) -> Result<SkillFileList> {
    let metadata = fs::symlink_metadata(root).map_err(|source| SkillError::io(root, source))?;
    if !metadata.file_type().is_dir() {
        return Err(SkillError::UnsupportedFileType {
            path: root.to_path_buf(),
        });
    }
    let mut result = SkillFileList::default();
    walk_dir(root, root, 0, &mut result)?;
    result.paths.sort();
    Ok(result)
}

fn walk_dir(root: &Path, dir: &Path, depth: usize, result: &mut SkillFileList) -> Result<()> {
    if depth > FILE_WALK_MAX_DEPTH {
        result.truncated = true;
        return Ok(());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|source| SkillError::io(dir, source))? {
        if result.visited_entries >= FILE_WALK_MAX_VISITED_ENTRIES {
            result.truncated = true;
            break;
        }
        let entry = entry.map_err(|source| SkillError::io(dir, source))?;
        result.visited_entries += 1;
        entries.push(entry);
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        if result.paths.len() >= FILE_WALK_MAX_RETURNED_PATHS {
            result.truncated = true;
            break;
        }
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| SkillError::io(&path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(SkillError::UnsupportedFileType { path });
        }
        if metadata.file_type().is_dir() {
            walk_dir(root, &path, depth + 1, result)?;
        } else if metadata.file_type().is_file() {
            if path == root.join(crate::config::SKILL_FILE) {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| SkillError::InvalidConfig {
                    message: format!("skill file 逃逸 root：{}", path.display()),
                })?;
            result
                .paths
                .push(relative.to_string_lossy().replace('\\', "/"));
        } else {
            return Err(SkillError::UnsupportedFileType { path });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{SkillCreateOptions, SkillStore};

    use super::*;

    fn home(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("agt-catalog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn snapshot(store: &SkillStore, references: &[&str]) -> SkillCatalogSnapshot {
        SkillCatalogSnapshot::from_persisted(
            references
                .iter()
                .map(|reference| store.read_metadata(&reference.parse().unwrap()).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn exact_snapshot_membership_has_no_short_alias() {
        let home = home("exact");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("@acme/review", "Review").unwrap())
            .unwrap();
        let snapshot = snapshot(&store, &["@acme/review"]);

        assert!(snapshot.load(&"@acme/review".parse().unwrap()).is_ok());
        assert!(matches!(
            snapshot.load(&"review".parse().unwrap()),
            Err(SkillError::NotFound { .. })
        ));
        fs::remove_dir_all(home.join("skills/default/@acme/review")).unwrap();
        assert!(matches!(
            snapshot.load(&"unknown".parse().unwrap()),
            Err(SkillError::NotFound { .. })
        ));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn load_returns_bounded_sorted_file_list() {
        let home = home("files");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("review", "Review").unwrap())
            .unwrap();
        for index in 0..12 {
            fs::write(
                home.join(format!("skills/default/review/references/{index:02}.md")),
                "file",
            )
            .unwrap();
        }
        let snapshot = snapshot(&store, &["review"]);
        let content = snapshot.load(&"review".parse().unwrap()).unwrap();

        assert_eq!(content.files.paths.len(), FILE_WALK_MAX_RETURNED_PATHS);
        assert!(content.files.truncated);
        assert!(
            content
                .files
                .paths
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn load_rejects_oversized_skill_document() {
        let home = home("large");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("large", "Large").unwrap())
            .unwrap();
        let metadata = store.read_metadata(&"large".parse().unwrap()).unwrap();
        let snapshot = SkillCatalogSnapshot::from_persisted([metadata]).unwrap();
        fs::write(
            home.join("skills/default/large/SKILL.md"),
            vec![b'x'; SKILL_DOCUMENT_MAX_BYTES as usize + 1],
        )
        .unwrap();

        assert!(matches!(
            snapshot.load(&"large".parse().unwrap()),
            Err(SkillError::LimitExceeded { .. })
        ));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn load_accepts_exact_document_limit() {
        let home = home("exact-limit");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("exact-limit", "Large").unwrap())
            .unwrap();
        let metadata = store
            .read_metadata(&"exact-limit".parse().unwrap())
            .unwrap();
        let snapshot = SkillCatalogSnapshot::from_persisted([metadata]).unwrap();
        let path = home.join("skills/default/exact-limit/SKILL.md");
        let mut document = fs::read(&path).unwrap();
        document.resize(SKILL_DOCUMENT_MAX_BYTES as usize, b'x');
        fs::write(&path, document).unwrap();

        assert!(snapshot.load(&"exact-limit".parse().unwrap()).is_ok());
        let _ = fs::remove_dir_all(home);
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_snapshot_root_replaced_by_symlink() {
        use std::os::unix::fs::symlink;

        let home = home("root-symlink");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("root-symlink", "Root").unwrap())
            .unwrap();
        let snapshot = snapshot(&store, &["root-symlink"]);
        let root = home.join("skills/default/root-symlink");
        let outside = home.join("outside");
        fs::rename(&root, &outside).unwrap();
        symlink(&outside, &root).unwrap();

        assert!(matches!(
            snapshot.load(&"root-symlink".parse().unwrap()),
            Err(SkillError::UnsupportedFileType { .. })
        ));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn walker_stops_at_depth_and_visited_entry_limits() {
        let home = home("walk-limits");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("walk-limits", "Walk").unwrap())
            .unwrap();
        let root = home.join("skills/default/walk-limits");
        let mut deep = root.clone();
        for index in 0..=FILE_WALK_MAX_DEPTH {
            deep.push(format!("depth-{index}"));
            fs::create_dir_all(&deep).unwrap();
        }
        fs::write(deep.join("too-deep.md"), "deep").unwrap();
        for index in 0..=FILE_WALK_MAX_VISITED_ENTRIES {
            fs::create_dir_all(root.join(format!("empty-{index:04}"))).unwrap();
        }
        let snapshot = snapshot(&store, &["walk-limits"]);
        let content = snapshot.load(&"walk-limits".parse().unwrap()).unwrap();

        assert!(content.files.truncated);
        assert_eq!(content.files.visited_entries, FILE_WALK_MAX_VISITED_ENTRIES);
        assert!(
            content
                .files
                .paths
                .iter()
                .all(|path| !path.ends_with("too-deep.md"))
        );
        let _ = fs::remove_dir_all(home);
    }
}
