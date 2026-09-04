mod mapper;
mod marker;
mod placer;
mod watcher;

use std::{
    fs, io,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::channel,
    thread,
    time::{Duration, Instant},
};

const COLLECTION_WINDOW: Duration = Duration::from_secs(10);
const STABILIZATION_BUFFER: Duration = Duration::from_secs(10);

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();

    if args.first().map(String::as_str) == Some("--worker") {
        run_worker();
        return;
    }

    let has_sync_dir = args.iter().any(|arg| arg == "--sync-dir");

    if !has_sync_dir {
        eprintln!("No command specified.");
        std::process::exit(1);
    }

    let config = match mapper::parse_args(&args) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Argument error: {error}");
            std::process::exit(1);
        }
    };

    let mappings = match mapper::build_mapping(&config) {
        Ok(mappings) => mappings,
        Err(error) => {
            eprintln!("Mapping failed: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = mapper::save_mapping(&config, &mappings) {
        eprintln!("Failed to save .salvation: {error}");
        std::process::exit(1);
    }

    if let Err(error) = restart_worker() {
        eprintln!("Failed to restart worker: {error}");
        std::process::exit(1);
    }
}

/* -------------------------------------------------------------------------- */
/* Worker                                                                     */
/* -------------------------------------------------------------------------- */

fn run_worker() {
    if worker_is_running() {
        return;
    }

    let pid_path = pid_file();

    if let Err(error) = fs::write(&pid_path, std::process::id().to_string()) {
        eprintln!("Failed to write worker PID: {error}");
        return;
    }

    let result = worker_loop();

    let _ = fs::remove_file(pid_path);

    if let Err(error) = result {
        eprintln!("Worker failed: {error}");
    }
}

fn worker_loop() -> io::Result<()> {
    let directories = mapper::load_directories()?;

    if directories.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No directories configured in ~/.salvation",
        ));
    }

    let (tx, rx) = channel();

    let directories = mapper::load_directories()?;

    thread::spawn(move || {
        if let Err(error) = watcher::watch(&directories, tx) {
            eprintln!("Watcher failed: {error}");
        }
    });

    let mut marker = marker::Marker::new();

    loop {
        /*
         * --------------------------------------------------------------
         * COLLECTION WINDOW
         * --------------------------------------------------------------
         */

        let deadline = Instant::now() + COLLECTION_WINDOW;

        while Instant::now() < deadline {
            while let Ok(path) = rx.try_recv() {
                marker.handle_event(path);
            }

            thread::sleep(Duration::from_millis(50));
        }

        /*
         * --------------------------------------------------------------
         * STABILIZATION BUFFER
         * --------------------------------------------------------------
         *
         * Events are deliberately ignored.
         */

        thread::sleep(STABILIZATION_BUFFER);

        while rx.try_recv().is_ok() {}

        /*
         * --------------------------------------------------------------
         * DRY-RUN SYNC BOUNDARY
         * --------------------------------------------------------------
         *
         * Server/manifest integration comes later.
         */

        if marker.flag {
            // Real sync transaction goes here.
        }

        /*
         * Anything generated during the transaction/boundary
         * is discarded.
         */

        while rx.try_recv().is_ok() {}

        marker.clear();
    }
}

/* -------------------------------------------------------------------------- */
/* Worker lifecycle                                                           */
/* -------------------------------------------------------------------------- */

fn restart_worker() -> io::Result<()> {
    stop_worker()?;

    thread::sleep(Duration::from_millis(100));

    let executable = std::env::current_exe()?;

    Command::new(executable)
        .arg("--worker")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    Ok(())
}

fn stop_worker() -> io::Result<()> {
    let path = pid_file();

    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return Ok(()),
    };

    let pid = match contents.trim().parse::<i32>() {
        Ok(pid) => pid,
        Err(_) => {
            let _ = fs::remove_file(path);
            return Ok(());
        }
    };

    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()?;

    if !status.success() {
        let _ = fs::remove_file(&path);
    }

    Ok(())
}

fn worker_is_running() -> bool {
    let path = pid_file();

    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return false,
    };

    let pid = match contents.trim().parse::<i32>() {
        Ok(pid) => pid,
        Err(_) => {
            let _ = fs::remove_file(path);
            return false;
        }
    };

    match Command::new("kill").arg("-0").arg(pid.to_string()).status() {
        Ok(status) if status.success() => true,

        _ => {
            let _ = fs::remove_file(path);
            false
        }
    }
}

/* -------------------------------------------------------------------------- */
/* Paths                                                                      */
/* -------------------------------------------------------------------------- */

fn pid_file() -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME is not set");

    PathBuf::from(home).join(".salvation.pid")
}
