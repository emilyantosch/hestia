use anyhow::{Context, Result, ensure};
use std::{
    fmt,
    path::{Component, Path, PathBuf},
};

pub mod decorations;
pub mod file;
pub mod folder;
pub mod tag;
pub mod thumbnail;

/// Absolute UTF-8 entry key. Resolve roots with `CanonPath` first; child symlinks
/// stay at their indexed locations. This also works after an entry is deleted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IndexedPath {
    path: String,
}

impl IndexedPath {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.path
    }
}

impl AsRef<Path> for IndexedPath {
    fn as_ref(&self) -> &Path {
        Path::new(&self.path)
    }
}

impl From<IndexedPath> for PathBuf {
    fn from(path: IndexedPath) -> Self {
        Self::from(path.path)
    }
}

impl TryFrom<&Path> for IndexedPath {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self> {
        ensure!(!path.as_os_str().is_empty(), "indexed path cannot be empty");
        path.to_str().context("indexed path is not valid UTF-8")?;
        let absolute = std::path::absolute(path)?;
        let mut normalized = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::CurDir => (),
                Component::ParentDir => {
                    normalized.pop();
                }
                component => normalized.push(component.as_os_str()),
            }
        }
        Ok(Self {
            path: normalized
                .into_os_string()
                .into_string()
                .map_err(|_| anyhow::anyhow!("indexed path is not valid UTF-8"))?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// Canonicalized successfully at construction; the entry may later disappear.
pub struct CanonPath {
    path: PathBuf,
}

impl fmt::Display for CanonPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path.display().fmt(f)
    }
}

impl TryFrom<PathBuf> for CanonPath {
    type Error = anyhow::Error;
    fn try_from(path: PathBuf) -> Result<CanonPath> {
        Ok(CanonPath {
            path: IndexedPath::try_from(path.canonicalize()?.as_path())?.into(),
        })
    }
}

impl From<CanonPath> for PathBuf {
    fn from(path: CanonPath) -> PathBuf {
        path.path
    }
}

impl AsRef<Path> for CanonPath {
    fn as_ref(&self) -> &Path {
        self.path.as_path()
    }
}

impl CanonPath {
    pub fn try_exists(&self) -> Result<bool> {
        Ok(self.path.try_exists()?)
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.path.to_str()
    }
}

#[cfg(test)]
mod tests {
    use super::{CanonPath, IndexedPath};
    use anyhow::Result;
    use std::path::Path;

    #[test]
    fn entry_keys_are_absolute_lexical_and_do_not_require_existence() -> Result<()> {
        let cwd = std::env::current_dir()?;
        assert_eq!(
            IndexedPath::try_from(Path::new("./missing/../entry"))?.as_ref(),
            cwd.join("entry")
        );
        assert_eq!(
            IndexedPath::try_from(Path::new("/../../entry/./"))?.as_ref(),
            Path::new("/entry")
        );
        assert_eq!(
            IndexedPath::try_from(Path::new("/entry"))?.as_str(),
            "/entry"
        );
        assert!(IndexedPath::try_from(Path::new("")).is_err());
        assert!(CanonPath::try_from(cwd.join("missing/entry")).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn entry_keys_reject_non_utf8() {
        use std::os::unix::ffi::OsStrExt;
        assert!(
            IndexedPath::try_from(Path::new(std::ffi::OsStr::from_bytes(b"/bad\xff"))).is_err()
        );
    }
}
