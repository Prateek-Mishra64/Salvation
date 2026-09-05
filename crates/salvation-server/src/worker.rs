use std::{io, path::PathBuf, thread, time::Duration};

pub fn run(
    manifest_path: PathBuf,
    tmp_root: PathBuf,
    recall_root: PathBuf,
) -> io::Result<()> {
    loop {
        if let Err(error) =
            crate::lifecycle::process_manifest_file(&manifest_path, &tmp_root, &recall_root)
        {
            eprintln!("Lifecycle error: {error}");
        }

        thread::sleep(Duration::from_secs(60));
    }
}
