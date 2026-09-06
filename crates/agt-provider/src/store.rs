use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agt_config::{AgtPaths, CATEGORY_README_FILE, DefinitionCategories};
use agt_core::CategoryName;
use agt_storage::{check_no_symlinks_under, ensure_dir_under, write_atomic_under};

use crate::config::{
    ProviderConfig, ProviderConfigUpdate, ProviderConfigYaml, ProviderType, ProviderValidation,
};
use crate::diagnostic::{ProviderDiagnostic, ProviderDiagnosticCategory};
use crate::error::{ProviderError, Result};
use crate::models::{ProviderModel, list_openai_compatible_models};
use crate::name::validate_provider_name;

const PROVIDER_FILE: &str = "provider.yaml";

#[derive(Clone, Debug)]
pub struct ProviderStore {
    paths: AgtPaths,
}

impl ProviderStore {
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

    pub fn providers_dir(&self) -> PathBuf {
        self.paths.data_dir("providers")
    }

    pub fn categories(&self) -> DefinitionCategories {
        DefinitionCategories::new(self.providers_dir())
    }

    pub fn category_dir(&self, category: &CategoryName) -> PathBuf {
        self.paths.category_dir("providers", category)
    }

    pub fn provider_dir(&self, name: &str) -> Result<PathBuf> {
        self.provider_dir_in(&CategoryName::default(), name)
    }

    pub fn provider_dir_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        validate_provider_name(name)?;
        Ok(self.category_dir(category).join(name))
    }

    pub fn provider_file(&self, name: &str) -> Result<PathBuf> {
        self.provider_file_in(&CategoryName::default(), name)
    }

    pub fn provider_file_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        Ok(self.provider_dir_in(category, name)?.join(PROVIDER_FILE))
    }

    pub fn create(&self, config: &ProviderConfig) -> Result<StoredProvider> {
        self.create_in(&CategoryName::default(), config)
    }

    pub fn create_in(
        &self,
        category: &CategoryName,
        config: &ProviderConfig,
    ) -> Result<StoredProvider> {
        self.require_write_category(category)?;
        let dir = self.provider_dir_in(category, config.name.as_str())?;
        let file = dir.join(PROVIDER_FILE);
        check_no_symlinks_under(self.paths.home(), &file).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => ProviderError::io(path, source),
            other => ProviderError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if file.exists() {
            return Err(ProviderError::AlreadyExists {
                name: config.name.to_string(),
            });
        }

        ensure_dir_under(self.paths.home(), &dir).map_err(map_core_error)?;
        self.write_config(&file, config)?;

        Ok(StoredProvider {
            category: category.clone(),
            path: file,
            config: config.clone(),
        })
    }

    pub fn read(&self, name: &str) -> Result<StoredProvider> {
        self.read_in(&CategoryName::default(), name)
    }

    pub fn read_in(&self, category: &CategoryName, name: &str) -> Result<StoredProvider> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        let file = self.provider_file_in(category, name)?;
        check_no_symlinks_under(self.paths.home(), &file).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => ProviderError::io(path, source),
            other => ProviderError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if !file.exists() {
            return Err(ProviderError::NotFound {
                name: name.to_string(),
            });
        }

        let text =
            fs::read_to_string(&file).map_err(|source| ProviderError::io(file.clone(), source))?;
        let yaml: ProviderConfigYaml = serde_yaml::from_str(&text)
            .map_err(|source| ProviderError::yaml(file.clone(), source))?;
        let config = ProviderConfig::try_from(yaml)?;
        config.validate()?;

        Ok(StoredProvider {
            category: category.clone(),
            path: file,
            config,
        })
    }

    pub fn list(&self) -> Result<Vec<StoredProvider>> {
        Ok(self.scan()?.items)
    }

    pub fn scan(&self) -> Result<ProviderScan> {
        check_no_symlinks_under(self.paths.home(), self.paths.home()).map_err(
            |error| match error {
                agt_core::AgtError::Io { path, source } => ProviderError::io(path, source),
                other => ProviderError::InvalidConfig {
                    message: other.to_string(),
                },
            },
        )?;
        let providers_dir = self.providers_dir();
        let root_metadata = match fs::symlink_metadata(&providers_dir) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(ProviderScan::default());
            }
            Err(source) => return Err(ProviderError::io(&providers_dir, source)),
        };
        if !root_metadata.file_type().is_dir() {
            return Ok(ProviderScan {
                items: Vec::new(),
                diagnostics: vec![ProviderDiagnostic::new(
                    &providers_dir,
                    None,
                    ProviderDiagnosticCategory::UnsupportedFileType,
                    "providers root 必须是普通目录",
                    "移除 symlink/特殊文件并创建普通 providers 目录",
                )],
            });
        }

        let mut scan = ProviderScan::default();
        for entry in read_dir_sorted(&providers_dir)? {
            let path = entry.path();
            let category_text = entry.file_name().to_string_lossy().into_owned();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(source) => {
                    scan.diagnostics.push(ProviderDiagnostic::new(
                        &path,
                        Some(category_text),
                        ProviderDiagnosticCategory::Io,
                        ProviderError::io(&path, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(ProviderDiagnostic::new(
                    &path,
                    Some(category_text.clone()),
                    if file_type.is_symlink() {
                        ProviderDiagnosticCategory::UnsupportedFileType
                    } else {
                        ProviderDiagnosticCategory::UnexpectedLayout
                    },
                    "providers root 只能包含 <category>/ 目录",
                    "移除该 entry 或改为规范 category 目录",
                ));
                continue;
            }
            let category = match CategoryName::new(&category_text) {
                Ok(category) => category,
                Err(error) => {
                    scan.diagnostics.push(ProviderDiagnostic::new(
                        &path,
                        Some(category_text),
                        ProviderDiagnosticCategory::InvalidIdentity,
                        error.to_string(),
                        "将目录名改为安全 category 名称",
                    ));
                    continue;
                }
            };
            self.scan_category(&category, &path, &mut scan)?;
        }
        scan.items.sort_by(|left, right| {
            (&left.category, left.config.name.as_str())
                .cmp(&(&right.category, right.config.name.as_str()))
        });
        scan.diagnostics
            .sort_by(|left, right| (&left.path, left.category).cmp(&(&right.path, right.category)));
        Ok(scan)
    }

    fn scan_category(
        &self,
        category: &CategoryName,
        category_dir: &Path,
        scan: &mut ProviderScan,
    ) -> Result<()> {
        for entry in read_dir_sorted(category_dir)? {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == CATEGORY_README_FILE {
                continue;
            }
            let name_hint = Some(format!("{category}/{name}"));
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(source) => {
                    scan.diagnostics.push(ProviderDiagnostic::new(
                        &path,
                        name_hint,
                        ProviderDiagnosticCategory::Io,
                        ProviderError::io(&path, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(ProviderDiagnostic::new(
                    &path,
                    name_hint,
                    if file_type.is_symlink() {
                        ProviderDiagnosticCategory::UnsupportedFileType
                    } else {
                        ProviderDiagnosticCategory::UnexpectedLayout
                    },
                    "provider category 只能包含 <name>/provider.yaml 目录",
                    "移除该 entry 或改为规范 provider 目录",
                ));
                continue;
            }

            let file = path.join(PROVIDER_FILE);
            match fs::symlink_metadata(&file) {
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    scan.diagnostics.push(ProviderDiagnostic::new(
                        &file,
                        name_hint,
                        ProviderDiagnosticCategory::MissingConfigFile,
                        "provider 目录缺少 provider.yaml",
                        "补充 provider.yaml 或删除空目录",
                    ));
                    continue;
                }
                Err(source) => {
                    scan.diagnostics.push(ProviderDiagnostic::new(
                        &file,
                        name_hint,
                        ProviderDiagnosticCategory::Io,
                        ProviderError::io(&file, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
                Ok(metadata) if !metadata.file_type().is_file() => {
                    scan.diagnostics.push(ProviderDiagnostic::new(
                        &file,
                        name_hint,
                        ProviderDiagnosticCategory::UnsupportedFileType,
                        "provider.yaml 必须是普通文件",
                        "移除 symlink/特殊文件并创建普通 provider.yaml",
                    ));
                    continue;
                }
                Ok(_) => {}
            }

            match self.read_in(category, &name) {
                Ok(provider) if provider.config.name.as_str() == name => scan.items.push(provider),
                Ok(provider) => scan.diagnostics.push(ProviderDiagnostic::new(
                    &file,
                    name_hint,
                    ProviderDiagnosticCategory::InvalidIdentity,
                    format!(
                        "provider.yaml 的 name `{}` 与目录名 `{name}` 不一致",
                        provider.config.name
                    ),
                    "让 provider.yaml 的 name 与父目录名称完全一致",
                )),
                Err(error) => scan
                    .diagnostics
                    .push(ProviderDiagnostic::from_error(&file, name_hint, &error)),
            }
        }
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf> {
        self.delete_in(&CategoryName::default(), name)
    }

    pub fn delete_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        let dir = self.provider_dir_in(category, name)?;
        let file = dir.join(PROVIDER_FILE);
        check_no_symlinks_under(self.paths.home(), &dir).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => ProviderError::io(path, source),
            other => ProviderError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if !file.exists() {
            return Err(ProviderError::NotFound {
                name: name.to_string(),
            });
        }

        ensure_child_path(&self.category_dir(category), &dir)?;
        fs::remove_dir_all(&dir).map_err(|source| ProviderError::io(dir.clone(), source))?;
        Ok(dir)
    }

    pub fn test(&self, name: &str) -> Result<ProviderValidation> {
        self.read(name)?.config.validate()
    }

    pub fn test_in(&self, category: &CategoryName, name: &str) -> Result<ProviderValidation> {
        self.read_in(category, name)?.config.validate()
    }

    pub fn list_models(&self, name: &str) -> Result<Vec<ProviderModel>> {
        let provider = self.read(name)?;
        self.list_models_for(provider)
    }

    pub fn list_models_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> Result<Vec<ProviderModel>> {
        let provider = self.read_in(category, name)?;
        self.list_models_for(provider)
    }

    pub fn update(&self, name: &str, update: ProviderConfigUpdate) -> Result<StoredProvider> {
        self.update_in(&CategoryName::default(), name, update)
    }

    pub fn update_in(
        &self,
        category: &CategoryName,
        name: &str,
        update: ProviderConfigUpdate,
    ) -> Result<StoredProvider> {
        if update.is_empty() {
            return Err(ProviderError::InvalidConfig {
                message: "至少提供一个需要修改的字段".to_string(),
            });
        }
        let mut stored = self.read_in(category, name)?;
        if let Some(base_url) = update.base_url {
            stored.config.base_url = base_url;
        }
        if let Some(api_key) = update.api_key {
            stored.config.api_key = api_key;
        }
        stored.config.validate()?;
        self.write_config(&stored.path, &stored.config)?;
        Ok(stored)
    }

    fn list_models_for(&self, provider: StoredProvider) -> Result<Vec<ProviderModel>> {
        match provider.config.provider_type {
            ProviderType::OpenAiCompatible => list_openai_compatible_models(&provider.config),
        }
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

    fn write_config(&self, file: &Path, config: &ProviderConfig) -> Result<()> {
        let yaml = serde_yaml::to_string(&ProviderConfigYaml::from(config))
            .map_err(|source| ProviderError::yaml(file, source))?;
        write_atomic_under(self.paths.home(), file, yaml.as_bytes())
            .map_err(|source| ProviderError::io(file, source))
    }
}

#[derive(Clone, Debug)]
pub struct StoredProvider {
    pub category: CategoryName,
    pub path: PathBuf,
    pub config: ProviderConfig,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderScan {
    pub items: Vec<StoredProvider>,
    pub diagnostics: Vec<ProviderDiagnostic>,
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| ProviderError::io(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| ProviderError::io(path, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<()> {
    let parent = parent
        .canonicalize()
        .map_err(|source| ProviderError::io(parent, source))?;
    let child = child
        .canonicalize()
        .map_err(|source| ProviderError::io(child, source))?;

    if child.starts_with(&parent) && child != parent {
        Ok(())
    } else {
        Err(ProviderError::InvalidConfig {
            message: "provider 删除路径越界".to_string(),
        })
    }
}

fn map_core_error(error: agt_core::AgtError) -> ProviderError {
    match error {
        agt_core::AgtError::Io { path, source } => ProviderError::io(path, source),
        other => ProviderError::InvalidConfig {
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::mask::mask_api_key;

    fn test_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("agt-provider-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn create_read_list_test_and_delete_provider() {
        let home = test_home("roundtrip");
        let store = ProviderStore::new(&home);
        let config = ProviderConfig::openai_compatible(
            "my-provider",
            "https://api.example.com/v1",
            "sk-test1234",
        )
        .expect("valid config");

        let stored = store.create(&config).expect("create provider");
        assert_eq!(
            stored.path,
            home.join("providers/default/my-provider/provider.yaml")
        );
        assert_eq!(mask_api_key(&stored.config.api_key), "****1234");

        let loaded = store.read("my-provider").expect("read provider");
        assert_eq!(loaded.config, config);

        let providers = store.list().expect("list providers");
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].config.name.as_str(), "my-provider");

        assert!(matches!(
            store.test("my-provider"),
            Ok(ProviderValidation::Ready)
        ));

        let deleted = store.delete("my-provider").expect("delete provider");
        assert_eq!(deleted, home.join("providers/default/my-provider"));
        assert!(!deleted.exists());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_duplicate_and_unsafe_names() {
        let home = test_home("duplicate");
        let store = ProviderStore::new(&home);
        let config = ProviderConfig::openai_compatible(
            "my-provider",
            "https://api.example.com/v1",
            "sk-test1234",
        )
        .expect("valid config");

        store.create(&config).expect("first create succeeds");
        assert!(matches!(
            store.create(&config),
            Err(ProviderError::AlreadyExists { .. })
        ));
        assert!(store.provider_dir("../bad").is_err());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_keeps_valid_items_and_reports_secret_safe_diagnostics() {
        let home = test_home("scan-diagnostics");
        let store = ProviderStore::new(&home);
        let config = ProviderConfig::openai_compatible(
            "valid",
            "https://api.example.com/v1",
            "sk-valid-secret",
        )
        .unwrap();
        store.create(&config).unwrap();
        fs::create_dir_all(home.join("providers/default/broken")).unwrap();
        fs::write(
            home.join("providers/default/broken/provider.yaml"),
            "name: broken\ntype: openai-compatible\nbase_url: ''\napi_key: super-secret-value\n",
        )
        .unwrap();
        fs::write(home.join("providers/default/stray"), "not a directory").unwrap();

        let scan = store.scan().unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].config.name.as_str(), "valid");
        assert_eq!(scan.diagnostics.len(), 2);
        assert!(
            scan.diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("super-secret-value"))
        );
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_rejects_config_directory_identity_mismatch() {
        let home = test_home("identity-mismatch");
        let store = ProviderStore::new(&home);
        let config = ProviderConfig::openai_compatible(
            "provider-name",
            "https://api.example.com/v1",
            "sk-test1234",
        )
        .unwrap();
        store.create(&config).unwrap();
        fs::rename(
            home.join("providers/default/provider-name"),
            home.join("providers/default/directory-name"),
        )
        .unwrap();

        let scan = store.scan().unwrap();
        assert!(scan.items.is_empty());
        assert_eq!(scan.diagnostics.len(), 1);
        assert_eq!(
            scan.diagnostics[0].category,
            ProviderDiagnosticCategory::InvalidIdentity
        );

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_reports_non_directory_root() {
        let home = test_home("scan-root");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("providers"), "not a directory").unwrap();

        let scan = ProviderStore::new(&home).scan().unwrap();
        assert!(scan.items.is_empty());
        assert_eq!(
            scan.diagnostics[0].category,
            ProviderDiagnosticCategory::UnsupportedFileType
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_symlinked_provider_category() {
        use std::os::unix::fs::symlink;

        let home = test_home("symlink-category");
        let outside = home.join("outside");
        let config = ProviderConfig::openai_compatible(
            "linked-provider",
            "https://api.example.com/v1",
            "sk-test1234",
        )
        .expect("valid config");
        let yaml =
            serde_yaml::to_string(&ProviderConfigYaml::from(&config)).expect("serialize provider");
        fs::create_dir_all(&outside).expect("create outside directory");
        fs::write(outside.join(PROVIDER_FILE), yaml).expect("write provider config");
        fs::create_dir_all(home.join("providers")).expect("create providers root");
        symlink(&outside, home.join("providers/default")).expect("create category symlink");

        let error = ProviderStore::new(&home)
            .read("linked-provider")
            .expect_err("read must reject category symlink");
        assert!(error.to_string().contains("symlink"));

        let _ = fs::remove_dir_all(&home);
    }
}
