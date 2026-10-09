pub mod fs;
pub mod sqlite;
pub mod wake;

pub use fs::{
    check_no_symlinks_under, ensure_dir, ensure_dir_under, write_atomic, write_atomic_under,
    write_file_if_missing, write_file_if_missing_under,
};

/// Resolve a managed SQLite database and reject symlinked sidecar files.
pub fn database_path(
    home: &std::path::Path,
    directory: &str,
    name: &str,
) -> agt_core::Result<std::path::PathBuf> {
    let dir = home.join(directory);
    ensure_dir_under(home, &dir)?;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        check_no_symlinks_under(home, dir.join(format!("{name}{suffix}")))?;
    }
    Ok(dir.join(name))
}
