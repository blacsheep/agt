use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use agt_config::{AgtPaths, CATEGORY_README_FILE, DefinitionCategories};
use agt_core::{CategoryName, PackageRef, PackageScope};
use agt_storage::{check_no_symlinks_under, ensure_dir, ensure_dir_under, write_atomic};
use tempfile::{Builder, TempDir};
use zip::ZipArchive;

use crate::config::{SKILL_FILE, SkillCreateOptions, SkillDocument, SkillMetadata};
use crate::diagnostic::{SkillDiagnostic, SkillDiagnosticCategory};
use crate::error::{ImportCleanupFailure, ImportCleanupReport, Result, SkillError};

pub const MAX_IMPORT_ENTRIES: usize = 4096;
pub const MAX_IMPORT_SINGLE_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_IMPORT_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_IMPORT_RELATIVE_DEPTH: usize = 16;
const MAX_FRONTMATTER_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct SkillStore {
    paths: AgtPaths,
}

impl SkillStore {
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

    pub fn skills_dir(&self) -> PathBuf {
        self.paths.data_dir("skills")
    }

    pub fn categories(&self) -> DefinitionCategories {
        DefinitionCategories::new(self.skills_dir())
    }

    pub fn category_dir(&self, category: &CategoryName) -> PathBuf {
        self.paths.category_dir("skills", category)
    }

    pub fn skill_dir(&self, package: &PackageRef) -> PathBuf {
        self.skill_dir_in(&CategoryName::default(), package)
    }

    pub fn skill_dir_in(&self, category: &CategoryName, package: &PackageRef) -> PathBuf {
        package_dir(&self.category_dir(category), package)
    }

    pub fn skill_file(&self, package: &PackageRef) -> PathBuf {
        self.skill_dir(package).join(SKILL_FILE)
    }

    pub fn skill_file_in(&self, category: &CategoryName, package: &PackageRef) -> PathBuf {
        self.skill_dir_in(category, package).join(SKILL_FILE)
    }

    pub fn create(&self, options: SkillCreateOptions) -> Result<StoredSkill> {
        self.create_in(&CategoryName::default(), options)
    }

    pub fn create_in(
        &self,
        category: &CategoryName,
        options: SkillCreateOptions,
    ) -> Result<StoredSkill> {
        let category_dir = self.require_write_category(category)?;
        let metadata = options.metadata()?;
        let package = metadata.package_ref()?;
        let document = SkillDocument::default_for(metadata)?;
        let final_dir = self.skill_dir_in(category, &package);
        self.ensure_missing(&package, &final_dir)?;

        let staging = staging_dir(&category_dir, "create")?;
        let staged_dir = staging.path().join("payload");
        ensure_skill_dirs(&staged_dir)?;
        self.write_document(&staged_dir.join(SKILL_FILE), &document)?;
        self.read_from_dir(Some(category), &package, &staged_dir)?;
        ensure_dir_under(
            self.paths.home(),
            final_dir.parent().unwrap_or_else(|| Path::new(".")),
        )
        .map_err(map_core_error)?;
        fs::rename(&staged_dir, &final_dir)
            .map_err(|source| SkillError::io(final_dir.clone(), source))?;
        self.read_in(category, &package)
    }

    pub fn import(&self, source: impl AsRef<Path>) -> Result<StoredSkill> {
        self.import_in(&CategoryName::default(), source)
    }

    pub fn import_in(
        &self,
        category: &CategoryName,
        source: impl AsRef<Path>,
    ) -> Result<StoredSkill> {
        let candidates = self.discover_import_candidates(source.as_ref())?;
        if candidates.len() != 1 {
            return Err(SkillError::InvalidImportSource {
                path: source.as_ref().to_path_buf(),
            });
        }
        self.import_candidates(category, candidates)?
            .into_iter()
            .next()
            .ok_or_else(|| SkillError::InvalidImportSource {
                path: source.as_ref().to_path_buf(),
            })
    }

    pub fn import_detected(&self, source: impl AsRef<Path>) -> Result<Vec<StoredSkill>> {
        self.import_detected_in(&CategoryName::default(), source)
    }

    pub fn import_detected_in(
        &self,
        category: &CategoryName,
        source: impl AsRef<Path>,
    ) -> Result<Vec<StoredSkill>> {
        let candidates = self.discover_import_candidates(source.as_ref())?;
        self.import_candidates(category, candidates)
    }

    pub fn add(&self, source: impl AsRef<Path>) -> Result<Vec<StoredSkill>> {
        self.add_in(&CategoryName::default(), source)
    }

    pub fn add_in(
        &self,
        category: &CategoryName,
        source: impl AsRef<Path>,
    ) -> Result<Vec<StoredSkill>> {
        let source = source.as_ref();
        match fs::symlink_metadata(source) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                self.import_detected_in(category, source)
            }
            Ok(metadata) if metadata.file_type().is_file() && is_zip_path(source) => {
                self.import_zip(category, source)
            }
            Ok(_) | Err(_) => Err(SkillError::InvalidImportSource {
                path: source.to_path_buf(),
            }),
        }
    }

    pub fn read(&self, package: &PackageRef) -> Result<StoredSkill> {
        self.read_in(&CategoryName::default(), package)
    }

    pub fn read_in(&self, category: &CategoryName, package: &PackageRef) -> Result<StoredSkill> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        let dir = self.skill_dir_in(category, package);
        check_no_symlinks_under(self.paths.home(), &dir).map_err(map_core_error)?;
        self.read_from_dir(Some(category), package, &dir)
    }

    pub fn read_metadata(&self, package: &PackageRef) -> Result<StoredSkillMetadata> {
        self.read_metadata_in(&CategoryName::default(), package)
    }

    pub fn read_metadata_in(
        &self,
        category: &CategoryName,
        package: &PackageRef,
    ) -> Result<StoredSkillMetadata> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        let dir = self.skill_dir_in(category, package);
        check_no_symlinks_under(self.paths.home(), &dir).map_err(map_core_error)?;
        self.read_metadata_from_dir(Some(category), package, &dir)
    }

    pub fn read_metadata_at(
        &self,
        package: &PackageRef,
        dir: impl AsRef<Path>,
    ) -> Result<StoredSkillMetadata> {
        self.read_metadata_from_dir(None, package, dir.as_ref())
    }

    pub fn scan(&self) -> Result<SkillScan> {
        check_no_symlinks_under(self.paths.home(), self.paths.home()).map_err(map_core_error)?;
        let root = self.skills_dir();
        let root_metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(SkillScan::default());
            }
            Err(source) => return Err(SkillError::io(&root, source)),
        };
        if !root_metadata.file_type().is_dir() {
            return Ok(SkillScan {
                items: Vec::new(),
                diagnostics: vec![SkillDiagnostic::new(
                    &root,
                    None,
                    SkillDiagnosticCategory::UnsupportedFileType,
                    "skills root 必须是普通目录",
                    "移除 symlink/特殊文件并创建普通 skills 目录",
                )],
            });
        }
        let mut scan = SkillScan::default();
        for entry in read_dir_sorted(&root)? {
            let category_text = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|source| SkillError::io(path.clone(), source))?;
            if !file_type.is_dir() {
                scan.diagnostics.push(SkillDiagnostic::new(
                    &path,
                    Some(category_text.clone()),
                    SkillDiagnosticCategory::UnexpectedLayout,
                    "skills root 只能包含 <category>/ 目录",
                    "移动或删除该文件，或改为规范 category 目录",
                ));
                continue;
            }
            let category = match CategoryName::new(&category_text) {
                Ok(category) => category,
                Err(error) => {
                    scan.diagnostics.push(SkillDiagnostic::new(
                        &path,
                        Some(category_text),
                        SkillDiagnosticCategory::InvalidIdentity,
                        error.to_string(),
                        "将目录名改为安全 category 名称",
                    ));
                    continue;
                }
            };
            self.scan_category(&category, &path, &mut scan)?;
        }
        scan.items.sort_by(|left, right| {
            (&left.category, &left.package).cmp(&(&right.category, &right.package))
        });
        scan.diagnostics
            .sort_by(|left, right| left.path.cmp(&right.path));
        Ok(scan)
    }

    fn scan_category(
        &self,
        category: &CategoryName,
        category_dir: &Path,
        scan: &mut SkillScan,
    ) -> Result<()> {
        for entry in read_dir_sorted(category_dir)? {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == CATEGORY_README_FILE || name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|source| SkillError::io(path.clone(), source))?;
            if !file_type.is_dir() {
                scan.diagnostics.push(SkillDiagnostic::new(
                    &path,
                    Some(format!("{category}/{name}")),
                    SkillDiagnosticCategory::UnexpectedLayout,
                    "skill category 只能包含 Skill 目录或 @scope container",
                    "移动或删除该文件",
                ));
                continue;
            }
            if let Some(raw_scope) = name.strip_prefix('@') {
                self.scan_scope_container(category, raw_scope, &path, scan)?;
            } else {
                match name.parse::<PackageRef>() {
                    Ok(package) => self.scan_package_dir(category, package, path, scan),
                    Err(error) => scan.diagnostics.push(SkillDiagnostic::new(
                        &path,
                        Some(format!("{category}/{name}")),
                        SkillDiagnosticCategory::InvalidIdentity,
                        error.to_string(),
                        "将目录改为 lowercase kebab-case",
                    )),
                }
            }
        }
        Ok(())
    }

    pub fn delete(&self, package: &PackageRef) -> Result<PathBuf> {
        self.delete_in(&CategoryName::default(), package)
    }

    pub fn delete_in(&self, category: &CategoryName, package: &PackageRef) -> Result<PathBuf> {
        let dir = self.skill_dir_in(category, package);
        self.read_metadata_in(category, package)?;
        ensure_child_path(&self.category_dir(category), &dir)?;
        fs::remove_dir_all(&dir).map_err(|source| SkillError::io(dir.clone(), source))?;
        if package.is_scoped()
            && let Some(parent) = dir.parent()
            && fs::read_dir(parent)
                .map_err(|source| SkillError::io(parent, source))?
                .next()
                .is_none()
        {
            fs::remove_dir(parent).map_err(|source| SkillError::io(parent, source))?;
        }
        Ok(dir)
    }

    fn scan_scope_container(
        &self,
        category: &CategoryName,
        raw_scope: &str,
        path: &Path,
        scan: &mut SkillScan,
    ) -> Result<()> {
        let scope = match PackageScope::try_from(raw_scope) {
            Ok(scope) => scope,
            Err(error) => {
                scan.diagnostics.push(SkillDiagnostic::new(
                    path,
                    Some(format!("{category}/@{raw_scope}")),
                    SkillDiagnosticCategory::InvalidIdentity,
                    error.to_string(),
                    "将 scope container 改为 @<lowercase-kebab-scope>",
                ));
                return Ok(());
            }
        };
        if matches!(
            fs::symlink_metadata(path.join(SKILL_FILE)),
            Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink()
        ) {
            scan.diagnostics.push(SkillDiagnostic::new(
                path.join(SKILL_FILE),
                Some(format!("{category}/@{raw_scope}")),
                SkillDiagnosticCategory::UnexpectedLayout,
                "scope container 不能直接作为 Skill",
                "把 SKILL.md 移到 @scope/<name>/SKILL.md",
            ));
        }
        for entry in read_dir_sorted(path)? {
            let child = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == SKILL_FILE || name.starts_with('.') {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|source| SkillError::io(child.clone(), source))?;
            if !file_type.is_dir() {
                scan.diagnostics.push(SkillDiagnostic::new(
                    &child,
                    Some(format!("{category}/@{raw_scope}/{name}")),
                    SkillDiagnosticCategory::UnexpectedLayout,
                    "scope container 只能包含 Skill 目录",
                    "移动或删除该文件",
                ));
                continue;
            }
            let package_text = format!("@{scope}/{name}");
            match package_text.parse::<PackageRef>() {
                Ok(package) => self.scan_package_dir(category, package, child, scan),
                Err(error) => scan.diagnostics.push(SkillDiagnostic::new(
                    &child,
                    Some(format!("{category}/{package_text}")),
                    SkillDiagnosticCategory::InvalidIdentity,
                    error.to_string(),
                    "将 Skill leaf 目录改为 lowercase kebab-case",
                )),
            }
        }
        Ok(())
    }

    fn scan_package_dir(
        &self,
        category: &CategoryName,
        package: PackageRef,
        dir: PathBuf,
        scan: &mut SkillScan,
    ) {
        match self.read_metadata_from_dir(Some(category), &package, &dir) {
            Ok(stored) => scan.items.push(stored),
            Err(error) => scan.diagnostics.push(SkillDiagnostic::from_error(
                &dir,
                Some(format!("{category}/{package}")),
                &error,
            )),
        }
    }

    fn read_from_dir(
        &self,
        category: Option<&CategoryName>,
        package: &PackageRef,
        dir: &Path,
    ) -> Result<StoredSkill> {
        ensure_regular_directory(dir, package)?;
        let file = dir.join(SKILL_FILE);
        ensure_regular_file(&file, package)?;
        let document = self.read_document_file(&file)?;
        let actual = document.metadata.package_ref()?;
        ensure_identity(package, &actual, dir)?;
        Ok(StoredSkill {
            category: category.cloned(),
            package: package.clone(),
            path: file,
            dir: dir.to_path_buf(),
            document,
        })
    }

    fn read_metadata_from_dir(
        &self,
        category: Option<&CategoryName>,
        package: &PackageRef,
        dir: &Path,
    ) -> Result<StoredSkillMetadata> {
        ensure_regular_directory(dir, package)?;
        let file = dir.join(SKILL_FILE);
        ensure_regular_file(&file, package)?;
        let metadata = self.read_metadata_file(&file)?;
        let actual = metadata.package_ref()?;
        ensure_identity(package, &actual, dir)?;
        Ok(StoredSkillMetadata {
            category: category.cloned(),
            package: package.clone(),
            path: file,
            dir: dir.to_path_buf(),
            metadata,
        })
    }

    fn read_document_file(&self, file: &Path) -> Result<SkillDocument> {
        let metadata = fs::symlink_metadata(file).map_err(|source| SkillError::io(file, source))?;
        if !metadata.file_type().is_file() {
            return Err(SkillError::UnsupportedFileType {
                path: file.to_path_buf(),
            });
        }
        if metadata.len() > MAX_IMPORT_SINGLE_FILE_BYTES {
            return Err(SkillError::LimitExceeded {
                path: file.to_path_buf(),
                message: format!(
                    "单文件 {} bytes，超过 {} bytes",
                    metadata.len(),
                    MAX_IMPORT_SINGLE_FILE_BYTES
                ),
            });
        }
        let text = fs::read_to_string(file).map_err(|source| SkillError::io(file, source))?;
        SkillDocument::parse(file, &text)
    }

    fn read_metadata_file(&self, file: &Path) -> Result<SkillMetadata> {
        let handle = File::open(file).map_err(|source| SkillError::io(file, source))?;
        let mut reader = BufReader::new(handle);
        let mut text = String::new();
        let mut line = String::new();
        let mut line_count = 0usize;
        loop {
            line.clear();
            let bytes = reader
                .read_line(&mut line)
                .map_err(|source| SkillError::io(file, source))?;
            if bytes == 0 {
                break;
            }
            line_count += 1;
            text.push_str(&line);
            if text.len() > MAX_FRONTMATTER_BYTES {
                return Err(SkillError::LimitExceeded {
                    path: file.to_path_buf(),
                    message: format!("frontmatter 超过 {MAX_FRONTMATTER_BYTES} bytes"),
                });
            }
            if line_count > 1 && line.trim_end_matches(&['\r', '\n'][..]) == "---" {
                break;
            }
        }
        SkillDocument::parse_metadata(file, &text)
    }

    /// Replace the document while retaining the package's ancillary files and identity.
    pub fn update_in(
        &self,
        category: &CategoryName,
        package: &PackageRef,
        text: &str,
    ) -> Result<StoredSkill> {
        let stored = self.read_in(category, package)?;
        let document = SkillDocument::parse(&stored.path, text)?;
        if document.metadata.package_ref()? != *package {
            return Err(SkillError::InvalidConfig {
                message: "skill package identity mismatch".into(),
            });
        }
        let rendered = document.render()?;
        if rendered.len() as u64 > crate::SKILL_DOCUMENT_MAX_BYTES {
            return Err(SkillError::LimitExceeded {
                path: stored.path.clone(),
                message: format!(
                    "Skill document exceeds {} bytes",
                    crate::SKILL_DOCUMENT_MAX_BYTES
                ),
            });
        }
        agt_storage::write_atomic_under(self.paths.home(), &stored.path, rendered.as_bytes())
            .map_err(|source| SkillError::io(&stored.path, source))?;
        self.read_in(category, package)
    }

    fn write_document(&self, file: &Path, document: &SkillDocument) -> Result<()> {
        let text = document.render()?;
        write_atomic(file, text.as_bytes()).map_err(|source| SkillError::io(file, source))
    }

    fn ensure_missing(&self, package: &PackageRef, target: &Path) -> Result<()> {
        check_no_symlinks_under(self.paths.home(), target).map_err(map_core_error)?;
        match fs::symlink_metadata(target) {
            Ok(_) => {
                return Err(SkillError::AlreadyExists {
                    name: package.to_string(),
                });
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(SkillError::io(target, source)),
        }
        Ok(())
    }

    fn discover_import_candidates(&self, source: &Path) -> Result<Vec<SkillImportCandidate>> {
        match fs::symlink_metadata(source) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(SkillError::UnsupportedFileType {
                    path: source.to_path_buf(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(SkillError::InvalidImportSource {
                    path: source.to_path_buf(),
                });
            }
            Err(error) => return Err(SkillError::io(source, error)),
        }
        if has_regular_skill_file(source)? {
            return Ok(vec![self.candidate_from_dir(source, None)?]);
        }

        let mut candidates = Vec::new();
        for entry in read_dir_sorted(source)? {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|source_error| SkillError::io(&path, source_error))?
                .is_dir()
            {
                continue;
            }
            if let Some(raw_scope) = name.strip_prefix('@') {
                let scope = PackageScope::try_from(raw_scope).map_err(|source_error| {
                    SkillError::InvalidConfig {
                        message: format!("导入 scope container `{name}` 无效：{source_error}"),
                    }
                })?;
                for child in read_dir_sorted(&path)? {
                    let child_path = child.path();
                    if !child
                        .file_type()
                        .map_err(|source_error| SkillError::io(&child_path, source_error))?
                        .is_dir()
                    {
                        continue;
                    }
                    let leaf = child.file_name().to_string_lossy().to_string();
                    let expected = format!("@{scope}/{leaf}").parse::<PackageRef>().map_err(
                        |source_error| SkillError::InvalidConfig {
                            message: format!("导入 package ref 无效：{source_error}"),
                        },
                    )?;
                    if has_regular_skill_file(&child_path)? {
                        candidates.push(self.candidate_from_dir(&child_path, Some(&expected))?);
                    }
                }
            } else if has_regular_skill_file(&path)? {
                candidates.push(self.candidate_from_dir(&path, None)?);
            }
        }
        if candidates.is_empty() {
            return Err(SkillError::InvalidImportSource {
                path: source.to_path_buf(),
            });
        }
        candidates.sort_by(|left, right| left.package.cmp(&right.package));
        ensure_unique_candidate_packages(&candidates)?;
        Ok(candidates)
    }

    fn candidate_from_dir(
        &self,
        dir: &Path,
        expected: Option<&PackageRef>,
    ) -> Result<SkillImportCandidate> {
        let document = self.read_document_file(&dir.join(SKILL_FILE))?;
        let package = document.metadata.package_ref()?;
        let leaf = dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SkillError::InvalidImportSource {
                path: dir.to_path_buf(),
            })?;
        if leaf != package.name().as_str() {
            return Err(SkillError::InvalidConfig {
                message: format!(
                    "导入目录 leaf `{leaf}` 与 SKILL.md name `{}` 不一致",
                    package.name()
                ),
            });
        }
        if let Some(expected) = expected {
            ensure_identity(expected, &package, dir)?;
        }
        Ok(SkillImportCandidate {
            source_dir: dir.to_path_buf(),
            package,
        })
    }

    fn import_candidates(
        &self,
        category: &CategoryName,
        candidates: Vec<SkillImportCandidate>,
    ) -> Result<Vec<StoredSkill>> {
        let category_dir = self.require_write_category(category)?;
        let mut seen = BTreeSet::new();
        for candidate in &candidates {
            if !seen.insert(candidate.package.clone()) {
                return Err(SkillError::InvalidConfig {
                    message: format!("导入源中存在重复 package：{}", candidate.package),
                });
            }
            self.ensure_missing(
                &candidate.package,
                &self.skill_dir_in(category, &candidate.package),
            )?;
        }

        let mut preflight_budget = ImportBudget::default();
        for candidate in &candidates {
            validate_dir_bounded(&candidate.source_dir, &mut preflight_budget)?;
        }

        let staging = staging_dir(&category_dir, "import")?;
        let mut budget = ImportBudget::default();
        let mut staged = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.iter().enumerate() {
            let staged_dir = staging.path().join(format!("item-{index}"));
            copy_dir_bounded(&candidate.source_dir, &staged_dir, &mut budget)?;
            self.read_from_dir(Some(category), &candidate.package, &staged_dir)?;
            staged.push((candidate.package.clone(), staged_dir));
        }

        commit_staged_items(&category_dir, staged, &mut StdCommitOps)?;

        candidates
            .iter()
            .map(|candidate| self.read_in(category, &candidate.package))
            .collect()
    }

    fn import_zip(&self, category: &CategoryName, source: &Path) -> Result<Vec<StoredSkill>> {
        let temp_dir =
            TempDir::new().map_err(|source_error| SkillError::io(source, source_error))?;
        extract_zip(source, temp_dir.path())?;
        self.import_detected_in(category, temp_dir.path())
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
pub struct StoredSkill {
    pub category: Option<CategoryName>,
    pub package: PackageRef,
    pub path: PathBuf,
    pub dir: PathBuf,
    pub document: SkillDocument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSkillMetadata {
    pub category: Option<CategoryName>,
    pub package: PackageRef,
    pub path: PathBuf,
    pub dir: PathBuf,
    pub metadata: SkillMetadata,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SkillScan {
    pub items: Vec<StoredSkillMetadata>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

#[derive(Clone, Debug)]
struct SkillImportCandidate {
    source_dir: PathBuf,
    package: PackageRef,
}

#[derive(Default)]
struct ImportBudget {
    entries: usize,
    total_bytes: u64,
}

fn package_dir(root: &Path, package: &PackageRef) -> PathBuf {
    match package.scope() {
        Some(scope) => root
            .join(format!("@{}", scope.as_str()))
            .join(package.name().as_str()),
        None => root.join(package.name().as_str()),
    }
}

fn staging_dir(root: &Path, operation: &str) -> Result<TempDir> {
    Builder::new()
        .prefix(&format!(".staging-{operation}-"))
        .tempdir_in(root)
        .map_err(|source| SkillError::io(root, source))
}

fn ensure_skill_dirs(dir: &Path) -> Result<()> {
    ensure_dir(dir).map_err(map_core_error)?;
    ensure_dir(dir.join("references")).map_err(map_core_error)?;
    ensure_dir(dir.join("scripts")).map_err(map_core_error)?;
    ensure_dir(dir.join("assets")).map_err(map_core_error)?;
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent).map_err(map_core_error)?;
    }
    Ok(())
}

fn ensure_regular_directory(dir: &Path, package: &PackageRef) -> Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(SkillError::UnsupportedFileType {
            path: dir.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Err(SkillError::NotFound {
            name: package.to_string(),
        }),
        Err(source) => Err(SkillError::io(dir, source)),
    }
}

fn ensure_regular_file(file: &Path, package: &PackageRef) -> Result<()> {
    match fs::symlink_metadata(file) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(SkillError::UnsupportedFileType {
            path: file.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Err(SkillError::NotFound {
            name: package.to_string(),
        }),
        Err(source) => Err(SkillError::io(file, source)),
    }
}

fn has_regular_skill_file(dir: &Path) -> Result<bool> {
    let file = dir.join(SKILL_FILE);
    match fs::symlink_metadata(&file) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(SkillError::UnsupportedFileType { path: file }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(SkillError::io(file, source)),
    }
}

fn ensure_identity(expected: &PackageRef, actual: &PackageRef, path: &Path) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(SkillError::InvalidConfig {
            message: format!(
                "路径 {} 表示 package `{expected}`，但 SKILL.md 声明 `{actual}`",
                path.display()
            ),
        })
    }
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| SkillError::io(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| SkillError::io(path, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn copy_dir_bounded(source: &Path, target: &Path, budget: &mut ImportBudget) -> Result<()> {
    process_dir_bounded(source, Some(target), 0, budget)
}

fn validate_dir_bounded(source: &Path, budget: &mut ImportBudget) -> Result<()> {
    process_dir_bounded(source, None, 0, budget)
}

fn process_dir_bounded(
    source: &Path,
    target: Option<&Path>,
    depth: usize,
    budget: &mut ImportBudget,
) -> Result<()> {
    if depth > MAX_IMPORT_RELATIVE_DEPTH {
        return Err(SkillError::LimitExceeded {
            path: source.to_path_buf(),
            message: format!("相对深度超过 {MAX_IMPORT_RELATIVE_DEPTH}"),
        });
    }
    if let Some(target) = target {
        ensure_dir(target).map_err(map_core_error)?;
    }
    for entry in read_dir_sorted(source)? {
        budget.entries += 1;
        if budget.entries > MAX_IMPORT_ENTRIES {
            return Err(SkillError::LimitExceeded {
                path: source.to_path_buf(),
                message: format!("entry 数超过 {MAX_IMPORT_ENTRIES}"),
            });
        }
        let source_path = entry.path();
        let target_path = target.map(|target| target.join(entry.file_name()));
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|source| SkillError::io(&source_path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(SkillError::UnsupportedFileType { path: source_path });
        }
        if metadata.file_type().is_dir() {
            process_dir_bounded(&source_path, target_path.as_deref(), depth + 1, budget)?;
        } else if metadata.file_type().is_file() {
            let size = metadata.len();
            check_file_budget(&source_path, size, budget)?;
            if let Some(target_path) = target_path {
                fs::copy(&source_path, &target_path)
                    .map_err(|source| SkillError::io(&source_path, source))?;
            }
        } else {
            return Err(SkillError::UnsupportedFileType { path: source_path });
        }
    }
    Ok(())
}

fn check_file_budget(path: &Path, size: u64, budget: &mut ImportBudget) -> Result<()> {
    if size > MAX_IMPORT_SINGLE_FILE_BYTES {
        return Err(SkillError::LimitExceeded {
            path: path.to_path_buf(),
            message: format!("单文件 {size} bytes，超过 {MAX_IMPORT_SINGLE_FILE_BYTES}"),
        });
    }
    budget.total_bytes = budget.total_bytes.saturating_add(size);
    if budget.total_bytes > MAX_IMPORT_TOTAL_BYTES {
        return Err(SkillError::LimitExceeded {
            path: path.to_path_buf(),
            message: format!(
                "总展开 {} bytes，超过 {} bytes",
                budget.total_bytes, MAX_IMPORT_TOTAL_BYTES
            ),
        });
    }
    Ok(())
}

fn ensure_unique_candidate_packages(candidates: &[SkillImportCandidate]) -> Result<()> {
    for window in candidates.windows(2) {
        if window[0].package == window[1].package {
            return Err(SkillError::InvalidConfig {
                message: format!("导入源中存在重复 package：{}", window[0].package),
            });
        }
    }
    Ok(())
}

fn is_zip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn extract_zip(source: &Path, target: &Path) -> Result<()> {
    let file = File::open(source).map_err(|source_error| SkillError::io(source, source_error))?;
    let mut archive = ZipArchive::new(file).map_err(|source_error| SkillError::InvalidArchive {
        path: source.to_path_buf(),
        message: source_error.to_string(),
    })?;
    let mut budget = ImportBudget::default();
    let mut outputs = HashSet::new();
    for index in 0..archive.len() {
        budget.entries += 1;
        if budget.entries > MAX_IMPORT_ENTRIES {
            return Err(SkillError::LimitExceeded {
                path: source.to_path_buf(),
                message: format!("zip entry 数超过 {MAX_IMPORT_ENTRIES}"),
            });
        }
        let mut entry =
            archive
                .by_index(index)
                .map_err(|source_error| SkillError::InvalidArchive {
                    path: source.to_path_buf(),
                    message: source_error.to_string(),
                })?;
        let Some(enclosed_name) = entry.enclosed_name() else {
            return Err(SkillError::InvalidArchive {
                path: source.to_path_buf(),
                message: format!(
                    "包含不安全路径：{}",
                    String::from_utf8_lossy(entry.name_raw())
                ),
            });
        };
        let relative = enclosed_name.to_path_buf();
        if relative.components().count() > MAX_IMPORT_RELATIVE_DEPTH {
            return Err(SkillError::LimitExceeded {
                path: relative,
                message: format!("zip 相对深度超过 {MAX_IMPORT_RELATIVE_DEPTH}"),
            });
        }
        track_zip_output(&mut outputs, &relative, source)?;
        let kind = entry.unix_mode().unwrap_or_default() & 0o170000;
        if (entry.is_dir() && kind != 0 && kind != 0o040000)
            || (!entry.is_dir() && kind != 0 && kind != 0o100000)
        {
            return Err(SkillError::UnsupportedFileType { path: relative });
        }
        let output_path = target.join(&relative);
        if entry.is_dir() {
            ensure_dir(&output_path).map_err(map_core_error)?;
            continue;
        }
        check_zip_declared_size(&relative, entry.size(), &budget)?;
        ensure_parent(&output_path)?;
        let mut output = File::create(&output_path)
            .map_err(|source_error| SkillError::io(&output_path, source_error))?;
        let remaining_total = MAX_IMPORT_TOTAL_BYTES.saturating_sub(budget.total_bytes);
        let allowed = MAX_IMPORT_SINGLE_FILE_BYTES.min(remaining_total);
        let copied = io::copy(
            &mut entry.by_ref().take(allowed.saturating_add(1)),
            &mut output,
        )
        .map_err(|source_error| SkillError::io(&output_path, source_error))?;
        if copied > allowed {
            return Err(SkillError::LimitExceeded {
                path: relative,
                message: format!(
                    "实际展开超过剩余限制 {allowed} bytes（单文件上限 {MAX_IMPORT_SINGLE_FILE_BYTES}，总展开上限 {MAX_IMPORT_TOTAL_BYTES}）"
                ),
            });
        }
        budget.total_bytes += copied;
    }
    Ok(())
}

fn check_zip_declared_size(path: &Path, size: u64, budget: &ImportBudget) -> Result<()> {
    if size > MAX_IMPORT_SINGLE_FILE_BYTES {
        return Err(SkillError::LimitExceeded {
            path: path.to_path_buf(),
            message: format!("单文件 {size} bytes，超过 {MAX_IMPORT_SINGLE_FILE_BYTES}"),
        });
    }
    if budget.total_bytes.saturating_add(size) > MAX_IMPORT_TOTAL_BYTES {
        return Err(SkillError::LimitExceeded {
            path: path.to_path_buf(),
            message: format!(
                "总展开至少 {} bytes，超过 {} bytes",
                budget.total_bytes.saturating_add(size),
                MAX_IMPORT_TOTAL_BYTES
            ),
        });
    }
    Ok(())
}

trait CommitOps {
    fn rename(&mut self, source: &Path, target: &Path) -> io::Result<()>;
    fn remove_dir_all(&mut self, path: &Path) -> io::Result<()>;
}

struct StdCommitOps;

impl CommitOps for StdCommitOps {
    fn rename(&mut self, source: &Path, target: &Path) -> io::Result<()> {
        fs::rename(source, target)
    }

    fn remove_dir_all(&mut self, path: &Path) -> io::Result<()> {
        fs::remove_dir_all(path)
    }
}

fn commit_staged_items(
    skills_dir: &Path,
    staged: Vec<(PackageRef, PathBuf)>,
    operations: &mut impl CommitOps,
) -> Result<Vec<PathBuf>> {
    let mut committed = Vec::new();
    for (package, staged_dir) in staged {
        let final_dir = package_dir(skills_dir, &package);
        let commit = ensure_dir_under(skills_dir, final_dir.parent().unwrap_or(skills_dir))
            .map_err(map_core_error)
            .and_then(|_| {
                operations
                    .rename(&staged_dir, &final_dir)
                    .map_err(|source| SkillError::io(&final_dir, source))
            });
        if let Err(primary) = commit {
            return Err(SkillError::ImportTransaction {
                primary: Box::new(primary),
                cleanup: rollback_paths(&committed, operations),
            });
        }
        committed.push(final_dir);
    }
    Ok(committed)
}

fn rollback_paths(paths: &[PathBuf], operations: &mut impl CommitOps) -> ImportCleanupReport {
    let mut failures = Vec::new();
    for path in paths.iter().rev() {
        if let Err(source) = operations.remove_dir_all(path) {
            failures.push(ImportCleanupFailure {
                path: path.clone(),
                source,
            });
        }
    }
    ImportCleanupReport { failures }
}

fn track_zip_output(outputs: &mut HashSet<PathBuf>, relative: &Path, source: &Path) -> Result<()> {
    if outputs.insert(relative.to_path_buf()) {
        Ok(())
    } else {
        Err(SkillError::InvalidArchive {
            path: source.to_path_buf(),
            message: format!("包含重复输出路径：{}", relative.display()),
        })
    }
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<()> {
    let parent = parent
        .canonicalize()
        .map_err(|source| SkillError::io(parent, source))?;
    let child = child
        .canonicalize()
        .map_err(|source| SkillError::io(child, source))?;
    if child.starts_with(&parent) && child != parent {
        Ok(())
    } else {
        Err(SkillError::InvalidConfig {
            message: "skill 删除路径越界".to_string(),
        })
    }
}

fn map_core_error(error: agt_core::AgtError) -> SkillError {
    match error {
        agt_core::AgtError::Io { path, source } => SkillError::io(path, source),
        other => SkillError::InvalidConfig {
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use zip::write::SimpleFileOptions;

    fn test_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("agt-skill-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    fn package(value: &str) -> PackageRef {
        value.parse().expect("package")
    }

    #[test]
    fn unscoped_and_scoped_crud_use_canonical_paths() {
        let home = test_home("crud");
        let store = SkillStore::new(&home);
        for (reference, relative) in [
            ("code-review", "skills/default/code-review/SKILL.md"),
            ("@acme/debug-db", "skills/default/@acme/debug-db/SKILL.md"),
        ] {
            let stored = store
                .create(SkillCreateOptions::new(reference, "Description").unwrap())
                .unwrap();
            assert_eq!(stored.package.to_string(), reference);
            assert_eq!(stored.path, home.join(relative));
            assert_eq!(store.read(&package(reference)).unwrap(), stored);
        }
        let scan = store.scan().unwrap();
        assert_eq!(scan.items.len(), 2);
        assert!(scan.diagnostics.is_empty());
        assert_eq!(
            store.delete(&package("@acme/debug-db")).unwrap(),
            home.join("skills/default/@acme/debug-db")
        );
        assert!(!home.join("skills/default/@acme").exists());
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scan_reports_invalid_items_without_hiding_valid_skills() {
        let home = test_home("scan-diagnostics");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("valid", "Valid").unwrap())
            .unwrap();
        fs::create_dir_all(home.join("skills/default/BAD_name")).unwrap();
        fs::write(home.join("skills/default/BAD_name/SKILL.md"), "bad").unwrap();

        let scan = store.scan().unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].package.to_string(), "valid");
        assert_eq!(scan.diagnostics.len(), 1);
        assert_eq!(
            scan.diagnostics[0].category,
            SkillDiagnosticCategory::InvalidIdentity
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn import_detected_supports_scoped_root_and_preserves_files() {
        let source = test_home("source");
        let target = test_home("target");
        let source_store = SkillStore::new(&source);
        source_store
            .create(SkillCreateOptions::new("@acme/review", "Review").unwrap())
            .unwrap();
        fs::write(
            source.join("skills/default/@acme/review/references/guide.md"),
            "guide",
        )
        .unwrap();

        let target_store = SkillStore::new(&target);
        let imported = target_store
            .import_detected(source.join("skills/default"))
            .unwrap();
        assert_eq!(imported[0].package.to_string(), "@acme/review");
        assert!(
            target
                .join("skills/default/@acme/review/references/guide.md")
                .is_file()
        );
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn import_preserves_original_skill_document_bytes() {
        let source = test_home("preserve-source");
        let target = test_home("preserve-target");
        let skill_dir = source.join("preserved");
        fs::create_dir_all(&skill_dir).unwrap();
        let original = b"---\nname: preserved\ndescription: Preserve bytes.\nfuture-field: keep-me\n---\n# Preserved\n\nBody.  \n";
        fs::write(skill_dir.join(SKILL_FILE), original).unwrap();

        let store = SkillStore::new(&target);
        store.import(&skill_dir).unwrap();

        assert_eq!(
            fs::read(target.join("skills/default/preserved/SKILL.md")).unwrap(),
            original
        );
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn add_imports_zip_and_tracks_duplicate_paths() {
        let home = test_home("zip-target");
        let archive = test_home("zip-source").join("skill.zip");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        write_skill_zip(&archive);
        let store = SkillStore::new(&home);
        let imported = store.add(&archive).unwrap();
        assert_eq!(imported[0].package.to_string(), "zipped");

        let mut outputs = HashSet::new();
        let duplicate = Path::new("zipped/SKILL.md");
        track_zip_output(&mut outputs, duplicate, &archive).unwrap();
        assert!(matches!(
            track_zip_output(&mut outputs, duplicate, &archive),
            Err(SkillError::InvalidArchive { .. })
        ));
        let _ = fs::remove_dir_all(home);
        let _ = fs::remove_dir_all(archive.parent().unwrap());
    }

    #[test]
    fn zip_import_rejects_traversal_duplicate_and_excessive_depth() {
        let root = test_home("zip-invalid");
        fs::create_dir_all(&root).unwrap();
        let store = SkillStore::new(root.join("target"));

        let traversal = root.join("traversal.zip");
        write_zip_entries(
            &traversal,
            &[("../escape/SKILL.md", b"invalid"), ("safe/file", b"safe")],
        );
        assert!(matches!(
            store.add(&traversal),
            Err(SkillError::InvalidArchive { .. })
        ));
        assert!(!root.join("escape").exists());

        let duplicate = root.join("duplicate.zip");
        write_zip_entries(
            &duplicate,
            &[("skill/SKILL.md", b"first"), ("skill//SKILL.md", b"second")],
        );
        let duplicate_error = store.add(&duplicate).unwrap_err();
        assert!(
            matches!(duplicate_error, SkillError::InvalidArchive { .. }),
            "unexpected duplicate error: {duplicate_error:?}"
        );

        let deep = root.join("deep.zip");
        let deep_path = format!(
            "{}/file",
            vec!["level"; MAX_IMPORT_RELATIVE_DEPTH + 1].join("/")
        );
        write_zip_entries(&deep, &[(deep_path.as_str(), b"deep")]);
        assert!(matches!(
            store.add(&deep),
            Err(SkillError::LimitExceeded { .. })
        ));

        let symlink = root.join("symlink.zip");
        write_zip_entries(
            &symlink,
            &[
                (
                    "linked/SKILL.md",
                    b"---\nname: linked\ndescription: Linked.\n---\n# Linked\n",
                ),
                ("linked/link", b"SKILL.md"),
            ],
        );
        patch_zip_entry_as_symlink(&symlink, b"linked/link");
        assert!(matches!(
            store.add(&symlink),
            Err(SkillError::UnsupportedFileType { .. })
        ));

        let bomb = root.join("bomb.zip");
        write_zip_bomb(&bomb);
        assert!(matches!(
            store.add(&bomb),
            Err(SkillError::LimitExceeded { .. })
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn directory_import_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let source = test_home("symlink-source");
        let target = test_home("symlink-target");
        fs::create_dir_all(source.join("skill")).unwrap();
        fs::write(
            source.join("skill/SKILL.md"),
            "---\nname: skill\ndescription: Test\n---\n# Skill\n",
        )
        .unwrap();
        symlink("SKILL.md", source.join("skill/link")).unwrap();

        let store = SkillStore::new(&target);
        assert!(matches!(
            store.import(source.join("skill")),
            Err(SkillError::UnsupportedFileType { .. })
        ));
        assert!(!target.join("skills/default/skill").exists());
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(target);
    }

    #[cfg(unix)]
    #[test]
    fn directory_import_rejects_symlinked_skill_file_and_special_files() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;

        let source = test_home("unsafe-directory-source");
        let target = test_home("unsafe-directory-target");
        fs::create_dir_all(source.join("linked")).unwrap();
        fs::write(
            source.join("outside.md"),
            "---\nname: linked\ndescription: Linked\n---\n# Linked\n",
        )
        .unwrap();
        symlink("../outside.md", source.join("linked/SKILL.md")).unwrap();
        let store = SkillStore::new(&target);
        assert!(matches!(
            store.import(source.join("linked")),
            Err(SkillError::UnsupportedFileType { .. })
        ));

        fs::create_dir_all(source.join("special")).unwrap();
        fs::write(
            source.join("special/SKILL.md"),
            "---\nname: special\ndescription: Special\n---\n# Special\n",
        )
        .unwrap();
        let _listener = UnixListener::bind(source.join("special/socket")).unwrap();
        assert!(matches!(
            store.import(source.join("special")),
            Err(SkillError::UnsupportedFileType { .. })
        ));
        assert!(!target.join("skills/default/linked").exists());
        assert!(!target.join("skills/default/special").exists());
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn import_budget_checks_are_batch_wide_and_precede_staging() {
        let root = test_home("budget");
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("a"), vec![0; 8]).unwrap();
        fs::write(second.join("b"), vec![0; 8]).unwrap();

        let mut budget = ImportBudget {
            entries: MAX_IMPORT_ENTRIES - 1,
            total_bytes: 0,
        };
        validate_dir_bounded(&first, &mut budget).unwrap();
        assert!(matches!(
            validate_dir_bounded(&second, &mut budget),
            Err(SkillError::LimitExceeded { .. })
        ));

        let mut total_budget = ImportBudget {
            entries: 0,
            total_bytes: MAX_IMPORT_TOTAL_BYTES - 4,
        };
        assert!(matches!(
            validate_dir_bounded(&first, &mut total_budget),
            Err(SkillError::LimitExceeded { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_failure_rolls_back_committed_targets_and_reports_cleanup_failure() {
        let root = test_home("rollback");
        let staging = root.join("staging");
        let skills = root.join("skills");
        fs::create_dir_all(staging.join("one")).unwrap();
        fs::create_dir_all(staging.join("two")).unwrap();

        let staged = vec![
            (package("one"), staging.join("one")),
            (package("two"), staging.join("two")),
        ];
        let mut operations = FailingCommitOps {
            fail_rename_at: 2,
            fail_cleanup: false,
            ..Default::default()
        };
        let error = commit_staged_items(&skills, staged, &mut operations).unwrap_err();
        match error {
            SkillError::ImportTransaction { cleanup, .. } => assert!(cleanup.is_empty()),
            other => panic!("unexpected error: {other}"),
        }
        assert!(!skills.join("one").exists());

        fs::create_dir_all(staging.join("three")).unwrap();
        fs::create_dir_all(staging.join("four")).unwrap();
        let staged = vec![
            (package("three"), staging.join("three")),
            (package("four"), staging.join("four")),
        ];
        let mut operations = FailingCommitOps {
            fail_rename_at: 2,
            fail_cleanup: true,
            ..Default::default()
        };
        let error = commit_staged_items(&skills, staged, &mut operations).unwrap_err();
        match error {
            SkillError::ImportTransaction { cleanup, .. } => {
                assert_eq!(cleanup.failures.len(), 1);
                assert_eq!(cleanup.failures[0].path, skills.join("three"));
            }
            other => panic!("unexpected error: {other}"),
        }
        assert!(skills.join("three").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_duplicate_and_invalid_references() {
        let home = test_home("reject");
        let store = SkillStore::new(&home);
        let options = SkillCreateOptions::new("code-review", "Review").unwrap();
        store.create(options.clone()).unwrap();
        assert!(matches!(
            store.create(options),
            Err(SkillError::AlreadyExists { .. })
        ));
        assert!(SkillCreateOptions::new("Code_Review", "Review").is_err());
        let _ = fs::remove_dir_all(home);
    }

    fn write_skill_zip(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        writer.start_file("zipped/SKILL.md", options).unwrap();
        writer
            .write_all(b"---\nname: zipped\ndescription: Zipped skill.\n---\n# Zipped\n")
            .unwrap();
        writer
            .start_file("zipped/references/guide.md", options)
            .unwrap();
        writer.write_all(b"Reference").unwrap();
        writer.finish().unwrap();
    }

    fn write_zip_entries(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for (name, body) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
    }

    fn write_zip_bomb(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("bomb/SKILL.md", options).unwrap();
        writer
            .write_all(b"---\nname: bomb\ndescription: Bomb.\n---\n# Bomb\n")
            .unwrap();
        writer.start_file("bomb/assets/zeros.bin", options).unwrap();
        io::copy(
            &mut io::repeat(0).take(MAX_IMPORT_SINGLE_FILE_BYTES + 1),
            &mut writer,
        )
        .unwrap();
        writer.finish().unwrap();
    }

    fn patch_zip_entry_as_symlink(path: &Path, target_name: &[u8]) {
        const CENTRAL_HEADER_SIGNATURE: &[u8; 4] = b"PK\x01\x02";
        const CENTRAL_HEADER_LEN: usize = 46;
        let mut bytes = fs::read(path).unwrap();
        let mut offset = 0usize;
        let mut patched = false;
        while offset + CENTRAL_HEADER_LEN <= bytes.len() {
            let Some(relative) = bytes[offset..]
                .windows(CENTRAL_HEADER_SIGNATURE.len())
                .position(|window| window == CENTRAL_HEADER_SIGNATURE)
            else {
                break;
            };
            let header = offset + relative;
            let name_len = u16::from_le_bytes([bytes[header + 28], bytes[header + 29]]) as usize;
            let extra_len = u16::from_le_bytes([bytes[header + 30], bytes[header + 31]]) as usize;
            let comment_len = u16::from_le_bytes([bytes[header + 32], bytes[header + 33]]) as usize;
            let name_start = header + CENTRAL_HEADER_LEN;
            let name_end = name_start + name_len;
            if &bytes[name_start..name_end] == target_name {
                bytes[header + 5] = 3;
                let attributes = (0o120777_u32 << 16).to_le_bytes();
                bytes[header + 38..header + 42].copy_from_slice(&attributes);
                patched = true;
                break;
            }
            offset = name_end + extra_len + comment_len;
        }
        assert!(patched, "target zip entry must be found");
        fs::write(path, bytes).unwrap();
    }

    #[derive(Default)]
    struct FailingCommitOps {
        rename_calls: usize,
        fail_rename_at: usize,
        fail_cleanup: bool,
    }

    impl CommitOps for FailingCommitOps {
        fn rename(&mut self, source: &Path, target: &Path) -> io::Result<()> {
            self.rename_calls += 1;
            if self.rename_calls == self.fail_rename_at {
                return Err(io::Error::other("injected rename failure"));
            }
            fs::rename(source, target)
        }

        fn remove_dir_all(&mut self, path: &Path) -> io::Result<()> {
            if self.fail_cleanup {
                return Err(io::Error::other("injected cleanup failure"));
            }
            fs::remove_dir_all(path)
        }
    }
}
