use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub struct Marker {
    pub flag: bool,
    pub pending: HashSet<PathBuf>,
}

impl Marker {
    pub fn new() -> Self {
        Self {
            flag: false,
            pending: HashSet::new(),
        }
    }

    pub fn handle_event(&mut self, path: PathBuf) {
        if !self.is_managed(&path) {
            return;
        }

        if !self.pending.insert(path.clone()) {
            return;
        }

        self.flag = true;

        println!("Marked: {}", path.display());
        println!("Pending: {}", self.pending.len());
        println!("Flag: UP");
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.flag = false;
    }

    fn is_managed(&self, path: &Path) -> bool {
        let home = match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home),
            None => return false,
        };

        let salvation = home.join(".salvation");

        let contents = match fs::read_to_string(salvation) {
            Ok(contents) => contents,
            Err(_) => return false,
        };

        contents.lines().any(|line| {
            let Some(saved_path) = line.strip_prefix("PATH: ") else {
                return false;
            };

            Path::new(saved_path) == path
        })
    }
}
