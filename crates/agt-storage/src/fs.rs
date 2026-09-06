use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use agt_core::{AgtError, Result};

/// Ensure a directory exists. Use [`ensure_dir_under`] for application-managed
/// paths where symlink components must be rejected.
pub fn ensure_dir(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    fs::create_dir_all(path).map_err(|source| AgtError::io(path, source))
}

/// Ensure a directory exists without following symlinks below `root`.
///
/// Parent paths above `root` are intentionally not inspected. This keeps
/// normal platform paths such as macOS's `/var -> /private/var` usable while
/// protecting the application's managed directory tree.
pub fn ensure_dir_under(root: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<()> {
    let root = root.as_ref();
    let path = path.as_ref();
    let relative = managed_relative(root, path)?;

    ensure_managed_root(root).map_err(|source| AgtError::io(root, source))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                current.push(name);
                ensure_directory_component(&current)
                    .map_err(|source| AgtError::io(&current, source))?;
            }
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(AgtError::invalid_input(format!(
                    "managed path {} contains an unsafe component",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Check an existing path below a trusted managed root without creating it or
/// following symlinks. Missing leaf components are allowed.
pub fn check_no_symlinks_under(root: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<()> {
    let root = root.as_ref();
    let path = path.as_ref();
    let relative = managed_relative(root, path)?;

    let mut current = root.to_path_buf();
    match fs::symlink_metadata(&current) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(AgtError::io(
                &current,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("managed path cannot be a symlink: {}", current.display()),
                ),
            ));
        }
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(AgtError::io(&current, source)),
    }

    for component in relative.components() {
        if let Component::Normal(name) = component {
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(AgtError::io(
                        &current,
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("managed path cannot be a symlink: {}", current.display()),
                        ),
                    ));
                }
                Ok(_) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(source) => return Err(AgtError::io(&current, source)),
            }
        }
    }
    Ok(())
}

fn managed_relative<'a>(root: &Path, path: &'a Path) -> Result<&'a Path> {
    let relative = if root == Path::new(".") || root.as_os_str().is_empty() {
        path
    } else {
        path.strip_prefix(root).map_err(|_| {
            AgtError::invalid_input(format!(
                "managed path {} is outside root {}",
                path.display(),
                root.display()
            ))
        })?
    };
    if relative.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return Err(AgtError::invalid_input(format!(
            "managed path {} contains an unsafe component",
            path.display()
        )));
    }
    Ok(relative)
}

fn ensure_managed_root(root: &Path) -> io::Result<()> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("managed root cannot be a symlink: {}", root.display()),
        )),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("managed root is not a directory: {}", root.display()),
        )),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(root)?;
            ensure_managed_root(root)
        }
        Err(source) => Err(source),
    }
}

fn ensure_directory_component(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("managed directory cannot be a symlink: {}", path.display()),
        )),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("path component is not a directory: {}", path.display()),
        )),
        Err(source) if source.kind() == io::ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                ensure_directory_component(path)
            }
            Err(source) => Err(source),
        },
        Err(source) => Err(source),
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replace a file's contents without exposing a partially written file.
///
/// The contents are written to a newly created sibling file, flushed to the
/// filesystem, and then atomically renamed over the destination. If the
/// destination already exists, its Unix permission bits are preserved.
/// This does not serialize independent read-modify-write operations.
pub fn write_atomic(path: impl AsRef<Path>, content: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write target must have a file name",
        )
    })?;

    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    let existing_mode = existing_file_mode(path);
    let (temporary, mut staged) = create_temporary_file(parent, file_name)?;
    let result = (|| {
        staged.write_all(content.as_ref())?;
        staged.sync_all()?;

        #[cfg(unix)]
        if let Some(mode) = existing_mode {
            staged.set_permissions(fs::Permissions::from_mode(mode))?;
            staged.sync_all()?;
        }

        drop(staged);
        fs::rename(&temporary, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Atomically write a file whose parent is inside a trusted managed root.
pub fn write_atomic_under(
    root: impl AsRef<Path>,
    path: impl AsRef<Path>,
    content: impl AsRef<[u8]>,
) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_dir_under(&root, parent).map_err(core_error_to_io)?;
    check_no_symlinks_under(&root, path).map_err(core_error_to_io)?;
    write_atomic(path, content)
}

pub fn write_file_if_missing(path: impl AsRef<Path>, content: &str) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        ensure_dir(parent)?;
    }

    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(source) => return Err(AgtError::io(path, source)),
    };
    let result = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|source| AgtError::io(path, source));
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

/// Initialize a file below a trusted managed root without following symlinks
/// in the managed parent directories.
pub fn write_file_if_missing_under(
    root: impl AsRef<Path>,
    path: impl AsRef<Path>,
    content: &str,
) -> Result<()> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_dir_under(&root, parent)?;
    check_no_symlinks_under(&root, path)?;

    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(source) => return Err(AgtError::io(path, source)),
    };
    let result = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|source| AgtError::io(path, source));
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn core_error_to_io(error: AgtError) -> io::Error {
    match error {
        AgtError::Io { source, .. } => source,
        AgtError::InvalidInput { message } => io::Error::new(io::ErrorKind::InvalidInput, message),
        AgtError::HomeDirUnavailable => {
            io::Error::new(io::ErrorKind::InvalidInput, "cannot resolve managed root")
        }
    }
}

fn create_temporary_file(
    parent: &Path,
    file_name: &std::ffi::OsStr,
) -> io::Result<(std::path::PathBuf, File)> {
    for _ in 0..1000 {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{}.tmp-{}-{suffix}",
            file_name.to_string_lossy(),
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(source),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "unable to allocate a unique temporary file for atomic write",
    ))
}

#[cfg(unix)]
fn existing_file_mode(path: &Path) -> Option<u32> {
    fs::symlink_metadata(path).ok().and_then(|metadata| {
        metadata
            .file_type()
            .is_file()
            .then(|| metadata.permissions().mode() & 0o7777)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn atomic_write_replaces_contents_and_cleans_up_temporary_file() {
        let dir = std::env::temp_dir().join(format!(
            "agt-storage-atomic-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let file = dir.join("config.yaml");

        let _ = fs::remove_dir_all(&dir);
        write_atomic(&file, "first").expect("write first file");
        write_atomic(&file, "second").expect("replace file");

        assert_eq!(fs::read_to_string(&file).expect("read file"), "second");
        let entries = fs::read_dir(&dir)
            .expect("read directory")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect directory entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_name(), "config.yaml");

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "agt-storage-mode-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let file = dir.join("config.yaml");

        let _ = fs::remove_dir_all(&dir);
        write_atomic(&file, "first").expect("write first file");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600))
            .expect("set file permissions");
        write_atomic(&file, "second").expect("replace file");

        let mode = fs::metadata(&file)
            .expect("read file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_dir_under_rejects_symlink_components() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "agt-storage-symlink-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let outside = root.join("outside");
        let linked = root.join("linked");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&outside).expect("create outside directory");
        symlink(&outside, &linked).expect("create directory symlink");

        let error = ensure_dir_under(&root, linked.join("child")).expect_err("reject symlink");
        assert!(error.to_string().contains("symlink"));
        assert!(!outside.join("child").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn managed_writes_reject_symlink_targets() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "agt-storage-write-symlink-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let outside = root.join("outside");
        let target = root.join("target");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&outside).expect("create outside directory");
        fs::write(outside.join("config.yaml"), "original").expect("write outside file");
        symlink(outside.join("config.yaml"), &target).expect("create file symlink");

        let error = write_atomic_under(&root, &target, "replacement")
            .expect_err("managed write must reject symlink target");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            fs::read_to_string(outside.join("config.yaml")).unwrap(),
            "original"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn writes_file_without_overwriting_existing_content() {
        let dir = std::env::temp_dir().join(format!("agt-storage-test-{}", std::process::id()));
        let file = dir.join("config.yaml");

        let _ = fs::remove_dir_all(&dir);
        write_file_if_missing(&file, "first").expect("write first file");
        write_file_if_missing(&file, "second").expect("skip existing file");

        let content = fs::read_to_string(&file).expect("read test file");
        assert_eq!(content, "first");

        let _ = fs::remove_dir_all(&dir);
    }
}
