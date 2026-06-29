//! Adaptive polling worker — refreshes `tiktok_stats_cache` rows for linked accounts.
//!
//! The interval is computed dynamically from cache size and `TIKTOK_QUOTA_PER_DAY`.
//! Active users (with at least one role assignment) are checked at the base cadence;
//! inactive users are checked 6× slower so they don't waste quota.
//!
//! On `TikTokTokenExpired` we proactively refresh the OAuth tokens and retry once.
//! On `TikTokTokenRevoked` we mark the account `revoked_at = now()` and back off for 24 hr.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

use crate::error::AppError;
use crate::services::sync::PlayerSyncEvent;
use crate::services::tiktok::TikTokUserInfo;
use crate::AppState;

const MIN_REFRESH_SECS: i64 = 1800; // 30 min floor
const MAX_REFRESH_SECS: i64 = 86_400; // 24 hr cap
const INTERVAL_CACHE_SECS: u64 = 300; // recompute every 5 minutes
const INACTIVE_MULTIPLIER: i64 = 6;
const REFRESH_BUFFER_SECS: i64 = 300; // refresh tokens this many seconds before expiry

struct CachedInterval {
    value: AtomicI64,
    last_computed: Mutex<Instant>,
}

impl CachedInterval {
    fn new() -> Self {
        Self {
            value: AtomicI64::new(MIN_REFRESH_SECS),
            last_computed: Mutex::new(
                Instant::now() - std::time::Duration::from_secs(INTERVAL_CACHE_SECS + 1),
            ),
        }
    }

    async fn get(&self, pool: &sqlx::PgPool, quota_per_day: i64) -> i64 {
        let mut last = self.last_computed.lock().await;
        if last.elapsed() >= std::time::Duration::from_secs(INTERVAL_CACHE_SECS) {
            let cache_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tiktok_stats_cache")
                .fetch_one(pool)
                .await
                .unwrap_or(0);

            let interval = if cache_count == 0 || quota_per_day <= 0 {
                MIN_REFRESH_SECS
            } else {
                // 1 API call per refresh per row.
                ((cache_count * 86_400) / quota_per_day.max(1))
                    .clamp(MIN_REFRESH_SECS, MAX_REFRESH_SECS)
            };

            self.value.store(interval, Ordering::Relaxed);
            *last = Instant::now();
        }
        self.value.load(Ordering::Relaxed)
    }
}

pub async fn run(state: Arc<AppState>) {
    tracing::info!("Refresh worker started (TikTok adaptive polling)");

    let cached_interval = CachedInterval::new();

    loop {
        // Rate-limit at the API layer.
        state.tiktok_client.wait_for_permit().await;

        // Pick next due cache entry — prefer active users, then lowest fetch_failures, then oldest due.
        let next = sqlx::query_as::<_, (String, bool)>(
            "SELECT c.discord_id, \
             EXISTS(SELECT 1 FROM role_assignments ra WHERE ra.discord_id = c.discord_id) AS is_active \
             FROM tiktok_stats_cache c \
             JOIN linked_accounts la ON la.discord_id = c.discord_id \
             WHERE c.next_fetch_at <= now() AND la.revoked_at IS NULL \
             ORDER BY is_active DESC, c.fetch_failures ASC, c.next_fetch_at ASC \
             LIMIT 1",
        )
        .fetch_optional(&state.pool)
        .await;

        let (discord_id, is_active) = match next {
            Ok(Some(row)) => row,
            Ok(None) => {
                tracing::debug!("No cache entries due for refresh, sleeping 30s");
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
            Err(e) => {
                tracing::error!("Refresh worker DB error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        let result = refresh_one_user(&state, &discord_id).await;

        match result {
            Ok(user_info) => {
                let bio = user_info.bio_description.trim();
                let has_bio = !bio.is_empty();
                let bio_length = bio.chars().count() as i32;

                let base_interval = cached_interval
                    .get(&state.pool, state.config.tiktok_quota_per_day)
                    .await;
                let multiplier = if is_active { 1 } else { INACTIVE_MULTIPLIER };
                let interval = base_interval * multiplier;
                let next_fetch = chrono::Utc::now() + chrono::Duration::seconds(interval);

                if let Err(e) = sqlx::query(
                    "UPDATE tiktok_stats_cache SET \
                       is_verified = $1, follower_count = $2, following_count = $3, \
                       likes_count = $4, video_count = $5, has_bio = $6, bio_length = $7, \
                       fetched_at = now(), next_fetch_at = $8, fetch_failures = 0, last_error = NULL \
                     WHERE discord_id = $9",
                )
                .bind(user_info.is_verified)
                .bind(user_info.follower_count)
                .bind(user_info.following_count)
                .bind(user_info.likes_count)
                .bind(user_info.video_count)
                .bind(has_bio)
                .bind(bio_length)
                .bind(next_fetch)
                .bind(&discord_id)
                .execute(&state.pool)
                .await
                {
                    tracing::error!(discord_id, "Failed to update stats cache: {e}");
                    continue;
                }

                // Also store the live username so the verify page stays in sync.
                let _ = sqlx::query(
                    "UPDATE linked_accounts SET tiktok_username = $1, tiktok_display_name = $2 \
                     WHERE discord_id = $3",
                )
                .bind(&user_info.username)
                .bind(&user_info.display_name)
                .bind(&discord_id)
                .execute(&state.pool)
                .await;

                let _ = state
                    .player_sync_tx
                    .send(PlayerSyncEvent::PlayerUpdated {
                        discord_id: discord_id.clone(),
                    })
                    .await;

                tracing::debug!(
                    discord_id,
                    is_active,
                    follower_count = user_info.follower_count,
                    "TikTok stats refreshed"
                );
            }
            Err(AppError::TikTokTokenRevoked) => {
                // User revoked OAuth (or refresh token aged past 365d). Back off 24h
                // and mark revoked so the verify page prompts them to relink.
                if let Err(e) = sqlx::query(
                    "UPDATE linked_accounts SET revoked_at = now() WHERE discord_id = $1",
                )
                .bind(&discord_id)
                .execute(&state.pool)
                .await
                {
                    tracing::error!(discord_id, "Failed to mark account revoked: {e}");
                }
                let _ = sqlx::query(
                    "UPDATE tiktok_stats_cache SET \
                       fetch_failures = fetch_failures + 1, \
                       last_error = $1, \
                       next_fetch_at = now() + INTERVAL '24 hours' \
                     WHERE discord_id = $2",
                )
                .bind("token_revoked")
                .bind(&discord_id)
                .execute(&state.pool)
                .await;
                tracing::warn!(discord_id, "TikTok OAuth token revoked, account marked");
            }
            Err(AppError::TikTokRateLimited) => {
                // Skip just this row, don't back off everyone.
                let _ = sqlx::query(
                    "UPDATE tiktok_stats_cache SET next_fetch_at = now() + INTERVAL '30 minutes' \
                     WHERE discord_id = $1",
                )
                .bind(&discord_id)
                .execute(&state.pool)
                .await;
                tracing::warn!(discord_id, "TikTok rate-limited, deferring 30 min");
            }
            Err(e) => {
                let _ = sqlx::query(
                    "UPDATE tiktok_stats_cache SET \
                       fetch_failures = fetch_failures + 1, \
                       last_error = $1, \
                       next_fetch_at = now() + LEAST(INTERVAL '60 seconds' * POWER(2, fetch_failures), INTERVAL '1 hour') \
                     WHERE discord_id = $2",
                )
                .bind(format!("{e}"))
                .bind(&discord_id)
                .execute(&state.pool)
                .await;
                tracing::warn!(discord_id, "TikTok refresh failed: {e}");
            }
        }
    }
}

/// Fetch one user's stats, refreshing the OAuth token first if it's expiring soon.
/// On 401 from the Display API we refresh the token and retry once.
async fn refresh_one_user(state: &AppState, discord_id: &str) -> Result<TikTokUserInfo, AppError> {
    let row = sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
        "SELECT tiktok_access_token, tiktok_refresh_token, tiktok_token_expires_at, tiktok_refresh_expires_at \
         FROM linked_accounts WHERE discord_id = $1 AND revoked_at IS NULL",
    )
    .bind(discord_id)
    .fetch_optional(&state.pool)
    .await?;

    let (mut access_token, mut refresh_token, token_expires_at, refresh_expires_at) =
        row.ok_or_else(|| AppError::Internal("linked_accounts row missing".into()))?;

    // If the refresh token itself is past TTL, the user has to relink — treat as revoked.
    if chrono::Utc::now() >= refresh_expires_at {
        return Err(AppError::TikTokTokenRevoked);
    }

    // Proactive refresh if the access token is within REFRESH_BUFFER_SECS of expiry.
    let needs_refresh =
        chrono::Utc::now() + chrono::Duration::seconds(REFRESH_BUFFER_SECS) >= token_expires_at;

    let mut already_refreshed = false;
    if needs_refresh {
        let new_tokens = state.tiktok_client.refresh_token(&refresh_token).await?;
        persist_refreshed_tokens(state, discord_id, &new_tokens).await?;
        access_token = new_tokens.access_token;
        refresh_token = new_tokens.refresh_token;
        already_refreshed = true;
    }

    match state.tiktok_client.fetch_user_info(&access_token).await {
        Ok(info) => Ok(info),
        Err(AppError::TikTokTokenExpired) if !already_refreshed => {
            // Reactive refresh on 401, retry once.
            let new_tokens = state.tiktok_client.refresh_token(&refresh_token).await?;
            persist_refreshed_tokens(state, discord_id, &new_tokens).await?;
            state
                .tiktok_client
                .fetch_user_info(&new_tokens.access_token)
                .await
        }
        Err(e) => Err(e),
    }
}

async fn persist_refreshed_tokens(
    state: &AppState,
    discord_id: &str,
    tokens: &crate::services::tiktok::TikTokTokens,
) -> Result<(), AppError> {
    let token_expires_at = chrono::Utc::now() + chrono::Duration::seconds(tokens.expires_in);
    let refresh_expires_at =
        chrono::Utc::now() + chrono::Duration::seconds(tokens.refresh_expires_in);

    sqlx::query(
        "UPDATE linked_accounts SET \
            tiktok_access_token = $1, \
            tiktok_refresh_token = $2, \
            tiktok_token_expires_at = $3, \
            tiktok_refresh_expires_at = $4 \
         WHERE discord_id = $5",
    )
    .bind(&tokens.access_token)
    .bind(&tokens.refresh_token)
    .bind(token_expires_at)
    .bind(refresh_expires_at)
    .bind(discord_id)
    .execute(&state.pool)
    .await?;

    Ok(())
}
