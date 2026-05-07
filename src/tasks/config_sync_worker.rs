use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::services::sync::{self, ConfigSyncEvent};
use crate::AppState;

const DEBOUNCE_SECS: u64 = 5;

pub async fn run(mut rx: mpsc::Receiver<ConfigSyncEvent>, state: Arc<AppState>) {
    tracing::info!("Config sync worker started");

    let mut pending: HashMap<(String, String), Instant> = HashMap::new();
    let debounce = Duration::from_secs(DEBOUNCE_SECS);

    loop {
        match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(event)) => {
                pending.insert((event.guild_id, event.role_id), Instant::now());
                while let Ok(event) = rx.try_recv() {
                    pending.insert((event.guild_id, event.role_id), Instant::now());
                }
            }
            Ok(None) => break,
            Err(_) => {
                // Timeout — fall through to process debounced events.
            }
        }

        let now = Instant::now();
        let ready: Vec<(String, String)> = pending
            .iter()
            .filter(|(_, ts)| now.duration_since(**ts) >= debounce)
            .map(|(k, _)| k.clone())
            .collect();

        for key in ready {
            pending.remove(&key);
            let (guild_id, role_id) = &key;
            tracing::debug!(guild_id, role_id, "Syncing roles for config change (debounced)");

            if let Err(e) = sync::sync_for_role_link(guild_id, role_id, &state).await {
                tracing::error!(guild_id, role_id, "Config sync failed: {e}");
            }
        }
    }

    tracing::warn!("Config sync worker channel closed");
}
