use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::manifest::{Location, Manifest};

pub const TMP_RETENTION: u64 = 60;
pub const RECALL_RETENTION: u64 = 60;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_secs()
}

pub fn expired(created_at: u64, current_time: u64, retention: u64) -> bool {
    current_time.saturating_sub(created_at) >= retention
}

pub fn is_tmp_path(path: &str) -> bool {
    let normalized = path.trim_start_matches('/');

    normalized == "home/prateek/tmp" || normalized.starts_with("home/prateek/tmp/")
}

pub fn move_tmp_to_recall(
    tmp_root: &Path,
    recall_root: &Path,
    relative_path: &str,
) -> io::Result<PathBuf> {
    move_file(tmp_root, recall_root, relative_path)
}

pub fn move_recall_to_tmp(
    recall_root: &Path,
    tmp_root: &Path,
    relative_path: &str,
) -> io::Result<PathBuf> {
    move_file(recall_root, tmp_root, relative_path)
}

pub fn annihilate(recall_root: &Path, relative_path: &str) -> io::Result<()> {
    let path = safe_path(recall_root, relative_path)?;
    fs::remove_file(path)
}

fn move_file(
    source_root: &Path,
    destination_root: &Path,
    relative_path: &str,
) -> io::Result<PathBuf> {
    let source = safe_path(source_root, relative_path)?;
    let destination = safe_path(destination_root, relative_path)?;

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::rename(&source, &destination)?;
    Ok(destination)
}

fn safe_path(root: &Path, relative_path: &str) -> io::Result<PathBuf> {
    let mut path = root.to_path_buf();

    for component in Path::new(relative_path).components() {
        match component {
            std::path::Component::Normal(part) => path.push(part),
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

pub fn process_manifest(
    manifest: &mut Manifest,
    tmp_root: &Path,
    recall_root: &Path,
    current_time: u64,
) -> io::Result<()> {
    let mut remove = Vec::new();

    for (index, file) in manifest.files.iter_mut().enumerate() {
        let age = current_time.saturating_sub(file.lifecycle_at);

        match file.location {
            Location::Tmp if age >= TMP_RETENTION => {
                move_tmp_to_recall(tmp_root, recall_root, &file.path)?;

                file.location = Location::Recall;
                file.lifecycle_at = current_time;
            }

            Location::Recall if age >= RECALL_RETENTION => {
                annihilate(recall_root, &file.path)?;
                remove.push(index);
            }

            _ => {}
        }
    }

    for index in remove.into_iter().rev() {
        manifest.files.remove(index);
    }

    Ok(())
}

pub fn process_manifest_file(
    manifest_path: &Path,
    tmp_root: &Path,
    recall_root: &Path,
) -> io::Result<()> {
    let mut manifest = crate::manifest::load(manifest_path)?;
    process_manifest(&mut manifest, tmp_root, recall_root, now())?;
    crate::manifest::save(manifest_path, &manifest)?;
    Ok(())
}

pub fn promote_to_files(
    phantom_root: &Path,
    files_root: &Path,
    relative_path: &str,
) -> io::Result<PathBuf> {
    move_file(phantom_root, files_root, relative_path)
}
