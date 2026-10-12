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

/// Environment variable that overrides the default `~/.agt` home.
pub const AGT_HOME_ENV: &str = "AGT_HOME";

/// Returns `$AGT_HOME` when it is set and non-empty, otherwise `~/.agt`.
pub fn default_agt_home() -> Result<PathBuf> {
    resolve_default_home(
        env::var_os(AGT_HOME_ENV).map(PathBuf::from),
        user_home_dir(),
    )
}

fn resolve_default_home(agt_home: Option<PathBuf>, user_home: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(home) = agt_home.filter(|home| !home.as_os_str().is_empty()) {
        if !home.is_absolute() {
            return Err(AgtError::invalid_input(format!(
                "{AGT_HOME_ENV} 必须是绝对路径：{}",
                home.display()
            )));
        }
        return Ok(home);
    }
    let home = user_home.ok_or(AgtError::HomeDirUnavailable)?;
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

    #[test]
    fn agt_home_overrides_the_user_home_default() {
        let user = Some(PathBuf::from("/home/someone"));

        assert_eq!(
            resolve_default_home(None, user.clone()).expect("default"),
            PathBuf::from("/home/someone/.agt")
        );
        assert_eq!(
            resolve_default_home(Some(PathBuf::new()), user.clone()).expect("empty is unset"),
            PathBuf::from("/home/someone/.agt")
        );
        assert_eq!(
            resolve_default_home(Some(PathBuf::from("/srv/agt")), None).expect("override"),
            PathBuf::from("/srv/agt")
        );
        assert!(resolve_default_home(Some(PathBuf::from("agt")), user).is_err());
    }
}
