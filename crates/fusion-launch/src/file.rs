//! The layout file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use fusion_layout::{Layout, LayoutError};

/// Why the layout file could not be read or written.
#[derive(Debug)]
pub enum FileError {
    /// The file system refused.
    Io(io::Error),
    /// The file is not a layout this build can read.
    Layout(LayoutError),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => err.fmt(f),
            Self::Layout(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for FileError {}

/// Where the launcher keeps the layout, as JSON.
#[derive(Debug, Clone)]
pub struct LayoutFile {
    path: PathBuf,
}

impl LayoutFile {
    /// The layout file at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Where the file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved layout, or `None` if nothing is saved yet.
    pub fn load(&self) -> Result<Option<Layout>, FileError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Layout::from_json(&text)
                .map(Some)
                .map_err(FileError::Layout),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FileError::Io(err)),
        }
    }

    /// Saves the layout. It writes a new file and then moves it over the
    /// old one, so a crash halfway leaves the old file whole.
    pub fn save(&self, layout: &Layout) -> Result<(), FileError> {
        let json = layout.to_json().map_err(FileError::Layout)?;
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder).map_err(FileError::Io)?;
        }
        let new = self.path.with_extension("json.new");
        fs::write(&new, json).map_err(FileError::Io)?;
        fs::rename(&new, &self.path).map_err(FileError::Io)
    }
}
