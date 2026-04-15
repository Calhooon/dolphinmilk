//! Shared test utilities for integration tests that need an Axum router.
//!
//! Each function creates a dev-mode router backed by a `TempDir` that is
//! intentionally leaked via `std::mem::forget()` — `AppState` holds an
//! `Arc<PathBuf>` that outlives the test scope.

use std::path::PathBuf;

use dolphin_milk::config::DmConfig;
use dolphin_milk::server;

/// Config for tests — points wallet at a dead port so tests never accidentally
/// hit a real wallet. Auth is off (no identity key fetched = no auth).
fn test_config() -> DmConfig {
    let mut cfg = DmConfig::default();
    cfg.wallet.url = "http://127.0.0.1:19999".to_string(); // unreachable
    cfg
}

/// Build a test router with a fresh, leaked workspace directory.
#[allow(dead_code)]
pub async fn test_router() -> axum::Router {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);
    server::build_router(test_config(), path).await
}

/// Build a test router and return the workspace path for inspection.
#[allow(dead_code)]
pub async fn test_router_with_workspace() -> (axum::Router, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);
    let router = server::build_router(test_config(), path.clone()).await;
    (router, path)
}

/// Build a test router using an existing workspace directory.
#[allow(dead_code)]
pub async fn test_router_for_workspace(workspace: PathBuf) -> axum::Router {
    server::build_router(test_config(), workspace).await
}
