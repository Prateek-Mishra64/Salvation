mod mapper;
mod marker;
mod placer;
mod sync;
mod watcher;

use std::{
    env, fs, io, path::PathBuf, process::Command, sync::mpsc::channel, thread, time::Duration,
};

const SYNC_INTERVAL: Duration = Duration::from_secs(10);

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();

    if args.first().map(String::as_str) == Some("--worker") {
        run_worker();
        return;
    }

    if !args.iter().any(|arg| arg == "--sync-dir") {
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

    println!("Salvation worker started.");
    println!("Synchronization interval: 10 minutes.");
}

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

    // Keep the watcher alive. Reconciliation below is the authoritative
    // mechanism for discovering new files and changed structure.
    let (tx, _rx) = channel();
    thread::spawn(move || {
        if let Err(error) = watcher::watch(&directories, tx) {
            eprintln!("Watcher failed: {error}");
        }
    });

    loop {
        println!("Reconciling filesystem state...");

        let mappings = mapper::reconcile()?;
        println!("Managed files: {}", mappings.len());

        let server = env::var("SALVATION_SERVER")
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "SALVATION_SERVER is not set"))?;

        let password = env::var("Salvation_Password").map_err(|_| {
            io::Error::new(io::ErrorKind::NotFound, "Salvation_Password is not set")
        })?;

        let runtime = tokio::runtime::Runtime::new().map_err(io::Error::other)?;

        if let Err(error) = runtime.block_on(sync::synchronize(&server, &password)) {
            eprintln!("Synchronization failed: {error}");
        }

        println!("Next synchronization in 10 minutes.");
        thread::sleep(sync_interval());
    }
}

fn sync_interval() -> Duration {
    match env::var("SALVATION_SYNC_INTERVAL") {
        Ok(value) => match value.parse::<u64>() {
            Ok(seconds) if seconds > 0 => Duration::from_secs(seconds),
            _ => SYNC_INTERVAL,
        },
        Err(_) => SYNC_INTERVAL,
    }
}

fn restart_worker() -> io::Result<()> {
    stop_worker()?;
    thread::sleep(Duration::from_millis(100));

    let executable = std::env::current_exe()?;

    Command::new(executable).arg("--worker").spawn()?;

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

fn pid_file() -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME is not set");
    PathBuf::from(home).join(".salvation.pid")
}
