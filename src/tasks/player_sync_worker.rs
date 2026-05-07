use std::sync::Arc;

use tokio::sync::mpsc;

use crate::services::sync::{self, PlayerSyncEvent};
use crate::AppState;

pub async fn run(mut rx: mpsc::Receiver<PlayerSyncEvent>, state: Arc<AppState>) {
    tracing::info!("Player sync worker started");

    while let Some(event) = rx.recv().await {
        let result = match &event {
            PlayerSyncEvent::AccountLinked { discord_id }
            | PlayerSyncEvent::PlayerUpdated { discord_id } => {
                tracing::debug!(discord_id, event = ?event, "Syncing roles for player");
                sync::sync_for_player(discord_id, &state).await
            }
            PlayerSyncEvent::AccountUnlinked { discord_id } => {
                tracing::debug!(discord_id, "Removing all assignments for unlinked user");
                sync::remove_all_assignments(discord_id, &state).await
            }
        };

        if let Err(e) = result {
            tracing::error!(event = ?event, "Player sync failed: {e}");
        }
    }

    tracing::warn!("Player sync worker channel closed");
}
