use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Router,
};

use std::{
    env,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};
use tokio::fs;

#[derive(Clone)]
struct AppState {
    password: String,
    storage_root: PathBuf,
}
#[tokio::main]
async fn main() {
    let password = env::var("Salvation_Password").expect("Salvation_Password is not set");

    let storage_root = env::var("Salvation_Storage").unwrap_or_else(|_| "./salvation-data".into());

    fs::create_dir_all(&storage_root)
        .await
        .expect("failed to create storage directory");

    let state = Arc::new(AppState {
        password,
        storage_root: PathBuf::from(storage_root),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/files/{*path}", post(upload).get(download))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .expect("failed to bind server");

    println!("Salvation server listening on 127.0.0.1:8080");

    axum::serve(listener, app).await.expect("server failed");
}

async fn health() -> &'static str {
    "OK\n"
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
            std::path::Component::Normal(part) => result.push(part),
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
) -> Result<&'static str, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let destination = safe_path(&state.storage_root, &path).ok_or(StatusCode::BAD_REQUEST)?;

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    fs::write(destination, body)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok("uploaded")
}

async fn download(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Result<Bytes, StatusCode> {
    if !authenticated(&headers, &state.password) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let source = safe_path(&state.storage_root, &path).ok_or(StatusCode::BAD_REQUEST)?;

    let data = fs::read(source).await.map_err(|_| StatusCode::NOT_FOUND)?;

    Ok(Bytes::from(data))
}
