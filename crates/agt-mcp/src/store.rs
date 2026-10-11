use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agt_config::{AgtPaths, CATEGORY_README_FILE, DefinitionCategories};
use agt_core::CategoryName;
use agt_storage::{check_no_symlinks_under, write_atomic};
use tempfile::Builder;

use crate::config::{MCP_FILE, McpDocument};
use crate::diagnostic::{McpDiagnostic, McpDiagnosticCategory};
use crate::error::{McpError, Result};
use crate::name::validate_mcp_name;

#[derive(Clone, Debug)]
pub struct McpStore {
    paths: AgtPaths,
}

impl McpStore {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            paths: AgtPaths::new(home),
        }
    }

    pub fn from_default_home() -> agt_core::Result<Self> {
        Ok(Self {
            paths: AgtPaths::from_default_home()?,
        })
    }

    pub fn mcps_dir(&self) -> PathBuf {
        self.paths.data_dir("mcps")
    }

    pub fn categories(&self) -> DefinitionCategories {
        DefinitionCategories::new(self.mcps_dir())
    }

    pub fn category_dir(&self, category: &CategoryName) -> PathBuf {
        self.paths.category_dir("mcps", category)
    }

    pub fn mcp_dir(&self, name: &str) -> Result<PathBuf> {
        self.mcp_dir_in(&CategoryName::default(), name)
    }

    pub fn mcp_dir_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        validate_mcp_name(name)?;
        Ok(self.category_dir(category).join(name))
    }

    pub fn mcp_file(&self, name: &str) -> Result<PathBuf> {
        self.mcp_file_in(&CategoryName::default(), name)
    }

    pub fn mcp_file_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        Ok(self.mcp_dir_in(category, name)?.join(MCP_FILE))
    }

    pub fn add_from_str(&self, text: &str) -> Result<StoredMcp> {
        self.add_from_str_in(&CategoryName::default(), text)
    }

    pub fn add_from_str_in(&self, category: &CategoryName, text: &str) -> Result<StoredMcp> {
        let document = McpDocument::parse(MCP_FILE, text)?;
        self.add_in(category, document)
    }

    pub fn add(&self, document: McpDocument) -> Result<StoredMcp> {
        self.add_in(&CategoryName::default(), document)
    }

    pub fn add_in(&self, category: &CategoryName, document: McpDocument) -> Result<StoredMcp> {
        self.add_with_writer_in(category, document, |file, document| {
            let text = document.render_pretty_json()?;
            write_atomic(file, text.as_bytes()).map_err(|source| McpError::io(file, source))
        })
    }

    pub fn apply_batch(&self, plans: Vec<McpWritePlan>) -> Result<Vec<StoredMcp>> {
        self.apply_batch_in(&CategoryName::default(), plans)
    }

    pub fn apply_batch_in(
        &self,
        category: &CategoryName,
        plans: Vec<McpWritePlan>,
    ) -> Result<Vec<StoredMcp>> {
        self.apply_batch_with_rename_in(category, plans, |from, to| fs::rename(from, to))
    }

    pub fn import(&self, source: impl AsRef<Path>) -> Result<StoredMcp> {
        self.import_in(&CategoryName::default(), source)
    }

    pub fn import_in(
        &self,
        category: &CategoryName,
        source: impl AsRef<Path>,
    ) -> Result<StoredMcp> {
        let source = source.as_ref();
        let metadata = fs::symlink_metadata(source).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                McpError::InvalidImportSource {
                    path: source.to_path_buf(),
                }
            } else {
                McpError::io(source, error)
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(McpError::InvalidImportSource {
                path: source.to_path_buf(),
            });
        }

        let text = fs::read_to_string(source).map_err(|error| McpError::io(source, error))?;
        let document = McpDocument::parse(source, &text)?;
        self.add_in(category, document)
    }

    pub fn read(&self, name: &str) -> Result<StoredMcp> {
        self.read_in(&CategoryName::default(), name)
    }

    pub fn read_in(&self, category: &CategoryName, name: &str) -> Result<StoredMcp> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        let dir = self.mcp_dir_in(category, name)?;
        check_no_symlinks_under(self.paths.home(), &dir).map_err(map_core_error)?;
        self.read_from_dir(category, name, &dir)
    }

    pub fn list(&self) -> Result<Vec<StoredMcp>> {
        let scan = self.scan()?;
        if !scan.diagnostics.is_empty() {
            return Err(McpError::StoreDiagnostics {
                count: scan.diagnostics.len(),
            });
        }
        Ok(scan.items)
    }

    pub fn scan(&self) -> Result<McpScan> {
        check_no_symlinks_under(self.paths.home(), self.paths.home()).map_err(map_core_error)?;
        let mcps_dir = self.mcps_dir();
        let root_metadata = match fs::symlink_metadata(&mcps_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(McpScan::default()),
            Err(error) => return Err(McpError::io(&mcps_dir, error)),
        };
        if !root_metadata.file_type().is_dir() {
            return Ok(McpScan {
                items: Vec::new(),
                diagnostics: vec![McpDiagnostic::new(
                    &mcps_dir,
                    None,
                    McpDiagnosticCategory::UnsupportedFileType,
                    "mcps root 必须是普通目录，不能是 symlink 或特殊文件",
                    "删除该路径并重新创建普通 mcps 目录",
                )],
            });
        }

        let mut scan = McpScan::default();
        for entry in read_dir_sorted(&mcps_dir)? {
            let path = entry.path();
            let category_text = entry.file_name().to_string_lossy().to_string();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    scan.diagnostics.push(McpDiagnostic::new(
                        &path,
                        Some(category_text),
                        McpDiagnosticCategory::Io,
                        McpError::io(&path, error).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(McpDiagnostic::new(
                    &path,
                    Some(category_text.clone()),
                    if file_type.is_symlink() {
                        McpDiagnosticCategory::UnsupportedFileType
                    } else {
                        McpDiagnosticCategory::UnexpectedLayout
                    },
                    "mcps root 只能包含 <category>/ 目录",
                    "移动或删除该 entry，或改为规范 category 目录",
                ));
                continue;
            }
            let category = match CategoryName::new(&category_text) {
                Ok(category) => category,
                Err(error) => {
                    scan.diagnostics.push(McpDiagnostic::new(
                        &path,
                        Some(category_text),
                        McpDiagnosticCategory::InvalidIdentity,
                        error.to_string(),
                        "将目录名改为安全 category 名称",
                    ));
                    continue;
                }
            };
            self.scan_category(&category, &path, &mut scan)?;
        }
        scan.items.sort_by(|left, right| {
            (&left.category, left.document.name.as_str())
                .cmp(&(&right.category, right.document.name.as_str()))
        });
        scan.diagnostics
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(scan)
    }

    fn scan_category(
        &self,
        category: &CategoryName,
        category_dir: &Path,
        scan: &mut McpScan,
    ) -> Result<()> {
        for entry in read_dir_sorted(category_dir)? {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == CATEGORY_README_FILE {
                continue;
            }
            let name_hint = Some(format!("{category}/{name}"));
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    scan.diagnostics.push(McpDiagnostic::new(
                        &path,
                        name_hint,
                        McpDiagnosticCategory::Io,
                        McpError::io(&path, error).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(McpDiagnostic::new(
                    &path,
                    name_hint,
                    if file_type.is_symlink() {
                        McpDiagnosticCategory::UnsupportedFileType
                    } else {
                        McpDiagnosticCategory::UnexpectedLayout
                    },
                    "mcp category 只能包含 <name>/mcp.json 目录",
                    "移动或删除该 entry",
                ));
                continue;
            }
            if let Err(error) = validate_mcp_name(&name) {
                scan.diagnostics
                    .push(McpDiagnostic::from_error(&path, name_hint, &error));
                continue;
            }
            match self.read_from_dir(category, &name, &path) {
                Ok(stored) => scan.items.push(stored),
                Err(error) => scan
                    .diagnostics
                    .push(McpDiagnostic::from_error(&path, name_hint, &error)),
            }
        }
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf> {
        self.delete_in(&CategoryName::default(), name)
    }

    pub fn delete_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        let dir = self.mcp_dir_in(category, name)?;
        self.read_in(category, name)?;
        ensure_child_path(&self.category_dir(category), &dir)?;
        fs::remove_dir_all(&dir).map_err(|source| McpError::io(&dir, source))?;
        Ok(dir)
    }

    fn add_with_writer_in<F>(
        &self,
        category: &CategoryName,
        document: McpDocument,
        writer: F,
    ) -> Result<StoredMcp>
    where
        F: FnOnce(&Path, &McpDocument) -> Result<()>,
    {
        document.validate_for_write()?;
        let category_dir = self.require_write_category(category)?;
        let name = document.name.to_string();
        let final_dir = self.mcp_dir_in(category, &name)?;
        check_no_symlinks_under(self.paths.home(), &final_dir).map_err(map_core_error)?;
        match fs::symlink_metadata(&final_dir) {
            Ok(_) => return Err(McpError::AlreadyExists { name }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(McpError::io(&final_dir, error)),
        }

        let staging = Builder::new()
            .prefix(".agt-mcp-stage-")
            .tempdir_in(&category_dir)
            .map_err(|source| McpError::io(&category_dir, source))?;
        let payload = staging.path().join("payload");
        fs::create_dir(&payload).map_err(|source| McpError::io(&payload, source))?;
        let staged_file = payload.join(MCP_FILE);
        writer(&staged_file, &document)?;

        let read_back = self.read_from_dir(category, &name, &payload)?;
        fs::rename(&payload, &final_dir).map_err(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists
                || fs::symlink_metadata(&final_dir).is_ok()
            {
                McpError::AlreadyExists { name: name.clone() }
            } else {
                McpError::io(&final_dir, source)
            }
        })?;

        Ok(StoredMcp {
            category: category.clone(),
            path: final_dir.join(MCP_FILE),
            dir: final_dir,
            document: read_back.document,
        })
    }

    fn apply_batch_with_rename_in<F>(
        &self,
        category: &CategoryName,
        plans: Vec<McpWritePlan>,
        mut rename: F,
    ) -> Result<Vec<StoredMcp>>
    where
        F: FnMut(&Path, &Path) -> io::Result<()>,
    {
        if plans.is_empty() {
            return Err(McpError::InvalidConfig {
                message: "MCP batch plan 不能为空".to_string(),
            });
        }
        let mut names = BTreeSet::new();
        for plan in &plans {
            plan.document.validate_for_write()?;
            let name = plan.document.name.as_str();
            if !names.insert(name.to_string()) {
                return Err(McpError::InvalidConfig {
                    message: format!("MCP batch 包含重复名称：{name}"),
                });
            }
            let final_dir = self.mcp_dir_in(category, name)?;
            check_no_symlinks_under(self.paths.home(), &final_dir).map_err(map_core_error)?;
            let exists = match fs::symlink_metadata(&final_dir) {
                Ok(_) => true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(McpError::io(&final_dir, error)),
            };
            match plan.mode {
                McpWriteMode::Create if exists => {
                    return Err(McpError::AlreadyExists {
                        name: name.to_string(),
                    });
                }
                McpWriteMode::Replace if !exists => {
                    return Err(McpError::NotFound {
                        name: name.to_string(),
                    });
                }
                McpWriteMode::Replace => {
                    self.read_in(category, name)?;
                }
                McpWriteMode::Create => {}
            }
        }

        let category_dir = self.require_write_category(category)?;
        let staging = Builder::new()
            .prefix(".agt-mcp-batch-")
            .tempdir_in(&category_dir)
            .map_err(|source| McpError::io(&category_dir, source))?;
        let payload_root = staging.path().join("payload");
        let backup_root = staging.path().join("backup");
        fs::create_dir(&payload_root).map_err(|source| McpError::io(&payload_root, source))?;
        fs::create_dir(&backup_root).map_err(|source| McpError::io(&backup_root, source))?;

        for plan in &plans {
            let name = plan.document.name.as_str();
            let staged_dir = payload_root.join(name);
            fs::create_dir(&staged_dir).map_err(|source| McpError::io(&staged_dir, source))?;
            let staged_file = staged_dir.join(MCP_FILE);
            let text = plan.document.render_pretty_json()?;
            write_atomic(&staged_file, text.as_bytes())
                .map_err(|source| McpError::io(&staged_file, source))?;
            self.read_from_dir(category, name, &staged_dir)?;
        }

        let mut committed = Vec::<CommittedBatchItem>::new();
        for plan in &plans {
            let name = plan.document.name.to_string();
            let staged_dir = payload_root.join(&name);
            let final_dir = self.mcp_dir_in(category, &name)?;
            let backup_dir = (plan.mode == McpWriteMode::Replace).then(|| backup_root.join(&name));
            if let Some(backup_dir) = &backup_dir
                && let Err(source) = rename(&final_dir, backup_dir)
            {
                let rollback_failures = rollback_committed(&mut rename, &committed);
                return Err(McpError::BatchTransaction {
                    message: McpError::io(&final_dir, source).to_string(),
                    rollback_failures,
                });
            }
            if let Err(source) = rename(&staged_dir, &final_dir) {
                let mut rollback_failures = Vec::new();
                if let Some(backup_dir) = &backup_dir
                    && let Err(error) = rename(backup_dir, &final_dir)
                {
                    rollback_failures.push(McpError::io(&final_dir, error).to_string());
                }
                rollback_failures.extend(rollback_committed(&mut rename, &committed));
                return Err(McpError::BatchTransaction {
                    message: McpError::io(&final_dir, source).to_string(),
                    rollback_failures,
                });
            }
            committed.push(CommittedBatchItem {
                final_dir,
                backup_dir,
            });
        }

        plans
            .into_iter()
            .map(|plan| self.read_in(category, plan.document.name.as_str()))
            .collect()
    }

    fn read_from_dir(
        &self,
        category: &CategoryName,
        expected_name: &str,
        dir: &Path,
    ) -> Result<StoredMcp> {
        let dir_metadata = fs::symlink_metadata(dir).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                McpError::NotFound {
                    name: expected_name.to_string(),
                }
            } else {
                McpError::io(dir, error)
            }
        })?;
        if !dir_metadata.file_type().is_dir() {
            return Err(McpError::UnsupportedFileType {
                path: dir.to_path_buf(),
            });
        }

        let file = dir.join(MCP_FILE);
        let file_metadata = fs::symlink_metadata(&file).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                McpError::NotFound {
                    name: expected_name.to_string(),
                }
            } else {
                McpError::io(&file, error)
            }
        })?;
        if !file_metadata.file_type().is_file() {
            return Err(McpError::UnsupportedFileType { path: file });
        }
        let document = self.read_document_file(&file)?;
        if document.name.as_str() != expected_name {
            return Err(McpError::InvalidConfig {
                message: format!(
                    "目录名 {expected_name} 与 mcpServers key {} 不一致",
                    document.name
                ),
            });
        }
        Ok(StoredMcp {
            category: category.clone(),
            path: file,
            dir: dir.to_path_buf(),
            document,
        })
    }

    fn read_document_file(&self, file: &Path) -> Result<McpDocument> {
        let text = fs::read_to_string(file).map_err(|source| McpError::io(file, source))?;
        McpDocument::parse(file, &text)
    }

    pub fn create_category(&self, category: &CategoryName) -> Result<PathBuf> {
        Ok(self.categories().create(category)?)
    }

    pub fn list_categories(&self) -> Result<Vec<CategoryName>> {
        Ok(self.categories().list()?)
    }

    pub fn delete_category(&self, category: &CategoryName) -> Result<PathBuf> {
        Ok(self.categories().delete_empty(category)?)
    }

    fn require_write_category(&self, category: &CategoryName) -> Result<PathBuf> {
        if category.is_default() {
            Ok(self.categories().ensure_default()?)
        } else {
            Ok(self.categories().require(category)?)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredMcp {
    pub category: CategoryName,
    pub path: PathBuf,
    pub dir: PathBuf,
    pub document: McpDocument,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpWriteMode {
    Create,
    Replace,
}

#[derive(Clone, Debug, PartialEq)]
pub struct McpWritePlan {
    pub document: McpDocument,
    pub mode: McpWriteMode,
}

impl McpWritePlan {
    pub fn create(document: McpDocument) -> Self {
        Self {
            document,
            mode: McpWriteMode::Create,
        }
    }

    pub fn replace(document: McpDocument) -> Self {
        Self {
            document,
            mode: McpWriteMode::Replace,
        }
    }
}

struct CommittedBatchItem {
    final_dir: PathBuf,
    backup_dir: Option<PathBuf>,
}

fn rollback_committed<F>(rename: &mut F, committed: &[CommittedBatchItem]) -> Vec<String>
where
    F: FnMut(&Path, &Path) -> io::Result<()>,
{
    let mut failures = Vec::new();
    for item in committed.iter().rev() {
        if let Err(error) = fs::remove_dir_all(&item.final_dir) {
            failures.push(McpError::io(&item.final_dir, error).to_string());
            continue;
        }
        if let Some(backup_dir) = &item.backup_dir
            && let Err(error) = rename(backup_dir, &item.final_dir)
        {
            failures.push(McpError::io(&item.final_dir, error).to_string());
        }
    }
    failures
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpScan {
    pub items: Vec<StoredMcp>,
    pub diagnostics: Vec<McpDiagnostic>,
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| McpError::io(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| McpError::io(path, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn map_core_error(error: agt_core::AgtError) -> McpError {
    match error {
        agt_core::AgtError::Io { path, source } => McpError::io(path, source),
        other => McpError::InvalidConfig {
            message: other.to_string(),
        },
    }
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<()> {
    let parent = parent
        .canonicalize()
        .map_err(|source| McpError::io(parent, source))?;
    let child = child
        .canonicalize()
        .map_err(|source| McpError::io(child, source))?;
    if child.starts_with(&parent) && child != parent {
        Ok(())
    } else {
        Err(McpError::InvalidConfig {
            message: "mcp 删除路径越界".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn test_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("agt-mcp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    fn sample_json(name: &str) -> String {
        format!(
            r#"{{"mcpServers":{{"{name}":{{"type":"stdio","command":"npx","args":["-y","pkg"],"env":{{"API_KEY":"secret"}},"cwd":"/srv/tools"}}}}}}"#
        )
    }

    #[test]
    fn add_read_scan_and_delete_mcp() {
        let home = test_home("roundtrip");
        let store = McpStore::new(&home);
        let stored = store
            .add_from_str(&sample_json("filesystem"))
            .expect("add mcp");
        assert_eq!(stored.path, home.join("mcps/default/filesystem/mcp.json"));
        assert_eq!(
            store
                .read("filesystem")
                .expect("read")
                .document
                .name
                .as_str(),
            "filesystem"
        );
        let scan = store.scan().expect("scan");
        assert_eq!(scan.items.len(), 1);
        assert!(scan.diagnostics.is_empty());
        store.delete("filesystem").expect("delete");
        assert!(!home.join("mcps/default/filesystem").exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn invalid_config_is_rejected_before_any_target_is_created() {
        let home = test_home("invalid-preflight");
        let store = McpStore::new(&home);
        let error = store
            .add_from_str(r#"{"mcpServers":{"broken":{"type":"stdio"}}}"#)
            .expect_err("invalid config");
        assert!(matches!(error, McpError::InvalidConfig { .. }));
        assert!(!home.join("mcps/default/broken").exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn read_back_failure_removes_staging_and_target() {
        let home = test_home("read-back-cleanup");
        let store = McpStore::new(&home);
        let document = McpDocument::parse(MCP_FILE, &sample_json("broken")).expect("document");
        let error = store
            .add_with_writer_in(&CategoryName::default(), document, |file, _| {
                fs::write(file, "not-json").map_err(|source| McpError::io(file, source))
            })
            .expect_err("read-back should fail");
        assert!(matches!(error, McpError::Json { .. }));
        assert!(!home.join("mcps/default/broken").exists());
        let entries = fs::read_dir(home.join("mcps/default"))
            .expect("default category dir")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("entries");
        assert!(entries.is_empty());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scan_returns_valid_items_and_actionable_diagnostics() {
        let home = test_home("scan-diagnostics");
        let store = McpStore::new(&home);
        store.add_from_str(&sample_json("valid")).expect("valid");
        fs::create_dir_all(home.join("mcps/default/BAD.name")).expect("bad dir");
        fs::write(home.join("mcps/default/BAD.name/mcp.json"), "{}").expect("bad file");
        fs::create_dir_all(home.join("mcps/default/missing")).expect("missing dir");

        let scan = store.scan().expect("scan");
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].document.name.as_str(), "valid");
        assert_eq!(scan.diagnostics.len(), 2);
        assert!(
            scan.diagnostics
                .iter()
                .all(|item| !item.remediation.is_empty())
        );
        assert!(
            scan.diagnostics
                .iter()
                .any(|item| item.category == McpDiagnosticCategory::InvalidIdentity)
        );
        assert!(
            scan.diagnostics
                .iter()
                .any(|item| item.category == McpDiagnosticCategory::MissingConfigFile)
        );
        assert!(matches!(
            store.list(),
            Err(McpError::StoreDiagnostics { count: 2 })
        ));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn batch_create_and_replace_commit_together() {
        let home = test_home("batch-commit");
        let store = McpStore::new(&home);
        store.add_from_str(&sample_json("alpha")).unwrap();
        let replacement = McpDocument::stdio(
            "alpha",
            "replacement",
            vec!["--new".to_string()],
            [("TOKEN".to_string(), "secret".to_string())],
            &std::env::temp_dir(),
        )
        .unwrap();
        let beta =
            McpDocument::stdio("beta", "beta-cmd", Vec::new(), [], &std::env::temp_dir()).unwrap();

        let stored = store
            .apply_batch(vec![
                McpWritePlan::replace(replacement),
                McpWritePlan::create(beta),
            ])
            .unwrap();

        assert_eq!(stored.len(), 2);
        assert_eq!(
            store
                .read("alpha")
                .unwrap()
                .document
                .summary()
                .target
                .as_deref(),
            Some("replacement")
        );
        assert_eq!(
            store
                .read("beta")
                .unwrap()
                .document
                .summary()
                .target
                .as_deref(),
            Some("beta-cmd")
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn batch_invalid_preflight_writes_nothing() {
        let home = test_home("batch-invalid");
        let store = McpStore::new(&home);
        let valid =
            McpDocument::stdio("valid", "cmd", Vec::new(), [], &std::env::temp_dir()).unwrap();
        let invalid = McpDocument {
            name: agt_core::McpId::new("invalid"),
            server: serde_json::json!({"type":"stdio"}),
        };

        assert!(
            store
                .apply_batch(vec![
                    McpWritePlan::create(valid),
                    McpWritePlan::create(invalid),
                ])
                .is_err()
        );
        assert!(!home.join("mcps/default/valid").exists());
        assert!(!home.join("mcps/default/invalid").exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn batch_commit_failure_restores_replacements_and_removes_new_items() {
        let home = test_home("batch-rollback");
        let store = McpStore::new(&home);
        store.add_from_str(&sample_json("alpha")).unwrap();
        let replacement = McpDocument::stdio(
            "alpha",
            "replacement",
            Vec::new(),
            [],
            &std::env::temp_dir(),
        )
        .unwrap();
        let beta =
            McpDocument::stdio("beta", "beta-cmd", Vec::new(), [], &std::env::temp_dir()).unwrap();
        let mut rename_calls = 0usize;

        let error = store
            .apply_batch_with_rename_in(
                &CategoryName::default(),
                vec![
                    McpWritePlan::replace(replacement),
                    McpWritePlan::create(beta),
                ],
                |from, to| {
                    rename_calls += 1;
                    if rename_calls == 3 {
                        return Err(io::Error::other("injected commit failure"));
                    }
                    fs::rename(from, to)
                },
            )
            .unwrap_err();

        assert!(matches!(error, McpError::BatchTransaction { .. }));
        assert_eq!(
            store
                .read("alpha")
                .unwrap()
                .document
                .summary()
                .target
                .as_deref(),
            Some("npx")
        );
        assert!(!home.join("mcps/default/beta").exists());
        let _ = fs::remove_dir_all(home);
    }
}
