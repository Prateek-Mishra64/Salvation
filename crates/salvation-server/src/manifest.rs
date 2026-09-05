use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Manifest {
    pub version: u64,

    #[serde(default)]
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    pub location: Location,
    pub size: u64,
    pub hash: String,
    pub last_modified: u64,
    pub lifecycle_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Location {
    Files,
    Tmp,
    Recall,
}

impl Manifest {
    pub fn empty() -> Self {
        Self {
            version: 0,
            files: Vec::new(),
        }
    }

    pub fn add_file(&mut self, file: FileEntry) {
        self.files.push(file);
    }

    pub fn find_file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|file| file.path == path)
    }

    pub fn find_file_mut(&mut self, path: &str) -> Option<&mut FileEntry> {
        self.files.iter_mut().find(|file| file.path == path)
    }

    pub fn update_location(&mut self, path: &str, location: Location, lifecycle_at: u64) -> bool {
        let Some(file) = self.find_file_mut(path) else {
            return false;
        };

        file.location = location;
        file.lifecycle_at = lifecycle_at;

        true
    }

    pub fn remove_file(&mut self, path: &str) -> Option<FileEntry> {
        let index = self.files.iter().position(|file| file.path == path)?;
        Some(self.files.remove(index))
    }
}

pub fn load(path: &Path) -> io::Result<Manifest> {
    let contents = fs::read_to_string(path)?;
    toml::from_str(&contents).map_err(io::Error::other)
}

pub fn save(path: &Path, manifest: &Manifest) -> io::Result<()> {
    let contents = toml::to_string_pretty(manifest).map_err(io::Error::other)?;
    fs::write(path, contents)
}
