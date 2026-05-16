use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("TikTok API error: {0}")]
    TikTokApi(String),

    #[error("TikTok OAuth token expired")]
    TikTokTokenExpired,

    #[error("TikTok OAuth token revoked by user")]
    TikTokTokenRevoked,

    #[error("TikTok rate limited")]
    TikTokRateLimited,

    #[error("RoleLogic API error: {0}")]
    RoleLogic(String),

    #[error("Role link not found on RoleLogic")]
    RoleLinkNotFound,

    #[error("Role link user limit reached ({limit})")]
    UserLimitReached { limit: usize },

    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Unauthorized: {0}")]
    UnauthorizedWith(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                tracing::error!("Database error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            AppError::TikTokApi(e) => {
                tracing::error!("TikTok API error: {e}");
                (
                    StatusCode::BAD_GATEWAY,
                    "Failed to fetch TikTok data".to_string(),
                )
            }
            AppError::TikTokTokenExpired => (
                StatusCode::UNAUTHORIZED,
                "TikTok session expired — please re-link your account".to_string(),
            ),
            AppError::TikTokTokenRevoked => (
                StatusCode::UNAUTHORIZED,
                "TikTok access was revoked — please re-link your account".to_string(),
            ),
            AppError::TikTokRateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "TikTok rate limit hit — please try again later".to_string(),
            ),
            AppError::RoleLogic(e) => {
                tracing::error!("RoleLogic API error: {e}");
                (StatusCode::BAD_GATEWAY, "Failed to sync roles".to_string())
            }
            AppError::RoleLinkNotFound => (StatusCode::NOT_FOUND, "Role link not found".to_string()),
            AppError::UserLimitReached { limit } => {
                tracing::warn!("Role link user limit reached: {limit}");
                (
                    StatusCode::FORBIDDEN,
                    "Role link user limit reached".to_string(),
                )
            }
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "Invalid or missing authorization".to_string(),
            ),
            AppError::UnauthorizedWith(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Internal(e) => {
                tracing::error!("Internal error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        let body = json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
