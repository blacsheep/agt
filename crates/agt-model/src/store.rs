use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agt_config::{AgtPaths, CATEGORY_README_FILE, DefinitionCategories};
use agt_core::CategoryName;
use agt_provider::ProviderStore;
use agt_storage::{check_no_symlinks_under, ensure_dir_under, write_atomic_under};

use crate::config::{ModelConfig, ModelConfigUpdate, ModelConfigYaml};
use crate::diagnostic::{ModelDiagnostic, ModelDiagnosticCategory};
use crate::error::{ModelError, Result};
use crate::name::validate_model_name;

const MODEL_FILE: &str = "model.yaml";

#[derive(Clone, Debug)]
pub struct ModelStore {
    paths: AgtPaths,
    provider_store: ProviderStore,
}

impl ModelStore {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            paths: AgtPaths::new(&home),
            provider_store: ProviderStore::new(home),
        }
    }

    pub fn from_default_home() -> agt_core::Result<Self> {
        let paths = AgtPaths::from_default_home()?;
        Ok(Self {
            provider_store: ProviderStore::new(paths.home().to_path_buf()),
            paths,
        })
    }

    pub fn models_dir(&self) -> PathBuf {
        self.paths.data_dir("models")
    }

    pub fn categories(&self) -> DefinitionCategories {
        DefinitionCategories::new(self.models_dir())
    }

    pub fn category_dir(&self, category: &CategoryName) -> PathBuf {
        self.paths.category_dir("models", category)
    }

    pub fn model_dir(&self, name: &str) -> Result<PathBuf> {
        self.model_dir_in(&CategoryName::default(), name)
    }

    pub fn model_dir_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        validate_model_name(name)?;
        Ok(self.category_dir(category).join(name))
    }

    pub fn model_file(&self, name: &str) -> Result<PathBuf> {
        self.model_file_in(&CategoryName::default(), name)
    }

    pub fn model_file_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        Ok(self.model_dir_in(category, name)?.join(MODEL_FILE))
    }

    pub fn create(&self, config: &ModelConfig) -> Result<StoredModel> {
        self.create_in(&CategoryName::default(), config)
    }

    pub fn create_in(&self, category: &CategoryName, config: &ModelConfig) -> Result<StoredModel> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        self.ensure_provider_exists(config)?;
        if category.is_default() {
            self.categories().ensure_default()?;
        }
        let dir = self.model_dir_in(category, config.name.as_str())?;
        let file = dir.join(MODEL_FILE);
        check_no_symlinks_under(self.paths.home(), &file).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => ModelError::io(path, source),
            other => ModelError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if file.exists() {
            return Err(ModelError::AlreadyExists {
                name: config.name.to_string(),
            });
        }

        ensure_dir_under(self.paths.home(), &dir).map_err(map_core_error)?;
        self.write_config(&file, config)?;

        Ok(StoredModel {
            category: category.clone(),
            path: file,
            config: config.clone(),
        })
    }

    pub fn read(&self, name: &str) -> Result<StoredModel> {
        self.read_in(&CategoryName::default(), name)
    }

    pub fn read_in(&self, category: &CategoryName, name: &str) -> Result<StoredModel> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        let file = self.model_file_in(category, name)?;
        check_no_symlinks_under(self.paths.home(), &file).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => ModelError::io(path, source),
            other => ModelError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if !file.exists() {
            return Err(ModelError::NotFound {
                name: name.to_string(),
            });
        }

        let text =
            fs::read_to_string(&file).map_err(|source| ModelError::io(file.clone(), source))?;
        let yaml: ModelConfigYaml =
            serde_yaml::from_str(&text).map_err(|source| ModelError::yaml(file.clone(), source))?;
        let config = ModelConfig::try_from(yaml)?;

        Ok(StoredModel {
            category: category.clone(),
            path: file,
            config,
        })
    }

    pub fn list(&self) -> Result<Vec<StoredModel>> {
        Ok(self.scan()?.items)
    }

    pub fn scan(&self) -> Result<ModelScan> {
        check_no_symlinks_under(self.paths.home(), self.paths.home()).map_err(
            |error| match error {
                agt_core::AgtError::Io { path, source } => ModelError::io(path, source),
                other => ModelError::InvalidConfig {
                    message: other.to_string(),
                },
            },
        )?;
        let models_dir = self.models_dir();
        let root_metadata = match fs::symlink_metadata(&models_dir) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(ModelScan::default());
            }
            Err(source) => return Err(ModelError::io(&models_dir, source)),
        };
        if !root_metadata.file_type().is_dir() {
            return Ok(ModelScan {
                items: Vec::new(),
                diagnostics: vec![ModelDiagnostic::new(
                    &models_dir,
                    None,
                    ModelDiagnosticCategory::UnsupportedFileType,
                    "models root 必须是普通目录",
                    "移除 symlink/特殊文件并创建普通 models 目录",
                )],
            });
        }

        let mut scan = ModelScan::default();
        for entry in read_dir_sorted(&models_dir)? {
            let path = entry.path();
            let category_text = entry.file_name().to_string_lossy().into_owned();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(source) => {
                    scan.diagnostics.push(ModelDiagnostic::new(
                        &path,
                        Some(category_text),
                        ModelDiagnosticCategory::Io,
                        ModelError::io(&path, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(ModelDiagnostic::new(
                    &path,
                    Some(category_text.clone()),
                    if file_type.is_symlink() {
                        ModelDiagnosticCategory::UnsupportedFileType
                    } else {
                        ModelDiagnosticCategory::UnexpectedLayout
                    },
                    "models root 只能包含 <category>/ 目录",
                    "移除该 entry 或改为规范 category 目录",
                ));
                continue;
            }
            let category = match CategoryName::new(&category_text) {
                Ok(category) => category,
                Err(error) => {
                    scan.diagnostics.push(ModelDiagnostic::new(
                        &path,
                        Some(category_text),
                        ModelDiagnosticCategory::InvalidIdentity,
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
        scan: &mut ModelScan,
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
                    scan.diagnostics.push(ModelDiagnostic::new(
                        &path,
                        name_hint,
                        ModelDiagnosticCategory::Io,
                        ModelError::io(&path, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(ModelDiagnostic::new(
                    &path,
                    name_hint,
                    if file_type.is_symlink() {
                        ModelDiagnosticCategory::UnsupportedFileType
                    } else {
                        ModelDiagnosticCategory::UnexpectedLayout
                    },
                    "model category 只能包含 <name>/model.yaml 目录",
                    "移除该 entry 或改为规范 model 目录",
                ));
                continue;
            }

            let file = path.join(MODEL_FILE);
            match fs::symlink_metadata(&file) {
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    scan.diagnostics.push(ModelDiagnostic::new(
                        &file,
                        name_hint,
                        ModelDiagnosticCategory::MissingConfigFile,
                        "model 目录缺少 model.yaml",
                        "补充 model.yaml 或删除空目录",
                    ));
                    continue;
                }
                Err(source) => {
                    scan.diagnostics.push(ModelDiagnostic::new(
                        &file,
                        name_hint,
                        ModelDiagnosticCategory::Io,
                        ModelError::io(&file, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
                Ok(metadata) if !metadata.file_type().is_file() => {
                    scan.diagnostics.push(ModelDiagnostic::new(
                        &file,
                        name_hint,
                        ModelDiagnosticCategory::UnsupportedFileType,
                        "model.yaml 必须是普通文件",
                        "移除 symlink/特殊文件并创建普通 model.yaml",
                    ));
                    continue;
                }
                Ok(_) => {}
            }

            match self.read_in(category, &name) {
                Ok(model) if model.config.name.as_str() == name => scan.items.push(model),
                Ok(model) => scan.diagnostics.push(ModelDiagnostic::new(
                    &file,
                    name_hint,
                    ModelDiagnosticCategory::InvalidIdentity,
                    format!(
                        "model.yaml 的 name `{}` 与目录名 `{name}` 不一致",
                        model.config.name
                    ),
                    "让 model.yaml 的 name 与父目录名称完全一致",
                )),
                Err(error) => scan
                    .diagnostics
                    .push(ModelDiagnostic::from_error(&file, name_hint, &error)),
            }
        }
        Ok(())
    }

    pub fn update(&self, name: &str, update: ModelConfigUpdate) -> Result<StoredModel> {
        self.update_in(&CategoryName::default(), name, update)
    }

    pub fn update_in(
        &self,
        category: &CategoryName,
        name: &str,
        update: ModelConfigUpdate,
    ) -> Result<StoredModel> {
        let mut stored = self.read_in(category, name)?;
        stored.config.apply_update(update)?;
        self.write_config(&stored.path, &stored.config)?;
        Ok(stored)
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf> {
        self.delete_in(&CategoryName::default(), name)
    }

    pub fn delete_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        let dir = self.model_dir_in(category, name)?;
        let file = dir.join(MODEL_FILE);
        check_no_symlinks_under(self.paths.home(), &dir).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => ModelError::io(path, source),
            other => ModelError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if !file.exists() {
            return Err(ModelError::NotFound {
                name: name.to_string(),
            });
        }

        ensure_child_path(&self.category_dir(category), &dir)?;
        fs::remove_dir_all(&dir).map_err(|source| ModelError::io(dir.clone(), source))?;
        Ok(dir)
    }

    fn ensure_provider_exists(&self, config: &ModelConfig) -> Result<()> {
        self.provider_store
            .read_in(config.provider.category(), config.provider.name().as_str())
            .map(|_| ())
            .map_err(|source| ModelError::ProviderUnavailable {
                name: config.provider.to_string(),
                source,
            })
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

    fn write_config(&self, file: &Path, config: &ModelConfig) -> Result<()> {
        let yaml = serde_yaml::to_string(&ModelConfigYaml::from(config))
            .map_err(|source| ModelError::yaml(file, source))?;
        write_atomic_under(self.paths.home(), file, yaml.as_bytes())
            .map_err(|source| ModelError::io(file, source))
    }
}

#[derive(Clone, Debug)]
pub struct StoredModel {
    pub category: CategoryName,
    pub path: PathBuf,
    pub config: ModelConfig,
}

#[derive(Clone, Debug, Default)]
pub struct ModelScan {
    pub items: Vec<StoredModel>,
    pub diagnostics: Vec<ModelDiagnostic>,
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| ModelError::io(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| ModelError::io(path, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<()> {
    let parent = parent
        .canonicalize()
        .map_err(|source| ModelError::io(parent, source))?;
    let child = child
        .canonicalize()
        .map_err(|source| ModelError::io(child, source))?;

    if child.starts_with(&parent) && child != parent {
        Ok(())
    } else {
        Err(ModelError::InvalidConfig {
            message: "model 删除路径越界".to_string(),
        })
    }
}

fn map_core_error(error: agt_core::AgtError) -> ModelError {
    match error {
        agt_core::AgtError::Io { path, source } => ModelError::io(path, source),
        other => ModelError::InvalidConfig {
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use agt_provider::ProviderConfig;

    use super::*;
    use crate::config::{DEFAULT_CONTEXT_WINDOW, ModelApiFormat, ReasoningEffort};
    use crate::reference::ModelReference;

    fn test_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("agt-model-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    fn create_provider(home: &Path) {
        let provider_store = ProviderStore::new(home);
        let provider = ProviderConfig::openai_compatible(
            "my-provider",
            "https://api.example.com/v1",
            "sk-test1234",
        )
        .expect("provider config");
        provider_store.create(&provider).expect("create provider");
    }

    fn model_config() -> ModelConfig {
        let reference = ModelReference::parse("my-provider/deepseek-chat").expect("reference");
        ModelConfig::new("fast-model", reference).expect("model config")
    }

    #[test]
    fn create_read_list_update_and_delete_model() {
        let home = test_home("roundtrip");
        create_provider(&home);
        let store = ModelStore::new(&home);

        let stored = store.create(&model_config()).expect("create model");
        assert_eq!(
            stored.path,
            home.join("models/default/fast-model/model.yaml")
        );
        assert_eq!(
            stored.config.capabilities.context_window,
            DEFAULT_CONTEXT_WINDOW
        );

        let loaded = store.read("fast-model").expect("read model");
        assert_eq!(loaded.config.name.as_str(), "fast-model");
        assert_eq!(loaded.config.provider.name().as_str(), "my-provider");
        assert!(loaded.config.provider.category().is_default());
        assert_eq!(loaded.config.model, "deepseek-chat");

        let models = store.list().expect("list models");
        assert_eq!(models.len(), 1);

        let updated = store
            .update(
                "fast-model",
                ModelConfigUpdate {
                    temperature: Some(0.3),
                    max_tokens: Some(4096),
                    api_format: Some(ModelApiFormat::Responses),
                    reasoning_effort: Some(ReasoningEffort::High),
                    thinking_budget_tokens: Some(8192),
                    show_reasoning: Some(true),
                    context_window: Some(1_000_000),
                    ..Default::default()
                },
            )
            .expect("update model");
        assert_eq!(updated.config.params.temperature, 0.3);
        assert_eq!(updated.config.params.max_tokens, Some(4096));
        assert_eq!(updated.config.api_format, ModelApiFormat::Responses);
        assert_eq!(
            updated.config.params.reasoning_effort,
            Some(ReasoningEffort::High)
        );
        assert_eq!(updated.config.params.thinking_budget_tokens, Some(8192));
        assert!(updated.config.params.show_reasoning);
        assert_eq!(updated.config.capabilities.context_window, 1_000_000);

        let deleted = store.delete("fast-model").expect("delete model");
        assert_eq!(deleted, home.join("models/default/fast-model"));
        assert!(!deleted.exists());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_missing_provider_duplicate_and_unsafe_names() {
        let home = test_home("validation");
        let store = ModelStore::new(&home);
        assert!(matches!(
            store.create(&model_config()),
            Err(ModelError::ProviderUnavailable { .. })
        ));
        assert!(!home.join("models").exists());

        create_provider(&home);
        store
            .create(&model_config())
            .expect("first create succeeds");
        assert!(matches!(
            store.create(&model_config()),
            Err(ModelError::AlreadyExists { .. })
        ));
        assert!(store.model_dir("../bad").is_err());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_keeps_valid_items_and_reports_malformed_siblings() {
        let home = test_home("scan-diagnostics");
        create_provider(&home);
        let store = ModelStore::new(&home);
        store.create(&model_config()).unwrap();
        fs::create_dir_all(home.join("models/default/broken")).unwrap();
        fs::write(
            home.join("models/default/broken/model.yaml"),
            "name: [broken",
        )
        .unwrap();
        fs::write(home.join("models/default/stray"), "not a directory").unwrap();

        let scan = store.scan().unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].config.name.as_str(), "fast-model");
        assert_eq!(scan.diagnostics.len(), 2);
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_rejects_config_directory_identity_mismatch() {
        let home = test_home("identity-mismatch");
        create_provider(&home);
        let store = ModelStore::new(&home);
        store.create(&model_config()).unwrap();
        fs::rename(
            home.join("models/default/fast-model"),
            home.join("models/default/directory-name"),
        )
        .unwrap();

        let scan = store.scan().unwrap();
        assert!(scan.items.is_empty());
        assert_eq!(scan.diagnostics.len(), 1);
        assert_eq!(
            scan.diagnostics[0].category,
            ModelDiagnosticCategory::InvalidIdentity
        );

        let _ = fs::remove_dir_all(&home);
    }
}
