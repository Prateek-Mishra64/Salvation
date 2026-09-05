use std::{
    fs, io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::manifest::{FileEntry, Location, Manifest};

pub struct Transaction {
    pub phantom_root: PathBuf,
    pub phantom_manifest: PathBuf,
    pub manifest: Manifest,
}

impl Transaction {
    pub fn begin(storage_root: &Path, current_manifest: &Manifest) -> io::Result<Self> {
        let phantom_root = storage_root.join("phantom");
        let phantom_manifest = storage_root.join(".phantom");

        if phantom_root.exists() {
            fs::remove_dir_all(&phantom_root)?;
        }

        fs::create_dir_all(&phantom_root)?;

        let mut manifest = current_manifest.clone();

        // The transaction represents one pending manifest mutation.
        manifest.version = manifest.version.saturating_add(1);

        crate::manifest::save(&phantom_manifest, &manifest)?;

        Ok(Self {
            phantom_root,
            phantom_manifest,
            manifest,
        })
    }

    pub fn stage_file(&mut self, relative_path: &str, data: &[u8]) -> io::Result<()> {
        let destination = safe_path(&self.phantom_root, relative_path)?;

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&destination, data)?;

        self.update_manifest(relative_path, &destination)?;

        Ok(())
    }

    fn update_manifest(&mut self, relative_path: &str, path: &Path) -> io::Result<()> {
        let metadata = fs::metadata(path)?;
        let data = fs::read(path)?;

        let hash = Sha256::digest(&data)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let last_modified = metadata
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();

        let location = if crate::lifecycle::is_tmp_path(relative_path) {
            Location::Tmp
        } else {
            Location::Files
        };

        let lifecycle_at = match location {
            Location::Tmp => crate::lifecycle::now(),
            Location::Files | Location::Recall => 0,
        };

        let entry = FileEntry {
            path: relative_path.to_string(),
            location,
            size: metadata.len(),
            hash,
            last_modified,
            lifecycle_at,
        };

        if let Some(existing) = self.manifest.find_file_mut(relative_path) {
            *existing = entry;
        } else {
            self.manifest.add_file(entry);
        }

        crate::manifest::save(&self.phantom_manifest, &self.manifest)?;

        Ok(())
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn commit(
        self,
        files_root: &Path,
        tmp_root: &Path,
        manifest_path: &Path,
    ) -> io::Result<()> {
        for file in &self.manifest.files {
            let source = safe_path(&self.phantom_root, &file.path)?;

            if !source.exists() {
                continue;
            }

            let destination_root = match file.location {
                Location::Files => files_root,
                Location::Tmp => tmp_root,
                Location::Recall => tmp_root,
            };

            let destination = safe_path(destination_root, &file.path)?;

            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }

            if destination.exists() {
                fs::remove_file(&destination)?;
            }

            fs::rename(source, destination)?;
        }

        crate::manifest::save(manifest_path, &self.manifest)?;

        if self.phantom_manifest.exists() {
            fs::remove_file(&self.phantom_manifest)?;
        }

        if self.phantom_root.exists() {
            fs::remove_dir_all(&self.phantom_root)?;
        }

        Ok(())
    }

    pub fn discard(self) -> io::Result<()> {
        if self.phantom_manifest.exists() {
            fs::remove_file(&self.phantom_manifest)?;
        }

        if self.phantom_root.exists() {
            fs::remove_dir_all(&self.phantom_root)?;
        }

        Ok(())
    }
}

fn safe_path(root: &Path, relative_path: &str) -> io::Result<PathBuf> {
    let mut path = root.to_path_buf();

    for component in Path::new(relative_path).components() {
        match component {
            std::path::Component::Normal(part) => {
                path.push(part);
            }

            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid relative path",
                ));
            }
        }
    }

    Ok(path)
}
