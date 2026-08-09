use anyhow::Result;
use std::{
    fmt,
    path::{Path, PathBuf},
};

pub mod decorations;
pub mod file;
pub mod folder;
pub mod tag;
pub mod thumbnail;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// This path is always considered to exist and to be canonical
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
            path: path.canonicalize()?,
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
