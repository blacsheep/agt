use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use agt_config::{AgtPaths, CATEGORY_README_FILE, DefinitionCategories};
use agt_core::{CategoryName, PromptId};
use agt_storage::{check_no_symlinks_under, ensure_dir_under, write_atomic_under};
use sha2::{Digest, Sha256};

use crate::diagnostic::{PromptDiagnostic, PromptDiagnosticCategory};
use crate::error::{PromptError, Result};
use crate::name::validate_prompt_name;

pub const PROMPT_FILE: &str = "PROMPT.md";
pub const DEFAULT_PROMPT_NAME: &str = "default";
pub const DEFAULT_PROMPT_BODY: &str = "You are a helpful assistant.";
/// Seeded at init for a team lead that knows the framework workflow; editable and deletable.
pub const LEAD_PROMPT_NAME: &str = "lead";
pub const LEAD_PROMPT_BODY: &str = include_str!("lead.md");
/// Lead prompts earlier releases seeded, by [`fingerprint`]. A home still holding
/// one never edited it, so [`PromptStore::upgrade_builtin_prompts`] replaces it.
/// When changing `lead.md`, move `CURRENT_LEAD` here and update it.
const SHIPPED_LEAD: &[&str] = &[
    "f70d1e28ac858010ec39e3850e6fc39407d91cff513b50bf0dca5d57584efc4f",
    "3fa6bc01b7de179566e049a7cc394b0ddfa77c8f4282047d2236b01033e7505a",
    "754d6c7312c87f89581f02a4ef079221370ffa13e10001596285e4848cfd0f0d",
    "50e68fab04d1c5cc96cfbb534df68e89dae9a46840bf363822e444a66401b1c5",
    "862b949d40be3f03e06fec03288fa71fe524776b07aa9f606303a323783f946d",
    "35f959cd00858c2bc60021e37e5e56a48f8b3de0646c5897a20bea50fd652de0",
    "21ffb386f316134547464666ef943156f50f511ef3db458b161e7eb6c653ba74",
];
#[cfg(test)]
const CURRENT_LEAD: &str = "99010e58c304002f603af82f3eab64e027a6ed491beb782bee89c756d3b4b0df";

/// SHA-256 of a prompt with LF line endings and no trailing whitespace, as hex.
/// Windows checkouts and builds carry CRLF; both spellings are the same prompt.
fn fingerprint(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    format!("{:x}", Sha256::digest(text.trim_end().as_bytes()))
}
pub const PROMPT_DOCUMENT_MAX_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug)]
pub struct PromptStore {
    paths: AgtPaths,
}

impl PromptStore {
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

    pub fn prompts_dir(&self) -> PathBuf {
        self.paths.data_dir("prompts")
    }

    pub fn categories(&self) -> DefinitionCategories {
        DefinitionCategories::new(self.prompts_dir())
    }

    pub fn category_dir(&self, category: &CategoryName) -> PathBuf {
        self.paths.category_dir("prompts", category)
    }

    pub fn prompt_dir(&self, name: &str) -> Result<PathBuf> {
        self.prompt_dir_in(&CategoryName::default(), name)
    }

    pub fn prompt_dir_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        validate_prompt_name(name)?;
        Ok(self.category_dir(category).join(name))
    }

    pub fn prompt_file(&self, name: &str) -> Result<PathBuf> {
        self.prompt_file_in(&CategoryName::default(), name)
    }

    pub fn prompt_file_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        Ok(self.prompt_dir_in(category, name)?.join(PROMPT_FILE))
    }

    pub fn ensure_default_prompt(&self) -> Result<StoredPrompt> {
        let category = CategoryName::default();
        self.categories().ensure_default()?;
        let dir = self.prompt_dir_in(&category, DEFAULT_PROMPT_NAME)?;
        match fs::symlink_metadata(&dir) {
            Ok(_) => self.read_in(&category, DEFAULT_PROMPT_NAME),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                self.create_in(&category, DEFAULT_PROMPT_NAME, DEFAULT_PROMPT_BODY)
            }
            Err(source) => Err(PromptError::io(&dir, source)),
        }
    }

    /// Default plus the built-in lead prompt; existing prompts are never overwritten.
    pub fn ensure_builtin_prompts(&self) -> Result<StoredPrompt> {
        let default = self.ensure_default_prompt()?;
        let dir = self.prompt_dir_in(&CategoryName::default(), LEAD_PROMPT_NAME)?;
        match fs::symlink_metadata(&dir) {
            Ok(_) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                self.create(LEAD_PROMPT_NAME, LEAD_PROMPT_BODY)?;
            }
            Err(source) => return Err(PromptError::io(&dir, source)),
        }
        Ok(default)
    }

    /// Bring an unedited built-in lead prompt up to date. Returns whether it changed;
    /// an edited, missing or unreadable prompt is left as it is.
    pub fn upgrade_builtin_prompts(&self) -> Result<bool> {
        let category = CategoryName::default();
        let Ok(stored) = self.read_in(&category, LEAD_PROMPT_NAME) else {
            return Ok(false);
        };
        if !SHIPPED_LEAD.contains(&fingerprint(&stored.content).as_str()) {
            return Ok(false);
        }
        self.update_in(&category, LEAD_PROMPT_NAME, LEAD_PROMPT_BODY)?;
        Ok(true)
    }

    pub fn create(&self, name: &str, content: &str) -> Result<StoredPrompt> {
        self.create_in(&CategoryName::default(), name, content)
    }

    pub fn create_in(
        &self,
        category: &CategoryName,
        name: &str,
        content: &str,
    ) -> Result<StoredPrompt> {
        self.require_write_category(category)?;
        validate_content_size(self.prompt_file_in(category, name)?, content.len())?;
        let dir = self.prompt_dir_in(category, name)?;
        match fs::symlink_metadata(&dir) {
            Ok(_) => {
                return Err(PromptError::AlreadyExists {
                    name: name.to_string(),
                });
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(PromptError::io(&dir, source)),
        }

        ensure_dir_under(self.paths.home(), &dir).map_err(map_core_error)?;
        let file = dir.join(PROMPT_FILE);
        if let Err(error) = write_atomic_under(self.paths.home(), &file, content.as_bytes())
            .map_err(|source| PromptError::io(&file, source))
        {
            let _ = fs::remove_dir(&dir);
            return Err(error);
        }
        self.read_in(category, name)
    }

    pub fn read(&self, name: &str) -> Result<StoredPrompt> {
        self.read_in(&CategoryName::default(), name)
    }

    pub fn read_in(&self, category: &CategoryName, name: &str) -> Result<StoredPrompt> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        let dir = self.prompt_dir_in(category, name)?;
        check_no_symlinks_under(self.paths.home(), dir.join(PROMPT_FILE))
            .map_err(map_core_error)?;
        let dir_metadata = fs::symlink_metadata(&dir).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                PromptError::NotFound {
                    name: name.to_string(),
                }
            } else {
                PromptError::io(&dir, source)
            }
        })?;
        if !dir_metadata.file_type().is_dir() {
            return Err(PromptError::UnsupportedFileType { path: dir });
        }
        ensure_only_prompt_file(&dir)?;

        let file = dir.join(PROMPT_FILE);
        let metadata = fs::symlink_metadata(&file).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                PromptError::NotFound {
                    name: name.to_string(),
                }
            } else {
                PromptError::io(&file, source)
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(PromptError::UnsupportedFileType { path: file });
        }
        validate_content_size(&file, metadata.len() as usize)?;

        let bytes = fs::read(&file).map_err(|source| PromptError::io(&file, source))?;
        validate_content_size(&file, bytes.len())?;
        let content =
            String::from_utf8(bytes.clone()).map_err(|source| PromptError::InvalidUtf8 {
                path: file.clone(),
                source,
            })?;
        let modified = metadata
            .modified()
            .map_err(|source| PromptError::io(&file, source))?;
        let content_hash = format!("{:x}", Sha256::digest(&bytes));

        Ok(StoredPrompt {
            category: category.clone(),
            name: PromptId::new(name),
            path: file,
            content,
            content_hash,
            bytes: bytes.len() as u64,
            modified,
        })
    }

    pub fn update(&self, name: &str, content: &str) -> Result<StoredPrompt> {
        self.update_in(&CategoryName::default(), name, content)
    }

    pub fn update_in(
        &self,
        category: &CategoryName,
        name: &str,
        content: &str,
    ) -> Result<StoredPrompt> {
        let stored = self.read_in(category, name)?;
        validate_content_size(&stored.path, content.len())?;
        write_atomic_under(self.paths.home(), &stored.path, content.as_bytes())
            .map_err(|source| PromptError::io(&stored.path, source))?;
        self.read_in(category, name)
    }

    pub fn list(&self) -> Result<Vec<StoredPrompt>> {
        Ok(self.scan()?.items)
    }

    pub fn scan(&self) -> Result<PromptScan> {
        check_no_symlinks_under(self.paths.home(), self.paths.home()).map_err(map_core_error)?;
        let prompts_dir = self.prompts_dir();
        let metadata = match fs::symlink_metadata(&prompts_dir) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PromptScan::default());
            }
            Err(source) => return Err(PromptError::io(&prompts_dir, source)),
        };
        if !metadata.file_type().is_dir() {
            return Ok(PromptScan {
                items: Vec::new(),
                diagnostics: vec![PromptDiagnostic::new(
                    &prompts_dir,
                    None,
                    PromptDiagnosticCategory::UnsupportedFileType,
                    "prompts root 必须是普通目录",
                    "移除 symlink/特殊文件并创建普通 prompts 目录",
                )],
            });
        }

        let mut scan = PromptScan::default();
        for entry in read_dir_sorted(&prompts_dir)? {
            let path = entry.path();
            let category_text = entry.file_name().to_string_lossy().into_owned();
            let file_type = entry
                .file_type()
                .map_err(|source| PromptError::io(&path, source))?;
            if !file_type.is_dir() {
                scan.diagnostics.push(PromptDiagnostic::new(
                    &path,
                    Some(category_text.clone()),
                    if file_type.is_symlink() {
                        PromptDiagnosticCategory::UnsupportedFileType
                    } else {
                        PromptDiagnosticCategory::UnexpectedLayout
                    },
                    "prompts root 只能包含 <category>/ 目录",
                    "移动或删除该 entry，或改为规范 category 目录",
                ));
                continue;
            }
            let category = match CategoryName::new(&category_text) {
                Ok(category) => category,
                Err(error) => {
                    scan.diagnostics.push(PromptDiagnostic::new(
                        &path,
                        Some(category_text),
                        PromptDiagnosticCategory::InvalidIdentity,
                        error.to_string(),
                        "将目录名改为安全 category 名称",
                    ));
                    continue;
                }
            };
            self.scan_category(&category, &path, &mut scan)?;
        }
        scan.items.sort_by(|left, right| {
            (&left.category, left.name.as_str()).cmp(&(&right.category, right.name.as_str()))
        });
        scan.diagnostics
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(scan)
    }

    fn scan_category(
        &self,
        category: &CategoryName,
        category_dir: &Path,
        scan: &mut PromptScan,
    ) -> Result<()> {
        for entry in read_dir_sorted(category_dir)? {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == CATEGORY_README_FILE {
                continue;
            }
            let name_hint = Some(format!("{category}/{name}"));
            let file_type = entry
                .file_type()
                .map_err(|source| PromptError::io(&path, source))?;
            if !file_type.is_dir() {
                scan.diagnostics.push(PromptDiagnostic::new(
                    &path,
                    name_hint,
                    if file_type.is_symlink() {
                        PromptDiagnosticCategory::UnsupportedFileType
                    } else {
                        PromptDiagnosticCategory::UnexpectedLayout
                    },
                    "Prompt category 只能包含 <name>/PROMPT.md 目录",
                    "移动或删除该 entry",
                ));
                continue;
            }
            if let Err(error) = validate_prompt_name(&name) {
                scan.diagnostics
                    .push(PromptDiagnostic::from_error(&path, name_hint, &error));
                continue;
            }
            match self.read_in(category, &name) {
                Ok(prompt) => scan.items.push(prompt),
                Err(error) => scan
                    .diagnostics
                    .push(PromptDiagnostic::from_error(&path, name_hint, &error)),
            }
        }
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf> {
        self.delete_in(&CategoryName::default(), name)
    }

    pub fn delete_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        let dir = self.preflight_delete_in(category, name)?;
        fs::remove_dir_all(&dir).map_err(|source| PromptError::io(&dir, source))?;
        Ok(dir)
    }

    pub fn preflight_delete_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        if category.is_default() && name == DEFAULT_PROMPT_NAME {
            return Err(PromptError::ReservedDefault);
        }
        let stored = self.read_in(category, name)?;
        let dir = stored.path.parent().map(Path::to_path_buf).ok_or_else(|| {
            PromptError::UnexpectedEntry {
                path: stored.path.clone(),
            }
        })?;
        ensure_child_path(&self.category_dir(category), &dir)?;
        Ok(dir)
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredPrompt {
    pub category: CategoryName,
    pub name: PromptId,
    pub path: PathBuf,
    pub content: String,
    pub content_hash: String,
    pub bytes: u64,
    pub modified: SystemTime,
}

#[derive(Clone, Debug, Default)]
pub struct PromptScan {
    pub items: Vec<StoredPrompt>,
    pub diagnostics: Vec<PromptDiagnostic>,
}

fn validate_content_size(path: impl Into<PathBuf>, size: usize) -> Result<()> {
    if size as u64 > PROMPT_DOCUMENT_MAX_BYTES {
        Err(PromptError::TooLarge {
            path: path.into(),
            size: size as u64,
            max: PROMPT_DOCUMENT_MAX_BYTES,
        })
    } else {
        Ok(())
    }
}

fn ensure_only_prompt_file(dir: &Path) -> Result<()> {
    for entry in read_dir_sorted(dir)? {
        if entry.file_name() != PROMPT_FILE {
            return Err(PromptError::UnexpectedEntry { path: entry.path() });
        }
    }
    Ok(())
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| PromptError::io(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| PromptError::io(path, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<()> {
    let parent = parent
        .canonicalize()
        .map_err(|source| PromptError::io(parent, source))?;
    let child = child
        .canonicalize()
        .map_err(|source| PromptError::io(child, source))?;
    if child.starts_with(&parent) && child != parent {
        Ok(())
    } else {
        Err(PromptError::UnexpectedEntry { path: child })
    }
}

fn map_core_error(error: agt_core::AgtError) -> PromptError {
    match error {
        agt_core::AgtError::Io { path, source } => PromptError::io(path, source),
        other => PromptError::io(
            "prompts",
            std::io::Error::new(std::io::ErrorKind::InvalidInput, other.to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn test_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("agt-prompt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    #[cfg(unix)]
    #[test]
    fn updating_prompt_preserves_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let home = test_home("permissions");
        let store = PromptStore::new(&home);
        let prompt = store.create("private", "initial").unwrap();
        fs::set_permissions(&prompt.path, fs::Permissions::from_mode(0o600)).unwrap();
        store.update("private", "updated").unwrap();
        assert_eq!(
            fs::metadata(&prompt.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(store.read("private").unwrap().content, "updated");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn seeds_lead_prompt_without_overwriting_edits() {
        let home = test_home("lead-seed");
        let store = PromptStore::new(&home);
        store.ensure_builtin_prompts().unwrap();
        assert_eq!(
            store.read(LEAD_PROMPT_NAME).unwrap().content,
            LEAD_PROMPT_BODY
        );
        store.update(LEAD_PROMPT_NAME, "my lead").unwrap();
        store.ensure_builtin_prompts().unwrap();
        assert_eq!(store.read(LEAD_PROMPT_NAME).unwrap().content, "my lead");
        store.delete(LEAD_PROMPT_NAME).unwrap();
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn unedited_lead_prompts_from_earlier_releases_are_upgraded() {
        assert_eq!(
            fingerprint(LEAD_PROMPT_BODY),
            CURRENT_LEAD,
            "lead.md changed: add CURRENT_LEAD to SHIPPED_LEAD and update it"
        );
        let home = test_home("lead-upgrade");
        let store = PromptStore::new(&home);
        store.ensure_builtin_prompts().unwrap();
        assert!(!store.upgrade_builtin_prompts().unwrap());
        // The text an earlier release seeded, as a Windows build would have written it.
        let previous = include_str!("../tests/fixtures/lead-previous.md");
        let windows = previous.replace("\r\n", "\n").replace('\n', "\r\n");
        store.update(LEAD_PROMPT_NAME, &windows).unwrap();
        assert!(store.upgrade_builtin_prompts().unwrap());
        assert_eq!(
            store.read(LEAD_PROMPT_NAME).unwrap().content,
            LEAD_PROMPT_BODY
        );
        store.update(LEAD_PROMPT_NAME, "my lead").unwrap();
        assert!(!store.upgrade_builtin_prompts().unwrap());
        assert_eq!(store.read(LEAD_PROMPT_NAME).unwrap().content, "my lead");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn initializes_preserves_and_protects_default_prompt() {
        let home = test_home("default");
        let store = PromptStore::new(&home);

        let initial = store.ensure_default_prompt().unwrap();
        assert_eq!(initial.content, DEFAULT_PROMPT_BODY);
        assert_eq!(initial.path, home.join("prompts/default/default/PROMPT.md"));

        store.update(DEFAULT_PROMPT_NAME, "custom default").unwrap();
        assert_eq!(
            store.ensure_default_prompt().unwrap().content,
            "custom default"
        );
        assert!(matches!(
            store.delete(DEFAULT_PROMPT_NAME),
            Err(PromptError::ReservedDefault)
        ));

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn custom_categories_keep_same_named_prompts_isolated() {
        let home = test_home("categories");
        let store = PromptStore::new(&home);
        store.ensure_default_prompt().unwrap();
        let team = CategoryName::new("team").unwrap();
        store.create_category(&team).unwrap();

        let default = store.create("review", "default review").unwrap();
        let custom = store.create_in(&team, "review", "team review").unwrap();
        assert_ne!(default.content_hash, custom.content_hash);
        assert_eq!(store.read("review").unwrap().content, "default review");
        assert_eq!(
            store.read_in(&team, "review").unwrap().content,
            "team review"
        );
        assert_eq!(store.scan().unwrap().items.len(), 3);

        store.delete_in(&team, "review").unwrap();
        store.delete_category(&team).unwrap();
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn enforces_size_utf8_and_layout_diagnostics() {
        let home = test_home("diagnostics");
        let store = PromptStore::new(&home);
        store.ensure_default_prompt().unwrap();

        let exact = "x".repeat(PROMPT_DOCUMENT_MAX_BYTES as usize);
        store.create("exact", &exact).unwrap();
        assert!(matches!(
            store.create("large", &format!("{exact}x")),
            Err(PromptError::TooLarge { .. })
        ));

        let bad_dir = home.join("prompts/default/bad");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join(PROMPT_FILE), [0xff, 0xfe]).unwrap();
        let extra_dir = home.join("prompts/default/extra");
        fs::create_dir_all(&extra_dir).unwrap();
        fs::write(extra_dir.join(PROMPT_FILE), "ok").unwrap();
        fs::write(extra_dir.join("notes.txt"), "unexpected").unwrap();

        let scan = store.scan().unwrap();
        assert_eq!(scan.items.len(), 2);
        assert_eq!(scan.diagnostics.len(), 2);
        assert!(
            scan.diagnostics.iter().any(|diagnostic| {
                diagnostic.category == PromptDiagnosticCategory::InvalidContent
            })
        );
        assert!(scan.diagnostics.iter().any(|diagnostic| {
            diagnostic.category == PromptDiagnosticCategory::UnexpectedLayout
        }));

        let _ = fs::remove_dir_all(home);
    }
}
