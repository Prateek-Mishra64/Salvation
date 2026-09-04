use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct FileMapping {
    pub full_path: PathBuf,
    pub parent_vector: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DirectoryConfig {
    pub path: PathBuf,
    pub files: Vec<String>,
    pub all: bool,
}

#[derive(Debug, Clone)]
pub struct MapperConfig {
    pub directories: Vec<DirectoryConfig>,
}

pub fn parse_args(args: &[String]) -> Result<MapperConfig, String> {
    let mut directories = Vec::new();
    let mut index = 0;

    while index < args.len() {
        if args[index] != "--sync-dir" {
            return Err(format!("Unknown argument: {}", args[index]));
        }

        index += 1;

        let directory = args.get(index).ok_or("--sync-dir requires a directory")?;

        let directory = expand_home(directory);

        index += 1;

        if args.get(index).map(String::as_str) != Some("--files") {
            return Err("--sync-dir must be followed by --files".into());
        }

        index += 1;

        let value = args
            .get(index)
            .ok_or("--files requires extensions or all")?;

        let (files, all) = if value.eq_ignore_ascii_case("all") {
            (Vec::new(), true)
        } else {
            let files = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    let value = value.to_lowercase();

                    if value.starts_with('.') {
                        value
                    } else {
                        format!(".{value}")
                    }
                })
                .collect::<Vec<_>>();

            if files.is_empty() {
                return Err("--files requires extensions or all".into());
            }

            (files, false)
        };

        directories.push(DirectoryConfig {
            path: directory,
            files,
            all,
        });

        index += 1;
    }

    if directories.is_empty() {
        return Err("At least one --sync-dir is required".into());
    }

    Ok(MapperConfig { directories })
}

pub fn build_mapping(config: &MapperConfig) -> io::Result<Vec<FileMapping>> {
    let mut mappings = Vec::new();

    for directory in &config.directories {
        let root = fs::canonicalize(&directory.path)?;

        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Not a directory: {}", root.display()),
            ));
        }

        scan(root.as_path(), root.as_path(), directory, &mut mappings)?;
    }

    Ok(mappings)
}

fn scan(
    root: &Path,
    current: &Path,
    config: &DirectoryConfig,
    mappings: &mut Vec<FileMapping>,
) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let path = entry?.path();

        if path.is_dir() {
            scan(root, &path, config, mappings)?;
            continue;
        }

        if !path.is_file() || !selected(&path, config) {
            continue;
        }

        let full_path = fs::canonicalize(&path)?;
        let parent_vector = collect_parent_vector(&full_path);

        mappings.push(FileMapping {
            full_path,
            parent_vector,
        });
    }

    let _ = root;

    Ok(())
}

fn collect_parent_vector(file: &Path) -> Vec<String> {
    let mut parents = file
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .filter_map(|path| path.file_name())
        .take(3)
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    parents.reverse();
    parents
}

fn selected(path: &Path, config: &DirectoryConfig) -> bool {
    if config.all {
        return true;
    }

    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };

    let extension = format!(".{}", extension.to_lowercase());

    config.files.iter().any(|file| file == &extension)
}

pub fn save_mapping(config: &MapperConfig, mappings: &[FileMapping]) -> io::Result<()> {
    let salvation = salvation_path();

    let existing = if salvation.exists() {
        fs::read_to_string(&salvation)?
    } else {
        String::new()
    };

    let mut updated = existing;

    for directory in &config.directories {
        let root = fs::canonicalize(&directory.path)?;

        let directory_mappings = mappings
            .iter()
            .filter(|mapping| mapping.full_path.starts_with(&root))
            .cloned()
            .collect::<Vec<_>>();

        let block = build_directory_block(&root, directory, &directory_mappings);

        updated = replace_directory_block(&updated, &root, &block);
    }

    let temp_path = salvation.with_extension("tmp");

    fs::write(&temp_path, updated)?;
    fs::rename(temp_path, salvation)?;

    Ok(())
}

pub fn load_directories() -> io::Result<Vec<PathBuf>> {
    let contents = fs::read_to_string(salvation_path())?;

    let mut directories = Vec::new();
    let mut in_directory = false;

    for line in contents.lines() {
        match line {
            "[DIRECTORY]" => {
                in_directory = true;
            }

            _ if in_directory => {
                if let Some(path) = line.strip_prefix("PATH: ") {
                    directories.push(PathBuf::from(path));
                    in_directory = false;
                }
            }

            _ => {}
        }
    }

    Ok(directories)
}

fn salvation_path() -> PathBuf {
    expand_home("~").join(".salvation")
}

fn build_directory_block(
    directory: &Path,
    config: &DirectoryConfig,
    mappings: &[FileMapping],
) -> String {
    let mut block = String::new();

    block.push_str("[DIRECTORY]\n");
    block.push_str(&format!("PATH: {}\n\n", directory.display()));

    block.push_str("[FILE RULES]\n");

    if config.all {
        block.push_str("all\n");
    } else {
        for file in &config.files {
            block.push_str(file);
            block.push('\n');
        }
    }

    block.push_str("\n[FILES]\n\n");

    for mapping in mappings {
        block.push_str(&format!("PATH: {}\n", mapping.full_path.display()));

        block.push_str("PARENTS:\n");

        for parent in &mapping.parent_vector {
            block.push_str(&format!("  {}\n", parent));
        }

        block.push('\n');
    }

    block
}

fn replace_directory_block(existing: &str, directory: &Path, new_block: &str) -> String {
    let marker = format!("[DIRECTORY]\nPATH: {}\n", directory.display());

    let Some(start) = existing.find(&marker) else {
        let mut result = existing.to_string();

        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }

        if !result.is_empty() {
            result.push('\n');
        }

        result.push_str(new_block);

        return result;
    };

    let rest = &existing[start..];

    let end = rest
        .find("\n[DIRECTORY]\n")
        .map(|offset| start + offset + 1)
        .unwrap_or(existing.len());

    let mut result = String::new();

    result.push_str(&existing[..start]);
    result.push_str(new_block);

    if end < existing.len() {
        result.push_str(&existing[end..]);
    }

    result
}

fn expand_home(path: &str) -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is not set"));

    if path == "~" {
        return home;
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return home.join(rest);
    }

    PathBuf::from(path)
}
