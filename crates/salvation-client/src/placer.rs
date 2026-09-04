use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::mapper::FileMapping;

pub fn place_file(file: &Path, mapping: &FileMapping, root: &Path) -> io::Result<PathBuf> {
    let destination_dir = find_destination(root, &mapping.parent_vector).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("No matching directory found for {}", file.display()),
        )
    })?;

    let file_name = file
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "File has no filename"))?;

    let destination = destination_dir.join(file_name);

    if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("Destination already exists: {}", destination.display()),
        ));
    }

    fs::rename(file, &destination)?;

    Ok(destination)
}

fn find_destination(root: &Path, parents: &[String]) -> Option<PathBuf> {
    if parents.is_empty() {
        return Some(root.to_path_buf());
    }

    // 1. a/b/c
    let full = parents
        .iter()
        .fold(root.to_path_buf(), |path, parent| path.join(parent));

    if full.is_dir() {
        return Some(full);
    }

    // 2. c
    let deepest = root.join(parents.last()?);

    if deepest.is_dir() {
        return Some(deepest);
    }

    // 3. b/c
    if parents.len() >= 2 {
        let last_two = root
            .join(&parents[parents.len() - 2])
            .join(&parents[parents.len() - 1]);

        if last_two.is_dir() {
            return Some(last_two);
        }
    }

    None
}
