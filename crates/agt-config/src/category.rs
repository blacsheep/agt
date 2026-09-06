use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agt_core::{CategoryName, CategoryNameError};
use agt_storage::{check_no_symlinks_under, ensure_dir_under};

pub const CATEGORY_README_FILE: &str = "README.md";

pub type Result<T> = std::result::Result<T, CategoryError>;

#[derive(Debug, thiserror::Error)]
pub enum CategoryError {
    #[error(transparent)]
    InvalidName(#[from] CategoryNameError),

    #[error("category 已存在：{category}")]
    AlreadyExists { category: CategoryName },

    #[error("category 不存在：{category}")]
    NotFound { category: CategoryName },

    #[error("default category 是保留分类，不能删除")]
    ReservedDefault,

    #[error("category 非空：{category}")]
    NotEmpty { category: CategoryName },

    #[error("category 文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl CategoryError {
    fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefinitionCategories {
    root: PathBuf,
}

impl DefinitionCategories {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, category: &CategoryName) -> PathBuf {
        self.root.join(category.as_str())
    }

    fn managed_root(&self) -> &Path {
        self.root
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    pub fn create(&self, category: &CategoryName) -> Result<PathBuf> {
        let path = self.path(category);
        check_no_symlinks_under(self.managed_root(), &path).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => CategoryError::io(path, source),
            other => CategoryError::io(
                &path,
                io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
            ),
        })?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CategoryError::io(
                    &path,
                    io::Error::new(io::ErrorKind::InvalidInput, "category path 不能是 symlink"),
                ));
            }
            Ok(_) => {
                return Err(CategoryError::AlreadyExists {
                    category: category.clone(),
                });
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(CategoryError::io(&path, source)),
        }

        ensure_dir_under(self.managed_root(), &path).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => CategoryError::io(path, source),
            other => CategoryError::io(
                &path,
                io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
            ),
        })?;
        Ok(path)
    }

    pub fn ensure_default(&self) -> Result<PathBuf> {
        let category = CategoryName::default();
        let path = self.path(&category);
        check_no_symlinks_under(self.managed_root(), &path).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => CategoryError::io(path, source),
            other => CategoryError::io(
                &path,
                io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
            ),
        })?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(CategoryError::io(
                &path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "default category path 不能是 symlink",
                ),
            )),
            Ok(metadata) if metadata.file_type().is_dir() => Ok(path),
            Ok(_) => Err(CategoryError::io(
                &path,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "default category path 必须是普通目录",
                ),
            )),
            Err(source) if source.kind() == io::ErrorKind::NotFound => self.create(&category),
            Err(source) => Err(CategoryError::io(&path, source)),
        }
    }

    pub fn require(&self, category: &CategoryName) -> Result<PathBuf> {
        let path = self.path(category);
        check_no_symlinks_under(self.managed_root(), &path).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => CategoryError::io(path, source),
            other => CategoryError::io(
                &path,
                io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
            ),
        })?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => Ok(path),
            Ok(_) => Err(CategoryError::io(
                &path,
                io::Error::new(io::ErrorKind::InvalidData, "category path 必须是普通目录"),
            )),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                Err(CategoryError::NotFound {
                    category: category.clone(),
                })
            }
            Err(source) => Err(CategoryError::io(&path, source)),
        }
    }

    pub fn list(&self) -> Result<Vec<CategoryName>> {
        check_no_symlinks_under(self.managed_root(), &self.root).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => CategoryError::io(path, source),
            other => CategoryError::io(
                &self.root,
                io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
            ),
        })?;
        let root_metadata = match fs::symlink_metadata(&self.root) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(CategoryError::io(&self.root, source)),
        };
        if !root_metadata.file_type().is_dir() {
            return Err(CategoryError::io(
                &self.root,
                io::Error::new(io::ErrorKind::InvalidData, "definition root 必须是普通目录"),
            ));
        }

        let mut categories = Vec::new();
        for entry in
            fs::read_dir(&self.root).map_err(|source| CategoryError::io(&self.root, source))?
        {
            let entry = entry.map_err(|source| CategoryError::io(&self.root, source))?;
            if !entry
                .file_type()
                .map_err(|source| CategoryError::io(entry.path(), source))?
                .is_dir()
            {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(category) = CategoryName::new(name) {
                categories.push(category);
            }
        }
        categories.sort();
        Ok(categories)
    }

    pub fn delete_empty(&self, category: &CategoryName) -> Result<PathBuf> {
        let path = self.preflight_delete_empty(category)?;

        let readme = path.join(CATEGORY_README_FILE);
        match fs::remove_file(&readme) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(CategoryError::io(&readme, source)),
        }
        fs::remove_dir(&path).map_err(|source| CategoryError::io(&path, source))?;
        Ok(path)
    }

    pub fn preflight_delete_empty(&self, category: &CategoryName) -> Result<PathBuf> {
        if category.is_default() {
            return Err(CategoryError::ReservedDefault);
        }
        let path = self.require(category)?;
        let mut has_definition_entry = false;
        for entry in fs::read_dir(&path).map_err(|source| CategoryError::io(&path, source))? {
            let entry = entry.map_err(|source| CategoryError::io(&path, source))?;
            if entry.file_name() != CATEGORY_README_FILE {
                has_definition_entry = true;
                break;
            }
        }
        if has_definition_entry {
            return Err(CategoryError::NotEmpty {
                category: category.clone(),
            });
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("agt-config-category-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn creates_lists_and_requires_categories() {
        let root = test_root("roundtrip");
        let categories = DefinitionCategories::new(&root);
        categories.ensure_default().unwrap();
        let team = CategoryName::new("team").unwrap();
        categories.create(&team).unwrap();

        assert_eq!(
            categories.list().unwrap(),
            vec![CategoryName::default(), team.clone()]
        );
        assert_eq!(categories.require(&team).unwrap(), root.join("team"));
        assert!(matches!(
            categories.create(&team),
            Err(CategoryError::AlreadyExists { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_rejects_default_and_non_empty_but_allows_readme() {
        let root = test_root("delete");
        let categories = DefinitionCategories::new(&root);
        categories.ensure_default().unwrap();
        assert!(matches!(
            categories.delete_empty(&CategoryName::default()),
            Err(CategoryError::ReservedDefault)
        ));

        let docs = CategoryName::new("docs").unwrap();
        let docs_path = categories.create(&docs).unwrap();
        fs::write(docs_path.join(CATEGORY_README_FILE), "# Docs\n").unwrap();
        categories.delete_empty(&docs).unwrap();

        let team = CategoryName::new("team").unwrap();
        let team_path = categories.create(&team).unwrap();
        fs::create_dir(team_path.join("agent")).unwrap();
        assert!(matches!(
            categories.delete_empty(&team),
            Err(CategoryError::NotEmpty { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_default_rejects_symlink_directory() {
        use std::os::unix::fs::symlink;

        let root = test_root("symlink-default");
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("default")).unwrap();

        let categories = DefinitionCategories::new(&root);
        let error = categories
            .ensure_default()
            .expect_err("default symlink must be rejected");
        assert!(error.to_string().contains("symlink"));

        let _ = fs::remove_dir_all(root);
    }
}
