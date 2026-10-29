use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::error::AppError;
use crate::services::sync::{self, ConfigSyncEvent};
use crate::AppState;

const DEBOUNCE_SECS: u64 = 5;
/// How soon a link whose write RoleLogic is holding gets synced again.
/// Confirming the removals in the RoleLogic dashboard lets the next write
/// through for an hour, and nothing else re-sends a link's list until its
/// config changes, so the re-check has to land inside that hour.
const HELD_RECHECK_SECS: u64 = 20 * 60;

pub async fn run(mut rx: mpsc::Receiver<ConfigSyncEvent>, state: Arc<AppState>) {
    tracing::info!("Config sync worker started");

    // When each link is next due. Every config event pushes its link back to
    // one debounce from now; a held write books one re-check, unless a fresh
    // event already has the link due sooner.
    let mut pending: HashMap<(String, String), Instant> = HashMap::new();
    let debounce = Duration::from_secs(DEBOUNCE_SECS);
    let held_recheck = Duration::from_secs(HELD_RECHECK_SECS);

    loop {
        match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(event)) => {
                pending.insert((event.guild_id, event.role_id), Instant::now() + debounce);
                while let Ok(event) = rx.try_recv() {
                    pending.insert((event.guild_id, event.role_id), Instant::now() + debounce);
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
            .filter(|(_, due)| **due <= now)
            .map(|(k, _)| k.clone())
            .collect();

        for key in ready {
            pending.remove(&key);
            let (guild_id, role_id) = &key;
            tracing::debug!(
                guild_id,
                role_id,
                "Syncing roles for config change (debounced)"
            );

            match sync::sync_for_role_link(guild_id, role_id, &state).await {
                Ok(()) => {}
                // A decision waiting on a person, not a failure: leave the
                // local mirror as it was and look again later, recomputing
                // the list then.
                Err(AppError::RemovalHeld) => {
                    tracing::info!(
                        guild_id,
                        role_id,
                        "RoleLogic is holding this link's removals for owner confirmation; re-checking in {} min",
                        HELD_RECHECK_SECS / 60
                    );
                    pending
                        .entry(key.clone())
                        .or_insert(Instant::now() + held_recheck);
                }
                Err(e) => {
                    tracing::error!(guild_id, role_id, "Config sync failed: {e}");
                }
            }
        }
    }

    tracing::warn!("Config sync worker channel closed");
}
