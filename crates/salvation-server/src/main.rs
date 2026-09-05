mod init;
mod lifecycle;
mod manifest;
mod transaction;
mod worker;

use axum::extract::DefaultBodyLimit;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};

use serde_json::json;

use std::{
    env, fs,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

use tokio::fs as tokio_fs;

#[derive(Clone)]
struct AppState {
    password: String,
    storage_root: PathBuf,
    transaction: Arc<std::sync::Mutex<Option<transaction::Transaction>>>,
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.get(1).map(String::as_str) == Some("--init") {
        init::initialize().expect("failed to initialize Salvation server");

        return;
    }

    let password = env::var("Salvation_Password").expect("Salvation_Password is not set");

    let storage_root =
        PathBuf::from(env::var("Salvation_Storage").unwrap_or_else(|_| "./salvation-data".into()));

    tokio_fs::create_dir_all(&storage_root)
        .await
        .expect("failed to create storage directory");

    let worker_manifest = storage_root.join(".manifest");

    let worker_tmp = storage_root.join("tmp");

    let worker_recall = storage_root.join("recall");

    std::thread::spawn(move || {
        worker::run(worker_manifest, worker_tmp, worker_recall).expect("lifecycle worker failed");
    });

    let state = Arc::new(AppState {
        password,
        storage_root,
        transaction: Arc::new(std::sync::Mutex::new(None)),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/manifest-info", get(manifest_info))
        .route("/recall", get(recall))
        .route("/files/{*path}", post(upload).get(download))
        .route("/transaction/confirm", post(confirm_transaction))
        .route("/tmp-log", get(tmp_log))
        .route("/recall-log", get(recall_log))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("failed to bind server");

    println!("Salvation server listening on 0.0.0.0:8080");

    axum::serve(listener, app).await.expect("server failed");
}

async fn health() -> &'static str {
    "OK\n"
}

async fn manifest_info(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<String, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let manifest_path = state.storage_root.join(".manifest");

    let manifest = manifest::load(&manifest_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    toml::to_string(&manifest).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn recall(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let manifest_path = state.storage_root.join(".manifest");

    let manifest = manifest::load(&manifest_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let current_time = lifecycle::now();

    let files = manifest
        .files
        .iter()
        .filter(|file| file.location == manifest::Location::Recall)
        .map(|file| {
            let elapsed = current_time.saturating_sub(file.lifecycle_at);

            let remaining = lifecycle::RECALL_RETENTION.saturating_sub(elapsed);

            json!({
                "path": file.path,
                "remaining_seconds": remaining
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(json!({ "files": files })))
}

fn authenticated(headers: &HeaderMap, password: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == format!("Bearer {password}"))
        .unwrap_or(false)
}

fn safe_path(root: &FsPath, requested: &str) -> Option<PathBuf> {
    let mut result = root.to_path_buf();

    for component in FsPath::new(requested).components() {
        match component {
            std::path::Component::Normal(part) => {
                result.push(part);
            }

            _ => return None,
        }
    }

    Some(result)
}

async fn upload(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<String, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let mut active = state
        .transaction
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if active.is_none() {
        let manifest_path = state.storage_root.join(".manifest");

        let current_manifest =
            manifest::load(&manifest_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let transaction = transaction::Transaction::begin(&state.storage_root, &current_manifest)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        *active = Some(transaction);
    }

    let transaction = active.as_mut().unwrap();

    transaction
        .stage_file(&path, &body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    toml::to_string(transaction.manifest()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn confirm_transaction(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<String, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let mut active = state
        .transaction
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let transaction = active.take().ok_or(StatusCode::CONFLICT)?;

    let files_root = state.storage_root.join("files");

    let tmp_root = state.storage_root.join("tmp");

    let manifest_path = state.storage_root.join(".manifest");

    transaction
        .commit(&files_root, &tmp_root, &manifest_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let manifest = manifest::load(&manifest_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    toml::to_string(&manifest).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn download(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Result<Bytes, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let manifest_path = state.storage_root.join(".manifest");

    let manifest = manifest::load(&manifest_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let entry = manifest.find_file(&path).ok_or(StatusCode::NOT_FOUND)?;

    let root = match entry.location {
        manifest::Location::Files => state.storage_root.join("files"),

        manifest::Location::Tmp => state.storage_root.join("tmp"),

        manifest::Location::Recall => state.storage_root.join("recall"),
    };

    let source = safe_path(&root, &path).ok_or(StatusCode::BAD_REQUEST)?;

    let data = tokio_fs::read(source)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Bytes::from(data))
}

async fn tmp_log(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<String, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    read_log(&state.storage_root.join("logs/tmp.log"))
}

async fn recall_log(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<String, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    read_log(&state.storage_root.join("logs/recall.log"))
}

fn read_log(path: &FsPath) -> Result<String, StatusCode> {
    if !path.exists() {
        return Ok(String::new());
    }

    let contents = fs::read_to_string(path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let lines: Vec<&str> = contents.lines().collect();

    let start = lines.len().saturating_sub(10);

    Ok(lines[start..].join("\n"))
}
