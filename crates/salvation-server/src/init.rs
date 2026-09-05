use std::{fs, io, path::PathBuf};

pub fn initialize() -> io::Result<()> {
    let storage_root = PathBuf::from(
        std::env::var("Salvation_Storage").unwrap_or_else(|_| "./salvation-data".into()),
    );

    fs::create_dir_all(storage_root.join("files"))?;
    fs::create_dir_all(storage_root.join("tmp"))?;
    fs::create_dir_all(storage_root.join("recall"))?;
    fs::create_dir_all(storage_root.join("phantom"))?;
    fs::create_dir_all(storage_root.join("logs"))?;

    let manifest_path = storage_root.join(".manifest");

    if !manifest_path.exists() {
        crate::manifest::save(&manifest_path, &crate::manifest::Manifest::empty())?;
    }

    let salvation_path = storage_root.join(".salvation");

    if !salvation_path.exists() {
        fs::write(&salvation_path, "# Salvation server mapping\n")?;
    }

    println!("Salvation server initialized.");

    Ok(())
}
