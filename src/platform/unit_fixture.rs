//! Library-test adapter for the retained fixture boundary. No cleanup on Drop.
//! A private parent must be supplied before the test process starts.
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

static RUN: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(0);

fn root() -> io::Result<PathBuf> {
    match RUN.get_or_init(|| {
        super::test_fixture::start()
            .map(|root| {
                eprintln!("UNIT_FIXTURE_RETAINED {}", root.display());
                root
            })
            .map_err(|error| error.to_string())
    }) {
        Ok(root) => Ok(root.clone()),
        Err(message) => Err(io::Error::new(io::ErrorKind::PermissionDenied, message.clone())),
    }
}

/// Allocate a new case under this process's retained run. Parallel tests
/// share initialization, but never share a case directory or clean old runs.
pub(crate) fn directory(tag: &str) -> io::Result<PathBuf> {
    if tag.is_empty() || tag.len() > 32
        || !tag.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid unit fixture tag"));
    }
    let run = root()?;
    // The empty relative path asks the shared boundary to recheck this run
    // before allocation, without adding or reusing a child directory.
    let verified = super::test_fixture::directory("")?;
    if verified != run {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "unit fixture root changed"));
    }
    let sequence = NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "unit fixture IDs exhausted"))?;
    let relative = format!("u-{tag}-{sequence}");
    let path = run.join(&relative);
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let mut builder = builder;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Exclusive allocation: a collision is an error, never an old case to
    // reuse or clean up. Then validate through the shared fixture boundary.
    builder.create(&path)?;
    super::test_fixture::directory(relative)
}

/// Retire a path by moving it to a new evidence directory, without deleting
/// it. The underlying boundary rejects root, traversal, and linked paths.
pub(crate) fn preserve(path: impl AsRef<Path>) -> io::Result<Option<PathBuf>> {
    let _ = root()?;
    super::test_fixture::preserve(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn repeated_cases_keep_prior_bytes_and_distinct_paths() {
        let first = directory("repeat").unwrap();
        fs::write(first.join("marker"), b"first run evidence").unwrap();
        let second = directory("repeat").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first.join("marker")).unwrap(), b"first run evidence");
        assert!(fs::read_dir(second).unwrap().next().is_none());
    }

    #[test]
    fn missing_path_simulation_retains_each_version() {
        let case = directory("preserve").unwrap();
        let path = case.join("colors.toml");
        fs::write(&path, b"first").unwrap();
        let first = preserve(&path).unwrap().unwrap();
        assert!(fs::symlink_metadata(&path).is_err());
        fs::write(&path, b"second").unwrap();
        let second = preserve(&path).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first).unwrap(), b"first");
        assert_eq!(fs::read(second).unwrap(), b"second");
        assert!(preserve(path).unwrap().is_none());
    }

    #[test]
    fn invalid_tags_and_root_moves_are_refused() {
        for tag in ["", ".", "..", "../escape", "one/two", "one\\two"] {
            assert_eq!(directory(tag).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        let run = root().unwrap();
        assert!(preserve(&run).is_err());
        assert!(preserve(run.join("..").join("outside")).is_err());
        assert!(fs::symlink_metadata(run).unwrap().is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn fixture_alias_is_refused_without_following_its_target() {
        let case = directory("linked").unwrap();
        let target = directory("target").unwrap();
        let marker = target.join("marker");
        fs::write(&marker, b"retained target").unwrap();
        let alias = case.join("alias");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        assert!(preserve(&alias).is_err());
        assert!(fs::symlink_metadata(alias).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(marker).unwrap(), b"retained target");
    }
}
