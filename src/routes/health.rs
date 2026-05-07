use std::sync::Arc;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::AppState;

pub async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=604800"),
        ],
        include_bytes!("../../favicon.ico").as_slice(),
    )
}

async fn check_service(http: &reqwest::Client, name: &str, url: &str) -> Value {
    let start = std::time::Instant::now();
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(3), http.get(url).send()).await;
    let latency = start.elapsed().as_millis() as u64;

    let is_up = matches!(result, Ok(Ok(_)));

    json!({
        "name": name,
        "status": if is_up { "up" } else { "down" },
        "latency_ms": latency
    })
}

pub async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let db_fut = async {
        let start = std::time::Instant::now();
        let ok = sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&state.pool)
            .await
            .is_ok();
        (ok, start.elapsed().as_millis() as u64)
    };
    // Liveness ping for the TikTok Display API host. We use a HEAD-equivalent GET on
    // an unauthenticated host root — we just want network/TLS reachability info.
    let svc_fut = check_service(
        &state.http,
        "TikTok Display API",
        "https://open.tiktokapis.com/",
    );

    let ((db_ok, db_latency), svc_check) = tokio::join!(db_fut, svc_fut);

    let svc_down = svc_check["status"] == "down";
    let status = match (db_ok, svc_down) {
        (true, false) => "healthy",
        (false, true) => "unhealthy",
        _ => "degraded",
    };

    Json(json!({
        "status": status,
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "checks": {
            "database": {
                "status": if db_ok { "up" } else { "down" },
                "latency_ms": db_latency
            }
        },
        "services": [svc_check]
    }))
}
