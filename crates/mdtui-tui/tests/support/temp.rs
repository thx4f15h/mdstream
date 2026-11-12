use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

pub struct TempWorkspace {
    directory: TempDir,
}

impl TempWorkspace {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            directory: tempfile::tempdir()?,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub fn write(
        &self,
        relative: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> io::Result<PathBuf> {
        let relative = relative.as_ref();
        if !relative.is_relative() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "temporary fixture path must be relative",
            ));
        }
        let path = self.directory.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, contents)?;
        Ok(path)
    }
}
