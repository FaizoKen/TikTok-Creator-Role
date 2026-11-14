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
/// config changes, so the re-check has to land inside that hour. Each booking
/// is also kept in `role_links.held_recheck_at`, so a restart resumes it.
const HELD_RECHECK_SECS: u64 = 20 * 60;

pub async fn run(mut rx: mpsc::Receiver<ConfigSyncEvent>, state: Arc<AppState>) {
    tracing::info!("Config sync worker started");

    // When each link is next due. Every config event pushes its link back to
    // one debounce from now; a held write books one re-check, unless a fresh
    // event already has the link due sooner.
    let mut pending: HashMap<(String, String), Instant> = HashMap::new();
    let debounce = Duration::from_secs(DEBOUNCE_SECS);
    let held_recheck = Duration::from_secs(HELD_RECHECK_SECS);

    // Re-checks booked before the last restart. Without them a held link
    // would not be re-sent until its config changed, and an owner's
    // confirmation would expire unused.
    for (key, due) in booked_held_rechecks(&state).await {
        pending.entry(key).or_insert(due);
    }

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
                // Written, or nothing to write: whatever was held is settled.
                Ok(()) => clear_held_recheck(&state, guild_id, role_id).await,
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
                    book_held_recheck(&state, guild_id, role_id).await;
                }
                // A booked re-check stays booked: a restart retries it.
                Err(e) => {
                    tracing::error!(guild_id, role_id, "Config sync failed: {e}");
                }
            }
        }
    }

    tracing::warn!("Config sync worker channel closed");
}

/// The held re-checks a previous run booked, each due when it was booked for,
/// or now if that time has passed.
async fn booked_held_rechecks(state: &AppState) -> Vec<((String, String), Instant)> {
    let rows: Result<Vec<(String, String, i64)>, sqlx::Error> = sqlx::query_as(
        "SELECT guild_id, role_id, \
                GREATEST(0, EXTRACT(EPOCH FROM held_recheck_at - now()))::bigint \
         FROM role_links WHERE held_recheck_at IS NOT NULL",
    )
    .fetch_all(&state.pool)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Could not load held re-checks from before the restart: {e}");
            return Vec::new();
        }
    };
    if !rows.is_empty() {
        tracing::info!(
            links = rows.len(),
            "Resuming held re-checks from before the restart"
        );
    }
    let now = Instant::now();
    rows.into_iter()
        .map(|(guild_id, role_id, secs)| {
            (
                (guild_id, role_id),
                now + Duration::from_secs(secs.max(0) as u64),
            )
        })
        .collect()
}

/// Keep a held link's next re-check across a restart.
async fn book_held_recheck(state: &AppState, guild_id: &str, role_id: &str) {
    if let Err(e) = sqlx::query(
        "UPDATE role_links SET held_recheck_at = now() + make_interval(secs => $3) \
         WHERE guild_id = $1 AND role_id = $2",
    )
    .bind(guild_id)
    .bind(role_id)
    .bind(HELD_RECHECK_SECS as f64)
    .execute(&state.pool)
    .await
    {
        tracing::warn!(guild_id, role_id, "Could not book the held re-check: {e}");
    }
}

/// The link synced, so nothing is held any more.
async fn clear_held_recheck(state: &AppState, guild_id: &str, role_id: &str) {
    if let Err(e) = sqlx::query(
        "UPDATE role_links SET held_recheck_at = NULL \
         WHERE guild_id = $1 AND role_id = $2 AND held_recheck_at IS NOT NULL",
    )
    .bind(guild_id)
    .bind(role_id)
    .execute(&state.pool)
    .await
    {
        tracing::warn!(guild_id, role_id, "Could not clear the held re-check: {e}");
    }
}
