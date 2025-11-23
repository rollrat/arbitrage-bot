use std::{net::SocketAddr, sync::Arc};

use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing::info;

use interface::{PerpSnapshot, SpotSnapshot, UnifiedSnapshot};

#[derive(Clone)]
pub struct AppState {
    pub perp_snapshots: Arc<RwLock<Vec<PerpSnapshot>>>,
    pub spot_snapshots: Arc<RwLock<Vec<SpotSnapshot>>>,
    pub unified_snapshots: Arc<RwLock<Vec<UnifiedSnapshot>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            perp_snapshots: Arc::new(RwLock::new(Vec::new())),
            spot_snapshots: Arc::new(RwLock::new(Vec::new())),
            unified_snapshots: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

async fn snapshots_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let data = state.perp_snapshots.read().await.clone();
    Json(data)
}

async fn spot_snapshots_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let data = state.spot_snapshots.read().await.clone();
    Json(data)
}

async fn unified_snapshots_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let data = state.unified_snapshots.read().await.clone();
    Json(data)
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn serve(state: Arc<AppState>, port: u16) -> eyre::Result<()> {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/snapshots", get(snapshots_handler))
        .route("/spot-snapshots", get(spot_snapshots_handler))
        .route("/unified-snapshots", get(unified_snapshots_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
