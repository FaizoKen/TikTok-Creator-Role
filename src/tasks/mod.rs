pub mod config_sync_worker;
pub mod player_sync_worker;
pub mod refresh_worker;

use std::sync::Arc;

use crate::AppState;

/// Periodically clean up expired OAuth states.
pub async fn cleanup_expired(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;

        if let Err(e) = sqlx::query("DELETE FROM oauth_states WHERE expires_at < now()")
            .execute(&state.pool)
            .await
        {
            tracing::error!("Failed to clean up OAuth states: {e}");
        }
    }
}
