use std::path::PathBuf;

use agt_core::Result;
use agt_storage::{ensure_dir_under, write_file_if_missing_under};

use crate::paths::{AgtPaths, default_agt_home};

const DEFAULT_CONFIG: &str = "# agt 用户配置\n";

const DEFAULT_DIRS: &[&str] = &["providers", "models", "skills", "mcps", "prompts", "agents"];

const DEFINITION_DIRS: &[&str] = &["providers", "models", "skills", "mcps", "prompts", "agents"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitSummary {
    pub home: PathBuf,
    pub config_file: PathBuf,
    pub data_dirs: Vec<PathBuf>,
}

/// Ensure the default `~/.agt` home has the base directory layout.
///
/// This is intended for the installed CLI's normal startup path.  Explicit
/// project homes should use [`initialize_home`] with their target directory.
pub fn initialize_default_home() -> Result<InitSummary> {
    initialize_home(None)
}

pub fn initialize_home(home: Option<PathBuf>) -> Result<InitSummary> {
    let home = match home {
        Some(path) => path,
        None => default_agt_home()?,
    };
    let paths = AgtPaths::new(home);

    ensure_dir_under(paths.home(), paths.home())?;

    let config_file = paths.config_file();
    write_file_if_missing_under(paths.home(), &config_file, DEFAULT_CONFIG)?;

    let mut data_dirs = Vec::with_capacity(DEFAULT_DIRS.len());
    for dir_name in DEFAULT_DIRS {
        let dir = paths.data_dir(dir_name);
        ensure_dir_under(paths.home(), &dir)?;
        if DEFINITION_DIRS.contains(dir_name) {
            ensure_dir_under(paths.home(), paths.default_category_dir(dir_name))?;
        }
        data_dirs.push(dir);
    }

    Ok(InitSummary {
        home: paths.home().clone(),
        config_file,
        data_dirs,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn initialize_home_creates_default_layout_under_explicit_home() {
        let home = std::env::temp_dir().join(format!("agt-config-test-{}", std::process::id()));

        let _ = fs::remove_dir_all(&home);
        let summary = initialize_home(Some(home.clone())).expect("initialize agt home");

        assert_eq!(summary.home, home);
        assert!(summary.config_file.is_file());
        for dir in ["providers", "models", "skills", "mcps", "prompts", "agents"] {
            assert!(summary.home.join(dir).is_dir(), "missing {dir}");
        }
        for domain in DEFINITION_DIRS {
            assert!(
                summary.home.join(domain).join("default").is_dir(),
                "missing {domain}/default"
            );
        }

        let _ = fs::remove_dir_all(&summary.home);
    }

    #[cfg(unix)]
    #[test]
    fn initialize_home_rejects_symlinked_managed_directory() {
        use std::os::unix::fs::symlink;

        let home =
            std::env::temp_dir().join(format!("agt-config-symlink-home-{}", std::process::id()));
        let outside =
            std::env::temp_dir().join(format!("agt-config-symlink-outside-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).expect("create outside directory");
        fs::create_dir_all(&home).expect("create home");
        symlink(&outside, home.join("providers")).expect("create providers symlink");

        let error = initialize_home(Some(home.clone())).expect_err("reject providers symlink");
        assert!(error.to_string().contains("symlink"));
        assert!(!outside.join("default").exists());

        let _ = fs::remove_dir_all(&home);
        let _ = fs::remove_dir_all(&outside);
    }
}
