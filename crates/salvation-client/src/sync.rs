use std::{
    fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::mapper::{self, FileMapping};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u64,

    #[serde(default)]
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileEntry {
    path: String,

    #[serde(default)]
    location: String,

    #[serde(default)]
    size: u64,

    #[serde(default)]
    hash: String,

    #[serde(default)]
    last_modified: u64,

    #[serde(default)]
    lifecycle_at: u64,
}

impl Manifest {
    fn find_file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|file| file.path == path)
    }
}

pub async fn synchronize(server: &str, password: &str) -> io::Result<()> {
    let mappings = mapper::load_mappings()?;

    if mappings.is_empty() {
        println!("No managed files.");
        return Ok(());
    }

    let client = Client::new();

    let server_manifest = get_server_manifest(&client, server, password).await?;

    let previous_manifest = load_previous_server_manifest();

    println!(
        "Synchronizing {} managed file(s) against server manifest v{}...",
        mappings.len(),
        server_manifest.version
    );

    let mut uploads: Vec<&FileMapping> = Vec::new();
    let mut downloads: Vec<(&FileMapping, String)> = Vec::new();

    for mapping in &mappings {
        let logical = logical_path(mapping)?;

        /*
         * Local file is missing.
         *
         * If the server has it, restore it.
         * If the server does not have it, there is nothing to do.
         */
        if !mapping.full_path.is_file() {
            if server_manifest.find_file(&logical).is_some() {
                println!("{logical}: local file missing -> DOWNLOAD");

                downloads.push((mapping, logical));
            }

            continue;
        }

        let local = local_metadata(&mapping.full_path)?;

        match server_manifest.find_file(&logical) {
            /*
             * Server does not currently have this file.
             *
             * If it was previously known, the server deliberately
             * removed it (for example through annihilation), so
             * remove the local copy.
             *
             * Otherwise this is a genuinely new local file, so upload it.
             */
            None => {
                let was_previously_known = previous_manifest
                    .as_ref()
                    .and_then(|manifest| manifest.find_file(&logical))
                    .is_some();

                if was_previously_known {
                    println!("{logical}: removed from server -> DELETE LOCAL");

                    fs::remove_file(&mapping.full_path)?;
                } else {
                    println!("{logical}: new local file -> UPLOAD");

                    uploads.push(mapping);
                }
            }

            Some(remote) if remote.hash == local.hash => {
                println!("{logical}: identical");
            }

            Some(remote) if local.last_modified >= remote.last_modified => {
                println!("{logical}: local differs and is newer -> UPLOAD");

                uploads.push(mapping);
            }

            Some(_) => {
                println!("{logical}: server differs and is newer -> DOWNLOAD");

                downloads.push((mapping, logical));
            }
        }
    }

    if !uploads.is_empty() {
        upload_files(&client, server, password, &uploads).await?;
    }

    for (mapping, logical) in &downloads {
        download_file(&client, server, password, logical, &mapping.full_path).await?;
    }

    /*
     * Store the latest authoritative server manifest locally.
     *
     * This is used on the next synchronization to distinguish:
     *
     *   genuinely new local file -> UPLOAD
     *
     * from:
     *
     *   previously known file removed by server -> DELETE LOCAL
     */
    let current_manifest = toml::to_string(&server_manifest).map_err(io::Error::other)?;

    save_local_server_manifest(&current_manifest)?;

    if uploads.is_empty() && downloads.is_empty() {
        println!("Synchronization complete: nothing to transfer.");
    } else {
        println!("Synchronization complete.");
    }

    Ok(())
}

async fn get_server_manifest(
    client: &Client,
    server: &str,
    password: &str,
) -> io::Result<Manifest> {
    let response = client
        .get(server_url(server, &["manifest-info"])?)
        .header("Authorization", format!("Bearer {password}"))
        .send()
        .await
        .map_err(io::Error::other)?;

    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "manifest request failed: {}",
            response.status()
        )));
    }

    let text = response.text().await.map_err(io::Error::other)?;

    toml::from_str(&text).map_err(io::Error::other)
}

async fn upload_files(
    client: &Client,
    server: &str,
    password: &str,
    mappings: &[&FileMapping],
) -> io::Result<()> {
    let mut staged = false;

    for mapping in mappings {
        let data = fs::read(&mapping.full_path)?;

        let logical = logical_path(mapping)?;

        println!("Uploading {} as {}", mapping.full_path.display(), logical);

        let response = client
            .post(server_url(server, &["files", &logical])?)
            .header("Authorization", format!("Bearer {password}"))
            .body(data)
            .send()
            .await
            .map_err(io::Error::other)?;

        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "upload failed for {logical}: {}",
                response.status()
            )));
        }

        let _ = response.text().await.map_err(io::Error::other)?;

        staged = true;
    }

    if !staged {
        return Ok(());
    }

    println!("Committing upload transaction...");

    let response = client
        .post(server_url(server, &["transaction", "confirm"])?)
        .header("Authorization", format!("Bearer {password}"))
        .send()
        .await
        .map_err(io::Error::other)?;

    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "transaction confirmation failed: {}",
            response.status()
        )));
    }

    let committed = response.text().await.map_err(io::Error::other)?;

    save_local_server_manifest(&committed)?;

    println!("Upload transaction committed.");

    Ok(())
}

async fn download_file(
    client: &Client,
    server: &str,
    password: &str,
    logical_path: &str,
    destination: &Path,
) -> io::Result<()> {
    let response = client
        .get(server_url(server, &["files", logical_path])?)
        .header("Authorization", format!("Bearer {password}"))
        .send()
        .await
        .map_err(io::Error::other)?;

    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "download failed for {logical_path}: {}",
            response.status()
        )));
    }

    let data = response.bytes().await.map_err(io::Error::other)?;

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(destination, &data)?;

    println!("Downloaded {logical_path} -> {}", destination.display());

    Ok(())
}

fn logical_path(mapping: &FileMapping) -> io::Result<String> {
    let filename = mapping
        .full_path
        .file_name()
        .ok_or_else(|| io::Error::other("managed file has no filename"))?;

    let mut parts = mapping.parent_vector.clone();

    parts.push(filename.to_string_lossy().into_owned());

    Ok(parts.join("/"))
}

struct LocalMetadata {
    hash: String,
    last_modified: u64,
}

fn local_metadata(path: &Path) -> io::Result<LocalMetadata> {
    let data = fs::read(path)?;

    let metadata = fs::metadata(path)?;

    let hash = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let last_modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();

    Ok(LocalMetadata {
        hash,
        last_modified,
    })
}

fn load_previous_server_manifest() -> Option<Manifest> {
    let home = std::env::var_os("HOME")?;

    let path = PathBuf::from(home).join(".salvation-server-manifest");

    let contents = fs::read_to_string(path).ok()?;

    toml::from_str(&contents).ok()
}

fn save_local_server_manifest(contents: &str) -> io::Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is not set"))?;

    fs::write(
        PathBuf::from(home).join(".salvation-server-manifest"),
        contents,
    )
}

fn server_url(server: &str, segments: &[&str]) -> io::Result<Url> {
    let mut url = Url::parse(server.trim_end_matches('/')).map_err(io::Error::other)?;

    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| io::Error::other("invalid server URL"))?;

        for segment in segments {
            path.push(segment);
        }
    }

    Ok(url)
}
