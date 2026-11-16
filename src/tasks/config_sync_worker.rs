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

/// A sync that fails is tried again a minute later, then after twice as long
/// for each further failure, up to 30 min. Without this a single dropped
/// connection lost the sync for good: an owner's save never reached
/// RoleLogic, and a held link's re-checks stopped until a restart.
const RETRY_FIRST_SECS: u64 = 60;
const RETRY_MAX_SECS: u64 = 30 * 60;
/// Failures in a row that log as warnings. A network blip heals on its first
/// retry; the failure after these lands about 15 min in and logs an error.
const QUIET_FAILURES: u32 = 4;

/// How long to wait after the `failures`-th failure in a row.
fn retry_delay(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    Duration::from_secs((RETRY_FIRST_SECS << doublings).min(RETRY_MAX_SECS))
}

pub async fn run(mut rx: mpsc::Receiver<ConfigSyncEvent>, state: Arc<AppState>) {
    tracing::info!("Config sync worker started");

    // When each link is next due. Every config event pushes its link back to
    // one debounce from now; a held write books one re-check, unless a fresh
    // event already has the link due sooner.
    let mut pending: HashMap<(String, String), Instant> = HashMap::new();
    let debounce = Duration::from_secs(DEBOUNCE_SECS);
    let held_recheck = Duration::from_secs(HELD_RECHECK_SECS);
    // Failures in a row per link, cleared by the next sync that settles.
    let mut failures: HashMap<(String, String), u32> = HashMap::new();

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
                Ok(()) => {
                    failures.remove(&key);
                    clear_held_recheck(&state, guild_id, role_id).await;
                }
                // A decision waiting on a person, not a failure: leave the
                // local mirror as it was and look again later, recomputing
                // the list then.
                Err(AppError::RemovalHeld) => {
                    failures.remove(&key);
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
                // Try again later, recomputing the list then. A held link's
                // booking stays as it was, so a restart still resumes it.
                Err(e) => {
                    let failed = failures.entry(key.clone()).or_insert(0);
                    *failed += 1;
                    let wait = retry_delay(*failed);
                    pending.entry(key.clone()).or_insert(Instant::now() + wait);
                    if *failed > QUIET_FAILURES {
                        tracing::error!(
                            guild_id,
                            role_id,
                            failures = *failed,
                            "Config sync keeps failing; retrying in {} min: {e}",
                            wait.as_secs() / 60
                        );
                    } else {
                        tracing::warn!(
                            guild_id,
                            role_id,
                            failures = *failed,
                            "Config sync failed; retrying in {} min: {e}",
                            wait.as_secs() / 60
                        );
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_to_the_cap() {
        let mins: Vec<u64> = (1..=8).map(|n| retry_delay(n).as_secs() / 60).collect();
        assert_eq!(mins, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(retry_delay(u32::MAX), Duration::from_secs(RETRY_MAX_SECS));
    }

    #[test]
    fn the_first_error_lands_about_15_min_in() {
        // Attempts run at 0, 1, 3, 7 and 15 min; the fifth failure logs an error.
        let before_error: u64 = (1..=QUIET_FAILURES).map(|n| retry_delay(n).as_secs()).sum();
        assert_eq!(before_error, 15 * 60);
    }
}
