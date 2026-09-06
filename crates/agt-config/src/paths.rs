use std::env;
use std::path::PathBuf;

use agt_core::{AgtError, CategoryName, Result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgtPaths {
    home: PathBuf,
}

impl AgtPaths {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn from_default_home() -> Result<Self> {
        Ok(Self::new(default_agt_home()?))
    }

    pub fn home(&self) -> &PathBuf {
        &self.home
    }

    pub fn config_file(&self) -> PathBuf {
        self.home.join("config.yaml")
    }

    pub fn data_dir(&self, name: &str) -> PathBuf {
        self.home.join(name)
    }

    pub fn category_dir(&self, domain: &str, category: &CategoryName) -> PathBuf {
        self.data_dir(domain).join(category.as_str())
    }

    pub fn default_category_dir(&self, domain: &str) -> PathBuf {
        self.category_dir(domain, &CategoryName::default())
    }
}

pub fn default_agt_home() -> Result<PathBuf> {
    let home = user_home_dir().ok_or(AgtError::HomeDirUnavailable)?;
    Ok(home.join(".agt"))
}

pub fn user_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_relative_to_home() {
        let paths = AgtPaths::new("/tmp/example-agt");

        assert_eq!(
            paths.config_file(),
            PathBuf::from("/tmp/example-agt/config.yaml")
        );
        assert_eq!(
            paths.data_dir("providers"),
            PathBuf::from("/tmp/example-agt/providers")
        );
        assert_eq!(
            paths.category_dir(
                "providers",
                &CategoryName::new("internal").expect("valid category")
            ),
            PathBuf::from("/tmp/example-agt/providers/internal")
        );
        assert_eq!(
            paths.default_category_dir("agents"),
            PathBuf::from("/tmp/example-agt/agents/default")
        );
    }
}
