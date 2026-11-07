//! Test-only fixture boundary, included by the UI harness without a public API.
//! Every run is new and retained. No cleanup removes a file or directory tree.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{atomic::{AtomicU64, Ordering}, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

static RUN: OnceLock<PathBuf> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(0);

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn checked(path: &Path) -> io::Result<fs::Metadata> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid("fixture paths must be absolute without parent traversal"));
    }
    // Inspect each component without canonicalize or directory traversal.
    // The final private parent excludes other users before any run is created.
    let mut parts: Vec<_> = path.ancestors().collect();
    parts.reverse();
    for part in parts {
        let metadata = fs::symlink_metadata(part)?;
        if metadata.file_type().is_symlink() {
            return Err(invalid("fixture paths cannot contain symlinks"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(invalid("fixture paths cannot contain reparse points"));
            }
        }
    }
    fs::symlink_metadata(path)
}

fn private_directory(path: &Path) -> io::Result<()> {
    let metadata = checked(path)?;
    if !metadata.is_dir() {
        return Err(invalid("fixture parent must be a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no arguments and does not modify process state.
        let uid = unsafe { libc::geteuid() };
        if metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
            return Err(invalid("fixture parent must be owned by this user with mode 0700"));
        }
    }
    // Windows directories inherit the caller-supplied private parent's ACL.
    Ok(())
}

fn create_private(path: &Path) -> io::Result<()> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    private_directory(path)
}

fn fresh_in(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    if prefix.is_empty() || !prefix.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err(invalid("fixture directory prefixes must be simple identifiers"));
    }
    private_directory(parent)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| invalid("fixture clock precedes the Unix epoch"))?.as_nanos();
    for _ in 0..32 {
        let path = parent.join(format!("{prefix}-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match create_private(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(invalid("could not allocate a new fixture directory"))
}

pub(crate) fn start() -> io::Result<PathBuf> {
    if RUN.get().is_some() {
        return Err(invalid("UI fixture run is already initialized"));
    }
    let parent = std::env::var_os("OMARCHY_FLTK_UI_PARENT")
        .or_else(|| std::env::var_os("TMPDIR"))
        .ok_or_else(|| invalid("set a private OMARCHY_FLTK_UI_PARENT or TMPDIR before running UI tests"))?;
    let path = fresh_in(Path::new(&parent), "ui")?;
    RUN.set(path.clone()).map_err(|_| invalid("UI fixture run is already initialized"))?;
    Ok(path)
}

pub(crate) fn root() -> PathBuf {
    RUN.get().expect("UI fixture run must be initialized first").clone()
}

pub(crate) fn directory(relative: impl AsRef<Path>) -> io::Result<PathBuf> {
    let relative = relative.as_ref();
    if relative.is_absolute() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(invalid("fixture children must be relative normal components"));
    }
    let mut path = root();
    private_directory(&path)?;
    for component in relative.components() {
        path.push(component.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(_) => private_directory(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => create_private(&path)?,
            Err(error) => return Err(error),
        }
    }
    Ok(path)
}

/// Make a fixture appear absent while retaining its original bytes for review.
pub(crate) fn preserve(path: impl AsRef<Path>) -> io::Result<Option<PathBuf>> {
    let path = path.as_ref();
    let root = root();
    if path == root || !path.starts_with(&root) {
        return Err(invalid("only strict children of this UI run may be preserved"));
    }
    match checked(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let retained = fresh_in(&root, "preserved")?;
    let target = retained.join(path.file_name().ok_or_else(|| invalid("fixture child has no name"))?);
    fs::rename(path, &target)?;
    Ok(Some(target))
}
