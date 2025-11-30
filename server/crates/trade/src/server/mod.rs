use std::net::SocketAddr;

use axum::{response::IntoResponse, routing::get, Json, Router};
use tower_http::cors::CorsLayer;
use tracing::{error, info};

use crate::record::{get_position_repository, get_repository};

/// API 서버 시작
/// 백그라운드에서 실행되며 거래 기록과 포지션 기록을 조회하는 API를 제공합니다
pub async fn start_server(port: u16) -> eyre::Result<()> {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/trade-records", get(trade_records_handler))
        .route("/position-records", get(position_records_handler))
        .layer(CorsLayer::permissive());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Trade API server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Health check 핸들러
async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

/// 모든 거래 기록 조회 핸들러
async fn trade_records_handler() -> impl IntoResponse {
    let repo = match get_repository() {
        Some(repo) => repo,
        None => {
            error!("Trade record repository is not initialized");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Repository not initialized"
                })),
            )
                .into_response();
        }
    };

    match repo.find_all(None).await {
        Ok(records) => {
            info!("Returning {} trade records", records.len());
            Json(serde_json::json!(records)).into_response()
        }
        Err(e) => {
            error!("Failed to fetch trade records: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to fetch trade records: {}", e)
                })),
            )
                .into_response()
        }
    }
}

/// 모든 포지션 기록 조회 핸들러
async fn position_records_handler() -> impl IntoResponse {
    let repo = match get_position_repository() {
        Some(repo) => repo,
        None => {
            error!("Position record repository is not initialized");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Repository not initialized"
                })),
            )
                .into_response();
        }
    };

    match repo.find_all(None).await {
        Ok(records) => {
            info!("Returning {} position records", records.len());
            Json(serde_json::json!(records)).into_response()
        }
        Err(e) => {
            error!("Failed to fetch position records: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to fetch position records: {}", e)
                })),
            )
                .into_response()
        }
    }
}

