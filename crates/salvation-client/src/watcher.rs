use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use std::{
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};

pub fn watch(paths: &[PathBuf], tx: Sender<PathBuf>) -> notify::Result<()> {
    let mut watcher = RecommendedWatcher::new(
        move |result| match result {
            Ok(Event { paths, .. }) => {
                for path in paths {
                    let _ = tx.send(path);
                }
            }

            Err(error) => {
                eprintln!("Watcher error: {error}");
            }
        },
        Config::default(),
    )?;

    for path in paths {
        watcher.watch(path, RecursiveMode::Recursive)?;
    }

    loop {
        std::thread::park();
    }
}
